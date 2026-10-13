use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerMode {
    Auto,
    Docker,
    Local,
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
        Err("KOREOGRAF_DATABASE_URL wird nicht unterstützt. Leer lassen für SQLite.")
    }
}

const POSTGRES_LATER: &str = "PostgreSQL ist noch nicht angebunden. Koreograf nutzt SQLite. Eine Sicherung aus der Admin-Oberfläche enthält die Datenbank und den Schlüssel für einen späteren Umzug.";

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
    pub ldap: Option<LdapConfig>,
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
            })
        };

        let password = env_string("KOREOGRAF_ADMIN_PASSWORD");
        Self {
            bind: env_or("KOREOGRAF_BIND", "0.0.0.0:8080"),
            data_dir: PathBuf::from(env_or("KOREOGRAF_DATA", "./data")),
            web_dir: PathBuf::from(env_or("KOREOGRAF_WEB", "./web")),
            examples_dir: PathBuf::from(env_or("KOREOGRAF_EXAMPLES", "./examples")),
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
            ldap,
        }
    }
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
