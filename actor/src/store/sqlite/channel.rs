use crate::store::{
    ChannelFilter, ChannelMembership, ChannelMembershipStatus, ChannelRecord, GatewayRecord,
    SpaceRecord,
};
use protocol::{
    ChannelId, ChannelKind, ConversationId, GatewayId, ProfileId, SpaceId,
    generated_conversation_id,
};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn upsert_gateway(
    conn: &Connection,
    gateway: &GatewayRecord,
) -> anyhow::Result<GatewayId> {
    let metadata_json = serde_json::to_string(&gateway.metadata)?;
    conn.execute(
        "INSERT INTO gateways (id, kind, display_name, metadata_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
            kind = excluded.kind,
            display_name = excluded.display_name,
            metadata_json = excluded.metadata_json,
            updated_at = excluded.updated_at",
        params![
            gateway.id.0,
            gateway.kind,
            gateway.display_name,
            metadata_json,
            gateway.created_at,
            gateway.updated_at,
        ],
    )?;
    Ok(gateway.id.clone())
}

pub(super) fn upsert_space(conn: &Connection, space: &SpaceRecord) -> anyhow::Result<SpaceId> {
    let metadata_json = serde_json::to_string(&space.metadata)?;
    conn.execute(
        "INSERT INTO spaces (
            id, gateway_id, external_id, kind, display_name, metadata_json,
            created_at, updated_at, last_seen_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(gateway_id, external_id) DO UPDATE SET
            kind = excluded.kind,
            display_name = COALESCE(excluded.display_name, spaces.display_name),
            metadata_json = excluded.metadata_json,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at",
        params![
            space.id.0,
            space.gateway.0,
            space.external_id,
            space.kind.as_str(),
            space.display_name,
            metadata_json,
            space.created_at,
            space.updated_at,
            space.last_seen_at,
        ],
    )?;
    let id = conn.query_row(
        "SELECT id FROM spaces WHERE gateway_id = ?1 AND external_id = ?2",
        params![space.gateway.0, space.external_id],
        |row| row.get::<_, String>(0),
    )?;
    Ok(SpaceId(id))
}

pub(super) fn upsert_channel(
    conn: &Connection,
    channel: &ChannelRecord,
) -> anyhow::Result<ChannelId> {
    let metadata_json = serde_json::to_string(&channel.metadata)?;
    let space_id = channel.space.as_ref().map(|id| id.0.as_str());
    let parent_id = channel.parent.as_ref().map(|id| id.0.as_str());
    conn.execute(
        "INSERT INTO channels (
            id, gateway_id, external_id, kind, space_id, parent_channel_id,
            display_name, metadata_json, created_at, updated_at, last_seen_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(gateway_id, external_id) DO UPDATE SET
            kind = excluded.kind,
            space_id = excluded.space_id,
            parent_channel_id = excluded.parent_channel_id,
            display_name = COALESCE(excluded.display_name, channels.display_name),
            metadata_json = excluded.metadata_json,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at",
        params![
            channel.id.0,
            channel.gateway.0,
            channel.external_id,
            channel.kind.as_str(),
            space_id,
            parent_id,
            channel.display_name,
            metadata_json,
            channel.created_at,
            channel.updated_at,
            channel.last_seen_at,
        ],
    )?;
    let id = conn.query_row(
        "SELECT id FROM channels WHERE gateway_id = ?1 AND external_id = ?2",
        params![channel.gateway.0, channel.external_id],
        |row| row.get::<_, String>(0),
    )?;
    Ok(ChannelId(id))
}

pub(super) fn get_channel(
    conn: &Connection,
    id: &ChannelId,
) -> anyhow::Result<Option<ChannelRecord>> {
    conn.query_row(
        "SELECT id, gateway_id, external_id, kind, space_id, parent_channel_id,
                display_name, metadata_json, created_at, updated_at, last_seen_at
         FROM channels WHERE id = ?1",
        params![id.0],
        read_channel,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn resolve_channel(
    conn: &Connection,
    gateway: &GatewayId,
    external_id: &str,
) -> anyhow::Result<Option<ChannelRecord>> {
    conn.query_row(
        "SELECT id, gateway_id, external_id, kind, space_id, parent_channel_id,
                display_name, metadata_json, created_at, updated_at, last_seen_at
         FROM channels WHERE gateway_id = ?1 AND external_id = ?2",
        params![gateway.0, external_id],
        read_channel,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn list_channels(
    conn: &Connection,
    filter: &ChannelFilter,
) -> anyhow::Result<Vec<ChannelRecord>> {
    let gateway = filter.gateway.as_ref().map(|id| id.0.as_str());
    let kind = filter.kind.as_ref().map(ChannelKind::as_str);
    let mut stmt = conn.prepare(
        "SELECT id, gateway_id, external_id, kind, space_id, parent_channel_id,
                display_name, metadata_json, created_at, updated_at, last_seen_at
         FROM channels
         WHERE (?1 IS NULL OR gateway_id = ?1)
           AND (?2 IS NULL OR kind = ?2)
         ORDER BY last_seen_at DESC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![gateway, kind], read_channel)?
        .filter_map(|row| row.ok())
        .collect();
    Ok(rows)
}

pub(super) fn upsert_channel_membership(
    conn: &Connection,
    membership: &ChannelMembership,
) -> anyhow::Result<()> {
    let metadata_json = serde_json::to_string(&membership.metadata)?;
    conn.execute(
        "INSERT INTO channel_memberships (
            channel_id, profile_id, role, status, first_seen_at, last_seen_at, metadata_json
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(channel_id, profile_id) DO UPDATE SET
            role = COALESCE(excluded.role, channel_memberships.role),
            status = excluded.status,
            last_seen_at = excluded.last_seen_at,
            metadata_json = excluded.metadata_json",
        params![
            membership.channel.0,
            membership.profile.0,
            membership.role,
            membership.status.as_str(),
            membership.first_seen_at,
            membership.last_seen_at,
            metadata_json,
        ],
    )?;
    Ok(())
}

pub(super) fn list_channel_memberships(
    conn: &Connection,
    channel: &ChannelId,
) -> anyhow::Result<Vec<ChannelMembership>> {
    let mut stmt = conn.prepare(
        "SELECT channel_id, profile_id, role, status, first_seen_at, last_seen_at, metadata_json
         FROM channel_memberships
         WHERE channel_id = ?1
         ORDER BY last_seen_at DESC, profile_id ASC",
    )?;
    let rows = stmt
        .query_map(params![channel.0], read_membership)?
        .filter_map(|row| row.ok())
        .collect();
    Ok(rows)
}

pub(super) fn get_or_create_active_conversation(
    conn: &Connection,
    channel: &ChannelId,
    now: i64,
) -> anyhow::Result<ConversationId> {
    let existing = conn
        .query_row(
            "SELECT id FROM conversations WHERE channel_id = ?1 AND status = 'active' LIMIT 1",
            params![channel.0],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(ConversationId(id));
    }

    let id = generated_conversation_id();
    conn.execute(
        "INSERT INTO conversations (
            id, channel_id, status, started_at, last_message_at, message_count
         )
         VALUES (?1, ?2, 'active', ?3, ?3, 0)",
        params![id.0, channel.0, now],
    )?;
    Ok(id)
}

pub(super) fn channel_for_conversation(
    conn: &Connection,
    conversation: &ConversationId,
) -> anyhow::Result<Option<ChannelRecord>> {
    conn.query_row(
        "SELECT c.id, c.gateway_id, c.external_id, c.kind, c.space_id, c.parent_channel_id,
                c.display_name, c.metadata_json, c.created_at, c.updated_at, c.last_seen_at
         FROM conversations v
         JOIN channels c ON c.id = v.channel_id
         WHERE v.id = ?1
         LIMIT 1",
        params![conversation.0],
        read_channel,
    )
    .optional()
    .map_err(Into::into)
}

fn read_channel(row: &rusqlite::Row) -> rusqlite::Result<ChannelRecord> {
    let kind: String = row.get("kind")?;
    let space_id: Option<String> = row.get("space_id")?;
    let parent_id: Option<String> = row.get("parent_channel_id")?;
    let metadata_json: String = row.get("metadata_json")?;
    Ok(ChannelRecord {
        id: ChannelId(row.get("id")?),
        gateway: GatewayId(row.get("gateway_id")?),
        external_id: row.get("external_id")?,
        kind: ChannelKind::parse(&kind).unwrap_or(ChannelKind::Unknown),
        space: space_id.map(SpaceId),
        parent: parent_id.map(ChannelId),
        display_name: row.get("display_name")?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_seen_at: row.get("last_seen_at")?,
    })
}

fn read_membership(row: &rusqlite::Row) -> rusqlite::Result<ChannelMembership> {
    let status: String = row.get("status")?;
    let metadata_json: String = row.get("metadata_json")?;
    Ok(ChannelMembership {
        channel: ChannelId(row.get("channel_id")?),
        profile: ProfileId(row.get("profile_id")?),
        role: row.get("role")?,
        status: ChannelMembershipStatus::parse(&status)
            .unwrap_or(ChannelMembershipStatus::Observed),
        first_seen_at: row.get("first_seen_at")?,
        last_seen_at: row.get("last_seen_at")?,
        metadata: serde_json::from_str(&metadata_json).unwrap_or_default(),
    })
}
