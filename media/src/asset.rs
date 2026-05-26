use std::path::PathBuf;

use protocol::{MediaAssetId, MediaKind};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaAsset {
    pub id: MediaAssetId,
    pub kind: MediaKind,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub size: u64,
    pub sha256: String,
    pub path: PathBuf,
    pub metadata: Value,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMediaAsset {
    pub kind: MediaKind,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub metadata: Value,
}

impl NewMediaAsset {
    pub fn new(kind: MediaKind) -> Self {
        Self {
            kind,
            mime: None,
            filename: None,
            metadata: Value::Object(Default::default()),
        }
    }

    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }
}
