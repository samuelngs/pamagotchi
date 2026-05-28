use super::*;

pub(super) async fn apply_profile_updates(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
) {
    for item in array_items(&args["profile_updates"]) {
        let Some(profile_id) = item["profile_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| ProfileId(id.to_string()))
        else {
            counts
                .skipped
                .push("profile_update missing profile_id".into());
            continue;
        };
        if !profile_update_target_allowed(ctx, state, item, &profile_id) {
            counts.skipped.push(format!(
                "profile {} is not present in review evidence",
                profile_id.0
            ));
            continue;
        }

        let display_name = item["display_name"]
            .as_str()
            .filter(|s| !s.trim().is_empty());
        let summary = item["summary"].as_str().filter(|s| !s.trim().is_empty());
        let comm_style = item["comm_style"].as_str().filter(|s| !s.trim().is_empty());
        if display_name.is_none() && summary.is_none() && comm_style.is_none() {
            counts
                .skipped
                .push(format!("profile {} had no fields", profile_id.0));
            continue;
        }

        let existing = ctx.store.get_profile(&profile_id).await.ok().flatten();
        let summary_update = summary.and_then(|summary| {
            merge_summary_update(
                existing
                    .as_ref()
                    .and_then(|profile| profile.summary.as_deref()),
                summary,
            )
        });

        let mut applied = false;
        if display_name.is_some() || summary_update.is_some() {
            if let Err(e) = ctx
                .store
                .update_profile(&profile_id, display_name, summary_update.as_deref())
                .await
            {
                counts
                    .skipped
                    .push(format!("profile {} update failed: {e}", profile_id.0));
                continue;
            }
            applied = true;
        }
        if let Some(comm_style) = comm_style {
            if let Err(e) = ctx
                .store
                .update_profile_comm_style(&profile_id, comm_style)
                .await
            {
                counts
                    .skipped
                    .push(format!("profile {} style failed: {e}", profile_id.0));
                continue;
            }
            applied = true;
        }
        if !applied {
            counts
                .skipped
                .push(format!("profile {} had no new fields", profile_id.0));
            continue;
        }
        counts.profile_updates += 1;
    }
}

pub(super) async fn apply_person_updates(
    args: &Value,
    ctx: &SessionContext,
    state: &SessionState,
    counts: &mut ApplyCounts,
) {
    for item in array_items(&args["person_updates"]) {
        let Some(person_id) = item["person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
        else {
            counts
                .skipped
                .push("person_update missing person_id".into());
            continue;
        };
        if !person_has_verified_or_strong_profile_context(ctx, &person_id).await {
            counts.skipped.push(format!(
                "person {} is not verified or strongly likely enough for person-level update",
                person_id.0
            ));
            continue;
        }
        if !person_update_target_allowed(ctx, state, item, &person_id) {
            counts.skipped.push(format!(
                "person {} is not present in review evidence",
                person_id.0
            ));
            continue;
        }

        let name = item["name"].as_str().filter(|s| !s.trim().is_empty());
        let summary = item["summary"].as_str().filter(|s| !s.trim().is_empty());
        let comm_style = item["comm_style"].as_str().filter(|s| !s.trim().is_empty());
        if name.is_none() && summary.is_none() && comm_style.is_none() {
            counts
                .skipped
                .push(format!("person {} had no fields", person_id.0));
            continue;
        }

        let existing = ctx.store.get_person(&person_id).await.ok().flatten();
        let summary_update = summary.and_then(|summary| {
            merge_summary_update(
                existing
                    .as_ref()
                    .and_then(|person| person.summary.as_deref()),
                summary,
            )
        });

        let mut applied = false;
        if name.is_some() || summary_update.is_some() {
            if let Err(e) = ctx
                .store
                .update_person(&person_id, name, summary_update.as_deref())
                .await
            {
                counts
                    .skipped
                    .push(format!("person {} update failed: {e}", person_id.0));
                continue;
            }
            applied = true;
        }
        if let Some(comm_style) = comm_style {
            if let Err(e) = ctx.store.update_comm_style(&person_id, comm_style).await {
                counts
                    .skipped
                    .push(format!("person {} style failed: {e}", person_id.0));
                continue;
            }
            applied = true;
        }
        if !applied {
            counts
                .skipped
                .push(format!("person {} had no new fields", person_id.0));
            continue;
        }
        counts.person_updates += 1;
    }
}
