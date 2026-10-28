#![forbid(unsafe_code)]

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if koreograf::version_requested(std::env::args()) {
        println!("{}", koreograf::version());
        return Ok(());
    }
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("koreograf=info,tower_http=info"));
    let json = std::env::var("KOREOGRAF_LOG_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    let cfg = koreograf::Config::from_env();
    tracing::info!(
        mode = cfg.runner.as_str(),
        ldap = cfg.ldap.is_some(),
        "configuration loaded"
    );
    koreograf::serve(cfg).await
}
