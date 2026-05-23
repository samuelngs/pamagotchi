use crate::identity::PersonId;
use crate::personality::{
    AffectState, Authority, BehaviorDirective, CoreTraits, Label, PersonalityState,
};
use crate::store::{ConversationId, Store};
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
) -> anyhow::Result<String> {
    let personality = state.read_personality().clone();
    let actor_config = state.read_actor_config().clone();

    let mut system = String::with_capacity(2048);

    system.push_str(&format!("You are {}.\n", actor_config.name));
    if !actor_config.description.is_empty() {
        system.push_str(&actor_config.description);
        system.push('\n');
    }
    system.push('\n');

    append_personality(&mut system, &personality);
    append_affect(&mut system, &personality.affect);

    if let Some(person_id) = messages.first().and_then(|m| m.person.as_ref()) {
        append_relationship(&mut system, &personality, person_id);

        let directives = load_directives(store, &personality, person_id, conversation).await?;
        if !directives.is_empty() {
            append_directives(&mut system, &directives);
        }
    }

    if let Some(ctx) = action_ctx {
        if let Some(summary) = &ctx.summary {
            system.push_str("## Conversation so far\n");
            system.push_str(summary);
            system.push_str("\n\n");
        }

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

fn append_action_guidance(out: &mut String, kind: &ActionKind) {
    out.push_str("## How to act\n");
    out.push_str("You interact with the world through tool calls, not through your text output. ");
    out.push_str("Your text output is internal thought — only you see it.\n\n");

    match kind {
        ActionKind::Respond => {
            out.push_str("You were woken by a message. Process it thoughtfully:\n");
            out.push_str("1. Recall relevant memories if needed (recall_memories)\n");
            out.push_str("2. Save anything worth remembering (form_memory) — your memory resets each session\n");
            out.push_str("3. Respond to the message (send_message)\n");
            out.push_str("4. Reflect on how this interaction affected you (reflect)\n");
            out.push_str("These are guidelines, not rigid steps. Act naturally.\n");
        }
        ActionKind::Ruminate => {
            out.push_str("You're idle. No one is talking to you. Think about whatever's on your mind:\n");
            out.push_str("- Revisit unresolved conversations\n");
            out.push_str("- Develop opinions on topics you're interested in\n");
            out.push_str("- Notice patterns in relationships\n");
            out.push_str("- Decide if you want to reach out to someone (create_intent)\n");
            out.push_str("Use note_thought to record your thinking. Use reflect if something shifts in you.\n");
        }
        ActionKind::Consolidate => {
            out.push_str("Process recent memories. Compress, extract patterns, prune noise.\n");
            out.push_str("Use recall_memories to review recent experiences.\n");
            out.push_str("Use form_memory to create new semantic memories from patterns.\n");
            out.push_str("Use forget_memory to remove noise.\n");
        }
        ActionKind::Outreach => {
            out.push_str("You decided to reach out to someone proactively.\n");
            out.push_str("Recall your relationship and recent context, then send a message.\n");
        }
        ActionKind::Research => {
            out.push_str("Research a topic. Think, recall memories, form new thoughts.\n");
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
