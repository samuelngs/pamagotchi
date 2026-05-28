mod form;
mod helpers;
mod promote;
mod recall;

pub use form::form;
pub use promote::{demote_person_memory_to_profile, promote_profile_memory_to_person};
pub use recall::recall;

use super::context::SessionContext;
use crate::state::Authority;
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
                    },
                    "max_sensitivity": {
                        "type": "number",
                        "description": "Maximum sensitivity to include, 0.0 to 1.0. Defaults to conservative recall."
                    },
                    "include_sensitive": {
                        "type": "boolean",
                        "description": "Include sensitive/secret memories. Use only when directly relevant and authority allows it.",
                        "default": false
                    },
                    "include_superseded": {
                        "type": "boolean",
                        "description": "Include superseded or outdated memories for audit/history. Defaults to false so stale facts do not appear in normal recall.",
                        "default": false
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
                    "sensitivity": {
                        "type": "number",
                        "description": "0.0 to 1.0, how private or sensitive this memory is",
                        "default": 0.0
                    },
                    "emotional_valence": {
                        "type": "number",
                        "description": "-1.0 to 1.0, emotional tone of the memory",
                        "default": 0.0
                    },
                    "confidence": {
                        "type": "number",
                        "description": "0.0 to 1.0 confidence in this memory based on available evidence",
                        "default": 1.0
                    },
                    "memory_type": {
                        "type": "string",
                        "enum": ["fact", "preference", "style_pattern", "boundary", "commitment", "open_loop", "event", "procedure", "relationship_fact", "identity_claim", "hypothesis", "correction", "emotional_state"],
                        "description": "Structured memory ontology. Use hypothesis for uncertain inference, correction for updated facts, and open_loop/commitment for follow-up obligations."
                    },
                    "truth_status": {
                        "type": "string",
                        "enum": ["observed", "stated", "inferred", "confirmed", "denied", "outdated"],
                        "description": "How the content is known."
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Short structured tags such as preference, boundary, correction, commitment, style_pattern, identity_claim, or open_loop."
                    },
                    "sensitivity_category": {
                        "type": "string",
                        "description": "Optional category for sensitive material, e.g. health, finance, identity, credentials, relationship, location."
                    },
                    "evidence_message_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Message ids that support this memory."
                    },
                    "evidence_quote": {
                        "type": "string",
                        "description": "Short quote or paraphrase from the supporting message."
                    },
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
                    "evidence": {
                        "type": "object",
                        "description": "Compact structured evidence object such as source spans, quotes, or review rationale."
                    },
                    "expires_at": {
                        "type": "integer",
                        "description": "Unix timestamp when this memory should expire, for temporary facts."
                    },
                    "stability": {
                        "type": "string",
                        "enum": ["transient", "seasonal", "stable"],
                        "description": "Expected durability of the memory."
                    },
                    "privacy_category": {
                        "type": "string",
                        "enum": ["public", "personal", "sensitive", "secret"],
                        "description": "Privacy class for recall and prompt inclusion policy."
                    },
                    "visibility_scope": {
                        "type": "string",
                        "enum": ["profile", "person", "chosen_person_only", "global"],
                        "description": "Default boundary for using this memory."
                    },
                    "dedupe_key": {
                        "type": "string",
                        "description": "Stable key for upserting/reinforcing an existing equivalent memory instead of duplicating it."
                    },
                    "supersedes": {
                        "type": "string",
                        "description": "Memory ID this new memory corrects or replaces. The old memory will be linked back to the new memory."
                    },
                    "contradiction_group": {
                        "type": "string",
                        "description": "Stable group key for mutually contradictory memories."
                    },
                    "last_confirmed_at": {
                        "type": "integer",
                        "description": "Unix timestamp when this memory was last confirmed."
                    },
                    "next_review_at": {
                        "type": "integer",
                        "description": "Unix timestamp when this memory should be reviewed again."
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
                    "subject_actor": {
                        "type": "boolean",
                        "description": "Chosen-person-only. Store this as an actor/self memory for Pamagotchi's own identity or core self facts."
                    },
                },
                "required": ["content", "kind"]
            }),
        },
        Tool {
            name: "inspect_memory".into(),
            description: "Chosen-person-only audit view for one memory by id, including subjects, source, privacy, evidence, supersession, and review metadata.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "ID of the memory to inspect"
                    }
                },
                "required": ["memory_id"]
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
        Tool {
            name: "delete_memory".into(),
            description: "Chosen-person-only deletion for any memory by id, including sensitive, secret, external, or cross-profile memories.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "memory_id": {
                        "type": "string",
                        "description": "ID of the memory to delete"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Brief audit reason for deleting this memory"
                    }
                },
                "required": ["memory_id"]
            }),
        },
    ]
}

pub async fn inspect(args: &Value, ctx: &SessionContext) -> String {
    if !matches!(ctx.authority, Authority::ChosenPerson) {
        return json!({
            "status": "error",
            "message": "Chosen-person authority is required to inspect memories by id."
        })
        .to_string();
    }
    let id = args["memory_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return json!({"status": "error", "message": "Provide memory_id."}).to_string();
    }
    match ctx.store.get_memory(&MemoryId(id.to_string())).await {
        Ok(Some(memory)) => {
            let embedding_present = memory.embedding.is_some();
            let mut value = serde_json::to_value(memory).unwrap_or(Value::Null);
            if let Some(object) = value.as_object_mut() {
                object.remove("embedding");
                object.insert("embedding_present".into(), Value::Bool(embedding_present));
            }
            let mutations = match ctx
                .store
                .memory_mutations_for_memory(&MemoryId(id.to_string()), 25)
                .await
            {
                Ok(mutations) => serde_json::to_value(mutations).unwrap_or(Value::Null),
                Err(e) => json!({"error": format!("{e}")}),
            };
            json!({
                "status": "ok",
                "memory": value,
                "mutations": mutations,
            })
            .to_string()
        }
        Ok(None) => json!({"status": "not_found", "memory_id": id}).to_string(),
        Err(e) => json!({"status": "error", "message": format!("{e}")}).to_string(),
    }
}

pub async fn forget(args: &Value, ctx: &SessionContext) -> String {
    let id = args["memory_id"].as_str().unwrap_or("");
    match ctx.store.forget(&MemoryId(id.to_string())).await {
        Ok(true) => "Memory forgotten.".into(),
        Ok(false) => "Memory not found.".into(),
        Err(e) => format!("Error: {e}"),
    }
}

pub async fn delete(args: &Value, ctx: &SessionContext) -> String {
    if !matches!(ctx.authority, Authority::ChosenPerson) {
        return json!({
            "status": "error",
            "message": "Chosen-person authority is required to delete memories by id."
        })
        .to_string();
    }
    let id = args["memory_id"].as_str().unwrap_or("");
    if id.is_empty() {
        return json!({"status": "error", "message": "Provide memory_id."}).to_string();
    }
    let reason = args["reason"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match ctx
        .store
        .forget_with_reason(&MemoryId(id.to_string()), reason)
        .await
    {
        Ok(true) => json!({"status": "deleted", "memory_id": id}).to_string(),
        Ok(false) => json!({"status": "not_found", "memory_id": id}).to_string(),
        Err(e) => json!({"status": "error", "message": format!("{e}")}).to_string(),
    }
}

#[cfg(test)]
mod tests;
