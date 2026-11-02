use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskpassKind {
    Git,
    Ssh,
}

pub fn askpass_kind<I, S>(args: I) -> Option<AskpassKind>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let first = args.into_iter().next()?;
    let name = Path::new(first.as_ref())
        .file_name()
        .and_then(|value| value.to_str())?;
    match name {
        "git-askpass" => Some(AskpassKind::Git),
        "ssh-askpass" => Some(AskpassKind::Ssh),
        _ => None,
    }
}

pub fn git_ssh_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .next()
        .and_then(|arg| {
            Path::new(arg.as_ref())
                .file_name()
                .and_then(|value| value.to_str())
                .map(|name| name == "git-ssh")
        })
        .unwrap_or(false)
}

pub fn git_ssh<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let key = std::env::var("KOREOGRAF_GIT_SSH_KEY")
        .map_err(|_| "KOREOGRAF_GIT_SSH_KEY is not set for the git ssh helper.".to_string())?;
    let mut command = std::process::Command::new("/usr/bin/ssh");
    command
        .arg("-i")
        .arg(key)
        .arg("-o")
        .arg("IdentitiesOnly=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        Err(format!("The git ssh helper could not start ssh ({err})."))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|err| format!("The git ssh helper could not start ssh ({err})."))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("The git ssh helper exited with {status}."))
        }
    }
}

pub fn askpass(kind: AskpassKind, prompt: &str) -> Result<String, String> {
    askpass_answer(kind, prompt, |name| std::env::var(name).ok())
}

fn askpass_answer(
    kind: AskpassKind,
    prompt: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Result<String, String> {
    match kind {
        AskpassKind::Git => {
            if prompt.to_ascii_lowercase().contains("sername") {
                return Ok(env("KOREOGRAF_GIT_USERNAME").unwrap_or_else(|| "oauth2".into()));
            }
            read_secret(env, "KOREOGRAF_GIT_PASSWORD_FILE")
        }
        AskpassKind::Ssh => read_secret(env, "KOREOGRAF_SSH_ASKPASS_FILE"),
    }
}

fn read_secret(env: impl Fn(&str) -> Option<String>, name: &str) -> Result<String, String> {
    let path = env(name).ok_or_else(|| format!("{name} is not set for the credential helper."))?;
    let value = std::fs::read_to_string(&path)
        .map_err(|err| format!("The credential helper could not read its secret ({err})."))?;
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_helper_names_get_the_askpass_path() {
        assert_eq!(
            askpass_kind(["/tmp/git-askpass", "Username:"]),
            Some(AskpassKind::Git)
        );
        assert_eq!(
            askpass_kind(["/tmp/ssh-askpass", "Passphrase:"]),
            Some(AskpassKind::Ssh)
        );
        assert_eq!(
            askpass_kind(["/usr/local/bin/koreograf", "entrypoint"]),
            None
        );
    }

    #[test]
    fn only_the_git_ssh_name_starts_the_ssh_helper() {
        assert!(git_ssh_requested(["/tmp/git-ssh"]));
        assert!(git_ssh_requested([
            "/usr/local/bin/git-ssh",
            "git@example.com"
        ]));
        assert!(!git_ssh_requested(["/tmp/git-askpass"]));
        assert!(!git_ssh_requested(["/usr/local/bin/koreograf"]));
        assert!(!git_ssh_requested(Vec::<String>::new()));
    }

    #[test]
    fn the_git_helper_reads_the_username_from_the_environment() {
        let answer = askpass_answer(
            AskpassKind::Git,
            "Username for 'https://example.com':",
            |_| None,
        )
        .unwrap();
        assert_eq!(answer, "oauth2");
        let answer = askpass_answer(
            AskpassKind::Git,
            "Username for 'https://example.com':",
            |name| (name == "KOREOGRAF_GIT_USERNAME").then(|| "bot".to_string()),
        )
        .unwrap();
        assert_eq!(answer, "bot");
    }

    #[test]
    fn the_git_helper_reads_the_password_file() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("koreograf-askpass-{nanos}"));
        std::fs::write(&path, "secret\n").unwrap();
        let shown = path.display().to_string();
        let answer = askpass_answer(
            AskpassKind::Git,
            "Password for 'https://example.com':",
            |name| (name == "KOREOGRAF_GIT_PASSWORD_FILE").then(|| shown.clone()),
        )
        .unwrap();
        assert_eq!(answer, "secret");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_file_is_an_error_without_a_secret() {
        let err = askpass_answer(AskpassKind::Ssh, "Passphrase:", |_| None).unwrap_err();
        assert!(err.contains("KOREOGRAF_SSH_ASKPASS_FILE"));
    }
}
