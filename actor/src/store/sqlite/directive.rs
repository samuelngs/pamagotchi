use super::rows::read_directive;
use super::support::TxGuard;
use crate::state::{BehaviorDirective, RelationshipStanding};
use protocol::{GroupId, PersonId};
use rusqlite::{Connection, params};

pub(super) fn add_directive(
    conn: &Connection,
    directive: &BehaviorDirective,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO behavior_directives (id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            directive.id,
            directive.scope.scope_type(),
            directive.scope.scope_value(),
            directive.directive,
            directive.set_by.0,
            directive.priority,
            directive.active as i32,
            directive.created_at,
            directive.expires_at,
        ],
    )?;
    Ok(())
}

pub(super) fn get_directives_for_context(
    conn: &Connection,
    person: &PersonId,
    relationship_standing: &RelationshipStanding,
    group: Option<&GroupId>,
) -> anyhow::Result<Vec<BehaviorDirective>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut stmt = conn.prepare(
        "SELECT id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at
         FROM behavior_directives
         WHERE active = 1
           AND (expires_at IS NULL OR expires_at > ?4)
           AND (
             scope_type = 'global'
             OR (scope_type = 'person' AND scope_value = ?1)
             OR (scope_type = 'relationship_standing' AND scope_value = ?2)
             OR (scope_type = 'group' AND scope_value = ?3)
         )
         ORDER BY priority DESC",
    )?;

    let group_value: Option<&str> = group.map(|g| g.0.as_str());
    let results = stmt
        .query_map(
            params![person.0, relationship_standing.as_str(), group_value, now],
            read_directive,
        )?
        .filter_map(|row| row.ok())
        .collect();
    Ok(results)
}

pub(super) fn update_directive(
    conn: &Connection,
    id: &str,
    directive: Option<&str>,
    active: Option<bool>,
    priority: Option<i32>,
    expires_at: Option<Option<i64>>,
) -> anyhow::Result<()> {
    let tx = TxGuard::begin(conn)?;
    if let Some(text) = directive {
        conn.execute(
            "UPDATE behavior_directives SET directive = ?1 WHERE id = ?2",
            params![text, id],
        )?;
    }
    if let Some(active) = active {
        conn.execute(
            "UPDATE behavior_directives SET active = ?1 WHERE id = ?2",
            params![active as i32, id],
        )?;
    }
    if let Some(priority) = priority {
        conn.execute(
            "UPDATE behavior_directives SET priority = ?1 WHERE id = ?2",
            params![priority, id],
        )?;
    }
    if let Some(expires) = expires_at {
        conn.execute(
            "UPDATE behavior_directives SET expires_at = ?1 WHERE id = ?2",
            params![expires, id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn remove_directive(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    let rows = conn.execute("DELETE FROM behavior_directives WHERE id = ?1", params![id])?;
    Ok(rows > 0)
}

pub(super) fn list_directives(conn: &Connection) -> anyhow::Result<Vec<BehaviorDirective>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope_type, scope_value, directive, set_by, priority, active, created_at, expires_at
         FROM behavior_directives ORDER BY priority DESC, created_at DESC",
    )?;
    let results = stmt
        .query_map([], read_directive)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}
