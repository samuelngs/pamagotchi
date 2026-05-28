use super::inbound::extract_message_content;
use crate::{GatewayConnectionState, GatewayRuntime, GatewaySetupInstructions};
use media::MediaStore;
use protocol::{ConversationId, GroupId, InboundMessage};
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
    tx: &mpsc::Sender<InboundMessage>,
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

            let sender = info.source.sender.to_string();
            let chat = info.source.chat.to_string();

            let inbound = InboundMessage {
                message_id: info.id.to_string(),
                gateway_id: gateway_id.to_string(),
                sender_external_id: sender.clone(),
                sender_display_name: serde_json::to_value(&info.push_name)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .filter(|name| !name.trim().is_empty()),
                reply_external_id: chat.clone(),
                conversation: ConversationId(format!("{gateway_id}:{chat}")),
                group: if info.source.is_group {
                    Some(GroupId(chat))
                } else {
                    None
                },
                identity: None,
                profile: None,
                person: None,
                content,
                attachments,
                timestamp: info.timestamp.timestamp(),
                metadata: serde_json::json!({
                    "sender": sender,
                    "message_id": info.id.to_string(),
                    "push_name": info.push_name,
                }),
            };

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
    let chat = event.chat.to_string();
    let participant = event.participant.as_ref().map(ToString::to_string);
    let state = format!("{:?}", event.state);
    let (conversation, sender_external_id, typing) =
        typing_update_from_chatstate(gateway_id, &chat, participant.as_deref(), &state);
    runtime
        .emit_typing(gateway_id, conversation, sender_external_id, typing)
        .await;
}

pub(super) fn typing_update_from_chatstate(
    gateway_id: &str,
    chat: &str,
    participant: Option<&str>,
    state: &str,
) -> (ConversationId, String, bool) {
    let sender_external_id = participant.unwrap_or(chat).to_string();
    let typing = matches!(state, "Typing" | "RecordingAudio");
    (
        ConversationId(format!("{gateway_id}:{chat}")),
        sender_external_id,
        typing,
    )
}

fn render_qr_compact(code: &str) -> String {
    QrCode::new(code.as_bytes())
        .map(|qr| qr.render::<unicode::Dense1x2>().quiet_zone(false).build())
        .unwrap_or_default()
}
