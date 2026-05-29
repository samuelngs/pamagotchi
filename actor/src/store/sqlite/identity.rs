use super::rows::{
    read_claim, read_identity, read_person, read_person_profile_link, read_profile,
    read_profile_identity_link,
};
use super::support::TxGuard;
use crate::identity::{
    ClaimStatus, Identity, IdentityClaim, Profile, ProfileIdentityLink, ResolvedActorIdentity,
};
use crate::store::{
    DisplayNameObservation, IdentityConflictIdentity, IdentityConflictRecord,
    IdentityDisclosureAudit,
};
use protocol::{ChannelId, IdentityId, PersonId, ProfileId};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn add_identity(conn: &Connection, identity: &Identity) -> anyhow::Result<IdentityId> {
    let metadata_json = identity
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO identities (id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(gateway_id, external_id) DO UPDATE SET
            display_name = COALESCE(excluded.display_name, identities.display_name),
            metadata_json = COALESCE(excluded.metadata_json, identities.metadata_json),
            last_seen_at = excluded.last_seen_at",
        params![
            identity.id.0,
            identity.gateway_id,
            identity.external_id,
            identity.display_name,
            metadata_json,
            identity.created_at,
            identity.last_seen_at,
        ],
    )?;
    let id = conn.query_row(
        "SELECT id FROM identities WHERE gateway_id = ?1 AND external_id = ?2",
        params![identity.gateway_id, identity.external_id],
        |row| row.get::<_, String>(0),
    )?;
    Ok(IdentityId(id))
}

pub(super) fn get_identity(conn: &Connection, id: &IdentityId) -> anyhow::Result<Option<Identity>> {
    conn.query_row(
        "SELECT id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at
         FROM identities WHERE id = ?1",
        params![id.0],
        read_identity,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn resolve_identity(
    conn: &Connection,
    gateway_id: &str,
    external_id: &str,
) -> anyhow::Result<Option<ResolvedActorIdentity>> {
    let identity = match conn.query_row(
        "SELECT id, gateway_id, external_id, display_name, metadata_json, created_at, last_seen_at
         FROM identities WHERE gateway_id = ?1 AND external_id = ?2",
        params![gateway_id, external_id],
        read_identity,
    ) {
        Ok(identity) => identity,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let Some((profile, _profile_link)) = conn.query_row(
        "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                l.profile_id, l.identity_id, l.status, l.confidence, l.evidence_json, l.created_at, l.removed_at
         FROM profile_identities l
         JOIN profiles p ON p.id = l.profile_id
         WHERE l.identity_id = ?1 AND l.status = 'active'
         ORDER BY l.confidence DESC, l.created_at DESC
         LIMIT 1",
        params![identity.id.0],
        |row| Ok((read_profile(row)?, read_profile_identity_link(row)?)),
    )
    .optional()? else {
        return Ok(None);
    };

    let person_link = conn
        .query_row(
            "SELECT p.id, p.display_name, p.summary, p.comm_style, p.created_at, p.updated_at,
                    l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
             FROM person_profiles l
             JOIN persons p ON p.id = l.person_id
             WHERE l.profile_id = ?1 AND l.status IN ('verified', 'likely')
             ORDER BY CASE l.status WHEN 'verified' THEN 0 ELSE 1 END, l.confidence DESC, l.updated_at DESC
             LIMIT 1",
            params![profile.id.0],
            |row| Ok((read_person(row)?, read_person_profile_link(row)?)),
        )
        .optional()?;

    Ok(Some(ResolvedActorIdentity {
        identity,
        profile,
        person: person_link.as_ref().map(|(person, _)| person.clone()),
        profile_person_link: person_link.map(|(_, link)| link),
    }))
}

pub(super) fn touch_identity(conn: &Connection, id: &IdentityId) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE identities SET last_seen_at = unixepoch() WHERE id = ?1",
        params![id.0],
    )?;
    Ok(())
}

pub(super) fn update_identity_display_name(
    conn: &Connection,
    id: &IdentityId,
    display_name: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE identities
         SET display_name = ?1, last_seen_at = unixepoch()
         WHERE id = ?2",
        params![display_name, id.0],
    )?;
    Ok(())
}

pub(super) fn record_display_name_observation(
    conn: &Connection,
    observation: &DisplayNameObservation,
) -> anyhow::Result<()> {
    let profile_id = observation.profile.as_ref().map(|id| id.0.as_str());
    let source_message_id = observation.source_message_id.as_deref();
    conn.execute(
        "INSERT OR IGNORE INTO display_name_observations (
            identity_id, profile_id, gateway_id, external_id, display_name,
            source_message_id, observed_at
         )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            observation.identity.0.as_str(),
            profile_id,
            observation.gateway_id.as_str(),
            observation.external_id.as_str(),
            observation.display_name.as_str(),
            source_message_id,
            observation.observed_at,
        ],
    )?;
    Ok(())
}

pub(super) fn display_name_observations(
    conn: &Connection,
    identity: &IdentityId,
    limit: usize,
) -> anyhow::Result<Vec<DisplayNameObservation>> {
    let mut stmt = conn.prepare(
        "SELECT identity_id, profile_id, gateway_id, external_id, display_name,
                source_message_id, observed_at
         FROM display_name_observations
         WHERE identity_id = ?1
         ORDER BY observed_at ASC, id ASC
         LIMIT ?2",
    )?;
    let observations = stmt
        .query_map(params![identity.0.as_str(), limit as i64], |row| {
            let identity_id: String = row.get("identity_id")?;
            let profile_id: Option<String> = row.get("profile_id")?;
            Ok(DisplayNameObservation {
                identity: IdentityId(identity_id),
                profile: profile_id.map(ProfileId),
                gateway_id: row.get("gateway_id")?,
                external_id: row.get("external_id")?,
                display_name: row.get("display_name")?,
                source_message_id: row.get("source_message_id")?,
                observed_at: row.get("observed_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(observations)
}

pub(super) fn record_identity_conflict(
    conn: &Connection,
    conflict: &IdentityConflictRecord,
) -> anyhow::Result<()> {
    let tx = TxGuard::begin(conn)?;
    let resolution_json = serde_json::to_string(&conflict.resolution)?;
    conn.execute(
        "INSERT INTO identity_conflicts (
            id, channel_id, platform_message_id, primary_identity_id, reason,
            status, created_at, resolved_at, resolution_json
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            channel_id = excluded.channel_id,
            platform_message_id = excluded.platform_message_id,
            primary_identity_id = excluded.primary_identity_id,
            reason = excluded.reason,
            status = excluded.status,
            resolved_at = excluded.resolved_at,
            resolution_json = excluded.resolution_json",
        params![
            conflict.id.as_str(),
            conflict.channel.as_ref().map(|id| id.0.as_str()),
            conflict.platform_message_id.as_deref(),
            conflict.primary_identity.as_ref().map(|id| id.0.as_str()),
            conflict.reason.as_str(),
            conflict.status.as_str(),
            conflict.created_at,
            conflict.resolved_at,
            resolution_json,
        ],
    )?;
    for identity in &conflict.identities {
        conn.execute(
            "INSERT OR REPLACE INTO identity_conflict_identities (
                conflict_id, identity_id, role, source
             )
             VALUES (?1, ?2, ?3, ?4)",
            params![
                conflict.id.as_str(),
                identity.identity.0.as_str(),
                identity.role.as_str(),
                identity.source.as_deref(),
            ],
        )?;
    }
    for profile in &conflict.profiles {
        conn.execute(
            "INSERT OR IGNORE INTO identity_conflict_profiles (conflict_id, profile_id)
             VALUES (?1, ?2)",
            params![conflict.id.as_str(), profile.0.as_str()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn identity_conflicts(
    conn: &Connection,
    limit: usize,
) -> anyhow::Result<Vec<IdentityConflictRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, platform_message_id, primary_identity_id, reason,
                status, created_at, resolved_at, resolution_json
         FROM identity_conflicts
         ORDER BY created_at DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        let channel: Option<String> = row.get("channel_id")?;
        let primary_identity: Option<String> = row.get("primary_identity_id")?;
        let resolution_json: String = row.get("resolution_json")?;
        Ok(IdentityConflictRecord {
            id: row.get("id")?,
            channel: channel.map(ChannelId),
            platform_message_id: row.get("platform_message_id")?,
            primary_identity: primary_identity.map(IdentityId),
            reason: row.get("reason")?,
            status: row.get("status")?,
            created_at: row.get("created_at")?,
            resolved_at: row.get("resolved_at")?,
            resolution: serde_json::from_str(&resolution_json).unwrap_or_default(),
            identities: Vec::new(),
            profiles: Vec::new(),
        })
    })?;
    let mut conflicts = Vec::new();
    for row in rows {
        let mut conflict = row?;
        conflict.identities = conflict_identities(conn, &conflict.id)?;
        conflict.profiles = conflict_profiles(conn, &conflict.id)?;
        conflicts.push(conflict);
    }
    Ok(conflicts)
}

fn conflict_identities(
    conn: &Connection,
    conflict_id: &str,
) -> anyhow::Result<Vec<IdentityConflictIdentity>> {
    let mut stmt = conn.prepare(
        "SELECT identity_id, role, source
         FROM identity_conflict_identities
         WHERE conflict_id = ?1
         ORDER BY CASE role WHEN 'primary' THEN 0 ELSE 1 END, identity_id ASC",
    )?;
    let identities = stmt
        .query_map(params![conflict_id], |row| {
            let identity: String = row.get("identity_id")?;
            Ok(IdentityConflictIdentity {
                identity: IdentityId(identity),
                role: row.get("role")?,
                source: row.get("source")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(identities)
}

fn conflict_profiles(conn: &Connection, conflict_id: &str) -> anyhow::Result<Vec<ProfileId>> {
    let mut stmt = conn.prepare(
        "SELECT profile_id
         FROM identity_conflict_profiles
         WHERE conflict_id = ?1
         ORDER BY profile_id ASC",
    )?;
    let profiles = stmt
        .query_map(params![conflict_id], |row| {
            let profile: String = row.get("profile_id")?;
            Ok(ProfileId(profile))
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(profiles)
}

pub(super) fn get_profile_for_identity(
    conn: &Connection,
    identity: &IdentityId,
) -> anyhow::Result<Option<(Profile, ProfileIdentityLink)>> {
    conn.query_row(
        "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                l.profile_id, l.identity_id, l.status, l.confidence, l.evidence_json, l.created_at, l.removed_at
         FROM profile_identities l
         JOIN profiles p ON p.id = l.profile_id
         WHERE l.identity_id = ?1 AND l.status = 'active'
         ORDER BY l.confidence DESC, l.created_at DESC
         LIMIT 1",
        params![identity.0],
        |row| Ok((read_profile(row)?, read_profile_identity_link(row)?)),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn link_identity_to_profile(
    conn: &Connection,
    identity: &IdentityId,
    profile: &ProfileId,
    confidence: f32,
    evidence: Option<&serde_json::Value>,
) -> anyhow::Result<ProfileIdentityLink> {
    let tx = TxGuard::begin(conn)?;
    let evidence_json = evidence.map(serde_json::to_string).transpose()?;
    conn.execute(
        "UPDATE profile_identities
         SET status = 'removed', removed_at = unixepoch()
         WHERE identity_id = ?1 AND status = 'active' AND profile_id <> ?2",
        params![identity.0, profile.0],
    )?;
    conn.execute(
        "INSERT INTO profile_identities (profile_id, identity_id, status, confidence, evidence_json, created_at, removed_at)
         VALUES (?1, ?2, 'active', ?3, ?4, unixepoch(), NULL)
         ON CONFLICT(profile_id, identity_id) DO UPDATE SET
            status = 'active',
            confidence = excluded.confidence,
            evidence_json = excluded.evidence_json,
            removed_at = NULL",
        params![profile.0, identity.0, confidence, evidence_json],
    )?;
    let link = conn.query_row(
        "SELECT profile_id, identity_id, status, confidence, evidence_json, created_at, removed_at
         FROM profile_identities WHERE profile_id = ?1 AND identity_id = ?2",
        params![profile.0, identity.0],
        read_profile_identity_link,
    )?;
    tx.commit()?;
    Ok(link)
}

pub(super) fn unlink_identity_from_profile(
    conn: &Connection,
    identity: &IdentityId,
    profile: &ProfileId,
    reason: Option<&serde_json::Value>,
) -> anyhow::Result<()> {
    let reason_json = reason.map(serde_json::to_string).transpose()?;
    conn.execute(
        "UPDATE profile_identities
         SET status = 'removed', removed_at = unixepoch(), evidence_json = COALESCE(?3, evidence_json)
         WHERE identity_id = ?1 AND profile_id = ?2 AND status = 'active'",
        params![identity.0, profile.0, reason_json],
    )?;
    Ok(())
}

pub(super) fn record_identity_disclosure(
    conn: &Connection,
    audit: &IdentityDisclosureAudit,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO identity_disclosure_audits (
            id, action_id, requester_person_id, target_person_id, reason,
            allowed, identity_count, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            audit.id.as_str(),
            audit.action_id.as_str(),
            audit.requester_person.as_ref().map(|id| id.0.as_str()),
            audit.target_person.0.as_str(),
            audit.reason.as_str(),
            if audit.allowed { 1 } else { 0 },
            audit.identity_count,
            audit.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn identity_disclosures_for_person(
    conn: &Connection,
    person: &PersonId,
    limit: usize,
) -> anyhow::Result<Vec<IdentityDisclosureAudit>> {
    let mut stmt = conn.prepare(
        "SELECT id, action_id, requester_person_id, target_person_id, reason,
                allowed, identity_count, created_at
         FROM identity_disclosure_audits
         WHERE target_person_id = ?1
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let results = stmt
        .query_map(params![person.0, limit as i64], |row| {
            let requester: Option<String> = row.get("requester_person_id")?;
            let target: String = row.get("target_person_id")?;
            let allowed: i64 = row.get("allowed")?;
            Ok(IdentityDisclosureAudit {
                id: row.get("id")?,
                action_id: row.get("action_id")?,
                requester_person: requester.map(PersonId),
                target_person: PersonId(target),
                reason: row.get("reason")?,
                allowed: allowed != 0,
                identity_count: row.get("identity_count")?,
                created_at: row.get("created_at")?,
            })
        })?
        .filter_map(|row| row.ok())
        .collect();
    Ok(results)
}

pub(super) fn create_claim(conn: &Connection, claim: &IdentityClaim) -> anyhow::Result<()> {
    let evidence_json = serde_json::to_string(&claim.evidence_json)?;
    conn.execute(
        "INSERT INTO identity_claims (
            id, claimant_id, claimed_person_id, evidence, reason, evidence_json,
            confidence, status, created_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            claim.id,
            claim.claimant.0,
            claim.claimed_person.0,
            claim.evidence.as_str(),
            claim.reason.as_deref(),
            evidence_json,
            claim.confidence,
            claim.status.as_str(),
            claim.created_at,
        ],
    )?;
    Ok(())
}

pub(super) fn get_pending_claims(conn: &Connection) -> anyhow::Result<Vec<IdentityClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, claimant_id, claimed_person_id, evidence, reason, evidence_json,
                confidence, status, created_at, resolved_at
         FROM identity_claims WHERE status = 'pending' ORDER BY created_at DESC",
    )?;
    let results = stmt
        .query_map([], read_claim)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn get_recent_claims(
    conn: &Connection,
    claimant: Option<&PersonId>,
    claimed_person: Option<&PersonId>,
    since: i64,
) -> anyhow::Result<Vec<IdentityClaim>> {
    let claimant_id = claimant.map(|id| id.0.as_str());
    let claimed_person_id = claimed_person.map(|id| id.0.as_str());
    let mut stmt = conn.prepare(
        "SELECT id, claimant_id, claimed_person_id, evidence, reason, evidence_json,
                confidence, status, created_at, resolved_at
         FROM identity_claims
         WHERE created_at >= ?1
            AND (?2 IS NULL OR claimant_id = ?2)
            AND (?3 IS NULL OR claimed_person_id = ?3)
         ORDER BY created_at DESC",
    )?;
    let results = stmt
        .query_map(params![since, claimant_id, claimed_person_id], read_claim)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn resolve_claim(
    conn: &Connection,
    claim_id: &str,
    status: &ClaimStatus,
) -> anyhow::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "UPDATE identity_claims SET status = ?1, resolved_at = ?2 WHERE id = ?3",
        params![status.as_str(), now, claim_id],
    )?;
    Ok(())
}
