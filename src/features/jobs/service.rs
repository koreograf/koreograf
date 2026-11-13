use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::{LocalJobs, RunnerMode};
use crate::db::{self, Hook, Job, WorkflowJob};
use crate::error::{ApiError, AppResult};
use crate::features::workflows::{self as workflow, NodePhase};
use crate::hooks::{self, AlertEvent};
use crate::runner;
use crate::Shared;

use super::model::{Dispatch, LaunchOverrides, LaunchRequest};

pub fn launch(state: &Shared, req: LaunchRequest) -> AppResult<String> {
    if state.in_maintenance() {
        return Err(ApiError::bad("maintenance_mode_is_active"));
    }
    let settings = crate::features::operations::load(state)?;
    let id = Uuid::new_v4().to_string();
    {
        let db = state.store();
        let template = db.template(&req.template_id)?.ok_or(ApiError::NotFound)?;
        if template.team_id != req.team_id {
            return Err(ApiError::Forbidden);
        }
        let local_tool = runner::local_tool_available(state, &template.tool);
        let controller_can_run =
            runner::controller_can_run(settings.runner, state.cfg.container_image, local_tool);
        let default_environment = settings.default_environment(&template.tool);
        let has_own_environment = template
            .environment_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
            || req
                .parameters_override
                .as_deref()
                .map(db::json_object)
                .and_then(|params| {
                    params
                        .get("environment_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .is_some_and(|id| !id.is_empty());
        if !controller_can_run && !has_own_environment && default_environment.is_empty() {
            return Err(ApiError::bad_vars(
                "no_default_execution_environment",
                [("tool", runner::tool_label(&template.tool))],
            ));
        }
        let parameters = parameters_for_launch(
            state,
            db,
            &template,
            &req,
            default_environment,
            controller_can_run,
        )?;
        let parsed = db::json_object(&parameters);
        {
            let workflow_id = workflow_definition_id(db, req.workflow_job_id.as_deref())?;
            let requires_workflow = req
                .workflow_job_id
                .as_deref()
                .is_some_and(|item| !item.is_empty());
            if local_launch_refused(
                refuses_local_launch(
                    state.cfg.local_jobs,
                    settings.runner,
                    parameter_has_environment(&template, &parsed),
                ),
                db,
                &template,
                &parsed,
                &req.launched_by,
                workflow_id.as_deref(),
                requires_workflow,
            )? {
                return Err(ApiError::bad("local_runs_are_limited_to_administrators"));
            }
        }
        let name = parsed
            .get("template_name")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .unwrap_or(&template.name)
            .to_string();
        let tool = parsed
            .get("tool")
            .and_then(Value::as_str)
            .filter(|text| runner::known_tool(text))
            .unwrap_or(&template.tool)
            .to_string();
        if tool == "ansible" {
            let has_inventory = parsed
                .get("inventory_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty());
            if !has_inventory {
                return Err(ApiError::bad("inventory_is_required"));
            }
        }
        let job = Job {
            id: id.clone(),
            team_id: req.team_id,
            template_id: Some(template.id),
            workflow_job_id: req.workflow_job_id,
            workflow_node_id: req.workflow_node_id,
            name,
            tool,
            status: "pending".into(),
            extra_vars: serde_json::to_string(&req.extra_vars).unwrap_or_else(|_| "{}".into()),
            launched_by: req.launched_by,
            trigger: req.trigger,
            started_at: None,
            finished_at: None,
            created_at: db::now(),
            parameters,
            failed_hosts: "[]".into(),
        };
        db.insert_job(&job)?;
    }
    let shared = state.clone();
    let job_id = id.clone();
    tokio::spawn(async move {
        runner::execute(shared, job_id).await;
    });
    Ok(id)
}

fn refuses_local_launch(local_jobs: LocalJobs, runner: RunnerMode, has_environment: bool) -> bool {
    local_jobs == LocalJobs::Admins && (!has_environment || runner == RunnerMode::Local)
}

/// True when the local-run restriction refuses this launch. Hooks use it to
/// reject a delivery before any run starts, with the same rules as [`launch`].
fn local_launch_refused(
    refused_by_policy: bool,
    store: &dyn crate::backend::Backend,
    template: &db::Template,
    parameters: &Value,
    launched_by: &str,
    workflow_id: Option<&str>,
    requires_workflow: bool,
) -> AppResult<bool> {
    if !refused_by_policy {
        return Ok(false);
    }
    let matches = crate::features::identity::command_matches_template(template, parameters);
    let requires_workflow = requires_workflow || !matches;
    let effective = template_with_saved_targets(template, parameters);
    let allowed = crate::features::identity::launcher_is_admin(store, launched_by)?
        && crate::features::identity::content_saved_by_admin(
            store,
            &effective,
            workflow_id,
            requires_workflow,
        )?;
    Ok(!allowed)
}

pub fn relaunch(
    state: &Shared,
    source_id: &str,
    launched_by: &str,
    only_failed_hosts: bool,
) -> AppResult<String> {
    let settings = crate::features::operations::load(state)?;
    let (team_id, template_id, extra, parameters, workflow_job_id) = {
        let db = state.store();
        let job = db.job(source_id)?.ok_or(ApiError::NotFound)?;
        if job.status != "failed" && job.status != "canceled" {
            return Err(ApiError::bad(
                "only_a_failed_or_canceled_job_can_be_relaunched",
            ));
        }
        let template_id = job
            .template_id
            .clone()
            .ok_or_else(|| ApiError::bad("this_job_s_template_is_missing"))?;
        let template = db
            .template(&template_id)?
            .ok_or_else(|| ApiError::bad("this_job_s_template_is_missing"))?;
        if template.team_id != job.team_id {
            return Err(ApiError::Forbidden);
        }
        let local_tool = runner::local_tool_available(state, &template.tool);
        let controller_can_run =
            runner::controller_can_run(settings.runner, state.cfg.container_image, local_tool);
        let mut parameters = if db::json_object(&job.parameters)
            .get("tool")
            .and_then(Value::as_str)
            .is_some()
        {
            job.parameters.clone()
        } else {
            capture_parameters(
                db,
                &template,
                &db::json_object(&job.extra_vars),
                None,
                settings.default_environment(&template.tool),
                controller_can_run,
            )
        };
        let mut parsed = db::json_object(&parameters);
        if let Some(map) = parsed.as_object_mut() {
            map.remove("project_commit");
            map.remove("project_commit_full");
            refresh_inventory(db, &job.team_id, map)?;
            refresh_credential(db, &job.team_id, map)?;
            parameters = parsed.to_string();
        }
        if only_failed_hosts {
            if job.tool != "ansible" {
                return Err(ApiError::bad("only_ansible_jobs_have_failed_hosts"));
            }
            let hosts = runner::failed_host_names(&job.failed_hosts);
            let limit = runner::limit_from_hosts(&hosts).map_err(ApiError::from)?;
            parameters =
                runner::parameters_with_limit(&parameters, &limit).map_err(ApiError::from)?;
        }
        let differs = !crate::features::identity::command_matches_template(
            &template,
            &db::json_object(&parameters),
        );
        // A stored command that is not the current template stays tied to the
        // workflow that saved it, so the local-run check can see that release.
        let workflow_job_id = if differs {
            job.workflow_job_id.clone()
        } else {
            None
        };
        (
            job.team_id,
            template_id,
            db::json_object(&job.extra_vars),
            parameters,
            workflow_job_id,
        )
    };
    launch(
        state,
        LaunchRequest {
            team_id,
            template_id,
            extra_vars: extra,
            launched_by: launched_by.to_string(),
            trigger: "relaunch".into(),
            workflow_job_id,
            workflow_node_id: None,
            parameters_override: Some(parameters),
            overrides: LaunchOverrides::default(),
        },
    )
}

fn parameter_has_environment(template: &db::Template, params: &Value) -> bool {
    if params.get("tool").and_then(Value::as_str).is_some() {
        return params
            .get("environment_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty());
    }
    template.environment_id.is_some()
}

fn template_with_saved_targets(template: &db::Template, params: &Value) -> db::Template {
    if params.get("tool").and_then(Value::as_str).is_none() {
        return template.clone();
    }
    let mut effective = template.clone();
    effective.project_id = params
        .get("project_id")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    effective.inventory_id = params
        .get("inventory_id")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    effective
}

fn workflow_definition_id(
    store: &dyn crate::backend::Backend,
    workflow_job_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(id) = workflow_job_id.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    Ok(store.workflow_job(id)?.and_then(|run| run.workflow_id))
}

fn stored_parameters(raw: &str) -> AppResult<String> {
    if raw.len() > 512 * 1024 {
        return Err(ApiError::bad("parameters_are_too_large"));
    }
    let value: Value =
        serde_json::from_str(raw).map_err(|_| ApiError::bad("parameters_are_invalid"))?;
    if !value.is_object() {
        return Err(ApiError::bad("parameters_are_invalid"));
    }
    Ok(value.to_string())
}

fn parameters_for_node(
    db: &dyn crate::backend::Backend,
    template: &db::Template,
    node: &workflow::GraphNode,
    workflow_extra: &Value,
    default_environment: &str,
    controller_can_run: bool,
) -> AppResult<String> {
    let mut params = db::json_object(&capture_parameters(
        db,
        template,
        workflow_extra,
        None,
        default_environment,
        controller_can_run,
    ));
    let had_extra = node.overrides.extra_vars.is_some();
    let had_spec = node.overrides.spec.is_some();
    {
        let Some(map) = params.as_object_mut() else {
            return Err(ApiError::bad("parameters_are_invalid"));
        };
        workflow::apply_overrides(map, &node.overrides);
        if had_extra {
            let base = map.get("extra_vars").cloned().unwrap_or_else(|| json!({}));
            map.insert(
                "extra_vars".into(),
                runner::merge_vars(&base, workflow_extra),
            );
        }
        if node.overrides.inventory_id.is_some() {
            refresh_inventory(db, &template.team_id, map)?;
        }
        if node.overrides.credential_id.is_some() {
            refresh_credential(db, &template.team_id, map)?;
        }
    }
    if had_spec && template.tool == "ansible" {
        let spec = params.get("spec").cloned().unwrap_or_else(|| json!({}));
        runner::check_ansible_spec(&spec).map_err(ApiError::from)?;
    }
    if had_spec && template.tool == "opentofu" {
        if let Some(action) = params
            .get("spec")
            .and_then(|spec| spec.get("action"))
            .and_then(Value::as_str)
        {
            if !matches!(action, "plan" | "apply") {
                return Err(ApiError::bad("action_is_plan_or_apply"));
            }
        }
    }
    let raw = serde_json::to_string(&params).map_err(|err| ApiError::Internal(err.to_string()))?;
    if raw.len() > 512 * 1024 {
        return Err(ApiError::bad("parameters_are_too_large"));
    }
    Ok(raw)
}

fn refresh_inventory(
    db: &dyn crate::backend::Backend,
    team_id: &str,
    params: &mut serde_json::Map<String, Value>,
) -> AppResult<()> {
    match params.get("inventory_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => {
            let inventory = db.inventory(id)?.ok_or(ApiError::NotFound)?;
            if inventory.team_id != team_id {
                return Err(ApiError::NotFound);
            }
            params.insert("inventory_id".into(), json!(inventory.id));
            params.insert("inventory_name".into(), json!(inventory.name));
        }
        _ => {
            params.insert("inventory_id".into(), Value::Null);
            params.insert("inventory_name".into(), Value::Null);
        }
    }
    Ok(())
}

fn refresh_credential(
    db: &dyn crate::backend::Backend,
    team_id: &str,
    params: &mut serde_json::Map<String, Value>,
) -> AppResult<()> {
    let chosen = params
        .get("credential_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let Some(id) = chosen else {
        params.insert("credential_id".into(), Value::Null);
        params.insert("credential_name".into(), Value::Null);
        params.insert("credential_kind".into(), Value::Null);
        params.insert("credentials".into(), json!([]));
        return Ok(());
    };
    let credential = db.credential(&id)?.ok_or(ApiError::NotFound)?;
    if credential.team_id != team_id {
        return Err(ApiError::NotFound);
    }
    let mut list = params
        .get("credentials")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    replace_credential_kind(&mut list, &credential);
    let primary = list.iter().find_map(|item| {
        let kind = item.get("kind").and_then(Value::as_str)?;
        if kind == "vault" {
            return None;
        }
        Some((
            item.get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            item.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            kind.to_string(),
        ))
    });
    let (primary_id, primary_name, primary_kind) = primary.unwrap_or_else(|| {
        (
            credential.id.clone(),
            credential.name.clone(),
            credential.kind.clone(),
        )
    });
    params.insert("credential_id".into(), json!(primary_id));
    params.insert("credential_name".into(), json!(primary_name));
    params.insert("credential_kind".into(), json!(primary_kind));
    params.insert("credentials".into(), Value::Array(list));
    Ok(())
}

fn credential_entry(credential: &db::Credential) -> Value {
    json!({
        "id": credential.id,
        "name": credential.name,
        "kind": credential.kind,
    })
}

fn replace_credential_kind(list: &mut Vec<Value>, credential: &db::Credential) {
    list.retain(|item| item.get("kind").and_then(Value::as_str) != Some(credential.kind.as_str()));
    list.push(credential_entry(credential));
}

fn parameters_for_launch(
    state: &Shared,
    db: &dyn crate::backend::Backend,
    template: &db::Template,
    req: &LaunchRequest,
    default_environment: &str,
    controller_can_run: bool,
) -> AppResult<String> {
    if let Some(raw) = &req.parameters_override {
        let stored = stored_parameters(raw)?;
        if controller_can_run {
            return Ok(stored);
        }
        let mut params = db::json_object(&stored);
        let has_environment = params
            .get("environment_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty());
        if !has_environment && !default_environment.is_empty() {
            if let Some(item) = db.environment(default_environment)? {
                if let Some(map) = params.as_object_mut() {
                    map.insert("environment_id".into(), json!(item.id));
                    map.insert("environment_name".into(), json!(item.name));
                    map.insert("environment_image".into(), json!(item.image));
                    map.insert("no_environment".into(), Value::Bool(false));
                }
            }
        }
        return Ok(params.to_string());
    }
    if let Some(spec) = &req.overrides.spec {
        if !spec.is_object() {
            return Err(ApiError::bad("spec_must_be_an_object"));
        }
    }
    let spec_overlay = req
        .overrides
        .spec
        .as_ref()
        .filter(|spec| spec.as_object().is_some_and(|map| !map.is_empty()));
    let extra_overlay = req.overrides.extra_vars_are_overrides
        && req
            .extra_vars
            .as_object()
            .is_some_and(|map| !map.is_empty());
    let asked = req.overrides.inventory_id.is_some()
        || req.overrides.credential_ids.is_some()
        || spec_overlay.is_some()
        || extra_overlay;
    if !asked {
        return Ok(capture_parameters(
            db,
            template,
            &req.extra_vars,
            None,
            default_environment,
            controller_can_run,
        ));
    }
    if !template.prompt_on_launch && !req.overrides.launch_with_options {
        return Err(ApiError::bad("prompt_on_launch_is_off"));
    }
    let mut effective = template.clone();
    if let Some(inventory_id) = &req.overrides.inventory_id {
        let inventory_id = blank_text(inventory_id);
        if let Some(id) = &inventory_id {
            let inventory = db.inventory(id)?.ok_or(ApiError::NotFound)?;
            if inventory.team_id != template.team_id {
                return Err(ApiError::NotFound);
            }
        }
        effective.inventory_id = inventory_id;
    }
    let credential_ids = if let Some(ids) = &req.overrides.credential_ids {
        let chosen = credentials_for_launch(state, db, &template.team_id, &template.tool, ids)?;
        effective.credential_id = chosen.primary_id;
        Some(chosen.ids)
    } else {
        None
    };
    let mut params = db::json_object(&capture_parameters(
        db,
        &effective,
        &req.extra_vars,
        credential_ids.as_deref(),
        default_environment,
        controller_can_run,
    ));
    if let Some(spec) = spec_overlay {
        if template.tool != "ansible" {
            return Err(ApiError::bad("launch_fields_are_only_for_ansible"));
        }
        let empty = json!({});
        let base = params.get("spec").unwrap_or(&empty);
        let merged = merge_prompt_spec(base, spec).map_err(ApiError::bad)?;
        runner::check_ansible_spec(&merged).map_err(ApiError::from)?;
        let Some(map) = params.as_object_mut() else {
            return Err(ApiError::bad("parameters_are_invalid"));
        };
        map.insert("spec".into(), merged);
    }
    if let Some(map) = params.as_object_mut() {
        map.insert("launch_with_options".into(), Value::Bool(true));
    }
    Ok(params.to_string())
}

fn blank_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn credentials_for_launch(
    state: &Shared,
    db: &dyn crate::backend::Backend,
    team_id: &str,
    tool: &str,
    ids: &[String],
) -> AppResult<crate::features::credentials::AssignedSet> {
    let mut loaded = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            return Err(ApiError::bad("credential_is_missing"));
        }
        let credential = db.credential(id)?.ok_or(ApiError::NotFound)?;
        loaded.push(credential);
    }
    let chosen = crate::features::credentials::assign_loaded(state, team_id, &loaded)?;
    for credential in &loaded {
        if !crate::features::credentials::kind_fits_tool(tool, &credential.kind) {
            return Err(ApiError::bad("this_credential_kind_does_not_fit_the_tool"));
        }
    }
    Ok(chosen)
}

/// Prompt fields replace the template spec. An empty string or null clears the key.
fn merge_prompt_spec(base: &Value, overlay: &Value) -> Result<Value, String> {
    let Some(incoming) = overlay.as_object() else {
        return Err("spec_must_be_an_object".into());
    };
    let mut spec = base.as_object().cloned().unwrap_or_default();
    for (key, value) in incoming {
        if !matches!(
            key.as_str(),
            "job_type" | "tags" | "skip_tags" | "limit" | "verbosity"
        ) {
            return Err("launch_fields_are_only_for_ansible".into());
        }
        let clear = match value {
            Value::Null => true,
            Value::String(text) if text.trim().is_empty() => true,
            Value::Number(number) if key == "verbosity" && number.as_u64() == Some(0) => true,
            _ => false,
        };
        if clear {
            spec.remove(key);
        } else {
            spec.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::Object(spec))
}

fn capture_parameters(
    db: &dyn crate::backend::Backend,
    template: &db::Template,
    request_extra: &Value,
    credential_override: Option<&[String]>,
    default_environment: &str,
    controller_can_run: bool,
) -> String {
    let project_name = template
        .project_id
        .as_deref()
        .and_then(|id| db.project(id).ok().flatten().map(|item| item.name));
    let inventory_name = template
        .inventory_id
        .as_deref()
        .and_then(|id| db.inventory(id).ok().flatten().map(|item| item.name));
    let credential = template
        .credential_id
        .as_deref()
        .and_then(|id| db.credential(id).ok().flatten())
        .map(|item| (item.name, item.kind));
    let linked = if let Some(ids) = credential_override {
        ids.to_vec()
    } else {
        let linked = db
            .template_credential_ids(&template.id)
            .ok()
            .unwrap_or_default();
        if linked.is_empty() {
            template
                .credential_id
                .clone()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            linked
        }
    };
    let credentials = linked
        .iter()
        .filter_map(|id| db.credential(id).ok().flatten())
        .map(|item| credential_entry(&item))
        .collect::<Vec<_>>();
    let environment_id =
        runner::resolved_environment_id(template, default_environment, controller_can_run);
    let environment = environment_id
        .as_deref()
        .and_then(|id| db.environment(id).ok().flatten())
        .map(|item| (item.name, item.image));
    json!({
        "template_id": template.id,
        "template_name": template.name,
        "tool": template.tool,
        "entrypoint": template.entrypoint,
        "project_id": template.project_id,
        "project_name": project_name,
        "inventory_id": template.inventory_id,
        "inventory_name": inventory_name,
        "credential_id": template.credential_id,
        "credential_name": credential.as_ref().map(|(name, _)| name),
        "credential_kind": credential.as_ref().map(|(_, kind)| kind),
        "credentials": credentials,
        "environment_id": environment_id,
        "no_environment": template.no_environment,
        "environment_name": environment.as_ref().map(|(name, _)| name),
        "environment_image": environment.as_ref().map(|(_, image)| image),
        "extra_vars": runner::merge_vars(&db::json_object(&template.extra_vars), request_extra),
        "spec": db::json_object(&template.spec),
        "update_on_launch": template.update_on_launch,
    })
    .to_string()
}

pub fn discard_project_checkout(state: &Shared, project: &db::Project) {
    let dest = state.cfg.data_dir.join("checkouts").join(&project.id);
    let _ = std::fs::remove_dir_all(dest);
}

pub async fn sync_project(state: &Shared, project: &db::Project) -> AppResult<String> {
    if project.source_type != "git" {
        return Ok(project.source.clone());
    }
    let proxy = crate::features::operations::load(state)?.proxy;
    let credential = runner::project_credential_secret(state, project)
        .map_err(|_| ApiError::bad("the_source_control_credential_cannot_be_read"))?;
    let result = runner::materialize_project(
        project,
        &state.cfg.data_dir,
        &proxy,
        true,
        &state.cfg.project_roots,
        credential.as_ref(),
    )
    .await;
    match result {
        Ok(path) => {
            let commit = runner::head_commit(&path).await;
            state.store().set_project_sync_success(
                &project.id,
                &db::now(),
                commit.as_deref().unwrap_or(""),
            )?;
            Ok(path.display().to_string())
        }
        Err(err) => {
            state.store().set_project_sync_failed(&project.id)?;
            Err(ApiError::from(err))
        }
    }
}

pub fn cancel(state: &Shared, job_id: &str) -> AppResult<()> {
    let unstarted = {
        let db = state.store();
        let job = db.job(job_id)?.ok_or(ApiError::NotFound)?;
        if matches!(job.status.as_str(), "successful" | "failed" | "canceled") {
            return Ok(());
        }
        let changed = db.finish_job(job_id, "canceled")?;
        db.insert_event(job_id, "system", "Cancel requested.")?;
        changed.then(|| crate::features::operations::job_labels(&job))
    };
    if let Some(labels) = unstarted {
        let meta = crate::features::operations::JobMeta {
            job_id,
            team_id: &labels.team_id,
            tool: &labels.tool,
            playbook: &labels.playbook,
            template: &labels.template,
        };
        // A running observation stays open so the runner can close it once.
        state.metrics.finish_unstarted(&meta, "canceled");
    }
    // After the database lock. The runner reads this set instead of the job row.
    if let Ok(mut canceled) = state.canceled.lock() {
        canceled.insert(job_id.to_string());
    }
    let running = state
        .procs
        .lock()
        .ok()
        .and_then(|procs| procs.get(job_id).cloned());
    if let Some(proc) = running {
        runner::signal_stop(state, job_id, proc);
    } else {
        // The process id may appear a moment later. A second signal is cheap.
        let state = state.clone();
        let job_id = job_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Some(proc) = state
                .procs
                .lock()
                .ok()
                .and_then(|procs| procs.get(&job_id).cloned())
            {
                runner::signal_stop(&state, &job_id, proc);
            }
        });
    }
    Ok(())
}

pub fn launch_workflow(
    state: &Shared,
    team_id: &str,
    workflow_id: &str,
    extra_vars: Value,
    launched_by: &str,
    trigger: &str,
) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    {
        let db = state.store();
        let item = db.workflow(workflow_id)?.ok_or(ApiError::NotFound)?;
        if item.team_id != team_id {
            return Err(ApiError::Forbidden);
        }
        let graph: workflow::Graph = serde_json::from_str(&item.graph)
            .map_err(|_| ApiError::bad("the_workflow_graph_is_invalid"))?;
        if graph.nodes.is_empty() {
            return Err(ApiError::bad("the_workflow_has_no_nodes_yet"));
        }
        workflow::validate(&graph).map_err(ApiError::bad)?;
        for node in &graph.nodes {
            let template = db.template(&node.template_id)?.ok_or_else(|| {
                ApiError::bad_vars("template.missing_named", [("name", node.name.as_str())])
            })?;
            if template.team_id != team_id {
                return Err(ApiError::bad_vars(
                    "template.other_team_named",
                    [("name", node.name.as_str())],
                ));
            }
        }
        let run = WorkflowJob {
            id: id.clone(),
            workflow_id: Some(item.id),
            team_id: team_id.to_string(),
            name: item.name,
            status: "running".into(),
            extra_vars: serde_json::to_string(&extra_vars).unwrap_or_else(|_| "{}".into()),
            launched_by: launched_by.to_string(),
            trigger: trigger.to_string(),
            created_at: db::now(),
            finished_at: None,
        };
        db.insert_workflow_job(&run)?;
        for node in &graph.nodes {
            db.insert_workflow_node(&id, &node.id)?;
        }
    }
    let shared = state.clone();
    let run_id = id.clone();
    tokio::spawn(async move {
        if let Err(err) = drive(shared.clone(), run_id.clone()).await {
            tracing::error!(error = %err, workflow_job = %run_id, "workflow");
            let _ = shared.store().set_workflow_status(&run_id, "failed", true);
        }
    });
    Ok(id)
}

async fn drive(state: Shared, wf_job_id: String) -> AppResult<()> {
    let (graph, team_id, extra) = {
        let db = state.store();
        let run = db.workflow_job(&wf_job_id)?.ok_or(ApiError::NotFound)?;
        let item = db
            .workflow(run.workflow_id.as_deref().unwrap_or(""))?
            .ok_or(ApiError::NotFound)?;
        let graph: workflow::Graph = serde_json::from_str(&item.graph)
            .map_err(|_| ApiError::bad("the_workflow_graph_is_invalid"))?;
        (graph, run.team_id, db::json_object(&run.extra_vars))
    };
    for _ in 0..4_000 {
        let phase = refresh(&state, &wf_job_id, &graph)?;
        if workflow::settled(&graph, &phase) {
            let status = workflow::final_status(&graph, &phase);
            state
                .store()
                .set_workflow_status(&wf_job_id, status, true)?;
            return Ok(());
        }
        let steps = workflow::advance(&graph, &phase);
        if steps.is_empty() {
            let running = phase.values().any(|item| *item == NodePhase::Running);
            if !running {
                state
                    .store()
                    .set_workflow_status(&wf_job_id, "failed", true)?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        }
        for step in steps {
            apply_step(&state, &wf_job_id, &graph, &team_id, &extra, step)?;
        }
    }
    state
        .store()
        .set_workflow_status(&wf_job_id, "failed", true)?;
    Ok(())
}

fn refresh(
    state: &Shared,
    wf_job_id: &str,
    graph: &workflow::Graph,
) -> AppResult<HashMap<String, NodePhase>> {
    let db = state.store();
    let rows = db.workflow_nodes(wf_job_id)?;
    let mut phase = HashMap::new();
    for row in rows {
        let mut current = NodePhase::parse(&row.phase);
        if !current.is_terminal() {
            if let Some(job_id) = &row.job_id {
                if let Some(job) = db.job(job_id)? {
                    if matches!(job.status.as_str(), "successful" | "failed" | "canceled") {
                        current = NodePhase::parse(&job.status);
                        db.set_workflow_node(wf_job_id, &row.node_id, current.as_db(), None)?;
                    } else {
                        current = NodePhase::Running;
                    }
                }
            }
        }
        phase.insert(row.node_id, current);
    }
    for node in &graph.nodes {
        phase.entry(node.id.clone()).or_insert(NodePhase::Pending);
    }
    Ok(phase)
}

fn apply_step(
    state: &Shared,
    wf_job_id: &str,
    graph: &workflow::Graph,
    team_id: &str,
    extra: &Value,
    step: workflow::Step,
) -> AppResult<()> {
    match step {
        workflow::Step::Skip(node_id) => {
            let db = state.store();
            db.set_workflow_node(wf_job_id, &node_id, NodePhase::Skipped.as_db(), None)?;
        }
        workflow::Step::Launch(node_id) => {
            let Some(node) = graph.nodes.iter().find(|item| item.id == node_id) else {
                return Ok(());
            };
            let prepared = {
                let settings = crate::features::operations::load(state)?;
                let db = state.store();
                match db.template(&node.template_id)? {
                    Some(template) if template.team_id == team_id => {
                        let local_tool = runner::local_tool_available(state, &template.tool);
                        let controller_can_run = runner::controller_can_run(
                            settings.runner,
                            state.cfg.container_image,
                            local_tool,
                        );
                        parameters_for_node(
                            db,
                            &template,
                            node,
                            extra,
                            settings.default_environment(&template.tool),
                            controller_can_run,
                        )
                    }
                    Some(_) => Err(ApiError::Forbidden),
                    None => Err(ApiError::NotFound),
                }
            };
            let launched = match prepared {
                Ok(parameters) => launch(
                    state,
                    LaunchRequest {
                        team_id: team_id.to_string(),
                        template_id: node.template_id.clone(),
                        extra_vars: extra.clone(),
                        launched_by: format!("workflow:{wf_job_id}"),
                        trigger: "workflow".into(),
                        workflow_job_id: Some(wf_job_id.to_string()),
                        workflow_node_id: Some(node_id.clone()),
                        parameters_override: Some(parameters),
                        overrides: LaunchOverrides::default(),
                    },
                ),
                Err(err) => {
                    tracing::warn!(error = %err, node = %node_id, "node parameters");
                    Err(err)
                }
            };
            let db = state.store();
            match launched {
                Ok(job_id) => {
                    let status = db
                        .job(&job_id)?
                        .map(|job| job.status)
                        .unwrap_or_else(|| "failed".into());
                    let phase = match status.as_str() {
                        "successful" => NodePhase::Success,
                        "failed" | "canceled" => NodePhase::Failed,
                        _ => NodePhase::Running,
                    };
                    db.set_workflow_node(wf_job_id, &node_id, phase.as_db(), Some(&job_id))?;
                }
                Err(_) => {
                    db.set_workflow_node(wf_job_id, &node_id, NodePhase::Failed.as_db(), None)?;
                }
            }
        }
    }
    Ok(())
}

/// True when a call to this workflow-targeted hook would be refused by the
/// local-run restriction. Every node is checked the way [`launch`] checks a job
/// so the delivery can be rejected before `launch_workflow` starts a run. A
/// graph that [`launch_workflow`] would reject on its own (missing workflow,
/// invalid graph, missing or foreign template, bad parameters) is left to it.
fn workflow_hook_refuses_local_run(
    state: &Shared,
    team_id: &str,
    workflow_id: &str,
    launched_by: &str,
    extra: &Value,
) -> AppResult<bool> {
    if state.cfg.local_jobs != LocalJobs::Admins {
        return Ok(false);
    }
    let settings = crate::features::operations::load(state)?;
    let db = state.store();
    let item = match db.workflow(workflow_id)? {
        Some(item) => item,
        None => return Ok(false),
    };
    if item.team_id != team_id {
        return Ok(false);
    }
    let graph: workflow::Graph = match serde_json::from_str(&item.graph) {
        Ok(graph) => graph,
        Err(_) => return Ok(false),
    };
    for node in &graph.nodes {
        let Some(template) = db.template(&node.template_id)? else {
            continue;
        };
        if template.team_id != team_id {
            continue;
        }
        let local_tool = runner::local_tool_available(state, &template.tool);
        let controller_can_run =
            runner::controller_can_run(settings.runner, state.cfg.container_image, local_tool);
        let parameters = match parameters_for_node(
            db,
            &template,
            node,
            extra,
            settings.default_environment(&template.tool),
            controller_can_run,
        ) {
            Ok(parameters) => parameters,
            Err(_) => continue,
        };
        let parsed = db::json_object(&parameters);
        if local_launch_refused(
            refuses_local_launch(
                state.cfg.local_jobs,
                settings.runner,
                parameter_has_environment(&template, &parsed),
            ),
            db,
            &template,
            &parsed,
            launched_by,
            Some(workflow_id),
            true,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn accept_hook(
    state: &Shared,
    hook: &Hook,
    payload: &Value,
    actor: Option<&str>,
) -> AppResult<Dispatch> {
    let events = hooks::parse_webhook(payload);
    let matched: Vec<&AlertEvent> = events
        .iter()
        .filter(|event| hooks::matches(&hook.match_alertname, &hook.match_status, event))
        .take(10)
        .collect();
    let payload_text = serde_json::to_string(payload).unwrap_or_else(|_| "{}".into());
    let payload_text = truncate(&payload_text, 16_000);
    let marker = actor
        .filter(|name| !name.is_empty())
        .map(|name| format!("Test, run as {name}: "))
        .unwrap_or_default();
    if !hook.enabled {
        let summary = format!("{marker}Hook is disabled.");
        let id = Uuid::new_v4().to_string();
        state.store().insert_delivery(&db::NewDelivery {
            id: &id,
            hook_id: &hook.id,
            status: "ignored",
            summary: &summary,
            payload: &payload_text,
            job_id: None,
            workflow_job_id: None,
        })?;
        return Ok(Dispatch {
            status: "ignored".into(),
            summary,
            job_id: None,
            workflow_job_id: None,
        });
    }
    if matched.is_empty() {
        let id = Uuid::new_v4().to_string();
        let summary = if events.is_empty() {
            "No alerts found in the payload.".to_string()
        } else {
            format!(
                "No alert matches {} / {}.",
                hook.match_alertname, hook.match_status
            )
        };
        let summary = format!("{marker}{summary}");
        state.store().insert_delivery(&db::NewDelivery {
            id: &id,
            hook_id: &hook.id,
            status: "ignored",
            summary: &summary,
            payload: &payload_text,
            job_id: None,
            workflow_job_id: None,
        })?;
        return Ok(Dispatch {
            status: "ignored".into(),
            summary,
            job_id: None,
            workflow_job_id: None,
        });
    }
    let hook_vars = db::json_object(&hook.extra_vars);
    let launched_by = match actor {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => format!("hook:{}", hook.name),
    };
    let mut first_job = None;
    let mut first_workflow = None;
    let mut local_run_checked = false;
    for event in &matched {
        let extra = runner::merge_vars(&hook_vars, &hooks::event_vars(event));
        if let Some(workflow_id) = &hook.workflow_id {
            if !local_run_checked {
                local_run_checked = true;
                if workflow_hook_refuses_local_run(
                    state,
                    &hook.team_id,
                    workflow_id,
                    &launched_by,
                    &extra,
                )? {
                    let summary = format!(
                        "{marker}Local runs are limited to administrators. Set KOREOGRAF_LOCAL_RUNNER_FOR=all on a trusted network or attach an execution environment with the Docker socket."
                    );
                    let id = Uuid::new_v4().to_string();
                    state.store().insert_delivery(&db::NewDelivery {
                        id: &id,
                        hook_id: &hook.id,
                        status: "rejected",
                        summary: &summary,
                        payload: &payload_text,
                        job_id: None,
                        workflow_job_id: None,
                    })?;
                    return Err(ApiError::bad("local_runs_are_limited_to_administrators"));
                }
            }
            let id = launch_workflow(
                state,
                &hook.team_id,
                workflow_id,
                extra,
                &launched_by,
                "hook",
            )?;
            if first_workflow.is_none() {
                first_workflow = Some(id);
            }
        } else if let Some(template_id) = &hook.template_id {
            let id = match launch(
                state,
                LaunchRequest {
                    team_id: hook.team_id.clone(),
                    template_id: template_id.clone(),
                    extra_vars: extra,
                    launched_by: launched_by.clone(),
                    trigger: "hook".into(),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    parameters_override: None,
                    overrides: LaunchOverrides::default(),
                },
            ) {
                Ok(id) => id,
                Err(ApiError::BadRequest(message))
                    if message.key == "local_runs_are_limited_to_administrators" =>
                {
                    let summary = format!(
                        "{marker}Local runs are limited to administrators. Set KOREOGRAF_LOCAL_RUNNER_FOR=all on a trusted network or attach an execution environment with the Docker socket."
                    );
                    let id = Uuid::new_v4().to_string();
                    state.store().insert_delivery(&db::NewDelivery {
                        id: &id,
                        hook_id: &hook.id,
                        status: "rejected",
                        summary: &summary,
                        payload: &payload_text,
                        job_id: None,
                        workflow_job_id: None,
                    })?;
                    return Err(ApiError::bad("local_runs_are_limited_to_administrators"));
                }
                Err(error) => return Err(error),
            };
            if first_job.is_none() {
                first_job = Some(id);
            }
        } else {
            return Err(ApiError::bad("the_hook_has_no_target"));
        }
    }
    let summary = format!(
        "{marker}{} alert(s) started, first {}.",
        matched.len(),
        matched[0].alertname
    );
    let id = Uuid::new_v4().to_string();
    {
        state.store().insert_delivery(&db::NewDelivery {
            id: &id,
            hook_id: &hook.id,
            status: "accepted",
            summary: &summary,
            payload: &payload_text,
            job_id: first_job.as_deref(),
            workflow_job_id: first_workflow.as_deref(),
        })?;
    }
    Ok(Dispatch {
        status: "accepted".into(),
        summary,
        job_id: first_job,
        workflow_job_id: first_workflow,
    })
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        value.to_string()
    } else {
        let mut end = max;
        while !value.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        value[..end].to_string()
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
