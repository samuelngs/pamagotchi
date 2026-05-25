use inference::Tool;
use protocol::{MemoryId, PersonId};
use crate::store::{Memory, MemoryKind, MemorySource, RecallQuery};
use serde_json::{json, Value};
use super::context::{SessionContext, SessionState};

pub fn tools() -> Vec<Tool> {
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

pub async fn recall(args: &Value, ctx: &SessionContext) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(3) as usize;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;

    let recall = RecallQuery::by_text(query, limit).with_offset(offset);
    match ctx.store.recall(&recall).await {
        Ok(memories) if memories.is_empty() => "No memories found.".into(),
        Ok(memories) => {
            let mut out = String::new();
            for m in &memories {
                out.push_str(&format!(
                    "[{}] ({}) {}\n",
                    m.id.0,
                    m.kind.as_str(),
                    m.content
                ));
            }
            out
        }
        Err(e) => format!("Error recalling memories: {e}"),
    }
}

pub async fn form(args: &Value, ctx: &SessionContext, state: &mut SessionState) -> String {
    let content = args["content"].as_str().unwrap_or("").to_string();
    let kind = args["kind"]
        .as_str()
        .and_then(MemoryKind::parse)
        .unwrap_or(MemoryKind::Episodic);
    let importance = args["importance"].as_f64().unwrap_or(0.5) as f32;
    let people: Vec<PersonId> = args["people"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| PersonId(s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let memory = Memory {
        id: MemoryId(format!("mem-{}", super::util::uuid_v4())),
        kind,
        content,
        source: ctx
            .conversation
            .as_ref()
            .and_then(|conv| {
                ctx.messages.first().map(|m| MemorySource::Conversation {
                    conversation_id: conv.clone(),
                    person: m.person.clone().unwrap_or(PersonId("unknown".into())),
                })
            })
            .unwrap_or(MemorySource::Reflection),
        importance,
        sensitivity: 0.0,
        emotional_valence: 0.0,
        created_at: super::util::now(),
        accessed_at: super::util::now(),
        access_count: 0,
        tags: vec![],
        people,
        embedding: None,
    };

    match ctx.store.store_memory(&memory).await {
        Ok(id) => {
            state.memories_formed.push(id.clone());
            format!("Memory saved: {}", id.0)
        }
        Err(e) => format!("Failed to save memory: {e}"),
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
