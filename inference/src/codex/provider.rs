use super::events::handle_event;
use super::options::CodexOptions;
use super::prompt::prompt_from_request;
use crate::{ChatRequest, ChatResponse, ChatStream, Provider, StreamEvent};
use anyhow::{Context, bail};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

static NEXT_OUTPUT_ID: AtomicU64 = AtomicU64::new(0);

pub struct CodexProvider {
    options: CodexOptions,
}

impl CodexProvider {
    pub fn new(options: CodexOptions) -> Self {
        Self { options }
    }

    fn build_command(&self, model: &str, output_path: &Path) -> Command {
        let mut cmd = Command::new(&self.options.command);
        cmd.arg("exec")
            .arg("--json")
            .arg("--color")
            .arg("never")
            .arg("--output-last-message")
            .arg(output_path)
            .arg("--model")
            .arg(model);

        if self.options.ephemeral {
            cmd.arg("--ephemeral");
        }
        if self.options.skip_git_repo_check {
            cmd.arg("--skip-git-repo-check");
        }
        if self.options.search {
            cmd.arg("--search");
        }
        if self.options.ignore_user_config {
            cmd.arg("--ignore-user-config");
        }
        if self.options.ignore_rules {
            cmd.arg("--ignore-rules");
        }
        if let Some(profile) = &self.options.profile {
            cmd.arg("--profile").arg(profile);
        }
        if let Some(profile_v2) = &self.options.profile_v2 {
            cmd.arg("--profile-v2").arg(profile_v2);
        }
        if let Some(sandbox) = &self.options.sandbox {
            cmd.arg("--sandbox").arg(sandbox);
        }
        if let Some(approval_policy) = &self.options.approval_policy {
            cmd.arg("--ask-for-approval").arg(approval_policy);
        }
        if let Some(cwd) = &self.options.cwd {
            cmd.arg("--cd").arg(cwd);
        }
        cmd.args(&self.options.extra_args);
        cmd.arg("-");
        cmd
    }
}

#[async_trait]
impl Provider for CodexProvider {
    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        self.chat_stream(request).await?.collect().await
    }

    async fn chat_stream(&self, request: &ChatRequest) -> anyhow::Result<ChatStream> {
        let output_path = temp_output_path();
        let prompt = prompt_from_request(request);
        let mut cmd = self.build_command(&request.model, &output_path);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        debug!(
            command = %self.options.command,
            model = %request.model,
            messages = request.messages.len(),
            tools = request.tools.len(),
            "starting codex exec"
        );

        let mut child = cmd.spawn().context("failed to spawn codex exec")?;
        let mut stdin = child
            .stdin
            .take()
            .context("failed to open codex exec stdin")?;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .context("failed to write prompt to codex exec")?;
        stdin
            .shutdown()
            .await
            .context("failed to close codex exec stdin")?;

        let stdout = child
            .stdout
            .take()
            .context("failed to open codex exec stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to open codex exec stderr")?;
        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(err) = stream_codex(stdout, stderr, child, output_path, tx.clone()).await {
                let _ = tx.send(Err(err)).await;
            }
        });

        Ok(ChatStream::new(rx))
    }
}

async fn stream_codex(
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    mut child: tokio::process::Child,
    output_path: PathBuf,
    tx: mpsc::Sender<anyhow::Result<StreamEvent>>,
) -> anyhow::Result<()> {
    let stderr_task = tokio::spawn(read_stderr(stderr));
    let mut lines = BufReader::new(stdout).lines();
    let mut saw_text = false;
    let mut failed = false;

    while let Some(line) = lines
        .next_line()
        .await
        .context("failed to read codex exec stdout")?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(event) => {
                handle_event(event, &tx, &mut saw_text, &mut failed).await?;
            }
            Err(err) => warn!(%err, line, "failed to parse codex exec JSONL event"),
        }
    }

    let status = child
        .wait()
        .await
        .context("failed to wait for codex exec")?;
    let stderr = stderr_task
        .await
        .unwrap_or_else(|err| format!("failed to join stderr task: {err}"));

    if !status.success() {
        let _ = tokio::fs::remove_file(&output_path).await;
        bail!("codex exec exited with {status}: {}", stderr.trim());
    }
    if failed {
        let _ = tokio::fs::remove_file(&output_path).await;
        bail!("codex exec turn failed");
    }

    if !saw_text {
        match tokio::fs::read_to_string(&output_path).await {
            Ok(text) if !text.is_empty() => {
                tx.send(Ok(StreamEvent::TextDelta(text))).await?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).context("failed to read codex last-message file"),
        }
    }
    let _ = tokio::fs::remove_file(&output_path).await;

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

fn temp_output_path() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = NEXT_OUTPUT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pamagotchi-codex-{now}-{seq}.txt"))
}
