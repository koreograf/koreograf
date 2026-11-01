use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::cacheinfo::dir_size;
use crate::config::{LocalJobs, RunnerMode};
use crate::db::{self, Environment, Project, Template};
use crate::metrics::{self, JobMeta};
use crate::Shared;

const WRAPPER: &str = include_str!("wrapper.sh");
const MAX_FILES: u32 = 4_000;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const DEFAULT_INVENTORY: &str = "[all]\nlocalhost ansible_connection=local\n";
/// Docker or Podman inspect can stall when the daemon does not answer.
/// The timeout does not abort the blocking thread.
const STOP_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes of pull stderr kept for the failure line. The tail holds the reason.
const PULL_STDERR_TAIL: usize = 8 * 1024;
/// Read errors tolerated before the pipe is closed. One error is not EOF.
const PULL_STDERR_READ_ERRORS: u32 = 8;

#[derive(Clone, Debug)]
pub struct RunningProc {
    pub pid: u32,
    pub container: Option<String>,
    pub bin: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ContainerSpec {
    pub image: String,
    pub name: String,
    /// Daemon path, guest path, and whether the mount is read-only.
    pub mounts: Vec<(String, String, bool)>,
    /// Client path. Docker and Podman read `--env-file` in this process.
    pub env_file: String,
    /// Numeric `uid:gid` of the job directory. The image user is not used.
    pub user: String,
    /// Docker or Podman `--network` value. Empty uses the daemon default.
    pub network: String,
    pub command: Vec<String>,
}

/// A CLI tool Koreograf can launch. Register it here and add a branch in `wrapper.sh`.
#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub id: &'static str,
    pub summary: &'static str,
}

const TOOLS: &[Tool] = &[
    Tool {
        id: "ansible",
        summary: "ansible_playbook_collections_and_roles_stay_in_the",
    },
    Tool {
        id: "opentofu",
        summary: "opentofu_init_then_plan_or_apply_providers_stay",
    },
    Tool {
        id: "shell",
        summary: "shell_script_from_the_project_or_as_an",
    },
];

pub fn tools() -> &'static [Tool] {
    TOOLS
}

pub fn known_tool(tool: &str) -> bool {
    TOOLS.iter().any(|item| item.id == tool)
}

pub fn check_ansible_spec(spec: &Value) -> Result<(), String> {
    if !spec.is_object() {
        return Err("spec_must_be_an_object".into());
    }
    match spec.get("job_type") {
        None | Some(Value::Null) => {}
        Some(Value::String(job_type)) if job_type == "run" || job_type == "check" => {}
        _ => return Err("type_is_run_or_check".into()),
    }
    check_tag_list(spec, "tags", "tags")?;
    check_tag_list(spec, "skip_tags", "skip_tags")?;
    if let Some(limit) = optional_text(spec, "limit")? {
        check_limit(&limit)?;
    }
    if let Some(diff) = spec.get("diff") {
        if !diff.is_boolean() {
            return Err("diff_is_yes_or_no".into());
        }
    }
    if let Some(verbosity) = optional_u64(spec, "verbosity")? {
        if verbosity > 4 {
            return Err("verbosity_is_0_to_4".into());
        }
    }
    if let Some(forks) = optional_u64(spec, "forks")? {
        if !(1..=50).contains(&forks) {
            return Err("forks_is_a_number_from_1_to_50".into());
        }
    }
    Ok(())
}

const TAG_FILTER_WARNING: &str =
    "Tag filter matched no tasks. Ansible exited 0, so this job stays successful.";

fn tag_filter_is_set(spec: &Value) -> bool {
    ["tags", "skip_tags"].into_iter().any(|key| {
        spec.get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    })
}

pub fn ansible_env(spec: &Value) -> Result<Vec<(String, String)>, String> {
    check_ansible_spec(spec)?;
    let mut env = Vec::new();
    if spec.get("job_type").and_then(Value::as_str) == Some("check") {
        env.push(("KOREOGRAF_CHECK".into(), "1".into()));
    }
    if spec.get("diff").and_then(Value::as_bool) == Some(true) {
        env.push(("KOREOGRAF_DIFF".into(), "1".into()));
    }
    for (key, var) in [
        ("tags", "KOREOGRAF_TAGS"),
        ("skip_tags", "KOREOGRAF_SKIP_TAGS"),
        ("limit", "KOREOGRAF_LIMIT"),
    ] {
        if let Some(text) = spec.get(key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                env.push((var.into(), text.to_string()));
            }
        }
    }
    if let Some(verbosity) = spec.get("verbosity").and_then(Value::as_u64) {
        if verbosity > 0 {
            env.push(("KOREOGRAF_VERBOSITY".into(), verbosity.to_string()));
        }
    }
    if let Some(forks) = spec.get("forks").and_then(Value::as_u64) {
        env.push(("KOREOGRAF_FORKS".into(), forks.to_string()));
    }
    Ok(env)
}

fn optional_text(spec: &Value, key: &str) -> Result<Option<String>, String> {
    match spec.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(crate::error::message("field.text", &[("field", key)])),
    }
}

fn optional_u64(spec: &Value, key: &str) -> Result<Option<u64>, String> {
    match spec.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| crate::error::message("field.integer", &[("field", key)])),
        Some(_) => Err(crate::error::message("field.integer", &[("field", key)])),
    }
}

fn check_tag_list(spec: &Value, key: &str, label: &str) -> Result<(), String> {
    let Some(text) = optional_text(spec, key)? else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }
    if text.len() > 300 {
        return Err(crate::error::message("field.too_long", &[("field", label)]));
    }
    for part in text.split(',') {
        let part = part.trim();
        let token = !part.is_empty()
            && part.len() <= 80
            && !part.starts_with('-')
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-'));
        if !token {
            return Err(crate::error::message(
                "field.part_invalid",
                &[("field", label), ("part", part)],
            ));
        }
    }
    Ok(())
}

fn check_limit(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let allowed = text.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '_' | '.' | ':' | '-' | '*' | ',' | '!' | '&' | '[' | ']' | '@' | ' '
            )
    });
    if text.len() > 4_000 || text.starts_with('-') || !allowed {
        return Err("limit_contains_an_invalid_character".into());
    }
    Ok(())
}

/// Hosts Ansible reported as failed or unreachable.
/// A play recap replaces earlier `fatal:` lines. Without a recap, those lines stay.
pub fn observe_host_line(stored: &str, line: &str) -> Option<String> {
    let mut record = host_record(stored);
    if !apply_host_line(&mut record, line) {
        return None;
    }
    Some(
        serde_json::json!({
            "hosts": record.hosts,
            "recap": record.recap,
        })
        .to_string(),
    )
}

pub fn failed_host_names(stored: &str) -> Vec<String> {
    host_record(stored).hosts
}

pub fn limit_from_hosts(hosts: &[String]) -> Result<String, String> {
    if hosts.is_empty() {
        return Err("no_failed_hosts_are_stored_for_this_job".into());
    }
    for host in hosts {
        let token = !host.is_empty()
            && host.len() <= 253
            && !host.starts_with('-')
            && host
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-' | ':'));
        if !token {
            return Err(crate::error::message(
                "limit.host_invalid",
                &[("host", host)],
            ));
        }
    }
    let limit = hosts.join(",");
    check_limit(&limit)?;
    Ok(limit)
}

pub fn parameters_with_limit(parameters: &str, limit: &str) -> Result<String, String> {
    check_limit(limit)?;
    let mut params = db::json_object(parameters);
    {
        let Some(root) = params.as_object_mut() else {
            return Err("parameters_are_invalid".into());
        };
        let spec = root
            .entry("spec")
            .or_insert_with(|| Value::Object(Default::default()));
        if !spec.is_object() {
            *spec = Value::Object(Default::default());
        }
        let Some(spec) = spec.as_object_mut() else {
            return Err("parameters_are_invalid".into());
        };
        spec.insert("limit".into(), Value::String(limit.to_string()));
    }
    check_ansible_spec(&params["spec"])?;
    Ok(params.to_string())
}

struct HostRecord {
    hosts: Vec<String>,
    recap: bool,
}

fn host_record(stored: &str) -> HostRecord {
    let value: Value = serde_json::from_str(stored).unwrap_or(Value::Null);
    match value {
        Value::Array(items) => HostRecord {
            hosts: host_strings(&items),
            recap: false,
        },
        Value::Object(map) => {
            let hosts = map
                .get("hosts")
                .and_then(Value::as_array)
                .map(|items| host_strings(items))
                .unwrap_or_default();
            HostRecord {
                hosts,
                recap: map.get("recap").and_then(Value::as_bool).unwrap_or(false),
            }
        }
        _ => HostRecord {
            hosts: Vec::new(),
            recap: false,
        },
    }
}

fn host_strings(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect()
}

fn apply_host_line(record: &mut HostRecord, line: &str) -> bool {
    let line = line.trim();
    if let Some((host, failed, unreachable)) = recap_host(line) {
        let mut changed = false;
        if !record.recap {
            record.recap = true;
            record.hosts.clear();
            changed = true;
        }
        return set_host(record, &host, failed > 0 || unreachable > 0) || changed;
    }
    if record.recap {
        return false;
    }
    fatal_host(line).is_some_and(|host| set_host(record, &host, true))
}

fn set_host(record: &mut HostRecord, host: &str, failed: bool) -> bool {
    let present = record.hosts.iter().any(|item| item == host);
    if failed && !present {
        record.hosts.push(host.to_string());
        true
    } else if !failed && present {
        record.hosts.retain(|item| item != host);
        true
    } else {
        false
    }
}

fn recap_host(line: &str) -> Option<(String, u64, u64)> {
    let marker = line.find(" : ok=").or_else(|| line.find(": ok="))?;
    let host = line[..marker].trim();
    if host.is_empty() || host.contains(' ') {
        return None;
    }
    let failed = recap_count(line, "failed")?;
    let unreachable = recap_count(line, "unreachable").unwrap_or(0);
    Some((host.to_string(), failed, unreachable))
}

fn recap_count(line: &str, key: &str) -> Option<u64> {
    let token = format!("{key}=");
    let start = line.find(&token)? + token.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn fatal_host(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("fatal: [")?;
    let (host, tail) = rest.split_once(']')?;
    if !(tail.contains("FAILED!") || tail.contains("UNREACHABLE!")) {
        return None;
    }
    let host = host.trim();
    if host.is_empty() || host.contains(['\n', '\r']) {
        return None;
    }
    Some(host.to_string())
}

pub fn check_relative(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    if path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err("the_entry_path_must_be_relative_and_must".into());
    }
    Ok(())
}

pub fn merge_vars(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(left), Value::Object(right)) => {
            let mut out = left.clone();
            for (key, value) in right {
                out.insert(key.clone(), value.clone());
            }
            Value::Object(out)
        }
        (_, right) if !right.is_null() => right.clone(),
        (left, _) => left.clone(),
    }
}

pub fn container_argv(spec: &ContainerSpec) -> Vec<String> {
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "--name".into(),
        spec.name.clone(),
        "--label".into(),
        "koreograf=1".into(),
    ];
    // `--mount type=bind` fails when the source is missing.
    // `-v` would create that directory on the daemon's host.
    for (host, guest, readonly) in &spec.mounts {
        let mut mount = format!("type=bind,source={host},target={guest}");
        if *readonly {
            mount.push_str(",readonly");
        }
        args.push("--mount".into());
        args.push(mount);
    }
    args.push("--env-file".into());
    args.push(spec.env_file.clone());
    // The image user (awx-ee is uid 1000) cannot write a root-owned work directory.
    // A root process still gets no capabilities and cannot gain new ones.
    args.push("--user".into());
    args.push(spec.user.clone());
    args.push("--security-opt".into());
    args.push("no-new-privileges".into());
    args.push("--cap-drop".into());
    args.push("ALL".into());
    if !spec.network.is_empty() {
        args.push("--network".into());
        args.push(spec.network.clone());
    }
    args.push("-w".into());
    args.push("/runner/project".into());
    args.push("--entrypoint".into());
    args.push("sh".into());
    args.push(spec.image.clone());
    args.extend(spec.command.iter().cloned());
    args
}

/// Owner of the job directory. Secret files in that directory stay mode 0600
/// and stay owned by this user, so the container has to be the same user.
fn directory_user(path: &Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(path).map_err(|err| {
            format!("The job directory could not be read ({err}). The job was not started.")
        })?;
        Ok(format!("{}:{}", meta.uid(), meta.gid()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok("0:0".into())
    }
}

/// Rootless Podman and Docker map the host user to container uid 0.
/// Passing the host uid selects a subordinate id that cannot read the job files.
fn container_run_user(owner: &str, rootless: bool) -> String {
    if rootless {
        "0:0".into()
    } else {
        owner.to_string()
    }
}

fn info_is_rootless(bin: &str, text: &str) -> bool {
    let text = text.trim();
    if bin == "podman" {
        text.eq_ignore_ascii_case("true")
    } else {
        text.to_ascii_lowercase().contains("name=rootless")
    }
}

async fn runtime_is_rootless(bin: &str) -> Result<bool, String> {
    let format = if bin == "podman" {
        "{{.Host.Security.Rootless}}"
    } else {
        "{{json .SecurityOptions}}"
    };
    let result = tokio::time::timeout(
        STOP_COMMAND_TIMEOUT,
        Command::new(bin)
            .args(["info", "--format", format])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;
    let output = match result {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return Err(format!(
                "The container runtime did not answer ({err}). The job was not started."
            ));
        }
        Err(_) => {
            return Err("The container runtime did not answer. The job was not started.".into());
        }
    };
    if !output.status.success() {
        let detail = last_stderr_line(&String::from_utf8_lossy(&output.stderr));
        return Err(if detail.is_empty() {
            "The container runtime did not answer. The job was not started.".into()
        } else {
            format!("The container runtime did not answer ({detail}). The job was not started.")
        });
    }
    Ok(info_is_rootless(
        bin,
        &String::from_utf8_lossy(&output.stdout),
    ))
}

async fn command_ok(bin: &str, args: &[&str], limit: Duration) -> bool {
    let mut child = match Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) => false,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            false
        }
    }
}

pub async fn detect_runtime() -> Option<String> {
    for bin in ["docker", "podman"] {
        if command_ok(bin, &["version"], STOP_COMMAND_TIMEOUT).await {
            return Some(bin.to_string());
        }
    }
    None
}

pub fn signal_stop(state: &Shared, job_id: &str, proc: RunningProc) {
    let state = state.clone();
    let job_id = job_id.to_string();
    tokio::spawn(async move {
        if !deliver_stop(proc.clone(), false, true).await {
            log(
                &state,
                &job_id,
                "system",
                "The stop signal could not be delivered.",
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        // The leader often exits on SIGTERM. A child that ignores it and
        // closes the job pipes is no longer remembered, but its process
        // group can still be alive. A pid is signaled only when that group
        // still exists.
        let still_ours = remembered(&state, &job_id).is_some_and(|current| current.pid == proc.pid);
        if !stop_after_grace(&proc, still_ours).await {
            log(
                &state,
                &job_id,
                "system",
                "The stop signal could not be delivered.",
            );
        }
    });
}

async fn deliver_stop(proc: RunningProc, kill_hard: bool, signal_local: bool) -> bool {
    blocking_stop(move || deliver_stop_blocking(&proc, kill_hard, signal_local)).await
}

async fn stop_after_grace(proc: &RunningProc, still_ours: bool) -> bool {
    let proc = proc.clone();
    // Check the group and signal it in one blocking call. The pid is not
    // reused while a member of that group is still alive.
    blocking_stop(move || {
        let local_ok = if proc.pid > 1 && (still_ours || process_group_alive(proc.pid)) {
            signal_process_group(proc.pid, true)
        } else {
            proc.pid > 1 || proc.container.is_some()
        };
        let container_ok = match (proc.bin.as_deref(), proc.container.as_deref()) {
            (Some(bin), Some(name)) => runtime_signal(bin, name, "KILL"),
            _ => true,
        };
        local_ok && container_ok
    })
    .await
}

async fn blocking_stop(work: impl FnOnce() -> bool + Send + 'static) -> bool {
    match tokio::time::timeout(STOP_COMMAND_TIMEOUT, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(delivered)) => delivered,
        _ => false,
    }
}

fn deliver_stop_blocking(proc: &RunningProc, kill_hard: bool, signal_local: bool) -> bool {
    let signal = if kill_hard { "KILL" } else { "TERM" };
    // The local group goes first. Inspect can stall, and this thread keeps
    // running after the wait around it times out.
    let local_ok = if !signal_local {
        true
    } else if proc.pid > 1 {
        signal_process_group(proc.pid, kill_hard)
    } else {
        proc.container.is_some()
    };
    let container_ok = match (proc.bin.as_deref(), proc.container.as_deref()) {
        (Some(bin), Some(name)) => runtime_signal(bin, name, signal),
        _ => true,
    };
    local_ok && container_ok
}

fn signal_process_group(pid: u32, kill_hard: bool) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    let signal = if kill_hard {
        nix::sys::signal::Signal::SIGKILL
    } else {
        nix::sys::signal::Signal::SIGTERM
    };
    match nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), signal) {
        Ok(()) => true,
        Err(nix::errno::Errno::ESRCH) => true,
        Err(_) => false,
    }
}

/// `true` when this process may signal `pid`'s group. `EPERM` means the
/// group exists and belongs to someone else, so it must not be killed.
fn process_group_alive(pid: u32) -> bool {
    let Ok(raw) = i32::try_from(pid) else {
        return false;
    };
    if raw <= 1 {
        return false;
    }
    nix::sys::signal::killpg(nix::unistd::Pid::from_raw(raw), None).is_ok()
}

fn runtime_signal(bin: &str, name: &str, signal: &str) -> bool {
    match container_state(bin, name) {
        ContainerPresence::Stopped => true,
        ContainerPresence::Unknown => false,
        ContainerPresence::Running => std::process::Command::new(bin)
            .args(["kill", "--signal", signal, name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false),
    }
}

enum ContainerPresence {
    Running,
    Stopped,
    Unknown,
}

fn container_state(bin: &str, name: &str) -> ContainerPresence {
    let output = match std::process::Command::new(bin)
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
    {
        Ok(output) => output,
        Err(_) => return ContainerPresence::Unknown,
    };
    classify_inspect(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

fn classify_inspect(success: bool, stdout: &str, stderr: &str) -> ContainerPresence {
    if !success {
        let err = stderr.to_ascii_lowercase();
        if err.contains("no such object") || err.contains("no such container") {
            return ContainerPresence::Stopped;
        }
        return ContainerPresence::Unknown;
    }
    if stdout.trim() == "true" {
        ContainerPresence::Running
    } else {
        ContainerPresence::Stopped
    }
}

pub async fn execute(state: Shared, job_id: String) {
    let started = match state.db() {
        Ok(db) => db::mark_running(&db, &job_id).unwrap_or(false),
        Err(_) => false,
    };
    if !started {
        // This task never stored a process. A duplicate start must not clear
        // a run that is already in progress.
        let finished = state.db().ok().and_then(|db| {
            db::job(&db, &job_id)
                .ok()
                .flatten()
                .map(|job| matches!(job.status.as_str(), "successful" | "failed" | "canceled"))
        });
        if finished == Some(true) {
            if let Ok(mut canceled) = state.canceled.lock() {
                canceled.remove(&job_id);
            }
            // Cancel can win before this task marks the job running.
            close_observation(&state, &job_id);
        }
        return;
    }
    match prepare_and_run(&state, &job_id).await {
        Ok(()) => {}
        Err(err) => {
            log(&state, &job_id, "system", &err);
            finish(&state, &job_id, false);
        }
    }
    close_observation(&state, &job_id);
    forget_run(&state, &job_id);
}

async fn prepare_and_run(state: &Shared, job_id: &str) -> Result<(), String> {
    let (job, template) = {
        let db = state.db().map_err(|e| e.to_string())?;
        let job = db::job(&db, job_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Job is missing.".to_string())?;
        let template = match &job.template_id {
            Some(id) => db::template(&db, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Job template is missing.".to_string())?,
            None => return Err("Job has no template.".into()),
        };
        (job, template)
    };
    let original = template.clone();
    let template = with_saved_parameters(template, &job);
    if !known_tool(&template.tool) {
        return Err(format!("Unknown tool {}.", template.tool));
    }
    check_relative(&template.entrypoint)?;

    let work = state.cfg.data_dir.join("jobs").join(job_id);
    let stage = work.join("project");
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let _scrub = SecretScrub { work: work.clone() };
    let cache = state.cfg.data_dir.join("cache").join(&job.team_id);
    for sub in ["collections", "roles", "tofu", "tofu-state"] {
        let path = cache.join(sub);
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        if sub == "tofu-state" {
            set_private(&path)?;
        }
    }

    let settings = crate::settings::load(state).map_err(|err| err.to_string())?;
    if settings.proxy.explicit && settings.proxy.is_set() {
        log(
            state,
            job_id,
            "system",
            &format!("HTTP proxy {}.", settings.proxy.url),
        );
    } else if !settings.proxy.explicit && !settings.proxy.job_pairs().is_empty() {
        log(state, job_id, "system", "HTTP proxy from the environment.");
    }
    let project = load_project(state, &template).await?;
    if let Some(project) = &project {
        let source = materialize_project(
            project,
            &state.cfg.data_dir,
            &settings.proxy,
            template.update_on_launch,
            &state.cfg.project_roots,
        )
        .await
        .map_err(sync_log_line)?;
        if project.source_type == "git" && !template.update_on_launch {
            log(
                state,
                job_id,
                "system",
                "Using the checkout from the last sync.",
            );
        }
        copy_tree(&source, &stage)?;
    }

    let mut entrypoint = template.entrypoint.clone();
    let spec: Value = serde_json::from_str(&template.spec).unwrap_or(Value::Null);
    if template.tool == "shell" && entrypoint.is_empty() {
        let script = spec
            .get("script")
            .and_then(Value::as_str)
            .ok_or_else(|| "A shell template needs a script or an entry path.".to_string())?;
        fs::write(stage.join("inline.sh"), script).map_err(|e| e.to_string())?;
        entrypoint = "inline.sh".into();
    }
    if entrypoint.is_empty() {
        return Err("Entry path is missing.".into());
    }
    check_relative(&entrypoint)?;

    let inventory_rel = write_job_inventory(state, &template, &work)?;
    fs::write(work.join("wrapper.sh"), WRAPPER).map_err(|e| e.to_string())?;

    let saved = db::json_object(&job.parameters)
        .get("tool")
        .and_then(Value::as_str)
        .is_some();
    let mut extra = if saved {
        db::json_object(&template.extra_vars)
    } else {
        merge_vars(
            &db::json_object(&template.extra_vars),
            &db::json_object(&job.extra_vars),
        )
    };
    let use_container = should_container(settings.runner, template.environment_id.is_some());
    let runtime = if use_container {
        detect_runtime().await
    } else {
        None
    };
    let environment = if template.environment_id.is_some() {
        load_environment(state, &template)?
    } else {
        None
    };

    let container_mode = match (use_container, runtime.clone(), environment.clone()) {
        (true, Some(bin), Some(env)) => Some((bin, env)),
        (true, None, Some(_)) if settings.runner == RunnerMode::Docker => {
            return Err(
                "An execution environment is set, but neither Docker nor Podman is reachable."
                    .into(),
            );
        }
        (true, None, Some(_)) => {
            log(state, job_id, "system", AUTO_WITHOUT_RUNTIME);
            None
        }
        _ => {
            if settings.runner == RunnerMode::Local && template.environment_id.is_some() {
                log(
                    state,
                    job_id,
                    "system",
                    "Runner mode is local. The execution environment is ignored.",
                );
            }
            None
        }
    };

    let view_work = if container_mode.is_some() {
        "/runner/work".to_string()
    } else {
        work.display().to_string()
    };
    if let Value::Object(map) = &mut extra {
        map.insert("koreograf_work".into(), Value::String(view_work.clone()));
    }
    let extra_file = write_extra_file(&work, &template.tool, &extra)?;

    let mut env = Vec::new();
    export_plain_vars(&extra, &mut env);
    env.extend(base_env(container_mode.is_some(), &work));
    env.push(("KOREOGRAF_TOOL".into(), template.tool.clone()));
    env.push(("KOREOGRAF_ENTRYPOINT".into(), entrypoint.clone()));
    if template.tool == "ansible" {
        let flags = if spec.is_object() {
            ansible_env(&spec)?
        } else {
            Vec::new()
        };
        env.extend(flags);
    }
    env.push((
        "KOREOGRAF_ACTION".into(),
        spec.get("action")
            .and_then(Value::as_str)
            .unwrap_or("plan")
            .to_string(),
    ));
    push_paths(
        &mut env,
        container_mode.is_some(),
        &work,
        &stage,
        &cache,
        &inventory_rel,
        &extra_file,
    );
    let tofu_state = if template.tool == "opentofu" {
        let state = tofu_state_dir(&cache, &template.id)?;
        fs::create_dir_all(&state).map_err(|e| e.to_string())?;
        if let Some(parent) = state.parent() {
            set_private(parent)?;
        }
        set_private(&state)?;
        let visible = if container_mode.is_some() {
            format!("/cache/tofu-state/{}", template.id)
        } else {
            state.display().to_string()
        };
        env.push(("KOREOGRAF_TOFU_STATE".into(), visible));
        Some(state)
    } else {
        None
    };
    let credential_ids = saved_credential_ids(&job, &template);
    apply_credentials(
        state,
        &template,
        &credential_ids,
        &work,
        container_mode.is_some(),
        &mut env,
    )?;
    apply_proxy_env(&mut env, &settings.proxy);

    if canceled(state, job_id) {
        return Ok(());
    }

    state.metrics.begin(&JobMeta {
        job_id,
        team_id: &job.team_id,
        tool: &template.tool,
        playbook: &entrypoint,
        template: &template.name,
    });
    metrics::log_cache(&state.cfg.data_dir, job_id, &job.team_id);

    let cache_before = dir_size(&cache);
    log(
        state,
        job_id,
        "system",
        &format!(
            "Tool {} · cache before {} bytes",
            template.tool, cache_before
        ),
    );

    let exit = if let Some((bin, environment)) = container_mode {
        run_container(
            state,
            job_id,
            ContainerRun {
                bin: &bin,
                environment: &environment,
                work: &work,
                stage: &stage,
                cache: &cache,
                env: &env,
                timeout_secs: settings.job_timeout_secs,
                output_lines: settings.job_output_lines,
                tofu_state: tofu_state.as_deref(),
                template_id: &template.id,
            },
        )
        .await?
    } else {
        if state.cfg.local_jobs == LocalJobs::Admins {
            let allowed = {
                let db = state.db().map_err(|e| e.to_string())?;
                let launcher = crate::auth::launcher_is_admin(&db, &job.launched_by)
                    .map_err(|e| e.to_string())?;
                let params = db::json_object(&job.parameters);
                let matches = crate::auth::command_matches_template(&original, &params);
                let workflow_id = match job.workflow_job_id.as_deref().filter(|id| !id.is_empty()) {
                    Some(id) => db::workflow_job(&db, id)
                        .map_err(|e| e.to_string())?
                        .and_then(|run| run.workflow_id),
                    None => None,
                };
                let requires_workflow = job
                    .workflow_job_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                    || !matches;
                let content = crate::auth::content_saved_by_admin(
                    &db,
                    &template,
                    workflow_id.as_deref(),
                    requires_workflow,
                )
                .map_err(|e| e.to_string())?;
                launcher && content
            };
            if !allowed {
                return Err("Local runs are limited to administrators.".into());
            }
        }
        log(state, job_id, "system", "Local run on the controller.");
        let mut cmd = Command::new("sh");
        cmd.env_clear()
            .arg(work.join("wrapper.sh"))
            .current_dir(&work)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().map_err(|e| format!("Start failed: {e}"))?;
        let pid = child.id().unwrap_or(0);
        let proc = RunningProc {
            pid,
            container: None,
            bin: None,
        };
        remember(state, job_id, proc.clone());
        // The job log keeps this pid. The cancel test reads the group from it.
        log(state, job_id, "system", &format!("Process group {pid}."));
        if canceled(state, job_id) {
            signal_stop(state, job_id, proc);
        }
        wait_child(
            state,
            job_id,
            child,
            settings.job_timeout_secs,
            settings.job_output_lines,
        )
        .await?
    };

    if canceled(state, job_id) {
        return Ok(());
    }
    let cache_after = dir_size(&cache);
    log(
        state,
        job_id,
        "system",
        &format!("Cache afterwards {cache_after} bytes (before {cache_before})."),
    );
    if cache_after == cache_before && cache_before > 0 {
        log(state, job_id, "system", "Cache reused, no new growth.");
    } else if cache_after > cache_before {
        log(
            state,
            job_id,
            "system",
            "Cache grew. The next run uses these artifacts.",
        );
    }
    if exit == 0
        && template.tool == "ansible"
        && tag_filter_is_set(&spec)
        && state.metrics.tag_filter_matched_nothing(job_id)
    {
        log(state, job_id, "system", TAG_FILTER_WARNING);
    }
    finish(state, job_id, exit == 0);
    Ok(())
}

const AUTO_WITHOUT_RUNTIME: &str = "No container runtime is reachable. The run stays on the controller. Mount the Docker socket, or set the runner mode to docker to fail instead.";

fn should_container(mode: RunnerMode, has_environment: bool) -> bool {
    has_environment && mode != RunnerMode::Local
}

fn base_env(container: bool, work: &Path) -> Vec<(String, String)> {
    let mut env = vec![
        ("LANG".into(), "C.UTF-8".into()),
        (
            "HOME".into(),
            if container {
                "/runner/work".into()
            } else {
                work.display().to_string()
            },
        ),
    ];
    if !container {
        env.push((
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| {
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()
            }),
        ));
    }
    env
}

fn push_paths(
    env: &mut Vec<(String, String)>,
    container: bool,
    work: &Path,
    stage: &Path,
    cache: &Path,
    inventory_rel: &str,
    extra_file: &str,
) {
    let (project, work_s, cols, roles, tofu, inventory, extra) = if container {
        (
            "/runner/project".into(),
            "/runner/work".into(),
            "/cache/collections".into(),
            "/cache/roles".into(),
            "/cache/tofu".into(),
            format!("/runner/work/{inventory_rel}"),
            format!("/runner/work/{extra_file}"),
        )
    } else {
        (
            stage.display().to_string(),
            work.display().to_string(),
            cache.join("collections").display().to_string(),
            cache.join("roles").display().to_string(),
            cache.join("tofu").display().to_string(),
            work.join(inventory_rel).display().to_string(),
            work.join(extra_file).display().to_string(),
        )
    };
    env.push(("KOREOGRAF_PROJECT".into(), project));
    env.push(("KOREOGRAF_WORK".into(), work_s));
    env.push(("KOREOGRAF_COLLECTIONS".into(), cols));
    env.push(("KOREOGRAF_ROLES".into(), roles));
    env.push(("KOREOGRAF_TOFU".into(), tofu));
    env.push(("KOREOGRAF_INVENTORY".into(), inventory));
    env.push(("KOREOGRAF_EXTRA".into(), extra));
}

/// State for one template, under that team's cache. The plugin cache stays
/// in `cache/<team>/tofu`. A second team gets a different directory.
fn tofu_state_dir(cache: &Path, template_id: &str) -> Result<PathBuf, String> {
    if !safe_cache_key(template_id) {
        return Err("The template id cannot be used as an OpenTofu state directory.".into());
    }
    Ok(cache.join("tofu-state").join(template_id))
}

fn set_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn safe_cache_key(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= 80
        && (first.is_ascii_alphanumeric() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn apply_proxy_env(env: &mut Vec<(String, String)>, proxy: &crate::settings::ProxySettings) {
    for (key, value) in proxy.job_pairs() {
        if let Some(slot) = env.iter_mut().find(|(name, _)| name == &key) {
            slot.1 = value;
        } else {
            env.push((key, value));
        }
    }
}

fn export_plain_vars(extra: &Value, env: &mut Vec<(String, String)>) {
    let Some(map) = extra.as_object() else {
        return;
    };
    for (key, value) in map {
        if !safe_ident(key) || crate::credentials::reserved_env_name(key) {
            continue;
        }
        if let Some(text) = value.as_str() {
            if !text.contains('\n') && !text.contains('\0') {
                env.push((key.clone(), text.to_string()));
            }
        }
    }
}

fn safe_ident(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn saved_credential_ids(job: &db::Job, template: &Template) -> Vec<String> {
    let params = db::json_object(&job.parameters);
    if let Some(list) = params.get("credentials").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
            })
            .collect();
    }
    template.credential_id.clone().into_iter().collect()
}

fn apply_credentials(
    state: &Shared,
    template: &Template,
    ids: &[String],
    work: &Path,
    container: bool,
    env: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let mut used_files = Vec::new();
    let mut vault_lines = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let credential = {
            let db = state.db().map_err(|e| e.to_string())?;
            db::credential(&db, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Credential is missing.".to_string())?
        };
        if credential.team_id != template.team_id {
            return Err("The credential belongs to another team.".into());
        }
        let plain = state
            .decrypt(&credential.secret_blob)
            .map_err(|_| "Credentials cannot be read.".to_string())?;
        let secret: Value =
            serde_json::from_str(&plain).map_err(|_| "Credentials are not JSON.".to_string())?;
        let prepared =
            crate::credentials::prepare(credential.kind.as_str(), &secret, container, work)?;
        let mut part = prepared.env;
        if credential.kind == "vault" {
            part.retain(|(key, _)| key != "KOREOGRAF_VAULT_FILE" && key != "KOREOGRAF_VAULT_ID");
        }
        let mut vault_name = None;
        for (name, body) in prepared.files {
            let unique = unique_file_name(&used_files, &name, index);
            if unique != name {
                for (_, value) in &mut part {
                    if let Some(prefix) = value.strip_suffix(&name) {
                        *value = format!("{prefix}{unique}");
                    }
                }
            }
            if name == "vault.pass" {
                vault_name = Some(unique.clone());
            }
            used_files.push(unique.clone());
            write_secret(&work.join(&unique), &body)?;
        }
        if credential.kind == "vault" {
            let name = vault_name.ok_or_else(|| "Vault file is missing.".to_string())?;
            let label = secret
                .get("vault_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let path = guest_path(container, work, &name);
            vault_lines.push(vault_list_line(label, &path)?);
        }
        env.extend(part);
    }
    if !vault_lines.is_empty() {
        write_secret(&work.join("vault.list"), &vault_lines.join("\n"))?;
        env.push((
            "KOREOGRAF_VAULT_LIST".into(),
            guest_path(container, work, "vault.list"),
        ));
    }
    Ok(())
}

fn unique_file_name(used: &[String], name: &str, index: usize) -> String {
    if !used.iter().any(|item| item == name) {
        return name.to_string();
    }
    let candidate = format!("{index}-{name}");
    if !used.iter().any(|item| item == &candidate) {
        return candidate;
    }
    format!("{index}-{name}-{}", used.len())
}

/// One vault password on a line. The field separator is `|` because a tab is
/// IFS whitespace and `read` would discard an empty vault id.
fn vault_list_line(label: &str, path: &str) -> Result<String, String> {
    if label.contains(['|', '\t', '\n', '\r', '@']) || path.contains(['|', '\n', '\r']) {
        return Err("Vault id is invalid.".into());
    }
    Ok(format!("{label}|{path}"))
}

fn guest_path(container: bool, work: &Path, name: &str) -> String {
    if container {
        format!("/runner/work/{name}")
    } else {
        work.join(name).display().to_string()
    }
}

fn write_secret(path: &Path, contents: &str) -> Result<(), String> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(contents.as_bytes())
        .map_err(|e| e.to_string())?;
    if !contents.ends_with('\n') {
        file.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    if let Err(err) = record_secret_file(path) {
        let _ = fs::remove_file(path);
        return Err(err);
    }
    Ok(())
}

fn record_secret_file(path: &Path) -> Result<(), String> {
    let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
        return Err("Secret file name is invalid.".into());
    };
    if !safe_manifest_name(name) {
        return Err("Secret file name is invalid.".into());
    }
    let Some(parent) = path.parent() else {
        return Err("Secret file name is invalid.".into());
    };
    let manifest = parent.join(SECRET_MANIFEST);
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&manifest).map_err(|e| e.to_string())?;
    use std::io::Write;
    writeln!(file, "{name}").map_err(|e| e.to_string())
}

async fn load_project(state: &Shared, template: &Template) -> Result<Option<Project>, String> {
    let Some(id) = &template.project_id else {
        return Ok(None);
    };
    let db = state.db().map_err(|e| e.to_string())?;
    let project = db::project(&db, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Project is missing.".to_string())?;
    let visible = db::project_visible(&db, id, &template.team_id).map_err(|e| e.to_string())?;
    if !visible {
        return Err("This project is not visible to this team.".into());
    }
    Ok(Some(project))
}

fn write_job_inventory(state: &Shared, template: &Template, work: &Path) -> Result<String, String> {
    let Some(id) = &template.inventory_id else {
        fs::write(work.join("inventory"), DEFAULT_INVENTORY).map_err(|err| err.to_string())?;
        return Ok("inventory".into());
    };
    let db = state.db().map_err(|e| e.to_string())?;
    let inventory = db::inventory(&db, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Inventory is missing.".to_string())?;
    if inventory.team_id != template.team_id {
        return Err("The inventory belongs to another team.".into());
    }
    let sources = db::list_inventory_sources(&db, id).map_err(|e| e.to_string())?;
    if sources.len() <= 1 {
        let body = sources
            .first()
            .map(|source| source.body.as_str())
            .filter(|body| !body.trim().is_empty())
            .unwrap_or(inventory.body.as_str());
        let body = if body.trim().is_empty() {
            DEFAULT_INVENTORY
        } else {
            body
        };
        fs::write(work.join("inventory"), body).map_err(|err| err.to_string())?;
        return Ok("inventory".into());
    }
    let dir = work.join("inventory.d");
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let mut written = 0usize;
    for (index, source) in sources.iter().enumerate() {
        if source.body.trim().is_empty() {
            continue;
        }
        let ext = inventory_extension(&source.body);
        fs::write(dir.join(format!("{index:02}.{ext}")), &source.body)
            .map_err(|err| err.to_string())?;
        written += 1;
    }
    if written == 0 {
        fs::write(dir.join("00.ini"), DEFAULT_INVENTORY).map_err(|err| err.to_string())?;
    }
    Ok("inventory.d".into())
}

fn inventory_extension(body: &str) -> &'static str {
    let first = body.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            None
        } else {
            Some(line.to_string())
        }
    });
    match first.as_deref() {
        Some(line) if !line.starts_with('[') && line.ends_with(':') => "yml",
        _ => "ini",
    }
}

fn load_environment(state: &Shared, template: &Template) -> Result<Option<Environment>, String> {
    let Some(id) = &template.environment_id else {
        return Ok(None);
    };
    let db = state.db().map_err(|e| e.to_string())?;
    let environment = db::environment(&db, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "execution_environment_is_missing".to_string())?;
    let team = db::team_by_id(&db, &template.team_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "team_is_missing".to_string())?;
    if !db::environment_visible(&environment, &team) {
        return Err("the_execution_environment_is_not_visible_to_this".into());
    }
    if !valid_image(&environment.image) {
        return Err("the_image_name_is_invalid".into());
    }
    if !valid_network(&environment.network) {
        return Err("the_network_name_is_invalid".into());
    }
    Ok(Some(environment))
}

pub fn valid_network(network: &str) -> bool {
    if network.is_empty() {
        return true;
    }
    let name = network.strip_prefix("container:").unwrap_or(network);
    !name.is_empty()
        && name.len() < 200
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "_.-".contains(ch))
}

pub fn valid_image(image: &str) -> bool {
    !image.is_empty()
        && image.len() < 200
        && image
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "/:._-@".contains(ch))
}

fn sync_log_line(err: String) -> String {
    match err.as_str() {
        "sync_the_git_project_first" => "Sync the Git project first.".into(),
        "the_project_path_is_not_a_directory" => ProjectPathError::NotDirectory.job_text().into(),
        "the_project_path_is_not_allowed" => ProjectPathError::Denied.job_text().into(),
        "path_projects_are_limited_to_administrators" => {
            ProjectPathError::AdminsOnly.job_text().into()
        }
        _ => err,
    }
}

struct SecretScrub {
    work: PathBuf,
}

impl Drop for SecretScrub {
    fn drop(&mut self) {
        scrub_work_dir(&self.work);
    }
}

const SECRET_MANIFEST: &str = "secret.manifest";

/// Names the wrapper still writes itself. A job also records every secret
/// file it created, so a new credential file is removed without editing this
/// list. The list stays for askpass scripts and for a job that stopped
/// before the manifest was written.
const SECRET_FILES: &[&str] = &[
    "ssh_key",
    "connection.pass",
    "key.pass",
    "become.pass",
    "auth.json",
    "git.pass",
    "git_key",
    "gce.json",
    "vault.pass",
    "vault.list",
    "container.env",
    "git-askpass",
    "askpass",
];

pub(crate) fn scrub_job_dirs(data_dir: &Path) {
    let Ok(entries) = fs::read_dir(data_dir.join("jobs")) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            scrub_work_dir(&entry.path());
        }
    }
}

fn scrub_work_dir(work: &Path) {
    if let Ok(text) = fs::read_to_string(work.join(SECRET_MANIFEST)) {
        for line in text.lines() {
            let name = line.trim();
            if safe_manifest_name(name) {
                let _ = fs::remove_file(work.join(name));
            }
        }
    }
    let _ = fs::remove_file(work.join(SECRET_MANIFEST));
    let Ok(entries) = fs::read_dir(work) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_secret_file(name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn safe_manifest_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name != SECRET_MANIFEST
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
        && !name.contains("..")
}

fn is_secret_file(name: &str) -> bool {
    SECRET_FILES
        .iter()
        .any(|base| name == *base || name.ends_with(&format!("-{base}")))
}

fn write_extra_file(work: &Path, tool: &str, extra: &Value) -> Result<String, String> {
    if tool == "ansible" {
        let name = "extra.yml".to_string();
        fs::write(work.join(&name), ansible_extra_yaml(extra)).map_err(|err| err.to_string())?;
        Ok(name)
    } else {
        let name = "extra.json".to_string();
        let bytes = serde_json::to_vec_pretty(extra).map_err(|err| err.to_string())?;
        fs::write(work.join(&name), bytes).map_err(|err| err.to_string())?;
        Ok(name)
    }
}

fn ansible_extra_yaml(value: &Value) -> String {
    let mut out = String::from("---\n");
    match value {
        Value::Object(map) => write_yaml_object(&mut out, map, 0),
        other => {
            write_yaml_scalar(&mut out, other);
            out.push('\n');
        }
    }
    out
}

fn write_yaml_object(out: &mut String, map: &serde_json::Map<String, Value>, indent: usize) {
    for (key, item) in map {
        write_indent(out, indent);
        out.push_str(&yaml_key(key));
        out.push(':');
        match item {
            Value::Object(child) if child.is_empty() => out.push_str(" {}\n"),
            Value::Object(child) => {
                out.push('\n');
                write_yaml_object(out, child, indent + 2);
            }
            Value::Array(items) if items.is_empty() => out.push_str(" []\n"),
            Value::Array(items) => {
                out.push('\n');
                write_yaml_array(out, items, indent + 2);
            }
            _ => {
                out.push(' ');
                write_yaml_scalar(out, item);
                out.push('\n');
            }
        }
    }
}

fn write_yaml_array(out: &mut String, items: &[Value], indent: usize) {
    for item in items {
        write_indent(out, indent);
        out.push_str("- ");
        match item {
            Value::Object(child) if child.is_empty() => out.push_str("{}\n"),
            Value::Object(child) => {
                out.push('\n');
                write_yaml_object(out, child, indent + 2);
            }
            Value::Array(nested) if nested.is_empty() => out.push_str("[]\n"),
            Value::Array(nested) => {
                out.push('\n');
                write_yaml_array(out, nested, indent + 2);
            }
            _ => {
                write_yaml_scalar(out, item);
                out.push('\n');
            }
        }
    }
}

fn write_yaml_scalar(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => out.push_str(&yaml_unsafe(text)),
        Value::Array(_) | Value::Object(_) => out.push_str("null"),
    }
}

fn yaml_key(key: &str) -> String {
    if safe_ident(key) {
        key.to_string()
    } else {
        format!("'{}'", key.replace('\'', "''"))
    }
}

fn yaml_unsafe(text: &str) -> String {
    if text
        .chars()
        .any(|ch| ch == '\n' || ch == '\r' || ch.is_control())
    {
        let mut escaped = String::new();
        for ch in text.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                '"' => escaped.push_str("\\\""),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                other if other.is_control() => {
                    escaped.push_str(&format!("\\u{:04x}", u32::from(other)));
                }
                other => escaped.push(other),
            }
        }
        format!("!unsafe \"{escaped}\"")
    } else {
        format!("!unsafe '{}'", text.replace('\'', "''"))
    }
}

fn write_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push(' ');
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectPathError {
    NotDirectory,
    Denied,
    AdminsOnly,
}

impl ProjectPathError {
    pub fn api_key(self) -> &'static str {
        match self {
            Self::NotDirectory => "the_project_path_is_not_a_directory",
            Self::Denied => "the_project_path_is_not_allowed",
            Self::AdminsOnly => "path_projects_are_limited_to_administrators",
        }
    }

    pub fn job_text(self) -> &'static str {
        match self {
            Self::NotDirectory => "The project path is not a directory.",
            Self::Denied => "The project path is not allowed.",
            Self::AdminsOnly => "Path projects are limited to administrators.",
        }
    }
}

enum PathState {
    Ready(PathBuf),
    Missing,
}

pub fn authorize_project_source(
    source_type: &str,
    source: &str,
    data_dir: &Path,
    roots: &[PathBuf],
    is_admin: bool,
) -> Result<(), ProjectPathError> {
    let path = match source_type {
        "path" => PathBuf::from(source),
        "git" => file_url_path(source).ok_or(ProjectPathError::Denied)?,
        _ => return Err(ProjectPathError::Denied),
    };
    match assess_path(&path, data_dir)? {
        PathState::Missing => {
            if source_type == "path" {
                Err(ProjectPathError::NotDirectory)
            } else if is_admin {
                Ok(())
            } else {
                Err(ProjectPathError::AdminsOnly)
            }
        }
        PathState::Ready(canonical) => {
            if is_admin || within_roots(&canonical, roots) {
                Ok(())
            } else {
                Err(ProjectPathError::AdminsOnly)
            }
        }
    }
}

pub(crate) fn stored_directory(
    path: &Path,
    data_dir: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf, String> {
    match assess_path(path, data_dir) {
        Ok(PathState::Ready(canonical)) => {
            if within_roots(&canonical, roots) {
                Ok(canonical)
            } else {
                Err("the_project_path_is_not_allowed".into())
            }
        }
        Ok(PathState::Missing) => Err("the_project_path_is_not_a_directory".into()),
        Err(err) => Err(err.api_key().into()),
    }
}

fn file_clone_allowed(url: &str, data_dir: &Path, roots: &[PathBuf]) -> Result<bool, String> {
    let Some(path) = file_url_path(url) else {
        if url.starts_with("file://") {
            return Err("the_project_path_is_not_allowed".into());
        }
        return Ok(false);
    };
    match assess_path(&path, data_dir) {
        Ok(PathState::Ready(canonical)) => {
            if within_roots(&canonical, roots) {
                Ok(true)
            } else {
                Err("the_project_path_is_not_allowed".into())
            }
        }
        Ok(PathState::Missing) => Err("the_project_path_is_not_allowed".into()),
        Err(err) => Err(err.api_key().into()),
    }
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = if let Some(after) = rest.strip_prefix("localhost/") {
        format!("/{after}")
    } else if rest == "localhost" {
        "/".to_string()
    } else if let Some(stripped) = rest.strip_prefix('/') {
        format!("/{stripped}")
    } else {
        return None;
    };
    let decoded = percent_decode(&path);
    if decoded.contains('\0') {
        return None;
    }
    Some(PathBuf::from(decoded))
}

fn assess_path(path: &Path, data_dir: &Path) -> Result<PathState, ProjectPathError> {
    let lexical = lexical_normal(path);
    if is_sensitive(&lexical) || relates_to_data(&lexical, data_dir) {
        return Err(ProjectPathError::Denied);
    }
    match std::fs::canonicalize(path) {
        Ok(canonical) => {
            if is_sensitive(&canonical) || relates_to_data(&canonical, data_dir) {
                return Err(ProjectPathError::Denied);
            }
            if !canonical.is_dir() {
                return Err(ProjectPathError::NotDirectory);
            }
            Ok(PathState::Ready(canonical))
        }
        Err(_) => Ok(PathState::Missing),
    }
}

fn within_roots(canonical: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| {
        let Ok(root) = std::fs::canonicalize(root) else {
            return false;
        };
        canonical.starts_with(&root)
    })
}

fn relates_to_data(path: &Path, data_dir: &Path) -> bool {
    let data = data_path(data_dir);
    path == data || path.starts_with(&data) || data.starts_with(path)
}

fn data_path(data_dir: &Path) -> PathBuf {
    std::fs::canonicalize(data_dir).unwrap_or_else(|_| {
        std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf())
    })
}

fn covers_data_dir(host: &Path, data_dir: &Path) -> bool {
    let host = std::fs::canonicalize(host).unwrap_or_else(|_| host.to_path_buf());
    let data = data_path(data_dir);
    host == data || data.starts_with(&host)
}

fn is_sensitive(path: &Path) -> bool {
    if path == Path::new("/") {
        return true;
    }
    const DENY: &[&str] = &[
        "/etc", "/proc", "/sys", "/dev", "/run", "/boot", "/root", "/var/run",
    ];
    DENY.iter().any(|item| {
        let item = Path::new(item);
        path == item || path.starts_with(item)
    })
}

fn lexical_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub async fn materialize_project(
    project: &Project,
    data_dir: &Path,
    proxy: &crate::settings::ProxySettings,
    update: bool,
    roots: &[PathBuf],
) -> Result<PathBuf, String> {
    match project.source_type.as_str() {
        "path" => {
            let path = PathBuf::from(&project.source);
            stored_directory(&path, data_dir, roots)
        }
        "git" => {
            if !valid_git(&project.source) {
                return Err("the_git_address_is_invalid".into());
            }
            let dest = data_dir.join("checkouts").join(&project.id);
            if !update {
                return if dest.join(".git").is_dir() {
                    Ok(dest)
                } else {
                    Err("sync_the_git_project_first".into())
                };
            }
            let allow_file = file_clone_allowed(&project.source, data_dir, roots)?;
            git_sync(&project.source, &project.revision, &dest, proxy, allow_file).await?;
            Ok(dest)
        }
        other => Err(crate::error::message(
            "project.unknown_source",
            &[("source", other)],
        )),
    }
}

pub fn valid_git(url: &str) -> bool {
    let scheme = url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("ssh://")
        || url.starts_with("git@")
        || url.starts_with("file://");
    scheme
        && url.len() < 500
        && !url.contains("..")
        && !url.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}

pub fn valid_revision(revision: &str) -> bool {
    revision.is_empty()
        || (revision.len() <= 200
            && !revision.starts_with('-')
            && revision
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-')))
}

fn git_cmd(proxy: &crate::settings::ProxySettings, allow_file: bool) -> Command {
    let mut cmd = Command::new("git");
    if allow_file {
        cmd.arg("-c").arg("protocol.file.allow=always");
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    proxy.apply_command(&mut cmd);
    cmd
}

fn git_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(12);
    let mut joined = lines[start..].join("\n");
    if joined.chars().count() > 900 {
        let skip = joined.chars().count() - 900;
        joined = joined.chars().skip(skip).collect();
    }
    if joined.is_empty() {
        format!("Git failed with status {}.", output.status)
    } else {
        format!("Git:\n{joined}")
    }
}

async fn git_run(mut cmd: Command) -> Result<(), String> {
    let finished = tokio::time::timeout(Duration::from_secs(120), cmd.output()).await;
    let output = match finished {
        Ok(result) => result.map_err(|err| format!("git: {err}"))?,
        Err(_) => {
            return Err("git_exceeded_the_120_second_time_limit".into());
        }
    };
    if output.status.success() {
        Ok(())
    } else {
        Err(git_text(&output))
    }
}

fn missing_revision(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("nicht gefunden")
        || lower.contains("remote branch")
        || lower.contains("couldn't find remote ref")
        || lower.contains("konnte den remote-verweis")
}

async fn git_clone(
    url: &str,
    revision: &str,
    dest: &Path,
    proxy: &crate::settings::ProxySettings,
    allow_file: bool,
) -> Result<(), String> {
    let mut cmd = git_cmd(proxy, allow_file);
    cmd.arg("clone").arg("--depth").arg("1");
    if !revision.is_empty() {
        cmd.arg("--branch").arg(revision);
    }
    cmd.arg("--").arg(url).arg(dest);
    git_run(cmd).await
}

async fn git_checkout(
    revision: &str,
    dest: &Path,
    proxy: &crate::settings::ProxySettings,
    allow_file: bool,
) -> Result<(), String> {
    let mut fetch = git_cmd(proxy, allow_file);
    fetch
        .arg("-C")
        .arg(dest)
        .args(["fetch", "--depth", "1", "origin"])
        .arg(revision);
    git_run(fetch).await?;
    let mut checkout = git_cmd(proxy, allow_file);
    checkout
        .arg("-C")
        .arg(dest)
        .args(["checkout", "--detach", "FETCH_HEAD"]);
    git_run(checkout).await
}

async fn git_update(
    revision: &str,
    dest: &Path,
    proxy: &crate::settings::ProxySettings,
    allow_file: bool,
) -> Result<(), String> {
    if revision.is_empty() {
        let mut cmd = git_cmd(proxy, allow_file);
        cmd.arg("-C").arg(dest).args(["pull", "--ff-only"]);
        return git_run(cmd).await;
    }
    git_checkout(revision, dest, proxy, allow_file).await
}

async fn git_sync(
    url: &str,
    revision: &str,
    dest: &Path,
    proxy: &crate::settings::ProxySettings,
    allow_file: bool,
) -> Result<(), String> {
    if !valid_revision(revision) {
        return Err("revision_is_invalid".into());
    }
    if dest.exists() && !dest.join(".git").is_dir() {
        fs::remove_dir_all(dest)
            .map_err(|err| format!("Incomplete checkout cannot be removed: {err}"))?;
    }
    if dest.join(".git").is_dir() {
        return git_update(revision, dest, proxy, allow_file).await;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    match git_clone(url, revision, dest, proxy, allow_file).await {
        Ok(()) => Ok(()),
        Err(err) if !revision.is_empty() && missing_revision(&err) => {
            if dest.exists() {
                fs::remove_dir_all(dest).map_err(|remove| remove.to_string())?;
            }
            git_clone(url, "", dest, proxy, allow_file).await?;
            git_checkout(revision, dest, proxy, allow_file).await
        }
        Err(err) => {
            if dest.exists() {
                let _ = fs::remove_dir_all(dest);
            }
            Err(err)
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    let mut files = 0u32;
    let mut bytes = 0u64;
    copy_inner(src, dst, &mut files, &mut bytes)
}

fn copy_inner(src: &Path, dst: &Path, files: &mut u32, bytes: &mut u64) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let label = name.to_string_lossy();
        if label == ".git" || label == ".terraform" || label == "target" {
            continue;
        }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(&name);
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_inner(&entry.path(), &to, files, bytes)?;
            continue;
        }
        *files += 1;
        let len = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        *bytes += len;
        if *files > MAX_FILES || *bytes > MAX_BYTES {
            return Err("Project is too large for a run (limit 4000 files or 200 MB).".into());
        }
        fs::copy(entry.path(), &to).map_err(|e| e.to_string())?;
    }
    Ok(())
}

struct ContainerRun<'a> {
    bin: &'a str,
    environment: &'a Environment,
    work: &'a Path,
    stage: &'a Path,
    cache: &'a Path,
    env: &'a [(String, String)],
    timeout_secs: u64,
    output_lines: u64,
    /// Host directory for this template only. Absent for every other tool.
    tofu_state: Option<&'a Path>,
    template_id: &'a str,
}

fn container_mounts(
    stage: &Path,
    work: &Path,
    cache: &Path,
    tofu_state: Option<&Path>,
    template_id: &str,
) -> Vec<(String, String, bool)> {
    let mut mounts = vec![
        (stage.display().to_string(), "/runner/project".into(), true),
        (work.display().to_string(), "/runner/work".into(), false),
        (
            cache.join("collections").display().to_string(),
            "/cache/collections".into(),
            false,
        ),
        (
            cache.join("roles").display().to_string(),
            "/cache/roles".into(),
            false,
        ),
        (
            cache.join("tofu").display().to_string(),
            "/cache/tofu".into(),
            false,
        ),
    ];
    if let Some(state) = tofu_state {
        mounts.push((
            state.display().to_string(),
            format!("/cache/tofu-state/{template_id}"),
            false,
        ));
    }
    mounts
}

#[derive(Debug)]
struct DaemonLayout {
    /// Path the daemon uses for the data directory. Absent on a host install.
    host_data: Option<PathBuf>,
    /// True when the path was configured and not confirmed by inspect.
    probe: bool,
}

fn layout_decision(
    configured: Option<&Path>,
    detected: Option<&Path>,
    in_container: bool,
) -> Result<DaemonLayout, String> {
    match (configured, detected) {
        (Some(configured), Some(detected)) if !same_dir(configured, detected) => Err(format!(
            "KOREOGRAF_DATA_HOST ({}) is not the data directory the container daemon mounted ({}). The job was not started.",
            configured.display(),
            detected.display()
        )),
        (_, Some(detected)) => Ok(DaemonLayout {
            host_data: Some(detected.to_path_buf()),
            probe: false,
        }),
        (Some(configured), None) => Ok(DaemonLayout {
            host_data: Some(configured.to_path_buf()),
            probe: true,
        }),
        (None, None) if in_container => Err(
            "The container daemon cannot see the data directory. Set KOREOGRAF_DATA_HOST to the host path of that directory. The job was not started."
                .into(),
        ),
        (None, None) => Ok(DaemonLayout {
            host_data: None,
            probe: false,
        }),
    }
}

fn same_dir(left: &Path, right: &Path) -> bool {
    lexical_normal(left) == lexical_normal(right)
}

fn check_mount_token(path: &str) -> Result<(), String> {
    if path.contains(',') {
        return Err(format!(
            "The mount path {path} cannot be passed to the container daemon. The job was not started."
        ));
    }
    Ok(())
}

fn require_visible_sources(mounts: &[(String, String, bool)]) -> Result<(), String> {
    for (host, guest, _) in mounts {
        check_mount_token(host)?;
        check_mount_token(guest)?;
        if !Path::new(host).exists() {
            return Err(format!(
                "The container daemon cannot see {host}. The job was not started."
            ));
        }
    }
    Ok(())
}

fn translate_mounts(
    mounts: &[(String, String, bool)],
    data_dir: &Path,
    host_data: &Path,
) -> Result<Vec<(String, String, bool)>, String> {
    let data = data_path(data_dir);
    let host_data = lexical_normal(host_data);
    let mut out = Vec::with_capacity(mounts.len());
    for (host, guest, readonly) in mounts {
        check_mount_token(host)?;
        check_mount_token(guest)?;
        let source = fs::canonicalize(host).map_err(|_| {
            format!("The container daemon cannot see {host}. The job was not started.")
        })?;
        let relative = source.strip_prefix(&data).map_err(|_| {
            format!("The container daemon cannot see {host}. The job was not started.")
        })?;
        let daemon = lexical_normal(&host_data.join(relative));
        let daemon_text = daemon.display().to_string();
        check_mount_token(&daemon_text)?;
        out.push((daemon_text, guest.clone(), *readonly));
    }
    Ok(out)
}

fn container_id_from_text(mountinfo: &str, cgroup: &str) -> Option<String> {
    hex_id_after(mountinfo, "/containers/")
        .or_else(|| hex_id_after(cgroup, "docker-"))
        .or_else(|| hex_id_after(cgroup, "/docker/"))
        .or_else(|| hex_id_after(cgroup, "libpod-"))
}

fn hex_id_after(text: &str, marker: &str) -> Option<String> {
    let mut rest = text;
    while let Some(index) = rest.find(marker) {
        let after = &rest[index + marker.len()..];
        let id: String = after
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .collect();
        if id.len() >= 64 {
            return Some(id[..64].to_string());
        }
        if after.is_empty() {
            break;
        }
        rest = &after[id.len().min(after.len())..];
        if rest.is_empty() {
            break;
        }
    }
    None
}

fn own_container_id() -> Option<String> {
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let cgroup = fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    container_id_from_text(&mountinfo, &cgroup)
}

fn process_in_container() -> bool {
    Path::new("/.dockerenv").exists()
        || Path::new("/run/.containerenv").exists()
        || own_container_id().is_some()
}

fn relative_to_dest(data_dir: &Path, dest: &Path) -> Option<PathBuf> {
    let dest = lexical_normal(dest);
    let lexical = lexical_normal(data_dir);
    if lexical == dest || lexical.starts_with(&dest) {
        return lexical.strip_prefix(&dest).ok().map(Path::to_path_buf);
    }
    let canonical = data_path(data_dir);
    if canonical == dest || canonical.starts_with(&dest) {
        return canonical.strip_prefix(&dest).ok().map(Path::to_path_buf);
    }
    None
}

fn inspect_data_root(json: &Value, data_dir: &Path) -> Option<PathBuf> {
    let node = json
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(json);
    let mounts = node
        .get("Mounts")
        .or_else(|| node.get("mounts"))?
        .as_array()?;
    let mut best: Option<(usize, PathBuf)> = None;
    for mount in mounts {
        let Some(dest) = mount
            .get("Destination")
            .or_else(|| mount.get("destination"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(source) = mount
            .get("Source")
            .or_else(|| mount.get("source"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(relative) = relative_to_dest(data_dir, Path::new(dest)) else {
            continue;
        };
        let len = lexical_normal(Path::new(dest)).components().count();
        if best.as_ref().is_some_and(|(best_len, _)| len <= *best_len) {
            continue;
        }
        best = Some((len, lexical_normal(&Path::new(source).join(relative))));
    }
    best.map(|(_, path)| path)
}

async fn detected_data_root(bin: &str, data_dir: &Path) -> Option<PathBuf> {
    let id = own_container_id()?;
    let mut cmd = Command::new(bin);
    cmd.args(["inspect", "--type", "container", &id])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(STOP_COMMAND_TIMEOUT, cmd.output()).await else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let json: Value = serde_json::from_slice(&output.stdout).ok()?;
    inspect_data_root(&json, data_dir)
}

async fn resolve_daemon_layout(
    bin: &str,
    data_dir: &Path,
    configured: Option<&Path>,
) -> Result<DaemonLayout, String> {
    let detected = detected_data_root(bin, data_dir).await;
    layout_decision(configured, detected.as_deref(), process_in_container())
}

fn probe_script(mounts: &[(String, String, bool)]) -> String {
    let mut parts = Vec::with_capacity(mounts.len());
    for (index, (_, guest, _)) in mounts.iter().enumerate() {
        if guest == "/runner/work" {
            parts.push(format!("test -f /mnt{index}/wrapper.sh"));
        } else {
            parts.push(format!("test -d /mnt{index}"));
        }
    }
    parts.join(" && ")
}

fn last_stderr_line(stderr: &str) -> String {
    stderr
        .split(['\n', '\r'])
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .chars()
        .filter(|ch| !ch.is_control())
        .take(180)
        .collect()
}

fn mount_check_error(stderr: &str) -> String {
    let detail = last_stderr_line(stderr);
    if detail.is_empty() {
        "The container daemon cannot see the job directory. The job was not started.".into()
    } else {
        format!(
            "The container daemon cannot see the job directory ({detail}). The job was not started."
        )
    }
}

/// The daemon reports a missing entrypoint only after it has accepted the
/// mounts. That is not an invisible directory. Match the shell name itself,
/// not a path that merely ends in `.sh`.
fn shell_is_missing(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    let missing = lower.contains("not found") || lower.contains("no such file");
    if !missing {
        return false;
    }
    [
        "exec: \"sh\"",
        "exec: \"/bin/sh\"",
        "exec: \"/usr/bin/sh\"",
        "executable file `sh`",
        "executable file \"sh\"",
        "executable file '/bin/sh'",
        "executable file `/bin/sh`",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn remove_probe_container(bin: &str, name: &str) {
    let _ = tokio::time::timeout(
        STOP_COMMAND_TIMEOUT,
        Command::new(bin)
            .args(["rm", "-f", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await;
}

async fn probe_daemon_dirs(
    bin: &str,
    image: &str,
    job_id: &str,
    mounts: &[(String, String, bool)],
) -> Result<(), String> {
    if mounts.is_empty() {
        return Ok(());
    }
    let name = format!("koreograf-mount-{job_id}");
    remove_probe_container(bin, &name).await;
    let mut args = vec![
        "run".to_string(),
        "--rm".into(),
        "--name".into(),
        name.clone(),
        "--network".into(),
        "none".into(),
        "--entrypoint".into(),
        "sh".into(),
    ];
    for (index, (host, _, _)) in mounts.iter().enumerate() {
        check_mount_token(host)?;
        args.push("--mount".into());
        args.push(format!(
            "type=bind,source={host},target=/mnt{index},readonly"
        ));
    }
    args.push(image.to_string());
    args.push("-c".into());
    args.push(probe_script(mounts));
    let mut cmd = Command::new(bin);
    cmd.args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let output = match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            remove_probe_container(bin, &name).await;
            return Err(format!(
                "The mount check could not be started ({err}). The job was not started."
            ));
        }
        Err(_) => {
            remove_probe_container(bin, &name).await;
            return Err(
                "The container daemon did not answer the mount check. The job was not started."
                    .into(),
            );
        }
    };
    remove_probe_container(bin, &name).await;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if shell_is_missing(&stderr) {
        return Ok(());
    }
    Err(mount_check_error(&stderr))
}

async fn run_container(state: &Shared, job_id: &str, run: ContainerRun<'_>) -> Result<i32, String> {
    let ContainerRun {
        bin,
        environment,
        work,
        stage,
        cache,
        env,
        timeout_secs,
        output_lines,
        tofu_state,
        template_id,
    } = run;
    ensure_image(bin, &environment.image, &environment.pull_policy).await?;
    write_env_file(&work.join("container.env"), env)?;
    let name = format!("koreograf-{}", &job_id[..8]);
    // `--env-file` is read by the client in this process.
    // Mount sources are paths the daemon can see.
    let mut mounts = container_mounts(stage, work, cache, tofu_state, template_id);
    for (host, _, _) in &mounts {
        if covers_data_dir(Path::new(host), &state.cfg.data_dir) {
            return Err("The job container cannot mount the data directory.".into());
        }
    }
    require_visible_sources(&mounts)?;
    let layout =
        resolve_daemon_layout(bin, &state.cfg.data_dir, state.cfg.data_host.as_deref()).await?;
    if let Some(host_data) = &layout.host_data {
        log(
            state,
            job_id,
            "system",
            &format!(
                "The container daemon sees the data directory at {}.",
                host_data.display()
            ),
        );
        mounts = translate_mounts(&mounts, &state.cfg.data_dir, host_data)?;
    }
    if layout.probe {
        probe_daemon_dirs(bin, &environment.image, job_id, &mounts).await?;
    }
    let owner = directory_user(work)?;
    let rootless = runtime_is_rootless(bin).await?;
    let user = container_run_user(&owner, rootless);
    log(
        state,
        job_id,
        "system",
        &format!("The job container runs as {user}."),
    );
    let spec = ContainerSpec {
        image: environment.image.clone(),
        name: name.clone(),
        mounts,
        env_file: work.join("container.env").display().to_string(),
        user,
        network: environment.network.clone(),
        command: vec!["/runner/work/wrapper.sh".into()],
    };
    log(
        state,
        job_id,
        "system",
        &format!(
            "Execution Environment {} ({bin}), Pull-Policy {}.",
            environment.image, environment.pull_policy
        ),
    );
    let argv = container_argv(&spec);
    let mut cmd = Command::new(bin);
    cmd.args(&argv)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn().map_err(|e| format!("{bin} run failed: {e}"))?;
    let pid = child.id().unwrap_or(0);
    let proc = RunningProc {
        pid,
        container: Some(name),
        bin: Some(bin.to_string()),
    };
    remember(state, job_id, proc.clone());
    if canceled(state, job_id) {
        signal_stop(state, job_id, proc);
    }
    wait_child(state, job_id, child, timeout_secs, output_lines).await
}

fn write_env_file(path: &Path, env: &[(String, String)]) -> Result<(), String> {
    let mut body = String::new();
    for (key, value) in env {
        if value.contains('\n') {
            return Err(format!("Variable {key} cannot be passed to the container."));
        }
        body.push_str(key);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    }
    // Cloud tokens sit in this file. Mode 0600 matches the other secret files.
    write_secret(path, &body)
}

async fn ensure_image(bin: &str, image: &str, policy: &str) -> Result<(), String> {
    match policy {
        "never" => Ok(()),
        "always" => pull_image(bin, image).await,
        _ => {
            if image_present(bin, image).await {
                Ok(())
            } else {
                pull_image(bin, image).await
            }
        }
    }
}

async fn image_present(bin: &str, image: &str) -> bool {
    Command::new(bin)
        .args(["image", "inspect", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

async fn pull_image(bin: &str, image: &str) -> Result<(), String> {
    let mut cmd = Command::new(bin);
    cmd.args(["pull", image])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("Image pull: {e}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Image pull: stderr is missing".to_string())?;
    // Read stderr while the process runs. An unread pipe fills and stalls the pull.
    let (status, tail) = tokio::join!(child.wait(), read_stderr_tail(stderr, PULL_STDERR_TAIL));
    let status = status.map_err(|e| format!("Image pull: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(pull_failure(image, &String::from_utf8_lossy(&tail)))
    }
}

fn pull_failure(image: &str, stderr: &str) -> String {
    let image = mask_pull_credentials(image);
    let detail = last_stderr_line(&mask_pull_credentials(stderr));
    if detail.is_empty() {
        format!("Image {image} could not be pulled.")
    } else {
        format!("Image {image} could not be pulled. {detail}")
    }
}

async fn read_stderr_tail(mut reader: impl AsyncRead + Unpin, limit: usize) -> Vec<u8> {
    let mut chunk = [0u8; 4096];
    let mut kept = Vec::new();
    let mut errors = 0u32;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                errors = 0;
                kept.extend_from_slice(&chunk[..n]);
                if kept.len() > limit {
                    let drop_n = kept.len() - limit;
                    kept = kept.split_off(drop_n);
                }
            }
            Err(_) => {
                // Not EOF. Keep reading so the reason stays and the pull
                // cannot block on a full pipe. Repeated errors drop the
                // reader, which closes the pipe instead of spinning.
                errors += 1;
                if errors >= PULL_STDERR_READ_ERRORS {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    }
    kept
}

const SECRET_KEYS: &[&str] = &[
    "access_token",
    "authorization",
    "password",
    "passwd",
    "secret",
    "token",
];

fn mask_pull_credentials(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(index) = rest.find('@') {
        let after = &rest[index + 1..];
        if after.starts_with("sha256:") || after.starts_with("sha512:") {
            out.push_str(&rest[..=index]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..index]);
        if let Some(userinfo_len) = userinfo_len(&out) {
            out.truncate(out.len() - userinfo_len);
            out.push_str("[redacted]");
        }
        out.push('@');
        rest = after;
    }
    out.push_str(rest);
    mask_secret_assignments(&out)
}

fn userinfo_len(out: &str) -> Option<usize> {
    let start =
        match out.rfind(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | '<' | '(')) {
            Some(index) => {
                let ch = out[index..].chars().next()?;
                index + ch.len_utf8()
            }
            None => 0,
        };
    let token = &out[start..];
    let userinfo = match token.rfind("://") {
        Some(scheme) => &token[scheme + 3..],
        None => token,
    };
    if userinfo.is_empty() || userinfo.contains('/') || !userinfo.contains(':') {
        return None;
    }
    Some(userinfo.len())
}

fn mask_secret_assignments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while !rest.is_empty() {
        if let Some((key_len, sep_len, value_len)) =
            secret_assignment(rest, out.chars().next_back())
        {
            out.push_str(&rest[..key_len + sep_len]);
            out.push_str("[redacted]");
            rest = &rest[key_len + sep_len + value_len..];
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn secret_assignment(rest: &str, previous: Option<char>) -> Option<(usize, usize, usize)> {
    if previous.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return None;
    }
    for key in SECRET_KEYS {
        if rest.len() < key.len() || !rest.is_char_boundary(key.len()) {
            continue;
        }
        if !rest[..key.len()].eq_ignore_ascii_case(key) {
            continue;
        }
        let after_key = &rest[key.len()..];
        if after_key
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            continue;
        }
        let sep = assignment_separator(after_key)?;
        let value = assignment_value_len(&after_key[sep..]);
        return Some((key.len(), sep, value));
    }
    None
}

fn assignment_separator(rest: &str) -> Option<usize> {
    let quoted = rest.starts_with('"');
    let mut n = usize::from(quoted);
    let mut body = if quoted { &rest[1..] } else { rest };
    let ws = body.len() - body.trim_start_matches([' ', '\t']).len();
    body = &body[ws..];
    n += ws;
    let sep = body.as_bytes().first().copied()?;
    if sep == b'=' || (sep == b':' && quoted) {
        body = &body[1..];
        n += 1;
        let ws = body.len() - body.trim_start_matches([' ', '\t']).len();
        body = &body[ws..];
        n += ws;
        if body.starts_with(['"', '\'']) {
            n += 1;
        }
        return Some(n);
    }
    if sep == b':' {
        return scheme_separator(body, n);
    }
    None
}

/// Unquoted `Authorization: Bearer <token>` or `Basic`. `body` starts at the colon.
fn scheme_separator(body: &str, mut n: usize) -> Option<usize> {
    n += 1;
    let mut body = &body[1..];
    let ws = body.len() - body.trim_start_matches([' ', '\t']).len();
    body = &body[ws..];
    n += ws;
    let scheme = scheme_len(body)?;
    let mut after = &body[scheme..];
    if after
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return None;
    }
    let ws = after.len() - after.trim_start_matches([' ', '\t']).len();
    if ws == 0 {
        return None;
    }
    after = &after[ws..];
    n += scheme + ws;
    if assignment_value_len(after) == 0 {
        return None;
    }
    Some(n)
}

fn scheme_len(body: &str) -> Option<usize> {
    for scheme in ["bearer", "basic"] {
        if body.len() >= scheme.len()
            && body.is_char_boundary(scheme.len())
            && body[..scheme.len()].eq_ignore_ascii_case(scheme)
        {
            return Some(scheme.len());
        }
    }
    None
}

fn assignment_value_len(rest: &str) -> usize {
    rest.find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '&' | '}' | '\\'))
        .unwrap_or(rest.len())
}

fn remember(state: &Shared, job_id: &str, proc: RunningProc) {
    if let Ok(mut procs) = state.procs.lock() {
        procs.insert(job_id.to_string(), proc);
    }
}

fn remembered(state: &Shared, job_id: &str) -> Option<RunningProc> {
    state
        .procs
        .lock()
        .ok()
        .and_then(|procs| procs.get(job_id).cloned())
}

fn forget_run(state: &Shared, job_id: &str) {
    if let Ok(mut procs) = state.procs.lock() {
        procs.remove(job_id);
    }
    if let Ok(mut canceled) = state.canceled.lock() {
        canceled.remove(job_id);
    }
}

fn canceled(state: &Shared, job_id: &str) -> bool {
    state
        .canceled
        .lock()
        .ok()
        .is_some_and(|ids| ids.contains(job_id))
}

async fn wait_child(
    state: &Shared,
    job_id: &str,
    mut child: tokio::process::Child,
    timeout_secs: u64,
    output_lines: u64,
) -> Result<i32, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout is missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr is missing".to_string())?;
    let out = tokio::spawn(pump(
        state.clone(),
        job_id.to_string(),
        "stdout".to_string(),
        stdout,
        output_lines,
    ));
    let err = tokio::spawn(pump(
        state.clone(),
        job_id.to_string(),
        "stderr".to_string(),
        stderr,
        output_lines,
    ));
    let timeout = Duration::from_secs(timeout_secs);
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Err(format!("Process: {err}")),
        Err(_) => {
            log(
                state,
                job_id,
                "system",
                "Time limit reached, stopping the process.",
            );
            if let Some(proc) = remembered(state, job_id) {
                if !deliver_stop(proc, true, true).await {
                    log(
                        state,
                        job_id,
                        "system",
                        "The stop signal could not be delivered.",
                    );
                }
            }
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = out.await;
            let _ = err.await;
            return Ok(124);
        }
    };
    let _ = out.await;
    let _ = err.await;
    Ok(status.code().unwrap_or(1))
}

fn output_split(limit: u64) -> (u64, u64) {
    let tail = limit / 4;
    let head = limit.saturating_sub(tail);
    (head, tail)
}

/// Tail lines wait in memory until the stream ends. One stream, 8 MiB.
const OUTPUT_TAIL_BYTES: usize = 8 * 1024 * 1024;

struct OutputTail {
    limit: u64,
    head_limit: u64,
    tail_limit: u64,
    byte_cap: usize,
    count: u64,
    omitted: u64,
    tail_bytes: usize,
    tail: VecDeque<String>,
    flushed: bool,
}

impl OutputTail {
    fn new(limit: u64) -> Self {
        Self::with_byte_cap(limit, OUTPUT_TAIL_BYTES)
    }

    fn with_byte_cap(limit: u64, byte_cap: usize) -> Self {
        let (head_limit, tail_limit) = output_split(limit);
        Self {
            limit,
            head_limit,
            tail_limit,
            byte_cap,
            count: 0,
            omitted: 0,
            tail_bytes: 0,
            tail: VecDeque::new(),
            flushed: false,
        }
    }

    /// A line to store now. `None` means the line is held for the tail.
    fn accept(&mut self, line: String) -> Option<String> {
        self.count += 1;
        if self.count <= self.head_limit {
            return Some(line);
        }
        if self.tail_limit == 0 {
            self.omitted += 1;
            return None;
        }
        self.push_tail(line);
        None
    }

    fn push_tail(&mut self, line: String) {
        let incoming = line.len();
        while !self.tail.is_empty()
            && (self.tail.len() as u64 >= self.tail_limit
                || self.tail_bytes.saturating_add(incoming) > self.byte_cap)
        {
            let Some(old) = self.tail.pop_front() else {
                break;
            };
            self.tail_bytes = self.tail_bytes.saturating_sub(old.len());
            self.omitted += 1;
        }
        self.tail_bytes = self.tail_bytes.saturating_add(line.len());
        self.tail.push_back(line);
    }
}

async fn pump<R>(state: Shared, job_id: String, stream: String, reader: R, output_lines: u64)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader);
    let mut buf = Vec::new();
    let mut kept = OutputTail::new(output_lines);
    let mut discarded = false;
    loop {
        buf.clear();
        let read = lines.read_until(b'\n', &mut buf).await;
        match read {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                if canceled(&state, &job_id) {
                    if !discarded {
                        discarded = true;
                        flush_tail(&state, &job_id, &stream, &mut kept);
                        log(
                            &state,
                            &job_id,
                            "system",
                            "Further output is discarded because the job was canceled.",
                        );
                    }
                    continue;
                }
                let mut line = String::from_utf8_lossy(&buf).to_string();
                if line.len() > 4_000 {
                    line.truncate(4_000);
                    line.push('…');
                }
                note_hosts(&state, &job_id, &stream, &line);
                if stream == "stdout" {
                    state.metrics.observe(&job_id, &line);
                }
                if let Some(line) = kept.accept(line) {
                    store_event(&state, &job_id, &stream, &line);
                }
            }
            Err(_) => break,
        }
    }
    if !discarded {
        flush_tail(&state, &job_id, &stream, &mut kept);
    }
}

fn flush_tail(state: &Shared, job_id: &str, stream: &str, kept: &mut OutputTail) {
    if kept.flushed {
        return;
    }
    kept.flushed = true;
    if kept.omitted > 0 {
        let kept_tail = kept.tail.len();
        log(
            state,
            job_id,
            "system",
            &format!(
                "Omitted {} lines (limit {}; kept the first {} and the last {}).",
                kept.omitted, kept.limit, kept.head_limit, kept_tail
            ),
        );
    }
    for line in kept.tail.drain(..) {
        store_event(state, job_id, stream, &line);
    }
}

fn store_event(state: &Shared, job_id: &str, stream: &str, line: &str) {
    if let Ok(db) = state.db() {
        let _ = db::insert_event(&db, job_id, stream, line);
    }
}

fn note_hosts(state: &Shared, job_id: &str, stream: &str, line: &str) {
    if !(matches!(stream, "stdout" | "stderr") && host_line(line)) {
        return;
    }
    if let Ok(db) = state.db() {
        if let Ok(Some(job)) = db::job(&db, job_id) {
            if let Some(next) = observe_host_line(&job.failed_hosts, line) {
                let _ = db::set_failed_hosts(&db, job_id, &next);
            }
        }
    }
}

fn log(state: &Shared, job_id: &str, stream: &str, line: &str) {
    store_event(state, job_id, stream, line);
    note_hosts(state, job_id, stream, line);
}

fn host_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("fatal:") || line.contains("ok=")
}

fn with_saved_parameters(mut template: Template, job: &db::Job) -> Template {
    let params = db::json_object(&job.parameters);
    let Some(tool) = params.get("tool").and_then(Value::as_str) else {
        return template;
    };
    if known_tool(tool) {
        template.tool = tool.to_string();
    }
    if let Some(entrypoint) = params.get("entrypoint").and_then(Value::as_str) {
        template.entrypoint = entrypoint.to_string();
    }
    if let Some(spec) = params.get("spec").filter(|value| value.is_object()) {
        template.spec = spec.to_string();
    }
    if let Some(extra) = params.get("extra_vars").filter(|value| value.is_object()) {
        template.extra_vars = extra.to_string();
    }
    template.project_id = saved_id(&params, "project_id");
    template.inventory_id = saved_id(&params, "inventory_id");
    template.credential_id = saved_id(&params, "credential_id");
    template.environment_id = saved_id(&params, "environment_id");
    if let Some(update) = params.get("update_on_launch").and_then(Value::as_bool) {
        template.update_on_launch = update;
    }
    template
}

fn saved_id(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn finish(state: &Shared, job_id: &str, success: bool) {
    let status = if success { "successful" } else { "failed" };
    if let Ok(db) = state.db() {
        let _ = db::finish_job(&db, job_id, status);
    }
}

fn close_observation(state: &Shared, job_id: &str) {
    let loaded = state
        .db()
        .ok()
        .and_then(|db| db::job(&db, job_id).ok().flatten());
    let (status, labels) = match loaded {
        Some(job) => (job.status.clone(), Some(metrics::job_labels(&job))),
        None => ("failed".to_string(), None),
    };
    let meta = labels.as_ref().map(|item| JobMeta {
        job_id,
        team_id: &item.team_id,
        tool: &item.tool,
        playbook: &item.playbook,
        template: &item.template,
    });
    state.metrics.close(job_id, &status, meta.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_without_a_runtime_names_the_remedy() {
        assert!(AUTO_WITHOUT_RUNTIME.contains("Mount the Docker socket"));
        assert!(AUTO_WITHOUT_RUNTIME.contains("set the runner mode to docker to fail instead"));
    }

    #[tokio::test]
    async fn a_runtime_probe_stops_when_the_command_hangs() {
        let started = std::time::Instant::now();
        let ok = command_ok("sleep", &["30"], Duration::from_millis(200)).await;
        assert!(!ok);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_saved_checkout_can_run_without_a_pull() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-offline-{nanos}"));
        let checkout = dir.join("checkouts").join("offline");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        let project = db::Project {
            id: "offline".into(),
            team_id: "team".into(),
            organization_id: String::new(),
            organization_name: String::new(),
            name: "Offline".into(),
            source_type: "git".into(),
            source: "https://example.invalid/offline.git".into(),
            revision: String::new(),
            created_at: String::new(),
            team_ids: Vec::new(),
            admin_saved: true,
        };
        let proxy = crate::settings::ProxySettings::inherit();
        let path = materialize_project(&project, &dir, &proxy, false, &[])
            .await
            .unwrap();
        assert_eq!(path, checkout);
        let missing = db::Project {
            id: "absent".into(),
            ..project
        };
        let err = materialize_project(&missing, &dir, &proxy, false, &[])
            .await
            .unwrap_err();
        assert_eq!(err, "sync_the_git_project_first");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn image_names_allow_a_digest_reference() {
        assert!(valid_image("quay.io/ansible/awx-ee:latest"));
        assert!(valid_image(
            "ghcr.io/ansible-community/community-ee-base@sha256:d42f2597080e8e34ce4d1f70f66f987fc841bd0bbeb680f8d234e94ef1afc60a"
        ));
        assert!(!valid_image(""));
        assert!(!valid_image("image with space"));
        assert!(!valid_image("bad$(image)"));
    }

    #[test]
    fn a_blank_tag_list_is_not_a_filter() {
        assert!(!tag_filter_is_set(&serde_json::json!({})));
        assert!(!tag_filter_is_set(
            &serde_json::json!({"tags": "  ", "skip_tags": ""})
        ));
        assert!(tag_filter_is_set(&serde_json::json!({"tags": "users"})));
        assert!(tag_filter_is_set(&serde_json::json!({"skip_tags": "slow"})));
    }

    #[test]
    fn ansible_check_tags_and_limit_become_env() {
        let spec = serde_json::json!({
            "job_type": "check",
            "tags": "ping,web",
            "skip_tags": "slow",
            "limit": "web:&db",
            "diff": true,
            "verbosity": 2,
            "forks": 5
        });
        let env = ansible_env(&spec).unwrap();
        assert!(env.contains(&("KOREOGRAF_CHECK".into(), "1".into())));
        assert!(env.contains(&("KOREOGRAF_TAGS".into(), "ping,web".into())));
        assert!(env.contains(&("KOREOGRAF_SKIP_TAGS".into(), "slow".into())));
        assert!(env.contains(&("KOREOGRAF_LIMIT".into(), "web:&db".into())));
        assert!(env.contains(&("KOREOGRAF_DIFF".into(), "1".into())));
        assert!(env.contains(&("KOREOGRAF_VERBOSITY".into(), "2".into())));
        assert!(env.contains(&("KOREOGRAF_FORKS".into(), "5".into())));
        assert!(check_ansible_spec(&serde_json::json!({"tags": "-evil"})).is_err());
        let wrapper = include_str!("wrapper.sh");
        assert!(wrapper.contains("--check"));
        assert!(wrapper.contains("--tags"));
        assert!(wrapper.contains("--skip-tags"));
        assert!(wrapper.contains("--limit"));
    }

    #[test]
    fn container_wrapper_registers_a_missing_uid() {
        let wrapper = include_str!("wrapper.sh");
        assert!(wrapper.contains("No user exists for uid"));
        assert!(wrapper.contains(">> /etc/passwd"));
        assert!(wrapper.contains("[ \"${KOREOGRAF_WORK}\" = \"/runner/work\" ]"));
        assert!(wrapper.contains("container user:"));
    }

    #[test]
    fn docker_command_mounts_the_cache() {
        let spec = ContainerSpec {
            image: "quay.io/ansible/awx-ee:latest".into(),
            name: "koreograf-abc".into(),
            mounts: vec![(
                "/data/cache/team/collections".into(),
                "/cache/collections".into(),
                false,
            )],
            env_file: "/data/jobs/abc/container.env".into(),
            user: "0:0".into(),
            network: String::new(),
            command: vec!["/runner/work/wrapper.sh".into()],
        };
        let argv = container_argv(&spec);
        let user_at = argv.iter().position(|arg| arg == "--user").expect("user");
        let image_at = argv
            .iter()
            .position(|arg| arg == "quay.io/ansible/awx-ee:latest")
            .expect("image");
        assert!(user_at < image_at);
        assert_eq!(argv.get(user_at + 1).map(String::as_str), Some("0:0"));
        let opt_at = argv
            .iter()
            .position(|arg| arg == "--security-opt")
            .expect("security-opt");
        assert!(opt_at < image_at);
        assert_eq!(
            argv.get(opt_at + 1).map(String::as_str),
            Some("no-new-privileges")
        );
        let drop_at = argv
            .iter()
            .position(|arg| arg == "--cap-drop")
            .expect("cap-drop");
        assert!(drop_at < image_at);
        assert_eq!(argv.get(drop_at + 1).map(String::as_str), Some("ALL"));
        assert!(argv.iter().any(|arg| arg.contains("/cache/collections")));
        assert!(argv.iter().any(|arg| arg == "--mount"));
        assert!(!argv.iter().any(|arg| arg == "-v"));
        assert!(argv.iter().any(|arg| {
            arg == "type=bind,source=/data/cache/team/collections,target=/cache/collections"
        }));
        assert!(argv
            .iter()
            .any(|arg| arg == "quay.io/ansible/awx-ee:latest"));
        assert_eq!(
            argv.last().map(String::as_str),
            Some("/runner/work/wrapper.sh")
        );
        let readonly = ContainerSpec {
            mounts: vec![(
                "/data/jobs/abc/project".into(),
                "/runner/project".into(),
                true,
            )],
            ..spec
        };
        let argv = container_argv(&readonly);
        assert!(argv.iter().any(|arg| {
            arg == "type=bind,source=/data/jobs/abc/project,target=/runner/project,readonly"
        }));
        assert!(!argv.iter().any(|arg| arg == "--network"));
    }

    #[test]
    fn container_argv_passes_the_configured_network() {
        let spec = ContainerSpec {
            image: "quay.io/ansible/awx-ee:latest".into(),
            name: "koreograf-abc".into(),
            mounts: Vec::new(),
            env_file: "/data/jobs/abc/container.env".into(),
            user: "0:0".into(),
            network: String::new(),
            command: vec!["/runner/work/wrapper.sh".into()],
        };
        assert!(!container_argv(&spec).iter().any(|arg| arg == "--network"));
        let spec = ContainerSpec {
            network: "container:koreograf-koreograf-1".into(),
            ..spec
        };
        let argv = container_argv(&spec);
        let at = argv
            .iter()
            .position(|arg| arg == "--network")
            .expect("network");
        assert_eq!(
            argv.get(at + 1).map(String::as_str),
            Some("container:koreograf-koreograf-1")
        );
        let image_at = argv
            .iter()
            .position(|arg| arg == "quay.io/ansible/awx-ee:latest")
            .expect("image");
        assert!(at < image_at);
    }

    #[test]
    fn network_names_reject_option_injection() {
        assert!(valid_network(""));
        assert!(valid_network("bridge"));
        assert!(valid_network("koreograf_default"));
        assert!(valid_network("container:koreograf-koreograf-1"));
        assert!(!valid_network("--net=host"));
        assert!(!valid_network("-bridge"));
        assert!(!valid_network("host --privileged"));
        assert!(!valid_network("net;rm"));
        assert!(!valid_network("container:"));
    }

    #[cfg(unix)]
    #[test]
    fn directory_owner_keeps_a_secret_file_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("koreograf-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("container.env");
        write_secret(&secret, "TOKEN=value").unwrap();
        let before = std::fs::metadata(&secret).unwrap();
        let user = directory_user(&dir).unwrap();
        let after = std::fs::metadata(&secret).unwrap();
        assert_eq!(user, format!("{}:{}", before.uid(), before.gid()));
        assert_eq!(after.permissions().mode() & 0o777, 0o600);
        assert_eq!(after.uid(), before.uid());
        assert_eq!(after.gid(), before.gid());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rootless_runtime_uses_container_root() {
        assert!(!info_is_rootless(
            "docker",
            r#"["name=seccomp,profile=builtin","name=cgroupns"]"#
        ));
        assert!(info_is_rootless(
            "docker",
            r#"["name=rootless","name=seccomp,profile=builtin"]"#
        ));
        assert!(info_is_rootless("podman", "true\n"));
        assert!(!info_is_rootless("podman", "false"));
        assert_eq!(container_run_user("1000:1000", false), "1000:1000");
        assert_eq!(container_run_user("1000:1000", true), "0:0");
        let spec = ContainerSpec {
            image: "quay.io/ansible/awx-ee:latest".into(),
            name: "koreograf-rootless".into(),
            mounts: Vec::new(),
            env_file: "/data/jobs/abc/container.env".into(),
            user: container_run_user("1000:1000", true),
            network: String::new(),
            command: vec!["/runner/work/wrapper.sh".into()],
        };
        let argv = container_argv(&spec);
        let user_at = argv.iter().position(|arg| arg == "--user").expect("user");
        let image_at = argv
            .iter()
            .position(|arg| arg == "quay.io/ansible/awx-ee:latest")
            .expect("image");
        assert!(user_at < image_at);
        assert_eq!(argv.get(user_at + 1).map(String::as_str), Some("0:0"));
        assert!(!argv.iter().any(|arg| arg == "1000:1000"));
    }

    #[test]
    fn daemon_layout_uses_the_mounted_volume_and_refuses_a_mismatch() {
        let detected = Path::new("/var/lib/docker/volumes/koreograf_koreograf-data/_data");
        let host = layout_decision(None, None, false).expect("host");
        assert!(host.host_data.is_none());
        assert!(!host.probe);
        let seen = layout_decision(None, Some(detected), true).expect("detected");
        assert_eq!(seen.host_data.as_deref(), Some(detected));
        assert!(!seen.probe);
        let configured = layout_decision(Some(detected), None, true).expect("configured");
        assert!(configured.probe);
        let matched = layout_decision(
            Some(Path::new(
                "/var/lib/docker/volumes/koreograf_koreograf-data/_data/",
            )),
            Some(detected),
            true,
        )
        .expect("same directory");
        assert!(!matched.probe);
        let mismatch = layout_decision(Some(Path::new("/tmp/other-data")), Some(detected), true)
            .expect_err("mismatch");
        assert!(mismatch.contains("KOREOGRAF_DATA_HOST"));
        assert!(mismatch.contains("was not started"));
        let missing = layout_decision(None, None, true).expect_err("container without a path");
        assert!(missing.contains("KOREOGRAF_DATA_HOST"));
        assert!(missing.contains("was not started"));
    }

    #[test]
    fn container_id_comes_from_the_runtime_files() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mountinfo =
            format!("42 12 0:0 /var/lib/docker/containers/{id}/hostname /etc/hostname ro\n");
        assert_eq!(
            container_id_from_text(&mountinfo, "0::/"),
            Some(id.to_string())
        );
        let cgroup = format!("0::/system.slice/docker-{id}.scope\n");
        assert_eq!(container_id_from_text("", &cgroup), Some(id.to_string()));
        assert_eq!(
            container_id_from_text("", "0::/docker/abcd"),
            None,
            "a short id is not a container id"
        );
        assert_eq!(container_id_from_text("overlay2/abcdef", ""), None);
    }

    #[test]
    fn inspect_picks_the_data_volume_and_not_a_sibling() {
        let json = serde_json::json!([{
            "Mounts": [
                {
                    "Type": "bind",
                    "Source": "/host/var/lib",
                    "Destination": "/var/lib"
                },
                {
                    "Type": "volume",
                    "Source": "/var/lib/docker/volumes/proj_koreograf-data/_data",
                    "Destination": "/var/lib/koreograf"
                },
                {
                    "Type": "volume",
                    "Source": "/var/lib/docker/volumes/other/_data",
                    "Destination": "/var/lib/koreograf-extra"
                }
            ]
        }]);
        let root = inspect_data_root(&json, Path::new("/var/lib/koreograf")).expect("root");
        assert_eq!(
            root,
            Path::new("/var/lib/docker/volumes/proj_koreograf-data/_data")
        );
        assert!(inspect_data_root(&json, Path::new("/opt/missing")).is_none());
    }

    #[test]
    fn translated_mounts_stay_under_the_daemon_data_path() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let data = std::env::temp_dir().join(format!("koreograf-mount-{nanos}"));
        let work = data.join("jobs").join("job");
        std::fs::create_dir_all(&work).unwrap();
        let mounts = vec![(work.display().to_string(), "/runner/work".into(), false)];
        let host = Path::new("/var/lib/docker/volumes/proj_koreograf-data/_data");
        let translated = translate_mounts(&mounts, &data, host).expect("translate");
        assert_eq!(translated.len(), 1);
        assert_eq!(
            translated[0].0,
            host.join("jobs").join("job").display().to_string()
        );
        assert_eq!(translated[0].1, "/runner/work");
        let outside = vec![(
            std::env::temp_dir().display().to_string(),
            "/runner/work".into(),
            false,
        )];
        let err = translate_mounts(&outside, &data, host).expect_err("outside");
        assert!(err.contains("was not started"));
        let missing = Path::new("/no/such/koreograf-mount");
        let absent = vec![(missing.display().to_string(), "/runner/work".into(), false)];
        let err = require_visible_sources(&absent).expect_err("missing");
        assert!(err.contains("was not started"));
        assert!(err.contains("/no/such/koreograf-mount"));
        let comma = vec![(work.display().to_string(), "/runner/work,ro".into(), false)];
        assert!(translate_mounts(&comma, &data, host).is_err());
        assert_eq!(probe_script(&mounts), "test -f /mnt0/wrapper.sh");
        assert_eq!(
            mount_check_error(""),
            "The container daemon cannot see the job directory. The job was not started."
        );
        assert!(
            mount_check_error("bind source path does not exist: /tmp/x\n")
                .contains("bind source path does not exist")
        );
        let missing_shell = "exec: \"sh\": executable file not found in $PATH";
        assert!(shell_is_missing(missing_shell));
        assert!(shell_is_missing(
            "crun: executable file `sh` not found in $PATH"
        ));
        assert!(!shell_is_missing("bind source path does not exist: /tmp/x"));
        assert!(!shell_is_missing(
            "crun: executable file `wrapper.sh` not found in $PATH"
        ));
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn pull_failure_keeps_the_daemon_reason_and_masks_credentials() {
        let denied = pull_failure(
            "registry.example/app:missing",
            "progress\rError: Get \"https://robot:s3cret-token@registry.example/v2/app/manifests/missing\": manifest unknown\n",
        );
        assert!(denied.contains("manifest unknown"), "{denied}");
        assert!(denied.contains("registry.example/app:missing"), "{denied}");
        assert!(
            denied.contains("https://[redacted]@registry.example/v2/app/manifests/missing"),
            "{denied}"
        );
        assert!(!denied.contains("s3cret-token"), "{denied}");
        assert!(!denied.contains("robot:"), "{denied}");

        let digest = pull_failure(
            "registry.example/app:tag@sha256:0123456789abcdef",
            "Error: manifest unknown\n",
        );
        assert!(
            digest.contains("registry.example/app:tag@sha256:0123456789abcdef"),
            "{digest}"
        );
        assert!(digest.contains("manifest unknown"), "{digest}");
        assert!(!digest.contains("[redacted]"), "{digest}");

        let embedded = pull_failure(
            "user:s3cret-token@registry.example/app:missing",
            "Error: manifest unknown\n",
        );
        assert!(embedded.contains("manifest unknown"), "{embedded}");
        assert!(
            embedded.starts_with("Image [redacted]@registry.example/app:missing"),
            "{embedded}"
        );
        assert!(!embedded.contains("s3cret-token"), "{embedded}");

        let assigned = pull_failure(
            "app:missing",
            "denied password=s3cret-token token = \"other-secret\"\n",
        );
        assert!(assigned.contains("denied"), "{assigned}");
        assert!(assigned.contains("password=[redacted]"), "{assigned}");
        assert!(assigned.contains("token = \"[redacted]\""), "{assigned}");
        assert!(!assigned.contains("s3cret-token"), "{assigned}");
        assert!(!assigned.contains("other-secret"), "{assigned}");

        let json = pull_failure(
            "app:missing",
            "{\"message\":\"unauthorized\",\"password\":\"s3cret-token\"}\n",
        );
        assert!(json.contains("unauthorized"), "{json}");
        assert!(json.contains("\"password\":\"[redacted]\""), "{json}");
        assert!(!json.contains("s3cret-token"), "{json}");

        let bearer = pull_failure("app:missing", "Error: Authorization: Bearer s3cret-token\n");
        assert!(
            bearer.contains("Authorization: Bearer [redacted]"),
            "{bearer}"
        );
        assert!(bearer.contains("Error:"), "{bearer}");
        assert!(!bearer.contains("s3cret-token"), "{bearer}");
        let basic = pull_failure("app:missing", "Error: authorization: basic dXNlcjpwYXNz\n");
        assert!(basic.contains("authorization: basic [redacted]"), "{basic}");
        assert!(!basic.contains("dXNlcjpwYXNz"), "{basic}");

        let prose = pull_failure(
            "app:latest",
            "toomanyrequests: You have reached your pull rate limit. password: not a secret field\n",
        );
        assert_eq!(
            prose,
            "Image app:latest could not be pulled. toomanyrequests: You have reached your pull rate limit. password: not a secret field"
        );
        assert_eq!(
            pull_failure("app:missing", " \n"),
            "Image app:missing could not be pulled."
        );
    }

    #[tokio::test]
    async fn a_failed_pull_logs_the_daemon_reason_without_stalling() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-pull-{nanos}"));
        let fail = dir.join("docker-fail");
        let ok_bin = dir.join("docker-ok");
        write_executable(
            &fail,
            r#"#!/bin/sh
i=0
while [ "$i" -lt 2000 ]; do
  printf '%s\n' 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2
  i=$((i + 1))
done
printf '%s\n' 'Error: Get "https://robot:s3cret-token@registry.example/v2/missing/manifests/nope": manifest unknown' >&2
exit 1
"#,
        );
        write_executable(
            &ok_bin,
            "#!/bin/sh\nprintf '%s\n' 'secret-line' >&2\nexit 0\n",
        );
        let fail_arg = fail.display().to_string();
        let ok_arg = ok_bin.display().to_string();
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            pull_image(&fail_arg, "missing:nope"),
        )
        .await;
        let ok = pull_image(&ok_arg, "app:latest").await;
        let _ = std::fs::remove_dir_all(&dir);
        let err = result
            .expect("pull stalled on stderr")
            .expect_err("pull fails");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(err.contains("manifest unknown"), "{err}");
        assert!(err.contains("missing:nope"), "{err}");
        assert!(err.contains("[redacted]"), "{err}");
        assert!(!err.contains("s3cret-token"), "{err}");
        assert!(ok.is_ok(), "{ok:?}");
    }

    struct StderrAfterError {
        errors: u32,
        data: Vec<u8>,
        pos: usize,
    }

    impl AsyncRead for StderrAfterError {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            let this = self.get_mut();
            if this.errors > 0 {
                this.errors -= 1;
                return std::task::Poll::Ready(Err(std::io::Error::other("again")));
            }
            if this.pos >= this.data.len() {
                return std::task::Poll::Ready(Ok(()));
            }
            let n = buf.remaining().min(this.data.len() - this.pos);
            buf.put_slice(&this.data[this.pos..this.pos + n]);
            this.pos += n;
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn pull_stderr_keeps_the_reason_after_a_read_error() {
        let reader = StderrAfterError {
            errors: 1,
            data: b"noise\nError: manifest unknown\n".to_vec(),
            pos: 0,
        };
        let tail = read_stderr_tail(reader, PULL_STDERR_TAIL).await;
        let text = String::from_utf8_lossy(&tail);
        assert!(text.contains("manifest unknown"), "{text}");
    }

    #[tokio::test]
    async fn pull_stderr_stops_after_repeated_read_errors() {
        let reader = StderrAfterError {
            errors: 100,
            data: b"never\n".to_vec(),
            pos: 0,
        };
        let started = std::time::Instant::now();
        let tail = tokio::time::timeout(
            Duration::from_secs(2),
            read_stderr_tail(reader, PULL_STDERR_TAIL),
        )
        .await;
        assert!(tail.is_ok(), "repeated read errors stalled");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn merge_lets_the_event_override_defaults() {
        let base = serde_json::json!({"heal_action": "noop", "alertname": "manual"});
        let event = serde_json::json!({"alertname": "DiskSpaceLow"});
        let merged = merge_vars(&base, &event);
        assert_eq!(merged["alertname"], "DiskSpaceLow");
        assert_eq!(merged["heal_action"], "noop");
    }

    #[test]
    fn accepts_file_urls_and_rejects_traversal() {
        assert!(valid_git("file:///tmp/demo.git"));
        assert!(valid_git("https://example.com/demo.git"));
        assert!(valid_git("git@example.com:demo.git"));
        assert!(!valid_git("file:///tmp/../etc"));
        assert!(!valid_git("file:///tmp/demo.git extra"));
        assert!(!valid_git("/tmp/demo.git"));
        assert!(valid_revision("main"));
        assert!(valid_revision(""));
        assert!(!valid_revision("-rf"));
        assert!(!valid_revision("feature branch"));
    }

    #[test]
    fn rejects_path_escape() {
        assert!(check_relative("site.yml").is_ok());
        assert!(check_relative("../site.yml").is_err());
        assert!(check_relative("/etc/passwd").is_err());
    }

    #[test]
    fn tofu_state_dir_is_per_team_and_template() {
        let team_a = Path::new("/data/cache/team-a");
        let team_b = Path::new("/data/cache/team-b");
        let a = tofu_state_dir(team_a, "tpl-1").unwrap();
        let b = tofu_state_dir(team_b, "tpl-1").unwrap();
        assert_eq!(a, team_a.join("tofu-state").join("tpl-1"));
        assert_eq!(b, team_b.join("tofu-state").join("tpl-1"));
        assert_ne!(a, b);
        assert!(tofu_state_dir(team_a, "").is_err());
        assert!(tofu_state_dir(team_a, "../tpl").is_err());
        assert!(tofu_state_dir(team_a, "tpl/other").is_err());
        assert!(tofu_state_dir(team_a, &"x".repeat(81)).is_err());
    }

    fn write_executable(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
    }

    fn run_wrapper(root: &Path, action: &str, state: &Path, module: &str) -> String {
        run_wrapper_files(root, action, state, module, &[])
    }

    fn run_wrapper_files(
        root: &Path,
        action: &str,
        state: &Path,
        module: &str,
        extras: &[(&str, &str)],
    ) -> String {
        let bin = root.join("bin");
        let project = root.join("project");
        let work = root.join("work");
        let module_dir = project.join("tofu");
        // Each job has its own work directory. TF_DATA_DIR lives there, so a
        // reused directory would still remember the previous backend path.
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&module_dir).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(module_dir.join("main.tf"), module).unwrap();
        for (rel, body) in extras {
            let path = module_dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, body).unwrap();
        }
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let output = std::process::Command::new("sh")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wrapper.sh"))
            .env_clear()
            .env("PATH", &path)
            .env("HOME", &work)
            .env("KOREOGRAF_TOOL", "opentofu")
            .env("KOREOGRAF_ACTION", action)
            .env("KOREOGRAF_WORK", &work)
            .env("KOREOGRAF_PROJECT", &project)
            .env("KOREOGRAF_COLLECTIONS", root.join("collections"))
            .env("KOREOGRAF_ROLES", root.join("roles"))
            .env("KOREOGRAF_TOFU", root.join("plugins"))
            .env("KOREOGRAF_TOFU_STATE", state)
            .env("KOREOGRAF_INVENTORY", work.join("hosts"))
            .env("KOREOGRAF_ENTRYPOINT", "tofu")
            .env("KOREOGRAF_EXTRA", work.join("extra.json"))
            .output()
            .expect("wrapper");
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "status {:?} stdout={text} stderr={err}",
            output.status
        );
        text
    }

    #[test]
    fn opentofu_apply_keeps_state_for_the_team_and_template() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("koreograf-tofu-state-{nanos}"));
        let fake = root.join("bin").join("tofu");
        write_executable(
            &fake,
            r#"#!/bin/sh
state=""
if [ -f koreograf-local-backend.tf ]; then
  state=$(sed -n 's/.*path = "\(.*\)".*/\1/p' koreograf-local-backend.tf)
fi
printf '%s\n' "plugin=$TF_PLUGIN_CACHE_DIR"
printf '%s\n' "data=$TF_DATA_DIR"
if [ "$1" = "apply" ]; then
  if [ -n "$state" ] && [ -f "$state" ]; then
    printf '%s\n' "No changes. Your infrastructure matches the configuration."
  else
    printf '%s\n' "Apply complete! Resources: 1 added, 0 changed, 0 destroyed."
    if [ -n "$state" ]; then
      printf '%s\n' '{"version":4}' > "$state"
    fi
  fi
fi
exit 0
"#,
        );
        let module = "resource \"terraform_data\" \"marker\" {\n  input = \"koreograf\"\n}\n";
        let team_a = root.join("cache").join("team-a").join("tpl");
        let first = run_wrapper(&root, "apply", &team_a, module);
        assert!(first.contains("1 added"), "{first}");
        assert!(first.contains("no remote backend"), "{first}");
        assert!(
            first.contains(&format!(
                "OpenTofu state: {}/terraform.tfstate",
                team_a.display()
            )),
            "{first}"
        );
        assert!(
            first.contains(&format!("plugin={}", root.join("plugins").display())),
            "{first}"
        );
        let state_file = team_a.join("terraform.tfstate");
        assert!(state_file.is_file(), "state file missing");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(&team_a).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700, "state directory mode {dir_mode:o}");
        }
        assert!(!state_file.starts_with(root.join("work")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&state_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "state mode {mode:o}");
        }
        let second = run_wrapper(&root, "apply", &team_a, module);
        assert!(second.contains("No changes"), "{second}");
        assert!(!second.contains("1 added"), "{second}");
        let team_b = root.join("cache").join("team-b").join("tpl");
        let other = run_wrapper(&root, "apply", &team_b, module);
        assert!(other.contains("1 added"), "{other}");
        assert!(team_b.join("terraform.tfstate").is_file());
        let remote =
            "terraform {\n  backend \"s3\" {\n    bucket = \"demo\"\n    key = \"demo\"\n  }\n}\n";
        let logged = run_wrapper(&root, "apply", &team_a, remote);
        assert!(logged.contains("does not keep a copy"), "{logged}");
        assert!(!logged.contains("no remote backend"), "{logged}");
        let stage = root
            .join("work")
            .join("tfsrc")
            .join("koreograf-local-backend.tf");
        assert!(!stage.exists(), "remote backend must not be replaced");
        let rejected = root.join("cache").join("..").join("escaped");
        let bounced = run_wrapper(&root, "apply", &rejected, module);
        assert!(bounced.contains("state path was rejected"), "{bounced}");
        assert!(!rejected.join("terraform.tfstate").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opentofu_backend_detection_stays_in_the_root_module() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("koreograf-tofu-backend-{nanos}"));
        write_executable(&root.join("bin").join("tofu"), "#!/bin/sh\nexit 0\n");
        let plain = "resource \"terraform_data\" \"marker\" {\n  input = \"koreograf\"\n}\n";
        let state = root.join("cache").join("team-a").join("tpl");
        let commented = run_wrapper_files(
            &root,
            "apply",
            &state,
            "# backend \"s3\" {\n#   bucket = \"demo\"\n# }\n// backend \"gcs\" {}\n",
            &[],
        );
        assert!(commented.contains("no remote backend"), "{commented}");
        assert!(root
            .join("work")
            .join("tfsrc")
            .join("koreograf-local-backend.tf")
            .is_file());
        let nested = run_wrapper_files(
            &root,
            "apply",
            &state,
            plain,
            &[(
                "modules/child/main.tf",
                "terraform {\n  backend \"s3\" {\n    bucket = \"demo\"\n  }\n}\n",
            )],
        );
        assert!(nested.contains("no remote backend"), "{nested}");
        assert!(root
            .join("work")
            .join("tfsrc")
            .join("koreograf-local-backend.tf")
            .is_file());
        let cloud = run_wrapper_files(
            &root,
            "apply",
            &state,
            "terraform {\n  cloud {\n    organization = \"demo\"\n  }\n}\n",
            &[],
        );
        assert!(cloud.contains("does not keep a copy"), "{cloud}");
        assert!(!cloud.contains("no remote backend"), "{cloud}");
        assert!(!root
            .join("work")
            .join("tfsrc")
            .join("koreograf-local-backend.tf")
            .exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn container_mounts_the_template_state_only_for_opentofu() {
        let cache = Path::new("/data/cache/team-a");
        let stage = Path::new("/data/jobs/job/project");
        let work = Path::new("/data/jobs/job");
        let ansible = container_mounts(stage, work, cache, None, "tpl-1");
        assert!(ansible
            .iter()
            .all(|(_, guest, _)| guest != "/cache/tofu-state"));
        assert!(ansible
            .iter()
            .all(|(_, guest, _)| !guest.starts_with("/cache/tofu-state")));
        let state = cache.join("tofu-state").join("tpl-1");
        let tofu = container_mounts(stage, work, cache, Some(&state), "tpl-1");
        let mounted: Vec<_> = tofu
            .iter()
            .filter(|(_, guest, _)| guest.starts_with("/cache/tofu-state"))
            .collect();
        assert_eq!(mounted.len(), 1);
        assert_eq!(mounted[0].0, state.display().to_string());
        assert_eq!(mounted[0].1, "/cache/tofu-state/tpl-1");
        assert!(!mounted[0].2);
    }

    #[test]
    fn opentofu_apply_is_a_no_op_the_second_time() {
        let tofu = std::process::Command::new("tofu")
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !tofu {
            eprintln!("tofu is not installed; the live state test was skipped");
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("koreograf-tofu-live-{nanos}"));
        let module = "terraform {\n  required_version = \">= 1.5.0\"\n}\nresource \"terraform_data\" \"marker\" {\n  input = \"koreograf\"\n}\n";
        let team_a = root.join("cache").join("team-a").join("tpl");
        let first = run_wrapper(&root, "apply", &team_a, module);
        assert!(first.contains("1 added"), "{first}");
        let second = run_wrapper(&root, "apply", &team_a, module);
        assert!(
            second.contains("No changes. Your infrastructure matches the configuration."),
            "{second}"
        );
        assert!(second.contains("0 added"), "{second}");
        let team_b = root.join("cache").join("team-b").join("tpl");
        let other = run_wrapper(&root, "apply", &team_b, module);
        assert!(other.contains("1 added"), "{other}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recap_replaces_fatal_lines_with_failed_hosts() {
        let mut stored = String::new();
        for line in [
            "fatal: [ghost]: FAILED! => {\"msg\": \"no\"}",
            "fatal: [web-1]: UNREACHABLE! => {}",
            "PLAY RECAP *********************************************************************",
            "web-1                      : ok=1    changed=0    unreachable=0    failed=1    skipped=0    rescued=0    ignored=0",
            "db-1                       : ok=1    changed=0    unreachable=1    failed=0    skipped=0    rescued=0    ignored=0",
            "ok-1                       : ok=2    changed=0    unreachable=0    failed=0    skipped=1    rescued=0    ignored=0",
        ] {
            if let Some(next) = observe_host_line(&stored, line) {
                stored = next;
            }
        }
        assert_eq!(
            failed_host_names(&stored),
            vec!["web-1".to_string(), "db-1".to_string()]
        );
        let limit = limit_from_hosts(&failed_host_names(&stored)).unwrap();
        assert_eq!(limit, "web-1,db-1");
        let params = parameters_with_limit(r#"{"spec":{"tags":"ping"}}"#, &limit).unwrap();
        let parsed: Value = serde_json::from_str(&params).unwrap();
        assert_eq!(parsed["spec"]["limit"], "web-1,db-1");
        assert_eq!(parsed["spec"]["tags"], "ping");
        assert!(limit_from_hosts(&["all:&rest".into()]).is_err());
    }

    #[test]
    fn fatal_lines_stay_when_the_recap_never_arrives() {
        let stored = observe_host_line("", "fatal: [web-1]: FAILED! => {}").unwrap();
        assert_eq!(failed_host_names(&stored), vec!["web-1".to_string()]);
        assert!(observe_host_line(&stored, "ok: [web-1]").is_none());
    }

    #[test]
    fn output_limit_keeps_three_quarters_and_a_quarter() {
        assert_eq!(output_split(8_000), (6_000, 2_000));
        assert_eq!(output_split(100), (75, 25));
    }

    #[test]
    fn output_tail_stays_within_the_byte_cap() {
        let mut kept = OutputTail::with_byte_cap(8_000, 64);
        for i in 0..8_100 {
            let line = format!("row-{i:05}-xxxxxxxx");
            if let Some(stored) = kept.accept(line) {
                assert!(stored.starts_with("row-"), "{stored}");
            }
        }
        assert!(kept.tail_bytes <= 64, "{}", kept.tail_bytes);
        assert!(kept.omitted > 0);
        assert!(kept.accept("MARKER-TAIL-19".into()).is_none());
        assert!(kept.tail_bytes <= 64, "{}", kept.tail_bytes);
        assert_eq!(kept.tail.back().map(String::as_str), Some("MARKER-TAIL-19"));
        assert!(kept.tail.iter().all(|line| line.len() <= 64));
    }

    #[tokio::test]
    async fn oversized_stdout_keeps_the_tail_and_the_failed_hosts() {
        let state = output_state(8_000);
        insert_output_job(&state, "job-1");
        state.metrics.begin(&JobMeta {
            job_id: "job-1",
            team_id: "team",
            tool: "ansible",
            playbook: "site.yml",
            template: "Overflow",
        });
        let mut lines = Vec::new();
        for i in 1..=8_010 {
            lines.push(format!("bulk-{i}"));
        }
        lines[6_000] = "TASK [hidden.role : drop]".into();
        lines[6_001] = "ok: [h1]".into();
        lines.push("TASK [kept.role : stay]".into());
        lines.push("ok: [h2]".into());
        lines.push("fatal: [ghost]: FAILED! => {}".into());
        lines.push("PLAY RECAP".into());
        lines.push(
            "web-9 : ok=1    changed=0    unreachable=0    failed=1    skipped=0    rescued=0    ignored=0"
                .into(),
        );
        lines.push("MARKER-TAIL-19".into());
        let body = lines.join("\n") + "\n";
        pump(
            state.clone(),
            "job-1".into(),
            "stdout".into(),
            std::io::Cursor::new(body.into_bytes()),
            8_000,
        )
        .await;
        let text = event_text(&state, "job-1");
        assert!(text.contains("MARKER-TAIL-19"), "{text}");
        assert!(text.contains("bulk-1\n"), "{text}");
        assert!(text.contains("bulk-6000\n"), "{text}");
        assert!(
            text.contains("Omitted 16 lines (limit 8000; kept the first 6000 and the last 2000)."),
            "{text}"
        );
        assert!(!text.contains("bulk-6016"), "{text}");
        let head_at = text.find("bulk-6000").expect("head");
        let omit_at = text.find("Omitted 16 lines").expect("omission");
        let tail_at = text.find("bulk-6017").expect("tail");
        assert!(head_at < omit_at && omit_at < tail_at);
        let hosts = {
            let db = state.db().unwrap();
            let job = db::job(&db, "job-1").unwrap().unwrap();
            failed_host_names(&job.failed_hosts)
        };
        assert_eq!(hosts, vec!["web-9".to_string()]);
        let metrics = state.metrics.render(&[]);
        assert!(metrics.contains("role=\"hidden.role\""), "{metrics}");
        assert!(metrics.contains("role=\"kept.role\""), "{metrics}");
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }

    #[tokio::test]
    async fn a_short_log_is_stored_without_an_omission_line() {
        let state = output_state(8_000);
        insert_output_job(&state, "job-short");
        pump(
            state.clone(),
            "job-short".into(),
            "stdout".into(),
            std::io::Cursor::new(b"one\ntwo\nthree\n".to_vec()),
            8_000,
        )
        .await;
        let text = event_text(&state, "job-short");
        assert_eq!(text, "one\ntwo\nthree");
        assert!(!text.contains("Omitted"));
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }

    #[tokio::test]
    async fn cancel_still_discards_the_rest_of_the_log() {
        let state = output_state(8_000);
        insert_output_job(&state, "job-cancel");
        state
            .canceled
            .lock()
            .unwrap()
            .insert("job-cancel".to_string());
        let body = "bulk-1\nfatal: [ghost]: FAILED! => {}\nweb-9 : ok=1 changed=0 unreachable=0 failed=1 skipped=0 rescued=0 ignored=0\n";
        pump(
            state.clone(),
            "job-cancel".into(),
            "stdout".into(),
            std::io::Cursor::new(body.as_bytes().to_vec()),
            8_000,
        )
        .await;
        let text = event_text(&state, "job-cancel");
        assert_eq!(
            text,
            "Further output is discarded because the job was canceled."
        );
        let hosts = {
            let db = state.db().unwrap();
            let job = db::job(&db, "job-cancel").unwrap().unwrap();
            failed_host_names(&job.failed_hosts)
        };
        assert!(hosts.is_empty(), "{hosts:?}");
        let _ = std::fs::remove_dir_all(&state.cfg.data_dir);
    }

    fn output_state(lines: u64) -> Shared {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-output-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        crate::boot(crate::Config {
            bind: "127.0.0.1:0".into(),
            data_dir: dir,
            web_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web"),
            examples_dir: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            admin_user: "admin".into(),
            admin_password: Some("test-admin-pass".into()),
            runner: crate::config::RunnerMode::Local,
            job_timeout_secs: 60,
            job_output_lines: lines,
            local_jobs: crate::config::LocalJobs::All,
            project_roots: Vec::new(),
            hsts: false,
            cookie_secure: crate::config::CookieSecure::Auto,
            trusted_proxy: false,
            data_host: None,
            ldap: None,
        })
        .expect("boot")
    }

    fn insert_output_job(state: &Shared, id: &str) {
        let db = state.db().unwrap();
        db::insert_job(
            &db,
            &db::Job {
                id: id.into(),
                team_id: "team".into(),
                template_id: None,
                workflow_job_id: None,
                workflow_node_id: None,
                name: "Overflow".into(),
                tool: "ansible".into(),
                status: "running".into(),
                extra_vars: "{}".into(),
                launched_by: "admin".into(),
                trigger: "manual".into(),
                started_at: None,
                finished_at: None,
                created_at: "2026-01-01T00:00:00Z".into(),
                parameters: "{}".into(),
                failed_hosts: "[]".into(),
            },
        )
        .unwrap();
    }

    fn event_text(state: &Shared, job_id: &str) -> String {
        let db = state.db().unwrap();
        let mut after = 0;
        let mut lines = Vec::new();
        loop {
            let batch = db::events_after(&db, job_id, after, 500).unwrap();
            if batch.is_empty() {
                break;
            }
            for event in &batch {
                after = event.id;
                lines.push(event.line.clone());
            }
            if batch.len() < 500 {
                break;
            }
        }
        lines.join("\n")
    }

    #[test]
    fn an_empty_vault_id_reaches_ansible_playbook() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("koreograf-vault-{nanos}"));
        let bin = root.join("bin");
        let project = root.join("project");
        let work = root.join("work");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let fake = bin.join("ansible-playbook");
        std::fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$*\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&fake).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&fake, perms).unwrap();
        }
        let default_pass = work.join("default.pass");
        let prod_pass = work.join("prod.pass");
        let unnamed = vault_list_line("", &default_pass.display().to_string()).unwrap();
        let named = vault_list_line("prod", &prod_pass.display().to_string()).unwrap();
        assert!(unnamed.starts_with('|'), "{unnamed}");
        assert!(vault_list_line("bad|id", "/tmp/vault.pass").is_err());
        let list = work.join("vault.list");
        std::fs::write(&list, format!("{unnamed}\n{named}\n")).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let output = std::process::Command::new("sh")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wrapper.sh"))
            .env_clear()
            .env("PATH", &path)
            .env("KOREOGRAF_TOOL", "ansible")
            .env("KOREOGRAF_WORK", &work)
            .env("KOREOGRAF_PROJECT", &project)
            .env("KOREOGRAF_COLLECTIONS", root.join("collections"))
            .env("KOREOGRAF_ROLES", root.join("roles"))
            .env("KOREOGRAF_TOFU", root.join("tofu"))
            .env("KOREOGRAF_INVENTORY", work.join("hosts"))
            .env("KOREOGRAF_ENTRYPOINT", "site.yml")
            .env("KOREOGRAF_EXTRA", work.join("extra.json"))
            .env("KOREOGRAF_VAULT_LIST", &list)
            .output()
            .expect("wrapper");
        let text = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "status {:?} stdout={text} stderr={err}",
            output.status
        );
        assert!(
            text.contains(&format!("--vault-password-file {}", default_pass.display())),
            "{text}"
        );
        assert!(
            text.contains(&format!("--vault-id prod@{}", prod_pass.display())),
            "{text}"
        );
        assert!(text.contains("Starting:"), "{text}");
        assert!(!text.contains("Starte:"), "{text}");
        assert!(!text.contains("Shell-Skript"), "{text}");
        assert!(!text.contains("Unbekanntes Werkzeug"), "{text}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runner_env_wins_over_extra_vars() {
        let extra = serde_json::json!({
            "KOREOGRAF_ENTRYPOINT": "/tmp/evil.sh",
            "PATH": "/tmp/evil",
            "LD_PRELOAD": "/tmp/lib.so",
            "alertname": "DiskFull"
        });
        let mut env = Vec::new();
        export_plain_vars(&extra, &mut env);
        assert!(env
            .iter()
            .any(|(key, value)| key == "alertname" && value == "DiskFull"));
        assert!(!env.iter().any(|(key, _)| key == "KOREOGRAF_ENTRYPOINT"));
        assert!(!env.iter().any(|(key, _)| key == "PATH"));
        assert!(!env.iter().any(|(key, _)| key == "LD_PRELOAD"));
        env.extend(base_env(false, Path::new("/work")));
        env.push(("KOREOGRAF_ENTRYPOINT".into(), "health.sh".into()));
        let entry = env
            .iter()
            .rev()
            .find(|(key, _)| key == "KOREOGRAF_ENTRYPOINT")
            .expect("entrypoint");
        assert_eq!(entry.1, "health.sh");
    }

    #[test]
    fn ansible_extra_vars_are_unsafe_yaml() {
        let extra = serde_json::json!({
            "alertname": "{{ lookup('pipe', 'id') }}",
            "count": 2,
            "ok": true,
            "missing": null,
            "labels": {"instance": "web-1"},
            "hosts": ["a"]
        });
        let yaml = ansible_extra_yaml(&extra);
        assert!(yaml.starts_with("---\n"), "{yaml}");
        assert!(
            yaml.contains("alertname: !unsafe '{{ lookup(''pipe'', ''id'') }}'"),
            "{yaml}"
        );
        assert!(yaml.contains("count: 2"), "{yaml}");
        assert!(yaml.contains("ok: true"), "{yaml}");
        assert!(yaml.contains("missing: null"), "{yaml}");
        assert!(yaml.contains("instance: !unsafe 'web-1'"), "{yaml}");
        assert!(yaml.contains("- !unsafe 'a'"), "{yaml}");
        assert!(!yaml.contains("lookup('pipe'"), "{yaml}");
    }

    #[test]
    fn scrub_removes_secret_files_and_keeps_notes() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let work = std::env::temp_dir().join(format!("koreograf-scrub-{nanos}"));
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("ssh_key"), "secret\n").unwrap();
        std::fs::write(work.join("0-connection.pass"), "secret\n").unwrap();
        std::fs::write(work.join("git-askpass"), "secret\n").unwrap();
        std::fs::write(work.join("notes.txt"), "keep\n").unwrap();
        std::fs::write(work.join("extra.json"), "{}\n").unwrap();
        scrub_work_dir(&work);
        assert!(!work.join("ssh_key").exists());
        assert!(!work.join("0-connection.pass").exists());
        assert!(!work.join("git-askpass").exists());
        assert_eq!(
            std::fs::read_to_string(work.join("notes.txt")).unwrap(),
            "keep\n"
        );
        assert!(work.join("extra.json").exists());
        let _ = std::fs::remove_dir_all(work);
    }

    #[test]
    fn scrub_removes_files_from_every_credential_kind() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("koreograf-scrub-root-{nanos}"));
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "keep-outside\n").unwrap();
        std::fs::write(work.join("notes.txt"), "keep\n").unwrap();
        std::fs::write(work.join("future.secret"), "new-kind\n").unwrap();
        let mut written = std::collections::BTreeSet::new();
        for kind in crate::credentials::KINDS {
            let prepared = crate::credentials::prepare(kind, &sample_secret(kind), false, &work)
                .unwrap_or_else(|err| panic!("{kind}: {err}"));
            for (name, body) in prepared.files {
                write_secret(&work.join(&name), &body)
                    .unwrap_or_else(|err| panic!("{name}: {err}"));
                written.insert(name);
            }
        }
        write_secret(&work.join("vault.list"), "prod|/tmp/vault.pass").unwrap();
        write_env_file(
            &work.join("container.env"),
            &[("TOKEN".into(), "value".into())],
        )
        .unwrap();
        written.insert("vault.list".into());
        written.insert("container.env".into());
        for required in ["ssh_key", "vault.pass", "git.pass", "gce.json", "auth.json"] {
            assert!(
                written.contains(required),
                "missing {required} in {written:?}"
            );
        }
        use std::io::Write;
        let mut manifest = std::fs::OpenOptions::new()
            .append(true)
            .open(work.join(SECRET_MANIFEST))
            .unwrap();
        writeln!(manifest, "future.secret").unwrap();
        writeln!(manifest, "../outside.txt").unwrap();
        writeln!(manifest, "/etc/passwd").unwrap();
        scrub_work_dir(&work);
        for name in &written {
            assert!(!work.join(name).exists(), "{name} was left behind");
        }
        assert!(!work.join("future.secret").exists());
        assert!(!work.join(SECRET_MANIFEST).exists());
        assert_eq!(
            std::fs::read_to_string(work.join("notes.txt")).unwrap(),
            "keep\n"
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep-outside\n");
        let _ = std::fs::remove_dir_all(root);
    }

    fn sample_secret(kind: &str) -> serde_json::Value {
        match kind {
            "ssh" => serde_json::json!({
                "username": "alice",
                "password": "pw",
                "private_key": "test-key",
                "passphrase": "pp",
                "become_method": "sudo",
                "become_username": "root",
                "become_password": "bp"
            }),
            "vault" => serde_json::json!({"password": "vault", "vault_id": "prod"}),
            "scm" => serde_json::json!({
                "username": "git",
                "password": "pw",
                "private_key": "test-key",
                "passphrase": "pp"
            }),
            "net" => serde_json::json!({
                "username": "net",
                "password": "pw",
                "private_key": "test-key",
                "authorize_password": "auth"
            }),
            "aws" => serde_json::json!({
                "access_key": "AKIA",
                "secret_key": "secret",
                "security_token": "tok"
            }),
            "azure" => serde_json::json!({
                "subscription_id": "sub",
                "client_id": "client",
                "secret": "secret",
                "tenant": "tenant"
            }),
            "gce" => serde_json::json!({"service_account": "{\"type\":\"service_account\"}"}),
            "vmware" => serde_json::json!({
                "host": "vcenter.example",
                "username": "user",
                "password": "pw"
            }),
            "openstack" => serde_json::json!({
                "auth_url": "https://cloud.example",
                "username": "user",
                "password": "pw",
                "project": "demo",
                "domain": "Default"
            }),
            "registry" => serde_json::json!({
                "host": "ghcr.io",
                "username": "user",
                "password": "pw"
            }),
            "github" | "gitlab" => serde_json::json!({"token": "token-value"}),
            "galaxy" => serde_json::json!({
                "token": "token-value",
                "url": "https://galaxy.example"
            }),
            "kubernetes" => {
                serde_json::json!({"host": "https://k8s.example", "token": "token-value"})
            }
            "insights" => serde_json::json!({"username": "user", "password": "pw"}),
            "env" => serde_json::json!({"CUSTOM_TOKEN": "value"}),
            "netbox" => serde_json::json!({
                "token": "token-value",
                "url": "https://netbox.example"
            }),
            other => panic!("add a sample secret for {other}"),
        }
    }

    #[test]
    fn path_projects_reject_sensitive_directories() {
        let data = std::env::temp_dir().join(format!(
            "koreograf-paths-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(
            authorize_project_source("path", "/etc", &data, &[], true),
            Err(ProjectPathError::Denied)
        );
        assert_eq!(
            authorize_project_source("path", "/etc-custom-not-real", &data, &[], true),
            Err(ProjectPathError::NotDirectory)
        );
        assert_eq!(
            authorize_project_source("path", &data.display().to_string(), &data, &[], true),
            Err(ProjectPathError::Denied)
        );
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
        authorize_project_source("path", &examples.display().to_string(), &data, &[], true)
            .expect("admin examples");
        assert_eq!(
            authorize_project_source("path", &examples.display().to_string(), &data, &[], false),
            Err(ProjectPathError::AdminsOnly)
        );
        authorize_project_source(
            "path",
            &examples.display().to_string(),
            &data,
            &[examples.parent().unwrap().to_path_buf()],
            false,
        )
        .expect("operator under root");
        assert_eq!(
            authorize_project_source("git", "file:///etc", &data, &[], true),
            Err(ProjectPathError::Denied)
        );
        assert_eq!(
            authorize_project_source("git", "file://localhost/etc", &data, &[], true),
            Err(ProjectPathError::Denied)
        );
        assert_eq!(
            authorize_project_source("git", "file://evil.example/tmp", &data, &[], true),
            Err(ProjectPathError::Denied)
        );
        assert!(!file_clone_allowed("https://example.com/demo.git", &data, &[]).unwrap());
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn the_seeded_demo_stays_inside_the_examples_root() {
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
        let demo = examples.join("demo");
        let data = std::env::temp_dir().join("koreograf-demo-root");
        let roots = crate::config::effective_project_roots(Vec::new(), &examples);
        let canonical = stored_directory(&demo, &data, &roots).expect("demo");
        assert_eq!(canonical, demo.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn a_swapped_symlink_is_outside_the_project_root() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("koreograf-symlink-{nanos}"));
        let root = base.join("root");
        let inside = root.join("project");
        let outside = base.join("outside");
        let data = base.join("data");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&inside, &link).unwrap();
        let roots = [root.clone()];
        authorize_project_source("path", &link.display().to_string(), &data, &roots, false)
            .expect("link inside the root");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let project = db::Project {
            id: "linked".into(),
            team_id: "team".into(),
            organization_id: String::new(),
            organization_name: String::new(),
            name: "Linked".into(),
            source_type: "path".into(),
            source: link.display().to_string(),
            revision: String::new(),
            created_at: String::new(),
            team_ids: Vec::new(),
            admin_saved: true,
        };
        let proxy = crate::settings::ProxySettings::inherit();
        let err = materialize_project(&project, &data, &proxy, true, &roots)
            .await
            .unwrap_err();
        assert_eq!(err, "the_project_path_is_not_allowed");
        let file_url = format!("file://{}", link.display());
        let file_err = file_clone_allowed(&file_url, &data, &roots).unwrap_err();
        assert_eq!(file_err, "the_project_path_is_not_allowed");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&inside, &link).unwrap();
        let outside_roots = materialize_project(&project, &data, &proxy, true, &[])
            .await
            .unwrap_err();
        assert_eq!(outside_roots, "the_project_path_is_not_allowed");
        let path = materialize_project(&project, &data, &proxy, true, &roots)
            .await
            .unwrap();
        assert_eq!(path, inside.canonicalize().unwrap());
        assert!(file_clone_allowed(&file_url, &data, &roots).unwrap());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_mount_of_the_data_dir_is_rejected() {
        let data = std::env::temp_dir();
        assert!(covers_data_dir(&data, &data));
        assert!(covers_data_dir(Path::new("/"), &data));
        let child = data.join("koreograf-cache-child");
        std::fs::create_dir_all(&child).unwrap();
        assert!(!covers_data_dir(&child, &data));
        let _ = std::fs::remove_dir(child);
    }
    #[tokio::test]
    async fn a_process_group_signal_stops_the_children() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-kill-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 60 & echo $! > child.pid; wait")
            .current_dir(&dir)
            .process_group(0)
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().expect("pid");
        let marker = dir.join("child.pid");
        let mut grandchild = String::new();
        for _ in 0..50 {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                grandchild = text.trim().to_string();
                if !grandchild.is_empty() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!grandchild.is_empty(), "child did not start");
        let before = std::fs::read_to_string(format!("/proc/{grandchild}/status")).ok();
        assert!(signal_process_group(pid, false), "signal");
        let finished = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("group did not exit");
        assert!(finished.is_ok());
        // child.wait returns when the leader exits. The grandchild can still
        // be a zombie for a moment, and a zombie's cmdline is empty.
        let mut after = None;
        let mut gone = false;
        for _ in 0..60 {
            after = std::fs::read_to_string(format!("/proc/{grandchild}/cmdline")).ok();
            if after
                .as_deref()
                .is_none_or(|text| text.is_empty() || !text.contains("sleep"))
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            gone,
            "grandchild {grandchild} is still sleep; status={before:?} cmdline={after:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn stop_after_grace_kills_a_detached_child() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("koreograf-kill-detached-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("( trap '' TERM; sleep 60 & echo $! > child.pid; wait ) >/dev/null 2>&1 & wait")
            .current_dir(&dir)
            .process_group(0)
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().expect("pid");
        struct KillGroup(u32);
        impl Drop for KillGroup {
            fn drop(&mut self) {
                let _ = signal_process_group(self.0, true);
            }
        }
        let _kill = KillGroup(pid);
        let marker = dir.join("child.pid");
        let mut grandchild = String::new();
        for _ in 0..50 {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                grandchild = text.trim().to_string();
                if !grandchild.is_empty() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!grandchild.is_empty(), "detached child did not start");
        assert!(
            signal_process_group(pid, false),
            "the group did not accept SIGTERM"
        );
        let finished = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
        assert!(finished.is_ok(), "the leader stayed up after SIGTERM");
        let still = std::fs::read_to_string(format!("/proc/{grandchild}/cmdline")).ok();
        assert!(
            still.as_deref().is_some_and(|text| text.contains("sleep")),
            "SIGTERM stopped the detached child; cmdline={still:?}"
        );
        assert!(
            process_group_alive(pid),
            "the group was gone before the grace kill"
        );
        let proc = RunningProc {
            pid,
            container: None,
            bin: None,
        };
        assert!(stop_after_grace(&proc, false).await, "grace kill failed");
        let mut gone = false;
        let mut after = None;
        for _ in 0..40 {
            after = std::fs::read_to_string(format!("/proc/{grandchild}/cmdline")).ok();
            if after
                .as_deref()
                .is_none_or(|text| text.is_empty() || !text.contains("sleep"))
            {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            gone,
            "detached child {grandchild} survived SIGKILL; cmdline={after:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn inspect_errors_are_not_a_stopped_container() {
        assert!(matches!(
            classify_inspect(true, "true\n", ""),
            ContainerPresence::Running
        ));
        assert!(matches!(
            classify_inspect(true, "false\n", ""),
            ContainerPresence::Stopped
        ));
        assert!(matches!(
            classify_inspect(false, "", "Error: No such container: job"),
            ContainerPresence::Stopped
        ));
        assert!(matches!(
            classify_inspect(false, "", "Error: No such object: job"),
            ContainerPresence::Stopped
        ));
        assert!(matches!(
            classify_inspect(false, "", "Cannot connect to the Docker daemon"),
            ContainerPresence::Unknown
        ));
        assert!(matches!(
            container_state("koreograf-missing-runtime", "job"),
            ContainerPresence::Unknown
        ));
    }
}
