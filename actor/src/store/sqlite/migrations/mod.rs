use rusqlite::Connection;

mod common;
mod v001_memory_schema_v2;
mod v002_message_source_and_reply_context;
mod v003_action_transcripts_and_intents;
mod v004_social_graph_metadata;
mod v005_identity_claim_metadata;
mod v006_thought_metadata;
mod v007_conversation_summary_metadata;
mod v008_identity_disclosure_audit;
mod v009_outbound_delivery_audit;
mod v010_event_inbox;
mod v011_action_message_source_dedupe;
mod v012_state_journal;
mod v013_display_name_observations;
mod v014_review_outputs;
mod v015_action_prompt_snapshots;
mod v016_intent_chosen_person_approval;
mod v017_event_inbox_failure_error;
mod v018_action_run_outcome_memory_artifacts;
mod v019_social_graph_direction;

struct Migration {
    version: i64,
    name: &'static str,
    apply: fn(&Connection) -> anyhow::Result<()>,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "memory_schema_v2",
        apply: v001_memory_schema_v2::apply,
    },
    Migration {
        version: 2,
        name: "message_source_and_reply_context",
        apply: v002_message_source_and_reply_context::apply,
    },
    Migration {
        version: 3,
        name: "action_transcripts_and_intents",
        apply: v003_action_transcripts_and_intents::apply,
    },
    Migration {
        version: 4,
        name: "social_graph_metadata",
        apply: v004_social_graph_metadata::apply,
    },
    Migration {
        version: 5,
        name: "identity_claim_metadata",
        apply: v005_identity_claim_metadata::apply,
    },
    Migration {
        version: 6,
        name: "thought_metadata",
        apply: v006_thought_metadata::apply,
    },
    Migration {
        version: 7,
        name: "conversation_summary_metadata",
        apply: v007_conversation_summary_metadata::apply,
    },
    Migration {
        version: 8,
        name: "identity_disclosure_audit",
        apply: v008_identity_disclosure_audit::apply,
    },
    Migration {
        version: 9,
        name: "outbound_delivery_audit",
        apply: v009_outbound_delivery_audit::apply,
    },
    Migration {
        version: 10,
        name: "event_inbox",
        apply: v010_event_inbox::apply,
    },
    Migration {
        version: 11,
        name: "action_message_source_dedupe",
        apply: v011_action_message_source_dedupe::apply,
    },
    Migration {
        version: 12,
        name: "state_journal",
        apply: v012_state_journal::apply,
    },
    Migration {
        version: 13,
        name: "display_name_observations",
        apply: v013_display_name_observations::apply,
    },
    Migration {
        version: 14,
        name: "review_outputs",
        apply: v014_review_outputs::apply,
    },
    Migration {
        version: 15,
        name: "action_prompt_snapshots",
        apply: v015_action_prompt_snapshots::apply,
    },
    Migration {
        version: 16,
        name: "intent_chosen_person_approval",
        apply: v016_intent_chosen_person_approval::apply,
    },
    Migration {
        version: 17,
        name: "event_inbox_failure_error",
        apply: v017_event_inbox_failure_error::apply,
    },
    Migration {
        version: 18,
        name: "action_run_outcome_memory_artifacts",
        apply: v018_action_run_outcome_memory_artifacts::apply,
    },
    Migration {
        version: 19,
        name: "social_graph_direction",
        apply: v019_social_graph_direction::apply,
    },
];

pub(super) fn ensure_migration_table(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

pub(super) fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    for migration in MIGRATIONS {
        run_migration(conn, migration)?;
    }
    Ok(())
}

fn run_migration(conn: &Connection, migration: &Migration) -> anyhow::Result<()> {
    let already_applied: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
        [migration.version],
        |row| row.get(0),
    )?;
    if already_applied {
        return Ok(());
    }

    (migration.apply)(conn)?;
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at)
         VALUES (?1, ?2, unixepoch())",
        rusqlite::params![migration.version, migration.name],
    )?;
    Ok(())
}
