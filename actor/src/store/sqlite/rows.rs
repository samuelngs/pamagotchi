use super::{Memory, MemoryKind, MemorySource, MessageRole, StoredMessage};
use crate::identity::{
    ClaimEvidence, ClaimStatus, Identity, IdentityClaim, Person, PersonProfileLink,
    PersonProfileStatus, Profile, ProfileIdentityLink, ProfileIdentityStatus,
};
use crate::state::{Authority, BehaviorDirective, DirectiveScope};
use protocol::{GroupId, IdentityId, MemoryId, PersonId, ProfileId};

pub(super) fn read_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let id: String = row.get("id")?;
    let kind_str: String = row.get("kind")?;
    let source_json: String = row.get("source")?;
    let tags_json: String = row.get("tags")?;

    Ok(Memory {
        id: MemoryId(id),
        kind: MemoryKind::parse(&kind_str).unwrap_or(MemoryKind::Episodic),
        content: row.get("content")?,
        source: serde_json::from_str(&source_json).unwrap_or(MemorySource::External),
        importance: row.get("importance")?,
        sensitivity: row.get("sensitivity")?,
        emotional_valence: row.get("emotional_valence")?,
        created_at: row.get("created_at")?,
        accessed_at: row.get("accessed_at")?,
        access_count: row.get("access_count")?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        subjects: vec![],
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
    Ok(IdentityClaim {
        id: row.get("id")?,
        claimant: PersonId(row.get("claimant_id")?),
        claimed_person: PersonId(row.get("claimed_person_id")?),
        evidence: ClaimEvidence::parse(&evidence_str).unwrap_or(ClaimEvidence::SelfDeclaration),
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
