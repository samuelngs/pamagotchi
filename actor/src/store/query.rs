use super::MemoryKind;
use protocol::{IdentityId, PersonId, ProfileId};

pub struct RecallQuery {
    pub text: Option<String>,
    pub embedding: Option<Vec<f32>>,
    pub kind: Option<MemoryKind>,
    pub identity: Option<IdentityId>,
    pub profile: Option<ProfileId>,
    pub person: Option<PersonId>,
    pub time_range: Option<TimeRange>,
    pub min_importance: Option<f32>,
    pub max_sensitivity: Option<f32>,
    pub limit: usize,
    pub offset: usize,
}

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
            identity: None,
            profile: None,
            person: None,
            time_range: None,
            min_importance: None,
            max_sensitivity: None,
            limit,
            offset: 0,
        }
    }

    pub fn by_embedding(embedding: Vec<f32>, limit: usize) -> Self {
        Self {
            text: None,
            embedding: Some(embedding),
            kind: None,
            identity: None,
            profile: None,
            person: None,
            time_range: None,
            min_importance: None,
            max_sensitivity: None,
            limit,
            offset: 0,
        }
    }

    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = Some(kind);
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

    pub fn with_max_sensitivity(mut self, max: f32) -> Self {
        self.max_sensitivity = Some(max);
        self
    }

    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}
