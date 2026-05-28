use super::*;

pub(super) struct JsonRpcConnection {
    pub(super) stdin: tokio::process::ChildStdin,
    pub(super) lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl JsonRpcConnection {
    pub(super) async fn send_request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> anyhow::Result<()> {
        self.send(json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .await
    }

    pub(super) async fn send_notification(
        &mut self,
        method: &str,
        params: Value,
    ) -> anyhow::Result<()> {
        let mut notification = json!({ "method": method });
        if !params.as_object().is_some_and(serde_json::Map::is_empty) {
            notification["params"] = params;
        }
        self.send(notification).await
    }

    pub(super) async fn send_error(
        &mut self,
        id: Value,
        code: i64,
        message: String,
    ) -> anyhow::Result<()> {
        self.send(json!({
            "id": id,
            "error": {
                "code": code,
                "message": message,
            },
        }))
        .await
    }

    pub(super) async fn send_response(&mut self, id: Value, result: Value) -> anyhow::Result<()> {
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

    pub(super) async fn wait_for_response(&mut self, id: u64) -> anyhow::Result<Value> {
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

    pub(super) async fn next_message(&mut self) -> anyhow::Result<RpcMessage> {
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

pub(super) enum RpcMessage {
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
