use super::{MemoryKind, MemoryType};
use protocol::{IdentityId, PersonId, ProfileId};

pub const DEFAULT_MAX_SENSITIVITY: f32 = 0.7;

#[derive(Clone)]
pub struct RecallQuery {
    pub text: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub kind: Option<MemoryKind>,
    pub memory_types: Vec<MemoryType>,
    pub identity: Option<IdentityId>,
    pub profile: Option<ProfileId>,
    pub person: Option<PersonId>,
    pub actor: bool,
    pub time_range: Option<TimeRange>,
    pub min_importance: Option<f32>,
    pub max_sensitivity: Option<f32>,
    pub next_review_before: Option<i64>,
    pub include_sensitive: bool,
    pub include_superseded: bool,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone)]
pub struct TimeRange {
    pub start: Option<i64>,
    pub end: Option<i64>,
}

impl RecallQuery {
    pub fn by_text(text: &str, limit: usize) -> Self {
        Self {
            text: Some(text.to_string()),
            embedding: None,
            kind: None,
            memory_types: Vec::new(),
            identity: None,
            profile: None,
            person: None,
            actor: false,
            time_range: None,
            min_importance: None,
            max_sensitivity: Some(DEFAULT_MAX_SENSITIVITY),
            next_review_before: None,
            include_sensitive: false,
            include_superseded: false,
            limit,
            offset: 0,
        }
    }

    pub fn by_embedding(embedding: Vec<f32>, limit: usize) -> Self {
        Self {
            text: None,
            embedding: Some(embedding),
            kind: None,
            memory_types: Vec::new(),
            identity: None,
            profile: None,
            person: None,
            actor: false,
            time_range: None,
            min_importance: None,
            max_sensitivity: Some(DEFAULT_MAX_SENSITIVITY),
            next_review_before: None,
            include_sensitive: false,
            include_superseded: false,
            limit,
            offset: 0,
        }
    }

    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn with_memory_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_types = vec![memory_type];
        self
    }

    pub fn with_memory_types(mut self, memory_types: impl IntoIterator<Item = MemoryType>) -> Self {
        self.memory_types = memory_types.into_iter().collect();
        self
    }

    pub fn with_min_importance(mut self, min: f32) -> Self {
        self.min_importance = Some(min);
        self
    }

    pub fn with_time_range(mut self, range: TimeRange) -> Self {
        self.time_range = Some(range);
        self
    }

    pub fn with_person(mut self, person: PersonId) -> Self {
        self.person = Some(person);
        self
    }

    pub fn with_profile(mut self, profile: ProfileId) -> Self {
        self.profile = Some(profile);
        self
    }

    pub fn with_identity(mut self, identity: IdentityId) -> Self {
        self.identity = Some(identity);
        self
    }

    pub fn with_actor_subject(mut self) -> Self {
        self.actor = true;
        self
    }

    pub fn with_max_sensitivity(mut self, max: f32) -> Self {
        self.max_sensitivity = Some(max);
        self.include_sensitive = false;
        self
    }

    pub fn with_next_review_due(mut self, now: i64) -> Self {
        self.next_review_before = Some(now);
        self
    }

    pub fn include_sensitive(mut self) -> Self {
        self.max_sensitivity = None;
        self.include_sensitive = true;
        self
    }

    pub fn include_superseded(mut self) -> Self {
        self.include_superseded = true;
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}
