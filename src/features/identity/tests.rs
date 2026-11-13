mod auth_tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Instant;

    use axum::http::HeaderMap;
    use serde_json::json;

    use crate::db::Template;
    use crate::features::identity::service::{
        clear_cookie, client_source_from, command_matches_template, record_failure_at,
        remaining_at, session_cookie, SignInKey, SIGNIN_FAILURE_LIMIT, SIGNIN_FAILURE_WINDOW,
        SIGNIN_LOCKOUT, SIGNIN_TRACKED_KEYS,
    };

    fn signin_key(username: &str, source: &str) -> SignInKey {
        SignInKey {
            data_dir: PathBuf::from("/tmp/koreograf-throttle-test"),
            username: username.into(),
            source: source.into(),
        }
    }

    #[test]
    fn repeated_failures_lock_one_account_and_source_until_the_lockout_ends() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        assert_eq!(remaining_at(&mut attempts, &key, start), None);
        record_failure_at(&mut attempts, key.clone(), start);
        assert!(remaining_at(&mut attempts, &key, start).is_some());
        assert!(remaining_at(&mut attempts, &key, start + SIGNIN_LOCKOUT / 2).is_some());
        assert_eq!(
            remaining_at(&mut attempts, &key, start + SIGNIN_LOCKOUT),
            None
        );
        assert!(!attempts.contains_key(&key));
    }

    #[test]
    fn the_lockout_is_scoped_to_the_username_and_source() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        assert!(remaining_at(&mut attempts, &key, start).is_some());
        assert_eq!(
            remaining_at(&mut attempts, &signin_key("ada", "203.0.113.11"), start),
            None
        );
        assert_eq!(
            remaining_at(&mut attempts, &signin_key("grace", "203.0.113.10"), start),
            None
        );
    }

    #[test]
    fn spraying_usernames_does_not_evict_a_lock() {
        let mut attempts = HashMap::new();
        let start = Instant::now();
        let locked = signin_key("admin", "203.0.113.10");
        for _ in 0..SIGNIN_FAILURE_LIMIT {
            record_failure_at(&mut attempts, locked.clone(), start);
        }
        for index in 0..SIGNIN_TRACKED_KEYS {
            record_failure_at(
                &mut attempts,
                signin_key(&format!("user{index}"), "203.0.113.99"),
                start,
            );
        }
        assert!(remaining_at(&mut attempts, &locked, start).is_some());
    }

    #[test]
    fn new_keys_are_not_tracked_once_every_slot_holds_a_lock() {
        let mut attempts = HashMap::new();
        let start = Instant::now();
        for index in 0..SIGNIN_TRACKED_KEYS {
            let key = signin_key(&format!("locked{index}"), "203.0.113.99");
            for _ in 0..SIGNIN_FAILURE_LIMIT {
                record_failure_at(&mut attempts, key.clone(), start);
            }
        }
        let extra = signin_key("extra", "203.0.113.99");
        record_failure_at(&mut attempts, extra.clone(), start);
        assert!(!attempts.contains_key(&extra));
        assert_eq!(attempts.len(), SIGNIN_TRACKED_KEYS);
    }

    #[test]
    fn failures_older_than_the_window_do_not_add_up() {
        let mut attempts = HashMap::new();
        let key = signin_key("ada", "203.0.113.10");
        let start = Instant::now();
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(&mut attempts, key.clone(), start);
        }
        for _ in 0..SIGNIN_FAILURE_LIMIT - 1 {
            record_failure_at(
                &mut attempts,
                key.clone(),
                start + SIGNIN_FAILURE_WINDOW + SIGNIN_FAILURE_WINDOW / 2,
            );
        }
        assert_eq!(
            remaining_at(
                &mut attempts,
                &key,
                start + SIGNIN_FAILURE_WINDOW + SIGNIN_FAILURE_WINDOW / 2
            ),
            None
        );
    }

    #[test]
    fn the_forwarded_address_is_only_used_when_the_proxy_is_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.7, 10.0.0.1".parse().unwrap());
        let peer = "192.0.2.5:4321".parse().ok();
        assert_eq!(client_source_from(true, &headers, peer), "198.51.100.7");
        assert_eq!(client_source_from(false, &headers, peer), "192.0.2.5");
        assert_eq!(client_source_from(false, &headers, None), "unknown");
        headers.insert("x-forwarded-for", "not-an-address".parse().unwrap());
        assert_eq!(client_source_from(true, &headers, peer), "192.0.2.5");
    }

    #[test]
    fn cookies_carry_secure_only_when_asked() {
        let plain = session_cookie("tok", false);
        assert_eq!(
            plain,
            "koreograf_session=tok; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800"
        );
        let secure = session_cookie("tok", true);
        assert_eq!(format!("{plain}; Secure"), secure);
        assert_eq!(
            clear_cookie(false),
            "koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"
        );
        assert_eq!(
            clear_cookie(true),
            "koreograf_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0; Secure"
        );
    }

    fn template(spec: &str) -> Template {
        Template {
            id: "t".into(),
            team_id: "team".into(),
            name: "Heal".into(),
            description: String::new(),
            tool: "ansible".into(),
            project_id: Some("project".into()),
            inventory_id: Some("inventory".into()),
            credential_id: None,
            environment_id: None,
            no_environment: true,
            entrypoint: "site.yml".into(),
            extra_vars: "{}".into(),
            spec: spec.into(),
            created_at: String::new(),
            update_on_launch: true,
            admin_saved: true,
            prompt_on_launch: false,
        }
    }

    #[test]
    fn a_failed_host_limit_does_not_change_the_command() {
        let template = template(r#"{"tags":"heal"}"#);
        let limited = json!({
            "tool": "ansible",
            "entrypoint": "site.yml",
            "project_id": "project",
            "inventory_id": "inventory",
            "spec": {"tags": "heal", "limit": "web-1,db-1"}
        });
        assert!(command_matches_template(&template, &limited));
        let mut prompted = template.clone();
        prompted.prompt_on_launch = true;
        let varied = json!({
            "tool": "ansible",
            "entrypoint": "site.yml",
            "project_id": "project",
            "inventory_id": "other",
            "spec": {"tags": "other", "skip_tags": "slow", "job_type": "check", "verbosity": 2, "limit": "db"}
        });
        assert!(command_matches_template(&prompted, &varied));
        assert!(!command_matches_template(&template, &varied));
        let replaced = json!({
            "tool": "shell",
            "entrypoint": "",
            "project_id": "project",
            "inventory_id": "inventory",
            "spec": {"script": "true"}
        });
        assert!(!command_matches_template(&template, &replaced));
    }
}

mod ldap_tests {
    use ldap3::SearchEntry;

    use crate::config::LdapConfig;
    use crate::features::identity::service::{
        authenticate, cn_from_dn, directory_username, escape_filter, group_keys, group_matches,
        group_search_filters, normalize_dn, redact_secret,
    };

    #[test]
    fn filter_escape_blocks_injection() {
        assert_eq!(escape_filter("a)(uid=*"), r"a\29\28uid=\2a");
    }

    #[test]
    fn reads_cn_from_ad_dn() {
        let dn = r"CN=Plattform\, Ops,OU=Gruppen,DC=example,DC=com";
        assert_eq!(cn_from_dn(dn).as_deref(), Some("Plattform, Ops"));
        let keys = group_keys(&[dn.into()]);
        assert!(group_matches(&keys, "Plattform, Ops"));
        assert!(group_matches(&keys, dn));
        assert!(group_matches(
            &keys,
            "cn=Plattform\\, Ops, ou=Gruppen, dc=example, dc=com"
        ));
    }

    #[test]
    fn a_group_is_identified_by_its_entry_dn() {
        let dn = "cn=Plattform,ou=groups,dc=example,dc=com";
        let keys = group_keys(&[dn.into()]);
        assert!(group_matches(&keys, "plattform"));
        assert!(group_matches(&keys, dn));
        let team_dn = "CN=Plattform,OU=Groups,DC=Example,DC=Com";
        assert!(group_matches(&keys, team_dn));
        assert!(!group_matches(&keys, "operations"));
    }

    #[test]
    fn a_cn_that_decodes_to_another_dn_does_not_match_that_dn() {
        let real = "cn=kg-admins,ou=groups,dc=example,dc=com";
        let spoof = r"cn=cn\3Dkg-admins\2Cou\3Dgroups\2Cdc\3Dexample\2Cdc\3Dcom,ou=groups,dc=example,dc=com";
        let forged = group_keys(&[spoof.into()]);
        assert!(!group_matches(&forged, real));
        assert!(!group_matches(&forged, "kg-admins"));
        let real_keys = group_keys(&[real.into()]);
        assert!(group_matches(&real_keys, real));
        assert!(group_matches(&real_keys, "KG-ADMINS"));
        assert!(!group_matches(&real_keys, spoof));
        let hex = r"cn=kg\2Dadmins,ou=groups,dc=example,dc=com";
        assert!(group_matches(&group_keys(&[hex.into()]), real));
        assert!(!group_matches(
            &group_keys(&["kg-admins".into()]),
            "kg-admins"
        ));
    }

    #[test]
    fn group_search_includes_unique_members() {
        let filters = group_search_filters("cn=alice,ou=people,dc=example,dc=com", "alice");
        assert!(filters
            .iter()
            .any(|filter| { filter == "(&(objectClass=posixGroup)(memberUid=alice))" }));
        assert!(filters.iter().any(|filter| {
        filter == "(&(objectClass=groupOfUniqueNames)(uniqueMember=cn=alice,ou=people,dc=example,dc=com))"
    }));
        let hostile = group_search_filters("cn=a)(uid=*,ou=people", "a)(uid=*");
        assert!(hostile.iter().all(|filter| !filter.contains("uid=*")));
        assert!(hostile
            .iter()
            .any(|filter| { filter.contains(r"uniqueMember=cn=a\29\28uid=\2a,ou=people") }));
    }

    #[test]
    fn the_directory_username_attribute_overrides_the_entered_login() {
        let mail = search_entry(&[("uid", "Alice"), ("mail", "alice@example.com")]);
        assert_eq!(
            directory_username(&mail, "uid", "alice@example.com"),
            ("alice".into(), true)
        );
        let ad = search_entry(&[("sAMAccountName", "Alice")]);
        assert_eq!(
            directory_username(&ad, "sAMAccountName", "Alice"),
            ("alice".into(), true)
        );
        let missing = search_entry(&[("mail", "alice@example.com")]);
        assert_eq!(
            directory_username(&missing, "uid", " Alice@Example.com "),
            ("alice@example.com".into(), false)
        );
        let blank = search_entry(&[("uid", "  ")]);
        assert_eq!(
            directory_username(&blank, "uid", "alice"),
            ("alice".into(), false)
        );
    }

    fn search_entry(attrs: &[(&str, &str)]) -> SearchEntry {
        let mut map = std::collections::HashMap::new();
        for (key, value) in attrs {
            map.insert((*key).to_string(), vec![(*value).to_string()]);
        }
        SearchEntry {
            dn: "uid=alice,ou=people,dc=example,dc=com".into(),
            attrs: map,
            bin_attrs: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn hex_escapes_match_the_same_cn_and_dn() {
        let server = r"cn=Ops\2C Nord,ou=groups,dc=example,dc=com";
        assert_eq!(cn_from_dn(server).as_deref(), Some("Ops, Nord"));
        assert_eq!(
            cn_from_dn(r"cn=a\2Bb,ou=groups,dc=example,dc=com").as_deref(),
            Some("a+b")
        );
        assert_eq!(
            cn_from_dn(r"cn=\23hash,ou=groups,dc=example,dc=com").as_deref(),
            Some("#hash")
        );
        assert_eq!(
            cn_from_dn(r"cn=a/b\3Bc,ou=groups,dc=example,dc=com").as_deref(),
            Some("a/b;c")
        );
        let decoded =
            String::from_utf8(vec![b'M', 0xC3, 0xBC, b'l', b'l', b'e', b'r']).expect("utf-8");
        assert_eq!(
            cn_from_dn(r"cn=M\C3\BCller,ou=groups,dc=example,dc=com").as_deref(),
            Some(decoded.as_str())
        );
        let keys = group_keys(&[server.into()]);
        assert!(group_matches(&keys, "Ops, Nord"));
        assert!(!group_matches(&keys, "Ops2C Nord"));
        assert!(group_matches(
            &keys,
            r"cn=Ops\, Nord,ou=groups,dc=example,dc=com"
        ));
        assert!(group_matches(
            &keys,
            r"CN=Ops\2C Nord,OU=Groups,DC=Example,DC=Com"
        ));
        let multi = group_keys(&[r"cn=mrdn+ou=x,dc=example,dc=com".into()]);
        assert!(!group_matches(&multi, "mrdn"));
    }

    #[test]
    fn escaped_comma_and_equals_stay_inside_the_cn_value() {
        let hex = r"cn=kg-admins\2Cou\3Dgroups,dc=example,dc=com";
        let slash = r"cn=kg-admins\,ou\=groups,dc=example,dc=com";
        let split = "cn=kg-admins,ou=groups,dc=example,dc=com";
        assert_eq!(cn_from_dn(hex).as_deref(), Some("kg-admins,ou=groups"));
        assert_eq!(cn_from_dn(slash).as_deref(), Some("kg-admins,ou=groups"));
        assert_eq!(normalize_dn(hex), normalize_dn(slash));
        assert_ne!(normalize_dn(hex), normalize_dn(split));
        let keys = group_keys(&[hex.into()]);
        assert!(group_matches(&keys, slash));
        assert!(!group_matches(&keys, split));
        assert!(!group_matches(&keys, "kg-admins"));
        assert!(!group_matches(&group_keys(&[slash.into()]), split));
        assert!(!group_matches(&keys, "kg-admins,ou=groups"));
    }

    #[test]
    fn the_cn_comes_only_from_the_first_rdn() {
        let dn = "ou=sub,cn=kg-admins,ou=groups,dc=example,dc=com";
        assert!(cn_from_dn(dn).is_none());
        let keys = group_keys(&[dn.into()]);
        assert!(!group_matches(&keys, "kg-admins"));
        assert!(group_matches(&keys, dn));
        assert!(group_matches(
            &keys,
            "OU=sub,CN=kg-admins,OU=groups,DC=example,DC=com"
        ));
    }

    #[test]
    fn a_multi_valued_rdn_has_no_cn() {
        let forward = r"cn=mrdn+ou=x,dc=example,dc=com";
        let reverse = r"ou=x+cn=mrdn,dc=example,dc=com";
        assert!(cn_from_dn(forward).is_none());
        assert!(cn_from_dn(reverse).is_none());
        assert_eq!(normalize_dn(forward), normalize_dn(reverse));
        let keys = group_keys(&[forward.into()]);
        assert!(!group_matches(&keys, "mrdn"));
        assert!(group_matches(&keys, reverse));
        assert_ne!(
            normalize_dn(r"cn=a\+b,dc=example,dc=com"),
            normalize_dn(r"cn=a+ou=b,dc=example,dc=com")
        );
        assert_eq!(
            cn_from_dn(r"cn=a\+b,dc=example,dc=com").as_deref(),
            Some("a+b")
        );
    }

    #[test]
    fn memberof_hex_escapes_match_the_group() {
        let member = r"cn=Ops\2C Nord,ou=groups,dc=example,dc=com";
        let keys = group_keys(&[member.into()]);
        assert!(group_matches(&keys, "Ops, Nord"));
        assert!(group_matches(
            &keys,
            r"cn=Ops\, Nord,ou=groups,dc=example,dc=com"
        ));
        assert!(group_matches(
            &keys,
            r"CN=Ops\2C Nord,OU=Groups,DC=Example,DC=Com"
        ));
        assert!(!group_matches(
            &keys,
            "cn=Ops,ou=Nord,ou=groups,dc=example,dc=com"
        ));
    }

    #[tokio::test]
    async fn an_empty_bind_password_is_not_fully_configured() {
        let cfg = LdapConfig {
            url: "ldap://127.0.0.1:389".into(),
            mode: "ldap".into(),
            bind_dn: "cn=reader,dc=example,dc=com".into(),
            bind_password: String::new(),
            user_base: "ou=people,dc=example,dc=com".into(),
            user_filter: "(uid={username})".into(),
            username_attr: "uid".into(),
            group_attr: "memberOf".into(),
            group_base: String::new(),
            starttls: false,
            tls_insecure: false,
        };
        let error = authenticate(&cfg, "alice", "secret")
            .await
            .expect_err("empty bind password");
        assert_eq!(error, "ldap_is_not_fully_configured");
    }

    #[test]
    fn redact_secret_hides_the_bind_password() {
        assert_eq!(
            redact_secret("service re-bind rejected: secret-value", "secret-value"),
            "service re-bind rejected: [redacted]"
        );
        assert_eq!(redact_secret("no secret here", ""), "no secret here");
    }
}

mod store_tests {
    use crate::features::identity::store::{list_users_sql, user_by_username_sql};
    use crate::store::sql::Dialect;

    #[test]
    fn username_lookup_uses_an_explicit_dialect_predicate() {
        let sqlite = user_by_username_sql(Dialect::Sqlite);
        let postgres = user_by_username_sql(Dialect::Postgres);
        assert!(sqlite.contains("COLLATE NOCASE"), "{sqlite}");
        assert!(!sqlite.contains("lower(username)"), "{sqlite}");
        assert!(
            postgres.contains("lower(username) = lower(?1)"),
            "{postgres}"
        );
        assert!(!postgres.contains("COLLATE NOCASE"), "{postgres}");
    }

    #[test]
    fn list_users_orders_with_an_explicit_dialect_expression() {
        assert!(list_users_sql(Dialect::Sqlite).contains("ORDER BY username COLLATE NOCASE"));
        assert!(list_users_sql(Dialect::Postgres).contains("ORDER BY lower(username)"));
    }
}

mod routes_tests {
    use crate::error::ApiJson;
    use crate::features::identity::model::{ClientPeer, LdapIdentity, LoginBody};
    use crate::features::identity::routes::login;
    use crate::features::identity::service::{
        directory_unreachable, ldap_failure_hint, normalize_email, profile_email, provision_ldap,
    };
    use crate::features::identity::store::user_by_username;

    #[test]
    fn ldap_failure_hint_splits_timeout_from_a_rejected_bind() {
        assert_eq!(ldap_failure_hint("LDAP connection: timed out"), "timeout");
        assert_eq!(ldap_failure_hint("operation timeout"), "timeout");
        assert_eq!(
            ldap_failure_hint("LDAP connection: connection refused"),
            "unreachable"
        );
        assert_eq!(ldap_failure_hint("LDAP TLS setup failed"), "unreachable");
        assert_eq!(ldap_failure_hint("ldap_rejected_the_password"), "reject");
        assert_eq!(
            ldap_failure_hint("the_user_was_not_found_uniquely"),
            "reject"
        );
    }

    #[test]
    fn directory_unreachable_covers_transport_failures_but_not_rejections() {
        assert!(directory_unreachable("LDAP connection: connection refused"));
        assert!(directory_unreachable("ldap tls setup: invalid CA file"));
        assert!(directory_unreachable("service bind: timed out"));
        assert!(directory_unreachable("user search: connection reset"));
        assert!(directory_unreachable("LDAP bind failed: timed out"));
        assert!(!directory_unreachable(
            "service bind rejected: invalid credentials"
        ));
        assert!(!directory_unreachable(
            "user search rejected: insufficient access rights"
        ));
        assert!(!directory_unreachable("ldap_rejected_the_password"));
        assert!(!directory_unreachable("the_user_was_not_found_uniquely"));
        assert!(!directory_unreachable("empty_password"));
        assert!(!directory_unreachable("ldap_is_not_fully_configured"));
    }

    #[test]
    fn email_trims_and_rejects_a_broken_address() {
        assert_eq!(
            normalize_email("  ada@example.com ").unwrap().as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(normalize_email("  ").unwrap(), None);
        assert!(normalize_email("ada").is_err());
        assert!(normalize_email("ada @example.com").is_err());
    }

    #[test]
    fn ldap_email_stays_with_the_directory() {
        let err =
            profile_email("ldap", Some("ada@example.com"), Some("other@example.com")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the_email_comes_from_ldap_or_active_directory"
        );
        assert_eq!(
            profile_email("ldap", Some("Ada@Example.com"), Some("ada@example.com"))
                .unwrap()
                .as_deref(),
            Some("ada@example.com")
        );
        assert_eq!(profile_email("local", None, Some("")).unwrap(), None);
    }

    #[test]
    fn ldap_sign_in_provisions_and_syncs_teams_on_sqlite() {
        use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ldap-sqlite-{nanos}"));
        let state = crate::boot(Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.clone(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            mcp: true,
            public_url: Vec::new(),
            data_host: None,
            ldap: None,
        })
        .expect("boot");
        let store = state.store();
        let org_id = uuid::Uuid::new_v4().to_string();
        store
            .insert_organization(&org_id, &format!("Org {nanos}"), "")
            .expect("organization");
        let team_id = uuid::Uuid::new_v4().to_string();
        store
            .insert_team(
                &team_id,
                &org_id,
                &format!("Plattform {nanos}"),
                "",
                Some("cn=Plattform,ou=groups,dc=example,dc=com"),
            )
            .expect("team");

        let user = provision_ldap(
            &state,
            LdapIdentity {
                username: "ada".into(),
                display_name: "Ada Lovelace".into(),
                email: Some("ada@example.com".into()),
                groups: vec!["cn=Plattform,ou=groups,dc=example,dc=com".into()],
            },
        )
        .expect("provision");
        assert_eq!(user.auth_source, "ldap");
        assert_eq!(user.email.as_deref(), Some("ada@example.com"));
        let memberships = store.list_user_memberships(&user.id).expect("memberships");
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].team_id, team_id);
        assert_eq!(memberships[0].role, "operator");
        assert_eq!(memberships[0].source, "ldap");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ldap_sign_in_does_not_take_over_a_local_account() {
        use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
        use crate::error::ApiError;
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ldap-local-{nanos}"));
        let state = crate::boot(Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.clone(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            mcp: true,
            public_url: Vec::new(),
            data_host: None,
            ldap: None,
        })
        .expect("boot");
        let err = provision_ldap(
            &state,
            LdapIdentity {
                username: "admin".into(),
                display_name: "Taken".into(),
                email: Some("taken@example.com".into()),
                groups: vec!["cn=kg-admins,ou=groups,dc=example,dc=com".into()],
            },
        )
        .expect_err("local account");
        assert!(matches!(err, ApiError::SignInFailed));
        let db = state.db().expect("db");
        let user = user_by_username(&db, "admin")
            .expect("read")
            .expect("admin");
        assert_eq!(user.auth_source, "local");
        assert!(user.is_admin);
        assert_eq!(user.display_name, "Administrator");
        assert!(user.email.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn boot_with_directory(url: String) -> (crate::Shared, std::path::PathBuf) {
        use crate::config::{Config, CookieSecure, LdapConfig, LocalJobs, RunnerMode};
        use std::path::PathBuf;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-ldap-outage-{nanos}"));
        let state = crate::boot(Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.clone(),
            web_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: CookieSecure::Auto,
            trusted_proxy: false,
            mcp: true,
            public_url: Vec::new(),
            data_host: None,
            ldap: Some(LdapConfig {
                url,
                mode: "ldap".into(),
                bind_dn: "cn=reader,dc=example,dc=com".into(),
                bind_password: "secret".into(),
                user_base: "ou=people,dc=example,dc=com".into(),
                user_filter: "(uid={username})".into(),
                username_attr: "uid".into(),
                group_attr: "memberOf".into(),
                group_base: String::new(),
                starttls: false,
                tls_insecure: false,
            }),
        })
        .expect("boot");
        (state, dir)
    }

    async fn closed_directory_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("ldap://{addr}")
    }

    #[tokio::test]
    async fn ldap_outage_reports_the_directory_as_unreachable() {
        use axum::extract::State;
        use axum::http::{HeaderMap, StatusCode};
        use axum::response::IntoResponse;
        use http_body_util::BodyExt;

        let (state, dir) = boot_with_directory(closed_directory_url().await);
        let error = login(
            State(state),
            HeaderMap::new(),
            ClientPeer(None),
            ApiJson(LoginBody {
                username: "ada".into(),
                password: "secret".into(),
            }),
        )
        .await
        .expect_err("sign-in must fail while the directory is down");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("directory_is_unreachable"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn local_sign_in_still_works_while_the_directory_is_down() {
        use axum::extract::State;
        use axum::http::{HeaderMap, StatusCode};

        let (state, dir) = boot_with_directory(closed_directory_url().await);
        let response = login(
            State(state),
            HeaderMap::new(),
            ClientPeer(None),
            ApiJson(LoginBody {
                username: "admin".into(),
                password: "test-admin-pass".into(),
            }),
        )
        .await
        .expect("local sign-in");
        assert_eq!(response.status(), StatusCode::OK);
        let _ = std::fs::remove_dir_all(dir);
    }
}
