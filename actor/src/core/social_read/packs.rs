use crate::core::action::ActionKind;
use crate::core::prompt::context::{RecentMessageCtx, SafetyCtx, ThoughtCtx};
use crate::core::tools::SessionKind;
use crate::state::Authority;
use crate::store::{MemorySubjectType, Store, StoredMessage, Thought};
use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
use std::{collections::HashSet, sync::Arc};

pub(crate) fn build_safety_ctx(authority: &Authority, kind: &SessionKind) -> SafetyCtx {
    let sensitive_memory_access = if matches!(authority, Authority::Owner)
        || matches!(
            kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        ) {
        "sensitive recall allowed only when directly relevant; logs and transcripts are redacted"
    } else {
        "conservative recall only; private or sensitive details require owner authority or review context"
    };
    let proactive_outreach = match kind {
        SessionKind::Action(ActionKind::Outreach) => {
            "active outreach; obey consent, quiet hours, stale conversation, unanswered outreach, and gateway availability guards"
        }
        _ => {
            "create or update proactive intents only when target, consent, timing, and owner-approval rules allow it"
        }
    };
    let third_party_outreach = if matches!(authority, Authority::Owner) {
        "owner can approve third-party outreach, but verified target and consent/timing guards still apply"
    } else {
        "third-party outreach requires a verified active target; sensitive third-party outreach requires owner approval"
    };

    SafetyCtx {
        authority: authority.as_str().to_string(),
        sensitive_memory_access: sensitive_memory_access.into(),
        proactive_outreach: proactive_outreach.into(),
        third_party_outreach: third_party_outreach.into(),
    }
}

pub(crate) async fn fetch_recent_messages(
    store: &Arc<dyn Store>,
    conversation: Option<&ConversationId>,
    current_messages: &[InboundMessage],
) -> Vec<RecentMessageCtx> {
    let Some(conversation) = conversation else {
        return vec![];
    };
    let current_source_ids = current_messages
        .iter()
        .map(|message| message.message_id.as_str())
        .collect::<HashSet<_>>();
    match store.get_messages(conversation, 6, None).await {
        Ok(messages) => messages
            .into_iter()
            .filter(|message| {
                message
                    .source_message_id
                    .as_deref()
                    .is_none_or(|id| !current_source_ids.contains(id))
            })
            .map(recent_message_ctx)
            .collect(),
        Err(_) => vec![],
    }
}

fn recent_message_ctx(message: StoredMessage) -> RecentMessageCtx {
    let message_id = message.readable_message_id();
    RecentMessageCtx {
        message_id,
        role: message.role.as_str().to_string(),
        speaker: message
            .sender_external_id
            .or(message.reply_external_id)
            .or_else(|| message.profile.map(|profile| profile.0))
            .or_else(|| message.person.map(|person| person.0)),
        content: message.content,
    }
}

pub(crate) async fn fetch_thoughts(
    store: &Arc<dyn Store>,
    identity: Option<&IdentityId>,
    profile: Option<&ProfileId>,
    person: Option<&PersonId>,
) -> Vec<ThoughtCtx> {
    let mut seen = HashSet::new();
    let mut thoughts = Vec::new();

    if let Some(profile) = profile {
        append_subject_thoughts(
            store,
            MemorySubjectType::Profile,
            &profile.0,
            &mut seen,
            &mut thoughts,
        )
        .await;
    }
    if let Some(person) = person {
        append_subject_thoughts(
            store,
            MemorySubjectType::Person,
            &person.0,
            &mut seen,
            &mut thoughts,
        )
        .await;
    }
    if let Some(identity) = identity {
        append_subject_thoughts(
            store,
            MemorySubjectType::Identity,
            &identity.0,
            &mut seen,
            &mut thoughts,
        )
        .await;
    }

    if thoughts.is_empty() && identity.is_none() && profile.is_none() && person.is_none() {
        thoughts = store.recent_thoughts(5).await.unwrap_or_default();
    }

    thoughts.sort_by(|a, b| {
        b.importance
            .partial_cmp(&a.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    thoughts.truncate(5);
    thoughts.sort_by_key(|thought| thought.timestamp);
    thoughts.into_iter().map(thought_ctx).collect()
}

async fn append_subject_thoughts(
    store: &Arc<dyn Store>,
    subject_type: MemorySubjectType,
    subject_id: &str,
    seen: &mut HashSet<String>,
    out: &mut Vec<Thought>,
) {
    let Ok(thoughts) = store
        .recent_thoughts_for_subject(subject_type, subject_id, 5)
        .await
    else {
        return;
    };
    for thought in thoughts {
        let key = format!(
            "{}:{}:{}",
            thought.timestamp,
            thought.action_id.as_deref().unwrap_or(""),
            thought.content
        );
        if seen.insert(key) {
            out.push(thought);
        }
    }
}

pub(crate) fn thought_ctx(t: Thought) -> ThoughtCtx {
    ThoughtCtx {
        kind: t.kind.as_str().to_string(),
        content: t.content,
        importance: pct(t.importance),
        confidence: pct(t.confidence),
        memory_ids: t
            .memories_accessed
            .into_iter()
            .map(|memory| memory.0)
            .collect(),
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}
