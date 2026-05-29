use super::inbound::extract_message_content;
use crate::{GatewayConnectionState, GatewayRuntime, GatewaySetupInstructions};
use media::MediaStore;
use protocol::{
    ChannelKey, ChannelKind, GatewayId, InboundEnvelope, ObservedIdentityKey, ObservedSender,
};
use qrcode::{QrCode, render::unicode};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use whatsapp_rust::ChatStateEvent;
use whatsapp_rust::Client;
use whatsapp_rust::proto_helpers::MessageExt;
use whatsapp_rust::types::events::Event;

pub(super) async fn handle_event(
    gateway_id: &str,
    event: &Event,
    tx: &mpsc::Sender<InboundEnvelope>,
    runtime: &GatewayRuntime,
    client: &Client,
    media_store: &MediaStore,
) {
    match event {
        Event::PairingQrCode { code, .. } => {
            info!("whatsapp pairing QR code received");
            let rendered = render_qr_compact(code);
            let setup = Some(GatewaySetupInstructions::QrCode {
                title: "Connect WhatsApp".into(),
                body: "Scan this QR code from WhatsApp > Linked devices.".into(),
                code: code.clone(),
                rendered,
            });
            runtime
                .emit_state(gateway_id, GatewayConnectionState::SetupRequired)
                .await;
            runtime.emit_setup(gateway_id, setup).await;
        }
        Event::Connected(_) => {
            info!("whatsapp connected");
            runtime
                .emit_state(gateway_id, GatewayConnectionState::Connected)
                .await;
            runtime.emit_setup(gateway_id, None).await;
        }
        Event::Disconnected(_) => {
            warn!("whatsapp disconnected");
            runtime
                .emit_state(gateway_id, GatewayConnectionState::Disconnected)
                .await;
        }
        Event::Message(msg, info) => {
            if info.source.is_from_me {
                debug!(message_id = %info.id, "dropping self-message (is_from_me)");
                return;
            }

            let base = msg.get_base_message();
            let (content, attachments) = extract_message_content(client, media_store, base).await;

            if content.is_empty() && attachments.is_empty() {
                return;
            }

            let gateway = GatewayId(gateway_id.to_string());
            let sender = info.source.sender.to_string();
            let chat = info.source.chat.to_string();
            let aliases = info
                .source
                .sender_alt
                .as_ref()
                .map(ToString::to_string)
                .filter(|alt| alt != &sender)
                .map(|alt| whatsapp_identity_key(&gateway, &alt, "sender_alt"))
                .into_iter()
                .collect();

            let inbound = InboundEnvelope {
                gateway_id: gateway.clone(),
                platform_message_id: info.id.to_string(),
                channel: whatsapp_channel_key(&gateway, &chat, info.source.is_group),
                sender: Some(ObservedSender {
                    primary: whatsapp_identity_key(&gateway, &sender, "primary_sender"),
                    aliases,
                    display_name: serde_json::to_value(&info.push_name)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .filter(|name| !name.trim().is_empty()),
                    metadata: serde_json::json!({
                        "push_name": info.push_name,
                    }),
                }),
                content,
                attachments,
                timestamp: info.timestamp.timestamp(),
                metadata: serde_json::json!({
                    "sender": sender,
                    "sender_alt": info.source.sender_alt.as_ref().map(ToString::to_string),
                    "message_id": info.id.to_string(),
                    "push_name": info.push_name,
                    "is_group": info.source.is_group,
                }),
            };

            if let Err(e) = inbound.validate() {
                warn!(%e, message_id = %info.id, "invalid whatsapp inbound envelope");
                return;
            }

            if let Err(e) = tx.send(inbound).await {
                warn!("failed to forward whatsapp message: {e}");
            }
        }
        _ => {}
    }
}

pub(super) async fn handle_chatstate_event(
    gateway_id: &str,
    event: ChatStateEvent,
    runtime: &GatewayRuntime,
) {
    let gateway = GatewayId(gateway_id.to_string());
    let chat = event.chat.to_string();
    let participant = event.participant.as_ref().map(ToString::to_string);
    let state = format!("{:?}", event.state);
    let (channel, sender, typing) =
        typing_update_from_chatstate(&gateway, &chat, participant.as_deref(), &state);
    runtime
        .emit_typing(gateway_id, channel, sender, typing)
        .await;
}

pub(super) fn typing_update_from_chatstate(
    gateway_id: &GatewayId,
    chat: &str,
    participant: Option<&str>,
    state: &str,
) -> (ChannelKey, ObservedIdentityKey, bool) {
    let sender_external_id = participant.unwrap_or(chat).to_string();
    let typing = matches!(state, "Typing" | "RecordingAudio");
    (
        whatsapp_channel_key(gateway_id, chat, participant.is_some()),
        whatsapp_identity_key(gateway_id, &sender_external_id, "chat_state"),
        typing,
    )
}

fn whatsapp_channel_key(gateway_id: &GatewayId, chat: &str, is_group: bool) -> ChannelKey {
    ChannelKey {
        gateway_id: gateway_id.clone(),
        external_id: chat.to_string(),
        kind: if is_group {
            ChannelKind::GroupChat
        } else {
            ChannelKind::Direct
        },
        display_name: None,
        space: None,
        parent: None,
        metadata: serde_json::json!({
            "platform": "whatsapp",
            "is_group": is_group,
        }),
    }
}

fn whatsapp_identity_key(
    gateway_id: &GatewayId,
    external_id: &str,
    source: &str,
) -> ObservedIdentityKey {
    ObservedIdentityKey {
        gateway_id: gateway_id.clone(),
        external_id: external_id.to_string(),
        kind: Some(whatsapp_identity_kind(external_id).to_string()),
        confidence: 1.0,
        source: source.to_string(),
    }
}

fn whatsapp_identity_kind(external_id: &str) -> &'static str {
    if external_id.contains("@lid") {
        "lid"
    } else if external_id.contains("@s.whatsapp.net") || external_id.contains("@c.us") {
        "phone"
    } else {
        "whatsapp_jid"
    }
}

fn render_qr_compact(code: &str) -> String {
    QrCode::new(code.as_bytes())
        .map(|qr| qr.render::<unicode::Dense1x2>().quiet_zone(false).build())
        .unwrap_or_default()
}
