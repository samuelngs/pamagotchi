use super::support::SlowSqliteQuery;
use crate::identity::{
    Relation, RelationDirection, RelationSource, RelationStatus, SocialRelation,
};
use protocol::PersonId;
use rusqlite::{Connection, params};

pub(super) fn add_relation(
    conn: &Connection,
    a: &PersonId,
    b: &PersonId,
    relation: &Relation,
) -> anyhow::Result<()> {
    upsert_relation(
        conn,
        &SocialRelation::new(a.clone(), b.clone(), relation.clone()),
    )
}

pub(super) fn upsert_relation(conn: &Connection, relation: &SocialRelation) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    let created_at = if relation.created_at > 0 {
        relation.created_at
    } else {
        now
    };
    let updated_at = if relation.updated_at > 0 {
        relation.updated_at
    } else {
        now
    };
    let evidence_json = relation
        .evidence
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO social_graph (
            person_a, person_b, relation, direction, confidence, status, evidence_json,
            source_kind, asserted_by_person_id, created_at, updated_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(person_a, person_b, relation) DO UPDATE SET
            direction = excluded.direction,
            confidence = excluded.confidence,
            status = excluded.status,
            evidence_json = excluded.evidence_json,
            source_kind = excluded.source_kind,
            asserted_by_person_id = excluded.asserted_by_person_id,
            updated_at = excluded.updated_at",
        params![
            relation.person_a.0.as_str(),
            relation.person_b.0.as_str(),
            relation.relation.as_str(),
            relation.direction.as_str(),
            relation.confidence.clamp(0.0, 1.0),
            relation.status.as_str(),
            evidence_json,
            relation.source_kind.as_str(),
            relation
                .asserted_by
                .as_ref()
                .map(|person| person.0.as_str()),
            created_at,
            updated_at,
        ],
    )?;
    Ok(())
}

pub(super) fn get_relations(
    conn: &Connection,
    person: &PersonId,
) -> anyhow::Result<Vec<SocialRelation>> {
    let _slow_query = SlowSqliteQuery::start("get_relations");
    let mut stmt = conn.prepare(
        "SELECT person_a, person_b, relation, direction, confidence, status, evidence_json,
                source_kind, asserted_by_person_id, created_at, updated_at
         FROM social_graph
         WHERE person_a = ?1 OR person_b = ?1",
    )?;
    let results = stmt
        .query_map(params![person.0], |row| {
            let a: String = row.get("person_a")?;
            let b: String = row.get("person_b")?;
            let rel: String = row.get("relation")?;
            let relation = Relation::parse(&rel);
            let direction: Option<String> = row.get("direction")?;
            let status: String = row.get("status")?;
            let source_kind: String = row.get("source_kind")?;
            let evidence_json: Option<String> = row.get("evidence_json")?;
            Ok(SocialRelation {
                person_a: PersonId(a),
                person_b: PersonId(b),
                direction: direction
                    .as_deref()
                    .and_then(RelationDirection::parse)
                    .unwrap_or_else(|| relation.default_direction()),
                relation,
                confidence: row.get("confidence")?,
                status: RelationStatus::parse(&status),
                evidence: evidence_json.and_then(|json| serde_json::from_str(&json).ok()),
                source_kind: RelationSource::parse(&source_kind),
                asserted_by: row
                    .get::<_, Option<String>>("asserted_by_person_id")?
                    .map(PersonId),
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}

pub(super) fn remove_relation(
    conn: &Connection,
    a: &PersonId,
    b: &PersonId,
    relation: &Relation,
) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM social_graph WHERE person_a = ?1 AND person_b = ?2 AND relation = ?3",
        params![a.0, b.0, relation.as_str()],
    )?;
    Ok(())
}
