use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
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
}

pub fn launch(state: &Shared, req: LaunchRequest) -> AppResult<String> {
    let id = Uuid::new_v4().to_string();
    {
        let db = state.db()?;
        let template = db::template(&db, &req.template_id)?.ok_or(ApiError::NotFound)?;
        if template.team_id != req.team_id {
            return Err(ApiError::Forbidden);
        }
        let job = Job {
            id: id.clone(),
            team_id: req.team_id,
            template_id: Some(template.id),
            workflow_job_id: req.workflow_job_id,
            workflow_node_id: req.workflow_node_id,
            name: template.name,
            tool: template.tool,
            status: "pending".into(),
            extra_vars: serde_json::to_string(&req.extra_vars).unwrap_or_else(|_| "{}".into()),
            launched_by: req.launched_by,
            trigger: req.trigger,
            started_at: None,
            finished_at: None,
            created_at: db::now(),
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

pub async fn sync_project(state: &Shared, project: &db::Project) -> AppResult<String> {
    if project.source_type != "git" {
        return Ok(project.source.clone());
    }
    let path = runner::materialize_project(project, &state.cfg.data_dir)
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
            .map_err(|_| ApiError::bad("Workflow-Graph ist ungültig."))?;
        if graph.nodes.is_empty() {
            return Err(ApiError::bad("Der Workflow hat noch keine Knoten."));
        }
        workflow::validate(&graph).map_err(ApiError::bad)?;
        for node in &graph.nodes {
            let template = db::template(&db, &node.template_id)?
                .ok_or_else(|| ApiError::bad(format!("Vorlage für „{}“ fehlt.", node.name)))?;
            if template.team_id != team_id {
                return Err(ApiError::bad(format!(
                    "Vorlage für „{}“ gehört zu einem anderen Team.",
                    node.name
                )));
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
            .map_err(|_| ApiError::bad("Workflow-Graph ist ungültig."))?;
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
            let launched = launch(
                state,
                LaunchRequest {
                    team_id: team_id.to_string(),
                    template_id: node.template_id.clone(),
                    extra_vars: extra.clone(),
                    launched_by: format!("workflow:{wf_job_id}"),
                    trigger: "workflow".into(),
                    workflow_job_id: Some(wf_job_id.to_string()),
                    workflow_node_id: Some(node_id.clone()),
                },
            );
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
                summary: "Hook ist deaktiviert.",
                payload: &payload_text,
                job_id: None,
                workflow_job_id: None,
            },
        )?;
        return Ok(Dispatch {
            status: "ignored".into(),
            summary: "Hook ist deaktiviert.".into(),
            job_id: None,
            workflow_job_id: None,
        });
    }
    if matched.is_empty() {
        let id = Uuid::new_v4().to_string();
        let summary = if events.is_empty() {
            "Keine Alerts in der Nutzlast erkannt.".to_string()
        } else {
            format!(
                "Kein Alert passt auf {} / {}.",
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
                },
            )?;
            if first_job.is_none() {
                first_job = Some(id);
            }
        } else {
            return Err(ApiError::bad("Hook hat kein Ziel."));
        }
    }
    let summary = format!(
        "{} Alert(s) gestartet, zuerst {}.",
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
