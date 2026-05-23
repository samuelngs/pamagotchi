use crate::identity::PersonId;
use super::ConversationId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "episodic" => Some(Self::Episodic),
            "semantic" => Some(Self::Semantic),
            "procedural" => Some(Self::Procedural),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MemorySource {
    Conversation {
        conversation_id: ConversationId,
        person: PersonId,
    },
    Consolidation {
        from_memories: Vec<MemoryId>,
    },
    Reflection,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub content: String,
    pub source: MemorySource,
    pub importance: f32,
    pub sensitivity: f32,
    pub emotional_valence: f32,
    pub created_at: i64,
    pub accessed_at: i64,
    pub access_count: u32,
    pub tags: Vec<String>,
    pub people: Vec<PersonId>,
    pub embedding: Option<Vec<f32>>,
}

pub struct MemoryUpdate {
    pub content: Option<String>,
    pub importance: Option<f32>,
    pub sensitivity: Option<f32>,
    pub emotional_valence: Option<f32>,
    pub tags: Option<Vec<String>>,
    pub people: Option<Vec<PersonId>>,
    pub embedding: Option<Vec<f32>>,
}
