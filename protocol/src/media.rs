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
            "image" | "Image" => Some(Self::Image),
            "video" | "Video" => Some(Self::Video),
            "audio" | "Audio" | "voice" | "Voice" => Some(Self::Audio),
            "sticker" | "Sticker" => Some(Self::Sticker),
            "file" | "File" => Some(Self::File),
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
mod tests;
