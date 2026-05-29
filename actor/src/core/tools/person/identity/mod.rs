use super::super::context::SessionContext;
use super::helpers::{current_person, remove_detached_person_subject_from_profile_memories};
use crate::identity::{ClaimEvidence, ClaimStatus, IdentityClaim, Person, PersonProfileStatus};
use crate::state::RelationshipStanding;
use crate::store::IntentRecord;
use protocol::PersonId;
use serde_json::{Value, json};
use tracing::{info, warn};

const CLAIM_RATE_WINDOW_SECS: i64 = 24 * 60 * 60;
const MAX_CLAIMS_PER_CLAIMANT: usize = 3;
const MAX_CLAIMS_PER_CLAIMED_PERSON: usize = 5;
const MIN_CONFIDENCE_TO_CONTACT: f32 = 0.4;

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

    let Some(reason) = args["reason"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return json!({
            "status": "error",
            "message": "Provide a short reason from the current conversation before contacting another person.",
        })
        .to_string();
    };

    if claimant == claimed_person {
        return json!({
            "status": "already_verified",
            "message": "The current identity is already linked to that person.",
            "person": claimed_person.0,
        })
        .to_string();
    }

    let claimed = match ctx.store.get_person(&claimed_person).await {
        Ok(Some(person)) => person,
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
    };

    let Some(claim_message) = recent_explicit_claim_message(ctx, &claimed) else {
        return json!({
            "status": "error",
            "message": "Identity verification requires a recent explicit identity claim in the current conversation.",
        })
        .to_string();
    };

    let now = super::super::util::now();
    let since = now - CLAIM_RATE_WINDOW_SECS;
    match ctx
        .store
        .get_recent_claims(Some(&claimant), Some(&claimed_person), since)
        .await
    {
        Ok(claims) => {
            if let Some(existing) = claims
                .iter()
                .find(|claim| matches!(claim.status, ClaimStatus::Pending))
                .or_else(|| claims.first())
            {
                return json!({
                    "status": "rate_limited",
                    "claim": existing.id,
                    "message": "A recent verification request already exists for this claimant and claimed person.",
                })
                .to_string();
            }
        }
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    match ctx
        .store
        .get_recent_claims(Some(&claimant), None, since)
        .await
    {
        Ok(claims) if claims.len() >= MAX_CLAIMS_PER_CLAIMANT => {
            return json!({
                "status": "rate_limited",
                "message": "Too many identity verification requests from this claimant recently.",
            })
            .to_string();
        }
        Ok(_) => {}
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    match ctx
        .store
        .get_recent_claims(None, Some(&claimed_person), since)
        .await
    {
        Ok(claims) if claims.len() >= MAX_CLAIMS_PER_CLAIMED_PERSON => {
            return json!({
                "status": "rate_limited",
                "message": "Too many recent identity verification requests for that person.",
            })
            .to_string();
        }
        Ok(_) => {}
        Err(e) => {
            return json!({
                "status": "error",
                "message": format!("{e}"),
            })
            .to_string();
        }
    }

    let evidence = match parse_allowed_claim_evidence(args, ctx) {
        Ok(evidence) => evidence,
        Err(message) => {
            return json!({
                "status": "error",
                "message": message,
            })
            .to_string();
        }
    };
    let confidence = match evidence {
        ClaimEvidence::ChosenHumanVouched | ClaimEvidence::ConfiguredIdentity => 0.8,
        ClaimEvidence::MutualClaim | ClaimEvidence::SharedKnowledge => 0.4,
        ClaimEvidence::SelfDeclaration => 0.05,
    };
    let evidence_json = json!({
        "reason": reason,
        "message_id": claim_message.message_id.as_str(),
        "conversation_id": claim_message.conversation.0.as_str(),
        "gateway_id": claim_message.gateway_id.as_str(),
        "claimant_person": claimant.0.as_str(),
        "claimed_person": claimed_person.0.as_str(),
    });

    let claim = IdentityClaim {
        id: format!("identity-claim-{}", super::super::util::uuid_v4()),
        claimant: claimant.clone(),
        claimed_person: claimed_person.clone(),
        evidence,
        reason: Some(reason.to_string()),
        evidence_json,
        confidence,
        status: ClaimStatus::Pending,
        created_at: now,
        resolved_at: None,
    };

    if let Err(e) = ctx.store.create_claim(&claim).await {
        return json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string();
    }

    if let Some(relationship_standing) =
        sensitive_claim_target_relationship_standing(&claimed_person, ctx)
    {
        let chosen_human_intent =
            create_chosen_human_identity_review_intent(&claim, chosen_human(ctx), ctx, reason)
                .await;
        info!(
            action = %ctx.action_id,
            claim = %claim.id,
            claimant = %claimant.0,
            claimed = %claimed_person.0,
            relationship_standing = %relationship_standing.as_str(),
            "identity verification claim recorded without contacting sensitive target"
        );
        return json!({
            "status": "chosen_human_confirmation_required",
            "claim": claim.id,
            "chosen_human_intent": chosen_human_intent,
            "contacted": 0,
            "failed": 0,
            "message": "Claim recorded, but contacting this person for verification requires chosen-human confirmation.",
        })
        .to_string();
    }

    if confidence < MIN_CONFIDENCE_TO_CONTACT {
        info!(
            action = %ctx.action_id,
            claim = %claim.id,
            claimant = %claimant.0,
            claimed = %claimed_person.0,
            confidence,
            "identity verification claim recorded without contact due low evidence confidence"
        );
        return json!({
            "status": "evidence_required",
            "claim": claim.id,
            "contacted": 0,
            "failed": 0,
            "message": "Claim recorded, but contacting another person for verification requires stronger evidence than a self-declaration.",
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

    let current_target = ctx.messages.first().and_then(|m| m.sender_key());
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

fn parse_allowed_claim_evidence(
    args: &Value,
    ctx: &SessionContext,
) -> Result<ClaimEvidence, &'static str> {
    let evidence = args["evidence"]
        .as_str()
        .and_then(ClaimEvidence::parse)
        .unwrap_or(ClaimEvidence::SelfDeclaration);
    if matches!(
        evidence,
        ClaimEvidence::ChosenHumanVouched | ClaimEvidence::ConfiguredIdentity
    ) && !matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman)
    {
        return Err(
            "chosen_human_vouched and configured_identity evidence require chosen-human relationship standing.",
        );
    }
    Ok(evidence)
}

fn recent_explicit_claim_message<'a>(
    ctx: &'a SessionContext,
    claimed_person: &Person,
) -> Option<&'a protocol::InboundMessage> {
    ctx.messages
        .iter()
        .rev()
        .find(|message| message_has_explicit_identity_claim(message, claimed_person))
}

fn message_has_explicit_identity_claim(
    message: &protocol::InboundMessage,
    person: &Person,
) -> bool {
    let text = normalize_claim_text(&message.content);
    if text.trim().is_empty() {
        return false;
    }

    let mentions_target = claim_target_labels(person)
        .iter()
        .any(|label| text.contains(label));
    let has_intro = [
        " i am ",
        " i'm ",
        " im ",
        " this is ",
        " my name is ",
        " i use ",
        " i also use ",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));
    let has_identity_context = [
        " account ",
        " profile ",
        " platform ",
        " handle ",
        " username ",
        " discord ",
        " whatsapp ",
        " signal ",
        " telegram ",
        " same person ",
        " other account ",
        " another account ",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));
    let has_me_backref = [
        " it's me ",
        " its me ",
        " that's me ",
        " thats me ",
        " that is me ",
    ]
    .iter()
    .any(|phrase| text.contains(phrase));

    (has_intro && (mentions_target || has_identity_context))
        || (has_me_backref && (mentions_target || has_identity_context))
        || (text.contains(" same person ") && (mentions_target || has_identity_context))
}

fn claim_target_labels(person: &Person) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(label) = normalized_label(&person.id.0) {
        labels.push(label);
    }
    if let Some(name) = &person.name {
        if let Some(label) = normalized_label(name) {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
    }
    labels
}

fn normalized_label(label: &str) -> Option<String> {
    let normalized = normalize_claim_text(label);
    let trimmed = normalized.trim();
    (trimmed.len() >= 3).then(|| format!(" {trimmed} "))
}

fn normalize_claim_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len() + 2);
    normalized.push(' ');
    let mut last_was_space = true;
    for ch in text.to_ascii_lowercase().chars() {
        let ch = if ch.is_ascii_alphanumeric() || ch == '\'' {
            ch
        } else {
            ' '
        };
        if ch == ' ' {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }
    if !normalized.ends_with(' ') {
        normalized.push(' ');
    }
    normalized
}

fn sensitive_claim_target_relationship_standing(
    claimed_person: &PersonId,
    ctx: &SessionContext,
) -> Option<RelationshipStanding> {
    if matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman) {
        return None;
    }
    let actor = ctx.state.read_state();
    let relationship_standing = actor
        .bonds
        .get(claimed_person)
        .map(|relationship| relationship.relationship_standing.clone())?;
    matches!(
        relationship_standing,
        RelationshipStanding::ChosenHuman
            | RelationshipStanding::Restricted
            | RelationshipStanding::Blocked
    )
    .then_some(relationship_standing)
}

fn chosen_human(ctx: &SessionContext) -> Option<PersonId> {
    let actor = ctx.state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, relationship)| {
            matches!(
                relationship.relationship_standing,
                RelationshipStanding::ChosenHuman
            )
        })
        .map(|(person, _)| person.clone())
}

async fn create_chosen_human_identity_review_intent(
    claim: &IdentityClaim,
    chosen_human: Option<PersonId>,
    ctx: &SessionContext,
    reason: &str,
) -> Option<String> {
    let chosen_human = chosen_human?;
    let now = super::super::util::now();
    let intent = IntentRecord {
        id: format!("intent-{}", super::super::util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review identity verification claim {} before anyone is contacted: {} claims to be {}. Claimed reason: {}",
            claim.id, claim.claimant.0, claim.claimed_person.0, reason
        ),
        person: Some(chosen_human),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!("identity-claim-chosen_human-review:{}", claim.id)),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: None,
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_human_approved: true,
    };
    let id = intent.id.clone();
    match ctx.store.create_intent(&intent).await {
        Ok(()) => Some(id),
        Err(e) => {
            warn!(
                action = %ctx.action_id,
                claim = %claim.id,
                %e,
                "failed to create chosen-human review intent for sensitive identity claim"
            );
            None
        }
    }
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
            Ok(()) => {
                let cleanup = remove_denied_claim_person_memory_subjects(ctx, &claim).await;
                let mut result = json!({
                    "status": "denied",
                    "claim": claim.id,
                });
                match cleanup {
                    Ok(count) => result["memories_demoted"] = json!(count),
                    Err(e) => result["memory_cleanup_error"] = json!(format!("{e}")),
                }
                result.to_string()
            }
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

    if let Err(e) = ctx.store.merge_person_context(&claimant, &keep).await {
        return json!({
            "status": "error",
            "message": format!("{e}"),
        })
        .to_string();
    }
    ctx.state.merge_person_context(&claimant, &keep).await;
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

async fn remove_denied_claim_person_memory_subjects(
    ctx: &SessionContext,
    claim: &IdentityClaim,
) -> anyhow::Result<usize> {
    let profiles = ctx.store.get_profiles_for_person(&claim.claimant).await?;
    let mut demoted = 0;
    for (profile, link) in profiles {
        if !link.status.is_active_person_context() {
            continue;
        }
        demoted += remove_detached_person_subject_from_profile_memories(
            ctx,
            &profile.id,
            &claim.claimed_person,
        )
        .await?;
    }
    Ok(demoted)
}

#[cfg(test)]
mod tests;
