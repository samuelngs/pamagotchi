mod tui;

use clap::Parser;

#[derive(Parser)]
#[command(name = "pamagotchi", version)]
struct Cli {
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
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
    tui::run(port).await
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
