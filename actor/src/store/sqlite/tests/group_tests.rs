use super::*;

#[tokio::test]
async fn groups() {
    let store = test_store();
    store.add_person(&sample_person("p1", "Sam")).await.unwrap();
    store.add_person(&sample_person("p2", "Mom")).await.unwrap();

    store
        .add_group(&Group {
            id: GroupId("g1".into()),
            name: "Family Chat".into(),
            gateway_id: "discord".into(),
            external_id: "discord-family".into(),
            context: GroupContext::Family,
            members: vec![PersonId("p1".into()), PersonId("p2".into())],
        })
        .await
        .unwrap();

    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.name, "Family Chat");
    assert_eq!(group.members.len(), 2);

    store
        .add_person(&sample_person("p3", "Sister"))
        .await
        .unwrap();
    store
        .add_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
        .await
        .unwrap();

    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.members.len(), 3);

    store
        .remove_group_member(&GroupId("g1".into()), &PersonId("p3".into()))
        .await
        .unwrap();
    let group = store
        .get_group(&GroupId("g1".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.members.len(), 2);

    let groups = store.debug_groups(10).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, GroupId("g1".into()));
    assert!(groups[0].members.contains(&PersonId("p1".into())));
    assert!(groups[0].members.contains(&PersonId("p2".into())));
}
