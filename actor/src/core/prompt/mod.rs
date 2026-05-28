pub(crate) mod context;

use super::action::ActionKind;
use super::handle::StateHandle;
use super::review;
use super::social_read;
use super::tools::{SessionContext, SessionKind};
use crate::state::{ActorState, Authority};
use crate::store::{ConversationSummary, MemoryKind, RecallQuery, Store};
use context::*;
use minijinja::Environment;
use protocol::{ConversationId, GroupId, InboundMessage, ProfileId};
use std::sync::Arc;

fn make_env() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.add_template("mind.j2", include_str!("templates/mind.j2"))
        .unwrap();
    env.add_template("action.j2", include_str!("templates/action.j2"))
        .unwrap();
    env.add_template(
        "action_task_respond.j2",
        include_str!("templates/action_task_respond.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_review.j2",
        include_str!("templates/action_task_review.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_ruminate.j2",
        include_str!("templates/action_task_ruminate.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_consolidate.j2",
        include_str!("templates/action_task_consolidate.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_outreach.j2",
        include_str!("templates/action_task_outreach.j2"),
    )
    .unwrap();
    env.add_template(
        "action_task_research.j2",
        include_str!("templates/action_task_research.j2"),
    )
    .unwrap();
    env
}

fn action_task_template(kind: &ActionKind) -> &'static str {
    match kind {
        ActionKind::Respond => "action_task_respond.j2",
        ActionKind::Review => "action_task_review.j2",
        ActionKind::Ruminate => "action_task_ruminate.j2",
        ActionKind::Consolidate => "action_task_consolidate.j2",
        ActionKind::Outreach => "action_task_outreach.j2",
        ActionKind::Research => "action_task_research.j2",
    }
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
        SessionKind::Mind => {
            build_mind(
                &env,
                state,
                store,
                messages,
                session_ctx,
                &session_ctx.concurrent_summaries,
            )
            .await
        }
        SessionKind::Action(action_kind) => {
            build_action(
                &env,
                state,
                store,
                action_kind,
                messages,
                conversation,
                session_ctx,
                authority,
            )
            .await
        }
    }
}

async fn build_mind(
    env: &Environment<'_>,
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    session_ctx: &SessionContext,
    concurrent_summaries: &[(String, String, String)],
) -> anyhow::Result<String> {
    let identity = recall_identity_name(store).await;
    let now = format_now();
    let now_ts = chrono::Utc::now();
    let authority =
        if let Some(person) = messages.first().and_then(|message| message.person.as_ref()) {
            let actor = state.read_state();
            actor
                .bonds
                .get(person)
                .map(|rel| rel.authority.clone())
                .unwrap_or(Authority::Default)
        } else {
            Authority::Default
        };
    let person = resolve_person_for_mind(state, store, messages).await;
    let first_message = messages.first();
    let profile = fetch_current_profile_ctx(
        store,
        first_message.and_then(|message| message.profile.as_ref()),
    )
    .await;
    let conversation_id = first_message.map(|message| &message.conversation);
    let conversation_summary = fetch_conversation_summary(store, conversation_id).await;
    let conversation = conversation_summary
        .as_ref()
        .map(conversation_ctx_from_summary);
    let group_id = first_message
        .and_then(|message| message.group.as_ref())
        .or_else(|| {
            conversation_summary
                .as_ref()
                .and_then(|summary| summary.group.as_ref())
        });
    let group = fetch_group_ctx(store, group_id).await;
    let timing = social_read::build_timing_ctx(
        store,
        messages,
        conversation_id,
        first_message.and_then(|message| message.person.as_ref()),
        state,
        session_ctx,
        now_ts,
    )
    .await;
    let safety = social_read::build_safety_ctx(&authority, &SessionKind::Mind);
    let recent_messages =
        social_read::fetch_recent_messages(store, conversation_id, messages).await;
    let social_relations = social_read::fetch_social_relations(
        store,
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let relationship_memories = social_read::fetch_relationship_memories(
        store,
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let relevant_memories = social_read::fetch_relevant_memories(
        store,
        messages,
        &[],
        first_message.and_then(|message| message.identity.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let open_loops = social_read::fetch_open_loops(
        store,
        first_message.and_then(|message| message.person.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.map(|message| &message.conversation),
        chrono::Utc::now().timestamp(),
    )
    .await;
    let actions: Vec<ActionBriefCtx> = concurrent_summaries
        .iter()
        .map(|(id, kind, task)| ActionBriefCtx {
            id: id.clone(),
            kind: kind.clone(),
            task: task.clone(),
        })
        .collect();
    let thoughts = social_read::fetch_thoughts(
        store,
        first_message.and_then(|message| message.identity.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;

    let ctx = MindContext {
        identity,
        now,
        profile,
        person,
        conversation,
        group,
        recent_messages,
        actions,
        timing,
        safety,
        social_relations,
        relationship_memories,
        relevant_memories,
        open_loops,
        thoughts,
    };
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
    interests.sort_by(|a, b| {
        b.intensity
            .partial_cmp(&a.intensity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let interests: Vec<InterestCtx> = interests
        .iter()
        .take(10)
        .map(|i| InterestCtx {
            topic: i.topic.clone(),
            intensity: pct(i.intensity),
        })
        .collect();

    let mood = match actor.affect.valence {
        v if v > 0.3 => "positive",
        v if v < -0.3 => "low",
        _ => "neutral",
    }
    .into();

    let energy = match actor.affect.arousal {
        a if a > 0.6 => "high energy",
        a if a < 0.3 => "low energy",
        _ => "moderate energy",
    }
    .into();

    let current_msg = messages.first();
    let conversation_summary = fetch_conversation_summary(store, conversation).await;
    let action_task = fetch_current_action_task(store, &session_ctx.action_id.0).await;
    let identity_id = current_msg.and_then(|m| m.identity.as_ref()).or_else(|| {
        conversation_summary
            .as_ref()
            .and_then(|summary| summary.identity.as_ref())
    });
    let profile_id = current_msg.and_then(|m| m.profile.as_ref()).or_else(|| {
        conversation_summary
            .as_ref()
            .and_then(|summary| summary.profile.as_ref())
    });
    let person_id = current_msg.and_then(|m| m.person.as_ref()).or_else(|| {
        conversation_summary
            .as_ref()
            .and_then(|summary| summary.person.as_ref())
    });
    let now_unix = now_ts.timestamp();

    let style_directive = session_ctx.style_directive.clone();
    let current_identity = match identity_id {
        Some(id) => {
            store
                .get_identity(id)
                .await
                .ok()
                .flatten()
                .map(|identity| CurrentIdentityCtx {
                    ref_id: identity.id.0,
                    display_name: identity.display_name,
                })
        }
        None => None,
    };
    let profile_info = match profile_id {
        Some(id) => store.get_profile(id).await.ok().flatten(),
        None => None,
    };
    let profile_person_link = match profile_id {
        Some(id) => store
            .get_person_for_profile(id)
            .await
            .ok()
            .flatten()
            .map(|(_, link)| link),
        None => None,
    };
    let current_profile = profile_id.map(|pid| CurrentProfileCtx {
        ref_id: pid.0.clone(),
        display_name: profile_info
            .as_ref()
            .and_then(|profile| profile.display_name.clone()),
        summary: profile_info
            .as_ref()
            .and_then(|profile| profile.summary.clone()),
        comm_style: profile_info
            .as_ref()
            .and_then(|profile| profile.comm_style.clone()),
        person_ref_id: profile_person_link
            .as_ref()
            .map(|link| link.person_id.0.clone()),
        person_link_status: profile_person_link
            .as_ref()
            .map(|link| link.status.as_str().to_string()),
        person_link_confidence: profile_person_link
            .as_ref()
            .map(|link| pct(link.confidence)),
    });
    let person_info = if let Some(pid) = person_id {
        Some(resolve_person_info(store, pid).await)
    } else {
        None
    };
    let current_person = person_id.map(|pid| CurrentPersonCtx {
        ref_id: pid.0.clone(),
        name: person_info.as_ref().and_then(|info| info.name.clone()),
        summary: person_info.as_ref().and_then(|info| info.summary.clone()),
        comm_style: person_info
            .as_ref()
            .and_then(|info| info.comm_style.clone()),
    });
    let conversation_ctx = conversation_summary
        .as_ref()
        .map(conversation_ctx_from_summary);
    let recent_messages = social_read::fetch_recent_messages(store, conversation, messages).await;
    let group_id = current_msg
        .and_then(|message| message.group.as_ref())
        .or_else(|| {
            conversation_summary
                .as_ref()
                .and_then(|summary| summary.group.as_ref())
        });
    let group = fetch_group_ctx(store, group_id).await;

    let (relationship, comm_style) = if let Some(pid) = person_id {
        let info = person_info
            .as_ref()
            .expect("person_info exists when person_id exists");
        let rel_ctx = actor.bonds.get(pid).map(|rel| {
            let tone = if rel.emotional_valence > 0.3 {
                "warm"
            } else if rel.emotional_valence < -0.3 {
                "strained"
            } else {
                "neutral"
            };
            RelationshipCtx {
                ref_id: pid.0.clone(),
                name: info.name.clone(),
                summary: info.summary.clone(),
                trust: pct(rel.trust),
                familiarity: pct(rel.familiarity),
                closeness: pct(rel.closeness),
                reliability: pct(rel.reliability),
                reciprocity: pct(rel.reciprocity),
                conflict_level: pct(rel.conflict_level),
                interactions: rel.interaction_count,
                inbound_count: rel.inbound_count,
                outbound_count: rel.outbound_count,
                proactive_consent: rel.proactive_consent.as_str().into(),
                response_cadence: rel.response_cadence.clone(),
                channel_preference: rel.channel_preference.clone(),
                tone: tone.into(),
                last_seen: info.last_seen.map(|ts| relative_duration(ts, now_unix)),
                last_inbound: (rel.last_inbound > 0)
                    .then(|| relative_duration(rel.last_inbound, now_unix)),
                last_outbound: (rel.last_outbound > 0)
                    .then(|| relative_duration(rel.last_outbound, now_unix)),
                first_met: info.first_seen.map(|ts| relative_duration(ts, now_unix)),
            }
        });
        let interaction_count = actor.bonds.get(pid).map_or(0, |r| r.interaction_count);
        let style = if let Some(profile_style) = profile_info
            .as_ref()
            .and_then(|profile| profile.comm_style.clone())
        {
            Some(profile_style)
        } else if interaction_count >= 10 {
            info.comm_style.clone().or(style_directive)
        } else {
            style_directive
        };
        (rel_ctx, style)
    } else {
        (
            None,
            profile_info
                .as_ref()
                .and_then(|profile| profile.comm_style.clone())
                .or(style_directive),
        )
    };

    let relevant_memories = {
        let mut supplemental_query_text = Vec::new();
        if let Some(task) = action_task.as_deref() {
            supplemental_query_text.push(task);
        }
        if let Some(summary) = conversation_summary
            .as_ref()
            .and_then(|summary| summary.summary.as_deref())
        {
            supplemental_query_text.push(summary);
        }
        social_read::fetch_relevant_memories(
            store,
            messages,
            &supplemental_query_text,
            identity_id,
            profile_id,
            person_id,
        )
        .await
    };
    let review_due_memories = review::fetch_review_due_memories(store, kind, now_unix).await;
    let conversation_backlog = review::fetch_conversation_backlog(store, kind, now_unix).await;
    let review_transcript = review::fetch_review_transcript(store, kind, session_ctx).await;
    let timing = social_read::build_timing_ctx(
        store,
        messages,
        conversation,
        person_id,
        state,
        session_ctx,
        now_ts,
    )
    .await;
    let safety = social_read::build_safety_ctx(authority, &SessionKind::Action(kind.clone()));
    let social_relations = social_read::fetch_social_relations(store, person_id).await;
    let relationship_memories =
        social_read::fetch_relationship_memories(store, profile_id, person_id).await;
    let open_loops =
        social_read::fetch_open_loops(store, person_id, profile_id, conversation, now_unix).await;

    let directives = if let Some(pid) = person_id {
        load_directives(store, &actor, pid, conversation)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };

    let thoughts = social_read::fetch_thoughts(store, identity_id, profile_id, person_id).await;
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
        action_task,
        identity_memories,
        traits,
        beliefs,
        interests,
        mood,
        energy,
        current_identity,
        current_profile,
        current_person,
        conversation: conversation_ctx,
        group,
        recent_messages,
        relationship,
        review_transcript,
        timing,
        safety,
        social_relations,
        relationship_memories,
        relevant_memories,
        review_due_memories,
        conversation_backlog,
        open_loops,
        directives,
        thoughts,
        cancelled_note,
        concurrent_actions,
        style: comm_style,
        authority: authority.as_str().to_string(),
        kind: kind.as_str().to_string(),
    };

    let base = env.get_template("action.j2")?.render(&ctx)?;
    let task = env.get_template(action_task_template(kind))?.render(&ctx)?;
    Ok(format!("{base}\n\n{task}"))
}

async fn fetch_current_action_task(store: &Arc<dyn Store>, action_id: &str) -> Option<String> {
    store
        .get_action_run(action_id)
        .await
        .ok()
        .flatten()
        .map(|run| run.task)
}

async fn fetch_current_profile_ctx(
    store: &Arc<dyn Store>,
    profile_id: Option<&ProfileId>,
) -> Option<CurrentProfileCtx> {
    let profile_id = profile_id?;
    let profile_info = store.get_profile(profile_id).await.ok().flatten();
    let profile_person_link = store
        .get_person_for_profile(profile_id)
        .await
        .ok()
        .flatten()
        .map(|(_, link)| link);

    Some(CurrentProfileCtx {
        ref_id: profile_id.0.clone(),
        display_name: profile_info
            .as_ref()
            .and_then(|profile| profile.display_name.clone()),
        summary: profile_info
            .as_ref()
            .and_then(|profile| profile.summary.clone()),
        comm_style: profile_info
            .as_ref()
            .and_then(|profile| profile.comm_style.clone()),
        person_ref_id: profile_person_link
            .as_ref()
            .map(|link| link.person_id.0.clone()),
        person_link_status: profile_person_link
            .as_ref()
            .map(|link| link.status.as_str().to_string()),
        person_link_confidence: profile_person_link
            .as_ref()
            .map(|link| pct(link.confidence)),
    })
}

async fn recall_identity_name(store: &Arc<dyn Store>) -> String {
    let query = RecallQuery::by_text("my name, who I am", 1)
        .with_kind(MemoryKind::Semantic)
        .with_actor_subject()
        .with_min_importance(0.5);
    match store.recall(&query).await {
        Ok(memories) if !memories.is_empty() => memories[0].content.clone(),
        _ => "an unnamed being".into(),
    }
}

async fn recall_identity_memories(store: &Arc<dyn Store>) -> Vec<String> {
    let query = RecallQuery::by_text("my name, who I am, my identity", 5)
        .with_kind(MemoryKind::Semantic)
        .with_actor_subject()
        .with_min_importance(0.5);
    match store.recall(&query).await {
        Ok(memories) => memories.into_iter().map(|m| m.content).collect(),
        Err(_) => vec![],
    }
}

async fn fetch_conversation_summary(
    store: &Arc<dyn Store>,
    conversation: Option<&ConversationId>,
) -> Option<ConversationSummary> {
    let conversation = conversation?;
    store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
}

fn conversation_ctx_from_summary(summary: &ConversationSummary) -> ConversationCtx {
    ConversationCtx {
        ref_id: summary.id.0.clone(),
        summary: summary.summary.clone(),
    }
}

async fn fetch_group_ctx(store: &Arc<dyn Store>, group_id: Option<&GroupId>) -> Option<GroupCtx> {
    let group_id = group_id?;
    match store.get_group(group_id).await {
        Ok(Some(group)) => {
            let mut members = Vec::new();
            for member in group.members.iter().take(12) {
                let info = resolve_person_info(store, member).await;
                members.push(GroupMemberCtx {
                    ref_id: member.0.clone(),
                    name: info.name,
                });
            }
            members.sort_by(|a, b| {
                a.name
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.name.as_deref().unwrap_or(""))
                    .then_with(|| a.ref_id.cmp(&b.ref_id))
            });
            Some(GroupCtx {
                ref_id: group.id.0,
                name: Some(group.name),
                gateway_id: Some(group.gateway_id),
                external_id: Some(group.external_id),
                context: Some(group.context.as_str().to_string()),
                member_count: group.members.len(),
                members,
            })
        }
        Ok(None) => Some(GroupCtx {
            ref_id: group_id.0.clone(),
            name: None,
            gateway_id: None,
            external_id: None,
            context: None,
            member_count: 0,
            members: vec![],
        }),
        Err(_) => None,
    }
}

async fn resolve_person_for_mind(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
) -> Option<PersonContext> {
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
            comm_style: info.comm_style,
            authority: rel.authority.as_str().to_string(),
            trust: pct(rel.trust),
            familiarity: pct(rel.familiarity),
            closeness: pct(rel.closeness),
            reliability: pct(rel.reliability),
            reciprocity: pct(rel.reciprocity),
            conflict_level: pct(rel.conflict_level),
            proactive_consent: rel.proactive_consent.as_str().into(),
            response_cadence: rel.response_cadence.clone(),
            channel_preference: rel.channel_preference.clone(),
            last_seen,
        })
    } else {
        Some(PersonContext {
            ref_id: person_id.0.clone(),
            name: info.name,
            summary: info.summary,
            comm_style: info.comm_style,
            authority: "default".into(),
            trust: 0,
            familiarity: 0,
            closeness: 0,
            reliability: 0,
            reciprocity: 0,
            conflict_level: 0,
            proactive_consent: "unknown".into(),
            response_cadence: None,
            channel_preference: None,
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
        _ => PersonInfo {
            name: None,
            summary: None,
            comm_style: None,
            first_seen: None,
            last_seen: None,
        },
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
        summaries
            .into_iter()
            .find(|s| s.id == *conv)
            .and_then(|s| s.group)
    } else {
        None
    };

    let directives = store
        .get_directives_for_context(person, &authority, group.as_ref())
        .await?;
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
        if m == 1 {
            "1 minute ago".into()
        } else {
            format!("{m} minutes ago")
        }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 {
            "1 hour ago".into()
        } else {
            format!("{h} hours ago")
        }
    } else if secs < 604800 {
        let d = secs / 86400;
        if d == 1 {
            "1 day ago".into()
        } else {
            format!("{d} days ago")
        }
    } else if secs < 2592000 {
        let w = secs / 604800;
        if w == 1 {
            "1 week ago".into()
        } else {
            format!("{w} weeks ago")
        }
    } else if secs < 31536000 {
        let mo = secs / 2592000;
        if mo == 1 {
            "1 month ago".into()
        } else {
            format!("{mo} months ago")
        }
    } else {
        let y = secs / 31536000;
        if y == 1 {
            "1 year ago".into()
        } else {
            format!("{y} years ago")
        }
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, ActionKind, RunningState};
    use crate::core::handle::SharedState;
    use crate::identity::{
        Group, GroupContext, Identity, Person, PersonProfileStatus, Profile, Relation,
        RelationSource, RelationStatus, SocialRelation,
    };
    use crate::state::{
        BehaviorDirective, CoreTraits, Delta, DirectiveScope, GrowthConfig,
        RelationshipSignalUpdate,
    };
    use crate::store::{
        ActionMessageRecord, ActionRunRecord, ActionTurnRecord, IntentRecord, Memory, MemorySource,
        MemorySubject, MemoryType, MessageRole, OutboundDeliveryRecord, PrivacyCategory,
        SqliteStore, StoredMessage, Thought, ThoughtKind, ToolCallRecord,
    };
    use async_trait::async_trait;
    use gateway::GatewayRouter;
    use inference::{
        Capability, ChatRequest, ChatResponse, ChatStream, FinishReason, InferenceEndpoint,
        InferenceProtocol, InferenceRouterBuilder, OpenAiCompatibleBridge, Reasoning,
        SamplingConfig, Usage,
    };
    use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};
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
            anyhow::bail!("noop bridge is not used by prompt tests")
        }
    }

    fn test_router() -> inference::InferenceRouter {
        InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap()
    }

    #[test]
    fn mind_style_directive_uses_adaptation_not_mirroring() {
        let template = include_str!("templates/mind.j2");
        assert!(template.contains("approximate length, formality, pace, and energy"));
        assert!(template.contains("Preserve your own voice"));
        assert!(!template.contains("mirrors how they communicate"));
    }

    #[test]
    fn prompts_use_transparent_identity_not_human_pretending() {
        let mind = include_str!("templates/mind.j2");
        let action = include_str!("templates/action.j2");

        for template in [mind, action] {
            assert!(template.contains("transparent digital being"));
            assert!(template.contains("Do not pretend to be human"));
            assert!(!template.contains("living being"));
            assert!(!template.contains("Write like a real person"));
        }
    }

    #[test]
    fn action_kind_task_templates_are_registered() {
        let env = make_env();
        let cases = [
            (ActionKind::Respond, "action_task_respond.j2"),
            (ActionKind::Review, "action_task_review.j2"),
            (ActionKind::Ruminate, "action_task_ruminate.j2"),
            (ActionKind::Consolidate, "action_task_consolidate.j2"),
            (ActionKind::Outreach, "action_task_outreach.j2"),
            (ActionKind::Research, "action_task_research.j2"),
        ];

        for (kind, template) in cases {
            assert_eq!(action_task_template(&kind), template);
            assert!(env.get_template(template).is_ok());
        }
    }

    #[test]
    fn respond_task_prioritizes_visible_reply_over_durable_learning() {
        let template = include_str!("templates/action_task_respond.j2");

        assert!(template.contains("Keep the live turn focused on the visible reply"));
        assert!(template.contains("Post-turn review handles normal memory"));
        assert!(template.contains("Use form_memory only for urgent facts"));
        assert!(!template.contains("reflect - note if something shifted"));
        assert!(!template.contains("save anything urgent or important"));
    }

    #[tokio::test]
    async fn review_prompt_includes_source_action_transcript() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let conversation = ConversationId("relay:local".into());
        let source_action = "source-action-1";

        store
            .start_action_run(&ActionRunRecord {
                action_id: source_action.into(),
                kind: "respond".into(),
                task: "Respond to message".into(),
                conversation: Some(conversation.clone()),
                started_at: 1000,
                ended_at: None,
                status: "running".into(),
                responded: false,
                attempts: 0,
            })
            .await
            .unwrap();
        store
            .append_action_message(&ActionMessageRecord {
                action_id: source_action.into(),
                role: "user".into(),
                conversation: Some(conversation.clone()),
                source_gateway_id: Some("relay".into()),
                source_message_id: Some("msg-1".into()),
                sender_external_id: Some("local".into()),
                reply_external_id: Some("local".into()),
                content: Some("hello".into()),
                created_at: 1001,
            })
            .await
            .unwrap();
        store
            .append_action_message(&ActionMessageRecord {
                action_id: source_action.into(),
                role: "assistant".into(),
                conversation: Some(conversation.clone()),
                source_gateway_id: None,
                source_message_id: None,
                sender_external_id: None,
                reply_external_id: Some("local".into()),
                content: Some("hi there".into()),
                created_at: 1002,
            })
            .await
            .unwrap();
        store
            .append_action_turn(&ActionTurnRecord {
                action_id: source_action.into(),
                turn: 0,
                attempt: 1,
                prompt_hash: "abc123".into(),
                model: Some("model-a".into()),
                finish: Some("tool_calls".into()),
                input_tokens: Some(20),
                output_tokens: Some(5),
                text_len: 0,
                reasoning_len: 0,
                tool_call_count: 1,
                created_at: 1002,
            })
            .await
            .unwrap();
        store
            .append_tool_call(&ToolCallRecord {
                action_id: source_action.into(),
                turn: 0,
                call_id: "call-1".into(),
                name: "send_message".into(),
                args: serde_json::json!({"content": "hi there"}),
                result: serde_json::json!({"result": "Message sent."}),
                success: true,
                started_at: 1003,
                ended_at: 1004,
            })
            .await
            .unwrap();
        store
            .log_thought(&Thought {
                timestamp: 1003,
                kind: ThoughtKind::Observation,
                content: "Sam may prefer shorter greetings.".into(),
                importance: 0.8,
                confidence: 0.7,
                action_id: Some(source_action.into()),
                memories_accessed: vec![MemoryId("memory-greeting-style".into())],
                subjects: vec![],
            })
            .await
            .unwrap();
        store
            .append_outbound_delivery(&OutboundDeliveryRecord {
                action_id: source_action.into(),
                conversation: Some(conversation.clone()),
                gateway_id: "relay".into(),
                external_id: "local".into(),
                status: "delivered".into(),
                error: None,
                attempted_at: 1004,
            })
            .await
            .unwrap();
        store
            .finish_action_run(
                source_action,
                1005,
                "completed",
                true,
                1,
                vec![MemoryId("memory-created-review".into())],
                vec![MemoryId("memory-greeting-style".into())],
            )
            .await
            .unwrap();

        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(CoreTraits::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let state = StateHandle::new(shared, delta_tx);
        let ctx = SessionContext {
            action_id: ActionId("review-action".into()),
            kind: SessionKind::Action(ActionKind::Review),
            messages: vec![],
            conversation: Some(conversation.clone()),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: Some(format!("Post-turn review for action {source_action}")),
            concurrent_summaries: vec![],
            state: state.clone(),
            store: store_dyn.clone(),
            media_store: None,
            router: Arc::new(test_router()),
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

        let prompt = build_system_prompt(
            &state,
            &store_dyn,
            &ctx.kind,
            &ctx.messages,
            Some(&conversation),
            &ctx,
            &Authority::Default,
        )
        .await
        .unwrap();

        assert!(prompt.contains("## Source action transcript"));
        assert!(prompt.contains("source-action-1"));
        assert!(prompt.contains("- task: Respond to message"));
        assert!(prompt.contains("- status: completed, responded: true, attempts: 1"));
        assert!(prompt.contains("user [local] msg-1: hello"));
        assert!(prompt.contains("assistant [local]: hi there"));
        assert!(prompt.contains("attempt 1, turn 0, model model-a"));
        assert!(prompt.contains("send_message success=true"));
        assert!(prompt.contains("Message sent."));
        assert!(prompt.contains("### Source action thoughts"));
        assert!(prompt.contains("Sam may prefer shorter greetings."));
        assert!(prompt.contains("memories: memory-greeting-style"));
        assert!(prompt.contains("### Outcome memory trace"));
        assert!(prompt.contains("formed memories: memory-created-review"));
        assert!(prompt.contains("recalled memories: memory-greeting-style"));
        assert!(prompt.contains("delivery relay:local: delivered"));
    }

    #[tokio::test]
    async fn outreach_prompt_uses_conversation_target_context_without_current_messages() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let identity_id = IdentityId("identity-outreach".into());
        let profile_id = ProfileId("profile-outreach".into());
        let person_id = PersonId("person-outreach".into());
        let conversation = ConversationId("relay:outreach".into());
        let group = GroupId("relay:team-chat".into());
        let now = chrono::Utc::now().timestamp();

        store
            .add_identity(&Identity {
                id: identity_id.clone(),
                gateway_id: "relay".into(),
                external_id: "local".into(),
                display_name: Some("Sam".into()),
                metadata: None,
                created_at: now,
                last_seen_at: now,
            })
            .await
            .unwrap();
        store
            .add_profile(&Profile {
                id: profile_id.clone(),
                display_name: Some("Sam relay".into()),
                summary: Some("Profile summary for outreach.".into()),
                comm_style: Some("Profile prefers short scheduling messages.".into()),
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .link_identity_to_profile(&identity_id, &profile_id, 1.0, None)
            .await
            .unwrap();
        store
            .add_person(&Person {
                id: person_id.clone(),
                name: Some("Sam".into()),
                summary: Some("Person summary for outreach.".into()),
                comm_style: Some("Person-level style for verified outreach.".into()),
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &profile_id,
                &person_id,
                PersonProfileStatus::Verified,
                1.0,
                None,
            )
            .await
            .unwrap();
        store
            .add_group(&Group {
                id: group.clone(),
                name: "Relay Team Chat".into(),
                gateway_id: "relay".into(),
                external_id: "team-chat".into(),
                context: GroupContext::Work,
                members: vec![person_id.clone()],
            })
            .await
            .unwrap();
        store
            .append_message(
                &conversation,
                Some("relay"),
                Some(&group),
                &StoredMessage {
                    timestamp: now,
                    role: MessageRole::User,
                    content: "please check in tomorrow".into(),
                    identity: Some(identity_id.clone()),
                    profile: Some(profile_id.clone()),
                    person: Some(person_id.clone()),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-outreach-source".into()),
                    sender_external_id: Some("local".into()),
                    reply_external_id: Some("local".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .update_conversation_summary(
                &conversation,
                "Sam asked for a proactive follow-up.",
                &["msg-outreach-source".into()],
            )
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-deployment-checklist".into()),
                content: "Sam wants deployment checklist follow-ups to mention release readiness."
                    .into(),
                importance: 0.8,
                confidence: 0.9,
                subjects: vec![
                    MemorySubject::profile(profile_id.clone(), Some("about".into()), 1.0),
                    MemorySubject::person(person_id.clone(), Some("about".into()), 1.0),
                ],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .start_action_run(&ActionRunRecord {
                action_id: "outreach-prompt-test".into(),
                kind: "outreach".into(),
                task: "Ask Sam whether the deployment checklist is ready".into(),
                conversation: Some(conversation.clone()),
                started_at: now,
                ended_at: None,
                status: "running".into(),
                responded: false,
                attempts: 0,
            })
            .await
            .unwrap();

        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let mut actor = ActorState::new(CoreTraits::default());
        actor.set_relationship_config(&person_id, Some(Authority::Default));
        if let Some(rel) = actor.bonds.get_mut(&person_id) {
            rel.response_cadence = Some("reply within one business day".into());
            rel.channel_preference = Some("relay for proactive check-ins".into());
        }
        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor),
            config: RwLock::new(GrowthConfig::default()),
        });
        let state = StateHandle::new(shared, delta_tx);
        let ctx = SessionContext {
            action_id: ActionId("outreach-prompt-test".into()),
            kind: SessionKind::Action(ActionKind::Outreach),
            messages: vec![],
            conversation: Some(conversation.clone()),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: state.clone(),
            store: store_dyn.clone(),
            media_store: None,
            router: Arc::new(test_router()),
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

        let prompt = build_system_prompt(
            &state,
            &store_dyn,
            &ctx.kind,
            &ctx.messages,
            Some(&conversation),
            &ctx,
            &Authority::Default,
        )
        .await
        .unwrap();

        assert!(prompt.contains("## Current gateway identity"));
        assert!(prompt.contains("identity-outreach"));
        assert!(prompt.contains("Profile summary for outreach."));
        assert!(prompt.contains("Profile prefers short scheduling messages."));
        assert!(prompt.contains("Person summary for outreach."));
        assert!(prompt.contains("## Current group"));
        assert!(prompt.contains("- id: relay:team-chat"));
        assert!(prompt.contains("- name: Relay Team Chat"));
        assert!(prompt.contains("- context: work"));
        assert!(prompt.contains("person-outreach (Sam)"));
        assert!(prompt.contains("Use group membership as local participant context only."));
        assert!(prompt.contains("## Current action"));
        assert!(prompt.contains("- kind: outreach"));
        assert!(prompt.contains("- task: Ask Sam whether the deployment checklist is ready"));
        assert!(prompt.contains("## Recent conversation"));
        assert!(prompt.contains("user [local] msg-outreach-source: please check in tomorrow"));
        assert!(prompt.contains("## Relevant memories"));
        assert!(prompt.contains("memory-deployment-checklist"));
        assert!(prompt.contains("release readiness"));
        assert!(prompt.contains("Response cadence preference: reply within one business day"));
        assert!(prompt.contains("Channel preference: relay for proactive check-ins"));
        assert!(prompt.contains("Sam asked for a proactive follow-up."));
    }

    #[tokio::test]
    async fn group_directive_appears_after_first_group_inbound_is_persisted() {
        let store = Arc::new(SqliteStore::open_in_memory(4).unwrap());
        let store_dyn: Arc<dyn Store> = store.clone();
        let person_id = PersonId("person-alice".into());
        let profile_id = ProfileId("profile-alice".into());
        let identity_id = IdentityId("identity-alice".into());
        let conversation = ConversationId("discord:channel-1".into());
        let group = GroupId("discord:guild-1".into());
        let now = chrono::Utc::now().timestamp();

        store
            .add_person(&Person {
                id: person_id.clone(),
                name: Some("Alice".into()),
                summary: Some("Alice coordinates deployment releases.".into()),
                comm_style: Some("Prefers practical release notes with direct status.".into()),
                first_seen: now,
                last_seen: now,
            })
            .await
            .unwrap();
        for (id, name) in [
            ("person-bob", "Bob"),
            ("person-carol", "Carol"),
            ("person-dave", "Dave"),
            ("person-eve", "Eve"),
        ] {
            store
                .add_person(&Person {
                    id: PersonId(id.into()),
                    name: Some(name.into()),
                    summary: None,
                    comm_style: None,
                    first_seen: now,
                    last_seen: now,
                })
                .await
                .unwrap();
        }
        store
            .add_group(&Group {
                id: group.clone(),
                name: "Deploy Guild".into(),
                gateway_id: "discord".into(),
                external_id: "guild-1".into(),
                context: GroupContext::Work,
                members: vec![
                    person_id.clone(),
                    PersonId("person-bob".into()),
                    PersonId("person-eve".into()),
                ],
            })
            .await
            .unwrap();
        store
            .add_profile(&Profile {
                id: profile_id.clone(),
                display_name: Some("Alice".into()),
                summary: Some("Alice's Discord profile tracks deployment coordination.".into()),
                comm_style: Some("On Discord, Alice prefers terse release status.".into()),
                first_seen: now,
                last_seen: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .attach_profile_to_person(
                &profile_id,
                &person_id,
                PersonProfileStatus::Verified,
                0.95,
                Some(&serde_json::json!({"test": "profile prompt context"})),
            )
            .await
            .unwrap();
        store
            .upsert_relation(&SocialRelation {
                person_a: person_id.clone(),
                person_b: PersonId("person-bob".into()),
                relation: Relation::Coworker,
                direction: Relation::Coworker.default_direction(),
                confidence: 0.8,
                status: RelationStatus::Confirmed,
                evidence: Some(serde_json::json!({"message_id": "social-msg-1"})),
                source_kind: RelationSource::Stated,
                asserted_by: Some(person_id.clone()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .upsert_relation(&SocialRelation {
                person_a: PersonId("person-carol".into()),
                person_b: person_id.clone(),
                relation: Relation::Friend,
                direction: Relation::Friend.default_direction(),
                confidence: 0.45,
                status: RelationStatus::Hypothesis,
                evidence: Some(serde_json::json!({"reason": "seen together in channel"})),
                source_kind: RelationSource::Inferred,
                asserted_by: None,
                created_at: now,
                updated_at: now - 1,
            })
            .await
            .unwrap();
        store
            .upsert_relation(&SocialRelation {
                person_a: person_id.clone(),
                person_b: PersonId("person-dave".into()),
                relation: Relation::Friend,
                direction: Relation::Friend.default_direction(),
                confidence: 0.9,
                status: RelationStatus::Denied,
                evidence: Some(serde_json::json!({"message_id": "social-msg-2"})),
                source_kind: RelationSource::Stated,
                asserted_by: Some(person_id.clone()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .upsert_relation(&SocialRelation {
                person_a: PersonId("person-bob".into()),
                person_b: PersonId("person-dave".into()),
                relation: Relation::Friend,
                direction: Relation::Friend.default_direction(),
                confidence: 0.9,
                status: RelationStatus::Confirmed,
                evidence: Some(serde_json::json!({"message_id": "social-msg-3"})),
                source_kind: RelationSource::Stated,
                asserted_by: Some(PersonId("person-bob".into())),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        store
            .add_directive(&BehaviorDirective {
                id: "group-directive".into(),
                scope: DirectiveScope::Group(group.clone()),
                directive: "Use the group norm: keep deployment updates brief.".into(),
                set_by: person_id.clone(),
                priority: 10,
                active: true,
                created_at: now,
                expires_at: None,
            })
            .await
            .unwrap();

        let inbound = InboundMessage {
            message_id: "discord-msg-1".into(),
            gateway_id: "discord".into(),
            sender_external_id: "author-a".into(),
            sender_display_name: Some("Alice".into()),
            reply_external_id: "channel-1".into(),
            conversation: conversation.clone(),
            group: Some(group.clone()),
            identity: Some(identity_id.clone()),
            profile: Some(profile_id.clone()),
            person: Some(person_id.clone()),
            content: "deploy status?".into(),
            attachments: vec![],
            timestamp: now,
            metadata: serde_json::Value::Null,
        };
        store
            .append_message(
                &conversation,
                Some("discord"),
                Some(&group),
                &StoredMessage {
                    timestamp: now - 60,
                    role: MessageRole::User,
                    content: "previous deploy thread".into(),
                    identity: Some(identity_id.clone()),
                    profile: Some(profile_id.clone()),
                    person: Some(person_id.clone()),
                    source_gateway_id: Some("discord".into()),
                    source_message_id: Some("discord-msg-0".into()),
                    sender_external_id: Some("author-a".into()),
                    reply_external_id: Some("channel-1".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .append_message(
                &conversation,
                Some("discord"),
                Some(&group),
                &StoredMessage {
                    timestamp: now,
                    role: MessageRole::User,
                    content: inbound.content.clone(),
                    identity: Some(identity_id),
                    profile: Some(profile_id.clone()),
                    person: Some(person_id.clone()),
                    source_gateway_id: Some("discord".into()),
                    source_message_id: Some("discord-msg-1".into()),
                    sender_external_id: Some("author-a".into()),
                    reply_external_id: Some("channel-1".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();
        store
            .update_conversation_summary(
                &conversation,
                "Alice asked for concise deployment status in this channel.",
                &[String::from("discord-msg-1")],
            )
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-current-profile".into()),
                kind: MemoryKind::Semantic,
                content: "Alice prefers brief deployment status updates.".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                confidence: 0.8,
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-current-boundary".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Boundary,
                content: "Do not mention surprise party details in shared channels.".into(),
                source: MemorySource::Reflection,
                importance: 0.85,
                confidence: 0.9,
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        for idx in 0..45 {
            store
                .store_memory(&Memory {
                    id: MemoryId(format!("memory-recent-generic-{idx}")),
                    kind: MemoryKind::Semantic,
                    memory_type: MemoryType::Fact,
                    content: format!("Recent generic observation {idx}."),
                    source: MemorySource::Reflection,
                    importance: 0.95,
                    confidence: 0.95,
                    created_at: now + idx,
                    subjects: vec![MemorySubject::profile(
                        profile_id.clone(),
                        Some("about".into()),
                        1.0,
                    )],
                    ..Memory::default()
                })
                .await
                .unwrap();
        }
        store
            .store_memory(&Memory {
                id: MemoryId("memory-other-profile".into()),
                kind: MemoryKind::Semantic,
                content: "Other profile wants verbose deployment status updates.".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                confidence: 0.8,
                subjects: vec![MemorySubject::profile(
                    ProfileId("profile-other".into()),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-sensitive-profile".into()),
                kind: MemoryKind::Semantic,
                content: "Alice has a secret deployment credential.".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                confidence: 0.8,
                sensitivity: 0.95,
                privacy_category: PrivacyCategory::Secret,
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-due-review".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Hypothesis,
                content: "Old launch checklist may be stale.".into(),
                source: MemorySource::Reflection,
                importance: 0.7,
                confidence: 0.6,
                next_review_at: Some(now - 60),
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .store_memory(&Memory {
                id: MemoryId("memory-secret-due-review".into()),
                kind: MemoryKind::Semantic,
                memory_type: MemoryType::Hypothesis,
                content: "Secret launch credential should be rotated.".into(),
                source: MemorySource::Reflection,
                importance: 0.9,
                confidence: 0.7,
                sensitivity: 0.95,
                privacy_category: PrivacyCategory::Secret,
                next_review_at: Some(now - 60),
                subjects: vec![MemorySubject::profile(
                    profile_id.clone(),
                    Some("about".into()),
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();
        store
            .log_thought(&Thought {
                timestamp: now - 30,
                kind: ThoughtKind::Reflection,
                content: "Alice seemed worried about deployment risk.".into(),
                importance: 0.8,
                confidence: 0.75,
                action_id: Some("action-current-thought".into()),
                memories_accessed: vec![MemoryId("memory-current-profile".into())],
                subjects: vec![
                    MemorySubject::profile(profile_id.clone(), Some("about".into()), 1.0),
                    MemorySubject::person(person_id.clone(), Some("about".into()), 1.0),
                ],
            })
            .await
            .unwrap();
        store
            .log_thought(&Thought {
                timestamp: now - 20,
                kind: ThoughtKind::Reflection,
                content: "Bob seemed worried about hiring risk.".into(),
                importance: 0.95,
                confidence: 0.95,
                action_id: Some("action-other-thought".into()),
                memories_accessed: vec![],
                subjects: vec![MemorySubject::profile(
                    ProfileId("profile-bob".into()),
                    Some("about".into()),
                    1.0,
                )],
            })
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-current-followup".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Ask Alice whether the deployment finished cleanly".into(),
                person: Some(person_id.clone()),
                profile: Some(profile_id.clone()),
                conversation: Some(conversation.clone()),
                fire_at: Some(now + 3600),
                condition: None,
                recurrence: None,
                priority: 75,
                dedupe_key: Some("followup:alice:deployment".into()),
                source_action: None,
                source_memory: Some(MemoryId("memory-deploy-followup".into())),
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();
        store
            .create_intent(&IntentRecord {
                id: "intent-other-person".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Ask Bob about unrelated hiring updates".into(),
                person: Some(PersonId("person-bob".into())),
                profile: None,
                conversation: None,
                fire_at: Some(now + 1800),
                condition: None,
                recurrence: None,
                priority: 100,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: now,
                updated_at: now,
                last_fired_at: None,
                owner_approved: false,
            })
            .await
            .unwrap();

        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let mut actor = ActorState::new(CoreTraits::default());
        actor.set_relationship_config(&person_id, Some(Authority::Default));
        if let Some(rel) = actor.bonds.get_mut(&person_id) {
            rel.response_cadence = Some("reply within one business day".into());
            rel.channel_preference = Some("Discord for deployment coordination".into());
        }
        actor.apply_delta(
            &Delta {
                relationship_signal_updates: vec![RelationshipSignalUpdate {
                    person: person_id.clone(),
                    closeness_delta: 0.4,
                    reliability_delta: 0.7,
                    reciprocity_delta: 0.5,
                    conflict_delta: 0.1,
                    reason: "prompt test signals".into(),
                }],
                ..Delta::default()
            },
            &GrowthConfig::default(),
        );
        let shared = Arc::new(SharedState {
            actor: RwLock::new(actor),
            config: RwLock::new(GrowthConfig::default()),
        });
        let state = StateHandle::new(shared, delta_tx);
        let ctx = SessionContext {
            action_id: ActionId("prompt-test".into()),
            kind: SessionKind::Action(ActionKind::Respond),
            messages: vec![inbound],
            conversation: Some(conversation.clone()),
            authority: Authority::Default,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: state.clone(),
            store: store_dyn.clone(),
            media_store: None,
            router: Arc::new(test_router()),
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
        ctx.typing.write().unwrap().insert(
            (conversation.clone(), "discord".into(), "author-a".into()),
            chrono::Utc::now().timestamp(),
        );

        let prompt = build_system_prompt(
            &state,
            &store_dyn,
            &ctx.kind,
            &ctx.messages,
            Some(&conversation),
            &ctx,
            &Authority::Default,
        )
        .await
        .unwrap();

        assert!(prompt.contains("## Active directives"));
        assert!(prompt.contains("Use the group norm: keep deployment updates brief."));
        assert!(prompt.contains("## Current conversation"));
        assert!(prompt.contains("Alice asked for concise deployment status in this channel."));
        assert!(prompt.contains("## Current group"));
        assert!(prompt.contains("- id: discord:guild-1"));
        assert!(prompt.contains("- name: Deploy Guild"));
        assert!(prompt.contains("- gateway: discord"));
        assert!(prompt.contains("- external id: guild-1"));
        assert!(prompt.contains("- context: work"));
        assert!(prompt.contains("- observed member count: 3"));
        assert!(prompt.contains("person-eve (Eve)"));
        assert!(prompt.contains("Do not treat group membership or display names as proof"));
        assert!(prompt.contains("## Timing and delivery"));
        assert!(prompt.contains("Gateway: discord (unregistered, not connected)"));
        assert!(prompt.contains("Currently typing:"));
        assert!(prompt.contains("discord:author-a (current sender), active for"));
        assert!(prompt.contains("## Safety boundaries"));
        assert!(prompt.contains("Authority: default"));
        assert!(prompt.contains("Sensitive memory access: conservative recall only"));
        assert!(prompt.contains(
            "Third-party outreach: third-party outreach requires a verified active target"
        ));
        assert!(prompt.contains("## Recent conversation"));
        assert!(prompt.contains("user [author-a] discord-msg-0: previous deploy thread"));
        assert!(!prompt.contains("discord-msg-1: deploy status?"));
        assert!(prompt.contains(
            "Relationship signals: closeness 40%, reliability 70%, reciprocity 50%, conflict 10%"
        ));
        assert!(prompt.contains("Response cadence preference: reply within one business day"));
        assert!(prompt.contains("Channel preference: Discord for deployment coordination"));
        assert!(prompt.contains("Alice coordinates deployment releases."));
        assert!(prompt.contains("Prefers practical release notes with direct status."));
        assert!(prompt.contains("## Relevant memories"));
        assert!(prompt.contains(
            "[memory-current-profile, current_profile, fact, stated, importance 90%, confidence 80%]"
        ));
        assert!(prompt.contains("Alice prefers brief deployment status updates."));
        assert!(prompt.contains("## Relationship memory pack"));
        assert!(
            prompt.contains(
                "[memory-current-boundary, current_profile, boundary, stated, importance 85%, confidence 90%]"
            )
        );
        assert!(prompt.contains("Do not mention surprise party details in shared channels."));
        assert!(!prompt.contains("Other profile wants verbose deployment status updates."));
        assert!(!prompt.contains("Alice has a secret deployment credential."));
        assert!(prompt.contains("## Social context"));
        assert!(prompt.contains(
            "person-alice (Alice) -> person-bob (Bob): coworker direction=bidirectional"
        ));
        assert!(
            prompt.contains(
                "(confirmed, confidence 80%, source stated, asserted by person-alice (Alice), evidence message social-msg-1)"
            )
        );
        assert!(prompt.contains(
            "person-carol (Carol) -> person-alice (Alice): friend direction=bidirectional"
        ));
        assert!(prompt.contains(
            "(hypothesis, confidence 45%, source inferred, evidence reason: seen together in channel)"
        ));
        assert!(!prompt.contains("person-alice (Alice) -> person-dave (Dave): friend"));
        assert!(!prompt.contains("person-bob (Bob) -> person-dave (Dave): friend"));
        assert!(prompt.contains("## Open loops"));
        assert!(prompt.contains("intent-current-followup"));
        assert!(prompt.contains("Ask Alice whether the deployment finished cleanly"));
        assert!(prompt.contains("priority 75"));
        assert!(prompt.contains("source memory memory-deploy-followup"));
        assert!(!prompt.contains("## Memories due for review"));
        assert!(!prompt.contains("Old launch checklist may be stale."));
        assert!(!prompt.contains("Ask Bob about unrelated hiring updates"));
        assert!(prompt.contains("## Recent thoughts"));
        assert!(prompt.contains("Alice seemed worried about deployment risk."));
        assert!(!prompt.contains("Bob seemed worried about hiring risk."));
        assert!(!prompt.contains("## Conversation summary backlog"));
        assert!(prompt.contains("A new message just arrived."));
        assert!(!prompt.contains("Post-turn review."));

        let mind_kind = SessionKind::Mind;
        let mind_prompt = build_system_prompt(
            &state,
            &store_dyn,
            &mind_kind,
            &ctx.messages,
            Some(&conversation),
            &ctx,
            &Authority::Default,
        )
        .await
        .unwrap();
        assert!(mind_prompt.contains("## Social context"));
        assert!(mind_prompt.contains(
            "person-alice (Alice) -> person-bob (Bob): coworker direction=bidirectional"
        ));
        assert!(mind_prompt.contains("asserted by person-alice (Alice)"));
        assert!(mind_prompt.contains("evidence message social-msg-1"));
        assert!(!mind_prompt.contains("person-bob (Bob) -> person-dave (Dave): friend"));
        assert!(mind_prompt.contains("## Relationship memory pack"));
        assert!(mind_prompt.contains("Do not mention surprise party details in shared channels."));
        assert!(mind_prompt.contains("## Relevant memories"));
        assert!(mind_prompt.contains(
            "[memory-current-profile, current_profile, fact, stated, importance 90%, confidence 80%]"
        ));
        assert!(mind_prompt.contains("Alice prefers brief deployment status updates."));
        assert!(!mind_prompt.contains("Other profile wants verbose deployment status updates."));
        assert!(!mind_prompt.contains("Alice has a secret deployment credential."));
        assert!(mind_prompt.contains("## Current conversation"));
        assert!(mind_prompt.contains("Alice asked for concise deployment status in this channel."));
        assert!(mind_prompt.contains("## Current group"));
        assert!(mind_prompt.contains("- id: discord:guild-1"));
        assert!(mind_prompt.contains("- name: Deploy Guild"));
        assert!(mind_prompt.contains("- context: work"));
        assert!(mind_prompt.contains("- observed member count: 3"));
        assert!(mind_prompt.contains("person-eve (Eve)"));
        assert!(mind_prompt.contains("Do not treat group membership or display names as proof"));
        assert!(mind_prompt.contains("## Current profile"));
        assert!(mind_prompt.contains("Alice's Discord profile tracks deployment coordination."));
        assert!(mind_prompt.contains("On Discord, Alice prefers terse release status."));
        assert!(mind_prompt.contains("linked person id: person-alice"));
        assert!(mind_prompt.contains("Communication style: Prefers practical release notes"));
        assert!(mind_prompt.contains("## Timing and delivery"));
        assert!(mind_prompt.contains("Gateway: discord (unregistered, not connected)"));
        assert!(mind_prompt.contains("Currently typing:"));
        assert!(mind_prompt.contains("discord:author-a (current sender), active for"));
        assert!(mind_prompt.contains("## Safety boundaries"));
        assert!(mind_prompt.contains(
            "Relationship signals: closeness 40%, reliability 70%, reciprocity 50%, conflict 10%"
        ));
        assert!(mind_prompt.contains("Response cadence preference: reply within one business day"));
        assert!(mind_prompt.contains("Channel preference: Discord for deployment coordination"));
        assert!(mind_prompt.contains("## Recent conversation"));
        assert!(mind_prompt.contains("user [author-a] discord-msg-0: previous deploy thread"));
        assert!(!mind_prompt.contains("discord-msg-1: deploy status?"));
        assert!(mind_prompt.contains("## Open loops"));
        assert!(mind_prompt.contains("intent-current-followup"));
        assert!(mind_prompt.contains("source memory memory-deploy-followup"));
        assert!(!mind_prompt.contains("Ask Bob about unrelated hiring updates"));
        assert!(mind_prompt.contains("## Recent thoughts"));
        assert!(mind_prompt.contains("Alice seemed worried about deployment risk."));
        assert!(!mind_prompt.contains("Bob seemed worried about hiring risk."));

        let consolidate_kind = SessionKind::Action(ActionKind::Consolidate);
        let consolidate_prompt = build_system_prompt(
            &state,
            &store_dyn,
            &consolidate_kind,
            &ctx.messages,
            Some(&conversation),
            &ctx,
            &Authority::Default,
        )
        .await
        .unwrap();
        assert!(consolidate_prompt.contains("## Memories due for review"));
        assert!(consolidate_prompt.contains("memory-due-review"));
        assert!(consolidate_prompt.contains("Old launch checklist may be stale."));
        assert!(consolidate_prompt.contains("overdue by 1 minute"));
        assert!(consolidate_prompt.contains("memory-secret-due-review"));
        assert!(consolidate_prompt.contains("sensitive memory content redacted"));
        assert!(!consolidate_prompt.contains("Secret launch credential should be rotated."));
        assert!(consolidate_prompt.contains("## Conversation summary backlog"));
        assert!(consolidate_prompt.contains("discord:channel-1"));
        assert!(consolidate_prompt.contains("1 uncovered message of 2 total"));
        assert!(
            consolidate_prompt
                .contains("Alice asked for concise deployment status in this channel.")
        );
    }
}
