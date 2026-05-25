use inference::Tool;
use protocol::PersonId;
use serde_json::{json, Value};
use tracing::info;
use super::context::SessionContext;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "update_person".into(),
            description: "Update a person's name or summary. Use after learning someone's name or after building a clearer picture of who they are.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle (e.g. x7Kp2mQ). Defaults to current conversation partner."
                    },
                    "name": {
                        "type": "string",
                        "description": "Person's name. Set when you learn it."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Compressed impression — who they are, what they care about, how they communicate. Overwrites previous summary."
                    }
                }
            }),
        },
        Tool {
            name: "get_person".into(),
            description: "Look up a person's current profile — name, summary, first/last seen.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle. Defaults to current conversation partner."
                    }
                }
            }),
        },
    ]
}

pub async fn update(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return "No person ref provided and no current conversation partner.".into();
    };

    let name = args["name"].as_str();
    let summary = args["summary"].as_str();

    if name.is_none() && summary.is_none() {
        return "Nothing to update — provide name or summary.".into();
    }

    match ctx.store.update_person(&person_id, name, summary).await {
        Ok(()) => {
            info!(action = %ctx.action_id, person = %person_id.0, "person updated");
            let mut parts = Vec::new();
            if name.is_some() { parts.push("name"); }
            if summary.is_some() { parts.push("summary"); }
            json!({
                "status": "updated",
                "ref": person_id.0,
                "fields": parts,
            }).to_string()
        }
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        }).to_string(),
    }
}

pub async fn get(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return json!({
            "status": "error",
            "message": "No person ref provided and no current conversation partner.",
        }).to_string();
    };

    match ctx.store.get_person(&person_id).await {
        Ok(Some(person)) => {
            let first_seen = chrono::DateTime::from_timestamp(person.first_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.first_seen.to_string());
            let last_seen = chrono::DateTime::from_timestamp(person.last_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.last_seen.to_string());

            json!({
                "ref": person.id.0,
                "name": person.name,
                "summary": person.summary,
                "comm_style": person.comm_style,
                "first_seen": first_seen,
                "last_seen": last_seen,
            }).to_string()
        }
        Ok(None) => json!({
            "status": "error",
            "message": "Person not found.",
        }).to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        }).to_string(),
    }
}

fn resolve_person_ref(args: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if let Some(r) = args["ref"].as_str() {
        return Some(PersonId(r.to_string()));
    }
    ctx.messages.first().and_then(|m| m.person.clone())
}
