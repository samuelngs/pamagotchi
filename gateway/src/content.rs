#[derive(Clone, Debug)]
pub struct GatewayCapabilities {
    pub content: GatewayContentCapabilities,
    pub composing: bool,
    pub read_receipts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GatewayContentKind {
    Text,
    Image,
    Video,
    Audio,
    Voice,
    VideoChat,
    Sticker,
    File,
}

impl GatewayContentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Voice => "voice",
            Self::VideoChat => "video_chat",
            Self::Sticker => "sticker",
            Self::File => "file",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Voice => "Voice",
            Self::VideoChat => "Video chat",
            Self::Sticker => "Sticker",
            Self::File => "File",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GatewayContentCapabilities {
    pub receive: Vec<GatewayContentKind>,
    pub send: Vec<GatewayContentKind>,
}

impl GatewayContentCapabilities {
    pub fn text_only() -> Self {
        Self {
            receive: vec![GatewayContentKind::Text],
            send: vec![GatewayContentKind::Text],
        }
    }
}

#[cfg(test)]
mod tests;
