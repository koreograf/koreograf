mod llm_tests {
    use serde_json::json;

    use crate::features::assistant::service::{
        is_write, redact_text, redact_value, truncate_chars, SecretSet, WRITE_TOOLS,
    };

    fn secrets(values: &[&str]) -> SecretSet {
        let mut set = SecretSet::default();
        for value in values {
            set.add(value);
        }
        set
    }

    #[test]
    fn known_secret_values_never_survive_redaction() {
        let set = secrets(&["bind-secret-value", "proxy-pass"]);
        let text = "ldap rejected bind-secret-value; proxy-pass works";
        let redacted = redact_text(text, &set);
        assert!(!redacted.contains("bind-secret-value"), "{redacted}");
        assert!(!redacted.contains("proxy-pass"), "{redacted}");
        assert!(redacted.contains("[redacted]"), "{redacted}");
    }

    #[test]
    fn secret_assignments_bearer_tokens_and_urls_are_masked() {
        let text = "DB_PASSWORD=hunter2 Authorization: Bearer abc123token git https://user:pw@example/repo.git";
        let redacted = redact_text(text, &SecretSet::default());
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(!redacted.contains("abc123token"), "{redacted}");
        assert!(!redacted.contains("user:pw"), "{redacted}");
        assert!(
            redacted.contains("https://[redacted]@example/repo.git"),
            "{redacted}"
        );
    }

    #[test]
    fn secret_shaped_tokens_and_private_keys_are_masked() {
        let openai = ["sk-", "live-abcdefghijklmnopqrstuvwxyz"].concat();
        let github = ["ghp_", "0123456789abcdef"].concat();
        let begin = ["-----BEGIN ", "OPENSSH", " PRIVATE KEY-----"].concat();
        let end = ["-----END ", "OPENSSH", " PRIVATE KEY-----"].concat();
        let text = format!("token {openai} and {github}\n{begin}\nbody\n{end}");
        let redacted = redact_text(&text, &SecretSet::default());
        assert!(
            !redacted.contains("live-abcdefghijklmnopqrstuvwxyz"),
            "{redacted}"
        );
        assert!(!redacted.contains("0123456789abcdef"), "{redacted}");
        assert!(!redacted.contains("body"), "{redacted}");
        assert!(redacted.contains("[redacted private key]"), "{redacted}");
    }

    #[test]
    fn extra_vars_values_never_reach_the_model() {
        let job = json!({
            "id": "job-1",
            "name": "Deploy",
            "extra_vars": {"api_password": "hunter2", "plain": "visible"},
            "parameters": {"script": "echo ok"},
            "template": {"token": "ghp_secretvalue", "credential_id": "cred-1"}
        });
        let redacted = redact_value(&job, &SecretSet::default());
        assert_eq!(redacted["extra_vars"], "[redacted]");
        assert_eq!(redacted["template"]["token"], "[redacted]");
        assert_eq!(redacted["template"]["credential_id"], "cred-1");
        assert_eq!(redacted["parameters"]["script"], "echo ok");
        let text = redacted.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert!(!text.contains("ghp_secretvalue"), "{text}");
    }

    #[test]
    fn vault_blocks_are_masked() {
        let text = "$ANSIBLE_VAULT;1.1;AES256\n63386436\nmore\n\nplain tail";
        let redacted = redact_text(text, &SecretSet::default());
        assert!(!redacted.contains("63386436"), "{redacted}");
        assert!(redacted.contains("plain tail"), "{redacted}");
    }

    #[test]
    fn a_long_output_is_truncated_after_redaction() {
        let text = format!("{} secret-in-the-tail", "x".repeat(60));
        let redacted = truncate_chars(&redact_text(&text, &secrets(&["secret-in-the-tail"])), 20);
        assert!(redacted.ends_with("[truncated]"), "{redacted}");
        assert!(!redacted.contains("secret-in-the-tail"), "{redacted}");
    }

    #[test]
    fn only_write_tools_need_a_confirmation() {
        for name in WRITE_TOOLS {
            assert!(is_write(name), "{name}");
        }
        assert!(!is_write("job_output"));
        assert!(!is_write("launch_everything"));
    }
}

mod mcp_tests {
    use crate::features::assistant::service::path_segment;

    #[test]
    fn ids_cannot_leave_the_route() {
        assert_eq!(path_segment("abc-123_X~"), "abc-123_X~");
        assert_eq!(path_segment("../admin"), "%2E%2E%2Fadmin");
        assert_eq!(path_segment("a/b"), "a%2Fb");
        assert_eq!(path_segment("a?b#c"), "a%3Fb%23c");
        assert_eq!(path_segment("a b"), "a%20b");
    }
}
