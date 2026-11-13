use std::ffi::OsString;
use std::fs;
#[cfg(unix)]
use std::os::fd::AsFd;
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
    let base = std::env::var_os("SSL_CERT_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SYSTEM_CA_BUNDLE));
    git_ca_bundle_from(
        Path::new(crate::tls::CA_DIRECTORY),
        &base,
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
    if bundle.is_empty() {
        eprintln!(
            "The system CA bundle {} is missing or empty; company CAs are not added for git.",
            system_bundle.display()
        );
        return Ok(None);
    }
    if !bundle.ends_with(b"\n") {
        bundle.push(b'\n');
    }
    bundle.extend(loaded.pem);
    fs::write(out, bundle).map_err(|err| format!("Could not write {} ({err}).", out.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(out, fs::Permissions::from_mode(0o644))
            .map_err(|err| format!("Could not set the mode of {} ({err}).", out.display()))?;
    }
    Ok(Some(out.to_path_buf()))
}

#[cfg(unix)]
fn prepare_data_dir(path: &Path) -> Result<(), String> {
    use nix::sys::stat::fstat;
    use nix::unistd::{fchown, Gid, Uid};

    fs::create_dir_all(path)
        .map_err(|err| format!("Could not create the data directory ({err})."))?;
    let mut dir = open_dir(path)?;
    let device = fstat(dir.as_fd())
        .map_err(|err| format!("Could not read {} ({err}).", path.display()))?
        .st_dev;
    fchown(
        dir.as_fd(),
        Some(Uid::from_raw(TARGET_UID)),
        Some(Gid::from_raw(TARGET_GID)),
    )
    .map_err(|err| format!("Could not change the owner of {} ({err}).", path.display()))?;
    walk_tree(
        &mut dir,
        Some(device),
        TARGET_UID,
        TARGET_GID,
        &mut |_, _| {},
    )
}

#[cfg(unix)]
fn open_dir(path: &Path) -> Result<nix::dir::Dir, String> {
    use nix::fcntl::{openat, OFlag, AT_FDCWD};
    use nix::sys::stat::Mode;

    let fd = openat(
        AT_FDCWD,
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| format!("Could not open {} ({err}).", path.display()))?;
    nix::dir::Dir::from_fd(fd).map_err(|err| format!("Could not read {} ({err}).", path.display()))
}

#[cfg(unix)]
fn walk_tree<F>(
    dir: &mut nix::dir::Dir,
    device: Option<u64>,
    uid: u32,
    gid: u32,
    before_descend: &mut F,
) -> Result<(), String>
where
    F: FnMut(&nix::dir::Dir, &std::ffi::CStr),
{
    use nix::errno::Errno;
    use nix::fcntl::{openat, AtFlags, OFlag};
    use nix::sys::stat::{fstat, fstatat, Mode, SFlag};

    let names = list_names(dir)?;
    for name in names {
        let name = name.as_c_str();
        let metadata = match fstatat(dir.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Ok(metadata) => metadata,
            Err(Errno::ENOENT) => continue,
            Err(err) => {
                return Err(format!(
                    "Could not read {} ({err}).",
                    name.to_string_lossy()
                ))
            }
        };
        if device.is_some_and(|device| metadata.st_dev != device) {
            continue;
        }
        if metadata.st_uid != uid || metadata.st_gid != gid {
            if let Err(err) = chown_at(dir, name, uid, gid) {
                if err == Errno::ENOENT {
                    continue;
                }
                return Err(format!(
                    "Could not change the owner of {} ({err}).",
                    name.to_string_lossy()
                ));
            }
        }
        if SFlag::from_bits_truncate(metadata.st_mode) != SFlag::S_IFDIR {
            continue;
        }
        before_descend(dir, name);
        let child = match openat(
            dir.as_fd(),
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(child) => child,
            Err(err) if err == Errno::ELOOP || err == Errno::ENOTDIR || err == Errno::ENOENT => {
                continue;
            }
            Err(err) => {
                return Err(format!(
                    "Could not open {} ({err}).",
                    name.to_string_lossy()
                ))
            }
        };
        let child_device = fstat(&child)
            .map_err(|err| format!("Could not read {} ({err}).", name.to_string_lossy()))?
            .st_dev;
        if device.is_some_and(|device| child_device != device) {
            continue;
        }
        let mut child_dir = nix::dir::Dir::from_fd(child)
            .map_err(|err| format!("Could not read {} ({err}).", name.to_string_lossy()))?;
        walk_tree(&mut child_dir, device, uid, gid, before_descend)?;
    }
    Ok(())
}

#[cfg(unix)]
fn list_names(dir: &mut nix::dir::Dir) -> Result<Vec<std::ffi::CString>, String> {
    let mut names = Vec::new();
    for entry in dir.iter() {
        let entry = entry.map_err(|err| format!("Could not list a directory ({err})."))?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

#[cfg(unix)]
fn chown_at(dir: &nix::dir::Dir, name: &std::ffi::CStr, uid: u32, gid: u32) -> nix::Result<()> {
    use nix::fcntl::AtFlags;
    use nix::unistd::{fchownat, Gid, Uid};

    fchownat(
        dir.as_fd(),
        name,
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
        AtFlags::AT_SYMLINK_NOFOLLOW,
    )
}

#[cfg(unix)]
pub(crate) fn own_tree(path: &Path, uid: u32, gid: u32) -> Result<(), String> {
    use nix::unistd::{fchown, Gid, Uid};

    let mut dir = open_dir(path)?;
    fchown(
        dir.as_fd(),
        Some(Uid::from_raw(uid)),
        Some(Gid::from_raw(gid)),
    )
    .map_err(|err| format!("Could not change the owner of {} ({err}).", path.display()))?;
    walk_tree(&mut dir, None, uid, gid, &mut |_, _| {})
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o644, "bundle mode");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_system_bundle_keeps_git_on_its_own_store() {
        let dir = scratch_ca("no-system");
        let ca = dir.join("ca");
        fs::create_dir_all(&ca).unwrap();
        fs::write(ca.join("company.pem"), crate::tls::TEST_CA_PEM).unwrap();
        let out = dir.join("bundle.pem");
        let missing = dir.join("missing-system.pem");
        assert!(git_ca_bundle_from(&ca, &missing, &out).unwrap().is_none());
        assert!(!out.exists());
        let empty = dir.join("empty-system.pem");
        fs::write(&empty, "").unwrap();
        assert!(git_ca_bundle_from(&ca, &empty, &out).unwrap().is_none());
        assert!(!out.exists());
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
    fn require_root(test: &str) -> bool {
        if nix::unistd::Uid::effective().is_root() {
            return true;
        }
        eprintln!("skipping {test}: it needs euid 0 to change owners");
        false
    }

    #[cfg(unix)]
    fn set_owner(path: &Path, uid: u32, gid: u32) {
        use nix::fcntl::AtFlags;
        use nix::unistd::{fchownat, Gid, Uid};

        fchownat(
            nix::fcntl::AT_FDCWD,
            path,
            Some(Uid::from_raw(uid)),
            Some(Gid::from_raw(gid)),
            AtFlags::empty(),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn owner(path: &Path) -> (u32, u32) {
        use std::os::unix::fs::MetadataExt;

        let metadata = fs::symlink_metadata(path).unwrap();
        (metadata.uid(), metadata.gid())
    }

    #[cfg(unix)]
    #[test]
    fn the_walk_keeps_symlink_targets_outside_the_data_directory() {
        use std::os::unix::fs::symlink;

        if !require_root("the_walk_keeps_symlink_targets_outside_the_data_directory") {
            return;
        }
        let base = scratch_ca("symlink-walk");
        let root = base.join("data");
        let outside_dir = base.join("outside-dir");
        let outside_file = base.join("outside-file");
        fs::create_dir_all(root.join("team")).unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        fs::write(root.join("team/state"), "inside").unwrap();
        fs::write(&outside_file, "outside").unwrap();
        fs::write(outside_dir.join("secret"), "secret").unwrap();
        for path in [&outside_dir, &outside_file, &outside_dir.join("secret")] {
            set_owner(path, 12345, 12345);
        }
        symlink(&outside_dir, root.join("team/dir-link")).unwrap();
        symlink(&outside_file, root.join("team/file-link")).unwrap();

        prepare_data_dir(&root).unwrap();

        assert_eq!(owner(&outside_dir), (12345, 12345));
        assert_eq!(owner(&outside_file), (12345, 12345));
        assert_eq!(owner(&outside_dir.join("secret")), (12345, 12345));
        assert_eq!(owner(&root), (TARGET_UID, TARGET_GID));
        assert_eq!(owner(&root.join("team/state")), (TARGET_UID, TARGET_GID));
        assert_eq!(owner(&root.join("team/dir-link")), (TARGET_UID, TARGET_GID));
        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_swapped_for_a_symlink_is_not_followed() {
        use std::os::unix::fs::symlink;

        if !require_root("a_directory_swapped_for_a_symlink_is_not_followed") {
            return;
        }
        let base = scratch_ca("symlink-race");
        let root = base.join("data");
        let outside = base.join("outside");
        fs::create_dir_all(root.join("swap")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("swap/state"), "inside").unwrap();
        fs::write(outside.join("secret"), "secret").unwrap();
        set_owner(&outside, 12345, 12345);
        set_owner(&outside.join("secret"), 12345, 12345);

        let mut dir = open_dir(&root).unwrap();
        let device = nix::sys::stat::fstat(dir.as_fd()).unwrap().st_dev;
        let mut swapped = false;
        let mut hook = |_: &nix::dir::Dir, name: &std::ffi::CStr| {
            if name.to_bytes() == b"swap" {
                fs::remove_dir_all(root.join("swap")).unwrap();
                symlink(&outside, root.join("swap")).unwrap();
                swapped = true;
            }
        };

        walk_tree(&mut dir, Some(device), TARGET_UID, TARGET_GID, &mut hook).unwrap();

        assert!(swapped, "the hook did not replace the directory");
        assert_eq!(owner(&outside), (12345, 12345));
        assert_eq!(owner(&outside.join("secret")), (12345, 12345));
        let _ = fs::remove_dir_all(&base);
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
