use super::super::context::SessionContext;
use crate::store::{MemorySubject, MemorySubjectType, MemoryUpdate};
use protocol::{PersonId, ProfileId};
use serde_json::Value;

const PROFILE_PERSON_MEMORY_CLEANUP_LIMIT: usize = 5_000;

pub(super) fn resolve_person_ref(args: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if let Some(r) = args["ref"].as_str().filter(|s| !s.is_empty()) {
        return Some(PersonId(r.to_string()));
    }
    ctx.messages.first().and_then(|m| m.person.clone())
}

pub(super) fn current_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.messages.first().and_then(|m| m.person.clone())
}

pub(super) fn current_profile(ctx: &SessionContext) -> Option<ProfileId> {
    ctx.messages.first().and_then(|m| m.profile.clone())
}

pub(super) async fn remove_detached_person_subject_from_profile_memories(
    ctx: &SessionContext,
    profile: &ProfileId,
    person: &PersonId,
) -> anyhow::Result<usize> {
    let memories = ctx
        .store
        .memories_for_subject(
            MemorySubjectType::Profile,
            &profile.0,
            PROFILE_PERSON_MEMORY_CLEANUP_LIMIT,
        )
        .await?;
    let mut rewritten = 0;
    for memory in memories {
        let has_profile_subject = memory.subjects.iter().any(|subject| {
            subject.subject_type == MemorySubjectType::Profile && subject.subject_id == profile.0
        });
        let has_person_subject = memory.subjects.iter().any(|subject| {
            subject.subject_type == MemorySubjectType::Person && subject.subject_id == person.0
        });
        if !has_profile_subject || !has_person_subject {
            continue;
        }

        let subjects = memory
            .subjects
            .iter()
            .filter(|subject| {
                !(subject.subject_type == MemorySubjectType::Person
                    && subject.subject_id == person.0)
            })
            .cloned()
            .collect::<Vec<_>>();
        let content = canonicalize_memory_content(&memory.content, &subjects, ctx).await;
        ctx.store
            .update_memory(
                &memory.id,
                &MemoryUpdate {
                    content: Some(content),
                    subjects: Some(subjects),
                    ..Default::default()
                },
            )
            .await?;
        rewritten += 1;
    }
    Ok(rewritten)
}

async fn canonicalize_memory_content(
    existing_content: &str,
    subjects: &[MemorySubject],
    ctx: &SessionContext,
) -> String {
    let body = memory_body(existing_content);
    if subjects.is_empty() {
        return body;
    }

    let mut lines = vec!["Subjects involved:".to_string()];
    for subject in subjects {
        let label = if subject.subject_type == MemorySubjectType::Person {
            ctx.store
                .get_person(&PersonId(subject.subject_id.clone()))
                .await
                .ok()
                .flatten()
                .and_then(|person| person.name)
                .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
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
    lines.push(format!("Memory: {body}"));
    lines.join("\n")
}

fn memory_body(content: &str) -> String {
    content
        .rsplit_once("\nMemory: ")
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| content.to_string())
}
