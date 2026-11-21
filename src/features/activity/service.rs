use uuid::Uuid;

use crate::backend::Backend;
use crate::db;
use crate::error::AppResult;
use crate::Shared;

use super::model::Activity;

/// The activity log keeps the newest 10,000 entries of each team. Older rows
/// are pruned after every insert so the table stays bounded without a
/// background job. The bound is a row count, not an age.
pub(crate) const RETENTION: i64 = 10_000;

/// Appends one activity entry and prunes the team back to [`RETENTION`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    store: &dyn Backend,
    team_id: &str,
    actor: &str,
    action: &str,
    object_type: &str,
    object_id: &str,
    object_name: &str,
    result: &str,
) -> AppResult<()> {
    let item = Activity {
        id: Uuid::new_v4().to_string(),
        team_id: team_id.to_string(),
        actor: actor.to_string(),
        action: action.to_string(),
        object_type: object_type.to_string(),
        object_id: object_id.to_string(),
        object_name: object_name.to_string(),
        result: result.to_string(),
        created_at: db::now(),
    };
    store.insert_activity(&item)?;
    store.prune_activity(team_id, RETENTION)
}

/// Records an activity entry without failing the action that just happened.
/// A broken activity table must not turn a completed write into an error.
#[allow(clippy::too_many_arguments)]
pub(crate) fn note(
    state: &Shared,
    team_id: &str,
    actor: &str,
    action: &str,
    object_type: &str,
    object_id: &str,
    object_name: &str,
    result: &str,
) {
    if let Err(err) = record(
        state.store(),
        team_id,
        actor,
        action,
        object_type,
        object_id,
        object_name,
        result,
    ) {
        tracing::warn!(%team_id, %action, %object_type, error = %err, "activity entry");
    }
}
