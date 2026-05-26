mod form;
mod helpers;
mod promote;
mod recall;

pub use form::form;
pub use promote::{demote_person_memory_to_profile, promote_profile_memory_to_person};
pub use recall::recall;

use super::context::SessionContext;
use inference::Tool;
use protocol::MemoryId;
use serde_json::{Value, json};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "recall_memories".into(),
            description: "Search memories by topic or keywords. Defaults to the current profile boundary; use scope=global only when intentionally searching across profiles.".into(),
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
                    "kind": {
                        "type": "string",
                        "enum": ["episodic", "semantic", "procedural"],
                        "description": "Optional memory kind filter"
                    },
                    "identity": {
                        "type": "string",
                        "description": "Identity ID to restrict account-specific recall to."
                    },
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to restrict recall to. Defaults to the current speaker profile when available."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID to restrict recall to for verified person-level memories."
                    },
                    "scope": {
                        "type": "string",
                        "enum": ["current", "global"],
                        "description": "Recall scope. Defaults to current profile. Use global only when intentionally searching across profiles."
                    },
                    "global": {
                        "type": "boolean",
                        "description": "Deprecated alias for scope=global."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Skip this many results. Use to paginate: first call with offset 0, then offset 3 for more.",
                        "default": 0
                    }
                },
                "required": ["query"]
            }),
        },
        Tool {
            name: "form_memory".into(),
            description: "Save something worth remembering. User-specific facts are saved to the current profile by default; use promote_profile_memory_to_person only after verification.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "What to remember. Names are display labels, not identity keys."
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
                    "subject_profile_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Profile IDs this memory is about. Defaults to the current speaker profile."
                    },
                    "subject_identity_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Identity IDs this memory is about when the fact is account-specific."
                    },
                },
                "required": ["content", "kind"]
            }),
        },
        Tool {
            name: "promote_profile_memory_to_person".into(),
            description: "Deliberately promote a profile-level memory to a verified person grouping. Use only with explicit confirmation or strong verified evidence.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "Memory ID to promote"
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID that should become a subject of this memory"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Evidence or reason for promoting this memory"
                    }
                },
                "required": ["memory_id", "person"]
            }),
        },
        Tool {
            name: "demote_person_memory_to_profile".into(),
            description: "Move an over-broad person-level memory back to a profile subject without deleting the memory.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "Memory ID to demote"
                    },
                    "profile": {
                        "type": "string",
                        "description": "Profile ID that should own this memory"
                    },
                    "person": {
                        "type": "string",
                        "description": "Optional person ID to remove. If omitted, all person subjects are removed."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Reason for demoting this memory"
                    }
                },
                "required": ["memory_id", "profile"]
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
    ]
}

pub async fn forget(args: &Value, ctx: &SessionContext) -> String {
    let id = args["memory_id"].as_str().unwrap_or("");
    match ctx.store.forget(&MemoryId(id.to_string())).await {
        Ok(true) => "Memory forgotten.".into(),
        Ok(false) => "Memory not found.".into(),
        Err(e) => format!("Error: {e}"),
    }
}
