use serde_json::Value;

/// Fields a prompted launch may replace for one job. `None` keeps the template.
#[derive(Default)]
pub struct LaunchOverrides {
    /// An empty string clears the inventory.
    pub inventory_id: Option<String>,
    /// Replaces the credential list, including an empty list.
    pub credential_ids: Option<Vec<String>>,
    /// Ansible fields merged onto the template spec.
    pub spec: Option<Value>,
    pub launch_with_options: bool,
    /// True when extra vars came from a launch body rather than a trigger.
    pub extra_vars_are_overrides: bool,
}

/// Fields a prompted relaunch may replace on the stored job parameters.
#[derive(Default)]
pub struct RelaunchOverrides {
    pub only_failed_hosts: bool,
    /// Extra variables merged onto the stored values when present.
    pub extra_vars: Option<Value>,
    /// An empty string clears the stored inventory.
    pub inventory_id: Option<String>,
    /// Replaces the stored credential list, including an empty list.
    pub credential_ids: Option<Vec<String>>,
    /// Ansible or OpenTofu fields merged onto the stored spec.
    pub spec: Option<Value>,
    pub launch_with_options: bool,
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

#[derive(Debug)]
pub struct Dispatch {
    pub status: String,
    pub summary: String,
    pub job_id: Option<String>,
    pub workflow_job_id: Option<String>,
}
