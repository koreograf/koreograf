use std::process::Command;

#[test]
fn version_flag_prints_the_package_version_and_exits() {
    let output = Command::new(env!("CARGO_BIN_EXE_koreograf"))
        .arg("--version")
        .env_remove("KOREOGRAF_DATA")
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
