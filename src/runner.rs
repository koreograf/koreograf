use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use crate::cacheinfo::dir_size;
use crate::config::RunnerMode;
use crate::db::{self, Environment, Project, Template};
use crate::metrics::{self, JobMeta};
use crate::Shared;

const WRAPPER: &str = include_str!("wrapper.sh");
const MAX_FILES: u32 = 4_000;
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const DEFAULT_INVENTORY: &str = "[all]\nlocalhost ansible_connection=local\n";

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
    pub mounts: Vec<(String, String, bool)>,
    /// Host path. Docker reads `--env-file` on the host, not inside the container.
    pub env_file: String,
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
    for (host, guest, readonly) in &spec.mounts {
        let mut mount = format!("{host}:{guest}");
        if *readonly {
            mount.push_str(":ro");
        }
        args.push("-v".into());
        args.push(mount);
    }
    args.push("--env-file".into());
    args.push(spec.env_file.clone());
    args.push("-w".into());
    args.push("/runner/project".into());
    args.push("--entrypoint".into());
    args.push("sh".into());
    args.push(spec.image.clone());
    args.extend(spec.command.iter().cloned());
    args
}

pub async fn detect_runtime() -> Option<String> {
    for bin in ["docker", "podman"] {
        let ok = Command::new(bin)
            .arg("version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false);
        if ok {
            return Some(bin.to_string());
        }
    }
    None
}

pub fn signal_stop(state: &Shared, job_id: &str, proc: RunningProc) {
    let state = state.clone();
    let job_id = job_id.to_string();
    tokio::spawn(async move {
        if !deliver_stop(&proc, false) {
            log(
                &state,
                &job_id,
                "system",
                "The stop signal could not be delivered.",
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        if still_running(&proc) && !deliver_stop(&proc, true) {
            log(
                &state,
                &job_id,
                "system",
                "The stop signal could not be delivered.",
            );
        }
    });
}

fn deliver_stop(proc: &RunningProc, kill_hard: bool) -> bool {
    let signal = if kill_hard { "KILL" } else { "TERM" };
    let container_ok = match (proc.bin.as_deref(), proc.container.as_deref()) {
        (Some(bin), Some(name)) => runtime_signal(bin, name, signal),
        _ => true,
    };
    let local_ok = if proc.pid > 1 {
        signal_process_group(proc.pid, kill_hard)
    } else {
        proc.container.is_some()
    };
    container_ok && local_ok
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

fn runtime_signal(bin: &str, name: &str, signal: &str) -> bool {
    if !container_running(bin, name) {
        return true;
    }
    std::process::Command::new(bin)
        .args(["kill", "--signal", signal, name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn container_running(bin: &str, name: &str) -> bool {
    let Ok(output) = std::process::Command::new(bin)
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
    else {
        return false;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
}

fn still_running(proc: &RunningProc) -> bool {
    let local = proc.pid > 1 && process_alive(proc.pid);
    let container = match (proc.bin.as_deref(), proc.container.as_deref()) {
        (Some(bin), Some(name)) => container_running(bin, name),
        _ => false,
    };
    local || container
}

fn process_alive(pid: u32) -> bool {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    !text
        .lines()
        .any(|line| line.starts_with("State:") && line.contains('Z'))
}

pub async fn execute(state: Shared, job_id: String) {
    let started = match state.db() {
        Ok(db) => db::mark_running(&db, &job_id).unwrap_or(false),
        Err(_) => false,
    };
    if !started {
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
    if let Ok(mut procs) = state.procs.lock() {
        procs.remove(&job_id);
    }
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
    let template = with_saved_parameters(template, &job);
    if !known_tool(&template.tool) {
        return Err(format!("Unknown tool {}.", template.tool));
    }
    check_relative(&template.entrypoint)?;

    let work = state.cfg.data_dir.join("jobs").join(job_id);
    let stage = work.join("project");
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let cache = state.cfg.data_dir.join("cache").join(&job.team_id);
    for sub in ["collections", "roles", "tofu"] {
        fs::create_dir_all(cache.join(sub)).map_err(|e| e.to_string())?;
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
            log(
                state,
                job_id,
                "system",
                "No container runtime is reachable. The run stays on the controller.",
            );
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
    fs::write(
        work.join("extra.json"),
        serde_json::to_vec_pretty(&extra).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    let mut env = base_env(container_mode.is_some(), &work);
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
    );
    let credential_ids = saved_credential_ids(&job, &template);
    apply_credentials(
        state,
        &template,
        &credential_ids,
        &work,
        container_mode.is_some(),
        &mut env,
    )?;
    export_plain_vars(&extra, &mut env);
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
            },
        )
        .await?
    } else {
        log(state, job_id, "system", "Local run on the controller.");
        let mut cmd = Command::new("sh");
        cmd.arg(work.join("wrapper.sh"))
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
        log(state, job_id, "system", &format!("Process group {pid}."));
        if canceled(state, job_id) {
            signal_stop(state, job_id, proc);
        }
        wait_child(state, job_id, child, settings.job_timeout_secs).await?
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
    finish(state, job_id, exit == 0);
    Ok(())
}

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
) {
    let (project, work_s, cols, roles, tofu, inventory, extra) = if container {
        (
            "/runner/project".into(),
            "/runner/work".into(),
            "/cache/collections".into(),
            "/cache/roles".into(),
            "/cache/tofu".into(),
            format!("/runner/work/{inventory_rel}"),
            "/runner/work/extra.json".into(),
        )
    } else {
        (
            stage.display().to_string(),
            work.display().to_string(),
            cache.join("collections").display().to_string(),
            cache.join("roles").display().to_string(),
            cache.join("tofu").display().to_string(),
            work.join(inventory_rel).display().to_string(),
            work.join("extra.json").display().to_string(),
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
        if !safe_ident(key) {
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
    Ok(())
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
    Ok(Some(environment))
}

pub fn valid_image(image: &str) -> bool {
    !image.is_empty()
        && image.len() < 200
        && image
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "/:._-@".contains(ch))
}

fn sync_log_line(err: String) -> String {
    if err == "sync_the_git_project_first" {
        "Sync the Git project first.".into()
    } else {
        err
    }
}

pub async fn materialize_project(
    project: &Project,
    data_dir: &Path,
    proxy: &crate::settings::ProxySettings,
    update: bool,
) -> Result<PathBuf, String> {
    match project.source_type.as_str() {
        "path" => {
            let path = PathBuf::from(&project.source);
            if !path.is_dir() {
                let shown = path.display().to_string();
                return Err(crate::error::message(
                    "project.path_not_directory",
                    &[("path", &shown)],
                ));
            }
            Ok(path)
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
            git_sync(&project.source, &project.revision, &dest, proxy).await?;
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

fn git_cmd(proxy: &crate::settings::ProxySettings) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("protocol.file.allow=always")
        .env("GIT_TERMINAL_PROMPT", "0")
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
) -> Result<(), String> {
    let mut cmd = git_cmd(proxy);
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
) -> Result<(), String> {
    let mut fetch = git_cmd(proxy);
    fetch
        .arg("-C")
        .arg(dest)
        .args(["fetch", "--depth", "1", "origin"])
        .arg(revision);
    git_run(fetch).await?;
    let mut checkout = git_cmd(proxy);
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
) -> Result<(), String> {
    if revision.is_empty() {
        let mut cmd = git_cmd(proxy);
        cmd.arg("-C").arg(dest).args(["pull", "--ff-only"]);
        return git_run(cmd).await;
    }
    git_checkout(revision, dest, proxy).await
}

async fn git_sync(
    url: &str,
    revision: &str,
    dest: &Path,
    proxy: &crate::settings::ProxySettings,
) -> Result<(), String> {
    if !valid_revision(revision) {
        return Err("revision_is_invalid".into());
    }
    if dest.exists() && !dest.join(".git").is_dir() {
        fs::remove_dir_all(dest)
            .map_err(|err| format!("Incomplete checkout cannot be removed: {err}"))?;
    }
    if dest.join(".git").is_dir() {
        return git_update(revision, dest, proxy).await;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    match git_clone(url, revision, dest, proxy).await {
        Ok(()) => Ok(()),
        Err(err) if !revision.is_empty() && missing_revision(&err) => {
            if dest.exists() {
                fs::remove_dir_all(dest).map_err(|remove| remove.to_string())?;
            }
            git_clone(url, "", dest, proxy).await?;
            git_checkout(revision, dest, proxy).await
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
    } = run;
    ensure_image(bin, &environment.image, &environment.pull_policy).await?;
    write_env_file(&work.join("container.env"), env)?;
    let name = format!("koreograf-{}", &job_id[..8]);
    let spec = ContainerSpec {
        image: environment.image.clone(),
        name: name.clone(),
        mounts: vec![
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
        ],
        env_file: work.join("container.env").display().to_string(),
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
    wait_child(state, job_id, child, timeout_secs).await
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
    let status = Command::new(bin)
        .args(["pull", image])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("Image pull: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Image {image} could not be pulled."))
    }
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

fn canceled(state: &Shared, job_id: &str) -> bool {
    state
        .db()
        .ok()
        .and_then(|db| db::job(&db, job_id).ok().flatten())
        .is_some_and(|job| job.status == "canceled")
}

async fn wait_child(
    state: &Shared,
    job_id: &str,
    mut child: tokio::process::Child,
    timeout_secs: u64,
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
    ));
    let err = tokio::spawn(pump(
        state.clone(),
        job_id.to_string(),
        "stderr".to_string(),
        stderr,
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
                if !deliver_stop(&proc, true) {
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

async fn pump<R>(state: Shared, job_id: String, stream: String, reader: R)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader);
    let mut buf = Vec::new();
    let mut count = 0i64;
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
                count += 1;
                if canceled(&state, &job_id) {
                    if !discarded {
                        discarded = true;
                        log(
                            &state,
                            &job_id,
                            "system",
                            "Further output is discarded because the job was canceled.",
                        );
                    }
                    continue;
                }
                if count > 8_000 {
                    if count == 8_001 {
                        log(
                            &state,
                            &job_id,
                            "system",
                            "Further output is discarded (limit 8000 lines).",
                        );
                    }
                    continue;
                }
                let mut line = String::from_utf8_lossy(&buf).to_string();
                if line.len() > 4_000 {
                    line.truncate(4_000);
                    line.push('…');
                }
                log(&state, &job_id, &stream, &line);
                if stream == "stdout" {
                    state.metrics.observe(&job_id, &line);
                }
            }
            Err(_) => break,
        }
    }
}

fn log(state: &Shared, job_id: &str, stream: &str, line: &str) {
    if let Ok(db) = state.db() {
        let _ = db::insert_event(&db, job_id, stream, line);
        if matches!(stream, "stdout" | "stderr") && host_line(line) {
            if let Ok(Some(job)) = db::job(&db, job_id) {
                if let Some(next) = observe_host_line(&job.failed_hosts, line) {
                    let _ = db::set_failed_hosts(&db, job_id, &next);
                }
            }
        }
    }
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
    let status = state
        .db()
        .ok()
        .and_then(|db| db::job(&db, job_id).ok().flatten())
        .map(|job| job.status)
        .unwrap_or_else(|| "failed".into());
    state.metrics.close(job_id, &status);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let proxy = crate::settings::ProxySettings::inherit();
        let path = materialize_project(&project, &dir, &proxy, false)
            .await
            .unwrap();
        assert_eq!(path, checkout);
        let missing = db::Project {
            id: "absent".into(),
            ..project
        };
        let err = materialize_project(&missing, &dir, &proxy, false)
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
            command: vec!["/runner/work/wrapper.sh".into()],
        };
        let argv = container_argv(&spec);
        assert!(argv.iter().any(|arg| arg.contains("/cache/collections")));
        assert!(argv
            .iter()
            .any(|arg| arg == "quay.io/ansible/awx-ee:latest"));
        assert_eq!(
            argv.last().map(String::as_str),
            Some("/runner/work/wrapper.sh")
        );
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
        let after = std::fs::read_to_string(format!("/proc/{grandchild}/cmdline")).ok();
        assert!(
            after.as_deref().is_none_or(|text| !text.contains("sleep")),
            "grandchild {grandchild} is still sleep; status={before:?} cmdline={after:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
