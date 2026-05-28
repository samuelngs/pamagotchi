use super::support::TxGuard;
use crate::identity::{Group, GroupContext};
use protocol::{GroupId, PersonId};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn add_group(conn: &Connection, group: &Group) -> anyhow::Result<GroupId> {
    let tx = TxGuard::begin(conn)?;
    conn.execute(
        "INSERT INTO groups (id, name, gateway_id, external_id, context) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            group.id.0,
            group.name,
            group.gateway_id,
            group.external_id,
            group.context.as_str(),
        ],
    )?;
    for member in &group.members {
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, person_id) VALUES (?1, ?2)",
            params![group.id.0, member.0],
        )?;
    }
    tx.commit()?;
    Ok(group.id.clone())
}

pub(super) fn get_group(conn: &Connection, id: &GroupId) -> anyhow::Result<Option<Group>> {
    let row = conn
        .query_row(
            "SELECT id, name, gateway_id, external_id, context FROM groups WHERE id = ?1",
            params![id.0],
            |row| {
                let context_str: String = row.get("context")?;
                Ok((
                    row.get::<_, String>("id")?,
                    row.get::<_, String>("name")?,
                    row.get::<_, String>("gateway_id")?,
                    row.get::<_, String>("external_id")?,
                    context_str,
                ))
            },
        )
        .optional()?;

    let Some((gid, name, gateway_id, external_id, context)) = row else {
        return Ok(None);
    };

    let mut stmt = conn.prepare("SELECT person_id FROM group_members WHERE group_id = ?1")?;
    let members = stmt
        .query_map(params![gid], |row| Ok(PersonId(row.get::<_, String>(0)?)))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Some(Group {
        id: GroupId(gid),
        name,
        gateway_id,
        external_id,
        context: GroupContext::parse(&context),
        members,
    }))
}

pub(super) fn list_groups(conn: &Connection, limit: usize) -> anyhow::Result<Vec<Group>> {
    let mut stmt =
        conn.prepare("SELECT id FROM groups ORDER BY name COLLATE NOCASE, id LIMIT ?1")?;
    let ids = stmt
        .query_map(params![limit as i64], |row| {
            Ok(GroupId(row.get::<_, String>(0)?))
        })?
        .filter_map(|row| row.ok())
        .collect::<Vec<_>>();

    let mut groups = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(group) = get_group(conn, &id)? {
            groups.push(group);
        }
    }
    Ok(groups)
}

pub(super) fn add_group_member(
    conn: &Connection,
    group: &GroupId,
    person: &PersonId,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO group_members (group_id, person_id) VALUES (?1, ?2)",
        params![group.0, person.0],
    )?;
    Ok(())
}

pub(super) fn remove_group_member(
    conn: &Connection,
    group: &GroupId,
    person: &PersonId,
) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM group_members WHERE group_id = ?1 AND person_id = ?2",
        params![group.0, person.0],
    )?;
    Ok(())
}
