use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde_json::{Map, Value};

use super::model::{Convergence, Graph, GraphEdge, NodeOverrides, NodePhase, Step, When};
use crate::error::AppResult;
use crate::{db, Shared};

/// Writes node overrides onto a parameter snapshot from `capture_parameters`.
pub fn apply_overrides(params: &mut Map<String, Value>, overrides: &NodeOverrides) {
    if let Some(id) = &overrides.inventory_id {
        params.insert(
            "inventory_id".into(),
            if id.is_empty() {
                Value::Null
            } else {
                Value::String(id.clone())
            },
        );
        params.insert("inventory_name".into(), Value::Null);
    }
    if let Some(id) = &overrides.credential_id {
        params.insert(
            "credential_id".into(),
            if id.is_empty() {
                Value::Null
            } else {
                Value::String(id.clone())
            },
        );
        params.insert("credential_name".into(), Value::Null);
        params.insert("credential_kind".into(), Value::Null);
        if id.is_empty() {
            params.insert("credentials".into(), Value::Array(Vec::new()));
        }
    }
    if let Some(extra) = &overrides.extra_vars {
        params.insert("extra_vars".into(), extra.clone());
    }
    if let Some(spec) = &overrides.spec {
        let base = params
            .get("spec")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        params.insert("spec".into(), merge_spec(&base, spec));
    }
}

fn merge_spec(base: &Value, overlay: &Value) -> Value {
    let mut out = match base {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    if let Some(extra) = overlay.as_object() {
        for (key, value) in extra {
            if value.is_null() {
                out.remove(key);
            } else {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(out)
}

pub fn validate(graph: &Graph) -> Result<(), String> {
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err("a_node_has_no_id".into());
        }
        if node.template_id.trim().is_empty() {
            return Err(crate::error::message(
                "workflow.node_template",
                &[("id", node.id.as_str())],
            ));
        }
        if node.name.trim().is_empty() {
            return Err(crate::error::message(
                "workflow.node_name",
                &[("id", node.id.as_str())],
            ));
        }
        if !ids.insert(node.id.clone()) {
            return Err(crate::error::message(
                "workflow.node_duplicate",
                &[("id", node.id.as_str())],
            ));
        }
    }
    let mut seen = HashSet::new();
    for edge in &graph.edges {
        if !ids.contains(&edge.from) || !ids.contains(&edge.to) {
            return Err("an_edge_points_at_an_unknown_node".into());
        }
        if edge.from == edge.to {
            return Err("an_edge_cannot_point_at_the_same_node".into());
        }
        let key = format!("{}->{}:{:?}", edge.from, edge.to, edge.when);
        if !seen.insert(key) {
            return Err("the_same_edge_is_present_twice".into());
        }
    }
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for node in &graph.nodes {
        if has_cycle(graph, &node.id, &mut visiting, &mut done) {
            return Err("the_workflow_contains_a_cycle".into());
        }
    }
    Ok(())
}

fn has_cycle(
    graph: &Graph,
    id: &str,
    visiting: &mut HashSet<String>,
    done: &mut HashSet<String>,
) -> bool {
    if done.contains(id) {
        return false;
    }
    if !visiting.insert(id.to_string()) {
        return true;
    }
    for edge in graph.edges.iter().filter(|e| e.from == id) {
        if has_cycle(graph, &edge.to, visiting, done) {
            return true;
        }
    }
    visiting.remove(id);
    done.insert(id.to_string());
    false
}

pub fn advance(graph: &Graph, phase: &HashMap<String, NodePhase>) -> Vec<Step> {
    let mut steps = Vec::new();
    for node in &graph.nodes {
        if phase.get(&node.id).copied().unwrap_or(NodePhase::Pending) != NodePhase::Pending {
            continue;
        }
        let incoming: Vec<&GraphEdge> = graph.edges.iter().filter(|e| e.to == node.id).collect();
        if incoming.is_empty() {
            steps.push(Step::Launch(node.id.clone()));
            continue;
        }
        let terminal = incoming.iter().all(|edge| {
            phase
                .get(&edge.from)
                .copied()
                .unwrap_or(NodePhase::Pending)
                .is_terminal()
        });
        let matched: Vec<bool> = incoming
            .iter()
            .map(|edge| edge_matches(edge.when, phase.get(&edge.from).copied()))
            .collect();
        match node.convergence {
            Convergence::Any => {
                if matched.iter().any(|hit| *hit) {
                    steps.push(Step::Launch(node.id.clone()));
                } else if terminal {
                    steps.push(Step::Skip(node.id.clone()));
                }
            }
            Convergence::All => {
                if !terminal {
                    continue;
                }
                if matched.iter().all(|hit| *hit) {
                    steps.push(Step::Launch(node.id.clone()));
                } else {
                    steps.push(Step::Skip(node.id.clone()));
                }
            }
        }
    }
    steps
}

fn edge_matches(when: When, source: Option<NodePhase>) -> bool {
    matches!(
        (when, source),
        (When::Success, Some(NodePhase::Success))
            | (When::Failure, Some(NodePhase::Failed))
            | (When::Always, Some(NodePhase::Success | NodePhase::Failed))
    )
}

/// A failed node is handled when a failure or always edge leaves it.
/// The workflow then succeeds if every failure was handled.
pub fn final_status(graph: &Graph, phase: &HashMap<String, NodePhase>) -> &'static str {
    for node in &graph.nodes {
        if phase.get(&node.id) != Some(&NodePhase::Failed) {
            continue;
        }
        let handled = graph
            .edges
            .iter()
            .any(|edge| edge.from == node.id && matches!(edge.when, When::Failure | When::Always));
        if !handled {
            return "failed";
        }
    }
    "successful"
}

pub fn settled(graph: &Graph, phase: &HashMap<String, NodePhase>) -> bool {
    graph.nodes.iter().all(|node| {
        phase
            .get(&node.id)
            .copied()
            .unwrap_or(NodePhase::Pending)
            .is_terminal()
    })
}

/// Runs and nodes per database batch. The lock is released between batches so
/// a long history does not stall another request.
const PRUNE_BATCH: i64 = 500;

/// Deletes finished workflow runs and their nodes past
/// `KOREOGRAF_WORKFLOW_RUN_RETENTION_DAYS`.
///
/// Only a run with a finished status and a `finished_at` older than the
/// cutoff qualifies. The loop takes the database lock one bounded batch at a
/// time, and the log line carries the totals for the whole pass.
pub(crate) fn prune_finished_workflow_runs(state: &crate::AppState) -> AppResult<()> {
    let days = state.cfg.workflow_run_retention_days;
    if days == 0 {
        return Ok(());
    }
    let Some(cutoff) = db::minus_days(days) else {
        return Ok(());
    };
    let mut runs = 0usize;
    let mut nodes = 0usize;
    loop {
        let batch = state.store().prune_workflow_runs(&cutoff, PRUNE_BATCH)?;
        if batch.run_ids.is_empty() {
            break;
        }
        runs = runs.saturating_add(batch.run_ids.len());
        nodes = nodes.saturating_add(batch.nodes);
        if batch.run_ids.len() < usize::try_from(PRUNE_BATCH).unwrap_or(usize::MAX) {
            break;
        }
        // Leave a gap for another request before taking the lock again.
        std::thread::yield_now();
    }
    tracing::info!(
        days,
        runs,
        nodes,
        "pruned finished workflow runs; KOREOGRAF_WORKFLOW_RUN_RETENTION_DAYS sets the cutoff"
    );
    Ok(())
}

/// Deliveries per team and batch for the hook delivery prune.
const DELIVERY_PRUNE_BATCH: i64 = 500;
/// Newest deliveries that one team keeps, independent of the day cutoff.
pub(crate) const HOOK_DELIVERY_KEEP_PER_TEAM: i64 = 1000;

/// Deletes inbound webhook deliveries older than
/// `KOREOGRAF_HOOK_DELIVERY_RETENTION_DAYS` and rows beyond the newest
/// [`HOOK_DELIVERY_KEEP_PER_TEAM`] of each team.
///
/// The loop takes the database lock one bounded batch at a time and releases
/// it between batches. The payload is never read or logged; only the removed
/// count reaches the log.
pub(crate) fn prune_hook_deliveries(state: &crate::AppState) -> AppResult<()> {
    let days = state.cfg.hook_delivery_retention_days;
    if days == 0 {
        return Ok(());
    }
    let Some(cutoff) = db::minus_days(days) else {
        return Ok(());
    };
    let mut removed = 0usize;
    loop {
        let batch = state.store().prune_hook_deliveries(
            &cutoff,
            HOOK_DELIVERY_KEEP_PER_TEAM,
            DELIVERY_PRUNE_BATCH,
        )?;
        removed = removed.saturating_add(batch);
        if batch < usize::try_from(DELIVERY_PRUNE_BATCH).unwrap_or(usize::MAX) {
            break;
        }
        // Leave a gap for another request before taking the lock again.
        std::thread::yield_now();
    }
    tracing::info!(
        days,
        deliveries = removed,
        per_team = HOOK_DELIVERY_KEEP_PER_TEAM,
        "pruned hook deliveries; KOREOGRAF_HOOK_DELIVERY_RETENTION_DAYS sets the cutoff"
    );
    Ok(())
}

/// Runs the hook delivery prune once when the task starts and then once a
/// day. The server calls this after the port is bound. Each pass runs on the
/// blocking pool and takes the database lock one batch at a time.
pub(crate) fn spawn_delivery_retention(state: Shared) {
    tokio::spawn(async move {
        loop {
            let task_state = state.clone();
            match tokio::task::spawn_blocking(move || prune_hook_deliveries(&task_state)).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::error!(error = %err, "hook delivery retention"),
                Err(err) => tracing::error!(error = %err, "hook delivery retention task stopped"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

/// Runs the workflow-run prune once when the task starts and then once a day.
/// The server calls this after the port is bound. Each pass runs on the
/// blocking pool and takes the database lock one batch at a time.
pub(crate) fn spawn_run_retention(state: Shared) {
    tokio::spawn(async move {
        loop {
            let task_state = state.clone();
            match tokio::task::spawn_blocking(move || prune_finished_workflow_runs(&task_state))
                .await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::error!(error = %err, "workflow run retention"),
                Err(err) => tracing::error!(error = %err, "workflow run retention task stopped"),
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
