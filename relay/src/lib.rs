mod api;

pub use api::{ApiClient, ApiClientRequest, ApiServer, ApiServerHandle};

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RelayEvent {
    Subscribe { channel: String },
    Message { content: String },
    ComposingStarted,
    ComposingStopped,
}

static NEXT_CONN_ID: AtomicUsize = AtomicUsize::new(0);

pub struct RelayServer {
    port: u16,
}

impl RelayServer {
    pub async fn listen(port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        let (bus_tx, _) = broadcast::channel::<(usize, String, String)>(256);

        info!(port, "relay server listening");

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
                        debug!(id, %peer, "relay connection accepted");
                        let tx = bus_tx.clone();
                        let rx = bus_tx.subscribe();
                        tokio::spawn(handle_connection(id, stream, tx, rx));
                    }
                    Err(e) => error!("relay accept error: {e}"),
                }
            }
        });

        Ok(Self { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

async fn handle_connection(
    id: usize,
    stream: TcpStream,
    tx: broadcast::Sender<(usize, String, String)>,
    mut rx: broadcast::Receiver<(usize, String, String)>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let mut line = String::new();
    match reader.read_line(&mut line).await {
        Ok(0) => return,
        Err(_) => return,
        Ok(_) => {}
    }

    let channel = match serde_json::from_str::<RelayEvent>(line.trim()) {
        Ok(RelayEvent::Subscribe { channel }) => channel,
        _ => {
            warn!(id, "first message must be Subscribe");
            return;
        }
    };

    debug!(id, %channel, "relay connection subscribed");

    let read_channel = channel.clone();
    let read_task = tokio::spawn({
        let tx = tx.clone();
        async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end().to_string();
                        if !trimmed.is_empty() {
                            tx.send((id, read_channel.clone(), trimmed)).ok();
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    });

    let write_task = tokio::spawn(async move {
        while let Ok((sender_id, msg_channel, line)) = rx.recv().await {
            if sender_id == id || msg_channel != channel {
                continue;
            }
            let mut data = line;
            data.push('\n');
            if writer.write_all(data.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = read_task => {}
        _ = write_task => {}
    }

    debug!(id, "relay connection closed");
}

pub struct RelaySender {
    writer_tx: mpsc::Sender<String>,
}

impl RelaySender {
    pub async fn send(&self, event: RelayEvent) -> anyhow::Result<()> {
        let mut json = serde_json::to_string(&event)?;
        json.push('\n');
        self.writer_tx.send(json).await?;
        Ok(())
    }
}

pub struct RelayReceiver {
    reader_rx: mpsc::Receiver<RelayEvent>,
}

impl RelayReceiver {
    pub async fn recv(&mut self) -> Option<RelayEvent> {
        self.reader_rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<RelayEvent> {
        self.reader_rx.try_recv().ok()
    }
}

pub async fn connect(port: u16, channel: &str) -> anyhow::Result<(RelaySender, RelayReceiver)> {
    let stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let subscribe = serde_json::to_string(&RelayEvent::Subscribe {
        channel: channel.to_string(),
    })?;
    writer.write_all(subscribe.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let (writer_tx, mut writer_rx) = mpsc::channel::<String>(256);
    let (reader_tx, reader_rx) = mpsc::channel::<RelayEvent>(256);

    tokio::spawn(async move {
        while let Some(line) = writer_rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    if let Ok(event) = serde_json::from_str::<RelayEvent>(line.trim()) {
                        if reader_tx.send(event).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    debug!(port, channel, "relay client connected");
    Ok((RelaySender { writer_tx }, RelayReceiver { reader_rx }))
}
