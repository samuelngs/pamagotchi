use super::*;
use protocol::{ClientRequest, SubscriptionTopic};

#[tokio::test]
async fn api_server_receives_requests_and_sends_events() {
    let (server, mut requests) = ApiServer::listen(0).await.unwrap();
    let handle = server.handle();
    let mut client = ApiClient::connect(server.port()).await.unwrap();

    client
        .send(ClientRequest::Subscribe {
            topics: vec![SubscriptionTopic::Chat],
        })
        .await
        .unwrap();

    let request = requests.recv().await.unwrap();
    assert!(matches!(
        request.request,
        ClientRequest::Subscribe { topics } if topics == vec![SubscriptionTopic::Chat]
    ));

    handle
        .send_to(
            request.client_id,
            ServerEvent::ChatMessage {
                content: "hello".into(),
                is_self: false,
            },
        )
        .await
        .unwrap();

    let event = client.recv().await.unwrap();
    assert_eq!(
        event,
        ServerEvent::ChatMessage {
            content: "hello".into(),
            is_self: false,
        }
    );
}
