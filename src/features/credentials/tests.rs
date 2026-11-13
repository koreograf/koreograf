use super::*;

use serde_json::json;

#[test]
fn ssh_password_stays_out_of_environment_values() {
    let secret = json!({
        "username": "deploy",
        "password": "s3cret-pass",
        "become_method": "sudo",
        "become_password": "root-pass"
    });
    check("ssh", &secret).unwrap();
    let work = std::env::temp_dir();
    let prepared = prepare("ssh", &secret, false, &work).unwrap();
    let rendered = prepared
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rendered.contains("s3cret-pass"));
    assert!(!rendered.contains("root-pass"));
    assert!(prepared
        .env
        .iter()
        .any(|(key, _)| key == "KOREOGRAF_CONNECTION_PASSWORD_FILE"));
    assert!(prepared
        .files
        .iter()
        .any(|(name, body)| name == "connection.pass" && body == "s3cret-pass"));
    assert!(prepared
        .files
        .iter()
        .any(|(name, body)| name == "auth.json" && body.contains("ansible_password")));
    assert!(check("ssh", &json!({})).is_err());
    assert!(check("ssh", &json!({"password": "only"})).is_ok());
}

#[test]
fn edit_keeps_a_blank_secret_and_hides_the_value() {
    let previous = json!({
        "username": "deploy",
        "password": "old-pass",
        "private_key": "stored-key-material",
        "become_method": "sudo"
    });
    let merged = merge_secret(
        "ssh",
        &previous,
        &json!({"username": "nas", "password": "  ", "become_method": ""}),
        &["private_key".to_string(), "username".to_string()],
    )
    .unwrap();
    assert_eq!(merged["username"], "nas");
    assert_eq!(merged["password"], "old-pass");
    assert!(merged.get("private_key").is_none());
    assert!(merged.get("become_method").is_none());
    let replaced = merge_secret(
        "ssh",
        &previous,
        &json!({"password": "new-pass"}),
        &["password".to_string()],
    )
    .unwrap();
    assert_eq!(replaced["password"], "new-pass");
    assert_eq!(replaced["private_key"], previous["private_key"]);
    let (fields, set) = describe("ssh", &replaced);
    assert_eq!(fields["username"], "deploy");
    assert!(fields.get("password").is_none());
    assert_eq!(set, vec!["password".to_string(), "private_key".to_string()]);
    let rendered = format!("{fields} {}", set.join(","));
    assert!(!rendered.contains("new-pass"));
    assert!(!rendered.contains("stored-key-material"));
    let env = merge_secret(
        "env",
        &json!({"A": "1", "B": "2"}),
        &json!({"A": "3"}),
        &["B".to_string()],
    )
    .unwrap();
    assert_eq!(env["A"], "3");
    assert!(env.get("B").is_none());
    assert!(merge_secret("ssh", &previous, &json!([]), &[]).is_err());
    assert!(check(
        "aws",
        &json!({"access_key": "AKIA", "secret_key": "secret"})
    )
    .is_ok());
}

#[test]
fn one_kind_except_many_vault_credentials() {
    let ssh = AssignedCredential {
        id: "ssh",
        kind: "ssh",
        vault_id: "",
    };
    let prod = AssignedCredential {
        id: "vault-prod",
        kind: "vault",
        vault_id: "prod",
    };
    let stage = AssignedCredential {
        id: "vault-stage",
        kind: "vault",
        vault_id: "stage",
    };
    assert!(check_assignment(&[ssh, prod, stage]).is_ok());
    let other = AssignedCredential {
        id: "ssh-2",
        kind: "ssh",
        vault_id: "",
    };
    assert_eq!(
        check_assignment(&[ssh, other]).unwrap_err(),
        "only_one_credential_of_each_kind_is_allowed"
    );
    let again = AssignedCredential {
        id: "vault-prod-2",
        kind: "vault",
        vault_id: "prod",
    };
    assert_eq!(
        check_assignment(&[prod, again]).unwrap_err(),
        "this_vault_id_is_already_selected"
    );
    let plain = AssignedCredential {
        id: "vault-plain",
        kind: "vault",
        vault_id: "",
    };
    let plain_two = AssignedCredential {
        id: "vault-plain-2",
        kind: "vault",
        vault_id: "",
    };
    assert_eq!(
        check_assignment(&[plain, plain_two]).unwrap_err(),
        "only_one_vault_credential_may_omit_the_vault"
    );
    assert!(check_assignment(&[ssh, plain, prod]).is_ok());
}

#[test]
fn only_the_helper_names_get_the_askpass_path() {
    assert_eq!(
        askpass_kind(["/tmp/git-askpass", "Username:"]),
        Some(AskpassKind::Git)
    );
    assert_eq!(
        askpass_kind(["/tmp/ssh-askpass", "Passphrase:"]),
        Some(AskpassKind::Ssh)
    );
    assert_eq!(
        askpass_kind(["/usr/local/bin/koreograf", "entrypoint"]),
        None
    );
}

#[test]
fn only_the_git_ssh_name_starts_the_ssh_helper() {
    assert!(git_ssh_requested(["/tmp/git-ssh"]));
    assert!(git_ssh_requested([
        "/usr/local/bin/git-ssh",
        "git@example.com"
    ]));
    assert!(!git_ssh_requested(["/tmp/git-askpass"]));
    assert!(!git_ssh_requested(["/usr/local/bin/koreograf"]));
    assert!(!git_ssh_requested(Vec::<String>::new()));
}

#[test]
fn the_git_helper_reads_the_username_from_the_environment() {
    let answer = askpass_answer(
        AskpassKind::Git,
        "Username for 'https://example.com':",
        |_| None,
    )
    .unwrap();
    assert_eq!(answer, "oauth2");
    let answer = askpass_answer(
        AskpassKind::Git,
        "Username for 'https://example.com':",
        |name| (name == "KOREOGRAF_GIT_USERNAME").then(|| "bot".to_string()),
    )
    .unwrap();
    assert_eq!(answer, "bot");
}

#[test]
fn the_git_helper_reads_the_password_file() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("koreograf-askpass-{nanos}"));
    std::fs::write(&path, "secret\n").unwrap();
    let shown = path.display().to_string();
    let answer = askpass_answer(
        AskpassKind::Git,
        "Password for 'https://example.com':",
        |name| (name == "KOREOGRAF_GIT_PASSWORD_FILE").then(|| shown.clone()),
    )
    .unwrap();
    assert_eq!(answer, "secret");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_missing_file_is_an_error_without_a_secret() {
    let err = askpass_answer(AskpassKind::Ssh, "Passphrase:", |_| None).unwrap_err();
    assert!(err.contains("KOREOGRAF_SSH_ASKPASS_FILE"));
}
