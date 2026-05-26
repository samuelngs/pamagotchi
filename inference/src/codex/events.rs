use crate::{FinishReason, StreamEvent, Usage};
use serde::Deserialize;
use tokio::sync::mpsc;

pub(super) async fn handle_event(
    event: CodexEvent,
    tx: &mpsc::Sender<anyhow::Result<StreamEvent>>,
    saw_text: &mut bool,
    failed: &mut bool,
) -> anyhow::Result<()> {
    match event {
        CodexEvent::ItemCompleted { item } => match item.details {
            CodexItemDetails::AgentMessage { text } => {
                if !text.is_empty() {
                    *saw_text = true;
                    tx.send(Ok(StreamEvent::TextDelta(text))).await?;
                }
            }
            CodexItemDetails::Reasoning { text } => {
                if !text.is_empty() {
                    tx.send(Ok(StreamEvent::ReasoningDelta(text))).await?;
                }
            }
            CodexItemDetails::Other => {}
        },
        CodexEvent::TurnCompleted { usage } => {
            tx.send(Ok(StreamEvent::Usage(Usage {
                input_tokens: clamp_usage(usage.input_tokens),
                output_tokens: clamp_usage(usage.output_tokens),
            })))
            .await?;
            tx.send(Ok(StreamEvent::FinishReason(FinishReason::Stop)))
                .await?;
        }
        CodexEvent::TurnFailed { error } | CodexEvent::Error(error) => {
            *failed = true;
            tx.send(Err(anyhow::anyhow!("codex exec error: {}", error.message)))
                .await?;
        }
        CodexEvent::Other => {}
    }
    Ok(())
}

fn clamp_usage(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(if value < 0 { 0 } else { u32::MAX })
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum CodexEvent {
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: CodexUsage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: CodexError },
    #[serde(rename = "error")]
    Error(CodexError),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub(super) struct CodexItem {
    #[serde(flatten)]
    pub(super) details: CodexItemDetails,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum CodexItemDetails {
    AgentMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub(super) struct CodexUsage {
    input_tokens: i64,
    output_tokens: i64,
}

#[derive(Deserialize)]
pub(super) struct CodexError {
    pub(super) message: String,
}
