use super::context::SessionContext;
use crate::identity::{ClaimEvidence, ClaimStatus, IdentityClaim, PersonProfileStatus};
use crate::state::Authority;
use inference::Tool;
use protocol::{PersonId, ProfileId};
use serde_json::{Value, json};
use tracing::{info, warn};

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
            description: "Look up a person's current profile — name, summary, first/last seen. Set include_identities=true only when you need attached gateway identities; for privacy, only the owner or that same person can see them. Use request_identity_verification instead when someone claims to be a known person on another platform.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ref": {
                        "type": "string",
                        "description": "Person ref handle. Defaults to current conversation partner."
                    },
                    "include_identities": {
                        "type": "boolean",
                        "description": "Include attached gateway identities. Defaults to false and is allowed only for self or owner."
                    }
                }
            }),
        },
        Tool {
            name: "request_identity_verification".into(),
            description: "Start verification when the current profile claims to be a known person on another platform. Creates a pending claim and asks the known person's existing identities to confirm before linking profiles.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "claimed_person": {
                        "type": "string",
                        "description": "Person ref for the existing known person being claimed."
                    }
                },
                "required": ["claimed_person"]
            }),
        },
        Tool {
            name: "resolve_identity_verification".into(),
            description: "Confirm or deny a pending identity verification request. Use only when the current conversation partner is the known person who was asked to confirm.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "claim": {
                        "type": "string",
                        "description": "Pending identity claim ID. If omitted, uses the newest pending claim for the current person."
                    },
                    "confirmed": {
                        "type": "boolean",
                        "description": "true if the current person confirms the claimant is really them; false if denied."
                    }
                },
                "required": ["confirmed"]
            }),
        },
        Tool {
            name: "detach_profile_from_person".into(),
            description: "Detach a profile from a person grouping without deleting profile memories. Use when a same-person association was wrong or no longer trusted.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to detach."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID to detach from. Defaults to the current conversation person."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the link is being detached."
                    }
                },
                "required": ["profile"]
            }),
        },
        Tool {
            name: "reject_profile_person_link".into(),
            description: "Record that a profile should not be linked to a person. This preserves audit history and blocks weak repeated same-person assumptions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Profile ID to reject."
                    },
                    "person": {
                        "type": "string",
                        "description": "Person ID the profile should not be associated with."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why the link is rejected."
                    }
                },
                "required": ["profile", "person"]
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

pub async fn request_identity_verification(args: &Value, ctx: &SessionContext) -> String {
    let Some(claimant) = current_person(ctx) else {
        return json!({
            "status": "error",
            "message": "No current conversation partner to verify.",
        })
        .to_string();
    };

    let Some(claimed_ref) = args["claimed_person"].as_str().filter(|s| !s.is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide claimed_person.",
        })
        .to_string();
    };
    let claimed_person = PersonId(claimed_ref.to_string());

    if claimant == claimed_person {
        return json!({
            "status": "already_verified",
            "message": "The current identity is already linked to that person.",
            "person": claimed_person.0,
        })
        .to_string();
    }

    match ctx.store.get_person(&claimed_person).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return json!({
                "status": "error",
                "message": "Claimed person not found.",
            })
            .to_string();
        }
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    let claim = IdentityClaim {
        id: format!("identity-claim-{}", super::util::uuid_v4()),
        claimant: claimant.clone(),
        claimed_person: claimed_person.clone(),
        evidence: ClaimEvidence::SelfDeclaration,
        confidence: 0.0,
        status: ClaimStatus::Pending,
        created_at: super::util::now(),
        resolved_at: None,
    };

    if let Err(e) = ctx.store.create_claim(&claim).await {
        return json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string();
    }

    let identities = match ctx.store.get_identities_for_person(&claimed_person).await {
        Ok(identities) => identities,
        Err(e) => {
            return json!({
                "status": "pending",
                "claim": claim.id,
                "contacted": 0,
                "message": format!("Claim created, but contact lookup failed: {e}"),
            })
            .to_string();
        }
    };

    let current_target = ctx
        .messages
        .first()
        .map(|m| (m.gateway_id.as_str(), m.external_id.as_str()));
    let mut contacted = 0usize;
    let mut failed = 0usize;

    for ident in identities {
        if current_target == Some((ident.gateway_id.as_str(), ident.external_id.as_str())) {
            continue;
        }

        let platform = ctx
            .messages
            .first()
            .map(|m| m.gateway_id.as_str())
            .unwrap_or("another");
        let content = format!(
            "hey, someone on {platform} just claimed they are you. can you confirm if that was really you? reply yes or no. i won't link anything unless you confirm. verification id: {}",
            claim.id
        );

        match ctx
            .gateway
            .send_message(&ident.gateway_id, &ident.external_id, &content, None)
            .await
        {
            Ok(()) => contacted += 1,
            Err(e) => {
                failed += 1;
                warn!(
                    action = %ctx.action_id,
                    claim = %claim.id,
                    gateway = %ident.gateway_id,
                    %e,
                    "identity verification delivery failed"
                );
            }
        }
    }

    info!(
        action = %ctx.action_id,
        claim = %claim.id,
        claimant = %claimant.0,
        claimed = %claimed_person.0,
        contacted,
        failed,
        "identity verification requested"
    );

    json!({
        "status": "pending",
        "claim": claim.id,
        "contacted": contacted,
        "failed": failed,
    })
    .to_string()
}

pub async fn resolve_identity_verification(args: &Value, ctx: &SessionContext) -> String {
    let Some(current) = current_person(ctx) else {
        return json!({
            "status": "error",
            "message": "No current conversation partner.",
        })
        .to_string();
    };

    let Some(confirmed) = args["confirmed"].as_bool() else {
        return json!({
            "status": "error",
            "message": "Provide confirmed as true or false.",
        })
        .to_string();
    };

    let pending = match ctx.store.get_pending_claims().await {
        Ok(claims) => claims,
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    };

    let requested_claim = args["claim"].as_str().filter(|s| !s.is_empty());
    let Some(claim) = pending.into_iter().find(|claim| {
        claim.claimed_person == current && requested_claim.is_none_or(|id| id == claim.id)
    }) else {
        return json!({
            "status": "error",
            "message": "No pending identity claim for the current person.",
        })
        .to_string();
    };

    if !confirmed {
        return match ctx
            .store
            .resolve_claim(&claim.id, &ClaimStatus::Denied)
            .await
        {
            Ok(()) => json!({
                "status": "denied",
                "claim": claim.id,
            })
            .to_string(),
            Err(e) => json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string(),
        };
    }

    let keep = claim.claimed_person.clone();
    let claimant = claim.claimant.clone();
    let evidence = json!({
        "reason": "identity_claim_confirmed",
        "claim": claim.id,
        "claimant_person": claimant.0,
        "claimed_person": keep.0,
    });
    let profiles = match ctx.store.get_profiles_for_person(&claimant).await {
        Ok(profiles) => profiles,
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    };
    let mut linked_profiles = Vec::new();
    for (profile, link) in profiles {
        if !link.status.is_active_person_context() {
            continue;
        }
        if let Err(e) = ctx
            .store
            .attach_profile_to_person(
                &profile.id,
                &keep,
                PersonProfileStatus::Verified,
                1.0,
                Some(&evidence),
            )
            .await
        {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
        let _ = ctx
            .store
            .detach_profile_from_person(&profile.id, &claimant, Some(&evidence))
            .await;
        linked_profiles.push(profile.id.0);
    }

    if linked_profiles.is_empty() {
        return json!({
            "status": "error",
            "message": "No active profiles found for the claimant.",
        })
        .to_string();
    }

    ctx.state
        .send_delta(super::util::empty_delta(Some(keep.clone())))
        .await;

    match ctx
        .store
        .resolve_claim(&claim.id, &ClaimStatus::Linked)
        .await
    {
        Ok(()) => {
            info!(
                action = %ctx.action_id,
                claim = %claim.id,
                person = %keep.0,
                "identity verification confirmed and profiles linked"
            );
            json!({
                "status": "linked",
                "claim": claim.id,
                "person": keep.0,
                "linked_profiles": linked_profiles,
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

pub async fn detach_profile(args: &Value, ctx: &SessionContext) -> String {
    let Some(profile) = args["profile"].as_str().filter(|s| !s.is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide profile.",
        })
        .to_string();
    };
    let person = args["person"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|id| PersonId(id.to_string()))
        .or_else(|| current_person(ctx));
    let Some(person) = person else {
        return json!({
            "status": "error",
            "message": "Provide person or use this from a current person context.",
        })
        .to_string();
    };
    let reason = args["reason"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|reason| json!({ "reason": reason }));

    match ctx
        .store
        .detach_profile_from_person(&ProfileId(profile.to_string()), &person, reason.as_ref())
        .await
    {
        Ok(()) => json!({
            "status": "detached",
            "profile": profile,
            "person": person.0,
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

pub async fn reject_profile_person_link(args: &Value, ctx: &SessionContext) -> String {
    let Some(profile) = args["profile"].as_str().filter(|s| !s.is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide profile.",
        })
        .to_string();
    };
    let Some(person) = args["person"].as_str().filter(|s| !s.is_empty()) else {
        return json!({
            "status": "error",
            "message": "Provide person.",
        })
        .to_string();
    };
    let reason = args["reason"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|reason| json!({ "reason": reason }));
    let person = PersonId(person.to_string());
    let profile = ProfileId(profile.to_string());

    match ctx
        .store
        .attach_profile_to_person(
            &profile,
            &person,
            PersonProfileStatus::Rejected,
            1.0,
            reason.as_ref(),
        )
        .await
    {
        Ok(link) => json!({
            "status": "rejected",
            "profile": link.profile_id.0,
            "person": link.person_id.0,
        })
        .to_string(),
        Err(e) => json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string(),
    }
}

fn resolve_person_ref(args: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if let Some(r) = args["ref"].as_str().filter(|s| !s.is_empty()) {
        return Some(PersonId(r.to_string()));
    }
    ctx.messages.first().and_then(|m| m.person.clone())
}

fn current_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.messages.first().and_then(|m| m.person.clone())
}
