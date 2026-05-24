mod tui;

use clap::{Parser, Subcommand};
use runtime::config::Config;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pamagotchi", version)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run headless server (no TUI)
    Server,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(Config::default_path);

    match cli.command {
        Some(Command::Server) => {
            let config = Config::load(&config_path)?;
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new(&config.log.level)),
                )
                .init();
            runtime::server::run(config).await
        }
        None => {
            let config = Config::load_or_default(&config_path)?;
            tui::run(config).await
        }
    }
}
