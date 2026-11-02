//! Checks the tools the configured runner mode needs.
//!
//! The probe never builds a shell command, never reads a value from the job
//! environment, and never prints `PATH`. The report carries binary paths and
//! versions only.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::config::RunnerMode;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const RUNTIME_TIMEOUT: Duration = Duration::from_secs(10);
pub const ANSIBLE_MINIMUM: &str = "2.14";
pub const TOFU_MINIMUM: &str = "1.6";

struct ToolSpec {
    id: &'static str,
    kind: &'static str,
    template_tool: &'static str,
    minimum: &'static str,
    args: &'static [&'static str],
}

const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "ansible-playbook",
        kind: "tool",
        template_tool: "ansible",
        minimum: ANSIBLE_MINIMUM,
        args: &["--version"],
    },
    ToolSpec {
        id: "tofu",
        kind: "tool",
        template_tool: "opentofu",
        minimum: TOFU_MINIMUM,
        args: &["version"],
    },
    ToolSpec {
        id: "sh",
        kind: "tool",
        template_tool: "shell",
        minimum: "",
        args: &["-c", "true"],
    },
    ToolSpec {
        id: "git",
        kind: "project",
        template_tool: "",
        minimum: "",
        args: &["--version"],
    },
    ToolSpec {
        id: "ssh",
        kind: "tool",
        template_tool: "ansible",
        minimum: "",
        args: &["-V"],
    },
];

#[derive(Clone, Debug, Serialize)]
pub struct ToolStatus {
    pub id: &'static str,
    pub kind: &'static str,
    pub template_tool: &'static str,
    pub minimum: &'static str,
    pub required: bool,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub meets_minimum: bool,
    pub reason: Option<&'static str>,
}

impl ToolStatus {
    fn ready(&self) -> bool {
        self.found && self.meets_minimum && self.reason.is_none()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeStatus {
    pub binary: Option<String>,
    pub found: bool,
    pub responding: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Missing {
    pub kind: &'static str,
    pub tool: &'static str,
    pub template_tool: &'static str,
    pub reason: &'static str,
    pub version: Option<String>,
    pub minimum: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub mode: &'static str,
    pub container_image: bool,
    pub tools: Vec<ToolStatus>,
    pub runtime: RuntimeStatus,
    pub missing: Vec<Missing>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.missing.is_empty()
    }

    pub fn messages(&self) -> Vec<String> {
        self.missing.iter().map(|item| self.message(item)).collect()
    }

    pub fn tool_ready(&self, template_tool: &str) -> bool {
        let mut found = false;
        for tool in &self.tools {
            if tool.kind != "tool" || tool.template_tool != template_tool {
                continue;
            }
            found = true;
            if !tool.ready() {
                return false;
            }
        }
        found
    }

    /// Why an automatic job of this template tool cannot start, in the same
    /// words as the overview notice.
    pub fn job_problem(&self, template_tool: &str) -> Option<String> {
        if self.mode != "auto" {
            return None;
        }
        self.missing
            .iter()
            .find(|item| item.kind == "tool" && item.template_tool == template_tool)
            .map(|item| self.message(item))
    }

    fn message(&self, item: &Missing) -> String {
        if item.kind == "runtime" {
            return self.runtime_message(item.reason);
        }
        if item.kind == "project" {
            let detail = match item.reason {
                "timeout" => format!("{} did not answer", item.tool),
                "failed" => format!("{} could not be checked", item.tool),
                _ => format!("{} is missing", item.tool),
            };
            return format!(
                "{detail}. Git projects cannot sync. Install {} on this machine.",
                install_name(item.tool, item.template_tool)
            );
        }
        let label = template_label(item.template_tool);
        let install = install_name(item.tool, item.template_tool);
        let detail = match item.reason {
            "version" => format!(
                "{} {} is older than {}",
                item.tool,
                item.version.as_deref().unwrap_or(""),
                item.minimum
            ),
            "timeout" => format!("{} did not answer", item.tool),
            "failed" => format!("{} could not be checked", item.tool),
            _ => format!("{} is missing", item.tool),
        };
        let no_runtime = self.missing.iter().any(|other| other.kind == "runtime");
        if no_runtime && self.mode == "auto" {
            format!(
                "No container service found and {detail}. {label} templates cannot run. Install {install} on this machine or start Docker or Podman and use an execution environment."
            )
        } else {
            format!("{detail}. {label} templates cannot run. Install {install} on this machine.")
        }
    }

    fn runtime_message(&self, reason: &str) -> String {
        if self.container_image {
            return match reason {
                "timeout" => "The container runtime did not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Mount a working Docker or Podman socket.".into(),
                "not_answering" => "The container runtime is installed but the daemon does not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Mount a working Docker or Podman socket.".into(),
                _ => "No container service found. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Mount the Docker socket and start Docker or Podman.".into(),
            };
        }
        match self.mode {
            "docker" => match reason {
                "timeout" => "The container runtime did not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or change the runner mode.".into(),
                "not_answering" => "The container runtime is installed but the daemon does not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or change the runner mode.".into(),
                _ => "No container service found. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or change the runner mode.".into(),
            },
            _ => match reason {
                "timeout" => "The container runtime did not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or install the local tools.".into(),
                "not_answering" => "The container runtime is installed but the daemon does not answer. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or install the local tools.".into(),
                _ => "No container service found. Ansible, OpenTofu, and Shell templates with an execution environment cannot run. Start Docker or Podman, or install the local tools.".into(),
            },
        }
    }

    pub fn log_block(&self) -> String {
        let mut lines = vec![format!("Runner preflight (mode {}):", self.mode)];
        for tool in &self.tools {
            if self.container_image {
                lines.push(format!(
                    "  {:<16} not checked in the container image",
                    tool.id
                ));
            } else {
                lines.push(format!("  {:<16} {}", tool.id, tool_summary(tool)));
            }
        }
        lines.push(format!(
            "  {:<16} {}",
            "container",
            runtime_summary(&self.runtime)
        ));
        if self.ok() {
            lines.push("Result: ready.".into());
        } else {
            for message in self.messages() {
                lines.push(format!("Result: {message}"));
            }
        }
        lines.join("\n")
    }

    pub fn json(&self) -> Value {
        json!({
            "mode": self.mode,
            "container_image": self.container_image,
            "ok": self.ok(),
            "tools": self.tools,
            "runtime": self.runtime,
            "missing": self.missing,
            "messages": self.messages(),
        })
    }
}

fn tool_summary(tool: &ToolStatus) -> String {
    if !tool.found {
        return "missing".into();
    }
    let path = tool.path.as_deref().unwrap_or("");
    match (tool.reason, tool.version.as_deref()) {
        (Some("timeout"), _) => format!("did not answer at {path}"),
        (Some("failed"), _) => format!("could not be checked at {path}"),
        (Some("version"), Some(version)) => {
            format!("{version} at {path} (needs {} or newer)", tool.minimum)
        }
        (_, Some(version)) => format!("{version} at {path}"),
        _ => format!("found at {path}"),
    }
}

fn runtime_summary(runtime: &RuntimeStatus) -> String {
    if !runtime.found {
        return "missing (docker or podman)".into();
    }
    let bin = runtime.binary.as_deref().unwrap_or("runtime");
    if runtime.responding {
        return match runtime.version.as_deref() {
            Some(version) => format!("{bin} answers, {version}"),
            None => format!("{bin} answers"),
        };
    }
    match runtime.reason {
        Some("timeout") => format!("{bin} found, did not answer"),
        _ => format!("{bin} found, daemon does not answer"),
    }
}

fn template_label(template_tool: &str) -> &'static str {
    match template_tool {
        "ansible" => "Ansible",
        "opentofu" => "OpenTofu",
        _ => "Shell",
    }
}

fn install_name(tool: &str, template_tool: &str) -> &'static str {
    match tool {
        "git" => "Git",
        "ssh" => "OpenSSH (ssh)",
        _ => match template_tool {
            "ansible" => "Ansible",
            "opentofu" => "OpenTofu",
            _ => "a POSIX shell (sh)",
        },
    }
}

pub async fn check(mode: RunnerMode, container_image: bool) -> Report {
    let paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();
    check_with_paths(mode, container_image, &paths, PROBE_TIMEOUT).await
}

async fn check_with_paths(
    mode: RunnerMode,
    container_image: bool,
    paths: &[PathBuf],
    timeout: Duration,
) -> Report {
    let mut tools = Vec::new();
    for spec in TOOLS {
        if container_image {
            tools.push(ToolStatus {
                id: spec.id,
                kind: spec.kind,
                template_tool: spec.template_tool,
                minimum: spec.minimum,
                required: false,
                found: false,
                path: None,
                version: None,
                meets_minimum: false,
                reason: Some("not_in_image"),
            });
        } else {
            tools.push(probe_tool(spec, paths, timeout).await);
        }
    }
    let runtime = probe_runtime(paths, RUNTIME_TIMEOUT).await;
    let tool_missing = tools
        .iter()
        .any(|tool| tool.kind == "tool" && !tool.ready());
    let runtime_required =
        container_image || mode == RunnerMode::Docker || (mode == RunnerMode::Auto && tool_missing);
    let tools_required = !container_image
        && (mode == RunnerMode::Local || (mode == RunnerMode::Auto && !runtime.responding));
    for tool in &mut tools {
        tool.required = if tool.kind == "project" {
            !container_image
        } else {
            tools_required
        };
    }
    let mut missing = Vec::new();
    if runtime_required && !runtime.responding {
        missing.push(Missing {
            kind: "runtime",
            tool: "docker/podman",
            template_tool: "",
            reason: runtime.reason.unwrap_or(if runtime.found {
                "not_answering"
            } else {
                "missing"
            }),
            version: runtime.version.clone(),
            minimum: "",
        });
    }
    for tool in &tools {
        if !tool.required || tool.ready() {
            continue;
        }
        missing.push(Missing {
            kind: tool.kind,
            tool: tool.id,
            template_tool: tool.template_tool,
            reason: tool
                .reason
                .unwrap_or(if tool.found { "failed" } else { "missing" }),
            version: tool.version.clone(),
            minimum: tool.minimum,
        });
    }
    Report {
        mode: mode.as_str(),
        container_image,
        tools,
        runtime,
        missing,
    }
}

async fn probe_tool(spec: &ToolSpec, paths: &[PathBuf], timeout: Duration) -> ToolStatus {
    let Some(path) = find_binary(paths, spec.id) else {
        return ToolStatus {
            id: spec.id,
            kind: spec.kind,
            template_tool: spec.template_tool,
            minimum: spec.minimum,
            required: false,
            found: false,
            path: None,
            version: None,
            meets_minimum: false,
            reason: Some("missing"),
        };
    };
    let shown = path.display().to_string();
    match probe_version(&path, spec.args, timeout).await {
        Probe::Ok(version) => {
            let meets = spec.minimum.is_empty() || version_meets(version.as_deref(), spec.minimum);
            ToolStatus {
                id: spec.id,
                kind: spec.kind,
                template_tool: spec.template_tool,
                minimum: spec.minimum,
                required: false,
                found: true,
                path: Some(shown),
                version,
                meets_minimum: meets,
                reason: if meets { None } else { Some("version") },
            }
        }
        Probe::Timeout => ToolStatus {
            id: spec.id,
            kind: spec.kind,
            template_tool: spec.template_tool,
            minimum: spec.minimum,
            required: false,
            found: true,
            path: Some(shown),
            version: None,
            meets_minimum: false,
            reason: Some("timeout"),
        },
        Probe::Failed => ToolStatus {
            id: spec.id,
            kind: spec.kind,
            template_tool: spec.template_tool,
            minimum: spec.minimum,
            required: false,
            found: true,
            path: Some(shown),
            version: None,
            meets_minimum: false,
            reason: Some("failed"),
        },
    }
}

async fn probe_runtime(paths: &[PathBuf], timeout: Duration) -> RuntimeStatus {
    let mut first: Option<(String, PathBuf)> = None;
    let mut failure = "missing";
    for bin in ["docker", "podman"] {
        let Some(path) = find_binary(paths, bin) else {
            continue;
        };
        if first.is_none() {
            first = Some((bin.to_string(), path.clone()));
        }
        match probe_version(&path, &["version"], timeout).await {
            Probe::Ok(version) => {
                return RuntimeStatus {
                    binary: Some(bin.to_string()),
                    found: true,
                    responding: true,
                    path: Some(path.display().to_string()),
                    version,
                    reason: None,
                };
            }
            Probe::Timeout => failure = "timeout",
            Probe::Failed => failure = "not_answering",
        }
    }
    match first {
        Some((bin, path)) => RuntimeStatus {
            binary: Some(bin),
            found: true,
            responding: false,
            path: Some(path.display().to_string()),
            version: None,
            reason: Some(failure),
        },
        None => RuntimeStatus {
            binary: None,
            found: false,
            responding: false,
            path: None,
            version: None,
            reason: Some("missing"),
        },
    }
}

enum Probe {
    Ok(Option<String>),
    Timeout,
    Failed,
}

async fn probe_version(path: &Path, args: &[&str], timeout: Duration) -> Probe {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return Probe::Failed,
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push(' ');
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            Probe::Ok(parse_version(&text))
        }
        Ok(_) => Probe::Failed,
        Err(_) => Probe::Timeout,
    }
}

fn parse_version(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let mut run = String::new();
        for ch in token.chars() {
            if ch.is_ascii_digit() || ch == '.' {
                run.push(ch);
            } else if let Some(version) = version_run(&run) {
                return Some(version);
            } else {
                run.clear();
            }
        }
        if let Some(version) = version_run(&run) {
            return Some(version);
        }
    }
    None
}

fn version_run(run: &str) -> Option<String> {
    let candidate = run.trim_matches('.');
    if candidate.is_empty() || !candidate.contains('.') {
        return None;
    }
    Some(candidate.to_string())
}

fn version_meets(version: Option<&str>, minimum: &str) -> bool {
    let Some(version) = version else {
        return false;
    };
    let parts = |value: &str| -> Vec<u64> {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let mut version = parts(version);
    let mut minimum = parts(minimum);
    let length = version.len().max(minimum.len());
    version.resize(length, 0);
    minimum.resize(length, 0);
    version >= minimum
}

fn find_binary(paths: &[PathBuf], bin: &str) -> Option<PathBuf> {
    paths
        .iter()
        .map(|dir| dir.join(bin))
        .find(|path| executable(path))
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("koreograf-preflight-{name}-{nanos}"));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write_tool(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).expect("write tool");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("mode");
        }
    }

    fn ready_tools(dir: &Path) {
        write_tool(
            dir,
            "ansible-playbook",
            "#!/bin/sh\necho 'ansible-playbook [core 2.17.2]'\n",
        );
        write_tool(dir, "tofu", "#!/bin/sh\necho 'OpenTofu v1.8.0'\n");
        write_tool(dir, "sh", "#!/bin/sh\nexit 0\n");
        write_tool(dir, "git", "#!/bin/sh\necho 'git version 2.43.0'\n");
        write_tool(
            dir,
            "ssh",
            "#!/bin/sh\necho 'OpenSSH_9.6p1 Ubuntu-3ubuntu13.5' >&2\n",
        );
        write_tool(
            dir,
            "docker",
            "#!/bin/sh\necho 'Docker version 27.0.0, build test'\n",
        );
    }

    fn short() -> Duration {
        Duration::from_millis(300)
    }

    fn blocking(report: &Report) -> Vec<&str> {
        report.missing.iter().map(|item| item.tool).collect()
    }

    #[tokio::test]
    async fn every_local_tool_is_found_with_a_version() {
        let dir = temp_dir("ready");
        ready_tools(&dir);
        let report = check_with_paths(
            RunnerMode::Local,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        assert!(report.ok(), "{:?}", report.missing);
        let ansible = report
            .tools
            .iter()
            .find(|tool| tool.id == "ansible-playbook")
            .expect("ansible");
        assert!(ansible.found);
        assert_eq!(ansible.version.as_deref(), Some("2.17.2"));
        assert_eq!(
            ansible.path.as_deref(),
            Some(dir.join("ansible-playbook").to_str().unwrap())
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn git_and_ssh_are_found_with_a_path_and_a_version() {
        let dir = temp_dir("git-ssh");
        ready_tools(&dir);
        let report = check_with_paths(
            RunnerMode::Local,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        let git = report
            .tools
            .iter()
            .find(|tool| tool.id == "git")
            .expect("git");
        assert!(git.found);
        assert_eq!(git.version.as_deref(), Some("2.43.0"));
        assert_eq!(git.path.as_deref(), Some(dir.join("git").to_str().unwrap()));
        let ssh = report
            .tools
            .iter()
            .find(|tool| tool.id == "ssh")
            .expect("ssh");
        assert!(ssh.found);
        assert_eq!(ssh.version.as_deref(), Some("9.6"));
        assert_eq!(ssh.path.as_deref(), Some(dir.join("ssh").to_str().unwrap()));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_missing_git_is_a_project_problem_not_a_template_problem() {
        let dir = temp_dir("no-git");
        ready_tools(&dir);
        fs::remove_file(dir.join("git")).expect("remove git");
        fs::remove_file(dir.join("docker")).expect("remove docker");
        let report =
            check_with_paths(RunnerMode::Auto, false, std::slice::from_ref(&dir), short()).await;
        assert!(!report.ok());
        let item = report
            .missing
            .iter()
            .find(|item| item.tool == "git")
            .expect("git missing");
        assert_eq!(item.kind, "project");
        assert_eq!(item.template_tool, "");
        assert!(report.messages().iter().any(
            |line| line.contains("git is missing") && line.contains("Git projects cannot sync")
        ));
        assert!(report.job_problem("ansible").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_missing_ssh_is_reported_for_ansible() {
        let dir = temp_dir("no-ssh");
        ready_tools(&dir);
        fs::remove_file(dir.join("ssh")).expect("remove ssh");
        fs::remove_file(dir.join("docker")).expect("remove docker");
        let report =
            check_with_paths(RunnerMode::Auto, false, std::slice::from_ref(&dir), short()).await;
        assert!(!report.ok());
        let item = report
            .missing
            .iter()
            .find(|item| item.tool == "ssh")
            .expect("ssh missing");
        assert_eq!(item.kind, "tool");
        assert_eq!(item.template_tool, "ansible");
        let problem = report.job_problem("ansible").expect("ansible problem");
        assert!(problem.contains("ssh is missing"));
        assert!(problem.contains("OpenSSH"));
        assert!(report.job_problem("shell").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_missing_tool_is_named_for_local_mode() {
        let dir = temp_dir("missing");
        write_tool(
            &dir,
            "ansible-playbook",
            "#!/bin/sh\necho 'ansible-playbook [core 2.17.2]'\n",
        );
        write_tool(
            &dir,
            "docker",
            "#!/bin/sh\necho 'Docker version 27.0.0, build test'\n",
        );
        let report = check_with_paths(
            RunnerMode::Local,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        assert!(!report.ok());
        assert_eq!(blocking(&report), vec!["tofu", "sh", "git", "ssh"]);
        assert!(report.messages()[0].contains("tofu is missing"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn auto_without_a_runtime_blocks_a_tool_that_is_missing() {
        let dir = temp_dir("auto");
        write_tool(&dir, "sh", "#!/bin/sh\nexit 0\n");
        let report =
            check_with_paths(RunnerMode::Auto, false, std::slice::from_ref(&dir), short()).await;
        assert_eq!(
            blocking(&report),
            vec!["docker/podman", "ansible-playbook", "tofu", "git", "ssh"]
        );
        let messages = report.messages();
        assert!(messages.iter().any(
            |line| line.contains("No container service found and ansible-playbook is missing")
        ));
        assert!(messages
            .iter()
            .any(|line| line.contains("Ansible templates cannot run")));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn auto_with_a_runtime_does_not_need_the_local_tools() {
        let dir = temp_dir("auto-runtime");
        write_tool(
            &dir,
            "docker",
            "#!/bin/sh\necho 'Docker version 27.0.0, build test'\n",
        );
        write_tool(&dir, "git", "#!/bin/sh\necho 'git version 2.43.0'\n");
        let report =
            check_with_paths(RunnerMode::Auto, false, std::slice::from_ref(&dir), short()).await;
        assert!(report.ok(), "{:?}", report.missing);
        assert!(report.runtime.responding);
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_old_version_is_reported_with_the_minimum() {
        let dir = temp_dir("old");
        write_tool(
            &dir,
            "ansible-playbook",
            "#!/bin/sh\necho 'ansible-playbook [core 2.9.0]'\n",
        );
        write_tool(&dir, "tofu", "#!/bin/sh\necho 'OpenTofu v1.8.0'\n");
        write_tool(&dir, "sh", "#!/bin/sh\nexit 0\n");
        let report = check_with_paths(
            RunnerMode::Local,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        let ansible = report
            .tools
            .iter()
            .find(|tool| tool.id == "ansible-playbook")
            .expect("ansible");
        assert_eq!(ansible.reason, Some("version"));
        assert_eq!(ansible.version.as_deref(), Some("2.9.0"));
        assert!(report.messages()[0].contains("older than 2.14"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_hanging_tool_times_out() {
        let dir = temp_dir("hang");
        write_tool(&dir, "ansible-playbook", "#!/bin/sh\nsleep 5\n");
        let report = check_with_paths(
            RunnerMode::Local,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        let ansible = report
            .tools
            .iter()
            .find(|tool| tool.id == "ansible-playbook")
            .expect("ansible");
        assert_eq!(ansible.reason, Some("timeout"));
        assert!(report.messages()[0].contains("did not answer"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_runtime_binary_without_an_answering_socket_is_not_ready() {
        let dir = temp_dir("socket");
        ready_tools(&dir);
        write_tool(
            &dir,
            "docker",
            "#!/bin/sh\necho 'Cannot connect to the Docker daemon' >&2\nexit 1\n",
        );
        let report = check_with_paths(
            RunnerMode::Docker,
            false,
            std::slice::from_ref(&dir),
            short(),
        )
        .await;
        assert!(report.runtime.found);
        assert!(!report.runtime.responding);
        assert_eq!(report.runtime.binary.as_deref(), Some("docker"));
        assert_eq!(blocking(&report), vec!["docker/podman"]);
        assert!(report.messages()[0].contains("does not answer"));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_container_image_checks_only_the_runtime() {
        let dir = temp_dir("image");
        ready_tools(&dir);
        let report =
            check_with_paths(RunnerMode::Auto, true, std::slice::from_ref(&dir), short()).await;
        assert!(report.ok(), "{:?}", report.missing);
        assert!(report.container_image);
        assert!(report.tools.iter().all(|tool| !tool.required));
        assert!(report.tools.iter().all(|tool| !tool.found));
        assert!(report
            .log_block()
            .contains("not checked in the container image"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_version_check_reads_the_first_dotted_number() {
        assert_eq!(
            parse_version("ansible-playbook [core 2.17.2]"),
            Some("2.17.2".into())
        );
        assert_eq!(parse_version("OpenTofu v1.8.0"), Some("1.8.0".into()));
        assert_eq!(
            parse_version("OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"),
            Some("9.6".into())
        );
        assert_eq!(parse_version("git version 2.43.0"), Some("2.43.0".into()));
        assert_eq!(parse_version("no version here"), None);
        assert!(version_meets(Some("2.17.2"), "2.14"));
        assert!(version_meets(Some("1.6"), "1.6"));
        assert!(!version_meets(Some("1.5.9"), "1.6"));
        assert!(!version_meets(None, "1.6"));
    }
}
