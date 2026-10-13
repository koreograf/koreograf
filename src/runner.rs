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
        summary: "ansible-playbook. Collections und Rollen bleiben im Team-Cache.",
    },
    Tool {
        id: "opentofu",
        summary: "OpenTofu init und plan oder apply. Provider liegen im Plugin-Cache.",
    },
    Tool {
        id: "shell",
        summary: "Shell-Skript aus dem Projekt oder als Inline-Skript.",
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
        return Err("spec muss ein Objekt sein.".into());
    }
    match spec.get("job_type") {
        None | Some(Value::Null) => {}
        Some(Value::String(job_type)) if job_type == "run" || job_type == "check" => {}
        _ => return Err("Art ist run oder check.".into()),
    }
    check_tag_list(spec, "tags", "Tags")?;
    check_tag_list(spec, "skip_tags", "Tags überspringen")?;
    if let Some(limit) = optional_text(spec, "limit")? {
        check_limit(&limit)?;
    }
    if let Some(diff) = spec.get("diff") {
        if !diff.is_boolean() {
            return Err("Diff ist ja oder nein.".into());
        }
    }
    if let Some(verbosity) = optional_u64(spec, "verbosity")? {
        if verbosity > 4 {
            return Err("Ausführlichkeit ist 0 bis 4.".into());
        }
    }
    if let Some(forks) = optional_u64(spec, "forks")? {
        if !(1..=50).contains(&forks) {
            return Err("Forks ist eine Zahl von 1 bis 50.".into());
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
        Some(_) => Err(format!("{key} muss Text sein.")),
    }
}

fn optional_u64(spec: &Value, key: &str) -> Result<Option<u64>, String> {
    match spec.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} muss eine ganze Zahl sein.")),
        Some(_) => Err(format!("{key} muss eine ganze Zahl sein.")),
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
        return Err(format!("{label} ist zu lang."));
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
            return Err(format!("{label} „{part}“ ist ungültig."));
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
    if text.len() > 200 || text.starts_with('-') || !allowed {
        return Err("Limit enthält ein ungültiges Zeichen.".into());
    }
    Ok(())
}

pub fn check_relative(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }
    if path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(
            "Der Einstiegspfad muss relativ sein und darf das Projekt nicht verlassen.".into(),
        );
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

pub fn signal_stop(proc: &RunningProc) {
    if proc.pid > 0 {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &proc.pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if let (Some(bin), Some(name)) = (proc.bin.as_deref(), proc.container.as_deref()) {
        let _ = std::process::Command::new(bin)
            .args(["kill", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
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
            .ok_or_else(|| "Auftrag fehlt.".to_string())?;
        let template = match &job.template_id {
            Some(id) => db::template(&db, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Auftragsvorlage fehlt.".to_string())?,
            None => return Err("Auftrag ohne Vorlage.".into()),
        };
        (job, template)
    };
    if !known_tool(&template.tool) {
        return Err(format!("Unbekanntes Werkzeug {}.", template.tool));
    }
    check_relative(&template.entrypoint)?;

    let work = state.cfg.data_dir.join("jobs").join(job_id);
    let stage = work.join("project");
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let cache = state.cfg.data_dir.join("cache").join(&job.team_id);
    for sub in ["collections", "roles", "tofu"] {
        fs::create_dir_all(cache.join(sub)).map_err(|e| e.to_string())?;
    }

    let project = load_project(state, &template).await?;
    if let Some(project) = &project {
        let source = materialize_project(project, &state.cfg.data_dir).await?;
        copy_tree(&source, &stage)?;
    }

    let mut entrypoint = template.entrypoint.clone();
    let spec: Value = serde_json::from_str(&template.spec).unwrap_or(Value::Null);
    if template.tool == "shell" && entrypoint.is_empty() {
        let script = spec.get("script").and_then(Value::as_str).ok_or_else(|| {
            "Shell-Vorlage braucht ein Skript oder einen Einstiegspfad.".to_string()
        })?;
        fs::write(stage.join("inline.sh"), script).map_err(|e| e.to_string())?;
        entrypoint = "inline.sh".into();
    }
    if entrypoint.is_empty() {
        return Err("Einstiegspfad fehlt.".into());
    }
    check_relative(&entrypoint)?;

    let inventory_rel = write_job_inventory(state, &template, &work)?;
    fs::write(work.join("wrapper.sh"), WRAPPER).map_err(|e| e.to_string())?;

    let mut extra = merge_vars(
        &db::json_object(&template.extra_vars),
        &db::json_object(&job.extra_vars),
    );
    let use_container = should_container(state.cfg.runner, template.environment_id.is_some());
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
        (true, None, Some(_)) if state.cfg.runner == RunnerMode::Docker => {
            return Err(
                "Execution Environment ist gesetzt, aber weder Docker noch Podman ist erreichbar."
                    .into(),
            );
        }
        (true, None, Some(_)) => {
            log(
                state,
                job_id,
                "system",
                "Keine Container-Runtime erreichbar. Der Lauf bleibt auf dem Controller.",
            );
            None
        }
        _ => {
            if state.cfg.runner == RunnerMode::Local && template.environment_id.is_some() {
                log(
                    state,
                    job_id,
                    "system",
                    "Runner-Modus ist lokal. Das Execution Environment wird ignoriert.",
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
    apply_credential(state, &template, &work, container_mode.is_some(), &mut env)?;
    export_plain_vars(&extra, &mut env);

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
            "Werkzeug {} · Cache vorher {} Bytes",
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
            },
        )
        .await?
    } else {
        log(state, job_id, "system", "Lokaler Lauf auf dem Controller.");
        let mut cmd = Command::new("sh");
        cmd.arg(work.join("wrapper.sh"))
            .current_dir(&work)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| format!("Start fehlgeschlagen: {e}"))?;
        let pid = child.id().unwrap_or(0);
        remember(
            state,
            job_id,
            RunningProc {
                pid,
                container: None,
                bin: None,
            },
        );
        if canceled(state, job_id) {
            signal_stop(&RunningProc {
                pid,
                container: None,
                bin: None,
            });
            return Ok(());
        }
        wait_child(state, job_id, child).await?
    };

    let cache_after = dir_size(&cache);
    log(
        state,
        job_id,
        "system",
        &format!("Cache nachher {cache_after} Bytes (vorher {cache_before})."),
    );
    if cache_after == cache_before && cache_before > 0 {
        log(
            state,
            job_id,
            "system",
            "Cache wiederverwendet, kein neues Wachstum.",
        );
    } else if cache_after > cache_before {
        log(
            state,
            job_id,
            "system",
            "Cache gewachsen. Der nächste Lauf verwendet diese Artefakte.",
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

fn apply_credential(
    state: &Shared,
    template: &Template,
    work: &Path,
    container: bool,
    env: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let Some(id) = &template.credential_id else {
        return Ok(());
    };
    let credential = {
        let db = state.db().map_err(|e| e.to_string())?;
        db::credential(&db, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Zugangsdaten fehlen.".to_string())?
    };
    if credential.team_id != template.team_id {
        return Err("Zugangsdaten gehören zu einem anderen Team.".into());
    }
    let plain = state
        .decrypt(&credential.secret_blob)
        .map_err(|_| "Zugangsdaten lassen sich nicht lesen.".to_string())?;
    let secret: Value =
        serde_json::from_str(&plain).map_err(|_| "Zugangsdaten sind kein JSON.".to_string())?;
    let guest = |name: &str| {
        if container {
            format!("/runner/work/{name}")
        } else {
            work.join(name).display().to_string()
        }
    };
    match credential.kind.as_str() {
        "ssh" => {
            let key = secret
                .get("private_key")
                .and_then(Value::as_str)
                .ok_or_else(|| "SSH-Zugang ohne private_key.".to_string())?;
            write_secret(&work.join("ssh_key"), key)?;
            env.push(("KOREOGRAF_PRIVATE_KEY".into(), guest("ssh_key")));
            if let Some(user) = secret.get("username").and_then(Value::as_str) {
                env.push(("KOREOGRAF_REMOTE_USER".into(), user.to_string()));
            }
        }
        "vault" => {
            let password = secret
                .get("password")
                .and_then(Value::as_str)
                .ok_or_else(|| "Vault-Zugang ohne password.".to_string())?;
            write_secret(&work.join("vault.pass"), password)?;
            env.push(("KOREOGRAF_VAULT_FILE".into(), guest("vault.pass")));
        }
        "env" => {
            let Some(map) = secret.as_object() else {
                return Err("Env-Zugang muss ein JSON-Objekt sein.".into());
            };
            for (key, value) in map {
                if !safe_ident(key) {
                    continue;
                }
                let Some(text) = value.as_str() else { continue };
                if text.contains('\n') || text.contains('\0') {
                    return Err(format!(
                        "Umgebungsvariable {key} enthält einen Zeilenumbruch."
                    ));
                }
                env.push((key.clone(), text.to_string()));
            }
        }
        other => return Err(format!("Unbekannte Zugangsart {other}.")),
    }
    Ok(())
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
        .ok_or_else(|| "Projekt fehlt.".to_string())?;
    let visible = db::project_visible(&db, id, &template.team_id).map_err(|e| e.to_string())?;
    if !visible {
        return Err("Das Projekt ist für dieses Team nicht sichtbar.".into());
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
        .ok_or_else(|| "Inventar fehlt.".to_string())?;
    if inventory.team_id != template.team_id {
        return Err("Inventar gehört zu einem anderen Team.".into());
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
        .ok_or_else(|| "Execution Environment fehlt.".to_string())?;
    if environment
        .team_id
        .as_ref()
        .is_some_and(|team| team != &template.team_id)
    {
        return Err("Execution Environment gehört zu einem anderen Team.".into());
    }
    if !valid_image(&environment.image) {
        return Err("Image-Name ist ungültig.".into());
    }
    Ok(Some(environment))
}

pub fn valid_image(image: &str) -> bool {
    !image.is_empty()
        && image.len() < 200
        && image
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "/:._-".contains(ch))
}

pub async fn materialize_project(project: &Project, data_dir: &Path) -> Result<PathBuf, String> {
    match project.source_type.as_str() {
        "path" => {
            let path = PathBuf::from(&project.source);
            if !path.is_dir() {
                return Err(format!(
                    "Projektpfad {} ist kein Verzeichnis.",
                    path.display()
                ));
            }
            Ok(path)
        }
        "git" => {
            if !valid_git(&project.source) {
                return Err("Git-Adresse ist ungültig.".into());
            }
            let dest = data_dir.join("checkouts").join(&project.id);
            git_sync(&project.source, &project.revision, &dest).await?;
            Ok(dest)
        }
        other => Err(format!("Unbekannte Projektquelle {other}.")),
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

fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("protocol.file.allow=always")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdin(Stdio::null())
        .kill_on_drop(true);
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
        format!("Git ist mit Status {} fehlgeschlagen.", output.status)
    } else {
        format!("Git:\n{joined}")
    }
}

async fn git_run(mut cmd: Command) -> Result<(), String> {
    let finished = tokio::time::timeout(Duration::from_secs(120), cmd.output()).await;
    let output = match finished {
        Ok(result) => result.map_err(|err| format!("git: {err}"))?,
        Err(_) => {
            return Err("Git hat das Zeitlimit von 120 Sekunden überschritten.".into());
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

async fn git_clone(url: &str, revision: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = git_cmd();
    cmd.arg("clone").arg("--depth").arg("1");
    if !revision.is_empty() {
        cmd.arg("--branch").arg(revision);
    }
    cmd.arg("--").arg(url).arg(dest);
    git_run(cmd).await
}

async fn git_checkout(revision: &str, dest: &Path) -> Result<(), String> {
    let mut fetch = git_cmd();
    fetch
        .arg("-C")
        .arg(dest)
        .args(["fetch", "--depth", "1", "origin"])
        .arg(revision);
    git_run(fetch).await?;
    let mut checkout = git_cmd();
    checkout
        .arg("-C")
        .arg(dest)
        .args(["checkout", "--detach", "FETCH_HEAD"]);
    git_run(checkout).await
}

async fn git_update(revision: &str, dest: &Path) -> Result<(), String> {
    if revision.is_empty() {
        let mut cmd = git_cmd();
        cmd.arg("-C").arg(dest).args(["pull", "--ff-only"]);
        return git_run(cmd).await;
    }
    git_checkout(revision, dest).await
}

async fn git_sync(url: &str, revision: &str, dest: &Path) -> Result<(), String> {
    if !valid_revision(revision) {
        return Err("Revision ist ungültig.".into());
    }
    if dest.exists() && !dest.join(".git").is_dir() {
        fs::remove_dir_all(dest)
            .map_err(|err| format!("Unvollständiges Checkout lässt sich nicht entfernen: {err}"))?;
    }
    if dest.join(".git").is_dir() {
        return git_update(revision, dest).await;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    match git_clone(url, revision, dest).await {
        Ok(()) => Ok(()),
        Err(err) if !revision.is_empty() && missing_revision(&err) => {
            if dest.exists() {
                fs::remove_dir_all(dest).map_err(|remove| remove.to_string())?;
            }
            git_clone(url, "", dest).await?;
            git_checkout(revision, dest).await
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
            return Err(
                "Projekt ist zu groß für einen Lauf (Grenze 4000 Dateien oder 200 MB).".into(),
            );
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
}

async fn run_container(state: &Shared, job_id: &str, run: ContainerRun<'_>) -> Result<i32, String> {
    let ContainerRun {
        bin,
        environment,
        work,
        stage,
        cache,
        env,
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
    let child = cmd
        .spawn()
        .map_err(|e| format!("{bin} run fehlgeschlagen: {e}"))?;
    let pid = child.id().unwrap_or(0);
    remember(
        state,
        job_id,
        RunningProc {
            pid,
            container: Some(name.clone()),
            bin: Some(bin.to_string()),
        },
    );
    if canceled(state, job_id) {
        signal_stop(&RunningProc {
            pid,
            container: Some(name),
            bin: Some(bin.to_string()),
        });
        return Ok(1);
    }
    wait_child(state, job_id, child).await
}

fn write_env_file(path: &Path, env: &[(String, String)]) -> Result<(), String> {
    let mut body = String::new();
    for (key, value) in env {
        if value.contains('\n') {
            return Err(format!(
                "Variable {key} lässt sich nicht an den Container übergeben."
            ));
        }
        body.push_str(key);
        body.push('=');
        body.push_str(value);
        body.push('\n');
    }
    fs::write(path, body).map_err(|e| e.to_string())
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
        .map_err(|e| format!("Image-Pull: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Image {image} konnte nicht gezogen werden."))
    }
}

fn remember(state: &Shared, job_id: &str, proc: RunningProc) {
    if let Ok(mut procs) = state.procs.lock() {
        procs.insert(job_id.to_string(), proc);
    }
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
) -> Result<i32, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout fehlt".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr fehlt".to_string())?;
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
    let timeout = Duration::from_secs(state.cfg.job_timeout_secs);
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Err(format!("Prozess: {err}")),
        Err(_) => {
            log(
                state,
                job_id,
                "system",
                "Zeitlimit erreicht, Prozess wird beendet.",
            );
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
                if count > 8_000 {
                    if count == 8_001 {
                        log(
                            &state,
                            &job_id,
                            "system",
                            "Weitere Ausgabe wird verworfen (Grenze 8000 Zeilen).",
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
    }
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
}
