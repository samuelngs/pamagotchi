use protocol::{ClientRequest, ServerEvent};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, warn};

static NEXT_API_CLIENT_ID: AtomicUsize = AtomicUsize::new(0);

type ClientWriters = Arc<Mutex<HashMap<usize, mpsc::Sender<ServerEvent>>>>;

#[derive(Clone)]
pub struct ApiServerHandle {
    clients: ClientWriters,
}

impl ApiServerHandle {
    pub async fn send_to(&self, client_id: usize, event: ServerEvent) -> anyhow::Result<()> {
        let tx = {
            let clients = self.clients.lock().await;
            clients.get(&client_id).cloned()
        };

        let Some(tx) = tx else {
            anyhow::bail!("api client not connected: {client_id}");
        };

        tx.send(event).await?;
        Ok(())
    }

    pub async fn broadcast(&self, event: ServerEvent) {
        let clients = {
            let clients = self.clients.lock().await;
            clients.values().cloned().collect::<Vec<_>>()
        };

        for tx in clients {
            let _ = tx.send(event.clone()).await;
        }
    }
}

#[derive(Clone, Debug)]
pub struct ApiClientRequest {
    pub client_id: usize,
    pub request: ClientRequest,
}

pub struct ApiServer {
    port: u16,
    handle: ApiServerHandle,
}

impl ApiServer {
    pub async fn listen(port: u16) -> anyhow::Result<(Self, mpsc::Receiver<ApiClientRequest>)> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let handle = ApiServerHandle {
            clients: clients.clone(),
        };
        let (request_tx, request_rx) = mpsc::channel(256);

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        let client_id = NEXT_API_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
                        debug!(client_id, %peer, "api client connected");
                        tokio::spawn(handle_connection(
                            client_id,
                            stream,
                            clients.clone(),
                            request_tx.clone(),
                        ));
                    }
                    Err(e) => error!("api accept error: {e}"),
                }
            }
        });

        Ok((Self { port, handle }, request_rx))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn handle(&self) -> ApiServerHandle {
        self.handle.clone()
    }
}

async fn handle_connection(
    client_id: usize,
    stream: TcpStream,
    clients: ClientWriters,
    request_tx: mpsc::Sender<ApiClientRequest>,
) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let (writer_tx, mut writer_rx) = mpsc::channel::<ServerEvent>(256);

    {
        let mut clients = clients.lock().await;
        clients.insert(client_id, writer_tx);
    }

    let write_task = tokio::spawn(async move {
        while let Some(event) = writer_rx.recv().await {
            let mut line = match serde_json::to_string(&event) {
                Ok(line) => line,
                Err(e) => {
                    warn!(%e, "failed to serialize api event");
                    continue;
                }
            };
            line.push('\n');
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let read_task = tokio::spawn(async move {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<ClientRequest>(trimmed) {
                        Ok(request) => {
                            let message = ApiClientRequest { client_id, request };
                            if request_tx.send(message).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => warn!(client_id, %e, "failed to parse api request"),
                    }
                }
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = read_task => {}
        _ = write_task => {}
    }

    {
        let mut clients = clients.lock().await;
        clients.remove(&client_id);
    }
    debug!(client_id, "api client disconnected");
}

pub struct ApiClient {
    writer_tx: mpsc::Sender<ClientRequest>,
    reader_rx: mpsc::Receiver<ServerEvent>,
}

impl ApiClient {
    pub async fn connect(port: u16) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port)).await?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let (writer_tx, mut writer_rx) = mpsc::channel::<ClientRequest>(256);
        let (reader_tx, reader_rx) = mpsc::channel::<ServerEvent>(256);

        tokio::spawn(async move {
            while let Some(request) = writer_rx.recv().await {
                let mut line = match serde_json::to_string(&request) {
                    Ok(line) => line,
                    Err(_) => continue,
                };
                line.push('\n');
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
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(event) = serde_json::from_str::<ServerEvent>(trimmed) {
                            if reader_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            writer_tx,
            reader_rx,
        })
    }

    pub async fn send(&self, request: ClientRequest) -> anyhow::Result<()> {
        self.writer_tx.send(request).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Option<ServerEvent> {
        self.reader_rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<ServerEvent> {
        self.reader_rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests;
