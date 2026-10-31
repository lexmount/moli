use super::*;
use moli_websocket::{Event, spawn_connection, test_support::test_websocket_context};

#[tokio::test]
async fn page_and_worker_websocket_connectors_follow_resource_runtime_owner() -> Result<()> {
    // Opening handshakes to one endpoint serialize. Use distinct ports so both
    // page and worker have a live pending socket when their shared owner drops.
    let listeners = [
        TcpListener::bind("127.0.0.1:0").await?,
        TcpListener::bind("127.0.0.1:0").await?,
    ];
    let owner = ResourceRequestClient::new(&FetchConfig::default())?;
    let page = owner.fork_with_isolated_page_network_policy();
    let worker = page.fork_with_isolated_worker_network_policy();
    let mut connections = Vec::new();
    let mut peers = Vec::new();
    for (index, client) in [&page, &worker].into_iter().enumerate() {
        let listener = &listeners[index];
        let url = format!("ws://{}/pending", listener.local_addr()?);
        let context = test_websocket_context();
        let (events, incoming) = tokio::sync::mpsc::channel(8);
        let connection = spawn_connection(
            client.websocket_connector(),
            3000 + index as u64,
            url,
            Vec::new(),
            context,
            events,
        );
        let (mut peer, _) = timeout(Duration::from_secs(3), listener.accept()).await??;
        timeout(Duration::from_secs(3), async {
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(peer.read_u8().await?);
            }
            anyhow::Ok(())
        })
        .await??;
        connections.push((connection, incoming));
        peers.push(peer);
    }
    drop(owner);
    for mut peer in peers {
        assert_eq!(
            timeout(Duration::from_secs(3), peer.read(&mut [0])).await??,
            0
        );
    }
    for (connection, mut events) in connections {
        assert!(matches!(
            timeout(Duration::from_secs(3), events.recv()).await?,
            Some(Event::Error { .. })
        ));
        assert!(matches!(
            timeout(Duration::from_secs(3), events.recv()).await?,
            Some(Event::Close { code: 1006, .. })
        ));
        assert!(connection.is_closed());
    }
    Ok(())
}
