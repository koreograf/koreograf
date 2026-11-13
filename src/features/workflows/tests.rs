use std::collections::HashMap;

use crate::features::workflows::model::{
    Convergence, Graph, GraphEdge, GraphNode, NodeOverrides, NodePhase, Step, When,
};

use super::{advance, apply_overrides, final_status, validate};

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
        "inventory_name": "template",
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
    let graph: Graph =
        serde_json::from_str(r#"{"nodes":[{"id":"a","template_id":"t","name":"A"}],"edges":[]}"#)
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
        "credential_name": "stored-key",
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
    assert!(params["credentials"]
        .as_array()
        .expect("credentials")
        .is_empty());
}

#[test]
fn cycles_are_rejected() {
    let graph = Graph {
        nodes: vec![node("a", Convergence::Any), node("b", Convergence::Any)],
        edges: vec![edge("a", "b", When::Success), edge("b", "a", When::Success)],
    };
    assert!(validate(&graph).is_err());
}
