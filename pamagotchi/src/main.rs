mod debug_view;
mod tui;

use clap::{Parser, Subcommand};
use protocol::{ClientRequest, ServerEvent};
use relay::ApiClient;

#[derive(Parser)]
#[command(name = "pamagotchi", version)]
struct Cli {
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Debug {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        human: bool,
    },
}

fn default_data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".pamagotchi/data")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(default_data_dir);
    let pid_path = data_dir.join("pamagotchid.pid");
    let port = read_daemon_port(&pid_path)?;
    match cli.command {
        Some(Command::Debug { limit, human }) => print_debug_snapshot(port, limit, human).await,
        None => tui::run(port).await,
    }
}

fn read_daemon_port(pid_path: &std::path::Path) -> anyhow::Result<u16> {
    let content = std::fs::read_to_string(pid_path)
        .map_err(|_| anyhow::anyhow!("daemon not running (no pid file)"))?;
    let mut lines = content.lines();
    let _pid = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("malformed pid file"))?;
    let port: u16 = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("no port in pid file"))?
        .parse()?;
    Ok(port)
}

async fn print_debug_snapshot(port: u16, limit: usize, human: bool) -> anyhow::Result<()> {
    let mut api = ApiClient::connect(port).await?;
    let request_id = format!("debug-{}", now_millis());
    api.send(ClientRequest::GetDebugSnapshot {
        request_id: request_id.clone(),
        limit: Some(limit),
    })
    .await?;

    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), api.recv())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for debug snapshot"))?
            .ok_or_else(|| anyhow::anyhow!("daemon closed connection"))?;

        match event {
            ServerEvent::DebugSnapshot {
                request_id: id,
                snapshot,
            } if id == request_id => {
                if human {
                    println!("{}", debug_view::format_snapshot(&snapshot));
                } else {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                }
                return Ok(());
            }
            ServerEvent::RequestError {
                request_id: Some(id),
                message,
            } if id == request_id => anyhow::bail!(message),
            _ => {}
        }
    }
}

fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
