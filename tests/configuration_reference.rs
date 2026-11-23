#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The environment reference in `docs/configuration.md` lists every
//! `KOREOGRAF_*` name that the server reads at startup.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Names that the server process does not read. Each group is documented
/// elsewhere and stays out of the configuration contract.
const NOT_SERVER_SETTINGS: &[&str] = &[
    // Given to a job process, see docs/jobs.md#job-environment.
    "KOREOGRAF_ACTION",
    "KOREOGRAF_AUTH_EXTRA",
    "KOREOGRAF_BECOME_METHOD",
    "KOREOGRAF_BECOME_PASSWORD_FILE",
    "KOREOGRAF_BECOME_USER",
    "KOREOGRAF_CHECK",
    "KOREOGRAF_COLLECTIONS",
    "KOREOGRAF_CONNECTION_PASSWORD_FILE",
    "KOREOGRAF_DIFF",
    "KOREOGRAF_ENTRYPOINT",
    "KOREOGRAF_EXTRA",
    "KOREOGRAF_FORKS",
    "KOREOGRAF_GIT_KEY_PASSPHRASE_FILE",
    "KOREOGRAF_GIT_PASSWORD_FILE",
    "KOREOGRAF_GIT_SSH_KEY",
    "KOREOGRAF_GIT_USERNAME",
    "KOREOGRAF_INVENTORY",
    "KOREOGRAF_KEY_PASSPHRASE_FILE",
    "KOREOGRAF_LIMIT",
    "KOREOGRAF_PRIVATE_KEY",
    "KOREOGRAF_PROJECT",
    "KOREOGRAF_REGISTRY_HOST",
    "KOREOGRAF_REGISTRY_PASSWORD",
    "KOREOGRAF_REGISTRY_USERNAME",
    "KOREOGRAF_REMOTE_USER",
    "KOREOGRAF_ROLES",
    "KOREOGRAF_SKIP_TAGS",
    "KOREOGRAF_SSH_ASKPASS_FILE",
    "KOREOGRAF_TAGS",
    "KOREOGRAF_TOFU",
    "KOREOGRAF_TOFU_STATE",
    "KOREOGRAF_TOOL",
    "KOREOGRAF_VAULT_FILE",
    "KOREOGRAF_VAULT_ID",
    "KOREOGRAF_VAULT_LIST",
    "KOREOGRAF_VERBOSITY",
    "KOREOGRAF_WORK",
    // Read by the `koreograf mcp` client, see docs/installation.md.
    "KOREOGRAF_TEAM_ID",
    "KOREOGRAF_TOKEN",
    "KOREOGRAF_URL",
    // Used by the PostgreSQL suite only.
    "KOREOGRAF_TEST_POSTGRES_URL",
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Names passed to `env_string`, `env_or`, `env_flag`, `std::env::var`, or
/// `std::env::var_os` as a string literal.
fn read_names(source: &str) -> BTreeSet<String> {
    const MARKERS: &[&str] = &[
        "env_string(",
        "env_or(",
        "env_flag(",
        "std::env::var(",
        "std::env::var_os(",
    ];
    let mut names = BTreeSet::new();
    for marker in MARKERS {
        let mut rest = source;
        while let Some(at) = rest.find(marker) {
            rest = &rest[at + marker.len()..];
            let Some(literal) = rest.trim_start().strip_prefix('"') else {
                continue;
            };
            let Some(end) = literal.find('"') else {
                continue;
            };
            let name = &literal[..end];
            if name.starts_with("KOREOGRAF_") {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// Names written in the reference, as whole tokens, so `KOREOGRAF_LLM` does
/// not count because `KOREOGRAF_LLM_URL` is present.
fn documented_names(reference: &str) -> BTreeSet<String> {
    const PREFIX: &str = "KOREOGRAF_";
    let bytes = reference.as_bytes();
    let mut names = BTreeSet::new();
    let mut from = 0;
    while let Some(at) = reference[from..].find(PREFIX) {
        let start = from + at;
        let after_prefix = start + PREFIX.len();
        from = after_prefix;
        let preceded_by_name =
            start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        if preceded_by_name {
            continue;
        }
        let mut end = after_prefix;
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        if end > after_prefix {
            names.insert(reference[start..end].to_string());
            from = end;
        }
    }
    names
}

#[test]
fn every_server_environment_variable_is_in_the_configuration_reference() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let reference = fs::read_to_string(root.join("docs/configuration.md")).unwrap();
    let documented = documented_names(&reference);
    let mut sources = Vec::new();
    rust_sources(&root.join("src"), &mut sources);

    let mut missing = BTreeSet::new();
    for path in sources {
        let source = fs::read_to_string(&path).unwrap();
        for name in read_names(&source) {
            if !documented.contains(&name) && !NOT_SERVER_SETTINGS.contains(&name.as_str()) {
                missing.insert(name);
            }
        }
    }

    assert!(
        missing.is_empty(),
        "docs/configuration.md must list these server variables: {missing:?}"
    );
}
