use crate::identity::{
    ClaimEvidence, ClaimStatus, Identity, IdentityClaim, Person, PersonProfileLink,
    PersonProfileStatus, Profile, ProfileIdentityLink, ProfileIdentityStatus,
};
use crate::state::{Authority, BehaviorDirective, DirectiveScope};
use crate::store::{
    IntentRecord, Memory, MemoryKind, MemorySource, MemoryStability, MemoryType, MessageRole,
    PrivacyCategory, StoredMessage, TruthStatus, VisibilityScope,
};
use protocol::{ConversationId, GroupId, IdentityId, MemoryId, PersonId, ProfileId};

pub(super) fn read_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let id: String = row.get("id")?;
    let kind_str: String = row.get("kind")?;
    let memory_type_str: String = row.get("memory_type")?;
    let truth_status_str: String = row.get("truth_status")?;
    let source_json: String = row.get("source")?;
    let tags_json: String = row.get("tags")?;
    let evidence_message_ids_json: String = row.get("evidence_message_ids")?;
    let evidence_json: String = row.get("evidence_json")?;
    let stability_str: String = row.get("stability")?;
    let supersedes: Option<String> = row.get("supersedes")?;
    let superseded_by: Option<String> = row.get("superseded_by")?;
    let privacy_category_str: String = row.get("privacy_category")?;
    let visibility_scope_str: String = row.get("visibility_scope")?;

    Ok(Memory {
        id: MemoryId(id),
        kind: MemoryKind::parse(&kind_str).unwrap_or(MemoryKind::Episodic),
        memory_type: MemoryType::parse(&memory_type_str).unwrap_or_default(),
        truth_status: TruthStatus::parse(&truth_status_str).unwrap_or_default(),
        content: row.get("content")?,
        source: serde_json::from_str(&source_json).unwrap_or(MemorySource::External),
        importance: row.get("importance")?,
        confidence: row.get("confidence")?,
        sensitivity: row.get("sensitivity")?,
        sensitivity_category: row.get("sensitivity_category")?,
        emotional_valence: row.get("emotional_valence")?,
        created_at: row.get("created_at")?,
        accessed_at: row.get("accessed_at")?,
        access_count: row.get("access_count")?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        subjects: vec![],
        evidence_message_ids: serde_json::from_str(&evidence_message_ids_json).unwrap_or_default(),
        evidence_quote: row.get("evidence_quote")?,
        evidence: serde_json::from_str(&evidence_json).unwrap_or(serde_json::Value::Null),
        expires_at: row.get("expires_at")?,
        stability: MemoryStability::parse(&stability_str).unwrap_or_default(),
        supersedes: supersedes.map(MemoryId),
        superseded_by: superseded_by.map(MemoryId),
        contradiction_group: row.get("contradiction_group")?,
        privacy_category: PrivacyCategory::parse(&privacy_category_str).unwrap_or_default(),
        visibility_scope: VisibilityScope::parse(&visibility_scope_str).unwrap_or_default(),
        last_confirmed_at: row.get("last_confirmed_at")?,
        next_review_at: row.get("next_review_at")?,
        dedupe_key: row.get("dedupe_key")?,
        embedding_model: row.get("embedding_model")?,
        embedding_version: row.get("embedding_version")?,
        embedding: None,
    })
}

pub(super) fn read_message(row: &rusqlite::Row) -> rusqlite::Result<StoredMessage> {
    let role_str: String = row.get("role")?;
    let metadata_json: String = row.get("metadata")?;
    let identity_id: Option<String> = row.get("identity_id")?;
    let profile_id: Option<String> = row.get("profile_id")?;
    let person_id: Option<String> = row.get("person_id")?;
    Ok(StoredMessage {
        timestamp: row.get("timestamp")?,
        role: MessageRole::parse(&role_str).unwrap_or(MessageRole::User),
        content: row.get("content")?,
        identity: identity_id.map(IdentityId),
        profile: profile_id.map(ProfileId),
        person: person_id.map(PersonId),
        source_gateway_id: row.get("source_gateway_id")?,
        source_message_id: row.get("source_message_id")?,
        sender_external_id: row.get("sender_external_id")?,
        reply_external_id: row.get("reply_external_id")?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
    })
}

pub(super) fn read_identity(row: &rusqlite::Row) -> rusqlite::Result<Identity> {
    let metadata_json: Option<String> = row.get("metadata_json")?;
    Ok(Identity {
        id: IdentityId(row.get("id")?),
        gateway_id: row.get("gateway_id")?,
        external_id: row.get("external_id")?,
        display_name: row.get("display_name")?,
        metadata: metadata_json.and_then(|json| serde_json::from_str(&json).ok()),
        created_at: row.get("created_at")?,
        last_seen_at: row.get("last_seen_at")?,
    })
}

pub(super) fn read_profile(row: &rusqlite::Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: ProfileId(row.get("id")?),
        display_name: row.get("display_name")?,
        summary: row.get("summary")?,
        comm_style: row.get("comm_style")?,
        first_seen: row.get("first_seen")?,
        last_seen: row.get("last_seen")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(super) fn read_profile_identity_link(
    row: &rusqlite::Row,
) -> rusqlite::Result<ProfileIdentityLink> {
    let status: String = row.get("status")?;
    let evidence_json: Option<String> = row.get("evidence_json")?;
    Ok(ProfileIdentityLink {
        profile_id: ProfileId(row.get("profile_id")?),
        identity_id: IdentityId(row.get("identity_id")?),
        status: ProfileIdentityStatus::parse(&status).unwrap_or(ProfileIdentityStatus::Removed),
        confidence: row.get::<_, f32>("confidence")?,
        evidence: evidence_json.and_then(|json| serde_json::from_str(&json).ok()),
        created_at: row.get("created_at")?,
        removed_at: row.get("removed_at")?,
    })
}

pub(super) fn read_person(row: &rusqlite::Row) -> rusqlite::Result<Person> {
    let created_at: i64 = row.get("created_at")?;
    let updated_at: i64 = row.get("updated_at")?;
    Ok(Person {
        id: PersonId(row.get("id")?),
        name: row.get("display_name")?,
        summary: row.get("summary")?,
        comm_style: row.get("comm_style")?,
        first_seen: created_at,
        last_seen: updated_at,
    })
}

pub(super) fn read_person_profile_link(row: &rusqlite::Row) -> rusqlite::Result<PersonProfileLink> {
    let status: String = row.get("status")?;
    let evidence_json: Option<String> = row.get("evidence_json")?;
    Ok(PersonProfileLink {
        person_id: PersonId(row.get("person_id")?),
        profile_id: ProfileId(row.get("profile_id")?),
        status: PersonProfileStatus::parse(&status).unwrap_or(PersonProfileStatus::Detached),
        confidence: row.get::<_, f32>("confidence")?,
        evidence: evidence_json.and_then(|json| serde_json::from_str(&json).ok()),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        detached_at: row.get("detached_at")?,
    })
}

pub(super) fn read_claim(row: &rusqlite::Row) -> rusqlite::Result<IdentityClaim> {
    let evidence_str: String = row.get("evidence")?;
    let status_str: String = row.get("status")?;
    let evidence_json: String = row.get("evidence_json")?;
    Ok(IdentityClaim {
        id: row.get("id")?,
        claimant: PersonId(row.get("claimant_id")?),
        claimed_person: PersonId(row.get("claimed_person_id")?),
        evidence: ClaimEvidence::parse(&evidence_str).unwrap_or(ClaimEvidence::SelfDeclaration),
        reason: row.get("reason")?,
        evidence_json: serde_json::from_str(&evidence_json)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default())),
        confidence: row.get("confidence")?,
        status: ClaimStatus::parse(&status_str).unwrap_or(ClaimStatus::Pending),
        created_at: row.get("created_at")?,
        resolved_at: row.get("resolved_at")?,
    })
}

pub(super) fn read_directive(row: &rusqlite::Row) -> rusqlite::Result<BehaviorDirective> {
    let scope_type: String = row.get("scope_type")?;
    let scope_value: Option<String> = row.get("scope_value")?;
    let active: i32 = row.get("active")?;

    let scope = match scope_type.as_str() {
        "person" => DirectiveScope::Person(PersonId(scope_value.unwrap_or_default())),
        "authority" => DirectiveScope::Authority(
            Authority::parse(&scope_value.unwrap_or_default()).unwrap_or(Authority::Default),
        ),
        "group" => DirectiveScope::Group(GroupId(scope_value.unwrap_or_default())),
        _ => DirectiveScope::Global,
    };

    Ok(BehaviorDirective {
        id: row.get("id")?,
        scope,
        directive: row.get("directive")?,
        set_by: PersonId(row.get("set_by")?),
        priority: row.get("priority")?,
        active: active != 0,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
    })
}

pub(super) fn read_intent(row: &rusqlite::Row) -> rusqlite::Result<IntentRecord> {
    let person_id: Option<String> = row.get("person_id")?;
    let profile_id: Option<String> = row.get("profile_id")?;
    let conversation_id: Option<String> = row.get("conversation_id")?;
    let source_memory_id: Option<String> = row.get("source_memory_id")?;

    Ok(IntentRecord {
        id: row.get("id")?,
        kind: row.get("kind")?,
        status: row.get("status")?,
        task: row.get("task")?,
        person: person_id.map(PersonId),
        profile: profile_id.map(ProfileId),
        conversation: conversation_id.map(ConversationId),
        fire_at: row.get("fire_at")?,
        condition: row.get("condition")?,
        recurrence: row.get("recurrence")?,
        priority: row.get::<_, u8>("priority")?,
        dedupe_key: row.get("dedupe_key")?,
        source_action: row.get("source_action_id")?,
        source_memory: source_memory_id.map(MemoryId),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_fired_at: row.get("last_fired_at")?,
        owner_approved: row.get("owner_approved")?,
    })
}
