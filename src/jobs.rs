use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::{self, Hook, Job, WorkflowJob};
use crate::error::{ApiError, AppResult};
use crate::hooks::{self, AlertEvent};
use crate::runner;
use crate::workflow::{self, NodePhase};
use crate::Shared;

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
}

pub fn launch(state: &Shared, req: LaunchRequest) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    {
        let db = state.db()?;
        let template = db::template(&db, &req.template_id)?.ok_or(ApiError::NotFound)?;
        if template.team_id != req.team_id {
            return Err(ApiError::Forbidden);
        }
        let parameters = match &req.parameters_override {
            Some(raw) => stored_parameters(raw)?,
            None => capture_parameters(&db, &template, &req.extra_vars),
        };
        let parsed = db::json_object(&parameters);
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

pub fn relaunch(
    state: &Shared,
    source_id: &str,
    launched_by: &str,
    only_failed_hosts: bool,
) -> AppResult<String> {
    let (team_id, template_id, extra, parameters) = {
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
            capture_parameters(&db, &template, &db::json_object(&job.extra_vars))
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
        (
            job.team_id,
            template_id,
            db::json_object(&job.extra_vars),
            parameters,
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
            workflow_job_id: None,
            workflow_node_id: None,
            parameters_override: Some(parameters),
        },
    )
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
    let mut params = db::json_object(&capture_parameters(db, template, workflow_extra));
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

fn capture_parameters(
    db: &rusqlite::Connection,
    template: &db::Template,
    request_extra: &Value,
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
    let linked = db::template_credential_ids(db, &template.id)
        .ok()
        .unwrap_or_default();
    let linked = if linked.is_empty() {
        template
            .credential_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        linked
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
    })
    .to_string()
}

pub async fn sync_project(state: &Shared, project: &db::Project) -> AppResult<String> {
    if project.source_type != "git" {
        return Ok(project.source.clone());
    }
    let proxy = crate::settings::load(state)?.proxy;
    let path = runner::materialize_project(project, &state.cfg.data_dir, &proxy)
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
        db::insert_event(&db, job_id, "system", "Abbruch angefordert.")?;
    }
    let running = state
        .procs
        .lock()
        .ok()
        .and_then(|procs| procs.get(job_id).cloned());
    if let Some(proc) = running {
        runner::signal_stop(&proc);
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
                runner::signal_stop(&proc);
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

pub fn accept_hook(state: &Shared, hook: &Hook, payload: &Value) -> AppResult<Dispatch> {
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
                &format!("hook:{}", hook.name),
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
                    launched_by: format!("hook:{}", hook.name),
                    trigger: "hook".into(),
                    workflow_job_id: None,
                    workflow_node_id: None,
                    parameters_override: None,
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
    use crate::config::{Config, RunnerMode};
    use std::path::PathBuf;

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
}
