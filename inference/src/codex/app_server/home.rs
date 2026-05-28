use super::*;

pub(super) struct IsolatedCodexHome {
    path: PathBuf,
}

impl IsolatedCodexHome {
    pub(super) async fn create() -> anyhow::Result<Self> {
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

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) async fn cleanup(self) {
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

fn temp_path(label: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pamagotchi-codex-{now}-{seq}.{label}"))
}
