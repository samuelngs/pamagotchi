use super::*;

pub(super) async fn apply_open_loops(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    for (idx, item) in array_items(&args["open_loops"]).enumerate() {
        let Some(task) = item["task"].as_str().filter(|task| !task.trim().is_empty()) else {
            counts.skipped.push("open_loop missing task".into());
            continue;
        };
        let fire_at = item["fire_at"].as_i64();
        let condition = item["condition"]
            .as_str()
            .map(str::trim)
            .filter(|condition| !condition.is_empty())
            .map(str::to_string);
        let Some(kind) =
            normalize_open_loop_kind(item["kind"].as_str(), fire_at, condition.as_deref())
        else {
            counts
                .skipped
                .push(format!("open_loop {idx} has unsupported kind"));
            continue;
        };
        if kind == "scheduled" && fire_at.is_none() {
            counts
                .skipped
                .push(format!("open_loop {idx} missing fire_at"));
            continue;
        }
        if kind == "triggered" && condition.is_none() {
            counts
                .skipped
                .push(format!("open_loop {idx} missing condition"));
            continue;
        }
        if permission::intent_requires_chosen_person_approval(item)
            && !matches!(ctx.authority, crate::state::Authority::ChosenPerson)
        {
            match create_chosen_person_proactive_approval_intent(
                item,
                ctx,
                idx,
                task,
                kind,
                fire_at,
                condition.as_deref(),
            )
            .await
            {
                Some(_) => counts.open_loops += 1,
                None => counts.skipped.push(format!(
                    "open_loop {idx} requires chosen-person approval for sensitive proactive outreach"
                )),
            }
            continue;
        }
        if !matches!(ctx.authority, crate::state::Authority::ChosenPerson) {
            match permission::intent_targets_current_or_verified_with_keys(
                item,
                ctx,
                "person_id",
                "profile_id",
                "conversation_id",
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    counts.skipped.push(format!(
                        "open_loop {idx} targets an unverified third-party outreach"
                    ));
                    continue;
                }
                Err(e) => {
                    counts
                        .skipped
                        .push(format!("open_loop {idx} target verification failed: {e}"));
                    continue;
                }
            }
        }
        let now = util::now();
        let conversation = item["conversation_id"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| ctx.conversation.clone());
        let person = item["person_id"]
            .as_str()
            .map(|id| PersonId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.person.clone())
            });
        let profile = item["profile_id"]
            .as_str()
            .map(|id| ProfileId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.profile.clone())
            });
        let dedupe_key = item["dedupe_key"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "review:{}:open_loop:{}:{}:{}:{}",
                    ctx.action_id.0,
                    idx,
                    open_loop_timing_key(fire_at, condition.as_deref()),
                    conversation
                        .as_ref()
                        .map(|conversation| conversation.0.as_str())
                        .unwrap_or("none"),
                    stable_hash(task)
                )
            });
        if !state
            .applied_review_keys
            .insert(format!("open_loop:{dedupe_key}"))
        {
            counts.skipped.push(format!("open_loop {idx} duplicate"));
            continue;
        }
        let intent = IntentRecord {
            id: format!("intent-{}", util::uuid_v4()),
            kind: kind.to_string(),
            status: "active".into(),
            task: task.to_string(),
            person,
            profile,
            conversation: conversation.clone(),
            fire_at: if kind == "scheduled" { fire_at } else { None },
            condition: if kind == "triggered" { condition } else { None },
            recurrence: None,
            priority: item["priority"].as_u64().unwrap_or(50).min(100) as u8,
            dedupe_key: Some(dedupe_key),
            source_action: Some(ctx.action_id.0.clone()),
            source_memory: source_memory_id(item),
            created_at: now,
            updated_at: now,
            last_fired_at: None,
            chosen_person_approved: matches!(ctx.authority, crate::state::Authority::ChosenPerson),
        };
        match ctx.store.create_intent(&intent).await {
            Ok(()) => counts.open_loops += 1,
            Err(e) => counts.skipped.push(format!("open_loop {idx} failed: {e}")),
        }
    }
}

fn source_memory_id(item: &Value) -> Option<MemoryId> {
    item["source_memory_id"]
        .as_str()
        .or_else(|| item["source_memory"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| MemoryId(id.to_string()))
}

fn open_loop_timing_key(fire_at: Option<i64>, condition: Option<&str>) -> String {
    if let Some(fire_at) = fire_at {
        return fire_at.to_string();
    }
    condition
        .map(stable_hash)
        .unwrap_or_else(|| "unspecified".into())
}

fn normalize_open_loop_kind(
    kind: Option<&str>,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> Option<&'static str> {
    match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
        Some("scheduled") => Some("scheduled"),
        Some("triggered") => Some("triggered"),
        Some("follow_up") | None => {
            if condition.is_some() && fire_at.is_none() {
                Some("triggered")
            } else {
                Some("scheduled")
            }
        }
        Some(_) => None,
    }
}

async fn create_chosen_person_proactive_approval_intent(
    item: &Value,
    ctx: &SessionContext,
    idx: usize,
    task: &str,
    kind: &str,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> Option<String> {
    let chosen_person = chosen_person(ctx)?;
    let now = util::now();
    let original_dedupe_key = item["dedupe_key"]
        .as_str()
        .filter(|key| !key.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "review:{}:open_loop:{}:{}:{}",
                ctx.action_id.0,
                idx,
                open_loop_timing_key(fire_at, condition),
                stable_hash(task)
            )
        });
    let pending_id = format!("intent-{}", util::uuid_v4());
    let pending_intent = IntentRecord {
        id: pending_id.clone(),
        kind: kind.to_string(),
        status: "pending_approval".into(),
        task: task.to_string(),
        person: item["person_id"]
            .as_str()
            .map(|id| PersonId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.person.clone())
            }),
        profile: item["profile_id"]
            .as_str()
            .map(|id| ProfileId(id.to_string()))
            .or_else(|| {
                ctx.messages
                    .first()
                    .and_then(|message| message.profile.clone())
            }),
        conversation: item["conversation_id"]
            .as_str()
            .map(|id| ConversationId(id.to_string()))
            .or_else(|| ctx.conversation.clone()),
        fire_at: if kind == "scheduled" { fire_at } else { None },
        condition: if kind == "triggered" {
            condition.map(str::to_string)
        } else {
            None
        },
        recurrence: item["recurrence"].as_str().map(str::to_string),
        priority: item["priority"].as_u64().unwrap_or(50).min(100) as u8,
        dedupe_key: Some(original_dedupe_key.clone()),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: source_memory_id(item),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved: false,
    };
    if let Err(e) = ctx.store.create_intent(&pending_intent).await {
        tracing::warn!(
            action = %ctx.action_id,
            %e,
            "failed to create pending sensitive open loop intent"
        );
        return None;
    }
    let target = chosen_person_approval_target_description(item, ctx, fire_at, condition);
    let intent = IntentRecord {
        id: format!("intent-{}", util::uuid_v4()),
        kind: "scheduled".into(),
        status: "active".into(),
        task: format!(
            "Review sensitive proactive outreach before it is sent. Pending intent: {pending_id}. Proposed task: {task}. {target} If the chosen person approves, update intent {pending_id} with status active. If the chosen person declines, delete intent {pending_id}."
        ),
        person: Some(chosen_person),
        profile: None,
        conversation: None,
        fire_at: Some(now),
        condition: None,
        recurrence: None,
        priority: 100,
        dedupe_key: Some(format!(
            "chosen-person-approval:sensitive-open-loop:{original_dedupe_key}"
        )),
        source_action: Some(ctx.action_id.0.clone()),
        source_memory: source_memory_id(item),
        created_at: now,
        updated_at: now,
        last_fired_at: None,
        chosen_person_approved: true,
    };
    let id = intent.id.clone();
    match ctx.store.create_intent(&intent).await {
        Ok(()) => Some(id),
        Err(e) => {
            tracing::warn!(
                action = %ctx.action_id,
                %e,
                "failed to create chosen-person approval intent for sensitive open loop"
            );
            None
        }
    }
}

fn chosen_person(ctx: &SessionContext) -> Option<PersonId> {
    let actor = ctx.state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, relationship)| {
            matches!(
                relationship.authority,
                crate::state::Authority::ChosenPerson
            )
        })
        .map(|(person, _)| person.clone())
}

fn chosen_person_approval_target_description(
    item: &Value,
    ctx: &SessionContext,
    fire_at: Option<i64>,
    condition: Option<&str>,
) -> String {
    let person = item["person_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            ctx.messages
                .first()
                .and_then(|message| message.person.as_ref())
                .map(|id| id.0.clone())
        })
        .unwrap_or_else(|| "unknown person".into());
    let profile = item["profile_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            ctx.messages
                .first()
                .and_then(|message| message.profile.as_ref())
                .map(|id| id.0.clone())
        })
        .unwrap_or_else(|| "unknown profile".into());
    let conversation = item["conversation_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| ctx.conversation.as_ref().map(|id| id.0.clone()))
        .unwrap_or_else(|| "unknown conversation".into());
    let timing = if let Some(fire_at) = fire_at {
        format!("Proposed fire_at: {fire_at}.")
    } else if let Some(condition) = condition {
        format!("Proposed condition: {condition}.")
    } else {
        "No timing specified.".into()
    };
    format!(
        "Target person: {person}. Target profile: {profile}. Target conversation: {conversation}. {timing}"
    )
}
