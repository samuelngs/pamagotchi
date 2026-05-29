use super::*;

#[tokio::test]
async fn behavior_directives() {
    let store = test_store();
    let sam = PersonId("sam".into());
    let mom = PersonId("mom".into());
    let channel = ChannelId("channel-family".into());

    store
        .add_directive(&BehaviorDirective {
            id: "d1".into(),
            scope: DirectiveScope::Global,
            directive: "Never share private info between persons".into(),
            set_by: sam.clone(),
            priority: 0,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .add_directive(&BehaviorDirective {
            id: "d2".into(),
            scope: DirectiveScope::Person(mom.clone()),
            directive: "Be polite, no crude humor".into(),
            set_by: sam.clone(),
            priority: 10,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .add_directive(&BehaviorDirective {
            id: "d3".into(),
            scope: DirectiveScope::RelationshipStanding(RelationshipStanding::Default),
            directive: "Be warm and respectful".into(),
            set_by: sam.clone(),
            priority: 5,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    store
        .add_directive(&BehaviorDirective {
            id: "d4".into(),
            scope: DirectiveScope::Channel(channel.clone()),
            directive: "Use the family chat norm".into(),
            set_by: sam.clone(),
            priority: 8,
            active: true,
            created_at: 1000,
            expires_at: None,
        })
        .await
        .unwrap();

    let directives = store
        .get_directives_for_context(&mom, &RelationshipStanding::Default, None)
        .await
        .unwrap();
    assert_eq!(directives.len(), 3);
    assert_eq!(directives[0].id, "d2");
    assert_eq!(directives[1].id, "d3");
    assert_eq!(directives[2].id, "d1");

    let directives = store
        .get_directives_for_context(&mom, &RelationshipStanding::Default, Some(&channel))
        .await
        .unwrap();
    assert_eq!(directives.len(), 4);
    assert_eq!(directives[1].id, "d4");

    store
        .update_directive("d2", None, Some(false), None, None)
        .await
        .unwrap();
    let directives = store
        .get_directives_for_context(&mom, &RelationshipStanding::Default, None)
        .await
        .unwrap();
    assert_eq!(directives.len(), 2);

    assert!(store.remove_directive("d1").await.unwrap());
    assert!(!store.remove_directive("nonexistent").await.unwrap());

    let all = store.list_directives().await.unwrap();
    assert_eq!(all.len(), 3);
}
