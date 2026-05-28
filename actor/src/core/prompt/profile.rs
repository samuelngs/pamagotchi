use super::format::pct;
use super::*;

pub(super) async fn fetch_current_profile_ctx(
    store: &Arc<dyn Store>,
    profile_id: Option<&ProfileId>,
) -> Option<CurrentProfileCtx> {
    let profile_id = profile_id?;
    let profile_info = store.get_profile(profile_id).await.ok().flatten();
    let profile_person_link = store
        .get_person_for_profile(profile_id)
        .await
        .ok()
        .flatten()
        .map(|(_, link)| link);

    Some(CurrentProfileCtx {
        ref_id: profile_id.0.clone(),
        display_name: profile_info
            .as_ref()
            .and_then(|profile| profile.display_name.clone()),
        summary: profile_info
            .as_ref()
            .and_then(|profile| profile.summary.clone()),
        comm_style: profile_info
            .as_ref()
            .and_then(|profile| profile.comm_style.clone()),
        person_ref_id: profile_person_link
            .as_ref()
            .map(|link| link.person_id.0.clone()),
        person_link_status: profile_person_link
            .as_ref()
            .map(|link| link.status.as_str().to_string()),
        person_link_confidence: profile_person_link
            .as_ref()
            .map(|link| pct(link.confidence)),
    })
}
