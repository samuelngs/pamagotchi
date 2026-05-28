use super::*;

pub(super) async fn recall_identity_name(store: &Arc<dyn Store>) -> String {
    match store
        .memories_for_subject(MemorySubjectType::Actor, "self", 24)
        .await
    {
        Ok(memories) => memories
            .iter()
            .find(|memory| memory.memory_type == MemoryType::IdentityClaim)
            .and_then(|memory| actor_name_from_identity_memory(&memory.content))
            .unwrap_or_else(|| "an unnamed Pamagotchi".into()),
        Err(_) => "an unnamed Pamagotchi".into(),
    }
}

pub(super) async fn recall_identity_memories(store: &Arc<dyn Store>) -> Vec<String> {
    match store
        .memories_for_subject(MemorySubjectType::Actor, "self", 12)
        .await
    {
        Ok(mut memories) => {
            memories.sort_by(|a, b| {
                actor_identity_memory_rank(a)
                    .cmp(&actor_identity_memory_rank(b))
                    .then_with(|| {
                        b.importance
                            .partial_cmp(&a.importance)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });
            memories.into_iter().take(8).map(|m| m.content).collect()
        }
        Err(_) => vec![],
    }
}

fn actor_name_from_identity_memory(content: &str) -> Option<String> {
    let rest = content.strip_prefix("My name is ")?;
    let name = rest
        .split(['.', ',', '\n'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(name.to_string())
}

fn actor_identity_memory_rank(memory: &crate::store::Memory) -> u8 {
    if memory.memory_type == MemoryType::IdentityClaim {
        0
    } else if memory.tags.iter().any(|tag| tag == "temperament") {
        1
    } else if memory.tags.iter().any(|tag| tag == "voice") {
        2
    } else if memory.tags.iter().any(|tag| tag == "first_contact") {
        3
    } else if memory.tags.iter().any(|tag| tag == "inner_life") {
        4
    } else if memory.tags.iter().any(|tag| tag == "baseline_story") {
        5
    } else {
        6
    }
}
