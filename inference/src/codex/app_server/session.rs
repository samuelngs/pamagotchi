use super::*;

impl AppServerSession {
    pub(in crate::codex) async fn chat_stream_with_tools(
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
