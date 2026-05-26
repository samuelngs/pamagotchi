use super::super::context::SessionContext;
use super::helpers::resolve_person_ref;
use crate::state::Authority;
use protocol::PersonId;
use serde_json::{Value, json};
use tracing::info;

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
            if name.is_some() {
                parts.push("name");
            }
            if summary.is_some() {
                parts.push("summary");
            }
            json!({
                "status": "updated",
                "ref": person_id.0,
                "fields": parts,
            })
            .to_string()
        }
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

pub async fn get(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return json!({
            "status": "error",
            "message": "No person ref provided and no current conversation partner.",
        })
        .to_string();
    };
    let include_identities = args["include_identities"].as_bool().unwrap_or(false);

    match ctx.store.get_person(&person_id).await {
        Ok(Some(person)) => {
            let first_seen = chrono::DateTime::from_timestamp(person.first_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.first_seen.to_string());
            let last_seen = chrono::DateTime::from_timestamp(person.last_seen, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| person.last_seen.to_string());

            let mut response = json!({
                "ref": person.id.0,
                "name": person.name,
                "summary": person.summary,
                "comm_style": person.comm_style,
                "first_seen": first_seen,
                "last_seen": last_seen,
            });

            if include_identities {
                match visible_identities(&person_id, ctx).await {
                    Ok(identities) => response["identities"] = identities,
                    Err(message) => response["identities_error"] = json!(message),
                }
            }

            response.to_string()
        }
        Ok(None) => json!({
            "status": "error",
            "message": "Person not found.",
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

async fn visible_identities(person: &PersonId, ctx: &SessionContext) -> Result<Value, String> {
    let current = ctx.messages.first().and_then(|m| m.person.as_ref());
    let is_self = current == Some(person);
    let is_owner = ctx.authority == Authority::Owner;

    if !is_self && !is_owner {
        return Err("Identities are private. If this is an identity claim, use request_identity_verification instead.".into());
    }

    match ctx.store.get_identities_for_person(person).await {
        Ok(identities) => Ok(json!(
            identities
                .into_iter()
                .map(|ident| json!({
                    "id": ident.id.0,
                    "gateway_id": ident.gateway_id,
                    "external_id": ident.external_id,
                    "display_name": ident.display_name,
                }))
                .collect::<Vec<_>>()
        )),
        Err(e) => Err(format!("{e}")),
    }
}
