use super::super::context::SessionContext;
use super::helpers::current_person;
use crate::identity::PersonProfileStatus;
use protocol::{PersonId, ProfileId};
use serde_json::{Value, json};

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
