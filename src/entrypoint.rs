use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub fn entrypoint_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .nth(1)
        .is_some_and(|arg| arg.as_ref() == "entrypoint")
}

const TARGET_UID: u32 = 10001;
const TARGET_GID: u32 = 10001;
const DEFAULT_DATA_DIR: &str = "/var/lib/koreograf";
const DEFAULT_SOCKET: &str = "/var/run/docker.sock";
const SYSTEM_CA_BUNDLE: &str = "/etc/ssl/certs/ca-certificates.crt";
pub(crate) const GIT_CA_BUNDLE: &str = "/tmp/koreograf-ca-bundle.pem";

pub fn entrypoint(args: &[String]) -> Result<(), String> {
    let command: Vec<OsString> = args.iter().skip(2).map(OsString::from).collect::<Vec<_>>();
    let command = if command.is_empty() {
        vec![OsString::from("koreograf")]
    } else {
        command
    };
    run_entrypoint(&command)
}

#[cfg(unix)]
fn run_entrypoint(command: &[OsString]) -> Result<(), String> {
    use nix::unistd::Uid;
    use std::os::unix::process::CommandExt;

    let data_dir =
        PathBuf::from(std::env::var("KOREOGRAF_DATA").unwrap_or_else(|_| DEFAULT_DATA_DIR.into()));
    let socket = PathBuf::from(
        std::env::var("KOREOGRAF_DOCKER_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET.into()),
    );
    let socket_group = socket_group(&socket)?;

    if Uid::effective().is_root() {
        prepare_data_dir(&data_dir)?;
        let group = socket_group.unwrap_or(TARGET_GID);
        drop_to_target(group).map_err(|err| format!("Could not drop privileges ({err})."))?;
        if socket_group.is_some() {
            check_socket(&socket, socket_group, true)?;
        }
    } else {
        check_socket(&socket, socket_group, false)?;
        set_no_new_privs().map_err(|err| format!("Could not set no_new_privs ({err})."))?;
    }

    let bundle = git_ca_bundle()?;
    let program = &command[0];
    let mut child = std::process::Command::new(program);
    child.args(&command[1..]);
    if let Some(path) = bundle {
        child.env("SSL_CERT_FILE", &path);
        child.env("GIT_SSL_CAINFO", &path);
    }
    let err = child.exec();
    Err(format!(
        "Could not start {}: {}",
        program.to_string_lossy(),
        err
    ))
}

#[cfg(not(unix))]
fn run_entrypoint(command: &[OsString]) -> Result<(), String> {
    let bundle = git_ca_bundle()?;
    let program = &command[0];
    let mut child = std::process::Command::new(program);
    child.args(&command[1..]);
    if let Some(path) = bundle {
        child.env("SSL_CERT_FILE", &path);
        child.env("GIT_SSL_CAINFO", &path);
    }
    let status = child
        .status()
        .map_err(|err| format!("Could not start {}: {err}", program.to_string_lossy()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with status {status}.",
            program.to_string_lossy()
        ))
    }
}

fn git_ca_bundle() -> Result<Option<PathBuf>, String> {
    git_ca_bundle_from(
        Path::new(crate::tls::CA_DIRECTORY),
        Path::new(SYSTEM_CA_BUNDLE),
        Path::new(GIT_CA_BUNDLE),
    )
}

fn git_ca_bundle_from(
    ca_dir: &Path,
    system_bundle: &Path,
    out: &Path,
) -> Result<Option<PathBuf>, String> {
    let loaded = crate::tls::load_cas(ca_dir)?;
    if loaded.pem.is_empty() {
        return Ok(None);
    }
    let mut bundle = match fs::read(system_bundle) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            return Err(format!(
                "The system CA bundle {} cannot be read.",
                system_bundle.display()
            ))
        }
    };
    if !bundle.is_empty() && !bundle.ends_with(b"\n") {
        bundle.push(b'\n');
    }
    bundle.extend(loaded.pem);
    fs::write(out, bundle).map_err(|err| format!("Could not write {} ({err}).", out.display()))?;
    Ok(Some(out.to_path_buf()))
}

#[cfg(unix)]
fn prepare_data_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    fs::create_dir_all(path)
        .map_err(|err| format!("Could not create the data directory ({err})."))?;
    let root =
        fs::metadata(path).map_err(|err| format!("Could not read the data directory ({err})."))?;
    let device = root.dev();
    chown(path)?;
    chown_tree(path, device)
}

#[cfg(unix)]
fn chown_tree(path: &Path, device: u64) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let entries =
        fs::read_dir(path).map_err(|err| format!("Could not read {} ({err}).", path.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Could not read {} ({err}).", path.display()))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)
            .map_err(|err| format!("Could not read {} ({err}).", child.display()))?;
        if metadata.dev() != device {
            continue;
        }
        if metadata.uid() != TARGET_UID || metadata.gid() != TARGET_GID {
            chown(&child)?;
        }
        if metadata.is_dir() {
            chown_tree(&child, device)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn chown(path: &Path) -> Result<(), String> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{fchownat, Gid, Uid};

    fchownat(
        nix::fcntl::AT_FDCWD,
        path,
        Some(Uid::from_raw(TARGET_UID)),
        Some(Gid::from_raw(TARGET_GID)),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .map_err(|err| format!("Could not change the owner of {} ({err}).", path.display()))
}

#[cfg(unix)]
pub(crate) fn own_tree(path: &Path, uid: u32, gid: u32) -> Result<(), String> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{fchownat, Gid, Uid};

    let metadata = fs::symlink_metadata(path)
        .map_err(|err| format!("Could not read {} ({err}).", path.display()))?;
    fchownat(
        nix::fcntl::AT_FDCWD,
        path,
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )
    .map_err(|err| format!("Could not change the owner of {} ({err}).", path.display()))?;
    if metadata.is_dir() {
        let entries = fs::read_dir(path)
            .map_err(|err| format!("Could not read {} ({err}).", path.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|err| format!("Could not read {} ({err}).", path.display()))?;
            own_tree(&entry.path(), uid, gid)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn socket_group(path: &Path) -> Result<Option<u32>, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(format!(
                "Cannot determine the group of {}. Check the mounted socket and give it a numeric group that uid 10001 can use.",
                path.display()
            ))
        }
    };
    if !metadata.file_type().is_socket() {
        return Ok(None);
    }
    Ok(Some(metadata.gid()))
}

#[cfg(unix)]
fn drop_to_target(socket_group: u32) -> nix::Result<()> {
    use nix::unistd::{setgid, setgroups, setuid, Gid, Uid};

    setgroups(&[Gid::from_raw(socket_group)])?;
    setgid(Gid::from_raw(TARGET_GID))?;
    setuid(Uid::from_raw(TARGET_UID))?;
    set_no_new_privs()
}

#[cfg(target_os = "linux")]
fn set_no_new_privs() -> nix::Result<()> {
    nix::sys::prctl::set_no_new_privs()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn set_no_new_privs() -> nix::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn check_socket(path: &Path, group: Option<u32>, as_root: bool) -> Result<(), String> {
    use nix::unistd::{access, AccessFlags};

    if group.is_none() {
        return Ok(());
    }
    if access(path, AccessFlags::R_OK).is_ok() {
        return Ok(());
    }
    let group = group.unwrap_or_default();
    if as_root {
        Err(format!(
            "The Docker socket is not readable for uid 10001 with group {group}. Give the socket that group and mode 0660, or add the group to the container."
        ))
    } else {
        Err(
            "The Docker socket is not readable for uid 10001. Set runner.dockerSocket.group to the socket's gid, or give the mounted socket that group and mode 0660."
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entrypoint_flag_is_a_subcommand_after_the_program_name() {
        assert!(entrypoint_requested([
            "koreograf",
            "entrypoint",
            "koreograf"
        ]));
        assert!(!entrypoint_requested(["koreograf"]));
        assert!(!entrypoint_requested(["koreograf", "healthcheck"]));
        assert!(!entrypoint_requested(["koreograf", "run", "entrypoint"]));
    }

    fn scratch_ca(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-entrypoint-{name}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_git_bundle_lives_under_tmp() {
        assert_eq!(GIT_CA_BUNDLE, "/tmp/koreograf-ca-bundle.pem");
    }

    #[test]
    fn company_cas_are_appended_to_the_system_bundle_for_git() {
        let dir = scratch_ca("bundle");
        let ca = dir.join("ca");
        fs::create_dir_all(&ca).unwrap();
        fs::write(ca.join("company.pem"), crate::tls::TEST_CA_PEM).unwrap();
        let system = dir.join("system.pem");
        fs::write(&system, "SYSTEM ROOTS\n").unwrap();
        let out = dir.join("bundle.pem");
        let path = git_ca_bundle_from(&ca, &system, &out).unwrap().unwrap();
        assert_eq!(path, out);
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.starts_with("SYSTEM ROOTS\n"), "{text}");
        assert!(text.contains("BEGIN CERTIFICATE"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_or_missing_ca_directory_writes_no_bundle() {
        let dir = scratch_ca("empty");
        let out = dir.join("bundle.pem");
        let system = dir.join("system.pem");
        assert!(git_ca_bundle_from(&dir.join("missing"), &system, &out)
            .unwrap()
            .is_none());
        assert!(!out.exists());
        assert!(git_ca_bundle_from(&dir, &system, &out).unwrap().is_none());
        assert!(!out.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_socket_has_no_group() {
        let path = std::env::temp_dir().join("koreograf-missing-socket");
        let _ = fs::remove_file(&path);
        assert_eq!(socket_group(&path).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_reports_its_group() {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::net::UnixListener;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("koreograf-socket-{nanos}"));
        let listener = UnixListener::bind(&path).unwrap();
        let expected = fs::metadata(&path).unwrap().gid();
        assert_eq!(socket_group(&path).unwrap(), Some(expected));
        drop(listener);
        let _ = fs::remove_file(path);
    }
}
