use super::action::{ActionContext, ActionId, ActionKind, ActionProgress, ActionResult};
use super::event::InboundMessage;
use super::prompt;
use super::state::StateHandle;
use super::tools::action_tools;
use crate::identity::PersonId;
use crate::llm::{AssistantMessage, ChatRequest, FinishReason, Message, Provider};
use crate::personality::{
    AffectShift, BeliefChange, PersonalityDelta, RelationshipChange, TraitNudge,
};
use crate::store::{
    ConversationId, Memory, MemoryId, MemoryKind, MemorySource, MessageRole, StoredMessage, Store,
    Thought, ThoughtKind,
};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct SessionContext {
    pub action_id: ActionId,
    pub kind: ActionKind,
    pub messages: Vec<InboundMessage>,
    pub conversation: Option<ConversationId>,
    pub state: StateHandle,
    pub store: Arc<dyn Store>,
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub context: Option<ActionContext>,
    pub inject_rx: mpsc::Receiver<InboundMessage>,
    pub progress: Arc<RwLock<ActionProgress>>,
}

#[allow(dead_code)]
struct SessionState {
    responded: bool,
    delta: PersonalityDelta,
    thoughts: Vec<Thought>,
    memories_formed: Vec<MemoryId>,
    injected_messages: Vec<InboundMessage>,
    reply_tx: Option<tokio::sync::mpsc::Sender<OutboundMessage>>,
}

pub struct OutboundMessage {
    pub conversation: ConversationId,
    pub content: String,
    pub person: Option<PersonId>,
}

pub async fn run_session(mut ctx: SessionContext) -> ActionResult {
    let action_ctx = ctx.context.as_ref();

    let system_prompt = match prompt::build_system_prompt(
        &ctx.state,
        &ctx.store,
        &ctx.kind,
        &ctx.messages,
        ctx.conversation.as_ref(),
        action_ctx,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, action = %ctx.action_id, "failed to build prompt");
            return ActionResult {
                delta: None,
                thoughts: vec![],
                memories_formed: vec![],
                unprocessed_messages: vec![],
                injected_messages: vec![],
            };
        }
    };

    let mut llm_messages = vec![Message::system(system_prompt)];

    if let Some(action_ctx) = action_ctx {
        for msg in &action_ctx.recent_messages {
            let llm_msg = match msg.role {
                MessageRole::User => Message::user(&msg.content),
                MessageRole::Assistant => Message::assistant(&msg.content),
                MessageRole::System => Message::system(&msg.content),
                MessageRole::Tool => continue,
            };
            llm_messages.push(llm_msg);
        }

        if !action_ctx.new_messages.is_empty() {
            llm_messages.push(Message::system("--- New messages (triggered this session) ---"));
        }
    }

    for inbound in &ctx.messages {
        llm_messages.push(Message::user(&inbound.content));

        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: inbound.timestamp,
                role: MessageRole::User,
                content: inbound.content.clone(),
                person: inbound.person.clone(),
                metadata: inbound.metadata.clone(),
            };
            ctx.store.append_message(conv, None, &stored).await.ok();
        }
    }

    let tools = action_tools();

    let mut session_state = SessionState {
        responded: false,
        delta: empty_delta(ctx.messages.first().and_then(|m| m.person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        injected_messages: vec![],
        reply_tx: None,
    };

    let max_turns = 10;
    for turn in 0..max_turns {
        while let Ok(msg) = ctx.inject_rx.try_recv() {
            info!(action = %ctx.action_id, "received injected message");
            llm_messages.push(Message::system(
                "--- New message arrived while you were working. Address it before finishing. ---",
            ));
            llm_messages.push(Message::user(&msg.content));

            if let Some(conv) = &ctx.conversation {
                let stored = StoredMessage {
                    timestamp: msg.timestamp,
                    role: MessageRole::User,
                    content: msg.content.clone(),
                    person: msg.person.clone(),
                    metadata: msg.metadata.clone(),
                };
                ctx.store.append_message(conv, None, &stored).await.ok();
            }

            session_state.injected_messages.push(msg);
        }

        let request = ChatRequest::new(&ctx.model, llm_messages.clone())
            .with_tools(tools.clone())
            .with_temperature(0.7);

        let stream = match ctx.provider.chat_stream(&request).await {
            Ok(s) => s,
            Err(e) => {
                warn!(%e, action = %ctx.action_id, turn, "LLM stream failed");
                break;
            }
        };

        let response = match stream.collect().await {
            Ok(r) => r,
            Err(e) => {
                warn!(%e, action = %ctx.action_id, turn, "LLM stream collection failed");
                break;
            }
        };

        debug!(
            action = %ctx.action_id,
            turn,
            finish = ?response.finish_reason,
            tool_calls = response.tool_calls().len(),
            "LLM response"
        );

        if let Some(text) = response.text() {
            if !text.is_empty() {
                info!(action = %ctx.action_id, thought = %text, "internal monologue");
            }
        }

        let has_tools = response.has_tool_calls();
        let finish = response.finish_reason;

        let text = response.message.text.clone();
        let tool_calls = response.message.tool_calls;

        if !has_tools {
            break;
        }

        llm_messages.push(Message::Assistant(AssistantMessage {
            text,
            tool_calls: tool_calls.clone(),
        }));

        for tool_call in &tool_calls {
            let result = execute_tool(
                &tool_call.name,
                &tool_call.arguments,
                &ctx,
                &mut session_state,
            )
            .await;

            llm_messages.push(Message::tool_result(&tool_call.id, &result));

            update_progress(&ctx.progress, &session_state, &tool_call.name);
        }

        if matches!(finish, FinishReason::Stop | FinishReason::Length) {
            break;
        }
    }

    let mut unprocessed = vec![];
    while let Ok(msg) = ctx.inject_rx.try_recv() {
        unprocessed.push(msg);
    }

    ActionResult {
        delta: if has_changes(&session_state.delta) {
            Some(session_state.delta)
        } else {
            None
        },
        thoughts: session_state.thoughts,
        memories_formed: session_state.memories_formed,
        unprocessed_messages: unprocessed,
        injected_messages: session_state.injected_messages,
    }
}

async fn execute_tool(
    name: &str,
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> String {
    match name {
        "recall_memories" => tool_recall_memories(args, ctx).await,
        "form_memory" => tool_form_memory(args, ctx, state).await,
        "send_message" => tool_send_message(args, ctx, state).await,
        "read_messages" => tool_read_messages(args, ctx).await,
        "reflect" => tool_reflect(args, ctx, state),
        "note_thought" => tool_note_thought(args, ctx, state).await,
        "create_intent" => tool_create_intent(args, ctx).await,
        "update_intent" => tool_update_intent(args, ctx).await,
        "delete_intent" => tool_delete_intent(args, ctx).await,
        "forget_memory" => tool_forget_memory(args, ctx).await,
        "start_composing" => tool_start_composing(args, ctx),
        "stop_composing" => tool_stop_composing(args, ctx),
        _ => format!("Unknown tool: {name}"),
    }
}

async fn tool_recall_memories(args: &Value, ctx: &SessionContext) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(3) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

    let recall = crate::store::RecallQuery::by_text(query, limit).with_offset(offset);
    match ctx.store.recall(&recall).await {
        Ok(memories) if memories.is_empty() => "No memories found.".into(),
        Ok(memories) => {
            let mut out = String::new();
            for m in &memories {
                out.push_str(&format!(
                    "[{}] ({}) {}\n",
                    m.id.0,
                    m.kind.as_str(),
                    m.content
                ));
            }
            out
        }
        Err(e) => format!("Error recalling memories: {e}"),
    }
}

async fn tool_form_memory(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let content = args["content"].as_str().unwrap_or("").to_string();
    let kind = args["kind"]
        .as_str()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let importance = args["importance"].as_f64().unwrap_or(0.5) as f32;
    let people: Vec<PersonId> = args["people"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| PersonId(s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let memory = Memory {
        id: MemoryId(format!("mem-{}", uuid_v4())),
        kind,
        content,
        source: ctx
            .conversation
            .as_ref()
            .and_then(|conv| {
                ctx.messages.first().map(|m| MemorySource::Conversation {
                    conversation_id: conv.clone(),
                    person: m.person.clone().unwrap_or(PersonId("unknown".into())),
                })
            })
            .unwrap_or(MemorySource::Reflection),
        importance,
        sensitivity: 0.0,
        emotional_valence: 0.0,
        created_at: now(),
        accessed_at: now(),
        access_count: 0,
        tags: vec![],
        people,
        embedding: None,
    };

    match ctx.store.store_memory(&memory).await {
        Ok(id) => {
            state.memories_formed.push(id.clone());
            format!("Memory saved: {}", id.0)
        }
        Err(e) => format!("Failed to save memory: {e}"),
    }
}

async fn tool_send_message(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> String {
    let content = args["content"].as_str().unwrap_or("").to_string();

    if let Some(conv) = &ctx.conversation {
        let stored = StoredMessage {
            timestamp: now(),
            role: MessageRole::Assistant,
            content: content.clone(),
            person: None,
            metadata: Value::Null,
        };
        ctx.store.append_message(conv, None, &stored).await.ok();
    }

    state.responded = true;

    // TODO: send through communication layer via reply_tx channel
    info!(
        action = %ctx.action_id,
        conversation = ?ctx.conversation,
        content_len = content.len(),
        "send_message"
    );

    "Message sent.".into()
}

fn tool_reflect(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    if let Some(nudges) = args["trait_nudges"].as_array() {
        for nudge in nudges {
            if let (Some(name), Some(dir)) = (nudge["trait_name"].as_str(), nudge["direction"].as_f64()) {
                state.delta.trait_nudges.push(TraitNudge {
                    trait_name: name.to_string(),
                    direction: dir as f32,
                    reason: nudge["reason"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }

    if let Some(beliefs) = args["belief_changes"].as_array() {
        for b in beliefs {
            state.delta.belief_changes.push(BeliefChange {
                topic: b["topic"].as_str().unwrap_or("").to_string(),
                new_stance: b["new_stance"].as_str().map(String::from),
                confidence_delta: b["confidence_delta"].as_f64().unwrap_or(0.0) as f32,
                reason: b["reason"].as_str().unwrap_or("").to_string(),
                about: b["about_person"]
                    .as_str()
                    .map(|s| PersonId(s.to_string())),
            });
        }
    }

    if let Some(rels) = args["relationship_changes"].as_array() {
        for r in rels {
            if let Some(person) = r["person"].as_str() {
                state.delta.relationship_changes.push(RelationshipChange {
                    person: PersonId(person.to_string()),
                    trust_delta: r["trust_delta"].as_f64().unwrap_or(0.0) as f32,
                    familiarity_delta: r["familiarity_delta"].as_f64().unwrap_or(0.0) as f32,
                    valence_delta: r["valence_delta"].as_f64().unwrap_or(0.0) as f32,
                });
            }
        }
    }

    if let Some(interests) = args["new_interests"].as_array() {
        for i in interests {
            if let Some(topic) = i.as_str() {
                state.delta.new_interests.push(topic.to_string());
            }
        }
    }

    if let Some(affect) = args.get("affect_shift") {
        state.delta.affect_shift = AffectShift {
            valence: affect["valence"].as_f64().unwrap_or(0.0) as f32,
            arousal: affect["arousal"].as_f64().unwrap_or(0.0) as f32,
            dominance: affect["dominance"].as_f64().unwrap_or(0.0) as f32,
        };
    }

    if let Some(note) = args["growth_note"].as_str() {
        state.delta.growth_note = Some(note.to_string());
    }

    info!(action = %ctx.action_id, "reflection applied");
    "Reflection recorded.".into()
}

async fn tool_note_thought(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
) -> String {
    let kind = args["kind"]
        .as_str()
        .and_then(ThoughtKind::parse)
        .unwrap_or(ThoughtKind::Observation);
    let content = args["content"].as_str().unwrap_or("").to_string();

    let thought = Thought {
        timestamp: now(),
        kind,
        content: content.clone(),
        memories_accessed: vec![],
        people: ctx
            .messages
            .iter()
            .filter_map(|m| m.person.clone())
            .collect(),
    };

    ctx.store.log_thought(&thought).await.ok();
    state.thoughts.push(thought);

    "Thought noted.".into()
}

async fn tool_create_intent(args: &Value, _ctx: &SessionContext) -> String {
    let task = args["task"].as_str().unwrap_or("");
    let kind = args["kind"].as_str().unwrap_or("scheduled");

    // TODO: persist intent to store (intent table not yet built)
    info!(task, kind, "intent created (stub)");

    format!("Intent created: {task}")
}

async fn tool_read_messages(args: &Value, ctx: &SessionContext) -> String {
    let conv = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| ctx.conversation.clone());

    let Some(conv) = conv else {
        return "No conversation specified and no current conversation.".into();
    };

    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    let before = args["before"].as_i64();

    match ctx.store.get_messages(&conv, limit, before).await {
        Ok(messages) if messages.is_empty() => "No messages found.".into(),
        Ok(messages) => {
            let mut out = String::new();
            for m in &messages {
                let who = m
                    .person
                    .as_ref()
                    .map_or("unknown", |p| p.0.as_str());
                out.push_str(&format!(
                    "[{}] {}: {}\n",
                    m.timestamp,
                    if matches!(m.role, MessageRole::Assistant) {
                        "you"
                    } else {
                        who
                    },
                    m.content
                ));
            }
            out
        }
        Err(e) => format!("Error reading messages: {e}"),
    }
}

async fn tool_forget_memory(args: &Value, ctx: &SessionContext) -> String {
    let id = args["memory_id"].as_str().unwrap_or("");
    match ctx.store.forget(&MemoryId(id.to_string())).await {
        Ok(true) => "Memory forgotten.".into(),
        Ok(false) => "Memory not found.".into(),
        Err(e) => format!("Error: {e}"),
    }
}

fn tool_start_composing(args: &Value, ctx: &SessionContext) -> String {
    let conversation = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| ctx.conversation.clone());

    // TODO: route through communication layer
    info!(
        action = %ctx.action_id,
        conversation = ?conversation,
        "start_composing"
    );

    "Composing signal sent.".into()
}

fn tool_stop_composing(args: &Value, ctx: &SessionContext) -> String {
    let conversation = args["conversation"]
        .as_str()
        .map(|s| ConversationId(s.to_string()))
        .or_else(|| ctx.conversation.clone());

    // TODO: route through communication layer
    info!(
        action = %ctx.action_id,
        conversation = ?conversation,
        "stop_composing"
    );

    "Composing signal cleared.".into()
}

async fn tool_delete_intent(args: &Value, _ctx: &SessionContext) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");

    // TODO: persist to store (intent table not yet built)
    info!(intent_id = id, "intent deleted (stub)");

    format!("Intent {id} deleted.")
}

async fn tool_update_intent(args: &Value, _ctx: &SessionContext) -> String {
    let id = args["intent_id"].as_str().unwrap_or("");
    let task = args["task"].as_str();
    let kind = args["kind"].as_str();

    // TODO: persist to store (intent table not yet built)
    info!(
        intent_id = id,
        new_task = ?task,
        new_kind = ?kind,
        "intent updated (stub)"
    );

    format!("Intent {id} updated.")
}

fn update_progress(
    progress: &Arc<RwLock<ActionProgress>>,
    state: &SessionState,
    tool_name: &str,
) {
    if let Ok(mut p) = progress.write() {
        p.responded = state.responded;
        p.thoughts_count = state.thoughts.len();
        p.memories_formed = state.memories_formed.len();
        p.last_activity = tool_name.to_string();
    }
}

fn empty_delta(triggered_by: Option<PersonId>) -> PersonalityDelta {
    PersonalityDelta {
        trait_nudges: vec![],
        belief_changes: vec![],
        relationship_changes: vec![],
        new_interests: vec![],
        affect_shift: AffectShift::default(),
        growth_note: None,
        triggered_by,
    }
}

fn has_changes(delta: &PersonalityDelta) -> bool {
    !delta.trait_nudges.is_empty()
        || !delta.belief_changes.is_empty()
        || !delta.relationship_changes.is_empty()
        || !delta.new_interests.is_empty()
        || delta.growth_note.is_some()
        || delta.affect_shift.valence != 0.0
        || delta.affect_shift.arousal != 0.0
        || delta.affect_shift.dominance != 0.0
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:032x}", t)
}
