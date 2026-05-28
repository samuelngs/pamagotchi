use super::super::context::{SessionContext, SessionState};
use super::helpers::{canonicalize_content_for_subjects, string_array};
use crate::store::{
    Memory, MemoryKind, MemorySource, MemoryStability, MemorySubject, MemoryType, PrivacyCategory,
    TruthStatus, VisibilityScope, memory_privacy_policy_for_subject, memory_stability_policy,
    memory_truth_status_policy, sensitive_memory_next_review_at,
};
use protocol::{IdentityId, InboundMessage, MemoryId, ProfileId};
use serde_json::{Value, json};

pub async fn form(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let raw_content = args["content"].as_str().unwrap_or("").to_string();
    let kind = args["kind"]
        .as_str()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let importance = clamp_unit(args["importance"].as_f64().unwrap_or(0.5) as f32);
    let sensitivity = clamp_unit(args["sensitivity"].as_f64().unwrap_or(0.0) as f32);
    let emotional_valence = clamp_valence(args["emotional_valence"].as_f64().unwrap_or(0.0) as f32);
    let tags = string_array(&args["tags"]).collect::<Vec<_>>();
    let confidence = clamp_unit(args["confidence"].as_f64().unwrap_or(1.0) as f32);
    let memory_type = args["memory_type"]
        .as_str()
        .and_then(MemoryType::parse)
        .unwrap_or_default();
    let explicit_truth_status = args["truth_status"].as_str().and_then(TruthStatus::parse);
    let truth_status = memory_truth_status_policy(&memory_type, explicit_truth_status);
    let evidence_source_messages = evidence_source_messages(ctx, state);
    let explicit_evidence_message_ids =
        string_array(&args["evidence_message_ids"]).collect::<Vec<_>>();
    let evidence_messages =
        match selected_evidence_messages(&evidence_source_messages, &explicit_evidence_message_ids)
        {
            Ok(messages) => messages,
            Err(missing) => {
                return json!({
                    "error": format!(
                        "Evidence message ids are not available in the current action: {}",
                        missing.join(", ")
                    ),
                })
                .to_string();
            }
        };
    let mut evidence_message_ids = explicit_evidence_message_ids;
    if evidence_message_ids.is_empty() {
        evidence_message_ids = evidence_messages
            .iter()
            .map(|message| message.message_id.clone())
            .filter(|id| !id.is_empty())
            .collect();
    }
    let evidence_quote = args["evidence_quote"].as_str().map(str::to_string);
    let evidence = super::super::util::evidence_with_source_spans(
        args,
        serde_json::Value::Object(Default::default()),
    );
    let expires_at = args["expires_at"].as_i64();
    let explicit_stability = args["stability"].as_str().and_then(MemoryStability::parse);
    let stability = memory_stability_policy(&memory_type, &truth_status, explicit_stability);
    let subject_actor = args["subject_actor"].as_bool().unwrap_or(false);
    let sensitivity_category = args["sensitivity_category"].as_str().map(str::to_string);
    let explicit_privacy_category = args["privacy_category"]
        .as_str()
        .and_then(PrivacyCategory::parse);
    let explicit_visibility_scope = args["visibility_scope"]
        .as_str()
        .and_then(VisibilityScope::parse);
    let (privacy_category, visibility_scope) = memory_privacy_policy_for_subject(
        sensitivity,
        sensitivity_category.as_deref(),
        explicit_privacy_category,
        explicit_visibility_scope,
        subject_actor,
    );
    let dedupe_key = args["dedupe_key"].as_str().map(str::to_string);
    let supersedes = args["supersedes"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| MemoryId(id.to_string()));
    let contradiction_group = args["contradiction_group"]
        .as_str()
        .filter(|group| !group.trim().is_empty())
        .map(str::to_string);
    let last_confirmed_at = args["last_confirmed_at"].as_i64();
    let explicit_next_review_at = args["next_review_at"].as_i64();

    let explicit_profile_ids = string_array(&args["subject_profile_ids"]).collect::<Vec<_>>();
    let explicit_person_ids = string_array(&args["subject_person_ids"]).collect::<Vec<_>>();
    let explicit_identity_ids = string_array(&args["subject_identity_ids"]).collect::<Vec<_>>();

    if !explicit_person_ids.is_empty() {
        return json!({
            "error": "form_memory no longer writes directly to person subjects. Save to the current profile first, then use promote_profile_memory_to_person after verification."
        })
        .to_string();
    }
    if subject_actor
        && (!explicit_profile_ids.is_empty()
            || !explicit_identity_ids.is_empty()
            || !explicit_person_ids.is_empty())
    {
        return json!({
            "error": "Actor self memories cannot be mixed with profile, identity, or person subjects."
        })
        .to_string();
    }

    let source_message = source_message_for_evidence(&evidence_messages, &evidence_message_ids);

    let allowed_profile_ids = evidence_messages
        .iter()
        .filter_map(|message| message.profile.as_ref().map(|id| id.0.as_str()))
        .collect::<Vec<_>>();
    if !allowed_profile_ids.is_empty() {
        if explicit_profile_ids
            .iter()
            .any(|id| !allowed_profile_ids.contains(&id.as_str()))
        {
            return json!({
                "error": "Refusing to save memory to a profile outside the current action messages."
            })
            .to_string();
        }
    }
    let allowed_identity_ids = evidence_messages
        .iter()
        .filter_map(|message| message.identity.as_ref().map(|id| id.0.as_str()))
        .collect::<Vec<_>>();
    if !allowed_identity_ids.is_empty() {
        if explicit_identity_ids
            .iter()
            .any(|id| !allowed_identity_ids.contains(&id.as_str()))
        {
            return json!({
                "error": "Refusing to save memory to an identity outside the current action messages."
            })
            .to_string();
        }
    }

    let mut subjects: Vec<MemorySubject> = if subject_actor {
        vec![MemorySubject::actor(Some("self".into()), 1.0)]
    } else {
        explicit_identity_ids
            .into_iter()
            .map(|id| MemorySubject::identity(IdentityId(id), Some("about".into()), 1.0))
            .collect()
    };
    if !subject_actor {
        subjects.extend(
            explicit_profile_ids
                .into_iter()
                .map(|id| MemorySubject::profile(ProfileId(id), Some("about".into()), 1.0)),
        );
    }
    if subjects.is_empty() {
        if let Some(profile) = source_message.and_then(|message| message.profile.clone()) {
            subjects.push(MemorySubject::profile(profile, Some("about".into()), 1.0));
        } else if let Some(identity) = source_message.and_then(|message| message.identity.clone()) {
            subjects.push(MemorySubject::identity(identity, Some("about".into()), 1.0));
        }
    }

    let content = canonicalize_content_for_subjects(&raw_content, &subjects, ctx).await;
    let source_conversation = source_message
        .map(|m| m.conversation.clone())
        .or_else(|| ctx.conversation.clone())
        .or_else(|| ctx.messages.first().map(|m| m.conversation.clone()));

    let embedding_result = ctx.router.embed_with_metadata(&[&content]).await.ok();
    let embedding_model = embedding_result.as_ref().map(|result| result.model.clone());
    let embedding = embedding_result.and_then(|mut result| result.embeddings.pop());

    let now = super::super::util::now();
    let next_review_at =
        sensitive_memory_next_review_at(now, &privacy_category, explicit_next_review_at);
    let memory = Memory {
        id: MemoryId(format!("mem-{}", super::super::util::uuid_v4())),
        kind,
        memory_type,
        truth_status,
        content,
        source: source_conversation
            .as_ref()
            .map(|conv| MemorySource::Conversation {
                conversation_id: conv.clone(),
                identity_id: source_message.and_then(|m| m.identity.clone()),
                profile_id: source_message.and_then(|m| m.profile.clone()),
                person_id: source_message.and_then(|m| m.person.clone()),
                message_id: evidence_message_ids
                    .first()
                    .cloned()
                    .or_else(|| source_message.map(|m| m.message_id.clone())),
            })
            .unwrap_or(MemorySource::Reflection),
        importance,
        confidence,
        sensitivity,
        sensitivity_category,
        emotional_valence,
        created_at: now,
        accessed_at: now,
        access_count: 0,
        tags,
        subjects,
        evidence_message_ids,
        evidence_quote,
        evidence,
        expires_at,
        stability,
        supersedes: supersedes.clone(),
        superseded_by: None,
        contradiction_group,
        privacy_category,
        visibility_scope,
        last_confirmed_at,
        next_review_at,
        dedupe_key,
        embedding_model,
        embedding_version: None,
        embedding,
    };

    match ctx.store.store_memory(&memory).await {
        Ok(id) => {
            if id == memory.id {
                ctx.metrics.record_memory_created();
            } else {
                ctx.metrics.record_memory_updated();
            }
            if let Some(superseded) = supersedes.filter(|superseded| superseded != &id) {
                if let Err(e) = ctx
                    .store
                    .update_memory(
                        &superseded,
                        &crate::store::MemoryUpdate {
                            truth_status: Some(TruthStatus::Outdated),
                            superseded_by: Some(id.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                {
                    return format!(
                        "Memory saved: {}, but failed to link superseded memory: {e}",
                        id.0
                    );
                }
                ctx.metrics.record_memory_updated();
                ctx.metrics.record_memory_superseded();
            }
            state.memories_formed.push(id.clone());
            format!("Memory saved: {}", id.0)
        }
        Err(e) => format!("Failed to save memory: {e}"),
    }
}

fn evidence_source_messages(ctx: &SessionContext, state: &SessionState) -> Vec<InboundMessage> {
    ctx.messages
        .iter()
        .chain(state.presented_injected_messages.iter())
        .chain(state.presented_read_messages.iter())
        .cloned()
        .collect()
}

fn selected_evidence_messages(
    messages: &[InboundMessage],
    evidence_message_ids: &[String],
) -> Result<Vec<InboundMessage>, Vec<String>> {
    if evidence_message_ids.is_empty() {
        return Ok(messages.to_vec());
    }

    let mut selected = Vec::new();
    let mut missing = Vec::new();
    for id in evidence_message_ids {
        match messages.iter().find(|message| message.message_id == *id) {
            Some(message) => selected.push(message.clone()),
            None => missing.push(id.clone()),
        }
    }
    if missing.is_empty() {
        Ok(selected)
    } else {
        Err(missing)
    }
}

fn source_message_for_evidence<'a>(
    messages: &'a [InboundMessage],
    evidence_message_ids: &[String],
) -> Option<&'a InboundMessage> {
    evidence_message_ids
        .iter()
        .find_map(|id| messages.iter().find(|message| message.message_id == *id))
        .or_else(|| messages.first())
}

fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn clamp_valence(value: f32) -> f32 {
    value.clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::SessionKind;
    use crate::state::{ActorState, Authority, Delta, GrowthConfig};
    use crate::store::{MemorySubjectType, RecallQuery, SqliteStore, Store};
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{ConversationId, InboundMessage};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;
    struct EmbeddingBridge;

    #[async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(ChatResponse {
                message: inference::AssistantMessage {
                    text: Some(String::new()),
                    reasoning_content: None,
                    tool_calls: vec![],
                },
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("noop bridge is not used by form_memory tests")
        }

        async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedding endpoint unavailable")
        }
    }

    #[async_trait]
    impl OpenAiCompatibleBridge for EmbeddingBridge {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            anyhow::bail!("embedding bridge is not used for chat")
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> anyhow::Result<ChatStream> {
            anyhow::bail!("embedding bridge is not used for streaming")
        }

        async fn embed(&self, model: &str, input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            assert_eq!(model, "embed-test");
            assert_eq!(input.len(), 1);
            Ok(vec![vec![0.1, 0.2, 0.3, 0.4]])
        }
    }

    fn router_with_failing_embedding_endpoint() -> inference::InferenceRouter {
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "chat-noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "embed-unavailable".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Embedding],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap()
    }

    #[test]
    fn memory_numeric_fields_are_clamped() {
        assert_eq!(clamp_unit(-0.2), 0.0);
        assert_eq!(clamp_unit(1.2), 1.0);
        assert_eq!(clamp_valence(-1.2), -1.0);
        assert_eq!(clamp_valence(1.2), 1.0);
    }

    #[tokio::test]
    async fn form_memory_rejects_unavailable_explicit_evidence_message_ids() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let profile = ProfileId("profile-sam".into());
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let ctx = SessionContext {
            action_id: ActionId("form-memory-missing-evidence-test".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![InboundMessage {
                message_id: "msg-present".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: conversation.clone(),
                group: None,
                identity: None,
                profile: Some(profile),
                person: None,
                content: "I prefer concise launch briefs.".into(),
                attachments: vec![],
                timestamp: 1000,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(conversation),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "Sam prefers concise launch briefs.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "evidence_message_ids": ["msg-missing"]
            }),
            &ctx,
            &mut state,
        )
        .await;
        let value: Value = serde_json::from_str(&result).unwrap();

        assert!(value["error"].as_str().unwrap().contains("not available"));
        assert!(state.memories_formed.is_empty());
        assert!(
            store
                .recall(&RecallQuery::by_text("concise launch briefs", 5))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn form_memory_accepts_read_message_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let profile = ProfileId("profile-sam".into());
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let ctx = SessionContext {
            action_id: ActionId("form-memory-read-evidence-test".into()),
            kind: SessionKind::Action(ActionKind::Consolidate),
            messages: vec![],
            conversation: Some(conversation.clone()),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let read_message = InboundMessage {
            message_id: "msg-read".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: conversation.clone(),
            group: None,
            identity: None,
            profile: Some(profile.clone()),
            person: None,
            content: "I prefer concise rollout notes.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![read_message],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "Sam prefers concise rollout notes.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "evidence_message_ids": ["msg-read"]
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(state.memories_formed.last().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.evidence_message_ids, vec!["msg-read"]);
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, profile.0);
        match memory.source {
            MemorySource::Conversation { message_id, .. } => {
                assert_eq!(message_id.as_deref(), Some("msg-read"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn form_memory_defaults_uncertain_and_emotional_memories_to_transient() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let profile = ProfileId("profile-sam".into());
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let ctx = SessionContext {
            action_id: ActionId("form-memory-transient-defaults-test".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![InboundMessage {
                message_id: "msg-1".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: conversation.clone(),
                group: None,
                identity: None,
                profile: Some(profile),
                person: None,
                content: "I might be annoyed about launch today.".into(),
                attachments: vec![],
                timestamp: 1000,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(conversation),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "Sam might be annoyed about launch today.",
                "kind": "semantic",
                "memory_type": "hypothesis",
                "stability": "stable",
                "evidence_message_ids": ["msg-1"],
                "dedupe_key": "hypothesis:profile-sam:annoyed-launch"
            }),
            &ctx,
            &mut state,
        )
        .await;
        assert!(result.starts_with("Memory saved: "));
        let hypothesis = store
            .get_memory(state.memories_formed.last().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hypothesis.memory_type, MemoryType::Hypothesis);
        assert_eq!(hypothesis.truth_status, TruthStatus::Inferred);
        assert_eq!(hypothesis.stability, MemoryStability::Transient);

        let result = form(
            &json!({
                "content": "Sam feels annoyed about launch today.",
                "kind": "episodic",
                "memory_type": "emotional_state",
                "truth_status": "stated",
                "stability": "stable",
                "evidence_message_ids": ["msg-1"],
                "dedupe_key": "emotion:profile-sam:annoyed-launch"
            }),
            &ctx,
            &mut state,
        )
        .await;
        assert!(result.starts_with("Memory saved: "));
        let emotion = store
            .get_memory(state.memories_formed.last().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(emotion.memory_type, MemoryType::EmotionalState);
        assert_eq!(emotion.truth_status, TruthStatus::Stated);
        assert_eq!(emotion.stability, MemoryStability::Transient);
    }

    #[tokio::test]
    async fn form_memory_defaults_evidence_to_current_action_messages() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let profile = ProfileId("profile-sam".into());
        let identity = IdentityId("identity-local".into());
        let other_profile = ProfileId("profile-alice".into());
        let other_identity = IdentityId("identity-alice".into());
        store
            .store_memory(&Memory {
                id: MemoryId("old-memory".into()),
                kind: MemoryKind::Semantic,
                content: "Sam prefers long deployment updates.".into(),
                source: MemorySource::Reflection,
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        let messages = vec![
            InboundMessage {
                message_id: "msg-1".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: conversation.clone(),
                group: None,
                identity: Some(identity.clone()),
                profile: Some(profile.clone()),
                person: None,
                content: "I prefer short deploy updates.".into(),
                attachments: vec![],
                timestamp: 1000,
                metadata: serde_json::Value::Null,
            },
            InboundMessage {
                message_id: "msg-2".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: conversation.clone(),
                group: None,
                identity: Some(other_identity.clone()),
                profile: Some(other_profile.clone()),
                person: None,
                content: "Actually, concise is best.".into(),
                attachments: vec![],
                timestamp: 1001,
                metadata: serde_json::Value::Null,
            },
        ];
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(EmbeddingBridge)),
                model: "embed-test".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Embedding],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();
        let ctx = SessionContext {
            action_id: ActionId("form-memory-test".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages,
            conversation: Some(conversation.clone()),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "Sam prefers concise deployment updates.",
                "kind": "semantic",
                "memory_type": "correction",
                "truth_status": "confirmed",
                "tags": ["preference", "deployment"],
                "confidence": 0.87,
                "emotional_valence": 0.2,
                "supersedes": "old-memory",
                "contradiction_group": "deploy-update-length",
                "evidence_quote": "I prefer short deploy updates.",
                "source_spans": [{
                    "message_id": "msg-1",
                    "start_char": 0,
                    "end_char": 30,
                    "quote": "I prefer short deploy updates."
                }],
                "evidence": {"reason": "user corrected preference"},
                "expires_at": 3234,
                "stability": "stable",
                "last_confirmed_at": 1234,
                "next_review_at": 2234,
                "dedupe_key": "correction:profile-sam:concise-deployment-updates"
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.evidence_message_ids, vec!["msg-1", "msg-2"]);
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, "profile-sam");
        assert_eq!(memory.memory_type, MemoryType::Correction);
        assert_eq!(memory.truth_status, TruthStatus::Confirmed);
        assert_eq!(memory.tags, vec!["preference", "deployment"]);
        assert_eq!(memory.confidence, 0.87);
        assert_eq!(memory.emotional_valence, 0.2);
        assert_eq!(
            memory.supersedes.as_ref().map(|id| id.0.as_str()),
            Some("old-memory")
        );
        assert_eq!(
            memory.contradiction_group.as_deref(),
            Some("deploy-update-length")
        );
        assert_eq!(memory.evidence["reason"], "user corrected preference");
        assert_eq!(memory.evidence["source_spans"][0]["message_id"], "msg-1");
        assert_eq!(
            memory.evidence["source_spans"][0]["quote"],
            "I prefer short deploy updates."
        );
        assert_eq!(
            memory.evidence_quote.as_deref(),
            Some("I prefer short deploy updates.")
        );
        assert_eq!(memory.expires_at, Some(3234));
        assert_eq!(memory.stability, MemoryStability::Stable);
        assert_eq!(memory.last_confirmed_at, Some(1234));
        assert_eq!(memory.next_review_at, Some(2234));
        assert_eq!(
            memory.dedupe_key.as_deref(),
            Some("correction:profile-sam:concise-deployment-updates")
        );
        assert_eq!(memory.embedding_model.as_deref(), Some("embed-test"));
        assert_eq!(memory.embedding.as_deref(), Some(&[0.1, 0.2, 0.3, 0.4][..]));
        let old_memory = store
            .get_memory(&MemoryId("old-memory".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            old_memory.superseded_by,
            Some(state.memories_formed[0].clone())
        );
        assert_eq!(old_memory.truth_status, TruthStatus::Outdated);
        match memory.source {
            MemorySource::Conversation {
                conversation_id,
                identity_id,
                profile_id,
                message_id,
                ..
            } => {
                assert_eq!(conversation_id, conversation);
                assert_eq!(identity_id, Some(identity));
                assert_eq!(profile_id, Some(profile));
                assert_eq!(message_id.as_deref(), Some("msg-1"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }

        let result = form(
            &json!({
                "content": "Alice prefers concise release notes.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "evidence_message_ids": ["msg-2"],
                "dedupe_key": "preference:profile-alice:concise-release-notes"
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let alice_memory_id = state.memories_formed.last().unwrap().clone();
        let memory = store.get_memory(&alice_memory_id).await.unwrap().unwrap();
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, "profile-alice");
        match memory.source {
            MemorySource::Conversation {
                identity_id,
                profile_id,
                message_id,
                ..
            } => {
                assert_eq!(identity_id, Some(other_identity));
                assert_eq!(profile_id, Some(other_profile));
                assert_eq!(message_id.as_deref(), Some("msg-2"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }

        let result = form(
            &json!({
                "content": "Alice prefers concise release notes with owners.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "confirmed",
                "evidence_message_ids": ["msg-2"],
                "dedupe_key": "preference:profile-alice:concise-release-notes"
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        assert_eq!(state.memories_formed.last(), Some(&alice_memory_id));
        let memory = store.get_memory(&alice_memory_id).await.unwrap().unwrap();
        assert!(
            memory
                .content
                .contains("Alice prefers concise release notes with owners.")
        );
        assert_eq!(memory.truth_status, TruthStatus::Confirmed);

        let metrics = ctx.metrics.snapshot();
        assert_eq!(metrics.memory_created, 2);
        assert_eq!(metrics.memory_updated, 2);
        assert_eq!(metrics.memory_superseded, 1);

        let recall_result = super::super::recall::recall(
            &json!({
                "query": "concise deployment updates",
                "limit": 5,
                "include_sensitive": true,
                "include_superseded": true
            }),
            &ctx,
            &mut state,
        )
        .await;
        let recalled: serde_json::Value = serde_json::from_str(&recall_result).unwrap();
        let recalled_memory = recalled["memories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"].as_str() == Some(state.memories_formed[0].0.as_str()))
            .expect("formed memory is recalled");
        assert_eq!(recalled_memory["memory_type"], "correction");
        assert_eq!(recalled_memory["truth_status"], "confirmed");
        assert!((recalled_memory["confidence"].as_f64().unwrap() - 0.87).abs() < 0.000001);
        assert!((recalled_memory["emotional_valence"].as_f64().unwrap() - 0.2).abs() < 0.000001);
        assert_eq!(recalled_memory["tags"], json!(["preference", "deployment"]));
        assert_eq!(
            recalled_memory["evidence_message_ids"],
            json!(["msg-1", "msg-2"])
        );
        assert_eq!(
            recalled_memory["evidence_quote"],
            "I prefer short deploy updates."
        );
        assert_eq!(
            recalled_memory["evidence"]["reason"],
            "user corrected preference"
        );
        assert_eq!(recalled_memory["expires_at"], 3234);
        assert_eq!(recalled_memory["stability"], "stable");
        assert_eq!(
            recalled_memory["dedupe_key"],
            "correction:profile-sam:concise-deployment-updates"
        );
        assert_eq!(recalled_memory["last_confirmed_at"], 1234);
        assert_eq!(recalled_memory["next_review_at"], 2234);

        let result = form(
            &json!({
                "content": "Alice's deployment credential should be rotated.",
                "kind": "semantic",
                "memory_type": "procedure",
                "sensitivity": 0.95,
                "sensitivity_category": "credentials",
                "evidence_message_ids": ["msg-2"]
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(state.memories_formed.last().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.privacy_category, PrivacyCategory::Secret);
        assert_eq!(memory.visibility_scope, VisibilityScope::OwnerOnly);
        assert!(memory.next_review_at.is_some());
    }

    #[tokio::test]
    async fn form_memory_uses_presented_injected_message_evidence() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let source_conversation = ConversationId("relay:source".into());
        let injected_conversation = ConversationId("relay:injected".into());
        let source_profile = ProfileId("profile-source".into());
        let source_identity = IdentityId("identity-source".into());
        let injected_profile = ProfileId("profile-injected".into());
        let injected_identity = IdentityId("identity-injected".into());
        let source_message = InboundMessage {
            message_id: "msg-source".into(),
            gateway_id: "relay".into(),
            sender_external_id: "source".into(),
            sender_display_name: None,
            reply_external_id: "source".into(),
            conversation: source_conversation.clone(),
            group: None,
            identity: Some(source_identity.clone()),
            profile: Some(source_profile.clone()),
            person: None,
            content: "I prefer terse status notes.".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        };
        let injected_message = InboundMessage {
            message_id: "msg-injected".into(),
            gateway_id: "relay".into(),
            sender_external_id: "injected".into(),
            sender_display_name: None,
            reply_external_id: "injected".into(),
            conversation: injected_conversation.clone(),
            group: None,
            identity: Some(injected_identity.clone()),
            profile: Some(injected_profile.clone()),
            person: None,
            content: "For release notes, include the owner and rollback path.".into(),
            attachments: vec![],
            timestamp: 1001,
            metadata: serde_json::Value::Null,
        };
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let ctx = SessionContext {
            action_id: ActionId("form-memory-injected-evidence-test".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![source_message],
            conversation: None,
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![injected_message],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 1,
        };

        let result = form(
            &json!({
                "content": "The source profile prefers terse status notes.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "dedupe_key": "preference:profile-source:terse-status-notes"
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            memory.evidence_message_ids,
            vec!["msg-source", "msg-injected"]
        );

        let result = form(
            &json!({
                "content": "The injected profile wants release notes to include owners and rollback paths.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "evidence_message_ids": ["msg-injected"],
                "dedupe_key": "preference:profile-injected:release-note-owner-rollback"
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(state.memories_formed.last().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, injected_profile.0);
        match memory.source {
            MemorySource::Conversation {
                conversation_id,
                identity_id,
                profile_id,
                message_id,
                ..
            } => {
                assert_eq!(conversation_id, injected_conversation);
                assert_eq!(identity_id, Some(injected_identity));
                assert_eq!(profile_id, Some(injected_profile));
                assert_eq!(message_id.as_deref(), Some("msg-injected"));
            }
            other => panic!("expected conversation source, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn form_memory_persists_without_embedding_when_embedding_endpoint_fails() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let profile = ProfileId("profile-sam".into());
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let ctx = SessionContext {
            action_id: ActionId("form-memory-embedding-failure-test".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![InboundMessage {
                message_id: "msg-embed-fail".into(),
                gateway_id: "relay".into(),
                sender_external_id: "local".into(),
                sender_display_name: None,
                reply_external_id: "local".into(),
                conversation: conversation.clone(),
                group: None,
                identity: None,
                profile: Some(profile.clone()),
                person: None,
                content: "I prefer concise launch briefs.".into(),
                attachments: vec![],
                timestamp: 1000,
                metadata: serde_json::Value::Null,
            }],
            conversation: Some(conversation),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router_with_failing_embedding_endpoint()),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "Sam prefers concise launch briefs.",
                "kind": "semantic",
                "memory_type": "preference",
                "truth_status": "stated",
                "evidence_message_ids": ["msg-embed-fail"]
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert!(memory.embedding.is_none());
        assert_eq!(memory.memory_type, MemoryType::Preference);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Profile);
        assert_eq!(memory.subjects[0].subject_id, profile.0);
    }

    #[tokio::test]
    async fn form_memory_can_store_owner_approved_actor_self_memory() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();
        let ctx = SessionContext {
            action_id: ActionId("actor-self-memory-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![],
            conversation: None,
            authority: Authority::Owner,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: store_dyn,
            media_store: None,
            router: Arc::new(router),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        };
        let mut state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let result = form(
            &json!({
                "content": "My name is Pamagotchi.",
                "kind": "semantic",
                "memory_type": "identity_claim",
                "sensitivity_category": "identity",
                "subject_actor": true
            }),
            &ctx,
            &mut state,
        )
        .await;

        assert!(result.starts_with("Memory saved: "));
        let memory = store
            .get_memory(&state.memories_formed[0])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(memory.subjects.len(), 1);
        assert_eq!(memory.subjects[0].subject_type, MemorySubjectType::Actor);
        assert_eq!(memory.subjects[0].subject_id, "self");
        assert_eq!(memory.memory_type, MemoryType::IdentityClaim);

        let recalled = store
            .recall(&RecallQuery::by_text("my name", 10).with_actor_subject())
            .await
            .unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].id, state.memories_formed[0]);
    }
}
