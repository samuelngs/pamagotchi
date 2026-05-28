use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GatewaySettings {
    #[serde(default)]
    pub gateway: Vec<GatewayEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GatewayEntry {
    pub id: String,
    pub kind: String,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, Value>,
}

#[derive(Clone, Debug)]
pub struct GatewayStore {
    path: PathBuf,
}

impl GatewayStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn for_data_dir(data_dir: impl AsRef<Path>) -> Self {
        Self::new(settings_path(data_dir))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> anyhow::Result<GatewaySettings> {
        if !self.path.exists() {
            return Ok(GatewaySettings::default());
        }

        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read gateway settings: {}", self.path.display()))?;
        let settings: GatewaySettings =
            yaml_serde::from_str(&raw).context("failed to parse gateway settings yaml")?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn load_or_create(&self) -> anyhow::Result<GatewaySettings> {
        if self.path.exists() {
            self.load()
        } else {
            let settings = GatewaySettings::default();
            self.save(&settings)?;
            Ok(settings)
        }
    }

    pub fn save(&self, settings: &GatewaySettings) -> anyhow::Result<()> {
        settings.validate()?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create gateway settings dir: {}",
                    parent.display()
                )
            })?;
        }
        let yaml =
            yaml_serde::to_string(settings).context("failed to serialize gateway settings")?;
        std::fs::write(&self.path, yaml).with_context(|| {
            format!("failed to write gateway settings: {}", self.path.display())
        })?;
        Ok(())
    }

    pub fn add(
        &self,
        kind: impl Into<String>,
        vars: BTreeMap<String, Value>,
    ) -> anyhow::Result<GatewayEntry> {
        let mut settings = self.load()?;
        let entry = GatewayEntry {
            id: generate_gateway_id(&settings),
            kind: kind.into(),
            vars,
        };
        settings.gateway.push(entry.clone());
        self.save(&settings)?;
        Ok(entry)
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<Option<GatewayEntry>> {
        let mut settings = self.load()?;
        let Some(index) = settings.gateway.iter().position(|entry| entry.id == id) else {
            return Ok(None);
        };
        let removed = settings.gateway.remove(index);
        self.save(&settings)?;
        Ok(Some(removed))
    }

    pub fn update_vars(
        &self,
        id: &str,
        vars: BTreeMap<String, Value>,
    ) -> anyhow::Result<Option<GatewayEntry>> {
        let mut settings = self.load()?;
        let Some(entry) = settings.gateway.iter_mut().find(|entry| entry.id == id) else {
            return Ok(None);
        };
        entry.vars = vars;
        let updated = entry.clone();
        self.save(&settings)?;
        Ok(Some(updated))
    }
}

impl GatewaySettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut ids = std::collections::HashSet::new();
        for entry in &self.gateway {
            if entry.id.trim().is_empty() {
                anyhow::bail!("gateway id cannot be empty");
            }
            if entry.kind.trim().is_empty() {
                anyhow::bail!("gateway kind cannot be empty");
            }
            if !ids.insert(entry.id.as_str()) {
                anyhow::bail!("duplicate gateway id: {}", entry.id);
            }
        }
        Ok(())
    }
}

pub fn settings_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("gateway.yml")
}

pub fn gateway_data_dir(data_dir: impl AsRef<Path>, gateway_id: &str) -> PathBuf {
    data_dir.as_ref().join("gateways").join(gateway_id)
}

fn generate_gateway_id(settings: &GatewaySettings) -> String {
    loop {
        let id = nanoid::nanoid!(12);
        if !settings.gateway.iter().any(|entry| entry.id == id) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests;
