use super::events::{
    AppServerEventState, AppServerNotification, handle_notification, parse_notification,
    send_finish_reason,
};
use super::options::CodexOptions;
use crate::{
    AppServerToolCall, AppServerToolResult, AppServerToolResultContent, AppServerToolRuntime,
    ChatRequest, ChatStream, FinishReason, StreamEvent,
};
use anyhow::{Context, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub(super) struct AppServerSession {
    options: CodexOptions,
}

impl AppServerSession {
    pub(super) fn new(options: CodexOptions) -> Self {
        Self { options }
    }

    pub(super) fn build_command(&self, codex_home: &Path) -> Command {
        let mut cmd = Command::new(&self.options.command);
        cmd.arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .arg("-c")
            .arg("approval_policy=\"never\"")
            .arg("--disable")
            .arg("hooks")
            .arg("--disable")
            .arg("plugin_hooks")
            .arg("--disable")
            .arg("plugins")
            .arg("--disable")
            .arg("apps")
            .arg("--disable")
            .arg("memories")
            .env("CODEX_HOME", codex_home);
        cmd.args(&self.options.extra_args);
        cmd
    }

    pub(super) async fn chat_stream(
        self,
        request: ChatRequest,
        prompt: String,
    ) -> anyhow::Result<ChatStream> {
        self.chat_stream_with_tools(request, prompt, Arc::new(UnsupportedToolRuntime))
            .await
    }

    pub(super) async fn chat_stream_with_tools(
        self,
        request: ChatRequest,
        prompt: String,
        tools: Arc<dyn AppServerToolRuntime>,
    ) -> anyhow::Result<ChatStream> {
        let codex_home = IsolatedCodexHome::create().await?;
        let mut cmd = self.build_command(codex_home.path());
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        debug!(
            command = %self.options.command,
            model = %request.model,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "starting codex app-server"
        );

        let mut child = cmd.spawn().context("failed to spawn codex app-server")?;
        let stdin = child
            .stdin
            .take()
            .context("failed to open codex app-server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to open codex app-server stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to open codex app-server stderr")?;

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            if let Err(err) = run_session(
                self.options,
                request,
                prompt,
                stdin,
                stdout,
                stderr,
                child,
                tools,
                tx.clone(),
            )
            .await
            {
                let _ = tx.send(Err(err)).await;
            }
            codex_home.cleanup().await;
        });

        Ok(ChatStream::new(rx))
    }
}

async fn run_session(
    options: CodexOptions,
    request: ChatRequest,
    prompt: String,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    mut child: tokio::process::Child,
    tools: Arc<dyn AppServerToolRuntime>,
    tx: mpsc::Sender<anyhow::Result<StreamEvent>>,
) -> anyhow::Result<()> {
    let stderr_task = tokio::spawn(read_stderr(stderr));
    let mut rpc = JsonRpcConnection {
        stdin,
        lines: BufReader::new(stdout).lines(),
    };
    let result = run_turn(options, request, prompt, &mut rpc, tools, &tx).await;

    let _ = child.start_kill();
    let status = child
        .wait()
        .await
        .context("failed to wait for codex app-server")?;
    let stderr = stderr_task
        .await
        .unwrap_or_else(|err| format!("failed to join stderr task: {err}"));

    result?;
    if !status.success() && status.code().is_some() {
        bail!("codex app-server exited with {status}: {}", stderr.trim());
    }
    Ok(())
}

async fn run_turn(
    options: CodexOptions,
    request: ChatRequest,
    prompt: String,
    rpc: &mut JsonRpcConnection,
    tools: Arc<dyn AppServerToolRuntime>,
    tx: &mpsc::Sender<anyhow::Result<StreamEvent>>,
) -> anyhow::Result<()> {
    rpc.send_request(
        1,
        "initialize",
        json!({
            "clientInfo": {
                "name": "pamagotchi",
                "title": "Pamagotchi",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": true
            }
        }),
    )
    .await?;
    rpc.wait_for_response(1).await?;
    rpc.send_notification("initialized", json!({})).await?;

    rpc.send_request(2, "thread/start", thread_start_params(&options, &request))
        .await?;
    let thread_response = rpc.wait_for_response(2).await?;
    let thread_id = thread_response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .context("codex app-server thread/start response missing thread.id")?
        .to_owned();

    rpc.send_request(
        3,
        "turn/start",
        turn_start_params(&options, &request, &prompt, &thread_id)?,
    )
    .await?;

    let mut state = AppServerEventState::new(true);
    while !state.completed() {
        match rpc.next_message().await? {
            RpcMessage::Response { id, result } if id == 3 => {
                debug!(?result, "codex app-server turn/start accepted");
            }
            RpcMessage::Response { .. } => {}
            RpcMessage::Error { id, message } => {
                bail!("codex app-server request {id} failed: {message}");
            }
            RpcMessage::Notification(notification) => {
                handle_notification(notification, &tx, &mut state).await?;
            }
            RpcMessage::Request { id, method, params } => {
                if method == "item/tool/call" {
                    let call = parse_dynamic_tool_call(params)?;
                    let response = tool_response(tools.call_tool(call).await?);
                    rpc.send_response(id, response).await?;
                } else {
                    rpc.send_error(id, -32601, format!("unsupported server request: {method}"))
                        .await?;
                }
            }
        }
    }

    send_finish_reason(&tx, FinishReason::Stop).await?;

    Ok(())
}

fn thread_start_params(options: &CodexOptions, request: &ChatRequest) -> Value {
    let mut params = json!({
        "model": request.model,
        "approvalPolicy": "never",
        "approvalsReviewer": "user",
        "ephemeral": true,
        "sandbox": options.sandbox.as_deref().unwrap_or("read-only"),
    });
    if let Some(cwd) = &options.cwd {
        params["cwd"] = Value::String(cwd.clone());
    }
    if !request.tools.is_empty() {
        params["dynamicTools"] = dynamic_tools(&request.tools);
    }
    params
}

fn turn_start_params(
    options: &CodexOptions,
    request: &ChatRequest,
    prompt: &str,
    thread_id: &str,
) -> anyhow::Result<Value> {
    let mut params = json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": prompt}],
        "model": request.model,
        "approvalPolicy": "never",
    });
    if let Some(cwd) = &options.cwd {
        params["cwd"] = Value::String(cwd.clone());
    }
    Ok(params)
}

pub(super) fn dynamic_tools(tools: &[crate::Tool]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.parameters,
                })
            })
            .collect(),
    )
}

pub(super) fn parse_dynamic_tool_call(params: Value) -> anyhow::Result<AppServerToolCall> {
    let params: DynamicToolCallParams =
        serde_json::from_value(params).context("failed to parse codex dynamic tool call")?;
    Ok(AppServerToolCall {
        id: params.call_id,
        name: params.tool,
        arguments: params.arguments,
        namespace: params.namespace,
    })
}

pub(super) fn tool_response(result: AppServerToolResult) -> Value {
    let content_items = result
        .content
        .into_iter()
        .map(|content| match content {
            AppServerToolResultContent::Text(text) => {
                json!({ "type": "inputText", "text": text })
            }
            AppServerToolResultContent::ImageUrl(image_url) => {
                json!({ "type": "inputImage", "imageUrl": image_url })
            }
        })
        .collect::<Vec<_>>();
    json!({
        "success": result.success,
        "contentItems": content_items,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicToolCallParams {
    arguments: Value,
    call_id: String,
    #[serde(default)]
    namespace: Option<String>,
    tool: String,
}

struct UnsupportedToolRuntime;

#[async_trait]
impl AppServerToolRuntime for UnsupportedToolRuntime {
    async fn call_tool(&self, call: AppServerToolCall) -> anyhow::Result<AppServerToolResult> {
        Ok(AppServerToolResult::error(format!(
            "unsupported app-server tool call: {}",
            call.name
        )))
    }
}

struct JsonRpcConnection {
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl JsonRpcConnection {
    async fn send_request(&mut self, id: u64, method: &str, params: Value) -> anyhow::Result<()> {
        self.send(json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        let mut notification = json!({ "method": method });
        if !params.as_object().is_some_and(serde_json::Map::is_empty) {
            notification["params"] = params;
        }
        self.send(notification).await
    }

    async fn send_error(&mut self, id: Value, code: i64, message: String) -> anyhow::Result<()> {
        self.send(json!({
            "id": id,
            "error": {
                "code": code,
                "message": message,
            },
        }))
        .await
    }

    async fn send_response(&mut self, id: Value, result: Value) -> anyhow::Result<()> {
        self.send(json!({
            "id": id,
            "result": result,
        }))
        .await
    }

    async fn send(&mut self, message: Value) -> anyhow::Result<()> {
        let mut bytes =
            serde_json::to_vec(&message).context("failed to serialize app-server message")?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .context("failed to write app-server message")?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn wait_for_response(&mut self, id: u64) -> anyhow::Result<Value> {
        loop {
            match self.next_message().await? {
                RpcMessage::Response { id: got, result } if got == id => return Ok(result),
                RpcMessage::Error { id: got, message } if got == Value::from(id) => {
                    bail!("codex app-server request {id} failed: {message}");
                }
                RpcMessage::Response { .. }
                | RpcMessage::Error { .. }
                | RpcMessage::Notification(_)
                | RpcMessage::Request { .. } => {}
            }
        }
    }

    async fn next_message(&mut self) -> anyhow::Result<RpcMessage> {
        loop {
            let Some(line) = self
                .lines
                .next_line()
                .await
                .context("failed to read codex app-server stdout")?
            else {
                bail!("codex app-server stdout closed");
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(err) => {
                    warn!(%err, line, "failed to parse codex app-server JSON-RPC message");
                    continue;
                }
            };
            return RpcMessage::from_value(value);
        }
    }
}

enum RpcMessage {
    Response {
        id: u64,
        result: Value,
    },
    Error {
        id: Value,
        message: String,
    },
    Notification(AppServerNotification),
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

impl RpcMessage {
    fn from_value(value: Value) -> anyhow::Result<Self> {
        if let Some(error) = value.get("error") {
            return Ok(Self::Error {
                id: value.get("id").cloned().unwrap_or(Value::Null),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown app-server error")
                    .to_owned(),
            });
        }
        if value.get("result").is_some() {
            let id = value
                .get("id")
                .and_then(Value::as_u64)
                .context("app-server response missing numeric id")?;
            return Ok(Self::Response {
                id,
                result: value.get("result").cloned().unwrap_or(Value::Null),
            });
        }
        if value.get("id").is_some() {
            return Ok(Self::Request {
                id: value.get("id").cloned().unwrap_or(Value::Null),
                method: value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            });
        }
        Ok(Self::Notification(parse_notification(value)?))
    }
}

struct IsolatedCodexHome {
    path: PathBuf,
}

impl IsolatedCodexHome {
    async fn create() -> anyhow::Result<Self> {
        let path = temp_path("home");
        tokio::fs::create_dir_all(&path)
            .await
            .context("failed to create isolated CODEX_HOME")?;

        let source_home = source_codex_home()?;
        link_or_copy(&source_home.join("auth.json"), &path.join("auth.json")).await?;
        link_or_copy(
            &source_home.join("installation_id"),
            &path.join("installation_id"),
        )
        .await?;

        tokio::fs::write(
            path.join("config.toml"),
            r#"approval_policy = "never"
approvals_reviewer = "user"
sandbox_mode = "read-only"

[features]
hooks = false
plugin_hooks = false
plugins = false
apps = false
memories = false
enable_mcp_apps = false
"#,
        )
        .await
        .context("failed to write isolated codex config")?;

        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    async fn cleanup(self) {
        let _ = tokio::fs::remove_dir_all(self.path).await;
    }
}

fn source_codex_home() -> anyhow::Result<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".codex"))
}

async fn link_or_copy(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if tokio::fs::metadata(source).await.is_err() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(source, destination).is_ok() {
            return Ok(());
        }
    }
    tokio::fs::copy(source, destination)
        .await
        .with_context(|| format!("failed to copy {}", source.display()))?;
    Ok(())
}

async fn read_stderr(stderr: tokio::process::ChildStderr) -> String {
    let mut lines = BufReader::new(stderr).lines();
    let mut out = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
    }
    out
}

fn temp_path(label: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pamagotchi-codex-{now}-{seq}.{label}"))
}
