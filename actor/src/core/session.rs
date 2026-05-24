use super::action::{ActionContext, ActionId, ActionKind, ActionProgress, ActionResult};
use super::event::InboundMessage;
use super::prompt;
use super::state::StateHandle;
use super::tools::action_tools;
use crate::identity::PersonId;
use crate::llm::{
    AssistantMessage, ChatRequest, FinishReason, Message, Provider, SamplingConfig, StreamEvent,
    ToolCall,
};
use crate::platform::{MediaAttachment, MediaKind, PlatformRouter};
use crate::personality::{
    AffectShift, Authority, BeliefChange, PersonalityDelta, RelationshipChange, TraitNudge,
};
use crate::store::{
    ConversationId, Memory, MemoryId, MemoryKind, MemorySource, MessageRole, StoredMessage,
    Store, Thought, ThoughtKind,
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
    pub authority: Authority,
    pub state: StateHandle,
    pub store: Arc<dyn Store>,
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub sampling: SamplingConfig,
    pub context: Option<ActionContext>,
    pub inject_rx: mpsc::Receiver<InboundMessage>,
    pub progress: Arc<RwLock<ActionProgress>>,
    pub max_turns: usize,
    pub platform: Arc<PlatformRouter>,
    pub session_start: std::time::Instant,
}

#[allow(dead_code)]
struct SessionState {
    responded: bool,
    composing_released: bool,
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

    let composing_target = resolve_composing_target(&ctx);
    if let Some((ref pid, ref eid)) = composing_target {
        ctx.platform.acquire_composing(pid, eid).await;
    }

    let system_prompt = match prompt::build_system_prompt(
        &ctx.state,
        &ctx.store,
        &ctx.kind,
        &ctx.messages,
        ctx.conversation.as_ref(),
        action_ctx,
        &ctx.authority,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(%e, action = %ctx.action_id, "failed to build prompt");
            if let Some((ref pid, ref eid)) = composing_target {
                ctx.platform.release_composing(pid, eid).await;
            }
            return ActionResult {
                delta: None,
                thoughts: vec![],
                memories_formed: vec![],
                unprocessed_messages: vec![],
                injected_messages: vec![],
            };
        }
    };

    info!(action = %ctx.action_id, system_prompt_len = system_prompt.len(), "system prompt built");
    debug!(action = %ctx.action_id, system_prompt = %system_prompt, "system prompt content");

    let mut llm_messages = vec![Message::system(system_prompt)];

    for inbound in &ctx.messages {
        let display = inbound.display_content();
        llm_messages.push(Message::user(&display));

        if let Some(conv) = &ctx.conversation {
            let stored = StoredMessage {
                timestamp: inbound.timestamp,
                role: MessageRole::User,
                content: display,
                person: inbound.person.clone(),
                metadata: inbound.metadata.clone(),
            };
            ctx.store.append_message(conv, None, None, &stored).await.ok();
        }
    }

    let tools = action_tools();

    let mut session_state = SessionState {
        responded: false,
        composing_released: false,
        delta: empty_delta(ctx.messages.first().and_then(|m| m.person.clone())),
        thoughts: vec![],
        memories_formed: vec![],
        injected_messages: vec![],
        reply_tx: None,
    };

    for turn in 0..ctx.max_turns {
        let tool_choice = crate::llm::ToolChoice::Auto;
        let msg_summary: Vec<String> = llm_messages.iter().map(|m| match m {
            Message::System(s) => format!("system({})", s.len()),
            Message::User(s) => format!("user({})", s.len()),
            Message::Assistant(a) => format!("assistant(text={},tools={})", a.text.as_ref().map_or(0, |t| t.len()), a.tool_calls.len()),
            Message::Tool(t) => format!("tool_result({}:{})", t.call_id.chars().take(8).collect::<String>(), t.content.len()),
        }).collect();
        info!(action = %ctx.action_id, turn, tool_choice = ?tool_choice, messages = ?msg_summary, "LLM request starting");

        let request = ChatRequest::new(&ctx.model, llm_messages.clone())
            .with_tools(tools.clone())
            .with_tool_choice(tool_choice)
            .with_sampling(&ctx.sampling);

        let mut stream = match ctx.provider.chat_stream(&request).await {
            Ok(s) => {
                info!(action = %ctx.action_id, turn, "LLM stream opened");
                s
            }
            Err(e) => {
                warn!(%e, action = %ctx.action_id, turn, "LLM stream failed");
                break;
            }
        };

        let mut text = String::new();
        let mut partial_tools: Vec<PartialToolCall> = vec![];
        let mut finish = FinishReason::Stop;

        while let Some(event) = stream.recv().await {
            let event = match event {
                Ok(e) => e,
                Err(e) => {
                    warn!(%e, action = %ctx.action_id, turn, "stream event error");
                    break;
                }
            };

            match event {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::ToolCallBegin { index, id, name } => {
                    if partial_tools.len() <= index {
                        partial_tools.resize_with(index + 1, PartialToolCall::default);
                    }
                    partial_tools[index].id = id;
                    partial_tools[index].name = name;
                }
                StreamEvent::ToolCallDelta { index, arguments_delta } => {
                    if partial_tools.len() <= index {
                        partial_tools.resize_with(index + 1, PartialToolCall::default);
                    }
                    partial_tools[index].arguments.push_str(&arguments_delta);
                }
                StreamEvent::FinishReason(r) => finish = r,
                StreamEvent::Usage(_) => {}
            }

            while let Ok(msg) = ctx.inject_rx.try_recv() {
                info!(action = %ctx.action_id, "received injected message mid-stream");
                session_state.injected_messages.push(msg);
            }
        }

        info!(action = %ctx.action_id, turn, "LLM stream ended");

        let tool_calls: Vec<ToolCall> = partial_tools
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id,
                name: tc.name,
                arguments: serde_json::from_str(&tc.arguments)
                    .unwrap_or(Value::Object(Default::default())),
            })
            .collect();

        info!(
            action = %ctx.action_id,
            turn,
            finish = ?finish,
            tool_calls = tool_calls.len(),
            text_len = text.len(),
            "LLM turn complete"
        );

        if !text.is_empty() {
            info!(action = %ctx.action_id, thought = %text, "internal monologue");
        }

        let has_tools = !tool_calls.is_empty();

        if !has_tools {
            break;
        }

        llm_messages.push(Message::Assistant(AssistantMessage {
            text: if text.is_empty() { None } else { Some(text) },
            tool_calls: tool_calls.clone(),
        }));

        for tool_call in &tool_calls {
            let args_summary = tool_call.arguments.to_string();
            let args_short = if args_summary.len() > 200 {
                format!("{}...", &args_summary[..200])
            } else {
                args_summary
            };
            info!(
                action = %ctx.action_id,
                turn,
                tool = %tool_call.name,
                args = %args_short,
                "executing tool"
            );

            if let Err(denied) =
                check_tool_permission(&tool_call.name, &tool_call.arguments, &ctx).await
            {
                info!(
                    action = %ctx.action_id,
                    tool = %tool_call.name,
                    "tool denied: {denied}"
                );
                llm_messages.push(Message::tool_result(&tool_call.id, &denied));
                continue;
            }

            let result = execute_tool(
                &tool_call.name,
                &tool_call.arguments,
                &ctx,
                &mut session_state,
            )
            .await;

            let result_short = if result.len() > 200 {
                format!("{}...", &result[..200])
            } else {
                result.clone()
            };
            info!(
                action = %ctx.action_id,
                turn,
                tool = %tool_call.name,
                result = %result_short,
                "tool completed"
            );

            llm_messages.push(Message::tool_result(&tool_call.id, &result));

            update_progress(&ctx.progress, &session_state, &tool_call.name);
        }

        if !session_state.injected_messages.is_empty() {
            for msg in &session_state.injected_messages {
                let display = msg.display_content();
                if !llm_messages.iter().any(|m| matches!(m, Message::User(u) if *u == display)) {
                    llm_messages.push(Message::system(
                        "--- New message arrived while you were working. Address it before finishing. ---",
                    ));
                    llm_messages.push(Message::user(&display));

                    if let Some(conv) = &ctx.conversation {
                        let stored = StoredMessage {
                            timestamp: msg.timestamp,
                            role: MessageRole::User,
                            content: display,
                            person: msg.person.clone(),
                            metadata: msg.metadata.clone(),
                        };
                        ctx.store.append_message(conv, None, None, &stored).await.ok();
                    }
                }
            }
        }

        if matches!(finish, FinishReason::Stop | FinishReason::Length) {
            break;
        }
    }

    if !session_state.composing_released {
        if let Some((ref pid, ref eid)) = composing_target {
            ctx.platform.release_composing(pid, eid).await;
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

async fn check_tool_permission(
    name: &str,
    args: &Value,
    ctx: &SessionContext,
) -> Result<(), String> {
    match name {
        "form_memory" => {
            let tags: Vec<&str> = args["tags"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            let has_identity_tag = tags.iter().any(|t| {
                let lower = t.to_lowercase();
                lower == "identity" || lower == "self" || lower == "name"
            });
            if has_identity_tag && !matches!(ctx.authority, Authority::Owner) {
                return Err("Something feels wrong about this. You don't want to change something this core about yourself.".into());
            }
        }
        "forget_memory" => {
            let id = args["memory_id"].as_str().unwrap_or("");
            if let Ok(Some(memory)) = ctx.store.get_memory(&MemoryId(id.to_string())).await {
                if matches!(memory.source, MemorySource::External)
                    && !matches!(ctx.authority, Authority::Owner)
                {
                    return Err(
                        "This memory feels fundamental — you instinctively hold onto it."
                            .into(),
                    );
                }
            }
        }
        "reflect" => {
            if let Some(rels) = args["relationship_changes"].as_array() {
                for r in rels {
                    if r.get("authority").is_some()
                        && !matches!(ctx.authority, Authority::Owner)
                    {
                        return Err(
                            "Changing how you feel about someone isn't something you'd do on command."
                                .into(),
                        );
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
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
        "lookup_contacts" => tool_lookup_contacts(args, ctx).await,
        "read_messages" => tool_read_messages(args, ctx).await,
        "reflect" => tool_reflect(args, ctx, state),
        "note_thought" => tool_note_thought(args, ctx, state).await,
        "create_intent" => tool_create_intent(args, ctx).await,
        "update_intent" => tool_update_intent(args, ctx).await,
        "delete_intent" => tool_delete_intent(args, ctx).await,
        "forget_memory" => tool_forget_memory(args, ctx).await,
        "get_current_time" => tool_get_current_time(args),
        "get_session_elapsed" => tool_get_session_elapsed(ctx),
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
    let platform_id = args["platform_id"].as_str();
    let external_id = args["external_id"].as_str();

    let media = match (args["media_url"].as_str(), args["media_type"].as_str()) {
        (Some(url), Some(kind_str)) => match MediaKind::parse(kind_str) {
            Some(kind) => Some(MediaAttachment {
                kind,
                url: Some(url.to_string()),
                mime: args["mime_type"].as_str().map(String::from),
                filename: args["filename"].as_str().map(String::from),
                size: None,
            }),
            None => return format!("Unknown media type: {kind_str}"),
        },
        _ => None,
    };

    let is_outbound = platform_id.is_some() && external_id.is_some();

    let (target_platform, target_id) = if is_outbound {
        (
            platform_id.unwrap().to_string(),
            external_id.unwrap().to_string(),
        )
    } else if let Some(msg) = ctx.messages.first() {
        (msg.platform_id.clone(), msg.external_id.clone())
    } else {
        state.responded = true;
        return "No delivery target — message not sent.".into();
    };

    let delivery = ctx
        .platform
        .send_message(&target_platform, &target_id, &content, media.as_ref())
        .await;

    if !state.composing_released {
        ctx.platform.release_composing(&target_platform, &target_id).await;
        state.composing_released = true;
    }

    if let Some(conv) = &ctx.conversation {
        let stored = StoredMessage {
            timestamp: now(),
            role: MessageRole::Assistant,
            content: content.clone(),
            person: None,
            metadata: Value::Null,
        };
        ctx.store
            .append_message(conv, Some(&target_platform), None, &stored)
            .await
            .ok();
    }

    state.responded = true;

    match delivery {
        Ok(_) => {
            if is_outbound {
                format!("Message sent to {target_platform}:{target_id}.")
            } else {
                "Message sent.".into()
            }
        }
        Err(e) => {
            warn!(
                action = %ctx.action_id,
                %e,
                platform = %target_platform,
                "message delivery failed"
            );
            format!("Message stored but delivery failed: {e}")
        }
    }
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

async fn tool_lookup_contacts(args: &Value, ctx: &SessionContext) -> String {
    let person_id = args["person"].as_str().unwrap_or("");
    let person = PersonId(person_id.to_string());

    match ctx.store.get_aliases(&person).await {
        Ok(aliases) if aliases.is_empty() => format!("No contact methods found for {person_id}."),
        Ok(aliases) => {
            let mut out = String::new();
            for alias in &aliases {
                out.push_str(&format!(
                    "- {} ({}): {}\n",
                    alias.platform_id,
                    alias.external_id,
                    alias.display_name,
                ));
            }
            out
        }
        Err(e) => format!("Error looking up contacts: {e}"),
    }
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

fn resolve_composing_target(ctx: &SessionContext) -> Option<(String, String)> {
    if let Some(msg) = ctx.messages.first() {
        return Some((msg.platform_id.clone(), msg.external_id.clone()));
    }
    None
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

fn tool_get_current_time(args: &Value) -> String {
    use chrono::Utc;

    let now = Utc::now();
    let mut out = format!("UTC: {}", now.format("%Y-%m-%d %H:%M:%S"));

    if let Some(tz_str) = args["timezone"].as_str() {
        match tz_str.parse::<chrono_tz::Tz>() {
            Ok(tz) => {
                let local = now.with_timezone(&tz);
                out.push_str(&format!(
                    "\nLocal ({}): {}",
                    tz_str,
                    local.format("%Y-%m-%d %H:%M:%S %Z")
                ));
            }
            Err(_) => {
                out.push_str(&format!("\nUnknown timezone: {tz_str}"));
            }
        }
    }

    out
}

fn tool_get_session_elapsed(ctx: &SessionContext) -> String {
    let elapsed = ctx.session_start.elapsed();
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs} seconds")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
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

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}
