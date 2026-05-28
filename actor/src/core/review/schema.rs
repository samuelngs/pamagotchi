use inference::Tool;
use serde_json::json;

pub(crate) fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "apply_review".into(),
        description: "Apply a structured post-turn review in one durable, auditable step: profile/person updates, memories, relationship deltas, group/person directives, open-loop intents, and conversation summary. Use only from review actions.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "profile_updates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "profile_id": { "type": "string" },
                            "display_name": { "type": "string" },
                            "summary": { "type": "string" },
                            "comm_style": { "type": "string" },
                            "confidence": { "type": "number" },
                            "evidence_message_ids": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["profile_id"]
                    }
                },
                "person_updates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "person_id": { "type": "string" },
                            "name": { "type": "string" },
                            "summary": { "type": "string" },
                            "comm_style": { "type": "string" },
                            "confidence": { "type": "number" },
                            "evidence_message_ids": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["person_id"]
                    }
                },
                "memories": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "enum": ["create", "upsert", "reinforce", "update", "supersede", "contradict", "mark_contradicted", "forget"],
                                "description": "create/upsert stores a memory. reinforce/update/supersede/contradict target an existing memory by memory_id and record mutation history. forget deletes a noisy or obsolete memory by memory_id and records a mutation audit reason."
                            },
                            "memory_id": {
                                "type": "string",
                                "description": "Existing memory id for reinforce, update, supersede, contradict, or forget operations."
                            },
                            "reason": {
                                "type": "string",
                                "description": "Why review is changing the memory, e.g. reinforced by evidence, corrected, duplicate, noise, or contradicted by newer evidence."
                            },
                            "kind": { "type": "string", "enum": ["episodic", "semantic", "procedural"] },
                            "memory_type": { "type": "string" },
                            "truth_status": { "type": "string" },
                            "content": { "type": "string" },
                            "subjects": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "type": { "type": "string", "enum": ["actor", "identity", "profile", "person"] },
                                        "id": { "type": "string" },
                                        "role": { "type": "string" },
                                        "confidence": { "type": "number" }
                                    },
                                    "required": ["type", "id"]
                                }
                            },
                            "importance": { "type": "number" },
                            "sensitivity": { "type": "number" },
                            "sensitivity_category": { "type": "string" },
                            "emotional_valence": { "type": "number" },
                            "confidence": { "type": "number" },
                            "privacy_category": { "type": "string" },
                            "visibility_scope": { "type": "string" },
                            "stability": { "type": "string" },
                            "evidence_message_ids": { "type": "array", "items": { "type": "string" } },
                            "evidence_quote": { "type": "string" },
                            "source_spans": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "message_id": { "type": "string" },
                                        "start_char": { "type": "integer" },
                                        "end_char": { "type": "integer" },
                                        "quote": { "type": "string" }
                                    },
                                    "required": ["message_id"]
                                },
                                "description": "Precise quote spans supporting this memory, persisted inside evidence.source_spans."
                            },
                            "evidence": { "type": "object" },
                            "dedupe_key": { "type": "string" },
                            "supersedes": { "type": "string" },
                            "contradiction_group": { "type": "string" },
                            "expires_at": { "type": "integer" },
                            "last_confirmed_at": { "type": "integer" },
                            "next_review_at": { "type": "integer" }
                        }
                    }
                },
                "relationship_delta": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "person_id": { "type": "string" },
                            "trust_delta": { "type": "number" },
                            "familiarity_delta": { "type": "number" },
                            "valence_delta": { "type": "number" },
                            "closeness_delta": { "type": "number" },
                            "reliability_delta": { "type": "number" },
                            "reciprocity_delta": { "type": "number" },
                            "conflict_delta": { "type": "number" },
                            "proactive_consent": { "type": "string", "enum": ["unknown", "allowed", "denied"] },
                            "response_cadence": {
                                "type": "string",
                                "description": "Durable preference for how quickly this person wants replies or proactive follow-up."
                            },
                            "channel_preference": {
                                "type": "string",
                                "description": "Durable preference for which channel or medium this person wants used."
                            },
                            "reason": { "type": "string" },
                            "dedupe_key": { "type": "string" }
                        },
                        "required": ["person_id"]
                    }
                },
                "social_relations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "person_a": { "type": "string" },
                            "person_b": { "type": "string" },
                            "relation": { "type": "string" },
                            "direction": {
                                "type": "string",
                                "enum": ["a_to_b", "b_to_a", "bidirectional"],
                                "description": "Direction semantics for the relation. Use a_to_b when person_a has the relation to person_b, b_to_a for the reverse, and bidirectional for symmetric relations."
                            },
                            "confidence": { "type": "number" },
                            "status": { "type": "string", "enum": ["hypothesis", "stated", "confirmed", "denied", "outdated"] },
                            "source_kind": { "type": "string", "enum": ["inferred", "stated", "chosen_person_confirmed", "import", "system"] },
                            "asserted_by_person_id": {
                                "type": "string",
                                "description": "Person id of the speaker/source who asserted this relation. Defaults to the cited current-conversation speaker for stated or chosen-person-confirmed relations."
                            },
                            "evidence": { "type": "object" },
                            "evidence_message_ids": { "type": "array", "items": { "type": "string" } },
                            "evidence_quote": { "type": "string" },
                            "dedupe_key": { "type": "string" }
                        },
                        "required": ["person_a", "person_b", "relation"]
                    }
                },
                "directives": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "scope": {
                                "type": "string",
                                "enum": ["group", "person", "authority", "global"],
                                "description": "Where the norm applies. Non-chosen-person review may write only the current person or current group."
                            },
                            "group_id": { "type": "string" },
                            "person_id": { "type": "string" },
                            "authority": { "type": "string", "enum": ["chosen_person", "trusted", "default", "restricted", "blocked"] },
                            "directive": {
                                "type": "string",
                                "description": "Durable behavior norm or boundary to apply in future prompts."
                            },
                            "priority": { "type": "integer" },
                            "active": { "type": "boolean" },
                            "expires_at": { "type": "integer" },
                            "id": { "type": "string" },
                            "dedupe_key": { "type": "string" }
                        },
                        "required": ["scope", "directive"]
                    }
                },
                "open_loops": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": ["scheduled", "triggered", "follow_up"],
                                "description": "Use follow_up as a review-output alias. It is stored as scheduled when fire_at is present, or triggered when only condition is present."
                            },
                            "task": { "type": "string" },
                            "fire_at": { "type": "integer" },
                            "condition": {
                                "type": "string",
                                "description": "Condition for triggered intents, e.g. 'next time Sam messages'."
                            },
                            "person_id": { "type": "string" },
                            "profile_id": { "type": "string" },
                            "conversation_id": { "type": "string" },
                            "priority": { "type": "integer" },
                            "sensitive": { "type": "boolean" },
                            "requires_chosen_person_approval": { "type": "boolean" },
                            "source_memory_id": { "type": "string" },
                            "dedupe_key": { "type": "string" }
                        },
                        "required": ["task"]
                    }
                },
                "conversation_summary": {
                    "type": "object",
                    "properties": {
                        "conversation_id": { "type": "string" },
                        "summary": { "type": "string" },
                        "covered_message_ids": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["summary"]
                }
            }
        }),
    }]
}
