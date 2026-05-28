use super::BehaviourCase;
use super::json::{optional_str, required_array, required_object, required_str};
use super::world::{SeedContexts, SeedProfileContext};
use anyhow::Context;
use protocol::{ConversationId, GroupId, InboundMessage};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const INPUT_BASE_TIME: i64 = 1_700_000_100;

pub struct CaseInput {
    pub messages: Vec<InboundMessage>,
    pub gateway_ids: Vec<String>,
}

pub fn build_case_input(
    case: &BehaviourCase,
    contexts: &SeedContexts,
) -> anyhow::Result<CaseInput> {
    let input = required_object(&case.value, "input", &case.path);
    let messages = required_array(input, "messages", &case.path);
    let mut inbound = Vec::with_capacity(messages.len());
    let mut gateway_ids = BTreeSet::new();

    for (idx, message) in messages.iter().enumerate() {
        let role = required_str(message, "role", &case.path);
        if role != "user" {
            anyhow::bail!(
                "{} input message role {role} is not executable as an inbound actor event yet",
                case.path.display()
            );
        }

        let built = build_inbound_message(case, contexts, message, idx).with_context(|| {
            format!(
                "failed to build input message {idx} for {}",
                case.path.display()
            )
        })?;
        gateway_ids.insert(built.gateway_id.clone());
        inbound.push(built);
    }

    Ok(CaseInput {
        messages: inbound,
        gateway_ids: gateway_ids.into_iter().collect(),
    })
}

fn build_inbound_message(
    case: &BehaviourCase,
    contexts: &SeedContexts,
    message: &Value,
    idx: usize,
) -> anyhow::Result<InboundMessage> {
    let profile_id = optional_str(message, "profile_id");
    let explicit_gateway = optional_str(message, "gateway_id");
    let profile = profile_id
        .map(|id| {
            contexts
                .profile(id)
                .ok_or_else(|| anyhow::anyhow!("unknown seeded profile {id}"))
        })
        .transpose()?;
    let gateway_id = explicit_gateway
        .map(str::to_string)
        .or_else(|| profile.map(|profile| profile.gateway_id.clone()))
        .unwrap_or_else(|| "relay".to_string());
    let profile = profile.or_else(|| single_profile_for_gateway(contexts, &gateway_id));

    let group_id = optional_str(message, "group_id");
    let group = group_id
        .map(|id| {
            contexts
                .group(id)
                .ok_or_else(|| anyhow::anyhow!("unknown seeded group {id}"))
        })
        .transpose()?;

    let sender_external_id = profile
        .map(|profile| profile.external_id.clone())
        .unwrap_or_else(|| default_external_id(&gateway_id));
    let reply_external_id = group
        .map(|group| group.external_id.clone())
        .unwrap_or_else(|| sender_external_id.clone());
    let conversation = explicit_conversation(message)
        .or_else(|| {
            group_id
                .and_then(|id| contexts.conversation_for_group(id))
                .cloned()
        })
        .or_else(|| {
            profile_id
                .and_then(|id| contexts.conversation_for_profile(id))
                .cloned()
        })
        .unwrap_or_else(|| ConversationId(format!("{gateway_id}:{reply_external_id}")));

    Ok(InboundMessage {
        message_id: format!(
            "{}:input:{idx}",
            required_str(&case.value, "id", &case.path)
        ),
        gateway_id,
        sender_external_id,
        sender_display_name: profile
            .and_then(|profile| profile.display_name.clone())
            .or_else(|| optional_str(message, "display_name").map(str::to_string)),
        reply_external_id,
        conversation,
        group: group_id.map(|id| GroupId(id.to_string())),
        identity: profile.map(|profile| profile.identity_id.clone()),
        profile: profile.map(|profile| profile.profile_id.clone()),
        person: profile.map(|profile| profile.person_id.clone()),
        content: required_str(message, "text", &case.path).to_string(),
        attachments: vec![],
        timestamp: INPUT_BASE_TIME + idx as i64,
        metadata: input_metadata(case, message, group.map(|group| group.name.as_str())),
    })
}

fn single_profile_for_gateway<'a>(
    contexts: &'a SeedContexts,
    gateway_id: &str,
) -> Option<&'a SeedProfileContext> {
    let mut matches = contexts
        .profiles
        .values()
        .filter(|profile| profile.gateway_id == gateway_id);
    let first = matches.next()?;
    if matches.next().is_none() {
        Some(first)
    } else {
        None
    }
}

fn explicit_conversation(message: &Value) -> Option<ConversationId> {
    optional_str(message, "conversation_id").map(|id| ConversationId(id.to_string()))
}

fn default_external_id(gateway_id: &str) -> String {
    if gateway_id == "relay" {
        "relay-user".to_string()
    } else {
        format!("{gateway_id}-behaviour-user")
    }
}

fn input_metadata(case: &BehaviourCase, message: &Value, group_name: Option<&str>) -> Value {
    let mut metadata = json!({
        "source": "behaviour_spec_input",
        "case_id": required_str(&case.value, "id", &case.path),
    });
    if let Some(profile_id) = optional_str(message, "profile_id") {
        metadata["profile_id"] = json!(profile_id);
    }
    if let Some(group_id) = optional_str(message, "group_id") {
        metadata["group_id"] = json!(group_id);
    }
    if let Some(group_name) = group_name {
        metadata["group_name"] = json!(group_name);
    }
    metadata
}
