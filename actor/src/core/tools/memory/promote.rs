use super::super::context::SessionContext;
use super::helpers::{canonicalize_content_for_subjects, has_subject, memory_body};
use crate::store::{MemorySubject, MemorySubjectType, MemoryUpdate};
use protocol::{MemoryId, PersonId, ProfileId};
use serde_json::{Value, json};

pub async fn promote_profile_memory_to_person(args: &Value, ctx: &SessionContext) -> String {
    let Some(memory_id) = args["memory_id"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide memory_id."}).to_string();
    };
    let Some(person) = args["person"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide person."}).to_string();
    };

    let memory_id = MemoryId(memory_id.trim().to_string());
    let person = PersonId(person.trim().to_string());
    let memory = match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => memory,
        Ok(None) => return json!({"error": "Memory not found."}).to_string(),
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    if ctx.store.get_person(&person).await.ok().flatten().is_none() {
        return json!({"error": "Person not found."}).to_string();
    }

    let mut subjects = memory.subjects.clone();
    let promoted = MemorySubject::person(person.clone(), Some("about".into()), 1.0);
    if !has_subject(&subjects, &promoted) {
        subjects.push(promoted);
    }

    let content =
        canonicalize_content_for_subjects(&memory_body(&memory.content), &subjects, ctx).await;
    let update = MemoryUpdate {
        content: Some(content),
        importance: None,
        sensitivity: None,
        emotional_valence: None,
        tags: None,
        subjects: Some(subjects),
        embedding: None,
        ..Default::default()
    };

    match ctx.store.update_memory(&memory_id, &update).await {
        Ok(()) => json!({
            "status": "promoted",
            "memory_id": memory_id.0,
            "person": person.0,
            "reason": args["reason"].as_str()
        })
        .to_string(),
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}

pub async fn demote_person_memory_to_profile(args: &Value, ctx: &SessionContext) -> String {
    let Some(memory_id) = args["memory_id"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide memory_id."}).to_string();
    };
    let Some(profile) = args["profile"].as_str().filter(|s| !s.trim().is_empty()) else {
        return json!({"error": "Provide profile."}).to_string();
    };

    let memory_id = MemoryId(memory_id.trim().to_string());
    let profile = ProfileId(profile.trim().to_string());
    let memory = match ctx.store.get_memory(&memory_id).await {
        Ok(Some(memory)) => memory,
        Ok(None) => return json!({"error": "Memory not found."}).to_string(),
        Err(e) => return json!({"error": format!("{e}")}).to_string(),
    };

    if ctx
        .store
        .get_profile(&profile)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return json!({"error": "Profile not found."}).to_string();
    }

    let remove_person = args["person"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    let mut subjects = memory
        .subjects
        .iter()
        .filter(|subject| {
            if subject.subject_type != MemorySubjectType::Person {
                return true;
            }
            match &remove_person {
                Some(person) => subject.subject_id != *person,
                None => false,
            }
        })
        .cloned()
        .collect::<Vec<_>>();
    let demoted = MemorySubject::profile(profile.clone(), Some("about".into()), 1.0);
    if !has_subject(&subjects, &demoted) {
        subjects.push(demoted);
    }

    let content =
        canonicalize_content_for_subjects(&memory_body(&memory.content), &subjects, ctx).await;
    let update = MemoryUpdate {
        content: Some(content),
        importance: None,
        sensitivity: None,
        emotional_valence: None,
        tags: None,
        subjects: Some(subjects),
        embedding: None,
        ..Default::default()
    };

    match ctx.store.update_memory(&memory_id, &update).await {
        Ok(()) => json!({
            "status": "demoted",
            "memory_id": memory_id.0,
            "profile": profile.0,
            "removed_person": remove_person,
            "reason": args["reason"].as_str()
        })
        .to_string(),
        Err(e) => json!({"error": format!("{e}")}).to_string(),
    }
}
