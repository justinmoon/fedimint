use clap::Parser;
use minimint_shared::{load_from_file};
use minimint::config::{ServerConfig, ServerOpts};
use minimint::run_minimint;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tide=error")),
        )
        .init();

    let opts = ServerOpts::parse();
    let cfg: ServerConfig = load_from_file(&opts.cfg_path);

    run_minimint(cfg).await;
}
