use super::context::{SessionContext, SessionState};
use crate::state::{AffectShift, BeliefChange, RelationshipChange, TraitNudge};
use crate::store::{MemorySubject, Thought, ThoughtKind};
use inference::Tool;
use protocol::{MemoryId, PersonId};
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
                        "description": "Persistent communication style for the current profile: tone, message length, formality, casing, punctuation habits, language patterns, emoji use, and preferred address/name-to-use. Use this for detailed style and addressing preferences; summaries may stay rich but should not be the only place style is stored. Overwrites previous profile style. Set it when the user states a preference or when a clear pattern emerges."
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
                    },
                    "importance": {
                        "type": "number",
                        "description": "0.0 to 1.0, how useful this thought is likely to be later",
                        "default": 0.5
                    },
                    "confidence": {
                        "type": "number",
                        "description": "0.0 to 1.0, how confident you are in this thought",
                        "default": 0.5
                    },
                    "memory_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Memory IDs this thought used or depends on, especially recalled memories that should support later review."
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
                let trust_ceiling =
                    super::permission::relationship_trust_ceiling(ctx, &person).await;
                state.delta.relationship_changes.push(RelationshipChange {
                    person,
                    trust_delta: clamp_relationship_delta(
                        r["trust_delta"].as_f64().unwrap_or(0.0) as f32,
                        0.05,
                    ),
                    trust_ceiling: Some(trust_ceiling),
                    familiarity_delta: clamp_relationship_delta(
                        r["familiarity_delta"].as_f64().unwrap_or(0.0) as f32,
                        0.1,
                    ),
                    valence_delta: clamp_relationship_delta(
                        r["valence_delta"].as_f64().unwrap_or(0.0) as f32,
                        0.1,
                    ),
                    proactive_consent: None,
                    response_cadence: None,
                    channel_preference: None,
                    interaction: None,
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

fn clamp_relationship_delta(value: f32, max_abs: f32) -> f32 {
    value.clamp(-max_abs, max_abs)
}

pub async fn note_thought(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let kind = args["kind"]
        .as_str()
        .and_then(ThoughtKind::parse)
        .unwrap_or(ThoughtKind::Observation);
    let content = args["content"].as_str().unwrap_or("").to_string();
    let importance = args["importance"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32;
    let confidence = args["confidence"].as_f64().unwrap_or(0.5).clamp(0.0, 1.0) as f32;
    let memories_accessed = thought_memory_ids(args, state);

    let thought = Thought {
        timestamp: super::util::now(),
        kind,
        content,
        importance,
        confidence,
        action_id: Some(ctx.action_id.0.clone()),
        memories_accessed,
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

fn memory_ids_from_args(args: &Value) -> Vec<MemoryId> {
    args["memory_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
        .collect()
}

fn thought_memory_ids(args: &Value, state: &SessionState) -> Vec<MemoryId> {
    let explicit = memory_ids_from_args(args);
    if !explicit.is_empty() {
        return explicit;
    }
    state.recalled_memory_ids.clone()
}

#[cfg(test)]
mod tests {
    use super::{clamp_relationship_delta, memory_ids_from_args, thought_memory_ids, tools};
    use crate::core::tools::SessionState;
    use crate::state::Delta;
    use protocol::MemoryId;
    use serde_json::json;

    #[test]
    fn reflect_comm_style_owns_style_and_addressing_preferences() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "reflect")
            .expect("reflect tool exists");
        let description = tool.parameters["properties"]["comm_style"]["description"]
            .as_str()
            .expect("comm_style description exists");

        assert!(description.contains("preferred address"));
        assert!(description.contains("summaries may stay rich"));
        assert!(description.contains("user states a preference"));
    }

    #[test]
    fn note_thought_schema_exposes_quality_metadata() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "note_thought")
            .expect("note_thought tool exists");
        let properties = tool.parameters["properties"]
            .as_object()
            .expect("properties object");

        assert!(properties.contains_key("importance"));
        assert!(properties.contains_key("confidence"));
        assert!(properties.contains_key("memory_ids"));
    }

    #[test]
    fn note_thought_memory_ids_are_parsed() {
        let ids = memory_ids_from_args(&json!({
            "memory_ids": ["memory-a", "", " memory-b "]
        }));
        let ids = ids.into_iter().map(|id| id.0).collect::<Vec<_>>();
        assert_eq!(ids, vec!["memory-a", "memory-b"]);
    }

    #[test]
    fn note_thought_uses_recalled_memory_ids_when_explicit_ids_absent() {
        let state = SessionState {
            responded: false,
            attempted_send: false,
            composing_released: false,
            delta: Delta::default(),
            thoughts: vec![],
            memories_formed: vec![],
            recalled_memory_ids: vec![MemoryId("memory-from-recall".into())],
            injected_messages: vec![],
            presented_injected_messages: vec![],
            presented_read_messages: vec![],
            pending_injected_messages: vec![],
            source_message_keys: Default::default(),
            queued_injected_message_keys: Default::default(),
            presented_injected_message_keys: Default::default(),
            applied_review_keys: Default::default(),
            presented_injection_count: 0,
        };

        let ids = thought_memory_ids(&json!({}), &state);
        assert_eq!(ids, vec![MemoryId("memory-from-recall".into())]);

        let explicit = thought_memory_ids(
            &json!({
                "memory_ids": ["memory-explicit"]
            }),
            &state,
        );
        assert_eq!(explicit, vec![MemoryId("memory-explicit".into())]);
    }

    #[test]
    fn relationship_deltas_are_small_per_reflection() {
        assert_eq!(clamp_relationship_delta(1.0, 0.05), 0.05);
        assert_eq!(clamp_relationship_delta(-1.0, 0.05), -0.05);
        assert_eq!(clamp_relationship_delta(0.02, 0.05), 0.02);
    }
}
