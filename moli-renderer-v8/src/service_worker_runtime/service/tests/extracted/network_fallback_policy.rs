use super::*;
use crate::page_task_queue::RendererResourceCompletionTestHarness;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FETCH_ID: u64 = 41;

async fn recording_server() -> (
    Url,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Vec<String>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = url(&format!("http://{}/app/", listener.local_addr().unwrap()));
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let (mut stream, _) = tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => accepted.unwrap(),
            };
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut chunk = [0_u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0, "request headers must finish");
                request.extend_from_slice(&chunk[..read]);
                assert!(request.len() < 65536, "bounded request headers");
            }
            requests.push(String::from_utf8(request).unwrap());
            let body = format!("network-{}", requests.len());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nCache-Control: max-age=600\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (base_url, stop_tx, server)
}

fn queued_controlled_fetch(
    request_client: ResourceRequestClient,
    request_url: Url,
    headers: moli_fetch::RequestHeaders,
) -> (
    ServiceWorkerRuntimeServiceOwner,
    ServiceWorkerFetchEvent,
    RendererResourceCompletionTestHarness,
) {
    let service = new_service_worker_runtime_service();
    let version_id = ServiceWorkerVersionId(1);
    let document_url = request_url.join("page.html").unwrap();
    insert_registered_version(
        &service,
        ServiceWorkerRegistrationId(1),
        version_id,
        document_url.join("worker.js").unwrap(),
        document_url.join("./").unwrap(),
        [document_url.clone()],
    );
    let client_id = client_id_for_document(&service, &document_url);
    let run = exact_version_run(&service, version_id);
    service
        .inner
        .state
        .lock()
        .versions
        .get_mut(&version_id)
        .unwrap()
        .running_state = ServiceWorkerVersionRunningState::Starting {
        host: new_loading_test_host(version_id, &run),
    };
    let queue = async_subresource_completion_queue();
    let mut request = test_fetch_request(client_id, request_url);
    request.destination = ServiceWorkerRequestDestination::Font;
    request.headers = headers.to_byte_strings();
    assert!(
        service.dispatch_controlled_fetch(ServiceWorkerFetchDispatch {
            internal_id: FETCH_ID,
            request,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report: None,
            network_context: AsyncSubresourceNetworkContext {
                frame_id: None,
                request_origin: (&document_url).into(),
                document_url,
                resource_type: crate::types::SubresourceResourceType::Font,
                policy_context: Default::default(),
            },
            completion_tx: queue.sender(),
            request_client,
            resource_task_runner: test_resource_task_runner(),
            cancel_handle: moli_fetch::FetchCancelHandle::new(),
            direct_completion_tx: None,
        })
    );
    let event = {
        let mut state = service.inner.state.lock();
        let version = state.versions.get_mut(&version_id).unwrap();
        let Some(ServiceWorkerPendingStartEvent::Fetch(event)) =
            version.pending_start_events.pop_front()
        else {
            panic!("controlled fetch must reach the ServiceWorker before network policy");
        };
        event
    };
    (service, event, queue)
}

async fn fetch_completion(
    queue: &mut RendererResourceCompletionTestHarness,
) -> AsyncSubresourceFetchCompletion {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            while let Some(event) = queue.pop_next_async_subresource_event() {
                if let crate::types::AsyncSubresourceFetchEvent::Completion(completion) = event {
                    assert_eq!(completion.internal_id, FETCH_ID);
                    return *completion;
                }
            }
            assert!(queue.wait_for_arrival_without_timeout().await);
        }
    })
    .await
    .expect("controlled fetch completion deadline")
}

#[tokio::test(flavor = "current_thread")]
async fn controlled_network_fallback_enforces_blocked_and_offline_policy() {
    let (base_url, stop, server) = recording_server().await;
    let request_url = base_url.join("font.woff2").unwrap();
    for offline in [false, true] {
        let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        if offline {
            client.set_network_offline(true);
        } else {
            client.set_blocked_url_patterns(&[request_url.to_string()]);
        }
        let frozen = client.frozen_request_client();
        for fallback in [false, true] {
            let (service, event, mut queue) = queued_controlled_fetch(
                frozen.clone(),
                request_url.clone(),
                moli_fetch::RequestHeaders::default(),
            );
            let result = if fallback {
                ServiceWorkerFetchResult::Fallback
            } else {
                ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                    status: 200,
                    status_text: "OK".to_owned(),
                    headers: Vec::new(),
                    body: b"cached-response".to_vec(),
                    final_url: None,
                    response_type: "default".to_owned(),
                    redirected: false,
                })
            };
            service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
                event_id: event.event_id,
                owner: event.owner,
                result,
            });
            let completion = fetch_completion(&mut queue).await;
            if fallback {
                let error = completion.result.into_result().unwrap_err();
                let expected = if offline {
                    "Network emulation offline"
                } else {
                    crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT
                };
                assert!(error.contains(expected), "{error}");
            } else {
                let response = completion.result.into_result().unwrap();
                assert_eq!(response.body_text(), "cached-response");
            }
        }
    }
    stop.send(()).unwrap();
    assert!(server.await.unwrap().is_empty(), "policy must prevent HTTP");
}

#[tokio::test(flavor = "current_thread")]
async fn controlled_network_fallback_honors_cache_disabled() {
    let (base_url, stop, server) = recording_server().await;
    let request_url = base_url.join("font.woff2").unwrap();
    let document_url = base_url.join("page.html").unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.path().display().to_string()));
    let client = ResourceRequestClient::new(&config).unwrap();
    let request = moli_fetch::Request::new_browser(
        "GET",
        request_url.clone(),
        None,
        moli_fetch::RequestHeaders::default(),
        (&document_url).into(),
    )
    .with_initiator_url(&document_url)
    .with_request_mode(moli_fetch::RequestMode::Cors)
    .with_credentials_mode(moli_fetch::RequestCredentialsMode::SameOrigin)
    .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Font)
    .with_page_network_policy();
    let first = client.fetch_raw(request.clone()).await.unwrap();
    assert_eq!(first.body_bytes(), b"network-1");
    let cached = client.fetch_raw(request).await.unwrap();
    assert!(cached.from_cache, "the HTTP cache must be populated");
    assert_eq!(cached.body_bytes(), b"network-1");

    client.set_cache_disabled(true);
    let (service, event, mut queue) = queued_controlled_fetch(
        client.frozen_request_client(),
        request_url,
        moli_fetch::RequestHeaders::default(),
    );
    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id: event.event_id,
        owner: event.owner,
        result: ServiceWorkerFetchResult::Fallback,
    });
    let response = fetch_completion(&mut queue)
        .await
        .result
        .into_result()
        .unwrap();
    assert_eq!(response.body_text(), "network-2");
    stop.send(()).unwrap();
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn controlled_network_fallback_keeps_frozen_headers_and_identity() {
    let (base_url, stop, server) = recording_server().await;
    let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let identity = |ua: &str, language: &str| {
        Some(Arc::new(moli_browser_profile::BrowserIdentityProfile::new(
            ua, language,
        )))
    };
    client
        .page_network_policy()
        .set_browser_identity_override(identity("Frozen/A", "fr-FR"));
    client.set_extra_http_headers(
        &vec![
            ("Accept".to_owned(), "application/page-old".to_owned()),
            ("X-Frozen".to_owned(), "old".to_owned()),
        ]
        .into(),
    );
    let headers = vec![
        ("aCcEpT".to_owned(), "application/captured".to_owned()),
        ("X-Empty".to_owned(), String::new()),
        ("X-Duplicate".to_owned(), "one".to_owned()),
        ("x-duplicate".to_owned(), "two".to_owned()),
    ];
    let (service, event, mut queue) = queued_controlled_fetch(
        client.frozen_request_client(),
        base_url.join("font.woff2").unwrap(),
        headers.into(),
    );
    client
        .page_network_policy()
        .set_browser_identity_override(identity("Changed/B", "de-DE"));
    client.set_extra_http_headers(
        &vec![
            ("Accept".to_owned(), "application/page-new".to_owned()),
            ("X-Frozen".to_owned(), "new".to_owned()),
            ("X-New".to_owned(), "later".to_owned()),
        ]
        .into(),
    );
    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id: event.event_id,
        owner: event.owner,
        result: ServiceWorkerFetchResult::Fallback,
    });
    assert!(fetch_completion(&mut queue).await.result.is_ok());
    stop.send(()).unwrap();
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    let header_values = |name: &str| {
        requests[0]
            .lines()
            .filter_map(|line| {
                let (field, value) = line.split_once(':')?;
                field.eq_ignore_ascii_case(name).then(|| value.trim())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(header_values("accept"), ["application/captured"]);
    assert_eq!(header_values("x-frozen"), ["old"]);
    assert_eq!(header_values("x-empty"), [""]);
    assert_eq!(header_values("x-duplicate"), ["one", "two"]);
    assert_eq!(header_values("user-agent"), ["Frozen/A"]);
    assert_eq!(header_values("accept-language"), ["fr-FR"]);
    assert!(header_values("x-new").is_empty());
}
