use std::process::Command;

#[test]
fn version_flag_prints_the_package_version_and_exits() {
    let output = Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .arg("--version")
        .env_remove("KOREOGRAF_DATA")
        .env_remove("KOREOGRAF_GIT_SHA")
        .env_remove("KOREOGRAF_BUILT_AT")
        .output()
        .expect("run --version");
    assert!(
        output.status.success(),
        "status {:?}\nstderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn version_flag_prints_the_commit_and_build_date_when_the_build_recorded_them() {
    let output = Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .arg("--version")
        .env_remove("KOREOGRAF_DATA")
        .env("KOREOGRAF_GIT_SHA", "4cf8792")
        .env("KOREOGRAF_BUILT_AT", "2026-10-28")
        .output()
        .expect("run --version");
    assert!(
        output.status.success(),
        "status {:?}\nstderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("{} (4cf8792, built 2026-10-28)", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn tool_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("koreograf-doctor-{name}-{nanos}"));
    std::fs::create_dir_all(&path).expect("tool dir");
    path
}

fn write_tool(dir: &std::path::Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
    }
}

fn doctor_output(path: &std::path::Path, runner: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .arg("doctor")
        .env("KOREOGRAF_DATA", path.join("missing-data"))
        .env("KOREOGRAF_RUNNER", runner)
        .env_remove("KOREOGRAF_CONTAINER_IMAGE")
        .env("PATH", path)
        .output()
        .expect("run doctor")
}

#[test]
fn doctor_names_missing_tools_and_exits_non_zero() {
    let dir = tool_dir("missing");
    write_tool(&dir, "sh", "#!/bin/sh\nexit 0\n");
    let output = doctor_output(&dir, "local");
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("Runner preflight (mode local):"), "{text}");
    assert!(text.contains("ansible-playbook"), "{text}");
    assert!(text.contains("tofu"), "{text}");
    assert!(text.contains("Result:"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn doctor_exits_zero_with_every_local_tool() {
    let dir = tool_dir("ready");
    write_tool(
        &dir,
        "ansible-playbook",
        "#!/bin/sh\necho 'ansible-playbook [core 2.17.2]'\n",
    );
    write_tool(&dir, "tofu", "#!/bin/sh\necho 'OpenTofu v1.8.0'\n");
    write_tool(&dir, "sh", "#!/bin/sh\nexit 0\n");
    let output = doctor_output(&dir, "local");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("Result: ready."), "{text}");
    assert!(!text.contains("PATH"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
