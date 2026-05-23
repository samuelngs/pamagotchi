mod config;
mod server;

use clap::{Parser, Subcommand};
use config::{ActorEntry, Config};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pamagotchi", version)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the runtime server
    Server,
    /// Manage actors
    Actor {
        #[command(subcommand)]
        command: ActorCommand,
    },
}

#[derive(Subcommand)]
enum ActorCommand {
    /// Add a new actor
    Add {
        /// Name for the actor
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(Config::default_path);

    match cli.command {
        Command::Server => {
            let config = Config::load(&config_path)?;
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| EnvFilter::new(&config.log.level)),
                )
                .init();
            server::run(config).await
        }
        Command::Actor { command } => match command {
            ActorCommand::Add { name } => actor_add(&name, &config_path),
        },
    }
}

fn actor_add(name: &str, config_path: &std::path::Path) -> anyhow::Result<()> {
    let mut config = Config::load_or_default(config_path)?;

    if config.actors.iter().any(|a| a.name == name) {
        anyhow::bail!("actor '{name}' already exists");
    }

    let id = config::generate_id();

    config.actors.push(ActorEntry {
        id: id.clone(),
        name: name.to_string(),
        provider: None,
        max_turns: 5,
        max_concurrency: 5,
        platforms: vec![],
    });

    config.save(config_path)?;

    println!("actor '{name}' added (id: {id})");
    println!("config: {}", config_path.display());
    println!();
    println!("next: edit {} to configure provider and platforms", config_path.display());

    Ok(())
}
