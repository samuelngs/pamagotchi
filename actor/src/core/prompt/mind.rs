use super::conversation::{
    conversation_ctx_from_summary, fetch_conversation_summary, fetch_group_ctx,
};
use super::format::format_now;
use super::identity::recall_identity_name;
use super::person::resolve_person_for_mind;
use super::profile::fetch_current_profile_ctx;
use super::*;

pub(super) async fn build_mind(
    env: &Environment<'_>,
    state: &StateHandle,
    store: &Arc<dyn Store>,
    messages: &[InboundMessage],
    session_ctx: &SessionContext,
    concurrent_summaries: &[(String, String, String)],
) -> anyhow::Result<String> {
    let identity = recall_identity_name(store).await;
    let now = format_now();
    let now_ts = chrono::Utc::now();
    let authority =
        if let Some(person) = messages.first().and_then(|message| message.person.as_ref()) {
            let actor = state.read_state();
            actor
                .bonds
                .get(person)
                .map(|rel| rel.authority.clone())
                .unwrap_or(Authority::Default)
        } else {
            Authority::Default
        };
    let person = resolve_person_for_mind(state, store, messages).await;
    let first_message = messages.first();
    let profile = fetch_current_profile_ctx(
        store,
        first_message.and_then(|message| message.profile.as_ref()),
    )
    .await;
    let conversation_id = first_message.map(|message| &message.conversation);
    let conversation_summary = fetch_conversation_summary(store, conversation_id).await;
    let conversation = conversation_summary
        .as_ref()
        .map(conversation_ctx_from_summary);
    let group_id = first_message
        .and_then(|message| message.group.as_ref())
        .or_else(|| {
            conversation_summary
                .as_ref()
                .and_then(|summary| summary.group.as_ref())
        });
    let group = fetch_group_ctx(store, group_id).await;
    let timing = social_read::build_timing_ctx(
        store,
        messages,
        conversation_id,
        first_message.and_then(|message| message.person.as_ref()),
        state,
        session_ctx,
        now_ts,
    )
    .await;
    let safety = social_read::build_safety_ctx(&authority, &SessionKind::Mind);
    let recent_messages =
        social_read::fetch_recent_messages(store, conversation_id, messages).await;
    let social_relations = social_read::fetch_social_relations(
        store,
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let relationship_memories = social_read::fetch_relationship_memories(
        store,
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let relevant_memories = social_read::fetch_relevant_memories(
        store,
        messages,
        &[],
        first_message.and_then(|message| message.identity.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;
    let open_loops = social_read::fetch_open_loops(
        store,
        first_message.and_then(|message| message.person.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.map(|message| &message.conversation),
        chrono::Utc::now().timestamp(),
    )
    .await;
    let actions: Vec<ActionBriefCtx> = concurrent_summaries
        .iter()
        .map(|(id, kind, task)| ActionBriefCtx {
            id: id.clone(),
            kind: kind.clone(),
            task: task.clone(),
        })
        .collect();
    let thoughts = social_read::fetch_thoughts(
        store,
        first_message.and_then(|message| message.identity.as_ref()),
        first_message.and_then(|message| message.profile.as_ref()),
        first_message.and_then(|message| message.person.as_ref()),
    )
    .await;

    let ctx = MindContext {
        identity,
        now,
        profile,
        person,
        conversation,
        group,
        recent_messages,
        actions,
        timing,
        safety,
        social_relations,
        relationship_memories,
        relevant_memories,
        open_loops,
        thoughts,
    };
    let tmpl = env.get_template("mind.j2")?;
    Ok(tmpl.render(&ctx)?)
}
