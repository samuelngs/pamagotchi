use super::*;

pub(super) async fn apply_directives(
    args: &Value,
    ctx: &SessionContext,
    state: &mut SessionState,
    counts: &mut ApplyCounts,
) {
    let mut existing_ids = match ctx.store.list_directives().await {
        Ok(directives) => directives
            .into_iter()
            .map(|directive| directive.id)
            .collect::<HashSet<_>>(),
        Err(e) => {
            if args["directives"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                counts.skipped.push(format!(
                    "directives could not load existing directives: {e}"
                ));
            }
            return;
        }
    };

    for (idx, item) in array_items(&args["directives"]).enumerate() {
        let Some(text) = trimmed_text(item["directive"].as_str(), 600) else {
            counts.skipped.push(format!("directive {idx} missing text"));
            continue;
        };
        let Some(scope) = directive_scope(item, ctx).await else {
            counts
                .skipped
                .push(format!("directive {idx} has unsupported or missing scope"));
            continue;
        };
        if !directive_scope_allowed(&scope, ctx).await {
            counts.skipped.push(format!(
                "directive {idx} targets a scope outside review context"
            ));
            continue;
        }
        let Some(set_by) = directive_set_by(item, ctx) else {
            counts.skipped.push(format!(
                "directive {idx} has no current person to attribute"
            ));
            continue;
        };

        let id = trimmed_text(item["id"].as_str(), 128)
            .or_else(|| trimmed_text(item["dedupe_key"].as_str(), 128))
            .unwrap_or_else(|| directive_id(&scope, &text));
        if !state.applied_review_keys.insert(format!("directive:{id}")) {
            counts.skipped.push(format!("directive {idx} duplicate"));
            continue;
        }
        if existing_ids.contains(&id) {
            counts
                .skipped
                .push(format!("directive {idx} already exists"));
            continue;
        }

        let priority = item["priority"].as_i64().unwrap_or(0).clamp(-100, 100) as i32;
        let directive = BehaviorDirective {
            id: id.clone(),
            scope,
            directive: text,
            set_by,
            priority,
            active: item["active"].as_bool().unwrap_or(true),
            created_at: util::now(),
            expires_at: item["expires_at"].as_i64(),
        };
        match ctx.store.add_directive(&directive).await {
            Ok(()) => {
                existing_ids.insert(id);
                counts.directives += 1;
            }
            Err(e) => counts.skipped.push(format!("directive {idx} failed: {e}")),
        }
    }
}

async fn directive_scope(item: &Value, ctx: &SessionContext) -> Option<DirectiveScope> {
    match item["scope"].as_str()? {
        "global" => Some(DirectiveScope::Global),
        "relationship_standing" => item["relationship_standing"]
            .as_str()
            .and_then(RelationshipStanding::parse)
            .map(DirectiveScope::RelationshipStanding),
        "person" => item["person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| DirectiveScope::Person(PersonId(id.to_string())))
            .or_else(|| current_review_person(ctx).map(DirectiveScope::Person)),
        "group" => {
            if let Some(id) = item["group_id"].as_str().filter(|id| !id.trim().is_empty()) {
                Some(DirectiveScope::Group(GroupId(id.to_string())))
            } else {
                current_review_group(ctx).await.map(DirectiveScope::Group)
            }
        }
        _ => None,
    }
}

async fn directive_scope_allowed(scope: &DirectiveScope, ctx: &SessionContext) -> bool {
    if matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman) {
        return true;
    }

    match scope {
        DirectiveScope::Person(person) => current_review_person(ctx).as_ref() == Some(person),
        DirectiveScope::Group(group) => current_review_group(ctx).await.as_ref() == Some(group),
        DirectiveScope::Global | DirectiveScope::RelationshipStanding(_) => false,
    }
}

fn directive_set_by(item: &Value, ctx: &SessionContext) -> Option<PersonId> {
    if matches!(ctx.relationship_standing, RelationshipStanding::ChosenHuman) {
        return item["set_by_person_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty())
            .map(|id| PersonId(id.to_string()))
            .or_else(|| current_review_person(ctx));
    }
    current_review_person(ctx)
}

fn current_review_person(ctx: &SessionContext) -> Option<PersonId> {
    ctx.messages
        .iter()
        .find_map(|message| message.person.clone())
}

async fn current_review_group(ctx: &SessionContext) -> Option<GroupId> {
    if let Some(group) = ctx
        .messages
        .iter()
        .find_map(|message| message.group.clone())
    {
        return Some(group);
    }
    let conversation = ctx.conversation.as_ref()?;
    ctx.store
        .list_conversations()
        .await
        .ok()?
        .into_iter()
        .find(|summary| summary.id == *conversation)
        .and_then(|summary| summary.group)
}

fn directive_id(scope: &DirectiveScope, directive: &str) -> String {
    let scope_type = scope.scope_type();
    let scope_value = scope.scope_value().unwrap_or_default();
    format!(
        "directive-{}",
        stable_hash(&format!("{scope_type}:{scope_value}:{directive}"))
    )
}
