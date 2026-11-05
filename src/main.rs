#![forbid(unsafe_code)]

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(kind) = koreograf::askpass_kind(&args) {
        let prompt = args.get(1).map(String::as_str).unwrap_or_default();
        match koreograf::askpass(kind, prompt) {
            Ok(answer) => println!("{answer}"),
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    if koreograf::git_ssh_requested(&args) {
        let rest = args.iter().skip(1).map(String::as_str);
        if let Err(err) = koreograf::git_ssh(rest) {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if koreograf::entrypoint_requested(&args) {
        if let Err(err) = koreograf::entrypoint(&args) {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if koreograf::healthcheck_requested(&args) {
        let code = koreograf::healthcheck().await?;
        std::process::exit(code);
    }
    if koreograf::version_requested(&args) {
        println!("{}", koreograf::version_line());
        return Ok(());
    }
    if koreograf::doctor_requested(&args) {
        let code = koreograf::doctor().await?;
        std::process::exit(code);
    }
    if koreograf::mcp::requested(&args) {
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("koreograf=info"));
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .init();
        if let Err(err) = koreograf::mcp::run().await {
            eprintln!("{err}");
            std::process::exit(1);
        }
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
        mode = cfg.runner.in_container(cfg.container_image).as_str(),
        ldap = cfg.ldap.is_some(),
        "configuration loaded"
    );
    koreograf::serve(cfg).await
}
