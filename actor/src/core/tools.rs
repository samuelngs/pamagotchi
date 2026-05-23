use crate::llm::Tool;
use serde_json::json;

pub fn action_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "recall_memories".into(),
            description: "Search your memories by topic or keywords.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results (default 3)",
                        "default": 3
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Skip this many results. Use to paginate — first call with offset 0, then offset 3 for more.",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "form_memory".into(),
            description: "Save something worth remembering. Your memory resets each session — anything not saved is lost.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "What to remember"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["episodic", "semantic", "procedural"],
                        "description": "episodic = what happened, semantic = facts/knowledge, procedural = how to do things"
                    },
                    "importance": {
                        "type": "number",
                        "description": "0.0 to 1.0, how important this is",
                        "default": 0.5
                    },
                    "people": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Person IDs this memory involves"
                    }
                },
                "required": ["content", "kind"]
            }),
        },
        Tool {
            name: "send_message".into(),
            description: "Send a message. Omit platform_id and external_id to reply in the current conversation. Provide both to send to a specific destination (use lookup_contacts to find someone's contact methods).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The message text"
                    },
                    "platform_id": {
                        "type": "string",
                        "description": "Platform to send through (e.g. discord, telegram, whatsapp)"
                    },
                    "external_id": {
                        "type": "string",
                        "description": "Recipient's ID on that platform. Must be paired with platform_id."
                    },
                    "media_url": {
                        "type": "string",
                        "description": "URL of media to attach"
                    },
                    "media_type": {
                        "type": "string",
                        "enum": ["image", "video", "audio", "sticker", "file"],
                        "description": "Type of media attachment"
                    },
                    "mime_type": {
                        "type": "string",
                        "description": "MIME type of the media (e.g. image/png, video/mp4)"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Filename for file attachments"
                    }
                },
                "required": ["content"]
            }),
        },
        Tool {
            name: "lookup_contacts".into(),
            description: "Look up how to reach a person. Returns their known contact methods across platforms.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "person": {
                        "type": "string",
                        "description": "Person ID to look up"
                    }
                },
                "required": ["person"]
            }),
        },
        Tool {
            name: "reflect".into(),
            description: "Reflect on how this interaction changed you. Propose personality shifts.".into(),
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
                                "person": { "type": "string" },
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
        Tool {
            name: "create_intent".into(),
            description: "Schedule something for later. A reminder, follow-up, or triggered action.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "What to do when the intent fires"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["scheduled", "triggered"],
                        "description": "scheduled = at a specific time, triggered = when a condition is met"
                    },
                    "fire_at": {
                        "type": "integer",
                        "description": "Unix timestamp for scheduled intents"
                    },
                    "condition": {
                        "type": "string",
                        "description": "Natural language condition for triggered intents, e.g. 'next time Sam messages'"
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID this intent relates to"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID for context"
                    }
                },
                "required": ["task", "kind"]
            }),
        },
        Tool {
            name: "forget_memory".into(),
            description: "Remove a memory that's no longer relevant.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "ID of the memory to forget"
                    }
                },
                "required": ["memory_id"]
            }),
        },
        Tool {
            name: "read_messages".into(),
            description: "Read messages from a conversation. Use to access older history beyond what's in your current context.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID. Defaults to current conversation."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 10)",
                        "default": 10
                    },
                    "before": {
                        "type": "integer",
                        "description": "Unix timestamp. Only return messages before this time. Use to page backwards through history."
                    }
                }
            }),
        },
        Tool {
            name: "start_composing".into(),
            description: "Signal that you are composing a response in a conversation. Shows activity to the other party.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID to signal composing in. Defaults to current conversation."
                    }
                }
            }),
        },
        Tool {
            name: "stop_composing".into(),
            description: "Signal that you stopped composing without sending. Use when you decide not to respond after starting to compose.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "conversation": {
                        "type": "string",
                        "description": "Conversation ID to stop composing in. Defaults to current conversation."
                    }
                }
            }),
        },
        Tool {
            name: "delete_intent".into(),
            description: "Cancel a scheduled or triggered intent.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "intent_id": {
                        "type": "string",
                        "description": "ID of the intent to cancel"
                    }
                },
                "required": ["intent_id"]
            }),
        },
        Tool {
            name: "update_intent".into(),
            description: "Modify an existing intent. Atomic update — safer than delete + create if the program crashes between operations.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "intent_id": {
                        "type": "string",
                        "description": "ID of the intent to update"
                    },
                    "task": {
                        "type": "string",
                        "description": "New task description"
                    },
                    "kind": {
                        "type": "string",
                        "enum": ["scheduled", "triggered"],
                        "description": "New intent kind"
                    },
                    "fire_at": {
                        "type": "integer",
                        "description": "New fire time (unix timestamp) for scheduled intents"
                    },
                    "condition": {
                        "type": "string",
                        "description": "New condition for triggered intents"
                    },
                    "person": {
                        "type": "string",
                        "description": "New person ID"
                    },
                    "conversation": {
                        "type": "string",
                        "description": "New conversation ID"
                    }
                },
                "required": ["intent_id"]
            }),
        },
    ]
}
