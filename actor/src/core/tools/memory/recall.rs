use super::super::context::{SessionContext, SessionState};
use super::helpers::{
    current_identity, current_profile, current_subject_relation, format_timestamp,
    global_recall_requested, relation_rank,
};
use crate::store::{MemoryKind, MemorySource, MemorySubjectType, RecallQuery};
use protocol::{IdentityId, MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};
use std::time::Instant;

pub async fn recall(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(3) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let kind = args["kind"].as_str().and_then(MemoryKind::parse);
    let identity = args["identity"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| IdentityId(s.trim().to_string()));
    let profile = args["profile"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| ProfileId(s.trim().to_string()));
    let person = args["person"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| PersonId(s.trim().to_string()));

    let embedding = match ctx.router.embed(&[query]).await {
        Ok(vecs) if !vecs.is_empty() => vecs.into_iter().next(),
        _ => None,
    };
    let make_query = |limit: usize| match embedding.clone() {
        Some(embedding) => RecallQuery::by_embedding(embedding, limit),
        None => RecallQuery::by_text(query, limit),
    };
    let current = ctx.messages.first();

    let has_explicit_scope = identity.is_some() || profile.is_some() || person.is_some();
    let global_scope = global_recall_requested(args);
    let mut recall = make_query(limit).with_offset(offset);
    if let Some(kind) = kind.clone() {
        recall = recall.with_kind(kind);
    }
    if let Some(identity) = identity {
        recall = recall.with_identity(identity);
    }
    if let Some(profile) = profile {
        recall = recall.with_profile(profile);
    }
    if let Some(person) = person {
        recall = recall.with_person(person);
    }
    if !has_explicit_scope && !global_scope {
        if let Some(profile) = current_profile(ctx) {
            recall = recall.with_profile(profile);
        } else if let Some(identity) = current_identity(ctx) {
            recall = recall.with_identity(identity);
        }
    }
    if args["include_sensitive"].as_bool().unwrap_or(false) {
        recall = recall.include_sensitive();
    } else if let Some(max_sensitivity) = args["max_sensitivity"].as_f64() {
        recall = recall.with_max_sensitivity(max_sensitivity.clamp(0.0, 1.0) as f32);
    }
    if args["include_superseded"].as_bool().unwrap_or(false) {
        recall = recall.include_superseded();
    }

    let recall_start = Instant::now();
    let memories = ctx.store.recall(&recall).await;
    let recall_latency_ms = recall_start.elapsed().as_millis() as u64;

    match memories {
        Ok(memories) if memories.is_empty() => {
            ctx.metrics.record_recall(recall_latency_ms, 0);
            json!({"memories": []}).to_string()
        }
        Ok(memories) => {
            ctx.metrics.record_recall(recall_latency_ms, memories.len());
            let mut ranked_items = Vec::new();
            for (search_rank, m) in memories.into_iter().enumerate() {
                remember_recalled_memory(state, &m.id);
                let created_at = format_timestamp(m.created_at);
                let accessed_at = format_timestamp(m.accessed_at);
                let mut subjects = Vec::new();
                for subject in &m.subjects {
                    let mut entry = json!({
                        "type": subject.subject_type.as_str(),
                        "id": subject.subject_id,
                        "role": subject.role,
                        "confidence": subject.confidence,
                    });
                    if subject.subject_type == MemorySubjectType::Person {
                        if let Ok(Some(p)) = ctx
                            .store
                            .get_person(&PersonId(subject.subject_id.clone()))
                            .await
                        {
                            if let Some(name) = &p.name {
                                entry["name"] = json!(name);
                            }
                        }
                    }
                    subjects.push(entry);
                }
                let source = match &m.source {
                    MemorySource::Conversation {
                        conversation_id,
                        identity_id,
                        profile_id,
                        person_id,
                        message_id,
                    } => json!({
                        "kind": "conversation",
                        "conversation_id": conversation_id.0,
                        "identity_id": identity_id.as_ref().map(|id| id.0.clone()),
                        "profile_id": profile_id.as_ref().map(|id| id.0.clone()),
                        "person_id": person_id.as_ref().map(|id| id.0.clone()),
                        "message_id": message_id,
                    }),
                    MemorySource::Consolidation { from_memories } => json!({
                        "kind": "consolidation",
                        "from_memories": from_memories.iter().map(|id| id.0.clone()).collect::<Vec<_>>(),
                    }),
                    MemorySource::Reflection => json!({"kind": "reflection"}),
                    MemorySource::External => json!({"kind": "external"}),
                };
                let relation = current_subject_relation(&m.subjects, current, ctx).await;
                ranked_items.push((
                    relation_rank(relation),
                    search_rank,
                    m.created_at,
                    json!({
                        "id": m.id.0,
                        "kind": m.kind.as_str(),
                        "memory_type": m.memory_type.as_str(),
                        "content": m.content,
                        "created": created_at,
                        "created_at": created_at,
                        "accessed_at": accessed_at,
                        "access_count": m.access_count,
                        "importance": m.importance,
                        "confidence": m.confidence,
                        "sensitivity": m.sensitivity,
                        "sensitivity_category": m.sensitivity_category,
                        "emotional_valence": m.emotional_valence,
                        "tags": m.tags,
                        "privacy_category": m.privacy_category.as_str(),
                        "visibility_scope": m.visibility_scope.as_str(),
                        "truth_status": m.truth_status.as_str(),
                        "evidence_message_ids": m.evidence_message_ids,
                        "evidence_quote": m.evidence_quote,
                        "evidence": m.evidence,
                        "expires_at": m.expires_at,
                        "stability": m.stability.as_str(),
                        "supersedes": m.supersedes.as_ref().map(|id| id.0.clone()),
                        "superseded_by": m.superseded_by.as_ref().map(|id| id.0.clone()),
                        "contradiction_group": m.contradiction_group,
                        "last_confirmed_at": m.last_confirmed_at,
                        "next_review_at": m.next_review_at,
                        "dedupe_key": m.dedupe_key,
                        "subjects": subjects,
                        "source": source,
                        "current_subject_relation": relation,
                        "rank_reason": relation,
                    }),
                ));
            }
            ranked_items.sort_by(
                |(rank_a, search_a, created_a, _), (rank_b, search_b, created_b, _)| {
                    rank_a
                        .cmp(rank_b)
                        .then_with(|| search_a.cmp(search_b))
                        .then_with(|| created_b.cmp(created_a))
                },
            );
            let items = ranked_items
                .into_iter()
                .map(|(_, _, _, item)| item)
                .collect::<Vec<_>>();
            json!({"memories": items}).to_string()
        }
        Err(e) => {
            ctx.metrics.record_recall(recall_latency_ms, 0);
            json!({"error": format!("{e}")}).to_string()
        }
    }
}

fn remember_recalled_memory(state: &mut SessionState, id: &MemoryId) {
    if state
        .recalled_memory_ids
        .iter()
        .any(|existing| existing == id)
    {
        return;
    }
    state.recalled_memory_ids.push(id.clone());
    const MAX_RECALLED_MEMORY_IDS: usize = 32;
    if state.recalled_memory_ids.len() > MAX_RECALLED_MEMORY_IDS {
        let overflow = state.recalled_memory_ids.len() - MAX_RECALLED_MEMORY_IDS;
        state.recalled_memory_ids.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests {
    use super::{recall, remember_recalled_memory};
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::core::tools::{SessionContext, SessionKind, SessionState};
    use crate::state::{ActorState, Authority, Delta, GrowthConfig};
    use crate::store::{Memory, MemoryKind, MemorySource, MemorySubject, SqliteStore, Store};
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{ConversationId, InboundMessage, MemoryId, ProfileId};
    use serde_json::{Value, json};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;

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
            anyhow::bail!("noop bridge is not used by recall_memory tests")
        }

        async fn embed(&self, _model: &str, _input: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedding endpoint unavailable")
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

    fn state() -> SessionState {
        SessionState {
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
        }
    }

    fn context(
        store: Arc<SqliteStore>,
        profile: &ProfileId,
        conversation: &ConversationId,
    ) -> (SessionContext, SessionState) {
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let message = InboundMessage {
            message_id: "msg-1".into(),
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
            metadata: Value::Null,
        };

        (
            SessionContext {
                action_id: ActionId("recall-memory-embedding-failure-test".into()),
                kind: SessionKind::Action(ActionKind::Respond),
                messages: vec![message],
                conversation: Some(conversation.clone()),
                authority: Authority::Default,
                style_directive: None,
                cancelled_note: None,
                concurrent_summaries: vec![],
                state: StateHandle::new(shared, delta_tx),
                store,
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
            },
            state(),
        )
    }

    #[test]
    fn recalled_memory_tracking_is_deduped_and_bounded() {
        let mut state = state();
        remember_recalled_memory(&mut state, &MemoryId("memory-a".into()));
        remember_recalled_memory(&mut state, &MemoryId("memory-a".into()));
        assert_eq!(state.recalled_memory_ids, vec![MemoryId("memory-a".into())]);

        for i in 0..40 {
            remember_recalled_memory(&mut state, &MemoryId(format!("memory-{i}")));
        }

        assert_eq!(state.recalled_memory_ids.len(), 32);
        assert!(
            !state
                .recalled_memory_ids
                .contains(&MemoryId("memory-a".into()))
        );
        assert!(
            state
                .recalled_memory_ids
                .contains(&MemoryId("memory-39".into()))
        );
    }

    #[tokio::test]
    async fn recall_uses_text_search_when_embedding_endpoint_fails() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let conversation = ConversationId("relay:local".into());
        store
            .store_memory(&Memory {
                id: MemoryId("memory-concise-launch-briefs".into()),
                kind: MemoryKind::Semantic,
                content: "Sam prefers concise launch briefs.".into(),
                source: MemorySource::Reflection,
                subjects: vec![MemorySubject::profile(
                    profile.clone(),
                    Some("about".into()),
                    1.0,
                )],
                embedding: None,
                ..Memory::default()
            })
            .await
            .unwrap();
        let (ctx, mut state) = context(store, &profile, &conversation);

        let result = recall(
            &json!({
                "query": "concise launch briefs",
                "limit": 3
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let memories = parsed["memories"].as_array().unwrap();

        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0]["id"], "memory-concise-launch-briefs");
        assert_eq!(
            state.recalled_memory_ids,
            vec![MemoryId("memory-concise-launch-briefs".into())]
        );
    }

    #[tokio::test]
    async fn recall_preserves_store_relevance_within_same_subject_relation() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let profile = ProfileId("profile-sam".into());
        let conversation = ConversationId("relay:local".into());

        let older_relevant = Memory {
            id: MemoryId("older-relevant".into()),
            kind: MemoryKind::Semantic,
            content: "Sam asked for a kubernetes budget review.".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            created_at: 1000,
            accessed_at: 1000,
            importance: 0.5,
            embedding: None,
            ..Memory::default()
        };

        let newer_weaker = Memory {
            id: MemoryId("newer-weaker".into()),
            kind: MemoryKind::Semantic,
            content: "Sam mentioned kubernetes.".into(),
            source: MemorySource::Reflection,
            subjects: vec![MemorySubject::profile(
                profile.clone(),
                Some("about".into()),
                1.0,
            )],
            created_at: 2000,
            accessed_at: 2000,
            importance: 0.5,
            embedding: None,
            ..Memory::default()
        };

        store.store_memory(&newer_weaker).await.unwrap();
        store.store_memory(&older_relevant).await.unwrap();

        let (ctx, mut state) = context(store, &profile, &conversation);
        let result = recall(
            &json!({
                "query": "kubernetes budget",
                "limit": 2
            }),
            &ctx,
            &mut state,
        )
        .await;
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let ids = parsed["memories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|memory| memory["id"].as_str().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["older-relevant", "newer-weaker"]);
    }
}
