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
