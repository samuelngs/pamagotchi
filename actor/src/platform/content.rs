use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub kind: MediaKind,
    pub url: Option<String>,
    pub mime: Option<String>,
    pub filename: Option<String>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct PlatformCapabilities {
    pub content_types: Vec<MediaKind>,
    pub composing: bool,
    pub read_receipts: bool,
}
