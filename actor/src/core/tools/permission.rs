use super::SessionKind;
use super::context::SessionContext;
use crate::core::ActionKind;
use crate::identity::{PersonProfileLink, PersonProfileStatus, RelationSource, RelationStatus};
use crate::state::Authority;
use crate::store::{
    DEFAULT_MAX_SENSITIVITY, Memory, MemorySource, MemorySubjectType, PrivacyCategory,
    VisibilityScope,
};
use protocol::{ConversationId, MemoryId, PersonId, ProfileId};
use serde_json::Value;

pub(crate) const STRONG_LIKELY_PERSON_LINK_CONFIDENCE: f32 = 0.75;
const CHOSEN_PERSON_SOCIAL_PATH_MIN_CONFIDENCE: f32 = 0.5;
const CHOSEN_PERSON_SOCIAL_PATH_MAX_NODES: usize = 128;

pub async fn check(name: &str, args: &Value, ctx: &SessionContext) -> Result<(), String> {
    match name {
        "send_message"
            if matches!(
                ctx.kind,
                SessionKind::Action(
                    ActionKind::Review
                        | ActionKind::Research
                        | ActionKind::Consolidate
                        | ActionKind::Ruminate
                )
            ) =>
        {
            return Err(
                "This action is internal/background only; do not send a visible message.".into(),
            );
        }
        "send_message" => {
            let gateway_id = args["gateway_id"].as_str();
            let external_id = args["external_id"].as_str();
            if gateway_id.is_some() != external_id.is_some() {
                return Err(
                    "Explicit outbound sends must include both gateway_id and external_id.".into(),
                );
            }
            if let (Some(gateway_id), Some(external_id)) = (gateway_id, external_id) {
                let current_reply_target = ctx
                    .messages
                    .first()
                    .and_then(|msg| msg.reply_target())
                    .map(|(gateway, target)| (gateway.to_string(), target.to_string()));
                let is_current_reply_target =
                    current_reply_target
                        .as_ref()
                        .is_some_and(|(gateway, target)| {
                            gateway == gateway_id && target == external_id
                        });
                let has_outbound_authority =
                    matches!(ctx.authority, Authority::ChosenPerson | Authority::Trusted);
                let is_scheduled_outreach_target =
                    explicit_scheduled_outreach_target_matches(ctx, gateway_id, external_id)
                        .await?;
                if !is_current_reply_target
                    && !has_outbound_authority
                    && !is_scheduled_outreach_target
                {
                    return Err(
                        "Explicit outbound messaging requires chosen-person/trusted authority or the scheduled outreach target."
                            .into(),
                    );
                }
            }
        }
        "update_profile" => {
            if !privileged_profile_write(ctx) {
                let target = args["ref"].as_str().filter(|id| !id.is_empty());
                let current = ctx
                    .messages
                    .first()
                    .and_then(|message| message.profile.as_ref());
                if target.is_some_and(|target| current.is_none_or(|current| current.0 != target)) {
                    return Err(
                        "Updating another profile requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "update_person" => {
            if !privileged_profile_write(ctx) {
                let target = args["ref"].as_str().filter(|id| !id.is_empty());
                let current = ctx
                    .messages
                    .first()
                    .and_then(|message| message.person.as_ref());
                if target.is_some_and(|target| current.is_none_or(|current| current.0 != target)) {
                    return Err(
                        "Updating another person requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "update_conversation_summary" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                let target = args["conversation"].as_str().filter(|id| !id.is_empty());
                if target.is_some_and(|target| {
                    current_conversation(ctx).is_none_or(|current| current != target)
                }) {
                    return Err(
                        "Updating another conversation summary requires chosen-person authority or review/consolidation context."
                            .into(),
                    );
                }
            }
        }
        "upsert_social_relation" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                return Err(
                    "Social graph updates require chosen-person authority or review context."
                        .into(),
                );
            }
            if args["source_kind"].as_str() == Some("chosen_person_confirmed")
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err(
                    "Chosen-person-confirmed social relations require chosen-person authority."
                        .into(),
                );
            }
            if !social_relation_targets_current_or_verified(args, ctx).await? {
                return Err(
                    "Social graph updates from review must include the current strongly verified person, or chosen-person authority."
                        .into(),
                );
            }
        }
        "form_memory" => {
            if identity_memory_write_requested(args)
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err("Something feels wrong about this. You don't want to change something this core about yourself.".into());
            }
        }
        "recall_memories" => {
            if sensitive_recall_requested(args) && !privileged_sensitive_recall(ctx) {
                return Err(
                    "Sensitive memory recall requires chosen-person authority or internal review context."
                        .into(),
                );
            }
            if !privileged_memory_recall(ctx) && !memory_recall_targets_current(args, ctx) {
                return Err(
                    "Reading memories outside the current identity, profile, or person requires chosen-person authority or internal review context."
                        .into(),
                );
            }
        }
        "read_messages" => {
            if !privileged_conversation_read(ctx) {
                let target = args["conversation"].as_str().filter(|id| !id.is_empty());
                if target.is_none() && current_conversation(ctx).is_none() {
                    return Err(
                        "Reading recent conversations without a current conversation requires chosen-person authority or internal review/consolidation/rumination context."
                            .into(),
                    );
                }
                if target.is_some_and(|target| {
                    current_conversation(ctx).is_none_or(|current| current != target)
                }) {
                    return Err(
                        "Reading another conversation requires chosen-person authority or review/consolidation/rumination context."
                            .into(),
                    );
                }
            }
        }
        "inspect_memory" | "delete_memory" => {
            if !matches!(ctx.authority, Authority::ChosenPerson) {
                return Err(
                    "Chosen-person authority is required to inspect or delete memories by id."
                        .into(),
                );
            }
        }
        "forget_memory" => {
            let id = args["memory_id"].as_str().unwrap_or("");
            if let Ok(Some(mem)) = ctx.store.get_memory(&MemoryId(id.to_string())).await {
                if matches!(ctx.authority, Authority::ChosenPerson)
                    || matches!(
                        ctx.kind,
                        SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                    )
                {
                    return Ok(());
                }
                if matches!(mem.source, MemorySource::External) {
                    return Err(
                        "This memory feels fundamental — you instinctively hold onto it.".into(),
                    );
                }
                if !memory_is_current_profile_owned_for_forget(&mem, ctx) {
                    return Err(
                        "Forgetting memories outside the current profile requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "promote_profile_memory_to_person" => {
            if matches!(ctx.authority, Authority::ChosenPerson) {
                return Ok(());
            }
            if !matches!(
                ctx.kind,
                SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
            ) {
                return Err(
                    "Promoting profile memories to person-level memories requires chosen-person authority or internal review."
                        .into(),
                );
            }
            if !memory_promotion_target_is_verified(args, ctx).await? {
                return Err(
                    "Promoting a profile memory to a person requires a verified or strong likely link between a memory profile subject and that person."
                        .into(),
                );
            }
        }
        "demote_person_memory_to_profile" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && !matches!(
                    ctx.kind,
                    SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
                )
            {
                return Err(
                    "Demoting person-level memories requires chosen-person authority or internal review."
                        .into(),
                );
            }
        }
        "create_intent" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && privileged_intent_create(ctx)
                && !intent_targets_current_or_verified(args, ctx).await?
            {
                return Err(
                    "Third-party proactive outreach requires chosen-person authority or a verified target profile."
                        .into(),
                );
            }
            if !privileged_intent_create(ctx) && !explicit_intent_targets_current(args, ctx) {
                return Err(
                    "Creating intents for another person, profile, or conversation requires chosen-person authority or internal context."
                        .into(),
                );
            }
        }
        "update_intent" => {
            if !matches!(ctx.authority, Authority::ChosenPerson)
                && update_activates_pending_chosen_person_approval_intent(args, ctx).await?
            {
                return Err(
                    "Activating an chosen-person-approval intent requires chosen-person authority."
                        .into(),
                );
            }
            if intent_requires_chosen_person_approval(args)
                && !matches!(ctx.authority, Authority::ChosenPerson)
            {
                return Err(
                    "Sensitive proactive outreach requires chosen-person approval before an intent is updated."
                        .into(),
                );
            }
            if !matches!(ctx.authority, Authority::ChosenPerson) && privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current_or_verified(id, ctx).await?
                    || !intent_targets_current_or_verified(args, ctx).await?
                {
                    return Err(
                        "Third-party proactive outreach requires chosen-person authority or a verified target profile."
                            .into(),
                    );
                }
            }
            if !privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current(id, ctx).await? {
                    return Err(
                        "Updating intents for another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
                if !explicit_intent_targets_current(args, ctx) {
                    return Err(
                        "Retargeting intents to another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "delete_intent" => {
            if !privileged_intent_write(ctx) {
                let id = args["intent_id"].as_str().unwrap_or("");
                if !intent_id_targets_current(id, ctx).await? {
                    return Err(
                        "Cancelling intents for another person, profile, or conversation requires chosen-person authority or review context."
                            .into(),
                    );
                }
            }
        }
        "apply_review" => {
            if !matches!(ctx.kind, SessionKind::Action(ActionKind::Review)) {
                return Err(
                    "Structured review application is allowed only in review actions.".into(),
                );
            }
        }
        "reflect" => {
            if let Some(rels) = args["relationship_changes"].as_array() {
                for r in rels {
                    if r.get("authority").is_some()
                        && !matches!(ctx.authority, Authority::ChosenPerson)
                    {
                        return Err(
                            "Changing how you feel about someone isn't something you'd do on command."
                                .into(),
                        );
                    }
                    if !privileged_profile_write(ctx) {
                        let target = r["person"].as_str().filter(|id| !id.is_empty());
                        if target.is_some_and(|target| {
                            current_person(ctx).is_none_or(|current| current != target)
                        }) {
                            return Err(
                                "Reflecting on relationship changes for another person requires chosen-person authority or review context."
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn privileged_profile_write(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

fn privileged_sensitive_recall(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

fn privileged_memory_recall(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

fn privileged_conversation_read(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(
                ActionKind::Review | ActionKind::Consolidate | ActionKind::Ruminate
            )
        )
}

fn privileged_intent_write(ctx: &SessionContext) -> bool {
    matches!(ctx.authority, Authority::ChosenPerson)
        || matches!(
            ctx.kind,
            SessionKind::Action(ActionKind::Review | ActionKind::Consolidate)
        )
}

fn privileged_intent_create(ctx: &SessionContext) -> bool {
    privileged_intent_write(ctx) || matches!(ctx.kind, SessionKind::Action(ActionKind::Ruminate))
}

pub(crate) fn intent_requires_chosen_person_approval(args: &Value) -> bool {
    args["requires_chosen_person_approval"]
        .as_bool()
        .unwrap_or(false)
        || args["sensitive"].as_bool().unwrap_or(false)
        || sensitive_outreach_text(args["task"].as_str())
        || sensitive_outreach_text(args["condition"].as_str())
}

async fn update_activates_pending_chosen_person_approval_intent(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if args["status"].as_str() != Some("active") {
        return Ok(false);
    }
    let Some(id) = args["intent_id"]
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(false);
    };
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent chosen-person approval status: {e}"))?;
    Ok(intent.is_some_and(|intent| intent.status == "pending_approval"))
}

pub(crate) async fn intent_targets_current_or_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    intent_targets_current_or_verified_with_keys(args, ctx, "person", "profile", "conversation")
        .await
}

pub(crate) async fn intent_targets_current_or_verified_with_keys(
    args: &Value,
    ctx: &SessionContext,
    person_key: &str,
    profile_key: &str,
    conversation_key: &str,
) -> Result<bool, String> {
    let person = args[person_key].as_str().filter(|id| !id.is_empty());
    let profile = args[profile_key].as_str().filter(|id| !id.is_empty());
    let conversation = args[conversation_key].as_str().filter(|id| !id.is_empty());
    if person.is_none() && profile.is_none() && conversation.is_none() {
        return Ok(true);
    }

    if let Some(person) = person {
        if current_person(ctx) != Some(person)
            && !person_has_active_profile_context(ctx, &PersonId(person.to_string())).await?
        {
            return Ok(false);
        }
    }
    if let Some(profile) = profile {
        if current_profile(ctx) != Some(profile)
            && !profile_has_active_person_context(ctx, &ProfileId(profile.to_string())).await?
        {
            return Ok(false);
        }
    }
    if let Some(conversation) = conversation {
        if current_conversation(ctx) != Some(conversation)
            && !conversation_has_active_person_context(
                ctx,
                &ConversationId(conversation.to_string()),
            )
            .await?
        {
            return Ok(false);
        }
    }

    Ok(true)
}

async fn intent_id_targets_current_or_verified(
    id: &str,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent target: {e}"))?;
    let Some(intent) = intent else {
        return Ok(true);
    };
    let has_target =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !has_target {
        return Ok(false);
    }
    if let Some(person) = intent.person {
        if current_person(ctx) != Some(person.0.as_str())
            && !person_has_active_profile_context(ctx, &person).await?
        {
            return Ok(false);
        }
    }
    if let Some(profile) = intent.profile {
        if current_profile(ctx) != Some(profile.0.as_str())
            && !profile_has_active_person_context(ctx, &profile).await?
        {
            return Ok(false);
        }
    }
    if let Some(conversation) = intent.conversation {
        if current_conversation(ctx) != Some(conversation.0.as_str())
            && !conversation_has_active_person_context(ctx, &conversation).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn person_has_active_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> Result<bool, String> {
    let profiles = ctx
        .store
        .get_profiles_for_person(person)
        .await
        .map_err(|e| format!("Could not verify third-party person target: {e}"))?;
    Ok(profiles
        .into_iter()
        .any(|(_, link)| link.status.is_active_person_context()))
}

pub(crate) async fn person_has_verified_or_strong_profile_context(
    ctx: &SessionContext,
    person: &PersonId,
) -> Result<bool, String> {
    let profiles = ctx
        .store
        .get_profiles_for_person(person)
        .await
        .map_err(|e| format!("Could not verify person profile context: {e}"))?;
    Ok(profiles
        .into_iter()
        .any(|(_, link)| person_link_allows_person_level_update(&link)))
}

fn person_link_allows_person_level_update(link: &PersonProfileLink) -> bool {
    match link.status {
        PersonProfileStatus::Verified => true,
        PersonProfileStatus::Likely => link.confidence >= STRONG_LIKELY_PERSON_LINK_CONFIDENCE,
        _ => false,
    }
}

pub(crate) async fn social_relation_targets_current_or_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    if matches!(ctx.authority, Authority::ChosenPerson) {
        return Ok(true);
    }

    let Some(person_a) = args["person_a"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return Ok(false);
    };
    let Some(person_b) = args["person_b"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .map(|id| PersonId(id.to_string()))
    else {
        return Ok(false);
    };
    if social_relation_mentions_chosen_person(ctx, &person_a, &person_b) {
        return Ok(false);
    }

    let Some(current) = ctx
        .messages
        .first()
        .and_then(|message| message.person.clone())
    else {
        return Ok(false);
    };
    if current != person_a && current != person_b {
        return Ok(false);
    }
    person_has_verified_or_strong_profile_context(ctx, &current).await
}

pub(crate) async fn relationship_trust_ceiling(ctx: &SessionContext, person: &PersonId) -> f32 {
    let (authority, current_trust, chosen_people) = {
        let actor = ctx.state.read_state();
        let relationship = actor.bonds.get(person);
        let authority = relationship
            .map(|relationship| relationship.authority.clone())
            .unwrap_or(Authority::Default);
        let current_trust = relationship
            .map(|relationship| relationship.trust)
            .unwrap_or_else(|| crate::state::Relationship::default().trust);
        let chosen_people = actor
            .bonds
            .iter()
            .filter_map(|(chosen_person, relationship)| {
                matches!(relationship.authority, Authority::ChosenPerson)
                    .then(|| chosen_person.clone())
            })
            .collect::<Vec<_>>();
        (authority, current_trust, chosen_people)
    };

    if !matches!(authority, Authority::Default) {
        return authority.trust_ceiling();
    }
    if chosen_people
        .iter()
        .any(|chosen_person| chosen_person == person)
    {
        return Authority::ChosenPerson.trust_ceiling();
    }
    if has_chosen_person_social_path(ctx, person, &chosen_people).await {
        Authority::Default.trust_ceiling()
    } else {
        current_trust.clamp(0.0, Authority::Default.trust_ceiling())
    }
}

async fn has_chosen_person_social_path(
    ctx: &SessionContext,
    person: &PersonId,
    chosen_people: &[PersonId],
) -> bool {
    let chosen_person_set = chosen_people
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let mut seen = std::collections::HashSet::from([person.clone()]);
    let mut frontier = vec![person.clone()];

    while !frontier.is_empty() {
        let mut next = Vec::new();
        for current in frontier {
            let Ok(relations) = ctx.store.get_relations(&current).await else {
                return false;
            };
            for relation in relations {
                if !relation_allows_trust_path(&relation) {
                    continue;
                }
                let Some(other) = other_relation_person(&relation, &current) else {
                    continue;
                };
                if chosen_person_set.contains(&other) {
                    return true;
                }
                if seen.insert(other.clone()) {
                    if seen.len() >= CHOSEN_PERSON_SOCIAL_PATH_MAX_NODES {
                        return false;
                    }
                    next.push(other);
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }

    false
}

fn relation_allows_trust_path(relation: &crate::identity::SocialRelation) -> bool {
    matches!(
        relation.status,
        RelationStatus::Confirmed | RelationStatus::Stated
    ) && !matches!(relation.source_kind, RelationSource::Inferred)
        && relation.confidence >= CHOSEN_PERSON_SOCIAL_PATH_MIN_CONFIDENCE
}

fn other_relation_person(
    relation: &crate::identity::SocialRelation,
    person: &PersonId,
) -> Option<PersonId> {
    if &relation.person_a == person {
        Some(relation.person_b.clone())
    } else if &relation.person_b == person {
        Some(relation.person_a.clone())
    } else {
        None
    }
}

fn social_relation_mentions_chosen_person(
    ctx: &SessionContext,
    person_a: &PersonId,
    person_b: &PersonId,
) -> bool {
    let actor = ctx.state.read_state();
    actor.bonds.iter().any(|(person, relationship)| {
        matches!(relationship.authority, Authority::ChosenPerson)
            && (person == person_a || person == person_b)
    })
}

async fn profile_has_active_person_context(
    ctx: &SessionContext,
    profile: &ProfileId,
) -> Result<bool, String> {
    let link = ctx
        .store
        .get_person_for_profile(profile)
        .await
        .map_err(|e| format!("Could not verify third-party profile target: {e}"))?;
    Ok(link.is_some_and(|(_, link)| link.status.is_active_person_context()))
}

async fn conversation_has_active_person_context(
    ctx: &SessionContext,
    conversation: &ConversationId,
) -> Result<bool, String> {
    let conversations = ctx
        .store
        .list_conversations()
        .await
        .map_err(|e| format!("Could not verify third-party conversation target: {e}"))?;
    let Some(summary) = conversations
        .into_iter()
        .find(|summary| summary.id == *conversation)
    else {
        return Ok(false);
    };
    if let Some(person) = summary.person {
        return person_has_active_profile_context(ctx, &person).await;
    }
    if let Some(profile) = summary.profile {
        return profile_has_active_person_context(ctx, &profile).await;
    }
    Ok(false)
}

fn sensitive_outreach_text(text: Option<&str>) -> bool {
    let Some(text) = text else {
        return false;
    };
    let text = text.to_ascii_lowercase();
    [
        "password",
        "passcode",
        "token",
        "secret",
        "confidential",
        "private",
        "medical",
        "health",
        "diagnosis",
        "therapy",
        "legal",
        "lawyer",
        "financial",
        "finance",
        "bank",
        "tax",
        "payment",
        "address",
        "social security",
        "ssn",
        "identity",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn explicit_intent_targets_current(args: &Value, ctx: &SessionContext) -> bool {
    explicit_target_matches(args, "person", current_person(ctx))
        && explicit_target_matches(args, "profile", current_profile(ctx))
        && explicit_target_matches(args, "conversation", current_conversation(ctx))
}

fn explicit_target_matches(args: &Value, key: &str, current: Option<&str>) -> bool {
    let Some(target) = args[key].as_str().filter(|id| !id.is_empty()) else {
        return true;
    };
    current.is_some_and(|current| current == target)
}

async fn intent_id_targets_current(id: &str, ctx: &SessionContext) -> Result<bool, String> {
    if id.is_empty() {
        return Ok(false);
    }
    let intent = ctx
        .store
        .get_intent(id)
        .await
        .map_err(|e| format!("Could not verify intent target: {e}"))?;
    let Some(intent) = intent else {
        return Ok(true);
    };

    let has_target =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !has_target {
        return Ok(false);
    }

    if intent
        .person
        .as_ref()
        .is_some_and(|target| current_person(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }
    if intent
        .profile
        .as_ref()
        .is_some_and(|target| current_profile(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }
    if intent
        .conversation
        .as_ref()
        .is_some_and(|target| current_conversation(ctx).is_none_or(|current| current != target.0))
    {
        return Ok(false);
    }

    Ok(true)
}

fn current_person(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.person.as_ref())
        .map(|id| id.0.as_str())
}

fn current_identity(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.identity.as_ref())
        .map(|id| id.0.as_str())
}

fn current_profile(ctx: &SessionContext) -> Option<&str> {
    ctx.messages
        .first()
        .and_then(|message| message.profile.as_ref())
        .map(|id| id.0.as_str())
}

fn current_conversation(ctx: &SessionContext) -> Option<&str> {
    ctx.conversation
        .as_ref()
        .or_else(|| ctx.messages.first().map(|message| &message.conversation))
        .map(|id| id.0.as_str())
}

fn current_conversation_id(ctx: &SessionContext) -> Option<ConversationId> {
    ctx.conversation.clone().or_else(|| {
        ctx.messages
            .first()
            .map(|message| message.conversation.clone())
    })
}

async fn explicit_scheduled_outreach_target_matches(
    ctx: &SessionContext,
    gateway_id: &str,
    external_id: &str,
) -> Result<bool, String> {
    if !matches!(ctx.kind, SessionKind::Action(ActionKind::Outreach)) {
        return Ok(false);
    }
    let Some(conversation) = current_conversation_id(ctx) else {
        return Ok(false);
    };

    let conversation_gateway = ctx
        .store
        .list_conversations()
        .await
        .map_err(|e| format!("Could not verify scheduled outreach target: {e}"))?
        .into_iter()
        .find(|summary| summary.id == conversation)
        .and_then(|summary| summary.gateway_id);
    let messages = ctx
        .store
        .get_messages(&conversation, 20, None)
        .await
        .map_err(|e| format!("Could not verify scheduled outreach target: {e}"))?;

    Ok(messages.iter().rev().any(|message| {
        let Some(reply_external_id) = message.reply_external_id.as_deref() else {
            return false;
        };
        let Some(reply_gateway_id) = message
            .source_gateway_id
            .as_deref()
            .or(conversation_gateway.as_deref())
        else {
            return false;
        };
        reply_gateway_id == gateway_id && reply_external_id == external_id
    }))
}

fn sensitive_recall_requested(args: &Value) -> bool {
    args["include_sensitive"].as_bool().unwrap_or(false)
        || args["max_sensitivity"]
            .as_f64()
            .is_some_and(|max| max as f32 > DEFAULT_MAX_SENSITIVITY)
}

fn memory_recall_targets_current(args: &Value, ctx: &SessionContext) -> bool {
    if memory_global_recall_requested(args) {
        return false;
    }

    let identity = args["identity"].as_str().filter(|id| !id.is_empty());
    let profile = args["profile"].as_str().filter(|id| !id.is_empty());
    let person = args["person"].as_str().filter(|id| !id.is_empty());
    if identity.is_none() && profile.is_none() && person.is_none() {
        return current_identity(ctx).is_some()
            || current_profile(ctx).is_some()
            || current_person(ctx).is_some();
    }

    identity.is_none_or(|target| current_identity(ctx) == Some(target))
        && profile.is_none_or(|target| current_profile(ctx) == Some(target))
        && person.is_none_or(|target| current_person(ctx) == Some(target))
}

fn memory_global_recall_requested(args: &Value) -> bool {
    matches!(args["scope"].as_str(), Some("global")) || args["global"].as_bool().unwrap_or(false)
}

fn identity_memory_write_requested(args: &Value) -> bool {
    let has_identity_tag = args["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .any(is_identity_memory_marker);
    let identity_type = args["memory_type"]
        .as_str()
        .is_some_and(|value| value == "identity_claim");
    let identity_sensitivity = args["sensitivity_category"]
        .as_str()
        .is_some_and(is_identity_memory_marker);
    let actor_subject = args["subject_actor"].as_bool().unwrap_or(false);

    has_identity_tag || identity_type || identity_sensitivity || actor_subject
}

fn is_identity_memory_marker(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "identity" | "identity_claim" | "self" | "name"
    )
}

fn memory_is_current_profile_owned_for_forget(mem: &Memory, ctx: &SessionContext) -> bool {
    let current = ctx.messages.first();
    let Some(current_profile) = current.and_then(|msg| msg.profile.as_ref()) else {
        return false;
    };

    if matches!(
        mem.privacy_category,
        PrivacyCategory::Sensitive | PrivacyCategory::Secret
    ) || matches!(
        mem.visibility_scope,
        VisibilityScope::Person | VisibilityScope::ChosenPersonOnly | VisibilityScope::Global
    ) {
        return false;
    }

    for subject in &mem.subjects {
        match subject.subject_type {
            MemorySubjectType::Profile if subject.subject_id == current_profile.0 => {}
            MemorySubjectType::Profile => return false,
            MemorySubjectType::Identity | MemorySubjectType::Person | MemorySubjectType::Actor => {
                return false;
            }
        }
    }

    if mem.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Profile
            && subject.subject_id == current_profile.0
    }) {
        return true;
    }

    match &mem.source {
        MemorySource::Conversation { profile_id, .. } => profile_id
            .as_ref()
            .is_some_and(|profile_id| profile_id == current_profile),
        _ => false,
    }
}

async fn memory_promotion_target_is_verified(
    args: &Value,
    ctx: &SessionContext,
) -> Result<bool, String> {
    let Some(memory_id) = args["memory_id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    let Some(person_id) = args["person"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    let target = PersonId(person_id.to_string());
    let memory = ctx
        .store
        .get_memory(&MemoryId(memory_id.to_string()))
        .await
        .map_err(|e| format!("Could not verify memory promotion target: {e}"))?;
    let Some(memory) = memory else {
        return Ok(false);
    };

    if memory.subjects.iter().any(|subject| {
        subject.subject_type == MemorySubjectType::Person && subject.subject_id == target.0
    }) {
        return Ok(true);
    }

    for subject in memory
        .subjects
        .iter()
        .filter(|subject| subject.subject_type == MemorySubjectType::Profile)
    {
        let profile = ProfileId(subject.subject_id.clone());
        let link = ctx
            .store
            .get_person_for_profile(&profile)
            .await
            .map_err(|e| format!("Could not verify memory promotion profile link: {e}"))?;
        if let Some((_person, link)) = link {
            if link.person_id == target && person_link_allows_person_level_update(&link) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::{ActionId, RunningState};
    use crate::core::handle::{SharedState, StateHandle};
    use crate::identity::{
        Person, PersonProfileStatus, Profile, Relation, RelationSource, RelationStatus,
        SocialRelation,
    };
    use crate::state::{ActorState, GrowthConfig};
    use crate::store::{
        IntentRecord, Memory, MemoryKind, MemorySource, MemorySubject, MessageRole,
        PrivacyCategory, SqliteStore, StoredMessage, VisibilityScope,
    };
    use gateway::GatewayRouter;
    use inference::{
        Capability, InferenceEndpoint, InferenceProtocol, InferenceRouterBuilder,
        OpenAiCompatibleBridge, Reasoning, SamplingConfig,
    };
    use protocol::{ConversationId, IdentityId, InboundMessage, PersonId, ProfileId};
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc;

    struct NoopBridge;

    #[async_trait::async_trait]
    impl OpenAiCompatibleBridge for NoopBridge {
        async fn chat(
            &self,
            _request: &inference::ChatRequest,
        ) -> anyhow::Result<inference::ChatResponse> {
            anyhow::bail!("noop bridge is not used by permission tests")
        }

        async fn chat_stream(
            &self,
            _request: &inference::ChatRequest,
        ) -> anyhow::Result<inference::ChatStream> {
            anyhow::bail!("noop bridge is not used by permission tests")
        }
    }

    fn test_context(authority: Authority, kind: ActionKind) -> SessionContext {
        let (_inject_tx, inject_rx) = mpsc::channel(1);
        let (delta_tx, _delta_rx) = mpsc::channel(1);
        let shared = Arc::new(SharedState {
            actor: RwLock::new(ActorState::new(Default::default())),
            config: RwLock::new(GrowthConfig::default()),
        });
        let router = InferenceRouterBuilder::new()
            .endpoint(InferenceEndpoint {
                protocol: InferenceProtocol::OpenAiCompatible(Arc::new(NoopBridge)),
                model: "noop".into(),
                sampling: SamplingConfig::default(),
                capabilities: vec![Capability::Chat],
                reasoning: Reasoning::Basic,
            })
            .build()
            .unwrap();
        let message = InboundMessage {
            message_id: "msg-1".into(),
            gateway_id: "relay".into(),
            sender_external_id: "local".into(),
            sender_display_name: None,
            reply_external_id: "local".into(),
            conversation: ConversationId("relay:local".into()),
            group: None,
            identity: None,
            profile: None,
            person: None,
            content: "hello".into(),
            attachments: vec![],
            timestamp: 1000,
            metadata: serde_json::Value::Null,
        };

        SessionContext {
            action_id: ActionId("permission-test".into()),
            kind: SessionKind::Action(kind),
            messages: vec![message],
            conversation: Some(ConversationId("relay:local".into())),
            authority,
            style_directive: None,
            cancelled_note: None,
            concurrent_summaries: vec![],
            state: StateHandle::new(shared, delta_tx),
            store: Arc::new(SqliteStore::open_in_memory(4).unwrap()),
            media_store: None,
            router: Arc::new(router),
            endpoints: vec![],
            reasoning: Reasoning::Basic,
            inject_rx,
            progress: Arc::new(RwLock::new(RunningState::new())),
            max_turns: 1,
            max_action_attempts: 1,
            escalate_after: 1,
            gateway: Arc::new(GatewayRouter::new()),
            typing: Arc::new(RwLock::new(Default::default())),
            metrics: Arc::new(crate::core::ActorMetrics::default()),
            session_start: std::time::Instant::now(),
        }
    }

    async fn add_verified_target(ctx: &SessionContext, profile: &ProfileId, person: &PersonId) {
        ctx.store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Verified target".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        ctx.store
            .add_person(&Person {
                id: person.clone(),
                name: Some("Verified Person".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
            })
            .await
            .unwrap();
        ctx.store
            .attach_profile_to_person(profile, person, PersonProfileStatus::Verified, 1.0, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn relationship_trust_ceiling_requires_chosen_person_social_path_for_default_people() {
        let ctx = test_context(Authority::Default, ActionKind::Review);
        let chosen_person = PersonId("person-chosen_person".into());
        let stranger = PersonId("person-stranger".into());
        {
            let mut actor = ctx.state.shared.actor.write().unwrap();
            actor.set_relationship_config(&chosen_person, Some(Authority::ChosenPerson));
        }

        let ceiling = relationship_trust_ceiling(&ctx, &stranger).await;

        assert_eq!(ceiling, crate::state::Relationship::default().trust);
    }

    #[tokio::test]
    async fn relationship_trust_ceiling_allows_chosen_person_connected_social_path() {
        let ctx = test_context(Authority::Default, ActionKind::Review);
        let chosen_person = PersonId("person-chosen_person".into());
        let middle = PersonId("person-middle".into());
        let connected = PersonId("person-connected".into());
        {
            let mut actor = ctx.state.shared.actor.write().unwrap();
            actor.set_relationship_config(&chosen_person, Some(Authority::ChosenPerson));
        }
        ctx.store
            .upsert_relation(&SocialRelation {
                person_a: chosen_person.clone(),
                person_b: middle.clone(),
                relation: Relation::Friend,
                direction: Relation::Friend.default_direction(),
                confidence: 0.9,
                status: RelationStatus::Confirmed,
                evidence: Some(serde_json::json!({"source": "test"})),
                source_kind: RelationSource::ChosenPersonConfirmed,
                asserted_by: Some(chosen_person.clone()),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        ctx.store
            .upsert_relation(&SocialRelation {
                person_a: middle.clone(),
                person_b: connected.clone(),
                relation: Relation::Coworker,
                direction: Relation::Coworker.default_direction(),
                confidence: 0.8,
                status: RelationStatus::Stated,
                evidence: Some(serde_json::json!({"source": "test"})),
                source_kind: RelationSource::Stated,
                asserted_by: Some(middle.clone()),
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();

        let ceiling = relationship_trust_ceiling(&ctx, &connected).await;

        assert_eq!(ceiling, Authority::Default.trust_ceiling());
    }

    #[tokio::test]
    async fn default_user_cannot_send_explicit_outbound_to_other_target() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "discord",
                "external_id": "channel-2"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("Explicit outbound messaging requires"));
    }

    #[tokio::test]
    async fn default_user_cannot_update_other_profile_or_person() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        let profile_denied = check(
            "update_profile",
            &serde_json::json!({
                "ref": "profile-other",
                "summary": "Cross-profile summary"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(profile_denied.contains("another profile"));

        let person_denied = check(
            "update_person",
            &serde_json::json!({
                "ref": "person-other",
                "summary": "Cross-person summary"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(person_denied.contains("another person"));
    }

    #[tokio::test]
    async fn current_profile_and_person_updates_are_allowed() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        check(
            "update_profile",
            &serde_json::json!({
                "ref": "profile-current",
                "summary": "Current profile summary"
            }),
            &ctx,
        )
        .await
        .unwrap();

        check(
            "update_person",
            &serde_json::json!({
                "ref": "person-current",
                "summary": "Current person summary"
            }),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn live_reflection_relationship_changes_are_current_person_only() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        check(
            "reflect",
            &serde_json::json!({
                "relationship_changes": [{
                    "person": "person-current",
                    "trust_delta": 0.01,
                    "familiarity_delta": 0.02,
                    "valence_delta": 0.01
                }]
            }),
            &ctx,
        )
        .await
        .unwrap();

        let denied = check(
            "reflect",
            &serde_json::json!({
                "relationship_changes": [{
                    "person": "person-other",
                    "trust_delta": 0.01,
                    "familiarity_delta": 0.02,
                    "valence_delta": 0.01
                }]
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("another person"));

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "reflect",
            &serde_json::json!({
                "relationship_changes": [{
                    "person": "person-other",
                    "trust_delta": 0.01
                }]
            }),
            &chosen_person,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "reflect",
            &serde_json::json!({
                "relationship_changes": [{
                    "person": "person-other",
                    "trust_delta": 0.01
                }]
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn chosen_person_or_review_can_update_other_profile_and_person() {
        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "update_profile",
            &serde_json::json!({
                "ref": "profile-other",
                "summary": "Chosen-person-visible profile summary"
            }),
            &chosen_person,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "update_person",
            &serde_json::json!({
                "ref": "person-other",
                "summary": "Review-supported person summary"
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_write_structured_identity_memory() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "form_memory",
            &serde_json::json!({
                "content": "I am a different core identity now.",
                "memory_type": "identity_claim"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("core about yourself"));

        let denied = check(
            "form_memory",
            &serde_json::json!({
                "content": "My private identity marker is different.",
                "sensitivity_category": "identity"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("core about yourself"));

        let denied = check(
            "form_memory",
            &serde_json::json!({
                "content": "My name is Pamagotchi.",
                "kind": "semantic",
                "subject_actor": true
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("core about yourself"));
    }

    #[tokio::test]
    async fn chosen_person_can_write_structured_identity_memory() {
        let ctx = test_context(Authority::ChosenPerson, ActionKind::Respond);

        check(
            "form_memory",
            &serde_json::json!({
                "content": "My name is Pamagotchi.",
                "memory_type": "identity_claim",
                "sensitivity_category": "identity"
            }),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_update_social_graph() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "upsert_social_relation",
            &serde_json::json!({
                "person_a": "person-a",
                "person_b": "person-b",
                "relation": "friend"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("Social graph updates require"));
    }

    #[tokio::test]
    async fn review_can_update_social_graph_but_not_chosen_person_confirm() {
        let mut ctx = test_context(Authority::Default, ActionKind::Review);
        let current_profile = ProfileId("profile-a".into());
        let current_person = PersonId("person-a".into());
        add_verified_target(&ctx, &current_profile, &current_person).await;
        ctx.messages[0].profile = Some(current_profile);
        ctx.messages[0].person = Some(current_person);

        check(
            "upsert_social_relation",
            &serde_json::json!({
                "person_a": "person-a",
                "person_b": "person-b",
                "relation": "friend",
                "source_kind": "stated"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let denied_third_party = check(
            "upsert_social_relation",
            &serde_json::json!({
                "person_a": "person-b",
                "person_b": "person-c",
                "relation": "friend",
                "source_kind": "stated"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied_third_party.contains("current strongly verified person"));

        let denied = check(
            "upsert_social_relation",
            &serde_json::json!({
                "person_a": "person-a",
                "person_b": "person-b",
                "relation": "friend",
                "source_kind": "chosen_person_confirmed"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("Chosen-person-confirmed"));
    }

    #[tokio::test]
    async fn default_user_cannot_opt_into_sensitive_memory_recall() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "deployment credentials",
                "include_sensitive": true
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Sensitive memory recall requires"));

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "private detail",
                "max_sensitivity": 0.95
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Sensitive memory recall requires"));
    }

    #[tokio::test]
    async fn chosen_person_or_review_can_opt_into_sensitive_memory_recall() {
        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "deployment credentials",
                "include_sensitive": true
            }),
            &chosen_person,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "private detail",
                "max_sensitivity": 0.95
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_memory_recall_is_current_target_only() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].identity = Some(IdentityId("identity-current".into()));
        ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        check(
            "recall_memories",
            &serde_json::json!({"query": "current context"}),
            &ctx,
        )
        .await
        .unwrap();
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "current person preference",
                "person": "person-current"
            }),
            &ctx,
        )
        .await
        .unwrap();
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "current profile preference",
                "profile": "profile-current"
            }),
            &ctx,
        )
        .await
        .unwrap();
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "current identity preference",
                "identity": "identity-current"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "other person's preferences",
                "person": "person-other"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("outside the current identity"));

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "other profile preferences",
                "profile": "profile-other"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("outside the current identity"));

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "other identity preferences",
                "identity": "identity-other"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("outside the current identity"));

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "anything",
                "scope": "global"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("outside the current identity"));
    }

    #[tokio::test]
    async fn chosen_person_or_review_can_recall_outside_current_target() {
        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "other person preference",
                "person": "person-other"
            }),
            &chosen_person,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "recall_memories",
            &serde_json::json!({
                "query": "cross-profile duplicate",
                "scope": "global"
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ruminate_can_recall_without_current_target_but_not_sensitive() {
        let ctx = test_context(Authority::Default, ActionKind::Ruminate);

        check(
            "recall_memories",
            &serde_json::json!({"query": "idle thought"}),
            &ctx,
        )
        .await
        .unwrap();

        let denied = check(
            "recall_memories",
            &serde_json::json!({
                "query": "private idle thought",
                "include_sensitive": true
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Sensitive memory recall requires"));
    }

    #[tokio::test]
    async fn default_user_can_create_current_target_intent() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].profile = Some(ProfileId("profile-current".into()));
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        check(
            "create_intent",
            &serde_json::json!({
                "task": "Follow up here later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-current",
                "profile": "profile-current",
                "conversation": "relay:local"
            }),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_create_cross_target_intent() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].person = Some(PersonId("person-current".into()));

        let denied = check(
            "create_intent",
            &serde_json::json!({
                "task": "Message Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("another person"));
    }

    #[tokio::test]
    async fn chosen_person_can_create_cross_target_intent_and_review_requires_verified_target() {
        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "create_intent",
            &serde_json::json!({
                "task": "Message Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice"
            }),
            &chosen_person,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        let denied = check(
            "create_intent",
            &serde_json::json!({
                "task": "Message Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice"
            }),
            &review,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Third-party proactive outreach"));

        add_verified_target(
            &review,
            &ProfileId("profile-alice".into()),
            &PersonId("person-alice".into()),
        )
        .await;
        check(
            "create_intent",
            &serde_json::json!({
                "task": "Message Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice",
                "profile": "profile-alice"
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ruminate_can_create_verified_target_intent_but_not_unverified_target() {
        let mut ruminate = test_context(Authority::Default, ActionKind::Ruminate);
        ruminate.messages.clear();
        ruminate.conversation = None;

        let denied = check(
            "create_intent",
            &serde_json::json!({
                "task": "Check in with Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice"
            }),
            &ruminate,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("verified target profile"));

        add_verified_target(
            &ruminate,
            &ProfileId("profile-alice".into()),
            &PersonId("person-alice".into()),
        )
        .await;
        check(
            "create_intent",
            &serde_json::json!({
                "task": "Check in with Alice later",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-alice",
                "profile": "profile-alice"
            }),
            &ruminate,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ruminate_cannot_update_or_cancel_cross_target_intent() {
        let ruminate = test_context(Authority::Default, ActionKind::Ruminate);
        add_verified_target(
            &ruminate,
            &ProfileId("profile-alice".into()),
            &PersonId("person-alice".into()),
        )
        .await;
        ruminate
            .store
            .create_intent(&IntentRecord {
                id: "intent-alice".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Follow up with Alice".into(),
                person: Some(PersonId("person-alice".into())),
                profile: Some(ProfileId("profile-alice".into())),
                conversation: None,
                fire_at: Some(1200),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 1000,
                updated_at: 1000,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let denied = check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-alice",
                "task": "Change Alice follow-up"
            }),
            &ruminate,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Updating intents"));

        let denied = check(
            "delete_intent",
            &serde_json::json!({
                "intent_id": "intent-alice"
            }),
            &ruminate,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Cancelling intents"));
    }

    #[tokio::test]
    async fn sensitive_proactive_intent_creates_can_be_routed_for_chosen_person_approval() {
        let mut current = test_context(Authority::Default, ActionKind::Respond);
        current.messages[0].person = Some(PersonId("person-current".into()));

        check(
            "create_intent",
            &serde_json::json!({
                "task": "Ask Sam about the private medical update",
                "kind": "scheduled",
                "fire_at": 1200,
                "person": "person-current"
            }),
            &current,
        )
        .await
        .unwrap();

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "create_intent",
            &serde_json::json!({
                "task": "Follow up about the confidential family issue",
                "kind": "scheduled",
                "fire_at": 1200,
                "requires_chosen_person_approval": true
            }),
            &review,
        )
        .await
        .unwrap();

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "create_intent",
            &serde_json::json!({
                "task": "Follow up about the private medical update",
                "kind": "scheduled",
                "fire_at": 1200,
                "sensitive": true
            }),
            &chosen_person,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn sensitive_intent_updates_require_chosen_person_authority() {
        let review = test_context(Authority::Default, ActionKind::Review);
        let denied = check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-1",
                "task": "Follow up about a bank payment",
                "sensitive": true
            }),
            &review,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Sensitive proactive outreach"));

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-1",
                "task": "Follow up about a bank payment",
                "requires_chosen_person_approval": true
            }),
            &chosen_person,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn pending_chosen_person_approval_intents_can_only_be_activated_by_chosen_person() {
        let review = test_context(Authority::Default, ActionKind::Review);
        review
            .store
            .create_intent(&IntentRecord {
                id: "intent-pending-chosen-person-approval".into(),
                kind: "scheduled".into(),
                status: "pending_approval".into(),
                task: "Ask Sam about the private medical update".into(),
                person: None,
                profile: None,
                conversation: None,
                fire_at: Some(1200),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 1000,
                updated_at: 1000,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let denied = check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-pending-chosen-person-approval",
                "status": "active"
            }),
            &review,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("chosen-person authority"));

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        chosen_person
            .store
            .create_intent(&IntentRecord {
                id: "intent-pending-chosen-person-approval".into(),
                kind: "scheduled".into(),
                status: "pending_approval".into(),
                task: "Ask Sam about the private medical update".into(),
                person: None,
                profile: None,
                conversation: None,
                fire_at: Some(1200),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 1000,
                updated_at: 1000,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();
        check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-pending-chosen-person-approval",
                "status": "active"
            }),
            &chosen_person,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn apply_review_is_review_only() {
        let respond = test_context(Authority::ChosenPerson, ActionKind::Respond);
        let denied = check("apply_review", &serde_json::json!({}), &respond)
            .await
            .unwrap_err();
        assert!(denied.contains("review actions"));

        let review = test_context(Authority::Default, ActionKind::Review);
        check("apply_review", &serde_json::json!({}), &review)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_update_or_cancel_cross_target_intent() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        ctx.messages[0].person = Some(PersonId("person-current".into()));
        ctx.store
            .create_intent(&IntentRecord {
                id: "intent-other".into(),
                kind: "scheduled".into(),
                status: "active".into(),
                task: "Follow up with someone else".into(),
                person: Some(PersonId("person-other".into())),
                profile: None,
                conversation: Some(ConversationId("relay:other".into())),
                fire_at: Some(1200),
                condition: None,
                recurrence: None,
                priority: 50,
                dedupe_key: None,
                source_action: None,
                source_memory: None,
                created_at: 1000,
                updated_at: 1000,
                last_fired_at: None,
                chosen_person_approved: false,
            })
            .await
            .unwrap();

        let denied = check(
            "update_intent",
            &serde_json::json!({
                "intent_id": "intent-other",
                "task": "Change the other follow-up"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Updating intents"));

        let denied = check(
            "delete_intent",
            &serde_json::json!({
                "intent_id": "intent-other"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Cancelling intents"));
    }

    #[tokio::test]
    async fn explicit_current_reply_target_is_allowed() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "relay",
                "external_id": "local"
            }),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn background_actions_cannot_send_visible_messages() {
        for kind in [
            ActionKind::Review,
            ActionKind::Research,
            ActionKind::Consolidate,
            ActionKind::Ruminate,
        ] {
            let ctx = test_context(Authority::ChosenPerson, kind);
            let denied = check(
                "send_message",
                &serde_json::json!({
                    "content": "hi",
                    "gateway_id": "relay",
                    "external_id": "local"
                }),
                &ctx,
            )
            .await
            .unwrap_err();
            assert!(denied.contains("internal/background"));
        }
    }

    #[tokio::test]
    async fn conversation_summary_updates_are_current_or_privileged() {
        let current = test_context(Authority::Default, ActionKind::Respond);
        check(
            "update_conversation_summary",
            &serde_json::json!({
                "conversation": "relay:local",
                "summary": "Current conversation summary."
            }),
            &current,
        )
        .await
        .unwrap();

        let denied = check(
            "update_conversation_summary",
            &serde_json::json!({
                "conversation": "relay:other",
                "summary": "Other conversation summary."
            }),
            &current,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("another conversation summary"));

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "update_conversation_summary",
            &serde_json::json!({
                "conversation": "relay:other",
                "summary": "Review can summarize backlog."
            }),
            &review,
        )
        .await
        .unwrap();

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "update_conversation_summary",
            &serde_json::json!({
                "conversation": "relay:other",
                "summary": "Chosen-person-directed summary maintenance."
            }),
            &chosen_person,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn message_reads_are_current_or_privileged() {
        let current = test_context(Authority::Default, ActionKind::Respond);
        check(
            "read_messages",
            &serde_json::json!({
                "conversation": "relay:local",
                "limit": 5
            }),
            &current,
        )
        .await
        .unwrap();

        check("read_messages", &serde_json::json!({"limit": 5}), &current)
            .await
            .unwrap();

        let mut no_current = test_context(Authority::Default, ActionKind::Respond);
        no_current.messages.clear();
        no_current.conversation = None;
        let denied = check(
            "read_messages",
            &serde_json::json!({"limit": 5}),
            &no_current,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("without a current conversation"));

        let mut ruminate = test_context(Authority::Default, ActionKind::Ruminate);
        ruminate.messages.clear();
        ruminate.conversation = None;
        check("read_messages", &serde_json::json!({"limit": 5}), &ruminate)
            .await
            .unwrap();

        let denied = check(
            "read_messages",
            &serde_json::json!({
                "conversation": "relay:other",
                "limit": 5
            }),
            &current,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Reading another conversation"));

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "read_messages",
            &serde_json::json!({
                "conversation": "relay:other",
                "limit": 5
            }),
            &review,
        )
        .await
        .unwrap();

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        check(
            "read_messages",
            &serde_json::json!({
                "conversation": "relay:other",
                "limit": 5
            }),
            &chosen_person,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn trusted_context_can_send_explicit_outbound() {
        let trusted = test_context(Authority::Trusted, ActionKind::Respond);
        check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "discord",
                "external_id": "channel-2"
            }),
            &trusted,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn outreach_context_can_only_send_explicit_scheduled_target() {
        let outreach = test_context(Authority::Default, ActionKind::Outreach);
        check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "relay",
                "external_id": "local"
            }),
            &outreach,
        )
        .await
        .unwrap();

        let denied = check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "discord",
                "external_id": "channel-2"
            }),
            &outreach,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("scheduled outreach target"));
    }

    #[tokio::test]
    async fn outreach_without_current_messages_uses_stored_conversation_target() {
        let mut outreach = test_context(Authority::Default, ActionKind::Outreach);
        let conversation = ConversationId("relay:outreach".into());
        outreach.messages.clear();
        outreach.conversation = Some(conversation.clone());
        outreach
            .store
            .append_message(
                &conversation,
                Some("relay"),
                None,
                &StoredMessage {
                    timestamp: 1000,
                    role: MessageRole::User,
                    content: "previous outreach context".into(),
                    identity: None,
                    profile: None,
                    person: Some(PersonId("person-target".into())),
                    source_gateway_id: Some("relay".into()),
                    source_message_id: Some("msg-outreach-context".into()),
                    sender_external_id: Some("target-1".into()),
                    reply_external_id: Some("target-1".into()),
                    metadata: serde_json::Value::Null,
                },
            )
            .await
            .unwrap();

        check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "relay",
                "external_id": "target-1"
            }),
            &outreach,
        )
        .await
        .unwrap();

        let denied = check(
            "send_message",
            &serde_json::json!({
                "content": "hi",
                "gateway_id": "relay",
                "external_id": "target-2"
            }),
            &outreach,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("scheduled outreach target"));
    }

    #[tokio::test]
    async fn default_user_can_forget_current_profile_memory_only() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        let profile = ProfileId("profile-current".into());
        ctx.messages[0].profile = Some(profile.clone());

        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-current".into()),
                kind: MemoryKind::Semantic,
                content: "current profile preference".into(),
                source: MemorySource::Conversation {
                    conversation_id: ctx.messages[0].conversation.clone(),
                    identity_id: None,
                    profile_id: Some(profile.clone()),
                    person_id: None,
                    message_id: Some(ctx.messages[0].message_id.clone()),
                },
                subjects: vec![MemorySubject::profile(profile, None, 1.0)],
                ..Memory::default()
            })
            .await
            .unwrap();

        check(
            "forget_memory",
            &serde_json::json!({"memory_id": "memory-current"}),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_forget_other_profile_memory() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-other".into()),
                kind: MemoryKind::Semantic,
                content: "other profile preference".into(),
                source: MemorySource::Conversation {
                    conversation_id: ConversationId("relay:other".into()),
                    identity_id: None,
                    profile_id: Some(ProfileId("profile-other".into())),
                    person_id: None,
                    message_id: Some("msg-other".into()),
                },
                subjects: vec![MemorySubject::profile(
                    ProfileId("profile-other".into()),
                    None,
                    1.0,
                )],
                ..Memory::default()
            })
            .await
            .unwrap();

        let denied = check(
            "forget_memory",
            &serde_json::json!({"memory_id": "memory-other"}),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("outside the current profile"));
    }

    #[tokio::test]
    async fn default_user_cannot_forget_current_person_level_memory() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        let profile = ProfileId("profile-current".into());
        let person = PersonId("person-current".into());
        ctx.messages[0].profile = Some(profile.clone());
        ctx.messages[0].person = Some(person.clone());

        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-person-level".into()),
                kind: MemoryKind::Semantic,
                content: "person-level preference".into(),
                source: MemorySource::Conversation {
                    conversation_id: ctx.messages[0].conversation.clone(),
                    identity_id: None,
                    profile_id: Some(profile),
                    person_id: Some(person.clone()),
                    message_id: Some(ctx.messages[0].message_id.clone()),
                },
                subjects: vec![MemorySubject::person(person, None, 1.0)],
                visibility_scope: VisibilityScope::Person,
                ..Memory::default()
            })
            .await
            .unwrap();

        let denied = check(
            "forget_memory",
            &serde_json::json!({"memory_id": "memory-person-level"}),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("outside the current profile"));
    }

    #[tokio::test]
    async fn live_user_cannot_promote_profile_memory_to_person_level_memory() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "promote_profile_memory_to_person",
            &serde_json::json!({
                "memory_id": "memory-current-profile",
                "person": "person-current"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("Promoting profile memories"));
    }

    #[tokio::test]
    async fn review_can_promote_profile_memory_to_verified_person() {
        let ctx = test_context(Authority::Default, ActionKind::Review);
        let profile = ProfileId("profile-current".into());
        let person = PersonId("person-current".into());
        add_verified_target(&ctx, &profile, &person).await;
        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-current-profile".into()),
                kind: MemoryKind::Semantic,
                content: "current profile preference".into(),
                source: MemorySource::Reflection,
                subjects: vec![MemorySubject::profile(profile, Some("about".into()), 1.0)],
                ..Memory::default()
            })
            .await
            .unwrap();

        check(
            "promote_profile_memory_to_person",
            &serde_json::json!({
                "memory_id": "memory-current-profile",
                "person": "person-current"
            }),
            &ctx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn review_cannot_promote_profile_memory_to_unverified_person() {
        let ctx = test_context(Authority::Default, ActionKind::Review);
        let profile = ProfileId("profile-unverified".into());
        let person = PersonId("person-unverified".into());
        ctx.store
            .add_profile(&Profile {
                id: profile.clone(),
                display_name: Some("Unverified profile".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();
        ctx.store
            .add_person(&Person {
                id: person,
                name: Some("Unverified person".into()),
                summary: None,
                comm_style: None,
                first_seen: 1000,
                last_seen: 1000,
            })
            .await
            .unwrap();
        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-unverified-profile".into()),
                kind: MemoryKind::Semantic,
                content: "unverified profile preference".into(),
                source: MemorySource::Reflection,
                subjects: vec![MemorySubject::profile(profile, Some("about".into()), 1.0)],
                ..Memory::default()
            })
            .await
            .unwrap();

        let denied = check(
            "promote_profile_memory_to_person",
            &serde_json::json!({
                "memory_id": "memory-unverified-profile",
                "person": "person-unverified"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("verified or strong likely link"));
    }

    #[tokio::test]
    async fn live_user_cannot_demote_person_level_memory_subjects() {
        let ctx = test_context(Authority::Default, ActionKind::Respond);

        let denied = check(
            "demote_person_memory_to_profile",
            &serde_json::json!({
                "memory_id": "memory-person-level",
                "profile": "profile-current"
            }),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(denied.contains("Demoting person-level memories"));

        let review = test_context(Authority::Default, ActionKind::Review);
        check(
            "demote_person_memory_to_profile",
            &serde_json::json!({
                "memory_id": "memory-person-level",
                "profile": "profile-current"
            }),
            &review,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn default_user_cannot_forget_sensitive_current_profile_memory() {
        let mut ctx = test_context(Authority::Default, ActionKind::Respond);
        let profile = ProfileId("profile-current".into());
        ctx.messages[0].profile = Some(profile.clone());

        ctx.store
            .store_memory(&Memory {
                id: MemoryId("memory-sensitive-profile".into()),
                kind: MemoryKind::Semantic,
                content: "current profile sensitive detail".into(),
                source: MemorySource::Conversation {
                    conversation_id: ctx.messages[0].conversation.clone(),
                    identity_id: None,
                    profile_id: Some(profile.clone()),
                    person_id: None,
                    message_id: Some(ctx.messages[0].message_id.clone()),
                },
                subjects: vec![MemorySubject::profile(profile, None, 1.0)],
                privacy_category: PrivacyCategory::Sensitive,
                ..Memory::default()
            })
            .await
            .unwrap();

        let denied = check(
            "forget_memory",
            &serde_json::json!({"memory_id": "memory-sensitive-profile"}),
            &ctx,
        )
        .await
        .unwrap_err();

        assert!(denied.contains("outside the current profile"));
    }

    #[tokio::test]
    async fn memory_inspection_and_deletion_by_id_are_chosen_person_only() {
        let default = test_context(Authority::Default, ActionKind::Respond);
        for tool in ["inspect_memory", "delete_memory"] {
            let denied = check(
                tool,
                &serde_json::json!({"memory_id": "memory-secret"}),
                &default,
            )
            .await
            .unwrap_err();
            assert!(denied.contains("Chosen-person authority"));
        }

        let chosen_person = test_context(Authority::ChosenPerson, ActionKind::Respond);
        for tool in ["inspect_memory", "delete_memory"] {
            check(
                tool,
                &serde_json::json!({"memory_id": "memory-secret"}),
                &chosen_person,
            )
            .await
            .unwrap();
        }
    }
}
