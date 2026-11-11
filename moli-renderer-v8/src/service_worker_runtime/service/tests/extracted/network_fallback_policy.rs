use super::*;
use crate::page_task_queue::RendererResourceCompletionTestHarness;
use crate::service_worker_runtime::service::fetch_settlement::service_worker_network_fallback_request_for_job;
use crate::types::{AsyncSubresourceFetchEvent, SubresourceResourceType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FETCH_ID: u64 = 41;

async fn recording_server(
    content_type: &'static str,
) -> (
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
            let body = if content_type == "text/event-stream" {
                format!("data: network-{}\n\n", requests.len())
            } else {
                format!("network-{}", requests.len())
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: max-age=600\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    resource_type: SubresourceResourceType,
    destination: ServiceWorkerRequestDestination,
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
    request.destination = destination;
    if destination == ServiceWorkerRequestDestination::Document {
        request.request_mode = moli_fetch::RequestMode::Navigate;
        request.credentials_mode = moli_fetch::RequestCredentialsMode::Include;
    }
    if resource_type == SubresourceResourceType::EventSource {
        request.metadata.cache = "no-store".to_owned();
    }
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
                resource_type,
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
    let (base_url, stop, server) = recording_server("application/octet-stream").await;
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
                SubresourceResourceType::Font,
                ServiceWorkerRequestDestination::Font,
            );
            let result = if fallback {
                ServiceWorkerFetchResult::Fallback
            } else {
                ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                    head: moli_fetch_types::FetchResponseHead {
                        metadata: moli_fetch_types::FetchResponseMetadata {
                            response_type: moli_fetch_types::FetchResponseType::Default,
                            timing_allow_passed: true,
                            url_list: (None).into_iter().collect(),
                            redirected: false,
                        },
                        status: 200,
                        status_text: "OK".to_owned(),
                        headers: Vec::new(),
                    },
                    body: b"cached-response".to_vec(),
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
    let (base_url, stop, server) = recording_server("application/octet-stream").await;
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
        SubresourceResourceType::Font,
        ServiceWorkerRequestDestination::Font,
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
    let (base_url, stop, server) = recording_server("application/octet-stream").await;
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
        SubresourceResourceType::Font,
        ServiceWorkerRequestDestination::Font,
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

#[tokio::test(flavor = "current_thread")]
async fn event_source_network_fallback_preserves_no_store_with_cache_disabled() {
    let (base_url, stop, server) = recording_server("text/event-stream").await;
    let cache_dir = tempfile::tempdir().unwrap();
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_cache_dir(Some(cache_dir.path().display().to_string()));
    let client = ResourceRequestClient::new(&config).unwrap();
    client.set_cache_disabled(true);
    let (service, event, mut queue) = queued_controlled_fetch(
        client.frozen_request_client(),
        base_url.join("events").unwrap(),
        moli_fetch::RequestHeaders::default(),
        SubresourceResourceType::EventSource,
        ServiceWorkerRequestDestination::Empty,
    );
    let final_cache_mode = {
        let state = service.inner.state.lock();
        let job = state.pending_fetch_jobs.get(&event.event_id).unwrap();
        client
            .page_network_policy()
            .snapshot()
            .apply_to_request(service_worker_network_fallback_request_for_job(job))
            .unwrap()
            .cache_mode()
    };
    service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
        event_id: event.event_id,
        owner: event.owner,
        result: ServiceWorkerFetchResult::Fallback,
    });
    let body = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut started = false;
        let mut body = Vec::new();
        loop {
            while let Some(event) = queue.pop_next_async_subresource_event() {
                match event {
                    AsyncSubresourceFetchEvent::StreamingStarted(start) => {
                        assert_eq!(start.internal_id, FETCH_ID);
                        assert_eq!(start.head.status, 200);
                        assert!(!start.head.from_cache);
                        started = true;
                    }
                    AsyncSubresourceFetchEvent::StreamingChunk(chunk) => {
                        body.extend_from_slice(&chunk.bytes);
                    }
                    AsyncSubresourceFetchEvent::StreamingFinished(finish) => {
                        assert_eq!(finish.internal_id, FETCH_ID);
                        assert!(started, "EventSource must start before finishing");
                        finish.result.unwrap();
                        return body;
                    }
                    AsyncSubresourceFetchEvent::Completion(completion) => {
                        panic!("expected streaming EventSource fallback, got {completion:?}");
                    }
                    AsyncSubresourceFetchEvent::ObservedNetworkRecord(_) => {}
                }
            }
            assert!(queue.wait_for_arrival_without_timeout().await);
        }
    })
    .await
    .expect("EventSource fallback completion deadline");
    assert_eq!(body, b"data: network-1\n\n");
    let stats = moli_fetch::http_cache_stats(&config).unwrap();
    assert_eq!(
        stats.entry_count, 0,
        "EventSource must not enter HTTP cache"
    );
    assert_eq!(stats.unreadable_entry_count, 0);
    assert_eq!(final_cache_mode, moli_fetch::RequestCacheMode::NoStore);
    stop.send(()).unwrap();
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn direct_network_fallback_after_synthetic_redirect_enforces_page_policy() {
    let (base_url, stop, server) = recording_server("text/html").await;
    let target_url = base_url.join("../redirected").unwrap();
    for blocked in [true, false] {
        let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        if blocked {
            client.set_blocked_url_patterns(&[target_url.to_string()]);
        }
        let (service, event, mut queue) = queued_controlled_fetch(
            client.frozen_request_client(),
            base_url.join("initial").unwrap(),
            moli_fetch::RequestHeaders::default(),
            SubresourceResourceType::Document,
            ServiceWorkerRequestDestination::Document,
        );
        let (direct_tx, direct_rx) = tokio::sync::oneshot::channel();
        service
            .inner
            .state
            .lock()
            .pending_fetch_jobs
            .get_mut(&event.event_id)
            .unwrap()
            .direct_completion_tx = Some(direct_tx);
        service.finish_fetch_event_completed(ServiceWorkerFetchCompletion {
            event_id: event.event_id,
            owner: event.owner,
            result: ServiceWorkerFetchResult::Response(ServiceWorkerFetchResponse {
                head: moli_fetch_types::FetchResponseHead {
                    metadata: moli_fetch_types::FetchResponseMetadata {
                        response_type: moli_fetch_types::FetchResponseType::Basic,
                        timing_allow_passed: true,
                        url_list: (None).into_iter().collect(),
                        redirected: false,
                    },
                    status: 302,
                    status_text: "Found".to_owned(),
                    headers: vec![(
                        "location".to_owned(),
                        target_url.as_str().as_bytes().to_vec(),
                    )],
                },
                body: Vec::new(),
            }),
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(15), direct_rx)
            .await
            .expect("direct fallback completion deadline")
            .unwrap();
        if blocked {
            let ServiceWorkerDirectFetchResult::Failure(error) = result else {
                panic!("expected blocked direct fallback, got {result:?}");
            };
            assert!(error.contains(crate::network_host::BLOCKED_BY_CLIENT_ERROR_TEXT));
        } else {
            let ServiceWorkerDirectFetchResult::Response(response) = result else {
                panic!("expected direct network response, got {result:?}");
            };
            assert_eq!(response.response.final_url, target_url);
            assert_eq!(response.response.body_text(), "network-1");
            assert_eq!(response.response.redirect_chain.len(), 1);
        }
        assert!(!queue.has_ready_completion());
    }
    stop.send(()).unwrap();
    assert_eq!(server.await.unwrap().len(), 1);
}
