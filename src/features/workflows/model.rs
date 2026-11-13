use serde::{Deserialize, Serialize};
use serde_json::Value;

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub when: When,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
