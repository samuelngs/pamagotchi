use super::rows::{read_identity, read_person, read_person_profile_link, read_profile};
use super::support::{SlowSqliteQuery, TxGuard};
use crate::identity::{Identity, Person, PersonProfileLink, PersonProfileStatus, Profile};
use protocol::{PersonId, ProfileId};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn add_profile(conn: &Connection, profile: &Profile) -> anyhow::Result<ProfileId> {
    conn.execute(
        "INSERT INTO profiles (id, display_name, summary, comm_style, first_seen, last_seen, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            profile.id.0,
            profile.display_name,
            profile.summary,
            profile.comm_style,
            profile.first_seen,
            profile.last_seen,
            profile.created_at,
            profile.updated_at,
        ],
    )?;
    Ok(profile.id.clone())
}

pub(super) fn get_profile(conn: &Connection, id: &ProfileId) -> anyhow::Result<Option<Profile>> {
    conn.query_row(
        "SELECT id, display_name, summary, comm_style, first_seen, last_seen, created_at, updated_at
         FROM profiles WHERE id = ?1",
        params![id.0],
        read_profile,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn list_profiles(conn: &Connection) -> anyhow::Result<Vec<Profile>> {
    let _slow_query = SlowSqliteQuery::start("list_profiles");
    let mut stmt = conn.prepare(
        "SELECT id, display_name, summary, comm_style, first_seen, last_seen, created_at, updated_at
         FROM profiles
         ORDER BY updated_at DESC, created_at DESC",
    )?;
    stmt.query_map([], read_profile)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(super) fn update_profile(
    conn: &Connection,
    id: &ProfileId,
    display_name: Option<&str>,
    summary: Option<&str>,
) -> anyhow::Result<()> {
    let tx = TxGuard::begin(conn)?;
    if let Some(display_name) = display_name {
        conn.execute(
            "UPDATE profiles SET display_name = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![display_name, id.0],
        )?;
    }
    if let Some(summary) = summary {
        conn.execute(
            "UPDATE profiles SET summary = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![summary, id.0],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn update_profile_comm_style(
    conn: &Connection,
    id: &ProfileId,
    style: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE profiles SET comm_style = ?1, updated_at = unixepoch() WHERE id = ?2",
        params![style, id.0],
    )?;
    Ok(())
}

pub(super) fn touch_profile(conn: &Connection, id: &ProfileId) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE profiles SET last_seen = unixepoch(), updated_at = unixepoch() WHERE id = ?1",
        params![id.0],
    )?;
    Ok(())
}

pub(super) fn add_person(conn: &Connection, person: &Person) -> anyhow::Result<PersonId> {
    conn.execute(
        "INSERT INTO persons (id, display_name, summary, comm_style, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            person.id.0,
            person.name,
            person.summary,
            person.comm_style,
            person.first_seen,
            person.last_seen
        ],
    )?;
    Ok(person.id.clone())
}

pub(super) fn get_person(conn: &Connection, id: &PersonId) -> anyhow::Result<Option<Person>> {
    conn.query_row(
        "SELECT id, display_name, summary, comm_style, created_at, updated_at FROM persons WHERE id = ?1",
        params![id.0],
        read_person,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn update_person(
    conn: &Connection,
    id: &PersonId,
    name: Option<&str>,
    summary: Option<&str>,
) -> anyhow::Result<()> {
    let tx = TxGuard::begin(conn)?;
    if let Some(name) = name {
        conn.execute(
            "UPDATE persons SET display_name = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![name, id.0],
        )?;
    }
    if let Some(summary) = summary {
        conn.execute(
            "UPDATE persons SET summary = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![summary, id.0],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn update_comm_style(
    conn: &Connection,
    id: &PersonId,
    style: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE persons SET comm_style = ?1, updated_at = unixepoch() WHERE id = ?2",
        params![style, id.0],
    )?;
    Ok(())
}

pub(super) fn touch_person(conn: &Connection, id: &PersonId) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE persons SET updated_at = unixepoch() WHERE id = ?1",
        params![id.0],
    )?;
    Ok(())
}

pub(super) fn list_persons(conn: &Connection) -> anyhow::Result<Vec<Person>> {
    let _slow_query = SlowSqliteQuery::start("list_persons");
    let mut stmt = conn.prepare(
        "SELECT id, display_name, summary, comm_style, created_at, updated_at FROM persons ORDER BY display_name",
    )?;
    let results = stmt
        .query_map([], read_person)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn attach_profile_to_person(
    conn: &Connection,
    profile: &ProfileId,
    person: &PersonId,
    status: PersonProfileStatus,
    confidence: f32,
    evidence: Option<&serde_json::Value>,
) -> anyhow::Result<PersonProfileLink> {
    let tx = TxGuard::begin(conn)?;
    let evidence_json = evidence.map(serde_json::to_string).transpose()?;
    if status.is_active_person_context() {
        conn.execute(
            "UPDATE person_profiles
             SET status = 'detached', updated_at = unixepoch(), detached_at = unixepoch()
             WHERE profile_id = ?1 AND status IN ('verified', 'likely') AND person_id <> ?2",
            params![profile.0, person.0],
        )?;
    }
    conn.execute(
        "INSERT INTO person_profiles (person_id, profile_id, status, confidence, evidence_json, created_at, updated_at, detached_at)
         VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch(), NULL)
         ON CONFLICT(person_id, profile_id) DO UPDATE SET
            status = excluded.status,
            confidence = excluded.confidence,
            evidence_json = excluded.evidence_json,
            updated_at = unixepoch(),
            detached_at = CASE WHEN excluded.status IN ('detached', 'rejected') THEN unixepoch() ELSE NULL END",
        params![person.0, profile.0, status.as_str(), confidence, evidence_json],
    )?;
    let link = conn.query_row(
        "SELECT person_id, profile_id, status, confidence, evidence_json, created_at, updated_at, detached_at
         FROM person_profiles WHERE person_id = ?1 AND profile_id = ?2",
        params![person.0, profile.0],
        read_person_profile_link,
    )?;
    tx.commit()?;
    Ok(link)
}

pub(super) fn detach_profile_from_person(
    conn: &Connection,
    profile: &ProfileId,
    person: &PersonId,
    reason: Option<&serde_json::Value>,
) -> anyhow::Result<()> {
    let reason_json = reason.map(serde_json::to_string).transpose()?;
    conn.execute(
        "UPDATE person_profiles
         SET status = 'detached', evidence_json = COALESCE(?3, evidence_json),
             updated_at = unixepoch(), detached_at = unixepoch()
         WHERE profile_id = ?1 AND person_id = ?2 AND status <> 'detached'",
        params![profile.0, person.0, reason_json],
    )?;
    Ok(())
}

pub(super) fn get_person_for_profile(
    conn: &Connection,
    profile: &ProfileId,
) -> anyhow::Result<Option<(Person, PersonProfileLink)>> {
    conn.query_row(
        "SELECT p.id, p.display_name, p.summary, p.comm_style, p.created_at, p.updated_at,
                l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
         FROM person_profiles l
         JOIN persons p ON p.id = l.person_id
         WHERE l.profile_id = ?1 AND l.status IN ('verified', 'likely')
         ORDER BY CASE l.status WHEN 'verified' THEN 0 ELSE 1 END, l.confidence DESC, l.updated_at DESC
         LIMIT 1",
        params![profile.0],
        |row| Ok((read_person(row)?, read_person_profile_link(row)?)),
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn get_profiles_for_person(
    conn: &Connection,
    person: &PersonId,
) -> anyhow::Result<Vec<(Profile, PersonProfileLink)>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.display_name, p.summary, p.comm_style, p.first_seen, p.last_seen, p.created_at, p.updated_at,
                l.person_id, l.profile_id, l.status, l.confidence, l.evidence_json, l.created_at, l.updated_at, l.detached_at
         FROM person_profiles l
         JOIN profiles p ON p.id = l.profile_id
         WHERE l.person_id = ?1
         ORDER BY l.status, l.confidence DESC, l.updated_at DESC",
    )?;
    let results = stmt
        .query_map(params![person.0], |row| {
            Ok((read_profile(row)?, read_person_profile_link(row)?))
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn get_identities_for_person(
    conn: &Connection,
    person: &PersonId,
) -> anyhow::Result<Vec<Identity>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.gateway_id, i.external_id, i.display_name, i.metadata_json, i.created_at, i.last_seen_at
         FROM identities i
         JOIN profile_identities pi ON pi.identity_id = i.id AND pi.status = 'active'
         JOIN person_profiles pp ON pp.profile_id = pi.profile_id AND pp.status IN ('verified', 'likely')
         WHERE pp.person_id = ?1
         ORDER BY i.gateway_id, i.external_id",
    )?;
    let results = stmt
        .query_map(params![person.0], read_identity)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn merge_person_context(
    conn: &Connection,
    from: &PersonId,
    into: &PersonId,
) -> anyhow::Result<()> {
    if from == into {
        return Ok(());
    }
    let tx = TxGuard::begin(conn)?;

    conn.execute(
        "DELETE FROM memory_subjects
         WHERE subject_type = 'person' AND subject_id = ?1
           AND EXISTS (
                SELECT 1 FROM memory_subjects existing
                WHERE existing.memory_id = memory_subjects.memory_id
                  AND existing.subject_type = 'person'
                  AND existing.subject_id = ?2
                  AND COALESCE(existing.role, '') = COALESCE(memory_subjects.role, '')
           )",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE memory_subjects
         SET subject_id = ?2
         WHERE subject_type = 'person' AND subject_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE messages SET person_id = ?2 WHERE person_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE conversations SET person_id = ?2 WHERE person_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE intents SET person_id = ?2, updated_at = unixepoch() WHERE person_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE behavior_directives
         SET scope_value = ?2
         WHERE scope_type = 'person' AND scope_value = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO group_members (group_id, person_id)
         SELECT group_id, ?2 FROM group_members WHERE person_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "DELETE FROM group_members WHERE person_id = ?1",
        params![from.0],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO social_graph (
            person_a, person_b, relation, direction, confidence, status, evidence_json,
            source_kind, asserted_by_person_id, created_at, updated_at
         )
         SELECT
            CASE WHEN person_a = ?1 THEN ?2 ELSE person_a END,
            CASE WHEN person_b = ?1 THEN ?2 ELSE person_b END,
            relation, direction, confidence, status, evidence_json, source_kind,
            CASE WHEN asserted_by_person_id = ?1 THEN ?2 ELSE asserted_by_person_id END,
            created_at, unixepoch()
         FROM social_graph
         WHERE (person_a = ?1 OR person_b = ?1)
           AND NOT (
                CASE WHEN person_a = ?1 THEN ?2 ELSE person_a END
                =
                CASE WHEN person_b = ?1 THEN ?2 ELSE person_b END
           )",
        params![from.0, into.0],
    )?;
    conn.execute(
        "UPDATE social_graph SET asserted_by_person_id = ?2 WHERE asserted_by_person_id = ?1",
        params![from.0, into.0],
    )?;
    conn.execute(
        "DELETE FROM social_graph WHERE person_a = ?1 OR person_b = ?1",
        params![from.0],
    )?;

    tx.commit()?;
    Ok(())
}
