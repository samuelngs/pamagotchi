use protocol::{ConversationId, IdentityId, MemoryId, PersonId, ProfileId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryType {
    Fact,
    Preference,
    StylePattern,
    Boundary,
    Commitment,
    OpenLoop,
    Event,
    Procedure,
    RelationshipFact,
    IdentityClaim,
    Hypothesis,
    Correction,
    EmotionalState,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::StylePattern => "style_pattern",
            Self::Boundary => "boundary",
            Self::Commitment => "commitment",
            Self::OpenLoop => "open_loop",
            Self::Event => "event",
            Self::Procedure => "procedure",
            Self::RelationshipFact => "relationship_fact",
            Self::IdentityClaim => "identity_claim",
            Self::Hypothesis => "hypothesis",
            Self::Correction => "correction",
            Self::EmotionalState => "emotional_state",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "style_pattern" => Some(Self::StylePattern),
            "boundary" => Some(Self::Boundary),
            "commitment" => Some(Self::Commitment),
            "open_loop" => Some(Self::OpenLoop),
            "event" => Some(Self::Event),
            "procedure" => Some(Self::Procedure),
            "relationship_fact" => Some(Self::RelationshipFact),
            "identity_claim" => Some(Self::IdentityClaim),
            "hypothesis" => Some(Self::Hypothesis),
            "correction" => Some(Self::Correction),
            "emotional_state" => Some(Self::EmotionalState),
            _ => None,
        }
    }
}

impl Default for MemoryType {
    fn default() -> Self {
        Self::Fact
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TruthStatus {
    Observed,
    Stated,
    Inferred,
    Confirmed,
    Denied,
    Outdated,
}

impl TruthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Stated => "stated",
            Self::Inferred => "inferred",
            Self::Confirmed => "confirmed",
            Self::Denied => "denied",
            Self::Outdated => "outdated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "observed" => Some(Self::Observed),
            "stated" => Some(Self::Stated),
            "inferred" => Some(Self::Inferred),
            "confirmed" => Some(Self::Confirmed),
            "denied" => Some(Self::Denied),
            "outdated" => Some(Self::Outdated),
            _ => None,
        }
    }
}

impl Default for TruthStatus {
    fn default() -> Self {
        Self::Stated
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PrivacyCategory {
    Public,
    Personal,
    Sensitive,
    Secret,
}

impl PrivacyCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Personal => "personal",
            Self::Sensitive => "sensitive",
            Self::Secret => "secret",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "personal" => Some(Self::Personal),
            "sensitive" => Some(Self::Sensitive),
            "secret" => Some(Self::Secret),
            _ => None,
        }
    }
}

impl Default for PrivacyCategory {
    fn default() -> Self {
        Self::Personal
    }
}

impl PrivacyCategory {
    fn rank(&self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Personal => 1,
            Self::Sensitive => 2,
            Self::Secret => 3,
        }
    }

    fn max(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VisibilityScope {
    Profile,
    Person,
    OwnerOnly,
    Global,
}

impl VisibilityScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Person => "person",
            Self::OwnerOnly => "owner_only",
            Self::Global => "global",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "profile" => Some(Self::Profile),
            "person" => Some(Self::Person),
            "owner_only" => Some(Self::OwnerOnly),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

impl Default for VisibilityScope {
    fn default() -> Self {
        Self::Profile
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryStability {
    Transient,
    Seasonal,
    Stable,
}

impl MemoryStability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Seasonal => "seasonal",
            Self::Stable => "stable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "transient" => Some(Self::Transient),
            "seasonal" => Some(Self::Seasonal),
            "stable" => Some(Self::Stable),
            _ => None,
        }
    }
}

impl Default for MemoryStability {
    fn default() -> Self {
        Self::Stable
    }
}

pub fn memory_truth_status_policy(
    memory_type: &MemoryType,
    explicit_truth_status: Option<TruthStatus>,
) -> TruthStatus {
    explicit_truth_status.unwrap_or(match memory_type {
        MemoryType::Hypothesis => TruthStatus::Inferred,
        _ => TruthStatus::default(),
    })
}

pub fn memory_stability_policy(
    memory_type: &MemoryType,
    truth_status: &TruthStatus,
    explicit_stability: Option<MemoryStability>,
) -> MemoryStability {
    if matches!(
        memory_type,
        MemoryType::EmotionalState | MemoryType::Hypothesis
    ) || matches!(truth_status, TruthStatus::Inferred)
    {
        return MemoryStability::Transient;
    }
    explicit_stability.unwrap_or_default()
}

const SENSITIVE_REVIEW_AFTER_SECS: i64 = 90 * 24 * 60 * 60;
const SECRET_REVIEW_AFTER_SECS: i64 = 30 * 24 * 60 * 60;

pub fn memory_privacy_policy(
    sensitivity: f32,
    sensitivity_category: Option<&str>,
    explicit_privacy: Option<PrivacyCategory>,
    explicit_visibility: Option<VisibilityScope>,
) -> (PrivacyCategory, VisibilityScope) {
    memory_privacy_policy_for_subject(
        sensitivity,
        sensitivity_category,
        explicit_privacy,
        explicit_visibility,
        false,
    )
}

pub fn memory_privacy_policy_for_subject(
    sensitivity: f32,
    sensitivity_category: Option<&str>,
    explicit_privacy: Option<PrivacyCategory>,
    explicit_visibility: Option<VisibilityScope>,
    actor_subject: bool,
) -> (PrivacyCategory, VisibilityScope) {
    let derived = derived_privacy_category(sensitivity, sensitivity_category, actor_subject);
    let privacy = explicit_privacy.unwrap_or_default().max(derived);
    let visibility = match privacy {
        PrivacyCategory::Secret => VisibilityScope::OwnerOnly,
        PrivacyCategory::Sensitive => match explicit_visibility {
            Some(VisibilityScope::OwnerOnly) => VisibilityScope::OwnerOnly,
            Some(scope) => scope,
            None => VisibilityScope::Profile,
        },
        PrivacyCategory::Public | PrivacyCategory::Personal => {
            explicit_visibility.unwrap_or_default()
        }
    };
    (privacy, visibility)
}

pub fn sensitive_memory_next_review_at(
    now: i64,
    privacy: &PrivacyCategory,
    explicit_next_review_at: Option<i64>,
) -> Option<i64> {
    explicit_next_review_at.or_else(|| match privacy {
        PrivacyCategory::Secret => Some(now + SECRET_REVIEW_AFTER_SECS),
        PrivacyCategory::Sensitive => Some(now + SENSITIVE_REVIEW_AFTER_SECS),
        PrivacyCategory::Public | PrivacyCategory::Personal => None,
    })
}

fn derived_privacy_category(
    sensitivity: f32,
    sensitivity_category: Option<&str>,
    actor_subject: bool,
) -> PrivacyCategory {
    if sensitivity >= 0.9
        || sensitivity_category.is_some_and(|category| category_is_secret(category))
    {
        PrivacyCategory::Secret
    } else if sensitivity >= crate::store::DEFAULT_MAX_SENSITIVITY
        || sensitivity_category
            .is_some_and(|category| category_is_sensitive(category, actor_subject))
    {
        PrivacyCategory::Sensitive
    } else {
        PrivacyCategory::Personal
    }
}

fn category_is_secret(category: &str) -> bool {
    let normalized = normalize_sensitivity_category(category);
    matches!(
        normalized.as_str(),
        "credential"
            | "credentials"
            | "password"
            | "passcode"
            | "token"
            | "api_key"
            | "secret"
            | "ssn"
            | "social_security"
    )
}

fn category_is_sensitive(category: &str, actor_subject: bool) -> bool {
    let normalized = normalize_sensitivity_category(category);
    if actor_subject && normalized == "identity" {
        return false;
    }
    category_is_secret(&normalized)
        || matches!(
            normalized.as_str(),
            "health"
                | "medical"
                | "therapy"
                | "diagnosis"
                | "finance"
                | "financial"
                | "legal"
                | "identity"
                | "location"
                | "address"
                | "relationship"
                | "family"
        )
}

fn normalize_sensitivity_category(category: &str) -> String {
    category
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
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
pub struct MemorySubjectDebugRecord {
    pub subject_type: MemorySubjectType,
    pub subject_id: String,
    pub memory_count: u32,
    pub latest_memory_at: Option<i64>,
    pub latest_memory_ids: Vec<MemoryId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemorySubjectType {
    Actor,
    Identity,
    Profile,
    Person,
}

impl MemorySubjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Actor => "actor",
            Self::Identity => "identity",
            Self::Profile => "profile",
            Self::Person => "person",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "actor" => Some(Self::Actor),
            "identity" => Some(Self::Identity),
            "profile" => Some(Self::Profile),
            "person" => Some(Self::Person),
            _ => None,
        }
    }
}

impl MemorySubject {
    pub fn actor(role: Option<String>, confidence: f32) -> Self {
        Self {
            subject_type: MemorySubjectType::Actor,
            subject_id: "self".into(),
            role,
            confidence,
        }
    }

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
    pub memory_type: MemoryType,
    pub truth_status: TruthStatus,
    pub content: String,
    pub source: MemorySource,
    pub importance: f32,
    pub confidence: f32,
    pub sensitivity: f32,
    pub sensitivity_category: Option<String>,
    pub emotional_valence: f32,
    pub created_at: i64,
    pub accessed_at: i64,
    pub access_count: u32,
    pub tags: Vec<String>,
    pub subjects: Vec<MemorySubject>,
    pub evidence_message_ids: Vec<String>,
    pub evidence_quote: Option<String>,
    pub evidence: serde_json::Value,
    pub expires_at: Option<i64>,
    pub stability: MemoryStability,
    pub supersedes: Option<MemoryId>,
    pub superseded_by: Option<MemoryId>,
    pub contradiction_group: Option<String>,
    pub privacy_category: PrivacyCategory,
    pub visibility_scope: VisibilityScope,
    pub last_confirmed_at: Option<i64>,
    pub next_review_at: Option<i64>,
    pub dedupe_key: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_version: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            id: MemoryId(String::new()),
            kind: MemoryKind::Episodic,
            memory_type: MemoryType::default(),
            truth_status: TruthStatus::default(),
            content: String::new(),
            source: MemorySource::External,
            importance: 0.5,
            confidence: 1.0,
            sensitivity: 0.0,
            sensitivity_category: None,
            emotional_valence: 0.0,
            created_at: 0,
            accessed_at: 0,
            access_count: 0,
            tags: vec![],
            subjects: vec![],
            evidence_message_ids: vec![],
            evidence_quote: None,
            evidence: serde_json::Value::Object(Default::default()),
            expires_at: None,
            stability: MemoryStability::default(),
            supersedes: None,
            superseded_by: None,
            contradiction_group: None,
            privacy_category: PrivacyCategory::default(),
            visibility_scope: VisibilityScope::default(),
            last_confirmed_at: None,
            next_review_at: None,
            dedupe_key: None,
            embedding_model: None,
            embedding_version: None,
            embedding: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct MemoryUpdate {
    pub content: Option<String>,
    pub memory_type: Option<MemoryType>,
    pub truth_status: Option<TruthStatus>,
    pub importance: Option<f32>,
    pub confidence: Option<f32>,
    pub sensitivity: Option<f32>,
    pub sensitivity_category: Option<String>,
    pub emotional_valence: Option<f32>,
    pub tags: Option<Vec<String>>,
    pub subjects: Option<Vec<MemorySubject>>,
    pub evidence_message_ids: Option<Vec<String>>,
    pub evidence_quote: Option<String>,
    pub evidence: Option<serde_json::Value>,
    pub expires_at: Option<i64>,
    pub stability: Option<MemoryStability>,
    pub supersedes: Option<MemoryId>,
    pub superseded_by: Option<MemoryId>,
    pub contradiction_group: Option<String>,
    pub privacy_category: Option<PrivacyCategory>,
    pub visibility_scope: Option<VisibilityScope>,
    pub last_confirmed_at: Option<i64>,
    pub next_review_at: Option<i64>,
    pub dedupe_key: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_version: Option<String>,
    pub embedding: Option<Vec<f32>>,
}
