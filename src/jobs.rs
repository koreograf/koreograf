use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::{LocalJobs, RunnerMode};
use crate::db::{self, Hook, Job, WorkflowJob};
use crate::error::{ApiError, AppResult};
use crate::hooks::{self, AlertEvent};
use crate::runner;
use crate::workflow::{self, NodePhase};
use crate::Shared;

/// Fields a prompted launch may replace for one job. `None` keeps the template.
#[derive(Default)]
pub struct LaunchOverrides {
    /// An empty string clears the inventory.
    pub inventory_id: Option<String>,
    /// Replaces the credential list, including an empty list.
    pub credential_ids: Option<Vec<String>>,
    /// Ansible fields merged onto the template spec.
    pub spec: Option<Value>,
}

pub struct LaunchRequest {
    pub team_id: String,
    pub template_id: String,
    pub extra_vars: Value,
    pub launched_by: String,
    pub trigger: String,
    pub workflow_job_id: Option<String>,
    pub workflow_node_id: Option<String>,
    /// When set, this is stored instead of a fresh snapshot of the template.
    pub parameters_override: Option<String>,
    pub overrides: LaunchOverrides,
}

pub fn launch(state: &Shared, req: LaunchRequest) -> AppResult<String> {
    let settings = crate::settings::load(state)?;
    let id = Uuid::new_v4().to_string();
    {
        let db = state.db()?;
        let template = db::template(&db, &req.template_id)?.ok_or(ApiError::NotFound)?;
        if template.team_id != req.team_id {
            return Err(ApiError::Forbidden);
        }
        let parameters = parameters_for_launch(state, &db, &template, &req)?;
        let parsed = db::json_object(&parameters);
        if refuses_local_launch(
            state.cfg.local_jobs,
            settings.runner,
            parameter_has_environment(&template, &parsed),
        ) {
            let workflow_id = workflow_definition_id(&db, req.workflow_job_id.as_deref())?;
            let matches = crate::auth::command_matches_template(&template, &parsed);
            let requires_workflow = req
                .workflow_job_id
                .as_deref()
                .is_some_and(|item| !item.is_empty())
                || !matches;
            let effective = template_with_saved_targets(&template, &parsed);
            let allowed = crate::auth::launcher_is_admin(&db, &req.launched_by)?
                && crate::auth::content_saved_by_admin(
                    &db,
                    &effective,
                    workflow_id.as_deref(),
                    requires_workflow,
                )?;
            if !allowed {
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
        db::insert_job(&db, &job)?;
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

pub fn relaunch(
    state: &Shared,
    source_id: &str,
    launched_by: &str,
    only_failed_hosts: bool,
) -> AppResult<String> {
    let (team_id, template_id, extra, parameters, workflow_job_id) = {
        let db = state.db()?;
        let job = db::job(&db, source_id)?.ok_or(ApiError::NotFound)?;
        if job.status != "failed" {
            return Err(ApiError::bad("only_a_failed_job_can_be_relaunched"));
        }
        let template_id = job
            .template_id
            .clone()
            .ok_or_else(|| ApiError::bad("this_job_s_template_is_missing"))?;
        let template = db::template(&db, &template_id)?
            .ok_or_else(|| ApiError::bad("this_job_s_template_is_missing"))?;
        if template.team_id != job.team_id {
            return Err(ApiError::Forbidden);
        }
        let mut parameters = if db::json_object(&job.parameters)
            .get("tool")
            .and_then(Value::as_str)
            .is_some()
        {
            job.parameters.clone()
        } else {
            capture_parameters(&db, &template, &db::json_object(&job.extra_vars), None)
        };
        if only_failed_hosts {
            if job.tool != "ansible" {
                return Err(ApiError::bad("only_ansible_jobs_have_failed_hosts"));
            }
            let hosts = runner::failed_host_names(&job.failed_hosts);
            let limit = runner::limit_from_hosts(&hosts).map_err(ApiError::bad)?;
            parameters =
                runner::parameters_with_limit(&parameters, &limit).map_err(ApiError::bad)?;
        }
        let differs =
            !crate::auth::command_matches_template(&template, &db::json_object(&parameters));
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
    db: &rusqlite::Connection,
    workflow_job_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(id) = workflow_job_id.filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    Ok(db::workflow_job(db, id)?.and_then(|run| run.workflow_id))
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
    db: &rusqlite::Connection,
    template: &db::Template,
    node: &workflow::GraphNode,
    workflow_extra: &Value,
) -> AppResult<String> {
    let mut params = db::json_object(&capture_parameters(db, template, workflow_extra, None));
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
        runner::check_ansible_spec(&spec).map_err(ApiError::bad)?;
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
    db: &rusqlite::Connection,
    team_id: &str,
    params: &mut serde_json::Map<String, Value>,
) -> AppResult<()> {
    match params.get("inventory_id").and_then(Value::as_str) {
        Some(id) if !id.is_empty() => {
            let inventory =
                db::inventory(db, id)?.ok_or_else(|| ApiError::bad("inventory_is_missing"))?;
            if inventory.team_id != team_id {
                return Err(ApiError::bad("the_inventory_belongs_to_another_team"));
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
    db: &rusqlite::Connection,
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
    let credential =
        db::credential(db, &id)?.ok_or_else(|| ApiError::bad("credential_is_missing"))?;
    if credential.team_id != team_id {
        return Err(ApiError::bad("the_credential_belongs_to_another_team"));
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
    db: &rusqlite::Connection,
    template: &db::Template,
    req: &LaunchRequest,
) -> AppResult<String> {
    if let Some(raw) = &req.parameters_override {
        return stored_parameters(raw);
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
    let asked = req.overrides.inventory_id.is_some()
        || req.overrides.credential_ids.is_some()
        || spec_overlay.is_some();
    if !asked {
        return Ok(capture_parameters(db, template, &req.extra_vars, None));
    }
    if !template.prompt_on_launch {
        return Err(ApiError::bad("prompt_on_launch_is_off"));
    }
    let mut effective = template.clone();
    if let Some(inventory_id) = &req.overrides.inventory_id {
        let inventory_id = blank_text(inventory_id);
        if let Some(id) = &inventory_id {
            let inventory =
                db::inventory(db, id)?.ok_or_else(|| ApiError::bad("inventory_is_missing"))?;
            if inventory.team_id != template.team_id {
                return Err(ApiError::bad("the_inventory_belongs_to_another_team"));
            }
        }
        effective.inventory_id = inventory_id;
    }
    let credential_ids = if let Some(ids) = &req.overrides.credential_ids {
        let chosen = credentials_for_launch(state, db, &template.team_id, ids)?;
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
    ));
    if let Some(spec) = spec_overlay {
        if template.tool != "ansible" {
            return Err(ApiError::bad("launch_fields_are_only_for_ansible"));
        }
        let empty = json!({});
        let base = params.get("spec").unwrap_or(&empty);
        let merged = merge_prompt_spec(base, spec).map_err(ApiError::bad)?;
        runner::check_ansible_spec(&merged).map_err(ApiError::bad)?;
        let Some(map) = params.as_object_mut() else {
            return Err(ApiError::bad("parameters_are_invalid"));
        };
        map.insert("spec".into(), merged);
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
    db: &rusqlite::Connection,
    team_id: &str,
    ids: &[String],
) -> AppResult<crate::credentials::AssignedSet> {
    let mut loaded = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id.trim();
        if id.is_empty() {
            return Err(ApiError::bad("credential_is_missing"));
        }
        let credential =
            db::credential(db, id)?.ok_or_else(|| ApiError::bad("credential_is_missing"))?;
        loaded.push(credential);
    }
    crate::credentials::assign_loaded(state, team_id, &loaded).map_err(ApiError::bad)
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
    db: &rusqlite::Connection,
    template: &db::Template,
    request_extra: &Value,
    credential_override: Option<&[String]>,
) -> String {
    let project_name = template
        .project_id
        .as_deref()
        .and_then(|id| db::project(db, id).ok().flatten().map(|item| item.name));
    let inventory_name = template
        .inventory_id
        .as_deref()
        .and_then(|id| db::inventory(db, id).ok().flatten().map(|item| item.name));
    let credential = template
        .credential_id
        .as_deref()
        .and_then(|id| db::credential(db, id).ok().flatten())
        .map(|item| (item.name, item.kind));
    let linked = if let Some(ids) = credential_override {
        ids.to_vec()
    } else {
        let linked = db::template_credential_ids(db, &template.id)
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
        .filter_map(|id| db::credential(db, id).ok().flatten())
        .map(|item| credential_entry(&item))
        .collect::<Vec<_>>();
    let environment = template
        .environment_id
        .as_deref()
        .and_then(|id| db::environment(db, id).ok().flatten())
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
        "environment_id": template.environment_id,
        "environment_name": environment.as_ref().map(|(name, _)| name),
        "environment_image": environment.as_ref().map(|(_, image)| image),
        "extra_vars": runner::merge_vars(&db::json_object(&template.extra_vars), request_extra),
        "spec": db::json_object(&template.spec),
        "update_on_launch": template.update_on_launch,
    })
    .to_string()
}

pub async fn sync_project(state: &Shared, project: &db::Project) -> AppResult<String> {
    if project.source_type != "git" {
        return Ok(project.source.clone());
    }
    let proxy = crate::settings::load(state)?.proxy;
    let path = runner::materialize_project(
        project,
        &state.cfg.data_dir,
        &proxy,
        true,
        &state.cfg.project_roots,
    )
    .await
    .map_err(ApiError::bad)?;
    Ok(path.display().to_string())
}

pub fn cancel(state: &Shared, job_id: &str) -> AppResult<()> {
    {
        let db = state.db()?;
        let job = db::job(&db, job_id)?.ok_or(ApiError::NotFound)?;
        if matches!(job.status.as_str(), "successful" | "failed" | "canceled") {
            return Ok(());
        }
        db::finish_job(&db, job_id, "canceled")?;
        db::insert_event(&db, job_id, "system", "Cancel requested.")?;
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
        let db = state.db()?;
        let item = db::workflow(&db, workflow_id)?.ok_or(ApiError::NotFound)?;
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
            let template = db::template(&db, &node.template_id)?.ok_or_else(|| {
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
        db::insert_workflow_job(&db, &run)?;
        for node in &graph.nodes {
            db::insert_workflow_node(&db, &id, &node.id)?;
        }
    }
    let shared = state.clone();
    let run_id = id.clone();
    tokio::spawn(async move {
        if let Err(err) = drive(shared.clone(), run_id.clone()).await {
            tracing::error!(error = %err, workflow_job = %run_id, "workflow");
            if let Ok(db) = shared.db() {
                let _ = db::set_workflow_status(&db, &run_id, "failed", true);
            }
        }
    });
    Ok(id)
}

async fn drive(state: Shared, wf_job_id: String) -> AppResult<()> {
    let (graph, team_id, extra) = {
        let db = state.db()?;
        let run = db::workflow_job(&db, &wf_job_id)?.ok_or(ApiError::NotFound)?;
        let item = db::workflow(&db, run.workflow_id.as_deref().unwrap_or(""))?
            .ok_or(ApiError::NotFound)?;
        let graph: workflow::Graph = serde_json::from_str(&item.graph)
            .map_err(|_| ApiError::bad("the_workflow_graph_is_invalid"))?;
        (graph, run.team_id, db::json_object(&run.extra_vars))
    };
    for _ in 0..4_000 {
        let phase = refresh(&state, &wf_job_id, &graph)?;
        if workflow::settled(&graph, &phase) {
            let status = workflow::final_status(&graph, &phase);
            let db = state.db()?;
            db::set_workflow_status(&db, &wf_job_id, status, true)?;
            return Ok(());
        }
        let steps = workflow::advance(&graph, &phase);
        if steps.is_empty() {
            let running = phase.values().any(|item| *item == NodePhase::Running);
            if !running {
                let db = state.db()?;
                db::set_workflow_status(&db, &wf_job_id, "failed", true)?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        }
        for step in steps {
            apply_step(&state, &wf_job_id, &graph, &team_id, &extra, step)?;
        }
    }
    let db = state.db()?;
    db::set_workflow_status(&db, &wf_job_id, "failed", true)?;
    Ok(())
}

fn refresh(
    state: &Shared,
    wf_job_id: &str,
    graph: &workflow::Graph,
) -> AppResult<HashMap<String, NodePhase>> {
    let db = state.db()?;
    let rows = db::workflow_nodes(&db, wf_job_id)?;
    let mut phase = HashMap::new();
    for row in rows {
        let mut current = NodePhase::parse(&row.phase);
        if !current.is_terminal() {
            if let Some(job_id) = &row.job_id {
                if let Some(job) = db::job(&db, job_id)? {
                    if matches!(job.status.as_str(), "successful" | "failed" | "canceled") {
                        current = NodePhase::parse(&job.status);
                        db::set_workflow_node(&db, wf_job_id, &row.node_id, current.as_db(), None)?;
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
            let db = state.db()?;
            db::set_workflow_node(&db, wf_job_id, &node_id, NodePhase::Skipped.as_db(), None)?;
        }
        workflow::Step::Launch(node_id) => {
            let Some(node) = graph.nodes.iter().find(|item| item.id == node_id) else {
                return Ok(());
            };
            let prepared = {
                let db = state.db()?;
                match db::template(&db, &node.template_id)? {
                    Some(template) if template.team_id == team_id => {
                        parameters_for_node(&db, &template, node, extra)
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
            let db = state.db()?;
            match launched {
                Ok(job_id) => {
                    let status = db::job(&db, &job_id)?
                        .map(|job| job.status)
                        .unwrap_or_else(|| "failed".into());
                    let phase = match status.as_str() {
                        "successful" => NodePhase::Success,
                        "failed" | "canceled" => NodePhase::Failed,
                        _ => NodePhase::Running,
                    };
                    db::set_workflow_node(&db, wf_job_id, &node_id, phase.as_db(), Some(&job_id))?;
                }
                Err(_) => {
                    db::set_workflow_node(
                        &db,
                        wf_job_id,
                        &node_id,
                        NodePhase::Failed.as_db(),
                        None,
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Dispatch {
    pub status: String,
    pub summary: String,
    pub job_id: Option<String>,
    pub workflow_job_id: Option<String>,
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
    if !hook.enabled {
        let id = Uuid::new_v4().to_string();
        let db = state.db()?;
        db::insert_delivery(
            &db,
            db::NewDelivery {
                id: &id,
                hook_id: &hook.id,
                status: "ignored",
                summary: "Hook is disabled.",
                payload: &payload_text,
                job_id: None,
                workflow_job_id: None,
            },
        )?;
        return Ok(Dispatch {
            status: "ignored".into(),
            summary: "Hook is disabled.".into(),
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
        let db = state.db()?;
        db::insert_delivery(
            &db,
            db::NewDelivery {
                id: &id,
                hook_id: &hook.id,
                status: "ignored",
                summary: &summary,
                payload: &payload_text,
                job_id: None,
                workflow_job_id: None,
            },
        )?;
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
    for event in &matched {
        let extra = runner::merge_vars(&hook_vars, &hooks::event_vars(event));
        if let Some(workflow_id) = &hook.workflow_id {
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
            let id = launch(
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
            )?;
            if first_job.is_none() {
                first_job = Some(id);
            }
        } else {
            return Err(ApiError::bad("the_hook_has_no_target"));
        }
    }
    let summary = format!(
        "{} alert(s) started, first {}.",
        matched.len(),
        matched[0].alertname
    );
    let id = Uuid::new_v4().to_string();
    {
        let db = state.db()?;
        db::insert_delivery(
            &db,
            db::NewDelivery {
                id: &id,
                hook_id: &hook.id,
                status: "accepted",
                summary: &summary,
                payload: &payload_text,
                job_id: first_job.as_deref(),
                workflow_job_id: first_workflow.as_deref(),
            },
        )?;
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
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    #[test]
    fn prompt_spec_replaces_listed_fields_and_clears_blanks() {
        let base =
            json!({"job_type": "run", "tags": "ping", "limit": "web", "forks": 5, "diff": true});
        let merged = merge_prompt_spec(
            &base,
            &json!({"limit": "db", "tags": "", "skip_tags": "slow", "job_type": "check", "verbosity": 0}),
        )
        .expect("merge");
        assert_eq!(merged["limit"], "db");
        assert_eq!(merged["job_type"], "check");
        assert_eq!(merged["skip_tags"], "slow");
        assert_eq!(merged["forks"], 5);
        assert_eq!(merged["diff"], true);
        assert!(merged.get("tags").is_none());
        assert!(merged.get("verbosity").is_none());
        let rejected = merge_prompt_spec(&base, &json!({"forks": 2})).expect_err("forks");
        assert_eq!(rejected, "launch_fields_are_only_for_ansible");
    }

    fn config(dir: &std::path::Path) -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.to_path_buf(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: crate::config::LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: crate::config::CookieSecure::Auto,
            trusted_proxy: false,
            ldap: None,
        }
    }

    #[tokio::test]
    async fn relaunch_copies_parameters_and_limits_failed_hosts() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-relaunch-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let (template_id, team_id) = {
            let db = state.db().expect("db");
            db.query_row(
                "SELECT id, team_id FROM templates WHERE name = 'Self-Heal'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .expect("template")
        };
        {
            let db = state.db().expect("db");
            db::insert_job(
                &db,
                &db::Job {
                    id: "src-failed".into(),
                    team_id,
                    template_id: Some(template_id),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    name: "Self-Heal".into(),
                    tool: "ansible".into(),
                    status: "failed".into(),
                    extra_vars: r#"{"alertname":"manual"}"#.into(),
                    launched_by: "admin".into(),
                    trigger: "manual".into(),
                    started_at: None,
                    finished_at: Some(db::now()),
                    created_at: db::now(),
                    parameters: json!({
                        "template_name": "Self-Heal",
                        "tool": "ansible",
                        "entrypoint": "missing.yml",
                        "extra_vars": {"alertname": "manual"},
                        "spec": {"tags": "heal"}
                    })
                    .to_string(),
                    failed_hosts: json!({"hosts": ["web-1", "db-1"], "recap": true}).to_string(),
                },
            )
            .expect("insert");
        }
        let copy = relaunch(&state, "src-failed", "admin", true).expect("relaunch");
        let params = {
            let db = state.db().expect("db");
            let job = db::job(&db, &copy).expect("read").expect("job");
            assert_eq!(job.trigger, "relaunch");
            assert_eq!(job.status, "pending");
            assert!(!job.parameters.contains("secret"));
            db::json_object(&job.parameters)
        };
        assert_eq!(params["spec"]["limit"], "web-1,db-1");
        assert_eq!(params["spec"]["tags"], "heal");
        assert_eq!(params["extra_vars"]["alertname"], "manual");
        {
            let db = state.db().expect("db");
            db.execute("UPDATE jobs SET tool = 'shell' WHERE id = 'src-failed'", [])
                .expect("tool");
            db.execute(
                "UPDATE jobs SET status = 'successful' WHERE id = ?1",
                [&copy],
            )
            .expect("status");
        }
        let shell = relaunch(&state, "src-failed", "admin", true).expect_err("shell");
        assert!(shell.to_string().contains("ansible"), "{shell}");
        let running = relaunch(&state, &copy, "admin", false).expect_err("not failed");
        assert!(
            running
                .to_string()
                .contains("only_a_failed_job_can_be_relaunched"),
            "{running}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn admins(dir: &std::path::Path, runner: RunnerMode) -> Config {
        let mut cfg = config(dir);
        cfg.local_jobs = crate::config::LocalJobs::Admins;
        cfg.runner = runner;
        cfg
    }

    fn team_of(state: &Shared) -> String {
        let db = state.db().expect("db");
        db.query_row("SELECT team_id FROM templates LIMIT 1", [], |row| {
            row.get(0)
        })
        .expect("team")
    }

    fn insert_shell(
        db: &rusqlite::Connection,
        team_id: &str,
        inventory_id: Option<&str>,
        script: &str,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        db::insert_template(
            db,
            &db::Template {
                id: id.clone(),
                team_id: team_id.to_string(),
                name: format!("Inline {id}"),
                description: String::new(),
                tool: "shell".into(),
                project_id: None,
                inventory_id: inventory_id.map(str::to_string),
                credential_id: None,
                environment_id: None,
                entrypoint: String::new(),
                extra_vars: "{}".into(),
                spec: json!({"script": script}).to_string(),
                created_at: db::now(),
                update_on_launch: false,
                admin_saved: true,
                prompt_on_launch: false,
            },
        )
        .expect("template");
        id
    }

    fn insert_inventory(db: &rusqlite::Connection, team_id: &str, admin_saved: bool) -> String {
        let id = Uuid::new_v4().to_string();
        db::insert_inventory(
            db,
            &db::Inventory {
                id: id.clone(),
                team_id: team_id.to_string(),
                name: format!("Hosts {id}"),
                body: "[all]\nlocalhost ansible_connection=local\n".into(),
                created_at: db::now(),
                source_kind: "ini".into(),
                source_json: "{}".into(),
                credential_id: None,
                admin_saved,
            },
        )
        .expect("inventory");
        id
    }

    fn insert_override_workflow(
        db: &rusqlite::Connection,
        team_id: &str,
        template_id: &str,
        overrides: Value,
        admin_saved: bool,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let graph = json!({
            "nodes": [{
                "id": "run",
                "template_id": template_id,
                "name": "Run",
                "overrides": overrides
            }],
            "edges": []
        });
        db::insert_workflow(
            db,
            &db::Workflow {
                id: id.clone(),
                team_id: team_id.to_string(),
                name: format!("Flow {id}"),
                description: String::new(),
                graph: graph.to_string(),
                created_at: db::now(),
                admin_saved,
            },
        )
        .expect("workflow");
        id
    }

    async fn wait_workflow(state: &Shared, id: &str) -> String {
        for _ in 0..50 {
            if let Ok(db) = state.db() {
                if let Ok(Some(run)) = db::workflow_job(&db, id) {
                    if matches!(run.status.as_str(), "successful" | "failed" | "canceled") {
                        return run.status;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        "timeout".into()
    }

    fn jobs_for(state: &Shared, template_id: &str) -> i64 {
        let db = state.db().expect("db");
        db.query_row(
            "SELECT COUNT(*) FROM jobs WHERE template_id = ?1",
            [template_id],
            |row| row.get(0),
        )
        .expect("count")
    }

    #[tokio::test]
    async fn an_operator_inventory_blocks_a_local_run() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-inv-release-{nanos}"));
        let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
        let team_id = team_of(&state);
        let inventory_id = {
            let db = state.db().expect("db");
            insert_inventory(&db, &team_id, false)
        };
        let template_id = {
            let db = state.db().expect("db");
            insert_shell(&db, &team_id, Some(&inventory_id), "true")
        };
        let denied = launch(
            &state,
            LaunchRequest {
                team_id: team_id.clone(),
                template_id: template_id.clone(),
                extra_vars: json!({}),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                workflow_job_id: None,
                workflow_node_id: None,
                parameters_override: None,
                overrides: LaunchOverrides::default(),
            },
        )
        .expect_err("operator inventory");
        assert!(
            denied
                .to_string()
                .contains("local_runs_are_limited_to_administrators"),
            "{denied}"
        );
        {
            let db = state.db().expect("db");
            db.execute(
                "UPDATE inventories SET admin_saved = 1 WHERE id = ?1",
                [&inventory_id],
            )
            .expect("release");
        }
        launch(
            &state,
            LaunchRequest {
                team_id,
                template_id,
                extra_vars: json!({}),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                workflow_job_id: None,
                workflow_node_id: None,
                parameters_override: None,
                overrides: LaunchOverrides::default(),
            },
        )
        .expect("admin inventory");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_operator_workflow_cannot_replace_a_local_shell_script() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-wf-shell-{nanos}"));
        let marker = dir.join("operator-script-ran");
        let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
        let team_id = team_of(&state);
        let template_id = {
            let db = state.db().expect("db");
            insert_shell(&db, &team_id, None, "true")
        };
        let workflow_id = {
            let db = state.db().expect("db");
            let script = format!("touch '{}'", marker.display());
            insert_override_workflow(
                &db,
                &team_id,
                &template_id,
                json!({"spec": {"script": script}}),
                false,
            )
        };
        let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
            .expect("start");
        let status = wait_workflow(&state, &run_id).await;
        assert_eq!(status, "failed", "{status}");
        assert_eq!(jobs_for(&state, &template_id), 0);
        assert!(!marker.exists(), "operator script ran");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_admin_workflow_cannot_point_a_local_run_at_an_operator_inventory() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-wf-inv-{nanos}"));
        let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
        let team_id = team_of(&state);
        let (template_id, workflow_id) = {
            let db = state.db().expect("db");
            let template_id = insert_shell(&db, &team_id, None, "true");
            let inventory_id = insert_inventory(&db, &team_id, false);
            let workflow_id = insert_override_workflow(
                &db,
                &team_id,
                &template_id,
                json!({"inventory_id": inventory_id}),
                true,
            );
            (template_id, workflow_id)
        };
        let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
            .expect("start");
        let status = wait_workflow(&state, &run_id).await;
        assert_eq!(status, "failed", "{status}");
        assert_eq!(jobs_for(&state, &template_id), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn relaunch_rejects_stored_parameters_the_template_does_not_have() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-relaunch-gate-{nanos}"));
        let marker = dir.join("stored-script-ran");
        let state = crate::boot(admins(&dir, RunnerMode::Auto)).expect("boot");
        let team_id = team_of(&state);
        let (template_id, environment_id) = {
            let db = state.db().expect("db");
            let template_id = insert_shell(&db, &team_id, None, "true");
            let environment_id: String = db
                .query_row("SELECT id FROM environments LIMIT 1", [], |row| row.get(0))
                .expect("environment");
            db.execute(
                "UPDATE templates SET environment_id = ?2 WHERE id = ?1",
                rusqlite::params![template_id, environment_id],
            )
            .expect("environment");
            (template_id, environment_id)
        };
        let script = format!("touch '{}'", marker.display());
        let stored = json!({
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": script},
            "environment_id": null,
            "extra_vars": {}
        })
        .to_string();
        {
            let db = state.db().expect("db");
            db::insert_job(
                &db,
                &db::Job {
                    id: "stored-shell".into(),
                    team_id: team_id.clone(),
                    template_id: Some(template_id.clone()),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    name: "Inline".into(),
                    tool: "shell".into(),
                    status: "failed".into(),
                    extra_vars: "{}".into(),
                    launched_by: "operator".into(),
                    trigger: "manual".into(),
                    started_at: None,
                    finished_at: Some(db::now()),
                    created_at: db::now(),
                    parameters: stored,
                    failed_hosts: "[]".into(),
                },
            )
            .expect("job");
        }
        let operator = relaunch(&state, "stored-shell", "operator", false).expect_err("operator");
        assert!(
            operator
                .to_string()
                .contains("local_runs_are_limited_to_administrators"),
            "{operator}"
        );
        let admin = relaunch(&state, "stored-shell", "admin", false).expect_err("admin");
        assert!(
            admin
                .to_string()
                .contains("local_runs_are_limited_to_administrators"),
            "{admin}"
        );
        assert!(!marker.exists(), "stored script ran");
        let matching = json!({
            "tool": "shell",
            "entrypoint": "",
            "spec": {"script": "true"},
            "environment_id": environment_id,
            "extra_vars": {}
        })
        .to_string();
        {
            let db = state.db().expect("db");
            db.execute(
                "UPDATE jobs SET parameters = ?1 WHERE id = 'stored-shell'",
                [&matching],
            )
            .expect("match");
        }
        relaunch(&state, "stored-shell", "admin", false).expect("matching snapshot");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_admin_workflow_may_replace_a_local_shell_script() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-wf-admin-shell-{nanos}"));
        let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
        let team_id = team_of(&state);
        let (template_id, workflow_id) = {
            let db = state.db().expect("db");
            let template_id = insert_shell(&db, &team_id, None, "true");
            let workflow_id = insert_override_workflow(
                &db,
                &team_id,
                &template_id,
                json!({"spec": {"script": ":"}}),
                true,
            );
            (template_id, workflow_id)
        };
        let run_id = launch_workflow(&state, &team_id, &workflow_id, json!({}), "admin", "manual")
            .expect("start");
        let status = wait_workflow(&state, &run_id).await;
        assert_eq!(status, "successful", "{status}");
        assert_eq!(jobs_for(&state, &template_id), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn relaunch_of_a_workflow_snapshot_needs_that_workflow_release() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-wf-relaunch-{nanos}"));
        let state = crate::boot(admins(&dir, RunnerMode::Local)).expect("boot");
        let team_id = team_of(&state);
        let workflow_id = {
            let db = state.db().expect("db");
            let template_id = insert_shell(&db, &team_id, None, "true");
            let workflow_id =
                insert_override_workflow(&db, &team_id, &template_id, json!({}), false);
            db::insert_workflow_job(
                &db,
                &db::WorkflowJob {
                    id: "wf-run".into(),
                    workflow_id: Some(workflow_id.clone()),
                    team_id: team_id.clone(),
                    name: "Flow".into(),
                    status: "failed".into(),
                    extra_vars: "{}".into(),
                    launched_by: "admin".into(),
                    trigger: "manual".into(),
                    created_at: db::now(),
                    finished_at: Some(db::now()),
                },
            )
            .expect("workflow job");
            db::insert_job(
                &db,
                &db::Job {
                    id: "node-job".into(),
                    team_id: team_id.clone(),
                    template_id: Some(template_id),
                    workflow_job_id: Some("wf-run".into()),
                    workflow_node_id: Some("run".into()),
                    name: "Inline".into(),
                    tool: "shell".into(),
                    status: "failed".into(),
                    extra_vars: "{}".into(),
                    launched_by: "workflow:wf-run".into(),
                    trigger: "workflow".into(),
                    started_at: None,
                    finished_at: Some(db::now()),
                    created_at: db::now(),
                    parameters: json!({
                        "tool": "shell",
                        "entrypoint": "",
                        "spec": {"script": ":"},
                        "extra_vars": {}
                    })
                    .to_string(),
                    failed_hosts: "[]".into(),
                },
            )
            .expect("job");
            workflow_id
        };
        let denied = relaunch(&state, "node-job", "admin", false).expect_err("unreleased");
        assert!(
            denied
                .to_string()
                .contains("local_runs_are_limited_to_administrators"),
            "{denied}"
        );
        {
            let db = state.db().expect("db");
            db.execute(
                "UPDATE workflows SET admin_saved = 1 WHERE id = ?1",
                [&workflow_id],
            )
            .expect("release");
        }
        relaunch(&state, "node-job", "admin", false).expect("released workflow");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_stored_shell_snapshot_does_not_run_on_the_controller() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-stored-run-{nanos}"));
        let marker = dir.join("runner-script-ran");
        let state = crate::boot(admins(&dir, RunnerMode::Auto)).expect("boot");
        let team_id = team_of(&state);
        let template_id = {
            let db = state.db().expect("db");
            let template_id = insert_shell(&db, &team_id, None, "true");
            let environment_id: String = db
                .query_row("SELECT id FROM environments LIMIT 1", [], |row| row.get(0))
                .expect("environment");
            db.execute(
                "UPDATE templates SET environment_id = ?2 WHERE id = ?1",
                rusqlite::params![template_id, environment_id],
            )
            .expect("environment");
            template_id
        };
        let script = format!("touch '{}'", marker.display());
        {
            let db = state.db().expect("db");
            db::insert_job(
                &db,
                &db::Job {
                    id: "bypass".into(),
                    team_id,
                    template_id: Some(template_id),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    name: "Inline".into(),
                    tool: "shell".into(),
                    status: "pending".into(),
                    extra_vars: "{}".into(),
                    launched_by: "admin".into(),
                    trigger: "manual".into(),
                    started_at: None,
                    finished_at: None,
                    created_at: db::now(),
                    parameters: json!({
                        "tool": "shell",
                        "entrypoint": "",
                        "spec": {"script": script},
                        "environment_id": null,
                        "extra_vars": {}
                    })
                    .to_string(),
                    failed_hosts: "[]".into(),
                },
            )
            .expect("job");
        }
        runner::execute(state.clone(), "bypass".into()).await;
        let (status, log) = {
            let db = state.db().expect("db");
            let job = db::job(&db, "bypass").expect("read").expect("job");
            let events = db::events_after(&db, "bypass", 0, 50).expect("events");
            let log = events
                .iter()
                .map(|event| event.line.clone())
                .collect::<Vec<_>>()
                .join("\n");
            (job.status, log)
        };
        assert_eq!(status, "failed", "{log}");
        assert!(
            log.contains("Local runs are limited to administrators."),
            "{log}"
        );
        assert!(!marker.exists(), "stored script ran");
        let _ = std::fs::remove_dir_all(dir);
    }
}
