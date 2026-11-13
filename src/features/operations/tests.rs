mod settings_tests {
    use crate::config::{LlmProvider, RunnerMode};
    use crate::features::operations::model::{LlmWrite, SettingsWrite};
    use crate::features::operations::service::save;
    use crate::features::operations::*;
    use crate::Shared;

    fn state() -> Shared {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-settings-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        crate::boot(crate::Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir,
            web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: RunnerMode::Local,
            container_image: false,
            job_timeout_secs: 60,
            job_output_lines: 8_000,
            local_jobs: crate::config::LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: crate::config::CookieSecure::Auto,
            trusted_proxy: false,
            mcp: true,
            public_url: Vec::new(),
            data_host: None,
            ldap: None,
        })
        .unwrap()
    }

    fn sample(password: &str) -> SettingsWrite {
        SettingsWrite {
            ldap_url: "ldap://ldap.example.com".into(),
            ldap_mode: "ldap".into(),
            ldap_bind_dn: "cn=svc,dc=example,dc=com".into(),
            ldap_bind_password: password.into(),
            ldap_user_base: "ou=users,dc=example,dc=com".into(),
            ldap_user_filter: "".into(),
            ldap_username_attr: "".into(),
            ldap_group_attr: "".into(),
            ldap_group_base: "".into(),
            ldap_starttls: true,
            ldap_tls_insecure: false,
            proxy_url: "http://proxy.example:3128".into(),
            proxy_username: "a b".into(),
            proxy_password: "p@ss:word".into(),
            proxy_no_proxy: "localhost, 127.0.0.1".into(),
            runner: "docker".into(),
            job_timeout_secs: 120,
            job_output_lines: 4_000,
            default_environment_ansible: String::new(),
            default_environment_opentofu: String::new(),
            default_environment_shell: String::new(),
            clear_ldap_password: false,
            clear_proxy_password: false,
            llm: None,
            reset: false,
        }
    }

    #[test]
    fn proxy_env_encodes_userinfo() {
        let proxy = ProxySettings {
            url: "http://proxy.example:3128".into(),
            username: "a b".into(),
            password: "p@ss:word".into(),
            no_proxy: "localhost".into(),
            explicit: true,
        };
        let value = proxy
            .env_pairs()
            .into_iter()
            .find(|(key, _)| key == "http_proxy")
            .unwrap()
            .1;
        assert_eq!(value, "http://a%20b:p%40ss%3Aword@proxy.example:3128");
        assert!(!value.contains("p@ss:word"));
    }

    #[test]
    fn saved_secret_stays_out_of_the_public_view() {
        let state = state();
        let saved = save(&state, &sample("bind-secret")).unwrap();
        assert!(saved.saved);
        assert!(!saved.ldap.bind_password.is_empty());
        assert!(!saved.proxy.password.is_empty());
        assert_eq!(saved.ldap.user_filter, "(uid={username})");
        assert_eq!(saved.runner.as_str(), "docker");
        assert_eq!(saved.job_output_lines, 4_000);
        let loaded = load(&state).unwrap();
        assert!(loaded.directory().is_some());
        assert_eq!(loaded.ldap.bind_password, "bind-secret");
        assert!(loaded.ldap.starttls);
        assert_eq!(loaded.proxy.password, "p@ss:word");
        assert!(loaded.proxy.explicit);
        let raw: String = {
            let db = state.db().unwrap();
            db.query_row(
                "SELECT value FROM settings WHERE key = 'proxy_password'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(!raw.contains("p@ss"));
        let kept = save(&state, &sample("")).unwrap();
        assert!(!kept.ldap.bind_password.is_empty());
        assert_eq!(load(&state).unwrap().ldap.bind_password, "bind-secret");
        let mut cleared = sample("");
        cleared.clear_ldap_password = true;
        cleared.clear_proxy_password = true;
        cleared.proxy_password.clear();
        let err = save(&state, &cleared).err().unwrap();
        assert_eq!(err.to_string(), "ldap_is_not_fully_configured");
        assert_eq!(load(&state).unwrap().ldap.bind_password, "bind-secret");
        assert_eq!(load(&state).unwrap().proxy.password, "p@ss:word");
        let mut proxy_only = sample("");
        proxy_only.clear_proxy_password = true;
        proxy_only.proxy_password.clear();
        let cleared = save(&state, &proxy_only).unwrap();
        assert!(!cleared.ldap.bind_password.is_empty());
        assert!(cleared.proxy.password.is_empty());
        let reset = save(
            &state,
            &SettingsWrite {
                reset: true,
                ..sample("")
            },
        )
        .unwrap();
        assert!(!reset.saved);
        assert!(reset.ldap.url.is_empty());
        assert_eq!(reset.runner.as_str(), "local");
        assert_eq!(reset.job_output_lines, 8_000);
    }

    #[test]
    fn an_omitted_output_limit_keeps_the_current_value() {
        let state = state();
        let mut keep = sample("bind-secret");
        keep.job_output_lines = 0;
        let saved = save(&state, &keep).unwrap();
        assert_eq!(saved.job_output_lines, 8_000);
        let mut bad = sample("bind-secret");
        bad.job_output_lines = 50;
        let err = save(&state, &bad).err().unwrap();
        assert_eq!(
            err.to_string(),
            "the_output_line_limit_must_be_between_100_and"
        );
        assert_eq!(load(&state).unwrap().job_output_lines, 8_000);
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }

    #[test]
    fn invalid_proxy_does_not_replace_a_saved_value() {
        let state = state();
        save(&state, &sample("bind-secret")).unwrap();
        let mut bad = sample("bind-secret");
        bad.proxy_url = "http://user:secret@proxy.example".into();
        let err = save(&state, &bad).err().unwrap();
        assert_eq!(
            err.to_string(),
            "put_the_credentials_in_their_own_fields_not"
        );
        assert_eq!(load(&state).unwrap().proxy.url, "http://proxy.example:3128");
    }

    fn environment(id: &str, tool: &str, scope: &str) -> crate::db::Environment {
        crate::db::Environment {
            id: id.into(),
            team_id: (scope == "team").then(|| "team".to_string()),
            organization_id: (scope == "organization").then(|| "org".to_string()),
            name: id.into(),
            tool: tool.into(),
            image: "quay.io/example/ee:1".into(),
            pull_policy: "if_not_present".into(),
            network: String::new(),
            description: String::new(),
            created_at: crate::db::now(),
            scope: String::new(),
            organization_name: String::new(),
            team_name: String::new(),
        }
    }

    #[test]
    fn default_environments_must_exist_be_global_and_match_their_tool() {
        let state = state();
        {
            let db = state.db().unwrap();
            crate::store::environments::insert_environment(
                &db,
                &environment("ansible-ee", "ansible", "global"),
            )
            .unwrap();
            crate::store::environments::insert_environment(
                &db,
                &environment("tofu-ee", "opentofu", "global"),
            )
            .unwrap();
            crate::store::environments::insert_environment(
                &db,
                &environment("team-ee", "ansible", "organization"),
            )
            .unwrap();
        }
        let mut write = sample("bind-secret");
        write.default_environment_ansible = "ansible-ee".into();
        write.default_environment_opentofu = "tofu-ee".into();
        let saved = save(&state, &write).unwrap();
        assert_eq!(saved.default_environment_ansible, "ansible-ee");
        assert_eq!(saved.default_environment_opentofu, "tofu-ee");
        assert_eq!(saved.default_environment_shell, "");

        let mut wrong_tool = write.clone();
        wrong_tool.default_environment_ansible = "tofu-ee".into();
        assert_eq!(
            save(&state, &wrong_tool).err().unwrap().to_string(),
            "the_default_environment_is_for_another_tool"
        );
        let mut not_global = write.clone();
        not_global.default_environment_shell = "team-ee".into();
        assert_eq!(
            save(&state, &not_global).err().unwrap().to_string(),
            "the_default_environment_must_be_global"
        );
        let mut missing = write.clone();
        missing.default_environment_shell = "gone".into();
        assert_eq!(
            save(&state, &missing).err().unwrap().to_string(),
            "the_default_environment_is_missing"
        );
        assert_eq!(
            load(&state).unwrap().default_environment_ansible,
            "ansible-ee"
        );

        {
            let db = state.db().unwrap();
            crate::store::environments::delete_environment(&db, "ansible-ee").unwrap();
        }
        assert_eq!(load(&state).unwrap().default_environment_ansible, "");
    }

    #[test]
    fn the_assistant_key_is_saved_masked_and_kept_until_cleared() {
        let state = state();
        let mut write = sample("bind-secret");
        write.llm = Some(LlmWrite {
            enabled: true,
            provider: "ollama".into(),
            base_url: "http://127.0.0.1:11434/v1".into(),
            model: "llama3".into(),
            api_key: "llm-secret-value".into(),
            clear_api_key: false,
        });
        let saved = save(&state, &write).unwrap();
        assert!(saved.llm_enabled);
        assert_eq!(saved.llm_provider, LlmProvider::Ollama);
        assert!(!saved.llm_api_key.is_empty());
        let loaded = load(&state).unwrap();
        assert!(loaded.llm_enabled);
        assert_eq!(loaded.llm_provider, LlmProvider::Ollama);
        assert_eq!(loaded.llm_api_key, "llm-secret-value");
        let raw: String = {
            let db = state.db().unwrap();
            db.query_row(
                "SELECT value FROM settings WHERE key = 'llm_api_key'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(!raw.contains("llm-secret"));

        let mut kept = write.clone();
        kept.llm = Some(LlmWrite {
            api_key: String::new(),
            ..write.llm.clone().unwrap()
        });
        assert!(!save(&state, &kept).unwrap().llm_api_key.is_empty());
        assert_eq!(load(&state).unwrap().llm_api_key, "llm-secret-value");

        let mut cleared = write.clone();
        cleared.llm = Some(LlmWrite {
            clear_api_key: true,
            api_key: String::new(),
            ..write.llm.clone().unwrap()
        });
        assert!(save(&state, &cleared).unwrap().llm_api_key.is_empty());
        assert_eq!(load(&state).unwrap().llm_api_key, "");

        let mut invalid = write;
        invalid.llm = Some(LlmWrite {
            base_url: "ftp://example.com".into(),
            ..invalid.llm.clone().unwrap()
        });
        assert_eq!(
            save(&state, &invalid).err().unwrap().to_string(),
            "the_assistant_address_must_be_http_or_https"
        );
    }

    #[test]
    fn tls_insecure_is_saved_without_a_ca_path() {
        let state = state();
        let mut write = sample("bind-secret");
        write.ldap_tls_insecure = true;
        let saved = save(&state, &write).unwrap();
        assert!(saved.ldap.tls_insecure);
        let loaded = load(&state).unwrap();
        assert!(loaded.ldap.tls_insecure);
    }
}

mod backup_tests {
    use std::fs::{self, OpenOptions};
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};

    use rusqlite::Connection;

    use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
    use crate::crypto;
    use crate::features::operations::service::{
        has_table, pack, restore_startup, unpack, write_extra_files, Manifest, DATABASE, MANIFEST,
        MAX_BYTES, SECRET,
    };
    use crate::features::operations::*;

    fn config(dir: &Path) -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.to_path_buf(),
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
        }
    }

    #[test]
    fn restore_brings_back_the_database_and_the_key() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-backup-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let blob = state.encrypt("token-geheim").expect("encrypt");
        let archive = export(&state, false).expect("export");
        {
            let db = state.db().expect("db");
            db.execute(
                "UPDATE organizations SET name = 'Anders' WHERE name = 'Plattform'",
                [],
            )
            .expect("rename");
        }
        let replacement = crypto::install_key(&dir, &[7u8; 32]).expect("new key");
        state.replace_cipher(replacement).expect("swap");
        assert!(state.decrypt(&blob).is_err());
        restore(&state, &archive).expect("restore");
        assert_eq!(state.decrypt(&blob).expect("decrypt"), "token-geheim");
        let name: String = state
            .db()
            .expect("db")
            .query_row(
                "SELECT name FROM organizations WHERE name = 'Plattform'",
                [],
                |row| row.get(0),
            )
            .expect("org");
        assert_eq!(name, "Plattform");
        assert!(dir.join("restore-previous.zip").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    fn archive_parts(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        unpack(archive).expect("unpack")
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("koreograf-{label}-{nanos}"))
    }

    #[test]
    fn export_lists_the_tofu_state_only_when_it_is_included() {
        let dir = temp_dir("backup-tofu");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::write(state_dir.join("terraform.tfstate"), b"{\"serial\":1}").expect("state");

        let without = export(&state, false).expect("export without state");
        let names: Vec<String> = archive_parts(&without)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(
            !names.iter().any(|name| name.starts_with("cache/")),
            "{names:?}"
        );

        let with = export(&state, true).expect("export with state");
        let parts = archive_parts(&with);
        let manifest: Manifest = serde_json::from_slice(
            &parts
                .iter()
                .find(|(name, _)| name == MANIFEST)
                .expect("manifest")
                .1,
        )
        .expect("manifest json");
        assert!(manifest
            .includes
            .iter()
            .any(|item| item == "opentofu_state"));
        assert!(parts.iter().any(|(name, bytes)| {
            name == &format!("cache/{team}/tofu-state/tpl-1/terraform.tfstate")
                && bytes == b"{\"serial\":1}"
        }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_writes_the_database_the_key_and_the_tofu_state() {
        let source = temp_dir("startup-source");
        let state = crate::boot(config(&source)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_path = source
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::create_dir_all(state_path.parent().expect("parent")).expect("state dir");
        fs::write(&state_path, b"state").expect("state");
        let archive = export(&state, true).expect("export");

        let dest = temp_dir("startup-dest");
        fs::create_dir_all(&dest).expect("dest");
        let summary = restore_startup(&dest, &archive, false).expect("restore");
        assert_eq!(summary.koreograf, env!("CARGO_PKG_VERSION"));
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert_eq!(
            fs::read(
                dest.join("cache")
                    .join(&team)
                    .join("tofu-state")
                    .join("tpl-1")
                    .join("terraform.tfstate")
            )
            .expect("restored state"),
            b"state"
        );
        let conn = Connection::open(dest.join(DATABASE)).expect("db");
        assert!(has_table(&conn, "users").expect("table"));
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_refuses_an_unknown_manifest_format() {
        let dir = temp_dir("unknown-format");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let parts = archive_parts(&archive);
        let mut files: Vec<(&str, &[u8])> = Vec::new();
        let bad = br#"{"format":999,"engine":"sqlite","created_at":"","koreograf":"x","includes":[],"omits":[]}"#;
        for (name, bytes) in &parts {
            if name == MANIFEST {
                files.push((name.as_str(), bad));
            } else {
                files.push((name.as_str(), bytes.as_slice()));
            }
        }
        let crafted = pack(&files).expect("pack");
        let err = restore_startup(&dir.join("dest"), &crafted, false).expect_err("format");
        assert!(
            err.to_string().contains("the_backup_has_an_unknown_format"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_refuses_a_truncated_archive() {
        let dir = temp_dir("truncated");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).expect("dest");
        let err =
            restore_startup(&dest, &archive[..archive.len() / 2], false).expect_err("truncated");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn startup_restore_refuses_traversal_and_symlink_entries() {
        let dir = temp_dir("unsafe");
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        let parts = archive_parts(&archive);
        let mut files: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        files.push(("../escape.txt", b"x"));
        let crafted = pack(&files).expect("pack");
        let dest = dir.join("dest");
        fs::create_dir_all(&dest).expect("dest");
        let err = restore_startup(&dest, &crafted, false).expect_err("traversal");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );

        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in &parts {
                writer.start_file(name.as_str(), options).expect("file");
                writer.write_all(bytes).expect("write");
            }
            writer
                .add_symlink("cache/team/tofu-state/link", "../outside", options)
                .expect("symlink");
            writer.finish().expect("finish");
        }
        let symlink_archive = cursor.into_inner();
        let err = restore_startup(&dest, &symlink_archive, false).expect_err("symlink");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_refuses_state_over_the_total_limit() {
        let dir = temp_dir("backup-total-limit");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir.join("cache").join(&team).join("tofu-state");
        for name in ["one.tfstate", "two.tfstate"] {
            let path = state_dir.join(name);
            fs::create_dir_all(&state_dir).expect("state dir");
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .expect("state file");
            file.set_len(MAX_BYTES / 2 + 1).expect("large state");
        }
        let err = export(&state, true).expect_err("too large");
        assert!(
            err.to_string().contains("the_backup_is_empty_or_too_large"),
            "{err}"
        );
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn http_restore_replaces_state_directories() {
        let dir = temp_dir("backup-state-replace");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_dir = dir.join("cache").join(&team).join("tofu-state");
        let kept = state_dir.join("keep").join("terraform.tfstate");
        fs::create_dir_all(kept.parent().expect("parent")).expect("state dir");
        fs::write(&kept, b"kept").expect("state");
        let archive = export(&state, true).expect("export");

        let stale = state_dir.join("stale").join("terraform.tfstate");
        fs::create_dir_all(stale.parent().expect("parent")).expect("stale dir");
        fs::write(&stale, b"stale").expect("stale");
        let other = dir
            .join("cache")
            .join("other-team")
            .join("tofu-state")
            .join("terraform.tfstate");
        fs::create_dir_all(other.parent().expect("parent")).expect("other dir");
        fs::write(&other, b"other").expect("other");

        restore(&state, &archive).expect("restore");
        assert_eq!(fs::read(&kept).expect("kept state"), b"kept");
        assert!(!stale.exists());
        assert!(!other.exists());
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn http_restore_keeps_state_when_the_backup_omits_it() {
        let dir = temp_dir("backup-state-keep");
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let kept = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl")
            .join("terraform.tfstate");
        fs::create_dir_all(kept.parent().expect("parent")).expect("state dir");
        fs::write(&kept, b"kept").expect("state");
        let archive = export(&state, false).expect("export without state");

        restore(&state, &archive).expect("restore");
        assert_eq!(fs::read(&kept).expect("kept state"), b"kept");
        drop(state);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn extra_files_stay_under_the_tofu_state_and_refuse_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir("backup-extra-paths");
        let outside = temp_dir("backup-extra-outside");
        fs::create_dir_all(dir.join("cache")).expect("cache");
        fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, dir.join("cache").join("team")).expect("symlink");

        let invalid = vec![("cache/team/other/file".to_string(), b"value".to_vec())];
        let err = write_extra_files(&dir, &invalid).expect_err("invalid path");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );

        let linked = vec![("cache/team/tofu-state/file".to_string(), b"value".to_vec())];
        let err = write_extra_files(&dir, &linked).expect_err("symlink");
        assert!(
            err.to_string().contains("the_backup_cannot_be_read"),
            "{err}"
        );
        assert!(!outside.join("tofu-state").exists());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn startup_restore_rolls_back_when_a_target_entry_blocks_the_move() {
        let source = temp_dir("startup-rollback-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = export(&state, false).expect("export");
        drop(state);

        let dest = temp_dir("startup-rollback-dest");
        fs::create_dir_all(dest.join(SECRET)).expect("blocker");
        let err = restore_startup(&dest, &archive, false).expect_err("blocked");
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).join("x").exists());
        assert!(dest.join(SECRET).is_dir());
        assert!(fs::read_dir(&dest).expect("dest").all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .starts_with(".restore-")));
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn running_job_blocks_restore() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-backup-busy-{nanos}"));
        let state = crate::boot(config(&dir)).expect("boot");
        let archive = export(&state, false).expect("export");
        {
            let db = state.db().expect("db");
            let team_id: String = db
                .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
                .expect("team");
            db.execute(
                "INSERT INTO jobs (
                id, team_id, template_id, workflow_job_id, workflow_node_id, name, tool, status,
                extra_vars, launched_by, trigger, started_at, finished_at, created_at
             ) VALUES (
                'job-1', ?1, NULL, NULL, NULL, 'Lauf', 'shell', 'running',
                '{}', 'admin', 'manual', NULL, NULL, '2026-01-01T00:00:00Z'
             )",
                [team_id],
            )
            .expect("job");
        }
        let err = restore(&state, &archive).expect_err("busy");
        assert!(err.to_string().contains("backup.still_running"), "{err}");
        let _ = fs::remove_dir_all(dir);
    }
}

mod migration_tests {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use rusqlite::{params, Connection};
    use serde_json::Value;

    use crate::config::{Config, CookieSecure, LocalJobs, RunnerMode};
    use crate::db;
    use crate::features::operations::model::Options;
    use crate::features::operations::service::{
        apply_path_map, import_directory, parse_path_map, prepare_with, DATABASE, SECRET,
    };
    use crate::features::operations::*;

    fn config(dir: &Path) -> Config {
        Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir.to_path_buf(),
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
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("koreograf-{label}-{nanos}"))
    }

    fn project(conn: &Connection, source: &str) {
        let team: String = conn
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        conn.execute(
            "INSERT INTO projects (id, team_id, name, source_type, source, created_at)
         VALUES ('import-project', ?1, 'Import', 'path', ?2, '2026-01-01T00:00:00Z')",
            params![team, source],
        )
        .expect("project");
    }

    fn source_with_wal(label: &str) -> (PathBuf, crate::Shared) {
        let dir = temp_dir(label);
        let state = crate::boot(config(&dir)).expect("boot");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_file = dir
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::create_dir_all(state_file.parent().expect("parent")).expect("state dir");
        fs::write(&state_file, b"{\"serial\":1}").expect("state");
        {
            let db = state.db().expect("db");
            db.pragma_update(None, "wal_autocheckpoint", 0)
                .expect("checkpoint");
            project(&db, "/home/user/projects/demo");
        }
        assert!(dir.join("koreograf.db-wal").is_file());
        (dir, state)
    }

    #[test]
    fn startup_restore_runs_in_an_empty_directory_and_renames_the_marker() {
        let source = temp_dir("restore-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = export(&state, true).expect("export");
        drop(state);
        let marker = source.join("restore.zip");
        fs::write(&marker, &archive).expect("marker");

        let dest = temp_dir("restore-dest");
        fs::create_dir_all(&dest).expect("dest");
        let options = Options {
            restore_from: Some(marker.clone()),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect("restore");
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert!(!marker.exists());
        assert!(PathBuf::from(format!("{}.applied", marker.display())).is_file());
        prepare_with(&config(&dest), &options).expect("second start is ignored");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_and_import_are_ignored_when_the_directory_has_data() {
        let dest = temp_dir("nonempty");
        fs::create_dir_all(&dest).expect("dest");
        fs::write(dest.join("keep.txt"), b"keep").expect("keep");
        let options = Options {
            restore_from: Some(dest.join("missing.zip")),
            import_from: Some(dest.join("missing-import")),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect("ignored");
        assert_eq!(fs::read(dest.join("keep.txt")).expect("keep"), b"keep");
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn startup_restore_reports_an_unknown_manifest_format() {
        let source = temp_dir("restore-format-source");
        let state = crate::boot(config(&source)).expect("boot");
        let archive = export(&state, false).expect("export");
        drop(state);
        let parts = {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).expect("zip");
            let mut out = Vec::new();
            for index in 0..zip.len() {
                let mut file = zip.by_index(index).expect("entry");
                let name = file.name().expect("name").to_string();
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut bytes).expect("read");
                out.push((name, bytes));
            }
            out
        };
        let dest = temp_dir("restore-format-dest");
        fs::create_dir_all(&dest).expect("dest");
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, bytes) in &parts {
                writer.start_file(name.as_str(), options).expect("file");
                if name == "manifest.json" {
                    writer
                    .write_all(br#"{"format":7,"engine":"sqlite","created_at":"","koreograf":"x","includes":[],"omits":[]}"#)
                    .expect("write");
                } else {
                    writer.write_all(bytes).expect("write");
                }
            }
            writer.finish().expect("finish");
        }
        let file = source.join("bad.zip");
        fs::write(&file, cursor.into_inner()).expect("bad zip");
        let options = Options {
            restore_from: Some(file),
            ..Options::default()
        };
        let err = prepare_with(&config(&dest), &options).expect_err("format");
        assert!(
            format!("{err:#}").contains("unknown manifest format"),
            "{err:#}"
        );
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn import_copies_an_active_wal_database_the_key_the_state_and_applies_the_map_once() {
        let (source, state) = source_with_wal("import-wal");
        let dest = temp_dir("import-wal-dest");
        let mapped_root = temp_dir("import-mapped");
        fs::create_dir_all(mapped_root.join("demo")).expect("mapped");
        let options = Options {
            import_from: Some(source.clone()),
            path_map: vec![(PathBuf::from("/home/user/projects"), mapped_root.clone())],
            ..Options::default()
        };
        let report = import_directory(&config(&dest), &source, &options).expect("import");
        assert_eq!(report.missing_paths, Vec::<String>::new());
        assert_eq!(report.tofu_files, 1);
        assert!(dest.join(SECRET).is_file());
        let conn = Connection::open(dest.join(DATABASE)).expect("dest db");
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, mapped_root.join("demo").display().to_string());
        let state_file = {
            let team: String = conn
                .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
                .expect("team");
            dest.join("cache")
                .join(team)
                .join("tofu-state")
                .join("tpl-1")
                .join("terraform.tfstate")
        };
        assert_eq!(fs::read(state_file).expect("state"), b"{\"serial\":1}");
        let applied = apply_path_map(
            &conn,
            &[(
                PathBuf::from("/home/user/projects"),
                PathBuf::from("/other"),
            )],
        )
        .expect("second map");
        assert_eq!(applied, 0);
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, mapped_root.join("demo").display().to_string());
        drop(conn);
        assert!(report_or_none(&dest).is_some());
        drop(state);
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
        let _ = fs::remove_dir_all(mapped_root);
    }

    #[cfg(unix)]
    #[test]
    fn import_reads_a_stopped_database_from_a_read_only_directory() {
        use std::os::unix::fs::PermissionsExt;

        let source = temp_dir("import-stopped");
        {
            let state = crate::boot(config(&source)).expect("boot");
            project(&state.db().expect("db"), "/home/user/projects/demo");
        }
        assert!(!source.join("koreograf.db-wal").exists());
        assert!(!source.join("koreograf.db-shm").exists());
        fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).expect("read-only");

        let dest = temp_dir("import-stopped-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let report = import_directory(&config(&dest), &source, &options).expect("import");
        assert!(report
            .missing_paths
            .iter()
            .any(|path| path.contains("/demo")));
        let conn = Connection::open(dest.join(DATABASE)).expect("dest db");
        let stored: String = conn
            .query_row(
                "SELECT source FROM projects WHERE id = 'import-project'",
                [],
                |row| row.get(0),
            )
            .expect("project");
        assert_eq!(stored, "/home/user/projects/demo");
        drop(conn);

        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).expect("writable");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_import_leaves_no_partial_data_and_the_next_start_retries() {
        use std::os::unix::fs::PermissionsExt;

        if nix::unistd::Uid::effective().is_root() {
            return;
        }
        let (source, state) = source_with_wal("import-failure");
        let team: String = state
            .db()
            .expect("db")
            .query_row("SELECT id FROM teams LIMIT 1", [], |row| row.get(0))
            .expect("team");
        let state_file = source
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate");
        fs::set_permissions(&state_file, fs::Permissions::from_mode(0o000)).expect("unreadable");

        let dest = temp_dir("import-failure-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        prepare_with(&config(&dest), &options).expect_err("unreadable state");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).exists());
        assert_eq!(fs::read_dir(&dest).expect("dest").count(), 0);

        prepare_with(&config(&dest), &options).expect_err("second start retries");
        assert!(!dest.join(DATABASE).exists());
        assert!(!dest.join(SECRET).exists());

        fs::set_permissions(&state_file, fs::Permissions::from_mode(0o644)).expect("readable");
        prepare_with(&config(&dest), &options).expect("third start imports");
        assert!(dest.join(DATABASE).is_file());
        assert!(dest.join(SECRET).is_file());
        assert!(report_or_none(&dest).is_some());
        assert!(dest
            .join("cache")
            .join(&team)
            .join("tofu-state")
            .join("tpl-1")
            .join("terraform.tfstate")
            .is_file());

        drop(state);
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    fn report_or_none(dir: &Path) -> Option<Value> {
        let conn = Connection::open(dir.join(DATABASE)).ok()?;
        report(&conn).ok().flatten()
    }

    #[test]
    fn import_refuses_a_directory_without_secret_key() {
        let source = temp_dir("import-no-key");
        {
            let state = crate::boot(config(&source)).expect("boot");
            drop(state);
        }
        fs::remove_file(source.join(SECRET)).expect("remove key");
        let dest = temp_dir("import-no-key-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let err = import_directory(&config(&dest), &source, &options).expect_err("key");
        assert!(format!("{err:#}").contains("secret.key"), "{err:#}");
        assert!(!dest.join(DATABASE).exists());
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn import_refuses_a_newer_schema() {
        let source = temp_dir("import-newer");
        {
            let state = crate::boot(config(&source)).expect("boot");
            let conn = state.db().expect("db");
            conn.pragma_update(None, "user_version", db::SCHEMA_VERSION + 1)
                .expect("version");
        }
        let dest = temp_dir("import-newer-dest");
        let options = Options {
            import_from: Some(source.clone()),
            ..Options::default()
        };
        let err = import_directory(&config(&dest), &source, &options).expect_err("newer");
        assert!(format!("{err:#}").contains("Upgrade Koreograf"), "{err:#}");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(dest);
    }

    #[test]
    fn the_path_map_needs_two_absolute_paths() {
        assert_eq!(parse_path_map("").expect("empty"), Vec::new());
        assert_eq!(
            parse_path_map("/old=/new,/a=/b").expect("map"),
            vec![
                (PathBuf::from("/old"), PathBuf::from("/new")),
                (PathBuf::from("/a"), PathBuf::from("/b")),
            ]
        );
        assert!(parse_path_map("/old").is_err());
        assert!(parse_path_map("old=/new").is_err());
        assert!(parse_path_map("/old=new").is_err());
    }
}

mod dbmigrate_tests {
    use crate::features::operations::service::{parse_target, target_argument};
    use crate::features::operations::*;

    #[test]
    fn the_command_is_the_db_migrate_pair_after_the_program_name() {
        assert!(db_migrate_requested([
            "koreograf",
            "db",
            "migrate",
            "--to",
            "x"
        ]));
        assert!(!db_migrate_requested(["koreograf", "migrate"]));
        assert!(!db_migrate_requested(["koreograf", "db"]));
        assert!(!db_migrate_requested(["koreograf", "db", "migrate-all"]));
    }

    #[test]
    fn the_target_is_the_value_after_to() {
        let args = ["koreograf", "db", "migrate", "--to", "postgres://db/x"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            target_argument(&args).unwrap().as_deref(),
            Some("postgres://db/x")
        );
        let args = ["koreograf", "db", "migrate", "--to=postgres://db/x"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            target_argument(&args).unwrap().as_deref(),
            Some("postgres://db/x")
        );
    }

    #[test]
    fn a_sqlite_target_is_refused() {
        assert!(parse_target("").is_err());
        assert!(parse_target("sqlite:data/koreograf.db").is_err());
    }

    #[test]
    fn the_form_target_keeps_the_password_out_of_the_url_and_audit_line() {
        let target = parse_form_target(
            "db.example",
            5432,
            "koreograf",
            "koreograf",
            "s3cret-password",
            "verify-full",
            None,
        )
        .expect("target");
        let url = target_url(&target);
        assert!(!url.contains("s3cret-password"), "{url}");
        assert!(url.contains("sslmode=verify-full"), "{url}");
        let audit = audit_entry("admin", &target);
        let line = audit_line(&audit);
        assert!(line.contains("admin"), "{line}");
        assert!(line.contains("db.example:5432/koreograf"), "{line}");
        assert!(!line.contains("s3cret-password"), "{line}");
    }

    #[test]
    fn an_incomplete_target_is_refused() {
        assert!(parse_form_target("", 5432, "db", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 0, "db", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "", "user", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "db", "", "pw", "require", None).is_err());
        assert!(parse_form_target("host", 5432, "db", "user", "pw", "broken", None).is_err());
    }
}

mod metrics_tests {
    use std::collections::HashMap;

    use crate::features::operations::service::{gauges_from_dir, CLOSED_LIMIT};
    use crate::features::operations::*;

    const FIXTURE: &str = r#"
PLAY [web] ***

TASK [Gathering Facts] ***
ok: [web-1]

TASK [web : install nginx] ***
ok: [web-1]
changed: [web-2]
fatal: [web-3]: FAILED! => {"password":"s3cret"}
skipping: [web-4]
unreachable: [web-5]

KOREOGRAF Role from cache: web

RUNNING HANDLER [web : restart nginx] ***
changed: [web-1]

TASK [namespace.collection.role : configure] ***
ok: [web-1]

PLAY RECAP ***
web-1 : ok=4 changed=2 unreachable=1 failed=1 skipped=1 rescued=0 ignored=0
ok: [web-1]
"#;

    fn meta<'a>(tool: &'a str, playbook: &'a str) -> JobMeta<'a> {
        JobMeta {
            job_id: "job-1",
            team_id: "team-1",
            tool,
            playbook,
            template: "Heal",
        }
    }

    #[test]
    fn ansible_role_results_stay_inside_the_playbook() {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "site.yml"));
        metrics.begin(&meta("ansible", "site.yml"));
        for line in FIXTURE.lines() {
            metrics.observe("job-1", line);
        }
        metrics.observe("job-1", "\u{1b}[0;31mTASK [web : colored] ***\u{1b}[0m");
        metrics.observe(
            "job-1",
            "\u{1b}[0;31mfatal: [web-9]: FAILED! => {\"password\":\"s3cret\"}\u{1b}[0m",
        );
        metrics.close("job-1", "failed", None);
        metrics.close("job-1", "failed", None);
        let text = metrics.render(&[]);
        assert!(
            text.contains("koreograf_jobs_started_total{tool=\"ansible\",playbook=\"site.yml\"} 1")
        );
        assert!(text.contains(
        "koreograf_jobs_finished_total{tool=\"ansible\",playbook=\"site.yml\",status=\"failed\"} 1"
    ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"ok\"} 1"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"changed\"} 2"
        ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"failed\"} 2"
        ));
        assert!(text.contains(
        "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"unreachable\"} 1"
    ));
        assert!(text.contains(
            "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"skipped\"} 1"
        ));
        assert!(text.contains(
        "koreograf_role_results_total{playbook=\"site.yml\",role=\"namespace.collection.role\",result=\"ok\"} 1"
    ));
        assert!(!text.contains("Gathering Facts"));
        assert!(!text.contains("s3cret"));
        assert!(!text.contains("Role from cache"));
    }

    fn watch(lines: &[&str]) -> bool {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "site.yml"));
        for line in lines {
            metrics.observe("job-1", line);
        }
        metrics.tag_filter_matched_nothing("job-1")
    }

    #[test]
    fn a_tag_filter_that_only_gathers_facts_matches_nothing() {
        assert!(watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=1 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(watch(&[
            "PLAY [all] ***",
            "\u{1b}[0;32mTASK [Gathering Facts] ***\u{1b}[0m",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=0 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(watch(&[
            "PLAY [all] ***",
            "PLAY RECAP ***",
            "web-1 : ok=0 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
    }

    #[test]
    fn a_matching_tag_is_not_an_empty_filter() {
        assert!(!watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "TASK [users : create] ***",
            "ok: [web-1]",
            "PLAY RECAP ***",
            "web-1 : ok=2 changed=0 unreachable=0 failed=0 skipped=0 rescued=0 ignored=0",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "TASK [Gathering Facts] ***",
            "ok: [web-1]",
            "\u{1b}[0;32mTASK [users : create] ***\u{1b}[0m",
            "changed: [web-1]",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "RUNNING HANDLER [users : restart] ***",
            "changed: [web-1]",
        ]));
        assert!(!watch(&[
            "PLAY [all] ***",
            "skipping: no hosts matched",
            "PLAY RECAP ***",
        ]));
        let metrics = Metrics::new();
        metrics.begin(&meta("shell", "health.sh"));
        metrics.observe("job-1", "PLAY [all] ***");
        metrics.observe("job-1", "ok: [web-1]");
        assert!(!metrics.tag_filter_matched_nothing("job-1"));
    }

    #[test]
    fn shell_output_is_not_a_role_result() {
        let metrics = Metrics::new();
        metrics.begin(&meta("shell", "health.sh"));
        metrics.observe("job-1", "TASK [web : install nginx] ***");
        metrics.observe(
            "job-1",
            "fatal: [web-1]: FAILED! => {\"password\":\"s3cret\"}",
        );
        metrics.close("job-1", "successful", None);
        let text = metrics.render(&[]);
        assert!(text.contains(
        "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"successful\"} 1"
    ));
        assert!(!text.contains("koreograf_role_results_total{"));
        assert!(!text.contains("s3cret"));
    }

    #[test]
    fn labels_are_escaped_and_unknown_status_is_failed() {
        let metrics = Metrics::new();
        metrics.begin(&meta("ansible", "a\"b\\c\n.yml"));
        metrics.close("job-1", "running", None);
        let text = metrics.render(&[]);
        assert!(text.contains("playbook=\"a\\\"b\\\\c .yml\""), "{text}");
        assert!(text.contains("status=\"failed\""), "{text}");
    }

    #[test]
    fn a_job_that_never_started_is_still_finished() {
        let metrics = Metrics::new();
        let empty = meta("shell", "");
        metrics.close("job-1", "canceled", Some(&empty));
        metrics.close("job-1", "canceled", Some(&empty));
        metrics.begin(&empty);
        let failed = JobMeta {
            job_id: "job-2",
            team_id: "team-1",
            tool: "ansible",
            playbook: "site.yml",
            template: "Heal",
        };
        metrics.close("job-2", "failed", Some(&failed));
        let pending = JobMeta {
            job_id: "job-3",
            team_id: "team-1",
            tool: "shell",
            playbook: "health.sh",
            template: "Heal",
        };
        metrics.finish_unstarted(&pending, "canceled");
        metrics.close("job-3", "canceled", Some(&pending));
        let text = metrics.render(&[]);
        assert!(!text.contains("koreograf_jobs_started_total{"), "{text}");
        assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"-\",status=\"canceled\"} 1"
        ),
        "{text}"
    );
        assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"ansible\",playbook=\"site.yml\",status=\"failed\"} 1"
        ),
        "{text}"
    );
        assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"shell\",playbook=\"health.sh\",status=\"canceled\"} 1"
        ),
        "{text}"
    );
        assert!(!text.contains("} 2"), "{text}");
    }

    #[test]
    fn an_unstarted_finish_keeps_a_running_observation() {
        let metrics = Metrics::new();
        let running = meta("ansible", "site.yml");
        metrics.begin(&running);
        metrics.observe("job-1", "PLAY [web] ***");
        metrics.observe("job-1", "TASK [web : install nginx] ***");
        metrics.observe("job-1", "ok: [web-1]");
        metrics.finish_unstarted(&running, "canceled");
        metrics.close("job-1", "canceled", None);
        let text = metrics.render(&[]);
        assert!(
            text.contains("koreograf_jobs_started_total{tool=\"ansible\",playbook=\"site.yml\"} 1"),
            "{text}"
        );
        assert!(
        text.contains(
            "koreograf_jobs_finished_total{tool=\"ansible\",playbook=\"site.yml\",status=\"canceled\"} 1"
        ),
        "{text}"
    );
        assert!(
            text.contains(
                "koreograf_role_results_total{playbook=\"site.yml\",role=\"web\",result=\"ok\"} 1"
            ),
            "{text}"
        );
    }

    #[test]
    fn closed_ids_drop_the_oldest_job() {
        let metrics = Metrics::new();
        let ids: Vec<String> = (0..=CLOSED_LIMIT)
            .map(|index| format!("job-{index}"))
            .collect();
        fn meta_for(id: &str) -> JobMeta<'_> {
            JobMeta {
                job_id: id,
                team_id: "team-1",
                tool: "shell",
                playbook: "cap.sh",
                template: "Cap",
            }
        }
        for id in &ids {
            let meta = meta_for(id);
            metrics.close(id, "failed", Some(&meta));
        }
        let oldest = meta_for(&ids[0]);
        let newest = meta_for(&ids[CLOSED_LIMIT]);
        metrics.begin(&oldest);
        metrics.begin(&newest);
        metrics.close(&ids[0], "failed", Some(&oldest));
        metrics.close(&ids[CLOSED_LIMIT], "failed", Some(&newest));
        let text = metrics.render(&[]);
        let finished = CLOSED_LIMIT + 2;
        assert!(
        text.contains(&format!(
            "koreograf_jobs_finished_total{{tool=\"shell\",playbook=\"cap.sh\",status=\"failed\"}} {finished}"
        )),
        "{text}"
    );
        assert!(
            text.contains("koreograf_jobs_started_total{tool=\"shell\",playbook=\"cap.sh\"} 1"),
            "{text}"
        );
    }

    #[test]
    fn cache_dir_counts_collections_and_roles() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-metrics-{nanos}"));
        let collection = dir.join("cache/t1/collections/ansible_collections/acme/web");
        std::fs::create_dir_all(&collection).unwrap();
        std::fs::write(collection.join("galaxy.yml"), "version: 1.2.3\n").unwrap();
        std::fs::create_dir_all(dir.join("cache/t1/roles/nginx")).unwrap();
        let mut names = HashMap::new();
        names.insert("t1".into(), "A \"Plattform\"".into());
        let gauges = gauges_from_dir(&dir, &names);
        assert_eq!(gauges.len(), 1);
        assert_eq!(gauges[0].collections, 1);
        assert_eq!(gauges[0].roles, 1);
        assert!(gauges[0].collections_bytes > 0);
        let text = Metrics::new().render(&gauges);
        assert!(text.contains(
            "koreograf_cache_collections{team_id=\"t1\",team=\"A \\\"Plattform\\\"\"} 1"
        ));
        assert!(
            text.contains("koreograf_cache_roles{team_id=\"t1\",team=\"A \\\"Plattform\\\"\"} 1")
        );
        assert!(text.contains("kind=\"collections\""));
        assert!(text.contains("kind=\"roles\""));
        assert!(text.contains("kind=\"tofu\""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
