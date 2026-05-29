use super::super::context::SessionContext;
use super::helpers::{current_profile, resolve_person_ref};
use crate::state::Authority;
use crate::store::IdentityDisclosureAudit;
use protocol::{PersonId, ProfileId};
use serde_json::{Value, json};
use tracing::{info, warn};

pub async fn update(args: &Value, ctx: &SessionContext) -> String {
    let person_id = resolve_person_ref(args, ctx);
    let Some(person_id) = person_id else {
        return "No person ref provided and no current conversation partner.".into();
    };

    let name = args["name"].as_str();
    let summary = args["summary"].as_str();
    let comm_style = args["comm_style"].as_str();

    if name.is_none() && summary.is_none() && comm_style.is_none() {
        return "Nothing to update — provide name, summary, or comm_style.".into();
    }

    if name.is_some() || summary.is_some() {
        if let Err(e) = ctx.store.update_person(&person_id, name, summary).await {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }
    if let Some(style) = comm_style {
        if let Err(e) = ctx.store.update_comm_style(&person_id, style).await {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    info!(action = %ctx.action_id, person = %person_id.0, "person updated");
    let mut parts = Vec::new();
    if name.is_some() {
        parts.push("name");
    }
    if summary.is_some() {
        parts.push("summary");
    }
    if comm_style.is_some() {
        parts.push("comm_style");
    }
    json!({
        "status": "updated",
        "ref": person_id.0,
        "fields": parts,
    })
    .to_string()
}

pub async fn update_profile(args: &Value, ctx: &SessionContext) -> String {
    let profile_id = args["ref"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|id| ProfileId(id.to_string()))
        .or_else(|| current_profile(ctx));
    let Some(profile_id) = profile_id else {
        return "No profile ref provided and no current profile.".into();
    };

    let display_name = args["display_name"].as_str();
    let summary = args["summary"].as_str();
    let comm_style = args["comm_style"].as_str();

    if display_name.is_none() && summary.is_none() && comm_style.is_none() {
        return "Nothing to update — provide display_name, summary, or comm_style.".into();
    }

    if display_name.is_some() || summary.is_some() {
        if let Err(e) = ctx
            .store
            .update_profile(&profile_id, display_name, summary)
            .await
        {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }
    if let Some(style) = comm_style {
        if let Err(e) = ctx
            .store
            .update_profile_comm_style(&profile_id, style)
            .await
        {
            return json!({
            "status": "error",
            "message": format!("{e}"),
            })
            .to_string();
        }
    }

    info!(action = %ctx.action_id, profile = %profile_id.0, "profile updated");
    let mut parts = Vec::new();
    if display_name.is_some() {
        parts.push("display_name");
    }
    if summary.is_some() {
        parts.push("summary");
    }
    if comm_style.is_some() {
        parts.push("comm_style");
    }
    json!({
        "status": "updated",
        "ref": profile_id.0,
        "fields": parts,
    })
    .to_string()
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
    let delivery_required = args["delivery_required"].as_bool().unwrap_or(false);
    let identity_reason = match identity_lookup_reason(args) {
        Ok(reason) => reason,
        Err(message) => {
            return json!({
                "status": "error",
                "message": message,
            })
            .to_string();
        }
    };

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
                let reason = identity_reason.expect("validated reason exists");
                info!(
                    action = %ctx.action_id,
                    person = %person_id.0,
                    reason,
                    "person identities requested"
                );
                match visible_identities(&person_id, ctx, delivery_required).await {
                    Ok(identities) => {
                        let identity_count = identities.as_array().map_or(0, |items| items.len());
                        match record_identity_disclosure(
                            ctx,
                            &person_id,
                            reason,
                            true,
                            identity_count as u32,
                        )
                        .await
                        {
                            Ok(()) => response["identities"] = identities,
                            Err(e) => {
                                warn!(
                                    action = %ctx.action_id,
                                    person = %person_id.0,
                                    %e,
                                    "refusing identity disclosure because audit failed"
                                );
                                response["identities_error"] = json!(
                                    "Identity lookup could not be audited, so identities were not returned."
                                );
                            }
                        }
                    }
                    Err(message) => {
                        if let Err(e) =
                            record_identity_disclosure(ctx, &person_id, reason, false, 0).await
                        {
                            warn!(
                                action = %ctx.action_id,
                                person = %person_id.0,
                                %e,
                                "failed to audit denied identity disclosure"
                            );
                        }
                        response["identities_error"] = json!(message);
                    }
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

async fn record_identity_disclosure(
    ctx: &SessionContext,
    target_person: &PersonId,
    reason: &str,
    allowed: bool,
    identity_count: u32,
) -> anyhow::Result<()> {
    let audit = IdentityDisclosureAudit {
        id: format!("identity-disclosure-{}", super::super::util::uuid_v4()),
        action_id: ctx.action_id.0.clone(),
        requester_person: ctx
            .messages
            .first()
            .and_then(|message| message.person.clone()),
        target_person: target_person.clone(),
        reason: reason.to_string(),
        allowed,
        identity_count,
        created_at: super::super::util::now(),
    };
    ctx.store.record_identity_disclosure(&audit).await
}

fn identity_lookup_reason(args: &Value) -> Result<Option<&str>, &'static str> {
    if !args["include_identities"].as_bool().unwrap_or(false) {
        return Ok(None);
    }

    args["reason"]
        .as_str()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(Some)
        .ok_or("Provide reason when include_identities=true so the identity lookup is auditable.")
}

async fn visible_identities(
    person: &PersonId,
    ctx: &SessionContext,
    reveal_external_ids: bool,
) -> Result<Value, String> {
    let current = ctx.messages.first().and_then(|m| m.person.as_ref());
    let is_self = current == Some(person);
    let is_chosen_human = ctx.authority == Authority::ChosenHuman;

    if !is_self && !is_chosen_human {
        return Err("Identities are private. If this is an identity claim, use request_identity_verification instead.".into());
    }

    match ctx.store.get_identities_for_person(person).await {
        Ok(identities) => Ok(json!(
            identities
                .into_iter()
                .map(|ident| {
                    let mut item = json!({
                        "id": ident.id.0,
                        "gateway_id": ident.gateway_id,
                        "display_name": ident.display_name,
                    });
                    if reveal_external_ids {
                        item["external_id"] = json!(ident.external_id);
                    } else {
                        item["external_id_masked"] = json!(mask_external_id(&ident.external_id));
                    }
                    item
                })
                .collect::<Vec<_>>()
        )),
        Err(e) => Err(format!("{e}")),
    }
}

fn mask_external_id(external_id: &str) -> String {
    let chars = external_id.chars().collect::<Vec<_>>();
    if chars.len() <= 4 {
        return "*".repeat(chars.len().max(1));
    }
    let tail = chars[chars.len() - 4..].iter().collect::<String>();
    format!("***{tail}")
}

#[cfg(test)]
mod tests;
