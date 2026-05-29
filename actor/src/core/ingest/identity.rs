use crate::core::handle::StateHandle;
use crate::identity::{Identity, Person, PersonProfileStatus, Profile, ResolvedActorIdentity};
use crate::state::RelationshipStanding;
use crate::store::{
    DisplayNameObservation, IdentityConflictIdentity, IdentityConflictRecord, Store,
};
use protocol::{
    ChannelId, GatewayId, IdentityId, InboundEnvelope, InboundMessage, ObservedIdentityKey,
    PersonId, ProfileId, channel_id, identity_id,
};
use std::collections::{HashMap, HashSet};
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
            RelationshipStanding::ChosenHuman,
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
        let resolved = resolve_or_create_identity_context(
            state,
            store,
            msg,
            RelationshipStanding::Default,
            None,
        )
        .await;
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
    let relationship_standing = if find_chosen_human(state).is_none() {
        RelationshipStanding::Default
    } else {
        RelationshipStanding::Default
    };
    if let Some(ctx) =
        resolve_or_create_identity_context(state, store, msg, relationship_standing, None).await
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
    relationship_standing: RelationshipStanding,
    attach_to: Option<PersonId>,
) -> Option<ResolvedActorIdentity> {
    let Some(mut observation) = SenderObservation::from_message(msg) else {
        warn!("cannot resolve identity without observed sender keys");
        return None;
    };
    let keys = observation.normalized_keys(msg);
    if keys.is_empty() {
        warn!("cannot resolve identity because all observed sender keys were empty or invalid");
        return None;
    }
    observation.primary = keys[0].clone();

    let mut resolved = Vec::new();
    for key in &keys {
        match store
            .resolve_identity(key.gateway_id.as_str(), key.external_id.as_str())
            .await
        {
            Ok(Some(ctx)) => resolved.push((key.clone(), ctx)),
            Ok(None) => {}
            Err(e) => warn!(
                %e,
                gateway_id = %key.gateway_id.0,
                external_id = %key.external_id,
                "failed to resolve observed identity"
            ),
        }
    }

    let now = chrono::Utc::now().timestamp();
    let evidence = alias_evidence(msg, &observation, &keys);
    let profile_contexts = contexts_by_profile(&resolved);

    if profile_contexts.len() > 1 {
        let identities = ensure_observed_identities(store, msg, &observation, &keys, now).await?;
        record_identity_conflict(
            store,
            &observation,
            &keys,
            &identities,
            &profile_contexts,
            now,
        )
        .await;
        if let Some(ctx) = resolved
            .iter()
            .find(|(key, _)| same_key(key, &observation.primary))
            .map(|(_, ctx)| ctx.clone())
        {
            touch_resolved_context(store, &ctx).await;
            observe_display_name(store, msg, &ctx).await;
            return Some(ctx);
        }
        return None;
    }

    if let Some(ctx) = profile_contexts.values().next().cloned() {
        touch_resolved_context(store, &ctx).await;
        observe_display_name(store, msg, &ctx).await;
        let identities = ensure_observed_identities(store, msg, &observation, &keys, now).await?;
        for identity in identities.values() {
            if let Err(e) = store
                .link_identity_to_profile(identity, &ctx.profile.id, 1.0, Some(&evidence))
                .await
            {
                warn!(
                    %e,
                    identity = %identity.0,
                    profile = %ctx.profile.id.0,
                    "failed to link observed sender alias to existing profile"
                );
                return None;
            }
        }
        if let Some(person_id) = attach_to.as_ref().filter(|_| ctx.person.is_none()) {
            if let Err(e) = store
                .attach_profile_to_person(
                    &ctx.profile.id,
                    person_id,
                    PersonProfileStatus::Verified,
                    1.0,
                    Some(&evidence),
                )
                .await
            {
                warn!(%e, profile = %ctx.profile.id.0, person = %person_id.0, "failed to attach resolved profile to chosen human");
                return None;
            }
        }
        return resolve_selected_primary_context(store, msg, &observation)
            .await
            .or_else(|| Some(ctx));
    }

    let profile = Profile {
        id: ProfileId(format!("profile-{}", nanoid::nanoid!())),
        display_name: observation.display_name.clone(),
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
    if let Err(e) = store.add_profile(&profile).await {
        warn!("failed to create profile: {e}");
        return None;
    }
    let identities = ensure_observed_identities(store, msg, &observation, &keys, now).await?;
    for identity in identities.values() {
        if let Err(e) = store
            .link_identity_to_profile(identity, &profile.id, 1.0, Some(&evidence))
            .await
        {
            warn!("failed to link identity to profile: {e}");
            return None;
        }
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
    if created_person || relationship_standing == RelationshipStanding::ChosenHuman {
        state
            .set_relationship_config(&person_id, Some(relationship_standing))
            .await;
    }
    let person = store.get_person(&person_id).await.ok().flatten();
    let ctx = resolve_selected_primary_context(store, msg, &observation)
        .await
        .unwrap_or_else(|| {
            let primary_id = identity_id(
                &observation.primary.gateway_id,
                observation.primary.external_id.as_str(),
            );
            ResolvedActorIdentity {
                identity: Identity {
                    id: primary_id,
                    gateway_id: observation.primary.gateway_id.0.clone(),
                    external_id: observation.primary.external_id.clone(),
                    display_name: observation.display_name.clone(),
                    metadata: Some(identity_metadata(msg, &observation.primary)),
                    created_at: now,
                    last_seen_at: now,
                },
                profile: profile.clone(),
                person,
                profile_person_link: Some(link.clone()),
            }
        });
    record_display_name_observation(store, msg, &ctx.identity, Some(&profile.id)).await;
    Some(ctx)
}

#[derive(Clone)]
struct SenderObservation {
    primary: ObservedIdentityKey,
    aliases: Vec<ObservedIdentityKey>,
    display_name: Option<String>,
    channel: Option<ChannelId>,
    platform_message_id: Option<String>,
}

impl SenderObservation {
    fn from_message(msg: &InboundMessage) -> Option<Self> {
        if let Some(value) = msg.metadata.get("normalized_envelope") {
            match serde_json::from_value::<InboundEnvelope>(value.clone()) {
                Ok(envelope) => {
                    if let Some(sender) = envelope.sender {
                        return Some(Self {
                            primary: sender.primary,
                            aliases: sender.aliases,
                            display_name: sender
                                .display_name
                                .or_else(|| msg.sender_display_name().map(str::to_string)),
                            channel: Some(channel_id(
                                &envelope.channel.gateway_id,
                                envelope.channel.external_id.as_str(),
                            )),
                            platform_message_id: Some(envelope.platform_message_id),
                        });
                    }
                    return None;
                }
                Err(e) => {
                    warn!(%e, "failed to parse normalized inbound envelope for identity resolution")
                }
            }
        }

        let (gateway_id, sender_external_id) = msg.sender_key()?;
        let gateway = GatewayId(gateway_id.to_string());
        let channel = (!msg.channel.external_id.trim().is_empty())
            .then(|| channel_id(&gateway, msg.channel.external_id.as_str()));
        Some(Self {
            primary: ObservedIdentityKey {
                gateway_id: gateway,
                external_id: sender_external_id.to_string(),
                kind: None,
                confidence: 1.0,
                source: "legacy_sender".into(),
            },
            aliases: Vec::new(),
            display_name: msg.sender_display_name().map(str::to_string),
            channel,
            platform_message_id: Some(msg.message_id.clone()).filter(|id| !id.is_empty()),
        })
    }

    fn normalized_keys(&self, msg: &InboundMessage) -> Vec<ObservedIdentityKey> {
        let mut seen = HashSet::new();
        let mut keys = Vec::new();
        for mut key in std::iter::once(self.primary.clone()).chain(self.aliases.clone()) {
            key.gateway_id.0 = key.gateway_id.0.trim().to_string();
            key.external_id = key.external_id.trim().to_string();
            key.source = key.source.trim().to_string();
            if key.gateway_id.0.is_empty() || key.external_id.is_empty() {
                continue;
            }
            if matches!(msg.channel.kind, protocol::ChannelKind::GroupChat)
                && key.external_id == msg.channel.external_id
            {
                warn!(
                    gateway_id = %key.gateway_id.0,
                    external_id = %key.external_id,
                    "ignored observed sender key that matched the platform group/channel id"
                );
                continue;
            }
            let dedupe = (key.gateway_id.0.clone(), key.external_id.clone());
            if seen.insert(dedupe) {
                keys.push(key);
            }
        }
        keys
    }
}

fn contexts_by_profile(
    resolved: &[(ObservedIdentityKey, ResolvedActorIdentity)],
) -> HashMap<ProfileId, ResolvedActorIdentity> {
    let mut contexts = HashMap::new();
    for (_, ctx) in resolved {
        contexts
            .entry(ctx.profile.id.clone())
            .or_insert_with(|| ctx.clone());
    }
    contexts
}

async fn ensure_observed_identities(
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    observation: &SenderObservation,
    keys: &[ObservedIdentityKey],
    now: i64,
) -> Option<HashMap<(String, String), IdentityId>> {
    let mut identities = HashMap::new();
    for key in keys {
        let identity = Identity {
            id: identity_id(&key.gateway_id, key.external_id.as_str()),
            gateway_id: key.gateway_id.0.clone(),
            external_id: key.external_id.clone(),
            display_name: observation.display_name.clone(),
            metadata: Some(identity_metadata(msg, key)),
            created_at: now,
            last_seen_at: now,
        };
        match store.add_identity(&identity).await {
            Ok(id) => {
                identities.insert((key.gateway_id.0.clone(), key.external_id.clone()), id);
            }
            Err(e) => {
                warn!(
                    %e,
                    gateway_id = %key.gateway_id.0,
                    external_id = %key.external_id,
                    "failed to create or update observed identity"
                );
                return None;
            }
        }
    }
    Some(identities)
}

async fn resolve_selected_primary_context(
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    observation: &SenderObservation,
) -> Option<ResolvedActorIdentity> {
    match store
        .resolve_identity(
            observation.primary.gateway_id.as_str(),
            observation.primary.external_id.as_str(),
        )
        .await
    {
        Ok(Some(ctx)) => {
            touch_resolved_context(store, &ctx).await;
            observe_display_name(store, msg, &ctx).await;
            Some(ctx)
        }
        Ok(None) => None,
        Err(e) => {
            warn!(
                %e,
                gateway_id = %observation.primary.gateway_id.0,
                external_id = %observation.primary.external_id,
                "failed to reload selected sender identity"
            );
            None
        }
    }
}

async fn touch_resolved_context(store: &Arc<dyn Store>, ctx: &ResolvedActorIdentity) {
    let _ = store.touch_identity(&ctx.identity.id).await;
    let _ = store.touch_profile(&ctx.profile.id).await;
    if let Some(person) = &ctx.person {
        let _ = store.touch_person(&person.id).await;
    }
}

async fn record_identity_conflict(
    store: &Arc<dyn Store>,
    observation: &SenderObservation,
    keys: &[ObservedIdentityKey],
    identities: &HashMap<(String, String), IdentityId>,
    contexts: &HashMap<ProfileId, ResolvedActorIdentity>,
    now: i64,
) {
    let Some(primary_identity) = identities.get(&(
        observation.primary.gateway_id.0.clone(),
        observation.primary.external_id.clone(),
    )) else {
        return;
    };
    let conflict = IdentityConflictRecord {
        id: format!("identity-conflict-{}", nanoid::nanoid!()),
        channel: observation.channel.clone(),
        platform_message_id: observation.platform_message_id.clone(),
        primary_identity: Some(primary_identity.clone()),
        reason: "same_gateway_message_sender_aliases".into(),
        status: "open".into(),
        created_at: now,
        resolved_at: None,
        resolution: serde_json::json!({}),
        identities: keys
            .iter()
            .filter_map(|key| {
                identities
                    .get(&(key.gateway_id.0.clone(), key.external_id.clone()))
                    .map(|identity| IdentityConflictIdentity {
                        identity: identity.clone(),
                        role: if same_key(key, &observation.primary) {
                            "primary".into()
                        } else {
                            "alias".into()
                        },
                        source: Some(key.source.clone()).filter(|source| !source.is_empty()),
                    })
            })
            .collect(),
        profiles: contexts.keys().cloned().collect(),
    };
    if let Err(e) = store.record_identity_conflict(&conflict).await {
        warn!(%e, "failed to record observed identity conflict");
    }
}

fn same_key(left: &ObservedIdentityKey, right: &ObservedIdentityKey) -> bool {
    left.gateway_id == right.gateway_id && left.external_id == right.external_id
}

fn identity_metadata(msg: &InboundMessage, key: &ObservedIdentityKey) -> serde_json::Value {
    serde_json::json!({
        "observed_key": {
            "gateway_id": key.gateway_id.0.clone(),
            "external_id": key.external_id.clone(),
            "kind": key.kind.clone(),
            "confidence": key.confidence,
            "source": key.source.clone(),
        },
        "channel": &msg.channel,
        "source_metadata": &msg.metadata,
    })
}

fn alias_evidence(
    msg: &InboundMessage,
    observation: &SenderObservation,
    keys: &[ObservedIdentityKey],
) -> serde_json::Value {
    serde_json::json!({
        "reason": "same_gateway_message_sender_aliases",
        "platform_message_id": observation.platform_message_id.as_deref().unwrap_or(msg.message_id.as_str()),
        "channel_id": observation.channel.as_ref().map(|channel| channel.0.clone()),
        "observed_keys": keys
            .iter()
            .map(|key| {
                serde_json::json!({
                    "gateway_id": key.gateway_id.0.clone(),
                    "external_id": key.external_id.clone(),
                    "kind": key.kind.clone(),
                    "confidence": key.confidence,
                    "source": key.source.clone(),
                })
            })
            .collect::<Vec<_>>(),
    })
}

async fn observe_display_name(
    store: &Arc<dyn Store>,
    msg: &InboundMessage,
    ctx: &ResolvedActorIdentity,
) {
    let Some(display_name) = msg
        .sender_display_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };

    record_display_name_observation(store, msg, &ctx.identity, Some(&ctx.profile.id)).await;

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
    identity: &Identity,
    profile: Option<&ProfileId>,
) {
    let Some(display_name) = msg
        .sender_display_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return;
    };
    let observation = DisplayNameObservation {
        identity: identity.id.clone(),
        profile: profile.cloned(),
        gateway_id: identity.gateway_id.clone(),
        external_id: identity.external_id.clone(),
        display_name: display_name.to_string(),
        source_message_id: Some(msg.message_id.clone()).filter(|id| !id.is_empty()),
        observed_at: msg.timestamp,
    };
    if let Err(e) = store.record_display_name_observation(&observation).await {
        warn!(
            %e,
            identity = %identity.id.0,
            "failed to record display name observation"
        );
    }
}

fn find_chosen_human(state: &StateHandle) -> Option<PersonId> {
    let actor = state.read_state();
    actor
        .bonds
        .iter()
        .find(|(_, rel)| rel.relationship_standing == RelationshipStanding::ChosenHuman)
        .map(|(id, _)| id.clone())
}
