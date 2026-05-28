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
mod tests;
