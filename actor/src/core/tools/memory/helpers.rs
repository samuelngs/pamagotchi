use super::super::context::SessionContext;
use crate::store::{MemorySubject, MemorySubjectType};
use protocol::{IdentityId, PersonId, ProfileId};
use serde_json::Value;

pub(super) fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

fn clean_display_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) async fn canonicalize_content_for_subjects(
    content: &str,
    subjects: &[MemorySubject],
    ctx: &SessionContext,
) -> String {
    if subjects.is_empty() || content.starts_with("Subjects involved:\n") {
        return content.to_string();
    }

    let mut lines = vec!["Subjects involved:".to_string()];
    for subject in subjects {
        let label = if subject.subject_type == MemorySubjectType::Person {
            ctx.store
                .get_person(&PersonId(subject.subject_id.clone()))
                .await
                .ok()
                .flatten()
                .and_then(|person| person.name.map(|name| clean_display_name(&name)))
                .filter(|name| !name.is_empty())
        } else {
            None
        };
        if let Some(name) = label {
            lines.push(format!(
                "- {} {} (display name: {})",
                subject.subject_type.as_str(),
                subject.subject_id,
                name
            ));
        } else {
            lines.push(format!(
                "- {} {}",
                subject.subject_type.as_str(),
                subject.subject_id
            ));
        }
    }
    lines.push(format!("Memory: {content}"));
    lines.join("\n")
}

pub(super) fn memory_body(content: &str) -> String {
    content
        .rsplit_once("\nMemory: ")
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| content.to_string())
}

pub(super) fn has_subject(subjects: &[MemorySubject], target: &MemorySubject) -> bool {
    subjects.iter().any(|subject| {
        subject.subject_type == target.subject_type && subject.subject_id == target.subject_id
    })
}

pub(super) async fn current_subject_relation(
    subjects: &[MemorySubject],
    current: Option<&protocol::InboundMessage>,
    ctx: &SessionContext,
) -> &'static str {
    if let Some(current) = current {
        if let Some(identity) = &current.identity {
            if subjects.iter().any(|s| {
                s.subject_type == MemorySubjectType::Identity && s.subject_id == identity.0
            }) {
                return "same_identity";
            }
        }
        if let Some(profile) = &current.profile {
            if subjects
                .iter()
                .any(|s| s.subject_type == MemorySubjectType::Profile && s.subject_id == profile.0)
            {
                return "same_profile";
            }
        }
        if let Some(person) = &current.person {
            if subjects
                .iter()
                .any(|s| s.subject_type == MemorySubjectType::Person && s.subject_id == person.0)
            {
                return "same_person";
            }
            for subject in subjects
                .iter()
                .filter(|s| s.subject_type == MemorySubjectType::Profile)
            {
                if let Ok(Some((_subject_person, link))) = ctx
                    .store
                    .get_person_for_profile(&ProfileId(subject.subject_id.clone()))
                    .await
                {
                    if link.person_id == *person {
                        return match link.status.as_str() {
                            "verified" => "verified_same_person_profile",
                            "likely" => "likely_same_person_profile",
                            _ => "different_subject",
                        };
                    }
                }
            }
        }
        if !subjects.is_empty() {
            return "different_subject";
        }
        return "unlinked";
    }
    "unknown"
}

pub(super) fn relation_rank(relation: &str) -> u8 {
    match relation {
        "same_profile" => 0,
        "same_identity" => 1,
        "same_person" => 2,
        "verified_same_person_profile" => 3,
        "likely_same_person_profile" => 4,
        "unlinked" => 5,
        "unknown" => 6,
        _ => 7,
    }
}

pub(super) fn string_array(value: &Value) -> impl Iterator<Item = String> + '_ {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

pub(super) fn current_identity(ctx: &SessionContext) -> Option<IdentityId> {
    ctx.messages.first().and_then(|m| m.identity.clone())
}

pub(super) fn current_profile(ctx: &SessionContext) -> Option<ProfileId> {
    ctx.messages.first().and_then(|m| m.profile.clone())
}

pub(super) fn global_recall_requested(args: &Value) -> bool {
    matches!(args["scope"].as_str(), Some("global")) || args["global"].as_bool().unwrap_or(false)
}
