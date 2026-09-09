use super::*;
use crate::browser::{
    BrowserContextStoragePartitionHandles, BrowserEvent, BrowserService, DocumentHandle,
    DocumentRetirement, NavigationAttempt, NavigationFailureReason, NavigationSnapshot,
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
            context.inherited_document_policy(Default::default(), &[], None, None),
        )
        .unwrap()
}

#[tokio::test]
async fn native_navigation_transport_failure_commits_error_document_without_devtools() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/unreachable", listener.local_addr().unwrap());
    drop(listener);
    assert_native_error_document(
        url,
        "net::ERR_CONNECTION_REFUSED",
        "This site can’t be reached",
    )
    .await;
}

#[tokio::test]
async fn native_navigation_empty_http_error_commits_error_document_without_devtools() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/empty-error", listener.local_addr().unwrap());
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    assert_native_error_document(url, "net::ERR_HTTP_RESPONSE_CODE_FAILURE", "HTTP ERROR 404")
        .await;
    served.await.unwrap();
}

async fn assert_native_error_document(url: String, error_text: &str, html_marker: &str) {
    let service = BrowserService::start().unwrap();
    let (context, _) = context_with_contents(&service);
    let (contents, _) = context
        .create_web_contents(WebContentsCreation::with_initial_document(
            "about:blank".into(),
            None,
            None,
        ))
        .unwrap();
    let waiter = context
        .navigate_document(
            contents,
            crate::browser::web_contents::NavigationRequestInterception::new(
                Url::parse(&url).unwrap(),
                "GET".into(),
                None,
                Vec::new(),
                NavigationRequestLoadPolicy::BrowserInitiated,
            ),
        )
        .unwrap();
    let request = waiter.request();
    let crate::browser::BrowserNavigationOutcome::Document(committed) =
        waiter.wait().await.unwrap()
    else {
        panic!("a failed document navigation must commit an error document, not a download");
    };
    assert_eq!(committed.document.id(), request.document);
    assert_eq!(committed.metadata.navigation, Some(request.navigation));
    let info = committed.metadata.info.as_ref().unwrap();
    assert_eq!(info.url.as_str(), url);
    let error = info.error_page.as_ref().unwrap();
    assert_eq!(error.unreachable_url.as_str(), url);
    assert_eq!(error.error_text, error_text);
    let captured = context
        .start_capture_document_snapshot(committed.document)
        .unwrap()
        .wait()
        .await;
    let captured = context.finish_capture_document_snapshot(captured).unwrap();
    assert!(captured.html.contains(html_marker), "{}", captured.html);
    assert_eq!(
        context.document_handle(contents).unwrap(),
        Some(committed.document)
    );
    service.shutdown();
}

#[test]
fn native_navigation_start_and_cancellation_publish_owner_occurrences() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (before, mut events) = browser.subscribe().unwrap();
    let navigation = context.start_document_navigation(contents).unwrap();
    assert!(
        context
            .accepts_pending_navigation(contents, &navigation)
            .unwrap()
    );
    let started = events
        .try_recv()
        .expect("native navigation admission must publish a Browser occurrence without DevTools");
    assert!(started.sequence > before.sequence);
    let BrowserEvent::NavigationStarted(request) = started.event else {
        panic!("{started:?}");
    };
    assert_eq!(request.web_contents, contents);
    assert_eq!(request.navigation, navigation);
    assert_eq!(
        browser.subscribe().unwrap().0.navigations,
        [NavigationSnapshot {
            web_contents: contents,
            committed: None,
            attempt: Some(NavigationAttempt::Started(request))
        }]
    );
    assert!(
        context
            .cancel_document_navigation(contents, &navigation)
            .unwrap()
    );
    let canceled = events
        .try_recv()
        .expect("native cancellation must publish its exact terminal occurrence");
    assert!(canceled.sequence > started.sequence);
    assert_eq!(
        canceled.event,
        BrowserEvent::NavigationFailed {
            request,
            reason: NavigationFailureReason::Canceled
        }
    );
    let failed = NavigationSnapshot {
        web_contents: contents,
        committed: None,
        attempt: Some(NavigationAttempt::Failed {
            request,
            reason: NavigationFailureReason::Canceled,
        }),
    };
    assert_eq!(context.navigation_snapshot(contents).unwrap(), failed);
    assert!(
        !context
            .cancel_document_navigation(contents, &navigation)
            .unwrap()
    );
    assert!(events.try_recv().is_err());
    for _ in 0..130 {
        let transient = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        transient.remove().unwrap();
    }
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ));
    let (recovered, mut events) = browser.subscribe().unwrap();
    assert_eq!(recovered.navigations, [failed]);
    let replacement = context.start_document_navigation(contents).unwrap();
    assert_ne!(replacement, navigation);
    assert!(
        matches!(events.try_recv().unwrap().event, BrowserEvent::NavigationStarted(next) if next.navigation == replacement && next.document != request.document)
    );
    assert!(
        !context
            .cancel_document_navigation(contents, &navigation)
            .unwrap()
    );
    assert!(events.try_recv().is_err());
    service.shutdown();
}

#[tokio::test]
async fn committed_native_navigation_is_not_reported_as_failed_on_close() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        "data:text/html,<title>committed</title>",
    )
    .await;
    let mut started = None;
    let mut committed = false;
    while let Ok(event) = events.try_recv() {
        match event.event {
            BrowserEvent::NavigationStarted(request) => {
                assert!(started.replace(request).is_none());
            }
            BrowserEvent::DocumentCommitted(actual) => {
                assert_eq!(actual, document);
                committed = true;
            }
            BrowserEvent::NavigationFailed { .. } => {
                panic!("committed attempt was reported as failed")
            }
            _ => {}
        }
    }
    assert!(committed);
    let request = started.unwrap();
    assert_eq!(request.document, document.id());
    let committed = NavigationSnapshot {
        web_contents: contents,
        committed: Some(request),
        attempt: None,
    };
    assert_eq!(context.navigation_snapshot(contents).unwrap(), committed);
    assert_eq!(browser.subscribe().unwrap().0.navigations, [committed]);
    assert!(
        !context
            .cancel_document_navigation(contents, &request.navigation)
            .unwrap()
    );
    context
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(
            event.event,
            BrowserEvent::NavigationFailed { .. }
        ));
    }
    service.shutdown();
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
    let destination = DocumentNavigationDestination::Document {
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
            context.inherited_document_policy(Default::default(), &[], None, None),
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
async fn native_document_lifecycle_advances_without_a_devtools_output_consumer() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        "data:text/html,<title>native lifecycle</title>",
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let crate::browser::BrowserEvent::DocumentLifecycleChanged(snapshot) =
                events.recv().await.unwrap().event
                && snapshot.document == document
                && snapshot.lifecycle.load.is_some()
            {
                assert_eq!(
                    context.document_lifecycle_snapshot(document).unwrap(),
                    Some(snapshot.lifecycle)
                );
                assert!(
                    browser
                        .subscribe()
                        .unwrap()
                        .0
                        .document_lifecycles
                        .contains(&snapshot)
                );
                break;
            }
        }
    })
    .await
    .expect("native load progress must be committed and published without DevTools ingress");
    let before = context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    context
        .evaluate_document_expression_for_test(document, "document.open(); 'opened'", false)
        .await
        .unwrap();
    let after = context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    assert_eq!(after.document, before.document);
    assert!(after.epoch.0 > before.epoch.0);
    assert!(after.started.sequence > before.sequence());
    assert_eq!(context.document_handle(contents).unwrap(), Some(document));
    service.shutdown();
}

fn next_document_commit(
    events: &mut crate::browser::BrowserEventReceiver,
    document: DocumentHandle,
) -> crate::browser::BrowserEventRecord {
    let mut started = false;
    loop {
        let event = events
            .try_recv()
            .expect("commit publishes before returning");
        if event.event == crate::browser::BrowserEvent::DocumentCommitted(document) {
            assert!(
                started,
                "native admission must precede its exact Document commit"
            );
            return event;
        }
        match event.event {
            BrowserEvent::NavigationStarted(request) => {
                assert!(!started, "duplicate navigation admission");
                assert_eq!(request.web_contents, document.web_contents());
                assert_eq!(request.document, document.id());
                started = true;
            }
            BrowserEvent::DocumentLifecycleChanged(_) => {}
            _ => panic!("unexpected event before exact Document commit: {event:?}"),
        }
    }
}

async fn next_native_dialog(
    events: &mut crate::browser::BrowserEventReceiver,
    document: DocumentHandle,
) -> crate::browser::JavaScriptDialogOpened {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let crate::browser::BrowserEvent::DialogOpened(dialog) =
                events.recv().await.unwrap().event
                && dialog.document == document
            {
                break dialog;
            }
        }
    })
    .await
    .expect("the exact native Document must own and publish its dialog without CDP ingress")
}

#[tokio::test]
async fn native_popup_admission_commits_initial_document_without_devtools_ingress() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let source = navigate(
        &context,
        contents,
        "data:text/html,<script>window.open('about:blank','native-popup-owner')</script>",
    )
    .await;
    let popup = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::WebContentsCreated(popup) = events.recv().await.unwrap().event
                && popup.context() == context.id()
                && popup != contents
                && context.web_contents_window_name(popup).unwrap().as_deref()
                    == Some("native-popup-owner")
            {
                break popup;
            }
        }
    })
    .await
    .expect("an accepted window.open must create its native WebContents without CDP ingress");
    assert_eq!(context.document_handle(contents).unwrap(), Some(source));
    assert_eq!(
        context.web_contents_opener(popup).unwrap(),
        Some((contents.id(), true))
    );
    assert_eq!(browser.subscribe().unwrap().0.web_contents.len(), 2);
    let document = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DocumentCommitted(document) = events.recv().await.unwrap().event
                && document.web_contents() == popup
            {
                break document;
            }
        }
    })
    .await;
    if document.is_err() {
        service.shutdown();
    }
    let document = document.expect("a blank popup must construct without a DevTools observer");
    assert_eq!(context.document_handle(popup).unwrap(), Some(document));
    assert_eq!(
        context.document_url(document).unwrap().as_str(),
        "about:blank"
    );
    assert_eq!(
        context.navigation_snapshot(popup).unwrap(),
        NavigationSnapshot {
            web_contents: popup,
            committed: None,
            attempt: None
        },
        "initial construction must not fabricate a cross-document navigation"
    );
    context
        .close_web_contents(popup)
        .unwrap()
        .close_async()
        .await;
    assert_eq!(context.document_handle(contents).unwrap(), Some(source));
    assert_eq!(browser.subscribe().unwrap().0.web_contents, [contents]);
    service.shutdown();
}

#[tokio::test]
async fn native_initial_document_disconnect_finishes_claimed_preparation() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let provider = browser.register_document_decision_provider().unwrap();
    let (context, contents) = context_with_contents(&service);
    let observation = context
        .start_initial_document(
            contents,
            context.inherited_document_policy(Default::default(), &[], None, None),
        )
        .unwrap()
        .unwrap();
    let key = observation.key();
    let claim = hold_initial_prepared_inspection(&browser, &context, contents, key).await;
    assert!(
        context
            .claim_initial_document_inspection(contents, key)
            .unwrap()
            .is_none()
    );
    drop(provider);
    let committed = tokio::time::timeout(std::time::Duration::from_secs(5), observation.wait())
        .await
        .expect("disconnected inspection must not hold Browser construction")
        .unwrap()
        .unwrap();
    assert_eq!(committed.key, key);
    assert_eq!(
        context.document_handle(contents).unwrap(),
        Some(committed.snapshot.document)
    );
    drop(claim);
    assert_eq!(
        context.document_handle(contents).unwrap(),
        Some(committed.snapshot.document)
    );
    service.shutdown();
}

#[tokio::test]
async fn native_initial_document_close_cancels_preparation_without_touching_peer() {
    use crate::browser::web_contents::InitialDocumentInspectionStage;
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let _provider = browser.register_document_decision_provider().unwrap();
    let (context, contents) = context_with_contents(&service);
    let (peer, _) = context
        .create_web_contents(WebContentsCreation::default())
        .unwrap();
    let peer_document = navigate(&context, peer, "data:text/html,<title>peer</title>").await;
    assert_eq!(
        context
            .evaluate_document_expression_for_test(
                peer_document,
                "globalThis.__nativeInitialPeer = 'peer'",
                false,
            )
            .await
            .unwrap()["value"],
        "peer"
    );
    let observation = context
        .start_initial_document(
            contents,
            context.inherited_document_policy(Default::default(), &[], None, None),
        )
        .unwrap()
        .unwrap();
    let key = observation.key();
    let claim = hold_initial_prepared_inspection(&browser, &context, contents, key).await;
    let InitialDocumentInspectionStage::Prepared(endpoint) = &claim.stage else {
        panic!("prepared phase");
    };
    context
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert_eq!(
        observation.wait().await.err().as_deref(),
        Some("InitialDocumentPageBuildCancelled")
    );
    assert!(
        endpoint.start_configure(Default::default()).await.is_err(),
        "closed preparation must release its real renderer reservation"
    );
    drop(claim);
    assert!(!context.contains_web_contents(contents));
    assert_eq!(context.document_handle(peer).unwrap(), Some(peer_document));
    assert_eq!(
        context
            .evaluate_document_expression_for_test(
                peer_document,
                "globalThis.__nativeInitialPeer",
                false,
            )
            .await
            .unwrap()["value"],
        "peer",
        "closing the prepared page must preserve the peer's live renderer state"
    );
    service.shutdown();
}

async fn hold_initial_prepared_inspection(
    browser: &BrowserHandle,
    context: &BrowserContextHandle,
    contents: WebContentsHandle,
    key: crate::browser::web_contents::InitialDocumentBuildKey,
) -> crate::browser::web_contents::InitialDocumentInspectionClaim {
    use crate::browser::web_contents::InitialDocumentInspectionStage;
    let (_, mut events) = browser.subscribe().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(claim) = context
                .claim_initial_document_inspection(contents, key)
                .unwrap()
            {
                if matches!(&claim.stage, InitialDocumentInspectionStage::Prepared(_)) {
                    return claim;
                }
                drop(claim);
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("exact native prepared inspection phase")
}

#[tokio::test]
async fn native_initial_document_construction_survives_a_dropped_observer() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    context
        .begin_initial_empty_document(contents, "about:blank#native-initial".into(), None, None)
        .unwrap();
    let (_, mut events) = browser.subscribe().unwrap();
    let observation = context
        .start_initial_document(
            contents,
            context.inherited_document_policy(Default::default(), &[], None, None),
        )
        .unwrap();
    drop(observation);
    let document = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DocumentCommitted(document) = events.recv().await.unwrap().event
                && document.web_contents() == contents
            {
                break document;
            }
        }
    })
    .await;
    let observed = context.document_handle(contents).unwrap();
    let url = observed.map(|document| context.document_url(document).unwrap());
    service.shutdown();
    assert_eq!(
        Some(document.expect("Browser construction must outlive its observer")),
        observed
    );
    assert_eq!(url.unwrap().as_str(), "about:blank#native-initial");
}

#[tokio::test]
async fn native_initial_url_failed_admission_does_not_change_the_next_history_entry() {
    let service = BrowserService::start().unwrap();
    let context = service
        .handle()
        .create_context(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        )
        .unwrap();
    let (contents, _) = context
        .create_web_contents(WebContentsCreation::default())
        .unwrap();
    context
        .begin_initial_empty_document(contents, "about:blank".into(), None, None)
        .unwrap();
    assert_eq!(
        context
            .navigate_initial_document(contents, "data:text/html,rejected".parse().unwrap())
            .unwrap_err(),
        "navigation WebContents engine unavailable"
    );
    assert!(
        context
            .navigation_snapshot(contents)
            .unwrap()
            .attempt
            .is_none()
    );
    context.bind_page_navigation_engines(Default::default(), None);
    navigate(&context, contents, "data:text/html,independent").await;
    let (_, history) = context.navigation_history_snapshot(contents).unwrap();
    assert_eq!(history.last().unwrap().transition_type, "typed");
    service.shutdown();
}

#[tokio::test]
async fn native_initial_url_loads_without_devtools_and_replaces_only_its_initial_history() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    context
        .begin_initial_empty_document(contents, "about:blank".into(), None, None)
        .unwrap();
    let (_, mut events) = browser.subscribe().unwrap();
    let url = server.url("/static?created=native-navigation");
    let navigation = context
        .navigate_initial_document(contents, url.parse().unwrap())
        .unwrap()
        .expect("original initial navigation admitted");
    assert!(
        context
            .navigate_initial_document(contents, "data:text/html,duplicate".parse().unwrap())
            .unwrap()
            .is_none(),
        "a second creation completion cannot supersede its pending request"
    );
    let document = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DocumentCommitted(document) = events.recv().await.unwrap().event
                && document.web_contents() == contents
                && context.document_url(document).unwrap().as_str() == url
            {
                break document;
            }
        }
    })
    .await
    .expect("Browser must finish the requested URL without a DevTools consumer");
    let snapshot = context.navigation_snapshot(contents).unwrap();
    assert_eq!(snapshot.committed.unwrap().navigation, navigation);
    let (index, history) = context.navigation_history_snapshot(contents).unwrap();
    assert_eq!(index, 0);
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].url, url);
    assert_eq!(history[0].transition_type, "auto_toplevel");
    assert!(
        context
            .navigate_initial_document(contents, "data:text/html,stale".parse().unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(context.document_handle(contents).unwrap(), Some(document));
    service.shutdown();
}

#[test]
fn native_initial_url_does_not_replace_an_existing_candidate_or_closed_contents() {
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    context
        .begin_initial_empty_document(contents, "about:blank".into(), None, None)
        .unwrap();
    let winner = context.start_document_navigation(contents).unwrap();
    assert!(
        context
            .navigate_initial_document(contents, "data:text/html,obsolete".parse().unwrap())
            .unwrap()
            .is_none()
    );
    assert!(
        context
            .accepts_pending_navigation(contents, &winner)
            .unwrap()
    );
    context.close_web_contents(contents).unwrap();
    assert!(
        context
            .navigate_initial_document(contents, "data:text/html,removed".parse().unwrap())
            .is_err()
    );
    service.shutdown();
}

#[tokio::test]
async fn native_popup_navigates_its_requested_url_without_devtools_ingress() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let url = server.url("/static?popup=native-navigation");
    let source = navigate(
        &context,
        contents,
        &format!("data:text/html,<script>window.open('{url}','native-popup-navigation')</script>"),
    )
    .await;
    let popup_document = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut popup = None;
        loop {
            match events.recv().await.unwrap().event {
                BrowserEvent::WebContentsCreated(handle)
                    if handle.context() == context.id()
                        && handle != contents
                        && context.web_contents_window_name(handle).unwrap().as_deref()
                            == Some("native-popup-navigation") =>
                {
                    assert!(
                        popup.replace(handle).is_none(),
                        "one accepted popup creates once"
                    );
                }
                BrowserEvent::DocumentCommitted(document)
                    if Some(document.web_contents()) == popup
                        && context.document_url(document).unwrap().as_str() == url =>
                {
                    break document;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the Browser must drive the accepted popup URL without a DevTools consumer");
    let captured = context
        .start_capture_document_snapshot(popup_document)
        .unwrap()
        .wait()
        .await;
    let captured = context.finish_capture_document_snapshot(captured).unwrap();
    assert_eq!(captured.url, url);
    assert!(captured.html.contains("fixture static"));
    assert_eq!(context.document_handle(contents).unwrap(), Some(source));
    assert_eq!(
        context
            .web_contents_opener(popup_document.web_contents())
            .unwrap(),
        Some((contents.id(), true))
    );
    context
        .close_web_contents(popup_document.web_contents())
        .unwrap()
        .close_async()
        .await;
    assert_eq!(context.document_handle(contents).unwrap(), Some(source));
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_popup_decision_provider_drop_resumes_the_exact_request() {
    assert_native_popup_request_release(false).await;
}

#[tokio::test]
async fn native_popup_download_outlives_its_navigation_without_devtools() {
    assert_native_popup_download(true).await;
}

#[tokio::test]
async fn native_popup_download_inherits_browser_policy_without_devtools() {
    assert_native_popup_download(false).await;
}

async fn assert_native_popup_download(context_override: bool) {
    use crate::browser::{DownloadBehavior, DownloadPolicy, DownloadState};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/native-popup-download",
        listener.local_addr().unwrap()
    );
    let (release, body_released) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut body_released = body_released.clone();
            tokio::spawn(async move {
                let mut request = [0; 2048];
                if stream.read(&mut request).await.unwrap_or(0) == 0 {
                    return;
                }
                let body = b"native popup download body";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"native-popup.txt\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                let _ = body_released.wait_for(|released| *released).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    struct DownloadDirectory(std::path::PathBuf);
    impl Drop for DownloadDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = DownloadDirectory(std::env::temp_dir().join(format!(
        "moli-native-popup-download-{}-{}",
        std::process::id(),
        NavigationId::allocate().get()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, source) = context_with_contents(&service);
    let policy = DownloadPolicy {
        behavior: DownloadBehavior::AllowAndName,
        download_path: Some(directory.0.to_string_lossy().into_owned()),
    };
    if context_override {
        browser.set_download_policy(DownloadPolicy {
            behavior: DownloadBehavior::Deny,
            download_path: None,
        });
        context.set_download_policy(Some(policy));
    } else {
        browser.set_download_policy(policy);
    }
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(
        &context,
        source,
        &format!("data:text/html,<script>window.open('{url}','native-download')</script>"),
    )
    .await;
    let download = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DownloadCreated(download) = events.recv().await.unwrap().event
                && download.event.web_contents != source
                && download
                    .event
                    .snapshot
                    .metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.url == url)
            {
                break download;
            }
        }
    })
    .await;
    if download.is_err() {
        // Release the server and retire pages asynchronously before reporting
        // the assertion. Synchronous Browser Drop must not wait for a body
        // whose producer can only run on this test's current-thread runtime.
        release.send(true).unwrap();
        for closing in context.close_all_web_contents() {
            closing.close_async().await;
        }
        service.shutdown();
        server.abort();
    }
    let download = download
        .expect("Browser must turn the popup attachment response into a download before EOF");
    assert_eq!(download.event.snapshot.state, DownloadState::Active);
    let popup = download.event.web_contents;
    let initial_document = context.document_handle(popup).unwrap().unwrap();
    let before = context.navigation_snapshot(popup).unwrap();
    let Some(NavigationAttempt::Failed { request, reason }) = before.attempt else {
        panic!("download must retire its exact navigation before admission becomes observable");
    };
    assert_eq!(reason, NavigationFailureReason::Download);
    assert_eq!(request.web_contents, popup);
    let responses = context.navigation_responses(popup).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].request, request);
    assert_eq!(
        responses[0].response.as_ref().unwrap().final_url.as_str(),
        url
    );
    assert!(matches!(&responses[0].body, Some(Err(error)) if error == "net::ERR_ABORTED"));
    let replacement = context.start_document_navigation(popup).unwrap();
    assert!(context.navigation_responses(popup).unwrap().is_empty());
    release.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DownloadUpdated(event) = events.recv().await.unwrap().event
                && event.guid == download.event.guid
                && event.snapshot.state != DownloadState::Active
            {
                assert!(matches!(
                    event.snapshot.state,
                    DownloadState::Completed { .. }
                ));
                break;
            }
        }
    })
    .await
    .expect("superseding navigation must not cancel its transferred download body");
    let body = context
        .read_download_artifact(&download.event.guid)
        .unwrap()
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body, b"native popup download body");
    let document = context.document_handle(popup).unwrap().unwrap();
    assert_eq!(document, initial_document);
    assert!(
        matches!(context.navigation_snapshot(popup).unwrap().attempt,
        Some(NavigationAttempt::Started(request)) if request.navigation == replacement)
    );
    assert_eq!(
        context.document_url(document).unwrap().as_str(),
        "about:blank"
    );
    assert!(
        context
            .cancel_document_navigation(popup, &replacement)
            .unwrap()
    );
    service.shutdown();
    server.abort();
}

#[tokio::test]
async fn native_popup_claim_drop_resumes_with_its_decision_provider_still_alive() {
    assert_native_popup_request_release(true).await;
}

async fn assert_native_popup_request_release(drop_claim: bool) {
    use crate::browser::{NavigationDecision, NavigationDecisionStage};
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let mut provider = Some(browser.register_document_decision_provider().unwrap());
    let (context, source) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let url = server.url("/static?popup=provider-drop");
    navigate(
        &context,
        source,
        &format!("data:text/html,<script>window.open('{url}','provider-drop')</script>"),
    )
    .await;
    let (popup, permit) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap().event;
            if let BrowserEvent::InitialDocumentAwaitingInspection { web_contents, key } = event {
                drop(
                    context
                        .claim_initial_document_inspection(web_contents, key)
                        .unwrap(),
                );
                continue;
            }
            if let BrowserEvent::NavigationAwaitingDecision(request) = event
                && request.web_contents != source
                && let Some(paused) = context.navigation_decision(request.web_contents).unwrap()
            {
                if matches!(paused.stage, NavigationDecisionStage::Request { .. }) {
                    break (request.web_contents, paused.permit);
                }
                assert!(
                    context
                        .resolve_navigation_decision(
                            request.web_contents,
                            paused.permit,
                            NavigationDecision::Continue
                        )
                        .unwrap()
                );
            }
        }
    })
    .await
    .expect("exact request-stage decision");
    assert_eq!(
        context.navigation_decision(popup).unwrap().unwrap().permit,
        permit
    );
    if drop_claim {
        let claimed = context.take_navigation_request(permit).unwrap();
        assert!(context.take_navigation_request(permit).is_none());
        drop(claimed);
    } else {
        provider.take();
    }
    let committed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap().event {
                BrowserEvent::InitialDocumentAwaitingInspection { web_contents, key }
                    if web_contents == popup =>
                {
                    drop(
                        context
                            .claim_initial_document_inspection(web_contents, key)
                            .unwrap(),
                    );
                }
                BrowserEvent::DocumentCommitted(document)
                    if document.web_contents() == popup
                        && context.document_url(document).unwrap().as_str() == url =>
                {
                    break document;
                }
                BrowserEvent::NavigationAwaitingDecision(request)
                    if request.web_contents == popup =>
                {
                    if let Some(paused) = context.navigation_decision(popup).unwrap() {
                        assert!(!matches!(
                            paused.stage,
                            NavigationDecisionStage::Request { .. }
                        ));
                        assert!(
                            context
                                .resolve_navigation_decision(
                                    popup,
                                    paused.permit,
                                    NavigationDecision::Continue
                                )
                                .unwrap()
                        );
                    }
                }
                _ => {}
            }
        }
    })
    .await
    .expect("abandoned decision must release native work without protocol ingress");
    assert_eq!(
        context
            .navigation_snapshot(popup)
            .unwrap()
            .committed
            .unwrap()
            .navigation,
        permit.navigation()
    );
    assert_eq!(context.document_handle(popup).unwrap(), Some(committed));
    assert!(
        !context
            .resolve_navigation_decision(popup, permit, NavigationDecision::Cancel)
            .unwrap(),
        "a consumed permit cannot cancel the committed document"
    );
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_popup_decision_cannot_resume_a_replacement_navigation() {
    use crate::browser::NavigationDecision;
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let _provider = browser.register_document_decision_provider().unwrap();
    let (context, source) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, source, "data:text/html,<script>window.open('data:text/html,obsolete','superseded-native')</script>").await;
    let (popup, permit) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap().event;
            if let BrowserEvent::InitialDocumentAwaitingInspection { web_contents, key } = event {
                drop(
                    context
                        .claim_initial_document_inspection(web_contents, key)
                        .unwrap(),
                );
                continue;
            }
            if let BrowserEvent::NavigationAwaitingDecision(request) = event
                && request.web_contents != source
                && let Some(paused) = context.navigation_decision(request.web_contents).unwrap()
            {
                break (request.web_contents, paused.permit);
            }
        }
    })
    .await
    .expect("native popup admission pause");
    let replacement = navigate(&context, popup, "data:text/html,<main>winner</main>").await;
    assert!(
        !context
            .resolve_navigation_decision(popup, permit, NavigationDecision::Continue)
            .unwrap()
    );
    assert_eq!(context.document_handle(popup).unwrap(), Some(replacement));
    assert_ne!(
        context
            .navigation_snapshot(popup)
            .unwrap()
            .committed
            .unwrap()
            .navigation,
        permit.navigation()
    );
    service.shutdown();
}

#[tokio::test]
async fn native_javascript_dialog_is_admitted_without_devtools_ingress() {
    use crate::page::RendererJavaScriptDialogSource;
    for (kind, script) in [
        ("root", "alert('native dialog')"),
        (
            "child",
            "let child=document.createElement('iframe');document.body.append(child);child.contentWindow.alert('native dialog')",
        ),
        (
            "popup",
            r#"window.open("javascript:alert('native dialog')", 'native-dialog')"#,
        ),
    ] {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (context, contents) = context_with_contents(&service);
        let (_, mut events) = browser.subscribe().unwrap();
        let document = navigate(
            &context,
            contents,
            &format!("data:text/html,<body><script>{script}</script>"),
        )
        .await;
        let dialog = next_native_dialog(&mut events, document).await;
        assert_eq!(context.document_handle(contents).unwrap(), Some(document));
        assert_eq!(
            context
                .document_javascript_dialog_snapshot(document, dialog.key)
                .unwrap()
                .message,
            "native dialog"
        );
        assert!(
            context
                .web_contents_has_pending_javascript_dialog(contents)
                .unwrap()
        );
        assert!(
            match kind {
                "root" => matches!(
                    dialog.opening.source,
                    RendererJavaScriptDialogSource::RootFrame
                ),
                "child" => matches!(
                    dialog.opening.source,
                    RendererJavaScriptDialogSource::ChildFrame { .. }
                ),
                "popup" => matches!(
                    dialog.opening.source,
                    RendererJavaScriptDialogSource::LightweightPopup { .. }
                ),
                _ => unreachable!(),
            },
            "{kind} must retain its exact renderer Window source: {:?}",
            dialog.opening.source
        );
        // No DevTools Target is required for a lightweight popup. Its request belongs
        // to the physical Page containing that Window, not a later projection.
        assert!(
            browser
                .subscribe()
                .unwrap()
                .0
                .web_contents
                .contains(&contents)
        );
        for _ in 0..130 {
            let transient = browser
                .create_context(
                    BrowserContextStoragePartitionHandles::memory(),
                    StoragePartitionKind::Ephemeral,
                    None,
                    None,
                )
                .unwrap();
            transient.remove().unwrap();
        }
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
        ));
        let (snapshot, mut events) = browser.subscribe().unwrap();
        assert!(snapshot.javascript_dialogs.contains(&dialog));
        context
            .finish_document_javascript_dialog(document, dialog.key, false, None)
            .unwrap();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                if event.event
                    == (crate::browser::BrowserEvent::DialogClosed {
                        document,
                        key: dialog.key,
                    })
                {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert!(closed.sequence > snapshot.sequence);
        assert_eq!(
            closed.event,
            crate::browser::BrowserEvent::DialogClosed {
                document,
                key: dialog.key
            }
        );
        assert!(browser.subscribe().unwrap().0.javascript_dialogs.is_empty());
        assert!(
            context
                .finish_document_javascript_dialog(document, dialog.key, true, None)
                .is_none()
        );
        service.shutdown();
    }
}

#[tokio::test]
async fn native_modal_dialog_resumes_its_original_renderer_without_devtools() {
    for (blocks_root_parser, script) in [
        (true, "globalThis.answer=prompt('native modal','seed')"),
        (
            true,
            "let child=document.createElement('iframe');document.body.append(child);globalThis.answer=child.contentWindow.prompt('native modal','seed')",
        ),
        (
            false,
            r#"window.open("javascript:void(opener.answer=prompt('native modal','seed'))", 'native-modal')"#,
        ),
    ] {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (context, contents) = context_with_contents(&service);
        context.set_javascript_dialog_handler_enabled(true);
        let (_, mut events) = browser.subscribe().unwrap();
        let document = navigate(
            &context,
            contents,
            &format!("data:text/html,<body><script>{script}</script>"),
        )
        .await;
        let dialog = next_native_dialog(&mut events, document).await;
        assert_eq!(dialog.opening.default_prompt, "seed");
        context
            .set_document_javascript_dialog_prompt_text(
                document,
                dialog.key,
                "native answer".into(),
            )
            .unwrap();
        let closed = context
            .finish_document_javascript_dialog(document, dialog.key, true, None)
            .unwrap();
        assert_eq!(closed.user_input, "native answer");
        if blocks_root_parser {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let crate::browser::BrowserEvent::DocumentLifecycleChanged(snapshot) =
                        events.recv().await.unwrap().event
                        && snapshot.document == document
                        && snapshot.lifecycle.load.is_some()
                    {
                        break;
                    }
                }
            })
            .await
            .expect("native dialog completion must release the real parser");
        }
        assert_eq!(
            context
                .evaluate_document_expression_for_test(document, "globalThis.answer", false)
                .await
                .unwrap()["value"],
            "native answer"
        );
        assert!(browser.subscribe().unwrap().0.javascript_dialogs.is_empty());
        service.shutdown();
    }
}

#[tokio::test]
async fn native_dialog_retirement_rejects_late_completion_and_admission_waiters() {
    use std::{
        future::Future,
        task::{Context, Waker},
    };
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    context.set_javascript_dialog_handler_enabled(true);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        "data:text/html,<script>confirm('retiring modal')</script>",
    )
    .await;
    let dialog = next_native_dialog(&mut events, document).await;
    let renderer = context.document_renderer_residence(document).unwrap();
    let mut later = (*dialog.opening).clone();
    later.id = crate::page::RendererJavaScriptDialogId::new(later.id.sequence() + 1);
    let waiting = browser.wait_for_renderer_javascript_dialog(renderer, std::sync::Arc::new(later));
    tokio::pin!(waiting);
    assert!(
        waiting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        browser.close_web_contents(contents).unwrap().close_async(),
    )
    .await
    .unwrap();
    assert!(waiting.await.is_none());
    assert!(
        context
            .finish_document_javascript_dialog(document, dialog.key, true, None)
            .is_none()
    );
    assert!(browser.subscribe().unwrap().0.javascript_dialogs.is_empty());
    let (replacement, _) = context.create_web_contents(Default::default()).unwrap();
    let next = navigate(&context, replacement, "data:text/html,replacement").await;
    assert_ne!(document, next);
    assert!(
        context
            .finish_document_javascript_dialog(document, dialog.key, true, None)
            .is_none()
    );
    assert!(
        !context
            .web_contents_has_pending_javascript_dialog(replacement)
            .unwrap()
    );
    service.shutdown();
}

#[tokio::test]
async fn native_document_stop_retires_dialogs_and_late_observers_without_devtools() {
    use crate::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };
    use std::{
        future::Future,
        task::{Context, Waker},
    };
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let document = navigate(&context, contents, "data:text/html,native-stop").await;
    let renderer = context.document_renderer_residence(document).unwrap();
    let source = context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .unwrap();
    let dialog = |id, completion| {
        RendererPendingJavaScriptDialog::new(
            RendererJavaScriptDialogId::new(id),
            source.into(),
            RendererJavaScriptDialogSource::RootFrame,
            "data:text/html,native-stop".into(),
            "alert".into(),
            "native dialog".into(),
            String::new(),
            Some(completion),
        )
    };
    let original_completion = RendererJavaScriptDialogCompletion::pending();
    assert!(
        context
            .install_document_javascript_dialog_for_test(
                document,
                dialog(1, original_completion.clone())
            )
            .unwrap()
            .is_some()
    );
    let (_, mut events) = browser.subscribe().unwrap();
    let stopped = context
        .start_document_lifecycle_stop(document)
        .unwrap()
        .wait()
        .await;
    context.finish_document_lifecycle_stop(stopped).unwrap();
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let crate::browser::BrowserEvent::DocumentLifecycleChanged(snapshot) =
                events.recv().await.unwrap().event
                && snapshot.document == document
                && snapshot.lifecycle.terminated.is_some()
            {
                break snapshot;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        !context
            .web_contents_has_pending_javascript_dialog(contents)
            .unwrap()
    );
    assert!(!original_completion.finish(true, "late".into()));
    assert!(!original_completion.wait().accepted);
    let late_completion = RendererJavaScriptDialogCompletion::pending();
    assert!(
        context
            .install_document_javascript_dialog_for_test(
                document,
                dialog(2, late_completion.clone())
            )
            .unwrap()
            .is_none()
    );
    assert!(!late_completion.finish(true, "resurrection".into()));
    assert!(!late_completion.wait().accepted);
    // Force actual bounded-stream lag. Recovery must retain the same physical
    // Document's terminal state, even with no protocol projection at all.
    for _ in 0..130 {
        let transient = browser
            .create_context(
                BrowserContextStoragePartitionHandles::memory(),
                StoragePartitionKind::Ephemeral,
                None,
                None,
            )
            .unwrap();
        transient.remove().unwrap();
    }
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_))
    ));
    assert!(
        browser
            .subscribe()
            .unwrap()
            .0
            .document_lifecycles
            .contains(&terminal)
    );

    let unproduced = crate::page::RendererDocumentLifecycleEvent {
        frame: source.frame,
        document: source.document,
        epoch: source.epoch,
        sequence: u64::MAX,
        timestamp_micros: 0,
        kind: crate::page::RendererDocumentLifecycleEventKind::Milestone(
            crate::page::RendererDocumentLifecycleMilestone::Load,
        ),
    };
    let observation = browser.wait_for_renderer_document_lifecycle(renderer, unproduced);
    tokio::pin!(observation);
    assert!(
        observation
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert!(observation.await.is_none());
    assert!(
        browser
            .subscribe()
            .unwrap()
            .0
            .document_lifecycles
            .is_empty()
    );
    service.shutdown();
}

#[tokio::test]
async fn native_document_commits_publish_exact_occurrences_and_recover_current_snapshot() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (before, mut events) = browser.subscribe().unwrap();
    assert!(before.documents.is_empty());
    let first = navigate(&context, contents, "data:text/html,<title>first</title>").await;
    let first_event = next_document_commit(&mut events, first);
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
    let second_event = next_document_commit(&mut events, second);
    assert_eq!(
        second_event.event,
        crate::browser::BrowserEvent::DocumentCommitted(second)
    );
    assert!(second_event.sequence > first_event.sequence);
    assert!(browser.document_commit_snapshot(first).is_err());
    assert_eq!(browser.document_for_renderer(first_renderer), None);
    let (current, _) = browser.subscribe().unwrap();
    assert_eq!(current.documents, [second]);
    assert!(current.sequence >= second_event.sequence);
    assert_eq!(
        context.document_commit_snapshot(second).unwrap().frame_slot,
        snapshot.frame_slot
    );
    while let Ok(event) = events.try_recv() {
        assert!(matches!(
            event.event,
            crate::browser::BrowserEvent::DocumentLifecycleChanged(_)
        ));
    }
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
        service
            .handle()
            .wait_for_renderer_document_lifecycle(renderer, event)
            .await
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
        service
            .handle()
            .wait_for_renderer_document_lifecycle(renderer, event)
            .await
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
async fn native_selection_updates_both_documents_without_replacing_them_or_their_policy() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let (context, first) = context_with_contents(&service);
    let first_document = navigate(&context, first, &server.url("/static")).await;
    let (peer, _) = context.create_web_contents(Default::default()).unwrap();
    let peer_document = navigate(&context, peer, &server.url("/static")).await;
    assert!(context.select_web_contents(first.id()));
    for (document, foreground) in [(first_document, true), (peer_document, false)] {
        let pending = context
            .start_document_page_surface_update(
                document,
                foreground,
                Some(crate::browser::EmulatedNetworkConditions::offline()),
                None,
            )
            .unwrap();
        context
            .finish_document_policy_batch(pending.wait().await)
            .unwrap();
    }
    let expression = "[document.hidden, document.hasFocus(), navigator.onLine].join(',')";
    for (selected, expected_first, expected_peer) in [
        (peer, "true,false,false", "false,true,false"),
        (first, "false,true,false", "true,false,false"),
    ] {
        assert!(context.select_web_contents(selected.id()));
        assert_eq!(context.selected_web_contents_handle(), Some(selected));
        for (document, expected) in [
            (first_document, expected_first),
            (peer_document, expected_peer),
        ] {
            assert_eq!(
                context
                    .evaluate_document_expression_for_test(document, expression, false)
                    .await
                    .unwrap()["value"],
                expected,
                "native activation must update visibility and preserve offline policy on both exact Documents"
            );
        }
        assert_eq!(
            context.document_handle(first).unwrap(),
            Some(first_document)
        );
        assert_eq!(context.document_handle(peer).unwrap(), Some(peer_document));
    }
    assert_eq!(context.loaded_document_count(), 2);
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
    context.finish_document_policy_batch(background).unwrap();
    let expression = "[document.hidden, document.hasFocus(), navigator.onLine].join(',')";
    assert_eq!(
        context
            .evaluate_document_expression_for_test(document, expression, false)
            .await
            .unwrap()["value"],
        "true,false,false"
    );
    let close = service.handle().close_web_contents(first).unwrap();
    assert_eq!(
        close.event.event,
        crate::browser::BrowserEvent::WebContentsClosed {
            web_contents: first,
            activated: Some(peer),
        }
    );
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
    let (_, mut navigation_events) = service.handle().subscribe().unwrap();
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
                    .cancel_document_navigation(contents, &stale_navigation)
                    .unwrap()
            );
            assert!(context.navigation_retains(contents, navigation).unwrap());
            assert!(
                context
                    .cancel_document_navigation(contents, &navigation)
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
    let failures = std::iter::from_fn(|| navigation_events.try_recv().ok())
        .filter_map(|record| match record.event {
            BrowserEvent::NavigationFailed { request, reason }
                if request.navigation == navigation =>
            {
                Some((request, reason))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        failures.len(),
        1,
        "every retired native request has exactly one terminal occurrence"
    );
    assert_eq!(failures[0].0.web_contents, contents);
    assert_eq!(
        failures[0].1,
        match retirement {
            NavigationRetirement::Context => NavigationFailureReason::ContextDisposed,
            NavigationRetirement::WebContents | NavigationRetirement::AllWebContents =>
                NavigationFailureReason::WebContentsClosed,
            NavigationRetirement::Supersession => NavigationFailureReason::Superseded,
            NavigationRetirement::ClearState | NavigationRetirement::CancelMatching =>
                NavigationFailureReason::Canceled,
        }
    );
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
