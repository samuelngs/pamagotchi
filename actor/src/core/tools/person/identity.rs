use super::super::context::SessionContext;
use super::helpers::current_person;
use crate::identity::{ClaimEvidence, ClaimStatus, IdentityClaim, PersonProfileStatus};
use protocol::PersonId;
use serde_json::{Value, json};
use tracing::{info, warn};

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
        id: format!("identity-claim-{}", super::super::util::uuid_v4()),
        claimant: claimant.clone(),
        claimed_person: claimed_person.clone(),
        evidence: ClaimEvidence::SelfDeclaration,
        confidence: 0.0,
        status: ClaimStatus::Pending,
        created_at: super::super::util::now(),
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
            .send_message(&ident.gateway_id, &ident.external_id, &content, &[])
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
        .send_delta(super::super::util::empty_delta(Some(keep.clone())))
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
