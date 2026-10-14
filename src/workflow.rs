use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum When {
    Success,
    Failure,
    Always,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Convergence {
    #[default]
    Any,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodePhase {
    Pending,
    Running,
    Success,
    Failed,
    Skipped,
}

impl NodePhase {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "successful",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "successful" => Self::Success,
            "failed" | "canceled" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Success | Self::Failed | Self::Skipped)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub template_id: String,
    pub name: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub convergence: Convergence,
    /// Fields that replace the template for this node only.
    /// Missing keys keep the template value.
    #[serde(default, skip_serializing_if = "NodeOverrides::is_empty")]
    pub overrides: NodeOverrides,
}

/// Per-node launch values. An empty string clears a template inventory or credential.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_vars: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Value>,
}

impl NodeOverrides {
    pub fn is_empty(&self) -> bool {
        self.inventory_id.is_none()
            && self.credential_id.is_none()
            && self.extra_vars.is_none()
            && self.spec.is_none()
    }
}

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub when: When,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Launch(String),
    Skip(String),
}

pub fn validate(graph: &Graph) -> Result<(), String> {
    let mut ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err("Ein Knoten hat keine ID.".into());
        }
        if node.template_id.trim().is_empty() {
            return Err(format!("Knoten {} hat keine Auftragsvorlage.", node.id));
        }
        if node.name.trim().is_empty() {
            return Err(format!("Knoten {} hat keinen Namen.", node.id));
        }
        if !ids.insert(node.id.clone()) {
            return Err(format!("Doppelte Knoten-ID {}.", node.id));
        }
    }
    let mut seen = HashSet::new();
    for edge in &graph.edges {
        if !ids.contains(&edge.from) || !ids.contains(&edge.to) {
            return Err("Eine Kante verweist auf einen unbekannten Knoten.".into());
        }
        if edge.from == edge.to {
            return Err("Eine Kante darf nicht auf denselben Knoten zeigen.".into());
        }
        let key = format!("{}->{}:{:?}", edge.from, edge.to, edge.when);
        if !seen.insert(key) {
            return Err("Dieselbe Kante ist doppelt vorhanden.".into());
        }
    }
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for node in &graph.nodes {
        if has_cycle(graph, &node.id, &mut visiting, &mut done) {
            return Err("Der Workflow enthält einen Zyklus.".into());
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
mod tests {
    use super::*;

    fn node(id: &str, convergence: Convergence) -> GraphNode {
        GraphNode {
            id: id.into(),
            template_id: "t".into(),
            name: id.into(),
            x: 0.0,
            y: 0.0,
            convergence,
            overrides: NodeOverrides::default(),
        }
    }

    fn edge(from: &str, to: &str, when: When) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
            when,
        }
    }

    fn branch_graph() -> Graph {
        Graph {
            nodes: vec![
                node("a", Convergence::Any),
                node("b", Convergence::Any),
                node("c", Convergence::Any),
            ],
            edges: vec![edge("a", "b", When::Success), edge("a", "c", When::Failure)],
        }
    }

    #[test]
    fn success_branch_skips_failure_node() {
        let graph = branch_graph();
        let mut phase = HashMap::from([
            ("a".into(), NodePhase::Pending),
            ("b".into(), NodePhase::Pending),
            ("c".into(), NodePhase::Pending),
        ]);
        assert_eq!(advance(&graph, &phase), vec![Step::Launch("a".into())]);
        phase.insert("a".into(), NodePhase::Success);
        let steps = advance(&graph, &phase);
        assert!(steps.contains(&Step::Launch("b".into())));
        assert!(steps.contains(&Step::Skip("c".into())));
    }

    #[test]
    fn failure_branch_is_a_handled_success() {
        let graph = branch_graph();
        let phase = HashMap::from([
            ("a".into(), NodePhase::Failed),
            ("b".into(), NodePhase::Skipped),
            ("c".into(), NodePhase::Success),
        ]);
        assert_eq!(final_status(&graph, &phase), "successful");
    }

    #[test]
    fn unhandled_failure_fails_the_workflow() {
        let graph = branch_graph();
        let phase = HashMap::from([
            ("a".into(), NodePhase::Success),
            ("b".into(), NodePhase::Failed),
            ("c".into(), NodePhase::Skipped),
        ]);
        assert_eq!(final_status(&graph, &phase), "failed");
    }

    #[test]
    fn all_convergence_waits_for_every_parent() {
        let graph = Graph {
            nodes: vec![
                node("a", Convergence::Any),
                node("b", Convergence::Any),
                node("d", Convergence::All),
            ],
            edges: vec![edge("a", "d", When::Success), edge("b", "d", When::Success)],
        };
        let phase = HashMap::from([
            ("a".into(), NodePhase::Success),
            ("b".into(), NodePhase::Running),
            ("d".into(), NodePhase::Pending),
        ]);
        assert!(advance(&graph, &phase).is_empty());
        let phase = HashMap::from([
            ("a".into(), NodePhase::Success),
            ("b".into(), NodePhase::Success),
            ("d".into(), NodePhase::Pending),
        ]);
        assert_eq!(advance(&graph, &phase), vec![Step::Launch("d".into())]);
    }

    #[test]
    fn overrides_replace_inventory_and_keep_other_spec_keys() {
        let mut params = serde_json::json!({
            "inventory_id": "inv-template",
            "inventory_name": "Vorlage",
            "credential_id": "cred-1",
            "extra_vars": {"stage": "template"},
            "spec": {"limit": "old", "job_type": "run", "forks": 5}
        });
        let map = params.as_object_mut().expect("object");
        apply_overrides(
            map,
            &NodeOverrides {
                inventory_id: Some("inv-prod".into()),
                credential_id: None,
                extra_vars: Some(serde_json::json!({"stage": "prod"})),
                spec: Some(serde_json::json!({"limit": "prod", "forks": null})),
            },
        );
        assert_eq!(params["inventory_id"], "inv-prod");
        assert!(params["inventory_name"].is_null());
        assert_eq!(params["credential_id"], "cred-1");
        assert_eq!(params["extra_vars"]["stage"], "prod");
        assert_eq!(params["spec"]["limit"], "prod");
        assert_eq!(params["spec"]["job_type"], "run");
        assert!(params["spec"].get("forks").is_none());
    }

    #[test]
    fn a_node_without_overrides_still_parses() {
        let graph: Graph = serde_json::from_str(
            r#"{"nodes":[{"id":"a","template_id":"t","name":"A"}],"edges":[]}"#,
        )
        .expect("graph");
        assert!(graph.nodes[0].overrides.is_empty());
        assert!(validate(&graph).is_ok());
    }

    #[test]
    fn empty_inventory_clears_the_snapshot() {
        let mut params = serde_json::json!({
            "inventory_id": "inv",
            "inventory_name": "Alt",
            "credential_id": "cred",
            "credential_name": "Schlüssel",
            "credential_kind": "ssh"
        });
        let map = params.as_object_mut().expect("object");
        apply_overrides(
            map,
            &NodeOverrides {
                inventory_id: Some(String::new()),
                credential_id: Some(String::new()),
                ..NodeOverrides::default()
            },
        );
        assert!(params["inventory_id"].is_null());
        assert!(params["inventory_name"].is_null());
        assert!(params["credential_id"].is_null());
        assert!(params["credential_name"].is_null());
        assert!(params["credential_kind"].is_null());
    }

    #[test]
    fn cycles_are_rejected() {
        let graph = Graph {
            nodes: vec![node("a", Convergence::Any), node("b", Convergence::Any)],
            edges: vec![edge("a", "b", When::Success), edge("b", "a", When::Success)],
        };
        assert!(validate(&graph).is_err());
    }
}
