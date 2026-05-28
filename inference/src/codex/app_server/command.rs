use super::*;

impl AppServerSession {
    pub(in crate::codex) fn build_command(&self, codex_home: &Path) -> Command {
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
}
