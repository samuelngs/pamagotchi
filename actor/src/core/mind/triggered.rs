use super::*;
use crate::store::IntentRecord;
use std::collections::HashSet;

pub(super) fn triggered_intent_satisfied_by_inbound_response(
    intent: &IntentRecord,
    msg: &InboundMessage,
) -> bool {
    if intent.kind != "triggered" || intent.status != "active" {
        return false;
    }
    if !intent_context_matches_message(intent, msg) {
        return false;
    }
    let Some(condition) = intent.condition.as_deref() else {
        return false;
    };
    is_simple_next_inbound_condition(condition)
        || content_specific_condition_matches_message(condition, msg)
}

fn intent_context_matches_message(intent: &IntentRecord, msg: &InboundMessage) -> bool {
    let targeted =
        intent.person.is_some() || intent.profile.is_some() || intent.conversation.is_some();
    if !targeted {
        let condition = intent.condition.as_deref().unwrap_or("");
        return is_generic_next_inbound_condition(condition)
            || has_content_specific_condition(&normalize_condition(condition));
    }

    if intent
        .person
        .as_ref()
        .is_some_and(|id| Some(id) != msg.person.as_ref())
    {
        return false;
    }
    if intent
        .profile
        .as_ref()
        .is_some_and(|id| Some(id) != msg.profile.as_ref())
    {
        return false;
    }
    if intent
        .conversation
        .as_ref()
        .is_some_and(|id| id != &msg.conversation)
    {
        return false;
    }
    true
}

fn is_simple_next_inbound_condition(condition: &str) -> bool {
    let condition = normalize_condition(condition);
    if condition.is_empty() || has_content_specific_condition(&condition) {
        return false;
    }
    let inbound = [
        "message", "messages", "msg", "reply", "replies", "respond", "responds", "response",
        "contact", "contacts", "ping", "pings", "dm", "chat", "talk",
    ]
    .iter()
    .any(|needle| condition.contains(needle));
    let nextish = condition.contains("next")
        || condition.contains("when they")
        || condition.contains("when this person")
        || condition.contains("when the person")
        || condition.contains("when user")
        || condition.contains("when the user");
    inbound && nextish
}

fn is_generic_next_inbound_condition(condition: &str) -> bool {
    let condition = normalize_condition(condition);
    if condition.is_empty() || has_content_specific_condition(&condition) {
        return false;
    }
    [
        "next message",
        "next reply",
        "next response",
        "next inbound",
        "next contact",
        "next ping",
        "next dm",
        "anyone messages",
        "someone messages",
        "someone replies",
    ]
    .iter()
    .any(|needle| condition.contains(needle))
}

fn has_content_specific_condition(condition: &str) -> bool {
    [
        "about ",
        "mention",
        "mentions",
        "ask ",
        "asks ",
        "asked ",
        "say ",
        "says ",
        "said ",
        "bring up",
        "brings up",
        "need ",
        "needs ",
    ]
    .iter()
    .any(|needle| condition.contains(needle))
}

fn normalize_condition(condition: &str) -> String {
    condition
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_specific_condition_matches_message(condition: &str, msg: &InboundMessage) -> bool {
    let condition = normalize_for_keyword_match(condition);
    let message = normalize_for_keyword_match(&msg.content);
    if condition.is_empty() || message.is_empty() {
        return false;
    }

    let Some(topic) = extract_condition_topic(&condition) else {
        return false;
    };
    if topic.is_empty() {
        return false;
    }

    let message_words = message.split_whitespace().collect::<HashSet<_>>();
    topic
        .split_whitespace()
        .all(|word| message_words.contains(word))
}

fn extract_condition_topic(condition: &str) -> Option<String> {
    for marker in [
        "mentions ",
        "mention ",
        "brings up ",
        "bring up ",
        "asks about ",
        "ask about ",
        "asked about ",
        "says ",
        "say ",
        "said ",
        "about ",
    ] {
        if let Some(rest) = condition.split(marker).nth(1) {
            let topic = rest
                .split_whitespace()
                .filter(|word| !condition_topic_stop_word(word))
                .take(6)
                .collect::<Vec<_>>()
                .join(" ");
            if !topic.is_empty() {
                return Some(topic);
            }
        }
    }
    None
}

fn condition_topic_stop_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "this"
            | "that"
            | "their"
            | "his"
            | "her"
            | "my"
            | "your"
            | "our"
            | "again"
            | "next"
            | "please"
    )
}

fn normalize_for_keyword_match(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
