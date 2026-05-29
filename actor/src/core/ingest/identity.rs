use crate::core::handle::StateHandle;
use crate::identity::{Identity, Person, PersonProfileStatus, Profile, ResolvedActorIdentity};
use crate::state::Authority;
use crate::store::{DisplayNameObservation, Store};
use protocol::{IdentityId, InboundMessage, PersonId, ProfileId};
use std::sync::Arc;
use tracing::{info, warn};

pub(super) async fn resolve_relay_person(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    msg: &mut InboundMessage,
) {
    if let Some(chosen_human_id) = find_chosen_human(state) {
        match resolve_or_create_identity_context(
            state,
            store,
            msg,
            Authority::ChosenHuman,
            Some(chosen_human_id.clone()),
        )
        .await
        {
            Some(ctx) => {
                msg.identity = Some(ctx.identity.id);
                msg.profile = Some(ctx.profile.id);
                msg.person = ctx.person.map(|person| person.id).or(Some(chosen_human_id));
            }
            None => {
                let _ = store.touch_person(&chosen_human_id).await;
                msg.person = Some(chosen_human_id);
            }
        }
    } else {
        let resolved =
            resolve_or_create_identity_context(state, store, msg, Authority::Default, None).await;
        if let Some(ctx) = resolved {
            let person_id = ctx.person.map(|person| person.id);
            if let Some(ref id) = person_id {
                info!(person = %id.0, "created adoption candidate from first relay contact");
            }
            msg.identity = Some(ctx.identity.id);
            msg.profile = Some(ctx.profile.id);
            msg.person = person_id;
        }
    }
}

pub(super) async fn resolve_gateway_person(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    msg: &mut InboundMessage,
) {
    let authority = if find_chosen_human(state).is_none() {
        Authority::Default
    } else {
        Authority::Default
    };
    if let Some(ctx) = resolve_or_create_identity_context(state, store, msg, authority, None).await
    {
        msg.identity = Some(ctx.identity.id);
        msg.profile = Some(ctx.profile.id);
        msg.person = ctx.person.map(|person| person.id);
    }
}

async fn resolve_or_create_identity_context(
    state: &StateHandle,
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    authority: Authority,
    attach_to: Option<PersonId>,
) -> Option<ResolvedActorIdentity> {
    let Some((gateway_id, sender_external_id)) = msg.sender_key() else {
        warn!("cannot resolve identity without gateway_id and sender_external_id");
        return None;
    };

    match store.resolve_identity(gateway_id, sender_external_id).await {
        Ok(Some(ctx)) => {
            let _ = store.touch_identity(&ctx.identity.id).await;
            let _ = store.touch_profile(&ctx.profile.id).await;
            observe_display_name(store, msg, &ctx).await;
            if let Some(person) = &ctx.person {
                let _ = store.touch_person(&person.id).await;
            }
            return Some(ctx);
        }
        Ok(None) => {}
        Err(e) => warn!("failed to resolve identity: {e}"),
    }

    let now = chrono::Utc::now().timestamp();
    let identity = Identity {
        id: IdentityId(format!("identity-{}", nanoid::nanoid!())),
        gateway_id: msg.gateway_id.clone(),
        external_id: msg.sender_external_id.clone(),
        display_name: msg.sender_display_name.clone(),
        metadata: Some(serde_json::json!({
            "reply_external_id": msg.reply_external_id,
            "source_metadata": msg.metadata,
        })),
        created_at: now,
        last_seen_at: now,
    };
    let profile = Profile {
        id: ProfileId(format!("profile-{}", nanoid::nanoid!())),
        display_name: msg.sender_display_name.clone(),
        summary: None,
        comm_style: None,
        first_seen: now,
        last_seen: now,
        created_at: now,
        updated_at: now,
    };
    let created_person = attach_to.is_none();
    let person_id = attach_to.unwrap_or_else(|| PersonId(format!("person-{}", nanoid::nanoid!())));
    if created_person {
        let person = Person {
            id: person_id.clone(),
            name: None,
            summary: None,
            comm_style: None,
            first_seen: now,
            last_seen: now,
        };
        if let Err(e) = store.add_person(&person).await {
            warn!("failed to create person: {e}");
            return None;
        }
    }
    if let Err(e) = store.add_identity(&identity).await {
        warn!("failed to create identity: {e}");
        return None;
    }
    if let Err(e) = store.add_profile(&profile).await {
        warn!("failed to create profile: {e}");
        return None;
    }
    let evidence = serde_json::json!({
        "reason": "first_seen_gateway_identity",
        "gateway_id": msg.gateway_id,
        "sender_external_id": msg.sender_external_id,
        "reply_external_id": msg.reply_external_id,
    });
    if let Err(e) = store
        .link_identity_to_profile(&identity.id, &profile.id, 1.0, Some(&evidence))
        .await
    {
        warn!("failed to link identity to profile: {e}");
        return None;
    }
    let link = match store
        .attach_profile_to_person(
            &profile.id,
            &person_id,
            PersonProfileStatus::Verified,
            1.0,
            Some(&evidence),
        )
        .await
    {
        Ok(link) => link,
        Err(e) => {
            warn!("failed to attach profile to person: {e}");
            return None;
        }
    };
    if created_person || authority == Authority::ChosenHuman {
        state
            .set_relationship_config(&person_id, Some(authority))
            .await;
    }
    record_display_name_observation(store, msg, &identity.id, Some(&profile.id)).await;
    let person = store.get_person(&person_id).await.ok().flatten();
    Some(ResolvedActorIdentity {
        identity,
        profile,
        person,
        profile_person_link: Some(link),
    })
}

async fn observe_display_name(
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    ctx: &ResolvedActorIdentity,
) {
    let Some(display_name) = msg
        .sender_display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };

    record_display_name_observation(store, msg, &ctx.identity.id, Some(&ctx.profile.id)).await;

    if ctx.identity.display_name.as_deref() != Some(display_name) {
        if let Err(e) = store
            .update_identity_display_name(&ctx.identity.id, display_name)
            .await
        {
            warn!(%e, identity = %ctx.identity.id.0, "failed to update observed identity display name");
        }
    }

    let identity_display_name = ctx.identity.display_name.as_deref().map(str::trim);
    let profile_display_name = ctx.profile.display_name.as_deref().map(str::trim);
    let profile_display_is_empty = profile_display_name.unwrap_or("").is_empty();
    let profile_display_was_auto_observed = profile_display_name.is_some()
        && identity_display_name.is_some()
        && profile_display_name == identity_display_name;

    if (profile_display_is_empty || profile_display_was_auto_observed)
        && profile_display_name != Some(display_name)
    {
        if let Err(e) = store
            .update_profile(&ctx.profile.id, Some(display_name), None)
            .await
        {
            warn!(%e, profile = %ctx.profile.id.0, "failed to update observed profile display name");
        }
    }
}

async fn record_display_name_observation(
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    identity: &IdentityId,
    profile: Option<&ProfileId>,
) {
    let Some(display_name) = msg
        .sender_display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let observation = DisplayNameObservation {
        identity: identity.clone(),
        profile: profile.cloned(),
        gateway_id: msg.gateway_id.clone(),
        external_id: msg.sender_external_id.clone(),
        display_name: display_name.to_string(),
        source_message_id: Some(msg.message_id.clone()).filter(|id| !id.is_empty()),
        observed_at: msg.timestamp,
    };
    if let Err(e) = store.record_display_name_observation(&observation).await {
        warn!(
            %e,
            identity = %identity.0,
            "failed to record display name observation"
        );
    }
}

fn find_chosen_human(state: &StateHandle) -> Option<PersonId> {
    let actor = state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, rel)| rel.authority == Authority::ChosenHuman)
        .map(|(id, _)| id.clone())
}
