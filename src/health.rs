use std::time::Duration;

const HEALTH_PATH: &str = "/healthz";

pub fn healthcheck_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .any(|arg| arg.as_ref() == "healthcheck")
}

pub async fn healthcheck() -> anyhow::Result<i32> {
    let url = std::env::var("KOREOGRAF_HEALTHCHECK_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let bind = std::env::var("KOREOGRAF_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
            healthcheck_url(&bind)
        });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => Ok(0),
        Ok(response) => {
            eprintln!("Health check failed: {url} answered {}.", response.status());
            Ok(1)
        }
        Err(err) => {
            eprintln!("Health check failed: {url} did not answer ({err}).");
            Ok(1)
        }
    }
}

pub fn healthcheck_url(bind: &str) -> String {
    let bind = bind.trim();
    let (host, port) = bind.rsplit_once(':').unwrap_or(("", "8080"));
    let host = host.trim_matches(['[', ']']);
    let host = if host.is_empty() || host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host
    };
    format!("http://{host}:{port}{HEALTH_PATH}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_healthcheck_flag_is_a_subcommand_after_the_program_name() {
        assert!(healthcheck_requested(["koreograf", "healthcheck"]));
        assert!(!healthcheck_requested(["koreograf"]));
        assert!(!healthcheck_requested(["koreograf", "--version"]));
    }

    #[test]
    fn a_wildcard_bind_uses_the_loopback_interface() {
        assert_eq!(
            healthcheck_url("0.0.0.0:8080"),
            "http://127.0.0.1:8080/healthz"
        );
        assert_eq!(
            healthcheck_url("[::]:9000"),
            "http://127.0.0.1:9000/healthz"
        );
        assert_eq!(
            healthcheck_url("127.0.0.1:8081"),
            "http://127.0.0.1:8081/healthz"
        );
        assert_eq!(
            healthcheck_url("koreograf:8080"),
            "http://koreograf:8080/healthz"
        );
    }
}
