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
mod tests {
    use super::*;

    #[test]
    fn content_kind_names_include_realtime_modes() {
        assert_eq!(GatewayContentKind::Voice.as_str(), "voice");
        assert_eq!(GatewayContentKind::Voice.label(), "Voice");
        assert_eq!(GatewayContentKind::VideoChat.as_str(), "video_chat");
        assert_eq!(GatewayContentKind::VideoChat.label(), "Video chat");
    }

    #[test]
    fn text_only_supports_text_in_both_directions() {
        let capabilities = GatewayContentCapabilities::text_only();

        assert_eq!(capabilities.receive, vec![GatewayContentKind::Text]);
        assert_eq!(capabilities.send, vec![GatewayContentKind::Text]);
    }
}
