use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use super::model::{Convergence, Graph, GraphEdge, NodeOverrides, NodePhase, Step, When};

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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
