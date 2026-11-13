pub const KINDS: &[&str] = &[
    "ssh",
    "vault",
    "scm",
    "net",
    "aws",
    "azure",
    "gce",
    "vmware",
    "openstack",
    "registry",
    "github",
    "gitlab",
    "galaxy",
    "kubernetes",
    "insights",
    "env",
    "netbox",
];

pub struct PreparedCredential {
    pub env: Vec<(String, String)>,
    pub files: Vec<(String, String)>,
}

/// One credential chosen for a template. `vault_id` is empty for every kind except vault.
#[derive(Clone, Copy)]
pub struct AssignedCredential<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub vault_id: &'a str,
}

/// Ids accepted for one team, plus the credential shown as the primary.
/// The primary is the first credential that is not a vault password.
pub struct AssignedSet {
    pub ids: Vec<String>,
    pub primary_id: Option<String>,
}
