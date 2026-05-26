use protocol::{ConversationId, IdentityId, MemoryId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};

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
        identity_id: Option<IdentityId>,
        profile_id: Option<ProfileId>,
        person_id: Option<PersonId>,
        message_id: Option<String>,
    },
    Consolidation {
        from_memories: Vec<MemoryId>,
    },
    Reflection,
    External,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemorySubject {
    pub subject_type: MemorySubjectType,
    pub subject_id: String,
    pub role: Option<String>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemorySubjectType {
    Identity,
    Profile,
    Person,
}

impl MemorySubjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Profile => "profile",
            Self::Person => "person",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "identity" => Some(Self::Identity),
            "profile" => Some(Self::Profile),
            "person" => Some(Self::Person),
            _ => None,
        }
    }
}

impl MemorySubject {
    pub fn identity(id: IdentityId, role: Option<String>, confidence: f32) -> Self {
        Self {
            subject_type: MemorySubjectType::Identity,
            subject_id: id.0,
            role,
            confidence,
        }
    }

    pub fn profile(id: ProfileId, role: Option<String>, confidence: f32) -> Self {
        Self {
            subject_type: MemorySubjectType::Profile,
            subject_id: id.0,
            role,
            confidence,
        }
    }

    pub fn person(id: PersonId, role: Option<String>, confidence: f32) -> Self {
        Self {
            subject_type: MemorySubjectType::Person,
            subject_id: id.0,
            role,
            confidence,
        }
    }
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
    pub subjects: Vec<MemorySubject>,
    pub embedding: Option<Vec<f32>>,
}

pub struct MemoryUpdate {
    pub content: Option<String>,
    pub importance: Option<f32>,
    pub sensitivity: Option<f32>,
    pub emotional_valence: Option<f32>,
    pub tags: Option<Vec<String>>,
    pub subjects: Option<Vec<MemorySubject>>,
    pub embedding: Option<Vec<f32>>,
}
