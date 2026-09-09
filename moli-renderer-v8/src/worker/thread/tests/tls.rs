use super::super::WorkerGlobalKind;
use super::*;
use moli_websocket::test_support::TlsWebSocketFixture;

async fn tls_credentials_reach_worker(kind: WorkerGlobalKind) {
    ensure_v8();
    let fixture = TlsWebSocketFixture::new();
    let (url, server) = fixture.spawn(true).await;
    let tls = fixture.tls_config();
    let mut config = FetchConfig::default();
    config.set_http_proxy(Some(String::new()));
    config.set_tls_credentials(
        tls.ca_cert,
        tls.client_cert,
        tls.client_key,
        tls.client_cert_password,
    );
    let client = ResourceRequestClient::new(&config).expect("Worker TLS request client");
    let worker = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
                const socket = new WebSocket({url:?});
                socket.onopen = () => socket.send('worker-tls-echo');
                socket.onmessage = event => {{
                    if (event.data === 'worker-tls-echo') socket.close(1000, 'done');
                }};
                socket.onclose = () => close();
                socket.onerror = () => close();
            "#
            ),
            // A different origin from the WSS endpoint: WebSocket credentials
            // are included even though module/fetch defaults use same-origin.
            "https://example.com/worker.js".to_owned(),
        )
        .with_request_client(client)
        .with_global_kind(kind),
    );
    let peer = server.await.expect("Worker TLS fixture task");
    worker.terminate_and_join();
    assert_eq!(
        peer.expect("Worker mTLS handshake, echo and close"),
        std::slice::from_ref(&fixture.client_certificate)
    );
}

#[tokio::test]
async fn websocket_tls_credentials_reach_dedicated_worker() {
    tls_credentials_reach_worker(WorkerGlobalKind::Dedicated {
        name: "tls-worker".to_owned(),
    })
    .await;
}

#[tokio::test]
async fn websocket_tls_credentials_reach_shared_worker() {
    tls_credentials_reach_worker(WorkerGlobalKind::Shared {
        name: "tls-shared-worker".to_owned(),
        storage_key: moli_storage_key::MoliStorageKey::new(
            "https://example.com".to_owned(),
            "https://example.com".to_owned(),
            None,
            moli_storage_key::StoragePartitionRelation::FirstParty,
        ),
    })
    .await;
}
