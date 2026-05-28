pub(crate) mod context;

use super::action::ActionKind;
use super::handle::StateHandle;
use super::review;
use super::social_read;
use super::tools::{SessionContext, SessionKind};
use crate::state::{ActorState, Authority};
use crate::store::{ConversationSummary, MemorySubjectType, MemoryType, Store};
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

    let actor_name = recall_identity_name(store).await;
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
                bond_role: bond_role(&rel.authority).into(),
                bond_state: bond_state(rel).into(),
                first_contact: rel.inbound_count <= 1 && info.name.is_none(),
                last_interaction_quality: interaction_quality(rel).into(),
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
        actor_name,
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
    match store
        .memories_for_subject(MemorySubjectType::Actor, "self", 24)
        .await
    {
        Ok(memories) => memories
            .iter()
            .find(|memory| memory.memory_type == MemoryType::IdentityClaim)
            .and_then(|memory| actor_name_from_identity_memory(&memory.content))
            .unwrap_or_else(|| "an unnamed Pamagotchi".into()),
        Err(_) => "an unnamed Pamagotchi".into(),
    }
}

async fn recall_identity_memories(store: &Arc<dyn Store>) -> Vec<String> {
    match store
        .memories_for_subject(MemorySubjectType::Actor, "self", 12)
        .await
    {
        Ok(mut memories) => {
            memories.sort_by(|a, b| {
                actor_identity_memory_rank(a)
                    .cmp(&actor_identity_memory_rank(b))
                    .then_with(|| {
                        b.importance
                            .partial_cmp(&a.importance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });
            memories.into_iter().take(8).map(|m| m.content).collect()
        }
        Err(_) => vec![],
    }
}

fn actor_name_from_identity_memory(content: &str) -> Option<String> {
    let rest = content.strip_prefix("My name is ")?;
    let name = rest
        .split(['.', ',', '\n'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(name.to_string())
}

fn actor_identity_memory_rank(memory: &crate::store::Memory) -> u8 {
    if memory.memory_type == MemoryType::IdentityClaim {
        0
    } else if memory.tags.iter().any(|tag| tag == "temperament") {
        1
    } else if memory.tags.iter().any(|tag| tag == "voice") {
        2
    } else if memory.tags.iter().any(|tag| tag == "first_contact") {
        3
    } else if memory.tags.iter().any(|tag| tag == "inner_life") {
        4
    } else if memory.tags.iter().any(|tag| tag == "baseline_story") {
        5
    } else {
        6
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
            bond_role: bond_role(&rel.authority).into(),
            bond_state: bond_state(rel).into(),
            last_interaction_quality: interaction_quality(rel).into(),
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
            bond_role: "new_person".into(),
            bond_state: "unfamiliar".into(),
            last_interaction_quality: "unknown".into(),
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

fn bond_role(authority: &Authority) -> &'static str {
    match authority {
        Authority::ChosenPerson => "chosen_person",
        Authority::Trusted => "trusted_person",
        Authority::Default => "current_person",
        Authority::Restricted => "guarded_person",
        Authority::Blocked => "blocked_person",
    }
}

fn bond_state(rel: &crate::state::Relationship) -> &'static str {
    if matches!(rel.authority, Authority::Blocked) {
        "blocked"
    } else if rel.conflict_level > 0.45 || rel.emotional_valence < -0.45 {
        "strained"
    } else if rel.inbound_count <= 1 && rel.familiarity < 0.05 {
        "first_contact"
    } else if rel.closeness >= 0.65 || rel.familiarity >= 0.65 {
        "bonded"
    } else if rel.familiarity >= 0.25 || rel.closeness >= 0.25 {
        "warming"
    } else {
        "acquaintance"
    }
}

fn interaction_quality(rel: &crate::state::Relationship) -> &'static str {
    if rel.emotional_valence > 0.3 {
        "warm"
    } else if rel.emotional_valence < -0.3 || rel.conflict_level > 0.3 {
        "strained"
    } else if rel.interaction_count == 0 {
        "unknown"
    } else {
        "neutral"
    }
}

fn pct(v: f32) -> i32 {
    (v * 100.0) as i32
}

#[cfg(test)]
mod tests;
