use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

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

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admins => "admins",
            Self::All => "all",
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

    /// The image has no local tool set, so a saved `local` mode runs in a
    /// container there. Outside the image every mode stays as configured.
    pub fn in_container(self, container_image: bool) -> Self {
        if container_image && self == Self::Local {
            Self::Docker
        } else {
            self
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "disable" => Some(Self::Disable),
            "allow" | "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            "verify-ca" => Some(Self::VerifyCa),
            "verify-full" => Some(Self::VerifyFull),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PostgresConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub sslmode: SslMode,
    pub ca_file: Option<PathBuf>,
    pub connect_timeout_secs: u64,
    password: String,
}

impl PostgresConfig {
    pub(crate) fn password(&self) -> &str {
        &self.password
    }

    pub fn target(&self) -> String {
        format!("{}:{}/{}", self.host, self.port, self.database)
    }
}

impl std::fmt::Debug for PostgresConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("sslmode", &self.sslmode)
            .field("ca_file", &self.ca_file)
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatabaseConfig {
    Sqlite,
    Postgres(PostgresConfig),
}

impl DatabaseConfig {
    pub fn engine(&self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres(_) => "postgresql",
        }
    }

    pub fn target(&self) -> String {
        match self {
            Self::Sqlite => "sqlite".to_string(),
            Self::Postgres(config) => config.target(),
        }
    }
}

pub fn database_from_env() -> Result<DatabaseConfig, String> {
    let url = env_string("KOREOGRAF_DATABASE_URL");
    let password_file = env_string("KOREOGRAF_DATABASE_PASSWORD_FILE");
    let ca_file = env_string("KOREOGRAF_DATABASE_CA_FILE");
    parse_database_config(
        &url,
        non_empty_path(&password_file),
        non_empty_path(&ca_file),
    )
}

pub fn parse_database_config(
    url: &str,
    password_file: Option<PathBuf>,
    ca_file: Option<PathBuf>,
) -> Result<DatabaseConfig, String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if url.is_empty() || lower == "sqlite" || lower.starts_with("sqlite:") {
        return Ok(DatabaseConfig::Sqlite);
    }
    if !lower.starts_with("postgres://") && !lower.starts_with("postgresql://") {
        return Err(
            "KOREOGRAF_DATABASE_URL must stay empty, start with sqlite:, or name a postgres:// or postgresql:// database."
                .to_string(),
        );
    }
    let parsed = url::Url::parse(url)
        .map_err(|_| "KOREOGRAF_DATABASE_URL is not a valid database URL.".to_string())?;
    let host = parsed
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "KOREOGRAF_DATABASE_URL must name a database host.".to_string())?
        .to_string();
    let port = parsed.port().unwrap_or(5432);
    let user = percent_decode_str(parsed.username())
        .decode_utf8()
        .map_err(|_| "KOREOGRAF_DATABASE_URL has an invalid database user.".to_string())?
        .trim()
        .to_string();
    if user.is_empty() {
        return Err("KOREOGRAF_DATABASE_URL must name a database user.".to_string());
    }
    let database = percent_decode_str(parsed.path().trim_start_matches('/'))
        .decode_utf8()
        .map_err(|_| "KOREOGRAF_DATABASE_URL has an invalid database name.".to_string())?
        .trim()
        .to_string();
    let database = if database.is_empty() {
        user.clone()
    } else {
        database
    };
    let mut sslmode = SslMode::Require;
    let mut connect_timeout_secs = 10;
    let mut root_cert = ca_file;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "sslmode" => {
                sslmode = SslMode::parse(&value)
                    .ok_or_else(|| "KOREOGRAF_DATABASE_URL has an unknown sslmode.".to_string())?;
            }
            "sslrootcert" => {
                if !value.trim().is_empty() {
                    root_cert = Some(PathBuf::from(value.trim()));
                }
            }
            "connect_timeout" => {
                connect_timeout_secs = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| {
                        "KOREOGRAF_DATABASE_URL has an invalid connect_timeout.".to_string()
                    })?;
            }
            _ => {}
        }
    }
    let password = match password_file {
        Some(path) => read_password_file(&path)?,
        None => parsed
            .password()
            .map(|value| percent_decode_str(value).decode_utf8_lossy().to_string())
            .unwrap_or_default(),
    };
    Ok(DatabaseConfig::Postgres(PostgresConfig {
        host,
        port,
        database,
        user,
        sslmode,
        ca_file: root_cert,
        connect_timeout_secs,
        password,
    }))
}

fn non_empty_path(value: &str) -> Option<PathBuf> {
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn read_password_file(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|_| "KOREOGRAF_DATABASE_PASSWORD_FILE cannot be read.".to_string())?;
    let password = raw.trim_end_matches(['\r', '\n']).to_string();
    if password.is_empty() {
        return Err("KOREOGRAF_DATABASE_PASSWORD_FILE is empty.".to_string());
    }
    Ok(password)
}

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

/// When the session cookie carries `Secure`.
///
/// `auto` sets it only when `trusted_proxy` is on and the proxy sends
/// `X-Forwarded-Proto: https`. The process itself speaks HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieSecure {
    True,
    False,
    Auto,
}

impl CookieSecure {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Self::Auto,
            "1" | "true" | "yes" | "ja" => Self::True,
            "0" | "false" | "no" | "nein" => Self::False,
            _ => {
                let shown: String = value
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(64)
                    .collect();
                tracing::warn!(
                    value = %shown,
                    "KOREOGRAF_COOKIE_SECURE is not true, false, or auto; using auto"
                );
                Self::Auto
            }
        }
    }

    pub fn enabled(self, trust_proxy: bool, forwarded_proto: Option<&str>) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Auto => {
                trust_proxy
                    && forwarded_proto
                        .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
            }
        }
    }
}

/// Default lines kept for each of stdout and stderr.
pub const OUTPUT_LINES_DEFAULT: u64 = 8_000;
/// Lowest accepted output line limit.
pub const OUTPUT_LINES_MIN: u64 = 100;
/// Highest accepted output line limit.
pub const OUTPUT_LINES_MAX: u64 = 100_000;

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub data_dir: PathBuf,
    /// `KOREOGRAF_DATA_HOST`. Absolute path the container daemon uses for
    /// `data_dir`. Empty detects that path from this container's mounts.
    pub data_host: Option<PathBuf>,
    pub web_dir: PathBuf,
    pub examples_dir: PathBuf,
    pub admin_user: String,
    /// Consumed during the first boot. Cleared afterwards.
    pub admin_password: Option<String>,
    pub runner: RunnerMode,
    /// Set by the image build. The image has no local tool set of its own.
    pub container_image: bool,
    pub job_timeout_secs: u64,
    /// `KOREOGRAF_JOB_OUTPUT_LINES`. Lines kept for each of stdout and stderr.
    pub job_output_lines: u64,
    /// `KOREOGRAF_LOCAL_RUNNER_FOR`. Not a saved setting.
    pub local_jobs: LocalJobs,
    /// Allowed path-project directories. `from_env` always includes `examples_dir`.
    pub project_roots: Vec<PathBuf>,
    pub ldap: Option<LdapConfig>,
    /// `KOREOGRAF_HSTS`. Off unless the process is reached over HTTPS.
    pub hsts: bool,
    /// `KOREOGRAF_COOKIE_SECURE`. Default `auto`.
    pub cookie_secure: CookieSecure,
    /// `KOREOGRAF_TRUSTED_PROXY`. Off ignores `X-Forwarded-Proto`.
    pub trusted_proxy: bool,
    /// `KOREOGRAF_MCP`. `0` turns the MCP routes off.
    pub mcp: bool,
    /// `KOREOGRAF_PUBLIC_URL`. Public origins (scheme, host, port) that
    /// browser writes may come from. Empty falls back to the request host.
    pub public_url: Vec<String>,
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
        let examples_dir = PathBuf::from(env_or("KOREOGRAF_EXAMPLES", "./examples"));
        let project_roots =
            effective_project_roots(env_paths("KOREOGRAF_PROJECT_ROOTS"), &examples_dir);
        Self {
            bind: env_or("KOREOGRAF_BIND", "0.0.0.0:8080"),
            data_dir: PathBuf::from(env_or("KOREOGRAF_DATA", "./data")),
            data_host: absolute_env_path("KOREOGRAF_DATA_HOST"),
            web_dir: PathBuf::from(env_or("KOREOGRAF_WEB", "./web")),
            examples_dir,
            admin_user: env_or("KOREOGRAF_ADMIN_USER", "admin").to_ascii_lowercase(),
            admin_password: if password.is_empty() {
                None
            } else {
                Some(password)
            },
            runner: RunnerMode::parse(&env_or("KOREOGRAF_RUNNER", "auto")),
            container_image: env_flag("KOREOGRAF_CONTAINER_IMAGE"),
            job_timeout_secs: env_or("KOREOGRAF_JOB_TIMEOUT_SECS", "1800")
                .parse()
                .unwrap_or(1800)
                .clamp(30, 86_400),
            job_output_lines: env_or("KOREOGRAF_JOB_OUTPUT_LINES", "8000")
                .parse()
                .unwrap_or(OUTPUT_LINES_DEFAULT)
                .clamp(OUTPUT_LINES_MIN, OUTPUT_LINES_MAX),
            local_jobs: LocalJobs::parse(&env_string("KOREOGRAF_LOCAL_RUNNER_FOR")),
            project_roots,
            ldap,
            hsts: env_flag("KOREOGRAF_HSTS"),
            cookie_secure: CookieSecure::parse(&env_string("KOREOGRAF_COOKIE_SECURE")),
            trusted_proxy: env_flag("KOREOGRAF_TRUSTED_PROXY"),
            mcp: env_string("KOREOGRAF_MCP") != "0",
            public_url: env_list("KOREOGRAF_PUBLIC_URL"),
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

fn env_list(key: &str) -> Vec<String> {
    env_string(key)
        .split([',', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn env_string(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

/// An empty value means the path is detected. A relative path is ignored.
fn absolute_env_path(key: &str) -> Option<PathBuf> {
    let value = env_string(key);
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(&value);
    if path.is_absolute() {
        return Some(path);
    }
    tracing::warn!("{key} must be an absolute path; the container daemon path is detected instead");
    None
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

    #[test]
    fn cookie_secure_auto_follows_only_a_trusted_https_proxy() {
        assert_eq!(CookieSecure::parse(""), CookieSecure::Auto);
        assert_eq!(CookieSecure::parse("auto"), CookieSecure::Auto);
        assert_eq!(CookieSecure::parse("true"), CookieSecure::True);
        assert_eq!(CookieSecure::parse("FALSE"), CookieSecure::False);
        assert!(CookieSecure::True.enabled(false, None));
        assert!(!CookieSecure::False.enabled(true, Some("https")));
        assert!(!CookieSecure::Auto.enabled(false, Some("https")));
        assert!(!CookieSecure::Auto.enabled(true, Some("http")));
        assert!(!CookieSecure::Auto.enabled(true, None));
        assert!(!CookieSecure::Auto.enabled(true, Some("https,http")));
        assert!(CookieSecure::Auto.enabled(true, Some(" HTTPS ")));
    }

    #[test]
    fn an_empty_or_sqlite_url_stays_on_sqlite() {
        assert_eq!(
            parse_database_config("", None, None),
            Ok(DatabaseConfig::Sqlite)
        );
        assert_eq!(
            parse_database_config(" sqlite:data/koreograf.db ", None, None),
            Ok(DatabaseConfig::Sqlite)
        );
        assert!(parse_database_config("mysql://db/koreograf", None, None).is_err());
    }

    #[test]
    fn a_postgres_url_is_parsed_without_exposing_the_password() {
        let database = parse_database_config(
            "postgresql://koreo:pa%24s@db.example:5433/automation?sslmode=verify-full&connect_timeout=7",
            None,
            None,
        )
        .expect("postgres");
        let DatabaseConfig::Postgres(config) = &database else {
            panic!("expected postgres");
        };
        assert_eq!(config.host, "db.example");
        assert_eq!(config.port, 5433);
        assert_eq!(config.database, "automation");
        assert_eq!(config.user, "koreo");
        assert_eq!(config.password(), "pa$s");
        assert_eq!(config.sslmode, SslMode::VerifyFull);
        assert_eq!(config.connect_timeout_secs, 7);
        let debug = format!("{database:?}");
        assert!(!debug.contains("pa$s"), "{debug}");
        assert!(!debug.contains("pa%24s"), "{debug}");
        assert_eq!(database.engine(), "postgresql");
    }

    #[test]
    fn the_password_file_wins_and_the_value_is_not_debugged() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("koreograf-db-password-{nanos}"));
        std::fs::write(&path, "file-secret\n").expect("write");
        let database = parse_database_config(
            "postgres://koreo:url-secret@db/automation",
            Some(path.clone()),
            None,
        )
        .expect("postgres");
        let DatabaseConfig::Postgres(config) = &database else {
            panic!("expected postgres");
        };
        assert_eq!(config.password(), "file-secret");
        let debug = format!("{config:?}");
        assert!(!debug.contains("file-secret"), "{debug}");
        assert!(!debug.contains("url-secret"), "{debug}");
        let _ = std::fs::remove_file(&path);
        assert!(parse_database_config("postgres://koreo@db/automation", Some(path), None).is_err());
    }

    #[test]
    fn the_default_sslmode_is_require() {
        let database =
            parse_database_config("postgres://u@db/x", None, None).expect("postgres url");
        let DatabaseConfig::Postgres(config) = &database else {
            panic!("expected postgres");
        };
        assert_eq!(config.sslmode, SslMode::Require);
    }

    #[test]
    fn an_invalid_sslmode_is_rejected() {
        assert!(parse_database_config(
            "postgres://koreo@db/automation?sslmode=sometimes",
            None,
            None
        )
        .is_err());
    }
}
