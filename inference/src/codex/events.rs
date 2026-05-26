use crate::{FinishReason, StreamEvent, Usage};
use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;

pub(super) struct AppServerEventState {
    emit_agent_text: bool,
    final_text: String,
    agent_items: HashMap<String, AgentMessagePhase>,
    saw_text: bool,
    saw_usage: bool,
    completed: bool,
}

impl AppServerEventState {
    pub(super) fn new(emit_agent_text: bool) -> Self {
        Self {
            emit_agent_text,
            final_text: String::new(),
            agent_items: HashMap::new(),
            saw_text: false,
            saw_usage: false,
            completed: false,
        }
    }

    #[cfg(test)]
    pub(super) fn final_text(&self) -> &str {
        &self.final_text
    }

    pub(super) fn completed(&self) -> bool {
        self.completed
    }
}

pub(super) async fn handle_notification(
    notification: AppServerNotification,
    tx: &mpsc::Sender<anyhow::Result<StreamEvent>>,
    state: &mut AppServerEventState,
) -> anyhow::Result<()> {
    match notification {
        AppServerNotification::ItemStarted(params) => {
            if let ThreadItem::AgentMessage { id, phase, .. } = params.item {
                state.agent_items.insert(id, phase);
            }
        }
        AppServerNotification::AgentMessageDelta(params) => {
            if state
                .agent_items
                .get(&params.item_id)
                .is_none_or(AgentMessagePhase::is_final)
            {
                state.final_text.push_str(&params.delta);
                if state.emit_agent_text {
                    state.saw_text = true;
                    tx.send(Ok(StreamEvent::TextDelta(params.delta))).await?;
                }
            }
        }
        AppServerNotification::ReasoningTextDelta(params) => {
            if !params.delta.is_empty() {
                tx.send(Ok(StreamEvent::ReasoningDelta(params.delta)))
                    .await?;
            }
        }
        AppServerNotification::ItemCompleted(params) => {
            if let ThreadItem::AgentMessage {
                id, text, phase, ..
            } = params.item
            {
                state.agent_items.insert(id, phase.clone());
                if phase.is_final() && !text.is_empty() && state.final_text.is_empty() {
                    state.final_text = text.clone();
                    if state.emit_agent_text && !state.saw_text {
                        state.saw_text = true;
                        tx.send(Ok(StreamEvent::TextDelta(text))).await?;
                    }
                }
            }
        }
        AppServerNotification::ThreadTokenUsageUpdated(params) => {
            state.saw_usage = true;
            tx.send(Ok(StreamEvent::Usage(Usage {
                input_tokens: clamp_usage(params.token_usage.last.input_tokens),
                output_tokens: clamp_usage(params.token_usage.last.output_tokens),
            })))
            .await?;
        }
        AppServerNotification::TurnCompleted(params) => {
            state.completed = true;
            if params.turn.status != TurnStatus::Completed {
                anyhow::bail!(
                    "codex app-server turn ended with status {}",
                    params.turn.status.as_str()
                );
            }
            if !state.saw_usage {
                tx.send(Ok(StreamEvent::Usage(Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                })))
                .await?;
            }
        }
        AppServerNotification::Error(params) => {
            anyhow::bail!("codex app-server error: {}", params.error.message);
        }
        AppServerNotification::Other => {}
    }
    Ok(())
}

pub(super) async fn send_finish_reason(
    tx: &mpsc::Sender<anyhow::Result<StreamEvent>>,
    finish_reason: FinishReason,
) -> anyhow::Result<()> {
    tx.send(Ok(StreamEvent::FinishReason(finish_reason)))
        .await?;
    Ok(())
}

fn clamp_usage(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(if value < 0 { 0 } else { u32::MAX })
}

pub(super) enum AppServerNotification {
    ItemStarted(ItemNotification),
    ItemCompleted(ItemNotification),
    AgentMessageDelta(AgentMessageDelta),
    ReasoningTextDelta(TextDelta),
    ThreadTokenUsageUpdated(TokenUsageNotification),
    TurnCompleted(TurnCompletedNotification),
    Error(ErrorNotification),
    Other,
}

#[derive(Deserialize)]
pub(super) struct AgentMessageDelta {
    #[serde(rename = "itemId")]
    item_id: String,
    delta: String,
}

#[derive(Deserialize)]
pub(super) struct TextDelta {
    delta: String,
}

#[derive(Deserialize)]
pub(super) struct ItemNotification {
    item: ThreadItem,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum AgentMessagePhase {
    FinalAnswer,
    Commentary,
    #[serde(other)]
    Other,
}

impl AgentMessagePhase {
    fn is_final(&self) -> bool {
        matches!(self, Self::FinalAnswer)
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum ThreadItem {
    #[serde(rename = "agentMessage")]
    AgentMessage {
        id: String,
        #[serde(default)]
        text: String,
        #[serde(default = "default_agent_message_phase")]
        phase: AgentMessagePhase,
    },
    #[serde(other)]
    Other,
}

fn default_agent_message_phase() -> AgentMessagePhase {
    AgentMessagePhase::FinalAnswer
}

#[derive(Deserialize)]
pub(super) struct TokenUsageNotification {
    #[serde(rename = "tokenUsage")]
    token_usage: ThreadTokenUsage,
}

#[derive(Deserialize)]
struct ThreadTokenUsage {
    last: TokenUsageBreakdown,
}

#[derive(Deserialize)]
struct TokenUsageBreakdown {
    #[serde(rename = "inputTokens")]
    input_tokens: i64,
    #[serde(rename = "outputTokens")]
    output_tokens: i64,
}

#[derive(Deserialize)]
pub(super) struct TurnCompletedNotification {
    turn: Turn,
}

#[derive(Deserialize)]
struct Turn {
    status: TurnStatus,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum TurnStatus {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    InProgress,
    #[serde(other)]
    Other,
}

impl TurnStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::InProgress => "inProgress",
            Self::Other => "unknown",
        }
    }
}

#[derive(Deserialize)]
pub(super) struct ErrorNotification {
    error: AppServerError,
}

#[derive(Deserialize)]
struct AppServerError {
    message: String,
}

pub(super) fn parse_notification(
    value: serde_json::Value,
) -> anyhow::Result<AppServerNotification> {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .context("codex app-server notification missing method")?;
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "item/started" => parse_params(params, method).map(AppServerNotification::ItemStarted),
        "item/completed" => parse_params(params, method).map(AppServerNotification::ItemCompleted),
        "item/agentMessage/delta" => {
            parse_params(params, method).map(AppServerNotification::AgentMessageDelta)
        }
        "item/reasoning/textDelta" => {
            parse_params(params, method).map(AppServerNotification::ReasoningTextDelta)
        }
        "thread/tokenUsage/updated" => {
            parse_params(params, method).map(AppServerNotification::ThreadTokenUsageUpdated)
        }
        "turn/completed" => parse_params(params, method).map(AppServerNotification::TurnCompleted),
        "error" => parse_params(params, method).map(AppServerNotification::Error),
        _ => Ok(AppServerNotification::Other),
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(params: Value, method: &str) -> anyhow::Result<T> {
    serde_json::from_value(params)
        .with_context(|| format!("failed to parse codex app-server notification params: {method}"))
}
