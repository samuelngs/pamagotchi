use super::conversation::{
    conversation_ctx_from_summary, fetch_conversation_summary, fetch_group_ctx,
};
use super::directives::load_directives;
use super::format::{format_now, pct, relative_duration};
use super::identity::{recall_identity_memories, recall_identity_name};
use super::person::{bond_role, bond_state, interaction_quality, resolve_person_info};
use super::templates::action_task_template;
use super::*;

pub(super) async fn build_action(
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
