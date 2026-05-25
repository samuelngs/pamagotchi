mod context;

use context::*;
use super::action::ActionKind;
use super::handle::StateHandle;
use super::tools::{SessionContext, SessionKind};
use crate::state::{ActorState, Authority};
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
    session_ctx: &SessionContext,
    authority: &Authority,
) -> anyhow::Result<String> {
    let env = make_env();
    match kind {
        SessionKind::Mind => build_mind(&env, state, store, messages, &session_ctx.concurrent_summaries).await,
        SessionKind::Action(action_kind) => {
            build_action(&env, state, store, action_kind, messages, conversation, session_ctx, authority).await
        }
    }
}

async fn build_mind(
    env: &Environment<'_>,
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    concurrent_summaries: &[(String, String, String)],
) -> anyhow::Result<String> {
    let identity = recall_identity_name(store).await;
    let now = format_now();
    let person = resolve_person_for_mind(state, store, messages).await;
    let actions: Vec<ActionBriefCtx> = concurrent_summaries
        .iter()
        .map(|(id, kind, task)| ActionBriefCtx {
            id: id.clone(),
            kind: kind.clone(),
            task: task.clone(),
        })
        .collect();
    let thoughts = fetch_thoughts(store).await;

    let ctx = MindContext { identity, now, person, actions, thoughts };
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
    session_ctx: &SessionContext,
    authority: &Authority,
) -> anyhow::Result<String> {
    let actor = state.read_state().clone();
    let now_ts = chrono::Utc::now();
    let now = format_now();
    let age = relative_duration(actor.created_at, now_ts.timestamp());

    let identity_memories = recall_identity_memories(store).await;
    let traits = TraitsCtx {
        openness: pct(actor.traits.openness),
        warmth: pct(actor.traits.warmth),
        assertiveness: pct(actor.traits.assertiveness),
        humor: pct(actor.traits.humor),
        curiosity: pct(actor.traits.curiosity),
        patience: pct(actor.traits.patience),
        directness: pct(actor.traits.directness),
        playfulness: pct(actor.traits.playfulness),
    };

    let mut beliefs: Vec<BeliefCtx> = Vec::new();
    for b in actor.beliefs.iter().take(20) {
        let about = match &b.about {
            Some(pid) => resolve_person_info(store, pid).await.name,
            None => None,
        };
        beliefs.push(BeliefCtx {
            topic: b.topic.clone(),
            about,
            stance: b.stance.clone(),
            confidence: pct(b.confidence),
        });
    }

    let mut interests: Vec<_> = actor.interests.iter().collect();
    interests.sort_by(|a, b| b.intensity.partial_cmp(&a.intensity).unwrap_or(std::cmp::Ordering::Equal));
    let interests: Vec<InterestCtx> = interests.iter().take(10).map(|i| InterestCtx {
        topic: i.topic.clone(),
        intensity: pct(i.intensity),
    }).collect();

    let mood = match actor.affect.valence {
        v if v > 0.3 => "positive",
        v if v < -0.3 => "low",
        _ => "neutral",
    }.into();

    let energy = match actor.affect.arousal {
        a if a > 0.6 => "high energy",
        a if a < 0.3 => "low energy",
        _ => "moderate energy",
    }.into();

    let person_id = messages.first().and_then(|m| m.person.as_ref());
    let now_unix = now_ts.timestamp();

    let style_directive = session_ctx.style_directive.clone();

    let (relationship, comm_style) = if let Some(pid) = person_id {
        let info = resolve_person_info(store, pid).await;
        let rel_ctx = actor.bonds.get(pid).map(|rel| {
            let tone = if rel.emotional_valence > 0.3 { "warm" }
                else if rel.emotional_valence < -0.3 { "strained" }
                else { "neutral" };
            RelationshipCtx {
                ref_id: pid.0.clone(),
                name: info.name.clone(),
                summary: info.summary.clone(),
                trust: pct(rel.trust),
                familiarity: pct(rel.familiarity),
                interactions: rel.interaction_count,
                tone: tone.into(),
                last_seen: info.last_seen.map(|ts| relative_duration(ts, now_unix)),
                first_met: info.first_seen.map(|ts| relative_duration(ts, now_unix)),
            }
        });
        let interaction_count = actor.bonds.get(pid).map_or(0, |r| r.interaction_count);
        let style = if interaction_count >= 10 && info.comm_style.is_some() {
            info.comm_style
        } else {
            style_directive
        };
        (rel_ctx, style)
    } else {
        (None, style_directive)
    };

    let directives = if let Some(pid) = person_id {
        load_directives(store, &actor, pid, conversation).await.unwrap_or_default()
    } else {
        vec![]
    };

    let thoughts = fetch_thoughts(store).await;
    let cancelled_note = session_ctx.cancelled_note.clone();
    let concurrent_actions: Vec<ActionBriefCtx> = session_ctx
        .concurrent_summaries
        .iter()
        .map(|(id, kind, task)| ActionBriefCtx {
            id: id.clone(),
            kind: kind.clone(),
            task: task.clone(),
        })
        .collect();

    let ctx = ActionPromptContext {
        now,
        age,
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
        style: comm_style,
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
    let info = resolve_person_info(store, person_id).await;
    let actor = state.read_state();
    let now_unix = chrono::Utc::now().timestamp();
    let last_seen = info.last_seen.map(|ts| relative_duration(ts, now_unix));
    if let Some(rel) = actor.bonds.get(person_id) {
        Some(PersonContext {
            ref_id: person_id.0.clone(),
            name: info.name,
            summary: info.summary,
            authority: rel.authority.as_str().to_string(),
            trust: pct(rel.trust),
            familiarity: pct(rel.familiarity),
            last_seen,
        })
    } else {
        Some(PersonContext {
            ref_id: person_id.0.clone(),
            name: info.name,
            summary: info.summary,
            authority: "default".into(),
            trust: 0,
            familiarity: 0,
            last_seen,
        })
    }
}

struct PersonInfo {
    name: Option<String>,
    summary: Option<String>,
    comm_style: Option<String>,
    first_seen: Option<i64>,
    last_seen: Option<i64>,
}

async fn resolve_person_info(store: &Arc<dyn Store>, person_id: &protocol::PersonId) -> PersonInfo {
    match store.get_person(person_id).await {
        Ok(Some(p)) => PersonInfo {
            name: p.name,
            summary: p.summary,
            comm_style: p.comm_style,
            first_seen: Some(p.first_seen),
            last_seen: Some(p.last_seen),
        },
        _ => PersonInfo { name: None, summary: None, comm_style: None, first_seen: None, last_seen: None },
    }
}

async fn load_directives(
    store: &Arc<dyn Store>,
    actor: &ActorState,
    person: &protocol::PersonId,
    conversation: Option<&ConversationId>,
) -> anyhow::Result<Vec<String>> {
    let rel = actor.bonds.get(person);
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

fn format_now() -> String {
    let now = chrono::Utc::now();
    now.format("%A %H:%M UTC, %B %-d %Y").to_string()
}

fn relative_duration(from: i64, to: i64) -> String {
    let secs = (to - from).max(0);
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 { "1 minute ago".into() } else { format!("{m} minutes ago") }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 { "1 hour ago".into() } else { format!("{h} hours ago") }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 { "1 day ago".into() } else { format!("{d} days ago") }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 { "1 week ago".into() } else { format!("{w} weeks ago") }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 { "1 month ago".into() } else { format!("{mo} months ago") }
    } else {
        let y = secs / 31536000;
        if y == 1 { "1 year ago".into() } else { format!("{y} years ago") }
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}
