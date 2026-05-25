use protocol::MediaKind;

#[derive(Clone, Debug)]
pub struct GatewayCapabilities {
    pub content_types: Vec<MediaKind>,
    pub composing: bool,
    pub read_receipts: bool,
}
