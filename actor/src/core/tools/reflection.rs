use super::context::{SessionContext, SessionState};
use crate::state::{AffectShift, BeliefChange, RelationshipChange, TraitNudge};
use crate::store::{MemorySubject, Thought, ThoughtKind};
use inference::Tool;
use protocol::PersonId;
use serde_json::{Value, json};
use tracing::info;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "reflect".into(),
            description: "Reflect on how this interaction changed you. Propose personality shifts."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "trait_nudges": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "trait_name": {
                                    "type": "string",
                                    "enum": ["openness", "warmth", "assertiveness", "humor", "curiosity", "patience", "directness", "playfulness"]
                                },
                                "direction": {
                                    "type": "number",
                                    "description": "Positive to increase, negative to decrease. Small values like -0.05 to 0.05."
                                },
                                "reason": { "type": "string" }
                            },
                            "required": ["trait_name", "direction", "reason"]
                        }
                    },
                    "belief_changes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "topic": { "type": "string" },
                                "new_stance": { "type": "string" },
                                "confidence_delta": { "type": "number" },
                                "reason": { "type": "string" },
                                "about_person": { "type": "string" }
                            },
                            "required": ["topic", "confidence_delta", "reason"]
                        }
                    },
                    "relationship_changes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "person": {
                                    "type": "string",
                                    "description": "Person id for the current verified/likely person grouping. Defaults to current conversation partner when available."
                                },
                                "trust_delta": { "type": "number" },
                                "familiarity_delta": { "type": "number" },
                                "valence_delta": { "type": "number" }
                            },
                            "required": ["person"]
                        }
                    },
                    "new_interests": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "affect_shift": {
                        "type": "object",
                        "properties": {
                            "valence": { "type": "number" },
                            "arousal": { "type": "number" },
                            "dominance": { "type": "number" }
                        }
                    },
                    "growth_note": {
                        "type": "string",
                        "description": "A brief note about personal growth from this interaction"
                    },
                    "comm_style": {
                        "type": "string",
                        "description": "Updated communication style for the current profile — tone, length, formality, language patterns, emoji use. Overwrites previous profile style. Only set when you have a clear picture from multiple interactions."
                    }
                }
            }),
        },
        Tool {
            name: "note_thought".into(),
            description: "Record an internal thought for your thought log.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {
                        "type": "string",
                        "enum": ["reflection", "rumination", "consolidation", "planning", "observation"]
                    },
                    "content": {
                        "type": "string",
                        "description": "Your thought"
                    }
                },
                "required": ["kind", "content"]
            }),
        },
    ]
}

pub async fn reflect(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    if let Some(nudges) = args["trait_nudges"].as_array() {
        for nudge in nudges {
            if let (Some(name), Some(dir)) =
                (nudge["trait_name"].as_str(), nudge["direction"].as_f64())
            {
                state.delta.trait_nudges.push(TraitNudge {
                    trait_name: name.to_string(),
                    direction: dir as f32,
                    reason: nudge["reason"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }

    if let Some(beliefs) = args["belief_changes"].as_array() {
        for b in beliefs {
            state.delta.belief_changes.push(BeliefChange {
                topic: b["topic"].as_str().unwrap_or("").to_string(),
                new_stance: b["new_stance"].as_str().map(String::from),
                confidence_delta: b["confidence_delta"].as_f64().unwrap_or(0.0) as f32,
                reason: b["reason"].as_str().unwrap_or("").to_string(),
                about: b["about_person"].as_str().map(|s| PersonId(s.to_string())),
            });
        }
    }

    if let Some(rels) = args["relationship_changes"].as_array() {
        let default_person = ctx.messages.first().and_then(|m| m.person.clone());
        for r in rels {
            let person = r["person"]
                .as_str()
                .map(|s| PersonId(s.to_string()))
                .or_else(|| default_person.clone());
            if let Some(person) = person {
                state.delta.relationship_changes.push(RelationshipChange {
                    person,
                    trust_delta: r["trust_delta"].as_f64().unwrap_or(0.0) as f32,
                    familiarity_delta: r["familiarity_delta"].as_f64().unwrap_or(0.0) as f32,
                    valence_delta: r["valence_delta"].as_f64().unwrap_or(0.0) as f32,
                });
            }
        }
    }

    if let Some(interests) = args["new_interests"].as_array() {
        for i in interests {
            if let Some(topic) = i.as_str() {
                state.delta.new_interests.push(topic.to_string());
            }
        }
    }

    if let Some(affect) = args.get("affect_shift") {
        state.delta.affect_shift = AffectShift {
            valence: affect["valence"].as_f64().unwrap_or(0.0) as f32,
            arousal: affect["arousal"].as_f64().unwrap_or(0.0) as f32,
            dominance: affect["dominance"].as_f64().unwrap_or(0.0) as f32,
        };
    }

    if let Some(note) = args["growth_note"].as_str() {
        state.delta.growth_note = Some(note.to_string());
    }

    if let Some(style) = args["comm_style"].as_str() {
        let profile_id = ctx.messages.first().and_then(|m| m.profile.clone());
        if let Some(pid) = profile_id {
            if let Err(e) = ctx.store.update_profile_comm_style(&pid, style).await {
                info!(action = %ctx.action_id, %e, "failed to update comm_style");
            } else {
                info!(action = %ctx.action_id, profile = %pid.0, "comm_style updated");
            }
        }
    }

    info!(action = %ctx.action_id, "reflection applied");
    "Reflection recorded.".into()
}

pub async fn note_thought(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let kind = args["kind"]
        .as_str()
        .and_then(ThoughtKind::parse)
        .unwrap_or(ThoughtKind::Observation);
    let content = args["content"].as_str().unwrap_or("").to_string();

    let thought = Thought {
        timestamp: super::util::now(),
        kind,
        content,
        memories_accessed: vec![],
        subjects: ctx
            .messages
            .iter()
            .flat_map(|m| {
                let mut subjects = Vec::new();
                if let Some(identity) = &m.identity {
                    subjects.push(MemorySubject::identity(
                        identity.clone(),
                        Some("source".into()),
                        1.0,
                    ));
                }
                if let Some(profile) = &m.profile {
                    subjects.push(MemorySubject::profile(
                        profile.clone(),
                        Some("about".into()),
                        1.0,
                    ));
                }
                if let Some(person) = &m.person {
                    subjects.push(MemorySubject::person(
                        person.clone(),
                        Some("related".into()),
                        1.0,
                    ));
                }
                subjects
            })
            .collect(),
    };

    ctx.store.log_thought(&thought).await.ok();
    state.thoughts.push(thought);

    "Thought noted.".into()
}
