use clap::Parser;
use pamagotchid::config::Config;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pamagotchid", version)]
struct Cli {
    #[arg(long)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(Config::default_path);
    let config = Config::load(&config_path)?;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log.level)),
        )
        .init();

    pamagotchid::server::run(config).await
}
