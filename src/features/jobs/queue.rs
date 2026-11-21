//! One FIFO queue for every job that waits for a slot.
//!
//! [`crate::AppState`] owns one queue. The global limit and the per-team limit
//! are enforced together: when a slot frees, the oldest waiting job whose team
//! is below its own limit takes it. A team therefore cannot refill its own
//! slots ahead of older jobs from other teams, and the global limit still
//! applies across all teams.
//!
//! Each waiting job holds the receiving half of a oneshot channel. The
//! admission wakes exactly that job instead of waking every waiter, so a free
//! slot cannot be taken by a job that arrived later.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use tokio::sync::oneshot;

struct Waiter {
    job_id: String,
    team_id: String,
    admit: oneshot::Sender<()>,
}

struct Inner {
    /// Arrival order. The first entry whose team has room is admitted next.
    waiting: Vec<Waiter>,
    running: usize,
    per_team: HashMap<String, usize>,
}

pub(crate) struct JobQueue {
    inner: Mutex<Inner>,
    global_limit: usize,
    per_team_limit: usize,
}

impl JobQueue {
    /// Builds the queue. The per-team limit is clamped to `1..=global`, so a
    /// caller that did not go through the startup check still gets a usable
    /// queue.
    pub(crate) fn new(global_limit: usize, per_team_limit: usize) -> Self {
        let global_limit = global_limit.max(1);
        let per_team_limit = per_team_limit.max(1).min(global_limit);
        Self {
            inner: Mutex::new(Inner {
                waiting: Vec::new(),
                running: 0,
                per_team: HashMap::new(),
            }),
            global_limit,
            per_team_limit,
        }
    }

    /// Adds a job to the queue. The receiver resolves once the job may run;
    /// it resolves with an error when the job left the queue, for example
    /// because it was canceled while it waited.
    pub(crate) fn enqueue(&self, job_id: &str, team_id: &str) -> oneshot::Receiver<()> {
        let (admit, admitted) = oneshot::channel();
        let mut inner = self.lock();
        inner.waiting.push(Waiter {
            job_id: job_id.to_string(),
            team_id: team_id.to_string(),
            admit,
        });
        self.dispatch(&mut inner);
        admitted
    }

    /// Removes a job that waits for a slot. A job that already received its
    /// grant is not in the queue; the runner refuses it because the stored row
    /// is no longer `pending`, and its caller releases the slot afterwards.
    pub(crate) fn cancel(&self, job_id: &str) -> bool {
        let mut inner = self.lock();
        let Some(index) = inner.waiting.iter().position(|item| item.job_id == job_id) else {
            return false;
        };
        // Dropping the sender wakes the waiting task with an error, so it
        // returns without running. A waiting job never held a slot.
        inner.waiting.remove(index);
        true
    }

    /// Gives the slot of a finished job back and admits the next waiters.
    pub(crate) fn release(&self, team_id: &str) {
        let mut inner = self.lock();
        inner.running = inner.running.saturating_sub(1);
        if let Some(count) = inner.per_team.get_mut(team_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.per_team.remove(team_id);
            }
        }
        self.dispatch(&mut inner);
    }

    /// Admits waiters while the global limit has room. The scan starts at the
    /// front, so the oldest waiting job whose team has room wins. A job whose
    /// team is at its own limit is skipped for the next job in arrival order,
    /// which may belong to another team.
    fn dispatch(&self, inner: &mut Inner) {
        while inner.running < self.global_limit {
            let Some(index) = inner.waiting.iter().position(|item| {
                inner.per_team.get(&item.team_id).copied().unwrap_or(0) < self.per_team_limit
            }) else {
                break;
            };
            let waiter = inner.waiting.remove(index);
            inner.running += 1;
            *inner.per_team.entry(waiter.team_id.clone()).or_insert(0) += 1;
            if waiter.admit.send(()).is_err() {
                // The waiting task is gone. Take the slot back right away.
                inner.running = inner.running.saturating_sub(1);
                if let Some(count) = inner.per_team.get_mut(&waiter.team_id) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        inner.per_team.remove(&waiter.team_id);
                    }
                }
            }
        }
    }

    /// A panicking task must not keep the whole queue waiting, so a poisoned
    /// lock is recovered rather than reported.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::JobQueue;
    use tokio::sync::oneshot::error::TryRecvError;

    fn waiting(receiver: &mut tokio::sync::oneshot::Receiver<()>) -> bool {
        matches!(receiver.try_recv(), Err(TryRecvError::Empty))
    }

    #[test]
    fn each_team_stays_at_its_limit_and_free_slots_follow_arrival_order() {
        let queue = JobQueue::new(4, 2);
        let mut a1 = queue.enqueue("a1", "a");
        let mut a2 = queue.enqueue("a2", "a");
        let mut a3 = queue.enqueue("a3", "a");
        let mut b1 = queue.enqueue("b1", "b");
        let mut b2 = queue.enqueue("b2", "b");
        let mut b3 = queue.enqueue("b3", "b");

        // Team A filled its two places; the next free global slots went to
        // team B, and both teams have one waiting job.
        assert!(matches!(a1.try_recv(), Ok(())));
        assert!(matches!(a2.try_recv(), Ok(())));
        assert!(waiting(&mut a3));
        assert!(matches!(b1.try_recv(), Ok(())));
        assert!(matches!(b2.try_recv(), Ok(())));
        assert!(waiting(&mut b3));

        // A1 finishes. The global limit is still full, so only one waiter can
        // take the place, and the oldest overall (A3) gets it.
        queue.release("a");
        assert!(matches!(a3.try_recv(), Ok(())));
        assert!(waiting(&mut b3));

        // A2 finishes. The global limit has room, but team B still holds both
        // of its places, so its third job keeps waiting.
        queue.release("a");
        assert!(waiting(&mut b3));

        // The first team B job finishes, and B3 finally has a place.
        queue.release("b");
        assert!(matches!(b3.try_recv(), Ok(())));
    }

    #[test]
    fn a_canceled_waiter_leaves_the_queue_and_the_oldest_waiter_takes_the_slot() {
        let queue = JobQueue::new(1, 1);
        let mut running = queue.enqueue("a1", "a");
        let mut canceled = queue.enqueue("a2", "a");
        let mut next = queue.enqueue("b1", "b");
        assert!(matches!(running.try_recv(), Ok(())));
        assert!(waiting(&mut canceled));
        assert!(waiting(&mut next));

        assert!(queue.cancel("a2"));
        assert!(matches!(canceled.try_recv(), Err(TryRecvError::Closed)));

        // The canceled job held no slot; the only running job still ends
        // before the next waiter starts.
        assert!(waiting(&mut next));
        queue.release("a");
        assert!(matches!(next.try_recv(), Ok(())));
    }

    #[test]
    fn the_first_team_keeps_its_own_slot_when_its_waiter_is_oldest() {
        let queue = JobQueue::new(2, 1);
        let mut a1 = queue.enqueue("a1", "a");
        let mut a2 = queue.enqueue("a2", "a");
        let mut b1 = queue.enqueue("b1", "b");
        assert!(matches!(a1.try_recv(), Ok(())));
        assert!(waiting(&mut a2));
        assert!(matches!(b1.try_recv(), Ok(())));

        // A2 arrived before B1, so the freed slot is its place. FIFO decides,
        // not the team that just finished.
        queue.release("a");
        assert!(matches!(a2.try_recv(), Ok(())));
    }

    #[test]
    fn a_per_team_limit_above_the_global_limit_uses_the_global_limit() {
        let queue = JobQueue::new(2, 9);
        let mut first = queue.enqueue("a1", "a");
        let mut second = queue.enqueue("a2", "a");
        let mut third = queue.enqueue("a3", "a");
        assert!(matches!(first.try_recv(), Ok(())));
        assert!(matches!(second.try_recv(), Ok(())));
        assert!(waiting(&mut third));
    }

    #[test]
    fn a_dropped_waiter_gives_its_place_back() {
        let queue = JobQueue::new(1, 1);
        let mut first = queue.enqueue("a1", "a");
        let dropped = queue.enqueue("a2", "a");
        let mut third = queue.enqueue("a3", "a");
        assert!(matches!(first.try_recv(), Ok(())));

        // A task whose receiver goes away never takes the slot it was offered.
        // The admission falls through to the next waiter.
        drop(dropped);
        queue.release("a");
        assert!(matches!(third.try_recv(), Ok(())));
        assert!(!queue.cancel("a2"), "the dropped waiter already left");
    }
}
