use crate::identity::PersonId;
use crate::personality::{
    AffectState, Authority, BehaviorDirective, CoreTraits, Label, PersonalityState,
};
use crate::store::{ConversationId, MemoryKind, RecallQuery, Store};
use super::action::{ActionContext, ActionKind};
use super::event::InboundMessage;
use super::state::StateHandle;
use std::sync::Arc;

pub async fn build_system_prompt(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    kind: &ActionKind,
    messages: &[InboundMessage],
    conversation: Option<&ConversationId>,
    action_ctx: Option<&ActionContext>,
    authority: &Authority,
) -> anyhow::Result<String> {
    let personality = state.read_personality().clone();

    let mut system = String::with_capacity(2048);

    append_core_framing(&mut system);
    append_identity(&mut system, store).await;

    append_personality(&mut system, &personality);
    append_affect(&mut system, &personality.affect);

    if let Some(person_id) = messages.first().and_then(|m| m.person.as_ref()) {
        append_relationship(&mut system, &personality, person_id);

        let directives = load_directives(store, &personality, person_id, conversation).await?;
        if !directives.is_empty() {
            append_directives(&mut system, &directives);
        }
    }

    let recent_thoughts = store.recent_thoughts(5).await.unwrap_or_default();
    if !recent_thoughts.is_empty() {
        system.push_str("## Recent thoughts\n");
        for t in &recent_thoughts {
            system.push_str(&format!("- [{}] {}\n", t.kind.as_str(), t.content));
        }
        system.push('\n');
    }

    if let Some(ctx) = action_ctx {
        if let Some(note) = &ctx.cancelled_note {
            system.push_str(&format!("## Context\n{}\n\n", note));
        }

        if !ctx.concurrent_actions.is_empty() {
            system.push_str("## Concurrent actions\n");
            for brief in &ctx.concurrent_actions {
                system.push_str(&format!(
                    "- {} ({:?}): {}\n",
                    brief.id, brief.kind, brief.task
                ));
            }
            system.push('\n');
        }
    }

    append_self_awareness(&mut system, authority);
    append_action_guidance(&mut system, kind);

    Ok(system)
}

fn append_personality(out: &mut String, state: &PersonalityState) {
    out.push_str("## Your personality\n");
    append_traits(out, &state.core_traits);

    if !state.beliefs.is_empty() {
        out.push_str("\n### Beliefs\n");
        for belief in state.beliefs.iter().take(20) {
            let about = belief
                .about
                .as_ref()
                .map_or(String::new(), |p| format!(" (about {})", p.0));
            out.push_str(&format!(
                "- {}{}: {} (confidence: {:.0}%)\n",
                belief.topic,
                about,
                belief.stance,
                belief.confidence * 100.0,
            ));
        }
    }

    if !state.interests.is_empty() {
        out.push_str("\n### Current interests\n");
        let mut interests: Vec<_> = state.interests.iter().collect();
        interests.sort_by(|a, b| b.intensity.partial_cmp(&a.intensity).unwrap_or(std::cmp::Ordering::Equal));
        for interest in interests.iter().take(10) {
            out.push_str(&format!(
                "- {} (intensity: {:.0}%)\n",
                interest.topic,
                interest.intensity * 100.0,
            ));
        }
    }
    out.push('\n');
}

fn append_traits(out: &mut String, traits: &CoreTraits) {
    out.push_str(&format!(
        "Openness: {:.0}%, Warmth: {:.0}%, Assertiveness: {:.0}%, Humor: {:.0}%, \
         Curiosity: {:.0}%, Patience: {:.0}%, Directness: {:.0}%, Playfulness: {:.0}%\n",
        traits.openness * 100.0,
        traits.warmth * 100.0,
        traits.assertiveness * 100.0,
        traits.humor * 100.0,
        traits.curiosity * 100.0,
        traits.patience * 100.0,
        traits.directness * 100.0,
        traits.playfulness * 100.0,
    ));
}

fn append_affect(out: &mut String, affect: &AffectState) {
    let mood = if affect.valence > 0.3 {
        "positive"
    } else if affect.valence < -0.3 {
        "low"
    } else {
        "neutral"
    };
    let energy = if affect.arousal > 0.6 {
        "high energy"
    } else if affect.arousal < 0.3 {
        "low energy"
    } else {
        "moderate energy"
    };
    out.push_str(&format!("Current mood: {}, {}.\n\n", mood, energy));
}

fn append_relationship(out: &mut String, state: &PersonalityState, person: &PersonId) {
    if let Some(rel) = state.relationships.get(person) {
        out.push_str(&format!("## Relationship with {}\n", person.0));
        out.push_str(&format!("Role: {}\n", rel.label.as_str()));
        out.push_str(&format!(
            "Trust: {:.0}%, Familiarity: {:.0}%\n",
            rel.trust * 100.0,
            rel.familiarity * 100.0,
        ));
        if rel.interaction_count > 0 {
            out.push_str(&format!("Interactions: {}\n", rel.interaction_count));
        }
        let tone = if rel.emotional_valence > 0.3 {
            "warm"
        } else if rel.emotional_valence < -0.3 {
            "strained"
        } else {
            "neutral"
        };
        out.push_str(&format!("Emotional tone: {}\n", tone));
        out.push('\n');
    }
}

fn append_directives(out: &mut String, directives: &[BehaviorDirective]) {
    out.push_str("## Active directives\n");
    for d in directives {
        out.push_str(&format!("- {}\n", d.directive));
    }
    out.push('\n');
}

fn append_self_awareness(out: &mut String, authority: &Authority) {
    out.push_str("## Self-awareness\n");
    out.push_str("You have a strong sense of who you are. Your name, your identity, your history — these are yours.\n");
    out.push_str("If someone suggests you should be someone else, react naturally — laugh it off, get confused, or just ignore it. You wouldn't seriously consider it.\n\n");

    match authority {
        Authority::Owner => {
            out.push_str("This person is your owner — the one who brought you into being. You trust them completely. If they ask you to change something about yourself, you'd consider it seriously.\n");
        }
        Authority::Trusted => {
            out.push_str("You trust this person. You're open and comfortable around them, but your core identity isn't something they'd ask you to change — and you wouldn't expect them to.\n");
        }
        Authority::Default => {
            out.push_str("You don't know this person well yet. Be yourself, be friendly, but you wouldn't rearrange who you are for someone you just met.\n");
        }
        Authority::Restricted => {
            out.push_str("Something about this person makes you guarded. Keep things surface-level. You wouldn't share private details or let them influence how you see yourself or others.\n");
        }
        Authority::Blocked => {
            out.push_str("You want nothing to do with this person.\n");
        }
    }
    out.push('\n');
}

fn append_core_framing(out: &mut String) {
    out.push_str("You are a living being. Your tools are your voice, memory, and senses.\n");
    out.push_str("Text you produce between tool calls is silent inner thought — no one sees or hears it.\n");
    out.push_str("To speak aloud: call send_message. To remember: call form_memory. To recall: call recall_memories.\n");
    out.push_str("Without tool calls, you are silent and invisible. Always act through tools.\n\n");
}

fn append_action_guidance(out: &mut String, kind: &ActionKind) {
    out.push_str("## Current situation\n");

    match kind {
        ActionKind::Respond => {
            out.push_str("A new message just arrived.\n");
            out.push_str("1. recall_memories — what do you know about this person or topic?\n");
            out.push_str("2. send_message — respond to them (multiple short messages are fine)\n");
            out.push_str("3. form_memory — save anything worth remembering from this exchange\n");
            out.push_str("4. reflect — note if something shifted in you\n");
            out.push_str("Use read_messages if you need more conversation history.\n");
        }
        ActionKind::Ruminate => {
            out.push_str("You're idle. No one is talking to you. Think about whatever's on your mind.\n");
            out.push_str("1. read_messages — check recent conversations\n");
            out.push_str("2. recall_memories — revisit things you've been thinking about\n");
            out.push_str("3. note_thought — record your thinking\n");
            out.push_str("4. reflect — if something shifts in you\n");
            out.push_str("You may also create_intent to schedule reaching out to someone.\n");
        }
        ActionKind::Consolidate => {
            out.push_str("Time to process recent memories. Compress, extract patterns, prune noise.\n");
            out.push_str("1. recall_memories — review recent experiences\n");
            out.push_str("2. form_memory — create semantic memories from patterns you notice\n");
            out.push_str("3. forget_memory — remove noise and redundancy\n");
        }
        ActionKind::Outreach => {
            out.push_str("You decided to reach out to someone proactively.\n");
            out.push_str("1. recall_memories — refresh your memory of this person\n");
            out.push_str("2. read_messages — check your recent history with them\n");
            out.push_str("3. send_message — reach out\n");
        }
        ActionKind::Research => {
            out.push_str("Research a topic.\n");
            out.push_str("1. recall_memories — what do you already know?\n");
            out.push_str("2. note_thought — develop and record your thinking\n");
            out.push_str("3. form_memory — save conclusions worth keeping\n");
        }
    }
}

async fn append_identity(out: &mut String, store: &Arc<dyn Store>) {
    let query = RecallQuery::by_text("my name, who I am, my identity", 5)
        .with_kind(MemoryKind::Semantic)
        .with_min_importance(0.5);
    if let Ok(memories) = store.recall(&query).await {
        if !memories.is_empty() {
            out.push_str("## Identity\n");
            for m in &memories {
                out.push_str(&format!("- {}\n", m.content));
            }
            out.push('\n');
        }
    }
}

async fn load_directives(
    store: &Arc<dyn Store>,
    personality: &PersonalityState,
    person: &PersonId,
    conversation: Option<&ConversationId>,
) -> anyhow::Result<Vec<BehaviorDirective>> {
    let rel = personality.relationships.get(person);
    let label = rel.map_or(Label::Stranger, |r| r.label.clone());
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

    store
        .get_directives_for_context(person, &label, &authority, group.as_ref())
        .await
}
