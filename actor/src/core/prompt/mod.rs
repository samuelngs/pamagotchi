mod context;

use context::*;
use super::action::{ActionContext, ActionKind};
use super::state::StateHandle;
use super::tools::SessionKind;
use crate::personality::Authority;
use crate::store::{MemoryKind, RecallQuery, Store};
use protocol::{ConversationId, InboundMessage};
use minijinja::Environment;
use std::sync::Arc;

fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.add_template("mind.j2", include_str!("templates/mind.j2")).unwrap();
    env.add_template("action.j2", include_str!("templates/action.j2")).unwrap();
    env
}

pub async fn build_system_prompt(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    kind: &SessionKind,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    action_ctx: Option<&ActionContext>,
    authority: &Authority,
) -> anyhow::Result<String> {
    let env = make_env();
    match kind {
        SessionKind::Mind => build_mind(&env, state, store, messages, action_ctx).await,
        SessionKind::Action(action_kind) => {
            build_action(&env, state, store, action_kind, messages, conversation, action_ctx, authority).await
        }
    }
}

async fn build_mind(
    env: &Environment<'_>,
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    action_ctx: Option<&ActionContext>,
) -> anyhow::Result<String> {
    let identity = recall_identity_name(store).await;
    let person = resolve_person_for_mind(state, store, messages).await;
    let actions = extract_actions(action_ctx);
    let thoughts = fetch_thoughts(store).await;

    let ctx = MindContext { identity, person, actions, thoughts };
    let tmpl = env.get_template("mind.j2")?;
    Ok(tmpl.render(&ctx)?)
}

async fn build_action(
    env: &Environment<'_>,
    state: &StateHandle,
    store: &Arc<dyn Store>,
    kind: &ActionKind,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    action_ctx: Option<&ActionContext>,
    authority: &Authority,
) -> anyhow::Result<String> {
    let ps = state.read_personality().clone();

    let identity_memories = recall_identity_memories(store).await;
    let traits = TraitsCtx {
        openness: pct(ps.core_traits.openness),
        warmth: pct(ps.core_traits.warmth),
        assertiveness: pct(ps.core_traits.assertiveness),
        humor: pct(ps.core_traits.humor),
        curiosity: pct(ps.core_traits.curiosity),
        patience: pct(ps.core_traits.patience),
        directness: pct(ps.core_traits.directness),
        playfulness: pct(ps.core_traits.playfulness),
    };

    let mut beliefs: Vec<BeliefCtx> = Vec::new();
    for b in ps.beliefs.iter().take(20) {
        let about = match &b.about {
            Some(pid) => Some(resolve_person_name(store, pid).await),
            None => None,
        };
        beliefs.push(BeliefCtx {
            topic: b.topic.clone(),
            about,
            stance: b.stance.clone(),
            confidence: pct(b.confidence),
        });
    }

    let mut interests: Vec<_> = ps.interests.iter().collect();
    interests.sort_by(|a, b| b.intensity.partial_cmp(&a.intensity).unwrap_or(std::cmp::Ordering::Equal));
    let interests: Vec<InterestCtx> = interests.iter().take(10).map(|i| InterestCtx {
        topic: i.topic.clone(),
        intensity: pct(i.intensity),
    }).collect();

    let mood = match ps.affect.valence {
        v if v > 0.3 => "positive",
        v if v < -0.3 => "low",
        _ => "neutral",
    }.into();

    let energy = match ps.affect.arousal {
        a if a > 0.6 => "high energy",
        a if a < 0.3 => "low energy",
        _ => "moderate energy",
    }.into();

    let person_id = messages.first().and_then(|m| m.person.as_ref());

    let relationship = if let Some(pid) = person_id {
        let name = resolve_person_name(store, pid).await;
        ps.relationships.get(pid).map(|rel| {
            let tone = if rel.emotional_valence > 0.3 { "warm" }
                else if rel.emotional_valence < -0.3 { "strained" }
                else { "neutral" };
            RelationshipCtx {
                name,
                trust: pct(rel.trust),
                familiarity: pct(rel.familiarity),
                interactions: rel.interaction_count,
                tone: tone.into(),
            }
        })
    } else {
        None
    };

    let directives = if let Some(pid) = person_id {
        load_directives(store, &ps, pid, conversation).await.unwrap_or_default()
    } else {
        vec![]
    };

    let thoughts = fetch_thoughts(store).await;
    let cancelled_note = action_ctx.and_then(|c| c.cancelled_note.clone());
    let concurrent_actions = extract_actions(action_ctx);

    let ctx = ActionPromptContext {
        identity_memories,
        traits,
        beliefs,
        interests,
        mood,
        energy,
        relationship,
        directives,
        thoughts,
        cancelled_note,
        concurrent_actions,
        authority: authority.as_str().to_string(),
        kind: kind.as_str().to_string(),
    };

    let tmpl = env.get_template("action.j2")?;
    Ok(tmpl.render(&ctx)?)
}

async fn recall_identity_name(store: &Arc<dyn Store>) -> String {
    let query = RecallQuery::by_text("my name, who I am", 1)
        .with_kind(MemoryKind::Semantic)
        .with_min_importance(0.5);
    match store.recall(&query).await {
        Ok(memories) if !memories.is_empty() => memories[0].content.clone(),
        _ => "an unnamed being".into(),
    }
}

async fn recall_identity_memories(store: &Arc<dyn Store>) -> Vec<String> {
    let query = RecallQuery::by_text("my name, who I am, my identity", 5)
        .with_kind(MemoryKind::Semantic)
        .with_min_importance(0.5);
    match store.recall(&query).await {
        Ok(memories) => memories.into_iter().map(|m| m.content).collect(),
        Err(_) => vec![],
    }
}

async fn fetch_thoughts(store: &Arc<dyn Store>) -> Vec<ThoughtCtx> {
    store.recent_thoughts(5).await.unwrap_or_default().into_iter().map(|t| ThoughtCtx {
        kind: t.kind.as_str().to_string(),
        content: t.content,
    }).collect()
}

async fn resolve_person_for_mind(state: &StateHandle, store: &Arc<dyn Store>, messages: &[InboundMessage]) -> Option<PersonContext> {
    let msg = messages.first()?;
    let person_id = msg.person.as_ref()?;
    let name = resolve_person_name(store, person_id).await;
    let ps = state.read_personality();
    if let Some(rel) = ps.relationships.get(person_id) {
        Some(PersonContext {
            name,
            authority: rel.authority.as_str().to_string(),
            trust: pct(rel.trust),
            familiarity: pct(rel.familiarity),
        })
    } else {
        Some(PersonContext {
            name,
            authority: "default".into(),
            trust: 0,
            familiarity: 0,
        })
    }
}

async fn resolve_person_name(store: &Arc<dyn Store>, person_id: &protocol::PersonId) -> String {
    match store.get_person(person_id).await {
        Ok(Some(p)) if !p.name.is_empty() => p.name,
        _ => "unnamed person".into(),
    }
}

fn extract_actions(action_ctx: Option<&ActionContext>) -> Vec<ActionBriefCtx> {
    action_ctx.map_or(vec![], |ctx| {
        ctx.concurrent_actions.iter().map(|a| ActionBriefCtx {
            id: a.id.0.clone(),
            kind: format!("{:?}", a.kind),
            task: a.task.clone(),
        }).collect()
    })
}

async fn load_directives(
    store: &Arc<dyn Store>,
    personality: &crate::personality::PersonalityState,
    person: &protocol::PersonId,
    conversation: Option<&ConversationId>,
) -> anyhow::Result<Vec<String>> {
    use crate::personality::Authority;
    let rel = personality.relationships.get(person);
    let authority = rel.map_or(Authority::Default, |r| r.authority.clone());

    let group = if let Some(conv) = conversation {
        let summaries = store.list_conversations().await?;
        summaries.into_iter().find(|s| s.id == *conv).and_then(|s| s.group)
    } else {
        None
    };

    let directives = store.get_directives_for_context(person, &authority, group.as_ref()).await?;
    Ok(directives.into_iter().map(|d| d.directive).collect())
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}
