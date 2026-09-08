use super::*;
use crate::browser::{
    BrowserContextStoragePartitionHandles, BrowserService, DocumentHandle, DocumentRetirement,
    StoragePartitionKind, WebContentsCreation,
};
use moli_test_support::FixtureServer;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

fn context_with_contents(service: &BrowserService) -> (BrowserContextHandle, WebContentsHandle) {
    let context = service
        .handle()
        .create_context(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        )
        .unwrap();
    context.bind_page_navigation_engines(Default::default(), None);
    let (contents, _) = context
        .create_web_contents(WebContentsCreation::default())
        .unwrap();
    (context, contents)
}

fn start_load(
    context: &BrowserContextHandle,
    contents: WebContentsHandle,
) -> BrowserNavigationLoad {
    let navigation = context.start_document_navigation(contents).unwrap();
    context
        .start_navigation_load(
            contents,
            navigation,
            NavigationRequestLoadPolicy::BrowserInitiated,
            context.inherited_document_policy(Default::default(), &[], None),
        )
        .unwrap()
}

// Uses only public Browser capabilities: no Page access, DevTools connection,
// renderer inspection binding or protocol lifecycle projection.
async fn navigate(
    context: &BrowserContextHandle,
    contents: WebContentsHandle,
    url: &str,
) -> DocumentHandle {
    let mut load = start_load(context, contents);
    let navigation = load.navigation_id();
    let fetched = load
        .fetch_navigation("GET", url, None, Vec::new())
        .await
        .unwrap();
    let response = fetched
        .fetch_result
        .into_parts_with_observation_journal()
        .0
        .into_materialized_raw_response()
        .await
        .unwrap();
    let destination = DocumentNavigationDestination {
        url: response.final_url.clone(),
        security_origin: response.final_url.origin().ascii_serialization(),
        secure_context_type: "SecureLocalhost".to_owned(),
    };
    let prepared = load
        .prepare_document_response_async(
            Url::parse(url).unwrap(),
            response.final_url.clone(),
            response.redirected,
            response.redirect_chain.len(),
            response.status,
            response.headers.clone(),
            ExternalRawDocumentBodyStream::from_bytes(response.clone_body_bytes()),
            PageVmInitStage::DomContentLoaded,
            RendererReplyBoundary::DocumentCommit,
            CommittedDocumentResourceSource::Navigation(Box::new(
                fetched.document_fetch_context_seed,
            )),
            fetched.reserved_service_worker_client,
        )
        .await
        .unwrap();
    let built = context
        .start_document_materialization(
            contents,
            navigation,
            prepared,
            destination,
            context.inherited_document_policy(Default::default(), &[], None),
        )
        .unwrap()
        .materialize()
        .await
        .unwrap();
    let committed = context.commit_document_navigation(built.page).unwrap();
    assert_eq!(committed.snapshot.document.web_contents(), contents);
    assert_eq!(committed.snapshot.metadata.navigation, Some(navigation));
    committed
        .post_response_continuation
        .expect("Browser commit must release the native DocumentCommit boundary without DevTools")
        .release();
    committed.retirement.close().await;
    // Dropping the unused inspection endpoint must not retire the Document.
    drop(committed.snapshot.inspection_endpoint);
    context.document_handle(contents).unwrap().unwrap()
}

#[tokio::test]
async fn native_document_commits_publish_exact_occurrences_and_recover_current_snapshot() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (before, mut events) = browser.subscribe().unwrap();
    assert!(before.documents.is_empty());
    let first = navigate(&context, contents, "data:text/html,<title>first</title>").await;
    let first_event = events.try_recv().unwrap();
    assert_eq!(
        first_event.event,
        crate::browser::BrowserEvent::DocumentCommitted(first)
    );
    let snapshot = browser.document_commit_snapshot(first).unwrap();
    let first_renderer = context.document_renderer_residence(first).unwrap();
    assert_eq!(browser.document_for_renderer(first_renderer), Some(first));
    assert_eq!(snapshot.metadata.lifecycle.document, first.id());
    assert_eq!(
        snapshot.metadata.lifecycle.browser_sequence,
        first_event.sequence
    );
    assert!(first_event.sequence > before.sequence);
    assert_eq!(
        snapshot.metadata.info.as_ref().unwrap().url.as_str(),
        "data:text/html,<title>first</title>"
    );
    let second = navigate(&context, contents, "data:text/html,<title>second</title>").await;
    let second_event = events.try_recv().unwrap();
    assert_eq!(
        second_event.event,
        crate::browser::BrowserEvent::DocumentCommitted(second)
    );
    assert!(second_event.sequence > first_event.sequence);
    assert!(browser.document_commit_snapshot(first).is_err());
    assert_eq!(browser.document_for_renderer(first_renderer), None);
    let (current, _) = browser.subscribe().unwrap();
    assert_eq!(current.documents, [second]);
    assert_eq!(current.sequence, second_event.sequence);
    assert_eq!(
        context.document_commit_snapshot(second).unwrap().frame_slot,
        snapshot.frame_slot
    );
    assert_eq!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    );
    let pending = start_load(&context, contents);
    assert_eq!(
        browser.document_for_renderer(pending.renderer_page()),
        Some(DocumentHandle::new(contents, pending.document_id()))
    );
    drop(pending);
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert!(browser.document_commit_snapshot(second).is_err());
    assert!(browser.subscribe().unwrap().0.documents.is_empty());
    service.shutdown();
}

#[tokio::test]
async fn retired_context_rejects_late_renderer_lifecycle_without_affecting_peer() {
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let document = navigate(&context, contents, "data:text/html,retiring").await;
    let renderer = context.document_renderer_residence(document).unwrap();
    let snapshot = context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    let event = crate::page::RendererDocumentLifecycleEvent {
        frame: snapshot.frame,
        document: snapshot.document,
        epoch: snapshot.epoch,
        sequence: u64::MAX,
        timestamp_micros: 100,
        kind: crate::page::RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(crate::page::RendererDocumentLifecycleMilestone::Load),
            reason: crate::page::RendererDocumentTerminationReason::RestartedByDocumentOpen,
        },
    };
    assert!(context.remove().unwrap());
    let (peer, peer_contents) = context_with_contents(&service);
    let peer_document = navigate(&peer, peer_contents, "data:text/html,surviving").await;
    assert!(
        context
            .apply_renderer_document_lifecycle(renderer, event)
            .is_none()
    );
    assert!(
        peer.apply_renderer_document_lifecycle(renderer, event)
            .is_none()
    );
    assert_eq!(
        peer.document_handle(peer_contents).unwrap(),
        Some(peer_document)
    );
    assert_eq!(
        peer.document_url(peer_document).unwrap().as_str(),
        "data:text/html,surviving"
    );
    service.shutdown();
    assert!(
        context
            .apply_renderer_document_lifecycle(renderer, event)
            .is_none()
    );
}

#[tokio::test]
async fn browser_service_navigates_queries_replaces_and_closes_without_devtools() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let physical_identity = context.web_contents_identity(contents).unwrap();
    let first_url = server.url("/static");
    let first = navigate(&context, contents, &first_url).await;
    let first_lifetime = context.observe_document_lifetime(first).unwrap();
    let snapshot = context
        .start_capture_document_snapshot(first)
        .unwrap()
        .wait()
        .await;
    let snapshot = context.finish_capture_document_snapshot(snapshot).unwrap();
    assert_eq!(snapshot.url, first_url);
    assert!(snapshot.html.contains("fixture static"));
    let stale = context
        .start_capture_document_snapshot(first)
        .unwrap()
        .wait()
        .await;

    let second_url = server.url("/inline-script");
    let second = navigate(&context, contents, &second_url).await;
    assert_ne!(first, second);
    assert_eq!(
        context.web_contents_identity(contents).unwrap(),
        physical_identity
    );
    assert_eq!(first_lifetime.wait().await, DocumentRetirement::Superseded);
    assert!(context.finish_capture_document_snapshot(stale).is_err());
    assert!(context.document_url(first).is_err());
    let snapshot = context
        .start_capture_document_snapshot(second)
        .unwrap()
        .wait()
        .await;
    let snapshot = context.finish_capture_document_snapshot(snapshot).unwrap();
    assert_eq!(snapshot.url, second_url);
    assert!(snapshot.html.contains("fixture inline script"));
    assert_eq!(
        context
            .navigation_history_snapshot(contents)
            .unwrap()
            .1
            .len(),
        2
    );

    let second_lifetime = context.observe_document_lifetime(second).unwrap();
    context
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert_eq!(second_lifetime.wait().await, DocumentRetirement::Superseded);
    assert!(!context.contains_web_contents(contents));
    assert_eq!(context.loaded_document_count(), 0);
    assert!(context.is_live());
    assert!(context.remove().unwrap());
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_web_contents_close_activates_loaded_peer_without_devtools() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let (context, first) = context_with_contents(&service);
    navigate(&context, first, &server.url("/static")).await;
    let (peer, _) = context.create_web_contents(Default::default()).unwrap();
    let document = navigate(&context, peer, &server.url("/static")).await;
    let (unloaded, _) = context.create_web_contents(Default::default()).unwrap();
    assert!(context.select_web_contents(first.id()));
    let background = context
        .start_document_page_surface_update(
            document,
            false,
            Some(crate::browser::EmulatedNetworkConditions::offline()),
            None,
        )
        .unwrap()
        .wait()
        .await;
    context.finish_document_policy_update(background).unwrap();
    let expression = "[document.hidden, document.hasFocus(), navigator.onLine].join(',')";
    assert_eq!(
        context
            .evaluate_document_expression_for_test(document, expression, false)
            .await
            .unwrap()["value"],
        "true,false,false"
    );
    let close = service.handle().close_web_contents(first).unwrap();
    assert_eq!(close.activated, Some(peer));
    close.close_async().await;
    assert_eq!(context.selected_web_contents_handle(), Some(peer));
    assert!(context.contains_web_contents(unloaded));
    assert_eq!(
        context
            .evaluate_document_expression_for_test(document, expression, false)
            .await
            .unwrap()["value"],
        "false,true,false",
        "native close must activate the loaded peer without changing offline policy"
    );
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn browser_service_shutdown_retires_documents_despite_retained_capabilities() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let document = navigate(&context, contents, &server.url("/static")).await;
    let lifetime = context.observe_document_lifetime(document).unwrap();
    let completed = context
        .start_capture_document_snapshot(document)
        .unwrap()
        .wait()
        .await;

    service.shutdown();

    assert_eq!(lifetime.wait().await, DocumentRetirement::Unavailable);
    assert!(!context.is_live());
    assert!(context.finish_capture_document_snapshot(completed).is_err());
    assert!(
        context
            .create_web_contents(WebContentsCreation::default())
            .is_err()
    );
    assert!(service.handle().endpoint.tx.is_closed());
    assert!(service.handle().endpoint.join.lock().is_none());
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn browser_runtime_retirement_cancels_worker_graph_and_pending_fetches_without_devtools() {
    for remove_context in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (received_tx, mut received) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let mut requests = tokio::task::JoinSet::new();
            // One document, three worker scripts, and five held requests.
            for _ in 0..9 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let received_tx = received_tx.clone();
                requests.spawn(async move {
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        assert_ne!(stream.read_buf(&mut request).await.unwrap(), 0);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    let (mime, body) = match path {
                        "/" => (
                            "text/html",
                            "<!doctype html><script>\
                             fetch('/pending-window').catch(() => {});\
                             import('/pending-module.js').catch(() => {});\
                             globalThis.worker = new Worker('/dedicated.js');\
                             globalThis.shared = new SharedWorker('/shared.js');\
                             shared.port.start();\
                             navigator.serviceWorker.register('/service.js');\
                             </script>",
                        ),
                        "/dedicated.js" => (
                            "text/javascript",
                            "fetch('/pending-dedicated').catch(() => {});",
                        ),
                        "/shared.js" => (
                            "text/javascript",
                            "onconnect = () => { fetch('/pending-shared').catch(() => {}); };",
                        ),
                        "/service.js" => (
                            "text/javascript",
                            "oninstall = event => event.waitUntil(fetch('/pending-service'));",
                        ),
                        path if path.starts_with("/pending-") => {
                            received_tx.send(path.to_owned()).unwrap();
                            assert_eq!(stream.read(&mut [0]).await.unwrap(), 0, "{path}");
                            return;
                        }
                        _ => panic!("unexpected Browser runtime request: {path}"),
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
            while let Some(result) = requests.join_next().await {
                result.unwrap();
            }
        });
        let service = BrowserService::start().unwrap();
        let (context, contents) = context_with_contents(&service);
        let (peer, peer_contents) = context_with_contents(&service);
        let document = navigate(&context, contents, &url).await;
        let lifetime = context.observe_document_lifetime(document).unwrap();
        let mut pending = std::collections::BTreeSet::new();
        for _ in 0..5 {
            pending.insert(received.recv().await.unwrap());
        }
        assert_eq!(
            pending,
            [
                "/pending-dedicated",
                "/pending-module.js",
                "/pending-service",
                "/pending-shared",
                "/pending-window",
            ]
            .map(str::to_owned)
            .into_iter()
            .collect()
        );
        // Worker startup happened after commit. Refresh the exact Document's
        // cached page state before checking its running-isolate count.
        let snapshot = context
            .start_document_diagnostics_snapshot(document)
            .unwrap()
            .wait()
            .await;
        context
            .finish_document_diagnostics_snapshot(snapshot)
            .unwrap();
        let runtime = context.worker_runtime_inspection_endpoint();
        let active = runtime.moli_memory_diagnostics();
        assert_eq!(context.dedicated_worker_running_isolate_count(), 1);
        assert_eq!(active["sharedWorker"]["runningInstanceCount"], 1);
        assert_eq!(active["serviceWorker"]["runningWorkers"], 1);

        if remove_context {
            assert!(context.remove().unwrap());
            assert!(peer.contains_web_contents(peer_contents));
        } else {
            service.shutdown();
            assert!(!peer.is_live());
        }

        assert_eq!(lifetime.wait().await, DocumentRetirement::Unavailable);
        assert!(!context.is_live());
        server.await.unwrap();
        let retired = runtime.moli_memory_diagnostics();
        assert_eq!(retired["sharedWorker"]["runningInstanceCount"], 0);
        assert_eq!(retired["sharedWorker"]["clientCount"], 0);
        assert_eq!(retired["serviceWorker"]["runningWorkers"], 0);
        assert_eq!(retired["serviceWorker"]["inFlightEvents"], 0);
        assert_eq!(retired["serviceWorker"]["versions"], 0);
        assert_eq!(retired["serviceWorker"]["pendingServiceLaneEventCount"], 0);
        service.shutdown();
    }
}

#[derive(Clone, Copy)]
enum NavigationRetirement {
    Context,
    WebContents,
    AllWebContents,
    Supersession,
    ClearState,
    CancelMatching,
}

async fn assert_in_flight_navigation_retirement(retirement: NavigationRetirement) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/pending", listener.local_addr().unwrap());
    let (received_tx, received) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert_ne!(stream.read_buf(&mut request).await.unwrap(), 0);
        }
        received_tx.send(()).unwrap();
        // Browser retirement must cancel the actual network request; the server
        // never supplies a response that could end the request on its own.
        assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    });
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let stale_navigation = context.start_document_navigation(contents).unwrap();
    let mut load = start_load(&context, contents);
    let navigation = load.navigation_id();
    let fetching = tokio::spawn(async move {
        let result = load.fetch_navigation("GET", &url, None, Vec::new()).await;
        (load, result)
    });
    received.await.unwrap();
    let replacement = match retirement {
        NavigationRetirement::Context => {
            assert!(context.remove().unwrap());
            None
        }
        NavigationRetirement::WebContents => {
            context
                .close_web_contents(contents)
                .unwrap()
                .close_async()
                .await;
            None
        }
        NavigationRetirement::AllWebContents => {
            for closing in context.close_all_web_contents() {
                closing.close_async().await;
            }
            None
        }
        NavigationRetirement::Supersession => {
            Some(context.start_document_navigation(contents).unwrap())
        }
        NavigationRetirement::ClearState => {
            context.clear_document_navigation_state(contents).unwrap();
            None
        }
        NavigationRetirement::CancelMatching => {
            assert!(
                !context
                    .clear_pending_navigation_if_matches(contents, &stale_navigation)
                    .unwrap()
            );
            assert!(context.navigation_retains(contents, navigation).unwrap());
            assert!(
                context
                    .clear_pending_navigation_if_matches(contents, &navigation)
                    .unwrap()
            );
            None
        }
    };
    let (load, result) = fetching.await.unwrap();
    assert!(result.is_err());
    server.await.unwrap();
    match retirement {
        NavigationRetirement::Context => assert!(!context.is_live()),
        NavigationRetirement::WebContents | NavigationRetirement::AllWebContents => {
            assert!(context.is_live());
            assert_eq!(context.web_contents_count(), 0);
        }
        NavigationRetirement::Supersession => {
            assert!(
                context
                    .navigation_retains(contents, replacement.unwrap())
                    .unwrap()
            );
        }
        NavigationRetirement::ClearState | NavigationRetirement::CancelMatching => {
            assert!(context.is_live());
            assert!(!context.has_pending_document_navigation(contents).unwrap());
        }
    }
    let retained = service
        .handle()
        .execute(|browser| browser.navigation_work.work.len())
        .unwrap();
    assert_eq!(
        retained, 0,
        "a late fetch completion must not retain retired navigation work"
    );
    drop(load);
    service.shutdown();
}

#[tokio::test]
async fn context_removal_does_not_resurrect_in_flight_navigation_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::Context).await;
}

#[tokio::test]
async fn web_contents_close_does_not_resurrect_in_flight_navigation_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::WebContents).await;
}

#[tokio::test]
async fn close_all_web_contents_does_not_resurrect_in_flight_navigation_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::AllWebContents).await;
}

#[tokio::test]
async fn supersession_does_not_resurrect_in_flight_navigation_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::Supersession).await;
}

#[tokio::test]
async fn clearing_navigation_state_does_not_resurrect_in_flight_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::ClearState).await;
}

#[tokio::test]
async fn canceling_matching_navigation_does_not_retire_a_replacement_or_resurrect_work() {
    assert_in_flight_navigation_retirement(NavigationRetirement::CancelMatching).await;
}
