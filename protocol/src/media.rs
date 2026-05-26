use serde::{Deserialize, Serialize};

use crate::id::MediaAssetId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Sticker,
    File,
}

impl MediaKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Sticker => "sticker",
            Self::File => "file",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Sticker => "Sticker",
            Self::File => "File",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "sticker" => Some(Self::Sticker),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub kind: MediaKind,
    #[serde(default)]
    pub asset_id: Option<MediaAssetId>,
    pub url: Option<String>,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_attachment_defaults_missing_asset_id() {
        let attachment: MediaAttachment = serde_json::from_value(serde_json::json!({
            "kind": "Image",
            "url": "https://example.test/image.png",
            "mime": "image/png",
            "filename": "image.png",
            "size": 10
        }))
        .unwrap();

        assert_eq!(attachment.kind, MediaKind::Image);
        assert_eq!(attachment.asset_id, None);
    }
}
