use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerMode {
    Auto,
    Docker,
    Local,
}

/// Who may start a job on the controller. The default is administrators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalJobs {
    Admins,
    All,
}

impl LocalJobs {
    pub fn parse(value: &str) -> Self {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("admins") {
            Self::Admins
        } else if value.eq_ignore_ascii_case("all") {
            Self::All
        } else {
            let shown: String = value
                .chars()
                .filter(|ch| !ch.is_control())
                .take(64)
                .collect();
            tracing::warn!(
                value = %shown,
                "KOREOGRAF_LOCAL_RUNNER_FOR is not all or admins; local jobs stay limited to administrators"
            );
            Self::Admins
        }
    }
}

impl RunnerMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "docker" | "podman" | "container" => Self::Docker,
            "local" | "lokal" => Self::Local,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Docker => "docker",
            Self::Local => "local",
        }
    }
}

/// `KOREOGRAF_DATABASE_URL` is empty or `sqlite:` today.
/// A Postgres URL fails here until that engine exists.
pub fn database_choice(url: &str) -> Result<(), &'static str> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if url.is_empty() || lower == "sqlite" || lower.starts_with("sqlite:") {
        Ok(())
    } else if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        Err(POSTGRES_LATER)
    } else {
        Err("KOREOGRAF_DATABASE_URL is not supported. Leave it empty to use SQLite.")
    }
}

const POSTGRES_LATER: &str = "PostgreSQL is not connected yet. Koreograf uses SQLite. A backup from the admin UI contains the database and the key for a later move.";

#[derive(Clone)]
pub struct LdapConfig {
    pub url: String,
    pub mode: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_base: String,
    pub user_filter: String,
    pub username_attr: String,
    pub group_attr: String,
    pub group_base: String,
    pub starttls: bool,
    pub tls_insecure: bool,
    pub ca_file: String,
}

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    pub web_dir: PathBuf,
    pub examples_dir: PathBuf,
    pub admin_user: String,
    /// Consumed during the first boot. Cleared afterwards.
    pub admin_password: Option<String>,
    pub runner: RunnerMode,
    pub job_timeout_secs: u64,
    /// `KOREOGRAF_LOCAL_RUNNER_FOR`. Not a saved setting.
    pub local_jobs: LocalJobs,
    /// Allowed path-project directories. `from_env` always includes `examples_dir`.
    pub project_roots: Vec<PathBuf>,
    pub ldap: Option<LdapConfig>,
    /// `KOREOGRAF_HSTS`. Off unless the process is reached over HTTPS.
    pub hsts: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let ldap_url = env_string("KOREOGRAF_LDAP_URL");
        let mode = env_string("KOREOGRAF_LDAP_MODE");
        let mode = if mode.is_empty() { "ldap".into() } else { mode };
        let ad = mode.eq_ignore_ascii_case("ad");
        let ldap = if ldap_url.is_empty() {
            None
        } else {
            Some(LdapConfig {
                url: ldap_url,
                mode: if ad { "ad".into() } else { "ldap".into() },
                bind_dn: env_string("KOREOGRAF_LDAP_BIND_DN"),
                bind_password: env_string("KOREOGRAF_LDAP_BIND_PASSWORD"),
                user_base: env_string("KOREOGRAF_LDAP_USER_BASE"),
                user_filter: env_or(
                    "KOREOGRAF_LDAP_USER_FILTER",
                    if ad {
                        "(sAMAccountName={username})"
                    } else {
                        "(uid={username})"
                    },
                ),
                username_attr: env_or(
                    "KOREOGRAF_LDAP_USERNAME_ATTR",
                    if ad { "sAMAccountName" } else { "uid" },
                ),
                group_attr: env_or("KOREOGRAF_LDAP_GROUP_ATTR", "memberOf"),
                group_base: env_string("KOREOGRAF_LDAP_GROUP_BASE"),
                starttls: env_flag("KOREOGRAF_LDAP_STARTTLS"),
                tls_insecure: env_flag("KOREOGRAF_LDAP_TLS_INSECURE"),
                ca_file: env_string("KOREOGRAF_LDAP_CA_FILE"),
            })
        };

        let password = env_string("KOREOGRAF_ADMIN_PASSWORD");
        let examples_dir = PathBuf::from(env_or("KOREOGRAF_EXAMPLES", "./examples"));
        let project_roots =
            effective_project_roots(env_paths("KOREOGRAF_PROJECT_ROOTS"), &examples_dir);
        Self {
            bind: env_or("KOREOGRAF_BIND", "0.0.0.0:8080"),
            data_dir: PathBuf::from(env_or("KOREOGRAF_DATA", "./data")),
            web_dir: PathBuf::from(env_or("KOREOGRAF_WEB", "./web")),
            examples_dir,
            admin_user: env_or("KOREOGRAF_ADMIN_USER", "admin").to_ascii_lowercase(),
            admin_password: if password.is_empty() {
                None
            } else {
                Some(password)
            },
            runner: RunnerMode::parse(&env_or("KOREOGRAF_RUNNER", "auto")),
            job_timeout_secs: env_or("KOREOGRAF_JOB_TIMEOUT_SECS", "1800")
                .parse()
                .unwrap_or(1800)
                .clamp(30, 86_400),
            local_jobs: LocalJobs::parse(&env_string("KOREOGRAF_LOCAL_RUNNER_FOR")),
            project_roots,
            ldap,
            hsts: env_flag("KOREOGRAF_HSTS"),
        }
    }
}

pub(crate) fn effective_project_roots(
    configured: Vec<PathBuf>,
    examples_dir: &Path,
) -> Vec<PathBuf> {
    let mut roots = configured;
    if roots.iter().any(|root| root == examples_dir) {
        return roots;
    }
    roots.push(examples_dir.to_path_buf());
    roots
}

fn env_paths(key: &str) -> Vec<PathBuf> {
    env_string(key)
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

fn env_or(key: &str, default: &str) -> String {
    let value = env_string(key);
    if value.is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn env_flag(key: &str) -> bool {
    matches!(
        env_string(key).to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "ja"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("log lock").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn an_unknown_local_runner_value_warns_and_stays_with_admins() {
        assert_eq!(LocalJobs::parse(""), LocalJobs::Admins);
        assert_eq!(LocalJobs::parse(" admins "), LocalJobs::Admins);
        assert_eq!(LocalJobs::parse(" ALL "), LocalJobs::All);
        let buf = Buf(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(buf.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(LocalJobs::parse("al"), LocalJobs::Admins);
            assert_eq!(LocalJobs::parse(""), LocalJobs::Admins);
        });
        let text = String::from_utf8(buf.0.lock().expect("log lock").clone()).expect("utf-8");
        assert!(text.contains("KOREOGRAF_LOCAL_RUNNER_FOR"), "{text}");
        assert!(text.contains("value=al"), "{text}");
        assert_eq!(
            text.matches("KOREOGRAF_LOCAL_RUNNER_FOR").count(),
            1,
            "{text}"
        );
    }

    #[test]
    fn the_examples_directory_is_always_a_project_root() {
        let examples = PathBuf::from("examples");
        let extra = PathBuf::from("/srv/projects");
        assert_eq!(
            effective_project_roots(Vec::new(), &examples),
            vec![examples.clone()]
        );
        assert_eq!(
            effective_project_roots(vec![extra.clone()], &examples),
            vec![extra, examples.clone()]
        );
        assert_eq!(
            effective_project_roots(vec![examples.clone()], &examples),
            vec![examples]
        );
    }
}
