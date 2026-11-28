use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One inventory-adjacent `group_vars` or `host_vars` file.
///
/// The path is relative to the inventory directory, for example
/// `group_vars/all/accounts.yml`. The body is kept verbatim so a vaulted file
/// reaches Ansible unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryVarFile {
    pub path: String,
    pub body: String,
}

/// One host parsed from an Ansible INI inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InventoryHost {
    pub name: String,
    pub groups: Vec<String>,
    pub vars: BTreeMap<String, String>,
    /// Vaulted `host_vars` files that apply to this host. They are shown as
    /// masked placeholders and never decrypted.
    #[serde(skip)]
    pub masked: Vec<String>,
}

/// One group parsed from an Ansible INI or YAML inventory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct InventoryGroup {
    pub name: String,
    pub children: Vec<String>,
    pub vars: BTreeMap<String, String>,
    /// Vaulted `group_vars` files that apply to this group. They are shown as
    /// masked placeholders and never decrypted.
    #[serde(skip)]
    pub masked: Vec<String>,
}

/// Hosts and groups of one or more inventory texts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct InventoryData {
    pub hosts: Vec<InventoryHost>,
    pub groups: Vec<InventoryGroup>,
}

/// One effective host variable and the place that set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EffectiveVar {
    pub name: String,
    pub value: String,
    pub origin: String,
    /// True for the placeholder of a vaulted file. The UI never shows the
    /// stored ciphertext or a decrypted value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

/// The masked value shown for a vaulted `group_vars` or `host_vars` file.
pub const MASKED_VALUE: &str = "••••••";

pub const DEFAULT_MAX_HOSTS: usize = 2_000;
pub const MAX_HOSTS_LIMIT: usize = 10_000;

pub const MAX_STATIC_HOSTS: usize = 10_000;
pub const MAX_STATIC_BYTES: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetboxSource {
    pub api_endpoint: String,
    pub validate_certs: bool,
    pub group_by: Vec<String>,
    pub group_names_raw: bool,
    pub load_config_context: bool,
    pub max_hosts: usize,
    pub query: Vec<(String, String)>,
    pub virtual_machines: bool,
}

#[derive(Clone, Debug)]
pub struct NetboxResult {
    pub body: String,
    /// JSON array of inventory-adjacent variables files. A NetBox source has none.
    pub vars_json: String,
    pub collisions: Vec<String>,
    pub dropped_vars: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NetboxPreview {
    pub host_count: usize,
    pub groups: Vec<(String, usize)>,
    pub sample_host: Option<(String, BTreeMap<String, String>)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FoundInventory {
    pub path: String,
    pub format: String,
    pub plugin: Option<String>,
    pub api_endpoint: Option<String>,
    pub group_by: Vec<String>,
    pub query_filters: Vec<String>,
    pub validate_certs: Option<bool>,
    pub virtual_machines: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectContents {
    pub playbooks: Vec<String>,
    pub inventories: Vec<FoundInventory>,
}
