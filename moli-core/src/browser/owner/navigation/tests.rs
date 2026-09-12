use super::*;
use crate::browser::NavigationId;
use crate::browser::{
    BrowserContextHandle, BrowserContextStoragePartitionHandles, BrowserEvent, BrowserHandle,
    BrowserService, DocumentHandle, DocumentRetirement, NavigationAttempt, NavigationFailureReason,
    NavigationRequestLoadPolicy, NavigationSnapshot, StoragePartitionKind, WebContentsCreation,
};
use moli_test_support::FixtureServer;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use url::Url;

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

#[tokio::test]
async fn native_child_document_network_completes_without_devtools() {
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        &format!(
            "data:text/html,<iframe src='{}'></iframe>",
            server.url("/static")
        ),
    )
    .await;
    let mut completed = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::NetworkRequestCompleted(occurrence)
                    if occurrence.owner == crate::browser::NetworkOwner::Document(document) =>
                {
                    completed.push(occurrence);
                }
                BrowserEvent::DocumentLifecycleChanged(snapshot)
                    if snapshot.document == document && snapshot.lifecycle.load.is_some() =>
                {
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("the native child and its parent must finish without a Protocol observer");
    assert_eq!(
        completed.len(),
        1,
        "the child's completed network fact must precede the parent's native Load"
    );
    let snapshot = browser.subscribe().unwrap().0;
    let requests = snapshot
        .network_requests
        .iter()
        .filter(|request| request.owner == crate::browser::NetworkOwner::Document(document))
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "native recovery retains the child response"
    );
    let crate::browser::NetworkRequestState::ChildDocument(response) = &requests[0].state else {
        panic!("the recovery record must identify a child Document response");
    };
    assert_eq!(response.snapshot.request_url, server.url("/static"));
    assert_eq!(response.snapshot.response.as_ref().unwrap().status, 200);
    assert!(
        String::from_utf8_lossy(
            &response
                .snapshot
                .response
                .as_ref()
                .unwrap()
                .response_body
                .as_ref()
                .unwrap()
                .clone_body_bytes()
        )
        .contains("fixture static")
    );
    let crate::page::RendererNetworkOutputItem::ChildDocument(occurred) =
        &completed[0].renderer.item
    else {
        panic!("the native occurrence must retain the same child response");
    };
    assert!(std::sync::Arc::ptr_eq(response, occurred));
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_child_document_network_failure_completes_without_devtools() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child_url = format!("http://{}/no-response", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        assert!(String::from_utf8_lossy(&request).starts_with("GET /no-response "));
        // Close the accepted request without sending an HTTP response.
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        &format!("data:text/html,<iframe src='{child_url}'></iframe>"),
    )
    .await;
    let mut completed = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap().event {
                BrowserEvent::NetworkRequestCompleted(occurrence)
                    if occurrence.owner == crate::browser::NetworkOwner::Document(document) =>
                {
                    completed.push(occurrence)
                }
                BrowserEvent::DocumentLifecycleChanged(snapshot)
                    if snapshot.document == document && snapshot.lifecycle.load.is_some() =>
                {
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("failed child transport must settle the parent without Protocol");
    server.await.unwrap();
    assert_eq!(
        completed.len(),
        1,
        "the actual failed request needs one native terminal"
    );
    let snapshot = browser.subscribe().unwrap().0;
    let requests = snapshot
        .network_requests
        .iter()
        .filter(|request| request.owner == crate::browser::NetworkOwner::Document(document))
        .collect::<Vec<_>>();
    assert_eq!(
        requests.len(),
        1,
        "recovery must retain the failed child request"
    );
    let crate::browser::NetworkRequestState::ChildDocument(activity) = &requests[0].state else {
        panic!("the failure must retain its child request identity");
    };
    assert_eq!(activity.snapshot.request_url, child_url);
    assert_eq!(activity.snapshot.request_method, "GET");
    assert!(
        !activity.snapshot.response.as_ref().unwrap_err().is_empty(),
        "no HTTP response may be fabricated for the failed fetch"
    );
    let crate::page::RendererNetworkOutputItem::ChildDocument(occurred) =
        &completed[0].renderer.item
    else {
        panic!("the event must carry the same failed child request");
    };
    assert!(std::sync::Arc::ptr_eq(activity, occurred));
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
}

#[tokio::test]
async fn native_child_document_network_precedes_held_child_script_and_load() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child_url = format!("http://{}/child", listener.local_addr().unwrap());
    let (script_started, script_request) = oneshot::channel();
    let (release_script, script_release) = oneshot::channel();
    let server = tokio::spawn(async move {
        for path in ["/child", "/held.js"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET {path} ")));
            if path == "/child" {
                let body = "<!doctype html><script src='/held.js'></script><p>child body</p>";
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            } else {
                script_started.send(()).unwrap();
                script_release.await.unwrap();
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                break;
            }
        }
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        &format!("data:text/html,<iframe src='{child_url}'></iframe>"),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), script_request)
        .await
        .unwrap()
        .unwrap();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::NetworkRequestCompleted(occurrence) = events.recv().await.unwrap().event
                && occurrence.owner == crate::browser::NetworkOwner::Document(document)
                && matches!(&occurrence.renderer.item, crate::page::RendererNetworkOutputItem::ChildDocument(response) if response.snapshot.request_url == child_url) {
                break occurrence;
            }
        }
    }).await.expect("the child response commits before its held script can finish");
    assert!(
        context
            .document_lifecycle_snapshot(document)
            .unwrap()
            .unwrap()
            .load
            .is_none()
    );
    let snapshot = browser.subscribe().unwrap().0;
    let request = snapshot.network_requests.iter().find(|request| request.owner == crate::browser::NetworkOwner::Document(document)
        && matches!(&request.state, crate::browser::NetworkRequestState::ChildDocument(response) if response.snapshot.request_url == child_url)).unwrap();
    let crate::browser::NetworkRequestState::ChildDocument(response) = &request.state else {
        unreachable!()
    };
    let crate::page::RendererNetworkOutputItem::ChildDocument(occurred) = &completed.renderer.item
    else {
        unreachable!()
    };
    assert!(std::sync::Arc::ptr_eq(response, occurred));
    assert!(
        String::from_utf8_lossy(
            &response
                .snapshot
                .response
                .as_ref()
                .unwrap()
                .response_body
                .as_ref()
                .unwrap()
                .clone_body_bytes()
        )
        .contains("child body")
    );
    release_script.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::DocumentLifecycleChanged(snapshot) =
                events.recv().await.unwrap().event
                && snapshot.document == document
                && snapshot.lifecycle.load.is_some()
            {
                break;
            }
        }
    })
    .await
    .expect("releasing the exact script must finish the parent Load");
    server.await.unwrap();
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
}

#[tokio::test]
async fn native_network_commits_request_response_and_body_without_devtools() {
    use crate::browser::NetworkRequestState;
    use crate::page::{ScriptNetworkOutputItem, SubresourceNetworkOutcome};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/native-network", listener.local_addr().unwrap());
    let (headers, release_headers) = oneshot::channel();
    let (body, release_body) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        release_headers.await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
        release_body.await.unwrap();
        stream.write_all(b"body").await.unwrap();
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(
        &context,
        contents,
        &format!("data:text/html,<script>fetch('{url}').then(r=>r.text())</script>"),
    )
    .await;
    let (handle, started) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkRequestStarted(occurrence) = event.event
                && occurrence.owner == crate::browser::NetworkOwner::Document(document)
                && let crate::page::RendererNetworkOutputItem::Resource(item) =
                    &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item.as_ref()
                && request.url().as_str() == url
            {
                break (request.handle(), event.sequence);
            }
        }
    })
    .await
    .expect("native start must not wait for response or a DevTools consumer");
    let snapshot = browser.subscribe().unwrap().0;
    assert!(snapshot.network_requests.iter().any(|request| request.owner == crate::browser::NetworkOwner::Document(document)
        && matches!(&request.state, NetworkRequestState::Started(start) if start.handle() == handle)));
    headers.send(()).unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkActivity(occurrence) = event.event
                && occurrence.owner == crate::browser::NetworkOwner::Document(document)
                && let crate::page::RendererNetworkOutputItem::Resource(item) =
                    &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceResponseStarted(response) = item.as_ref()
                && response.handle() == handle
            {
                break event.sequence;
            }
        }
    })
    .await
    .expect("native headers must not wait for the held body");
    assert!(response > started);
    assert!(browser.subscribe().unwrap().0.network_requests.iter().any(|request| request.owner == crate::browser::NetworkOwner::Document(document)
        && matches!(&request.state, NetworkRequestState::Responding { response, .. } if response.handle() == handle)));
    body.send(()).unwrap();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkRequestCompleted(occurrence) = event.event
                && occurrence.owner == crate::browser::NetworkOwner::Document(document)
                && crate::browser::network::request_key(&occurrence.renderer).is_some_and(|key| {
                    key.1 == crate::browser::network::NetworkRequestIdentity::Resource(handle.get())
                })
            {
                break event.sequence;
            }
        }
    })
    .await
    .expect("native completion needs no Protocol ingress");
    assert!(completed > response);
    let snapshot = browser.subscribe().unwrap().0;
    let records = snapshot
        .network_requests
        .iter()
        .filter_map(|request| match &request.state {
            NetworkRequestState::Recorded(record)
                if request.owner == crate::browser::NetworkOwner::Document(document)
                    && record.request_handle() == Some(handle) =>
            {
                Some(record)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    let SubresourceNetworkOutcome::Success { response_body, .. } = records[0].outcome() else {
        panic!("successful request must retain its real body");
    };
    assert_eq!(response_body.clone_body_bytes(), b"body");
    server.await.unwrap();
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if matches!(events.recv().await.unwrap().event, BrowserEvent::NetworkSourceClosed { owner: crate::browser::NetworkOwner::Document(closed), .. } if closed == document) {
                break;
            }
        }
    })
    .await
    .expect("physical source teardown must release its Network records");
    assert!(browser.subscribe().unwrap().0.network_requests.is_empty());
    service.shutdown();
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

// Uses only native Browser navigation and immutable Document observation.
#[tokio::test]
async fn native_document_titles_update_history_without_protocol_and_survive_replacement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let title_url = format!("http://{}/title", listener.local_addr().unwrap());
    let (release, released) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        released.await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\ndynamic").await.unwrap();
    });
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = service.handle().subscribe().unwrap();
    let first = navigate(&context, contents, &format!("data:text/html,<title>initial</title><script>fetch('{title_url}').then(r=>r.text()).then(t=>document.title=t)</script>")).await;
    loop {
        assert_eq!(context.document_handle(contents).unwrap(), Some(first));
        let (index, history) = context.navigation_history_snapshot(contents).unwrap();
        if history[index].title == "initial" {
            break;
        }
        events.recv().await.unwrap();
    }
    // External input drives the renderer without taking BrowserContext out of
    // its native owner through a test-only arbitrary evaluation helper.
    release.send(()).unwrap();
    loop {
        let (index, history) = context.navigation_history_snapshot(contents).unwrap();
        if history[index].title == "dynamic" {
            break;
        }
        events.recv().await.unwrap();
    }
    server.await.unwrap();
    let replacement = navigate(
        &context,
        contents,
        "data:text/html,<title>replacement</title>",
    )
    .await;
    loop {
        assert_eq!(
            context.document_handle(contents).unwrap(),
            Some(replacement)
        );
        let (index, history) = context.navigation_history_snapshot(contents).unwrap();
        assert_eq!(index, 1);
        assert_eq!(history[0].title, "dynamic");
        if history[1].title == "replacement" {
            break;
        }
        events.recv().await.unwrap();
    }
    assert!(context.start_capture_document_snapshot(first).is_err());
    service.shutdown();
}

// Uses only native Browser navigation and immutable Document observation.
async fn navigate(
    context: &BrowserContextHandle,
    contents: WebContentsHandle,
    url: &str,
) -> DocumentHandle {
    let (_, mut events) = context.browser.subscribe().unwrap();
    let document = commit_navigation(context, contents, url).await;
    while context
        .document_lifecycle_snapshot(document)
        .unwrap()
        .is_none_or(|snapshot| snapshot.dom_content_loaded.is_none())
    {
        events.recv().await.unwrap();
    }
    document
}

async fn commit_navigation(
    context: &BrowserContextHandle,
    contents: WebContentsHandle,
    url: &str,
) -> DocumentHandle {
    let (_, mut events) = context.browser.subscribe().unwrap();
    let waiter = context
        .navigate_document(
            contents,
            crate::browser::web_contents::NavigationRequestInterception::new(
                Url::parse(url).unwrap(),
                "GET".into(),
                None,
                Vec::new(),
                NavigationRequestLoadPolicy::BrowserInitiated,
            ),
        )
        .unwrap();
    let request = waiter.request();
    let completed = waiter.wait();
    tokio::pin!(completed);
    let committed = loop {
        if let Some(paused) = context.navigation_decision(contents).unwrap() {
            assert_eq!(paused.permit.navigation(), request.navigation);
            assert!(
                context
                    .resolve_navigation_decision(
                        contents,
                        paused.permit,
                        crate::browser::NavigationDecision::Continue,
                    )
                    .unwrap()
            );
        }
        tokio::select! {
            result = &mut completed => match result.unwrap() {
                crate::browser::BrowserNavigationOutcome::Document(committed) => break committed,
                crate::browser::BrowserNavigationOutcome::Download { .. } => panic!("fixture navigation must commit a Document"),
            },
            event = events.recv() => { event.unwrap(); }
        }
    };
    assert_eq!(committed.document.web_contents(), contents);
    assert_eq!(committed.document.id(), request.document);
    assert_eq!(committed.metadata.navigation, Some(request.navigation));
    let document = committed.document;
    // Dropping an observation cannot retire the Browser's live Document.
    drop(committed);
    assert_eq!(context.document_handle(contents).unwrap(), Some(document));
    document
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

#[tokio::test]
async fn native_service_worker_version_survives_stop_and_restart_without_devtools() {
    use crate::browser::{ServiceWorkerCommand, ServiceWorkerExecution, WorkerSnapshot};
    use crate::page::RendererServiceWorkerVersionStatus;

    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, &server.url("/native-service-worker/")).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut created = None;
        let mut first_run = None;
        let mut restarted = None;
        let mut phase = 0;
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(worker @ WorkerSnapshot::Service { .. }) => {
                    assert!(
                        created
                            .replace((worker.handle(), record.sequence))
                            .is_none()
                    );
                    let WorkerSnapshot::Service { worker, .. } = worker else {
                        unreachable!()
                    };
                    let ServiceWorkerExecution::Starting(run) = worker.execution else {
                        panic!("creation must expose the already installed physical host");
                    };
                    first_run = Some(run);
                }
                BrowserEvent::WorkerUpdated(snapshot @ WorkerSnapshot::Service { .. }) => {
                    let (handle, created_sequence) = created.as_ref().unwrap();
                    assert_eq!(snapshot.handle(), *handle);
                    assert!(record.sequence > *created_sequence);
                    let WorkerSnapshot::Service { worker, .. } = &snapshot else {
                        unreachable!()
                    };
                    match (phase, &worker.execution) {
                        (0, ServiceWorkerExecution::Running(run))
                            if worker.info.status
                                == RendererServiceWorkerVersionStatus::Activated =>
                        {
                            assert_eq!(Some(run), first_run.as_ref());
                            context
                                .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
                                    version_id: worker.info.version_id,
                                })
                                .unwrap();
                            phase = 1;
                        }
                        (1, ServiceWorkerExecution::Stopped) => {
                            assert_eq!(
                                browser.subscribe().unwrap().0.workers,
                                vec![snapshot.clone()]
                            );
                            context
                                .execute_service_worker_command(ServiceWorkerCommand::Start {
                                    scope: worker.info.scope_url.parse().unwrap(),
                                })
                                .unwrap();
                            phase = 2;
                        }
                        (2, ServiceWorkerExecution::Starting(run)) => {
                            assert_ne!(Some(run), first_run.as_ref());
                            assert!(restarted.replace(run.clone()).is_none());
                        }
                        (2, ServiceWorkerExecution::Running(run)) => {
                            assert_eq!(Some(run), restarted.as_ref());
                            assert_eq!(
                                browser.subscribe().unwrap().0.workers,
                                vec![snapshot.clone()]
                            );
                            assert!(context.remove().unwrap());
                            phase = 3;
                        }
                        _ => {}
                    }
                }
                BrowserEvent::WorkerDestroyed(handle) => {
                    assert_eq!(phase, 3);
                    assert_eq!(handle, created.unwrap().0);
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("native ServiceWorker creation, restart and retirement need no Protocol consumer");
    assert!(browser.subscribe().unwrap().0.workers.is_empty());
    service.shutdown();
    while let Ok(record) = events.try_recv() {
        assert!(!matches!(record.event, BrowserEvent::WorkerDestroyed(_)));
    }
    server.shutdown().await;
}

#[tokio::test]
async fn native_service_worker_failed_install_preserves_creation_and_exact_retirement() {
    use crate::browser::WorkerSnapshot;
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    // Inline script runs in the resident Browser-owned Document. The generic
    // test-only Context-borrowing evaluator temporarily removes its registry
    // entry and cannot be used to test native lifecycle admission.
    navigate(
        &context,
        contents,
        &server.url("/native-service-worker/failed-install"),
    )
    .await;
    let mut observed = Vec::new();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut created = None;
        loop {
            let record = events.recv().await.unwrap();
            observed.push(record.event.clone());
            match record.event {
                BrowserEvent::WorkerCreated(worker @ WorkerSnapshot::Service { .. }) => {
                    assert!(
                        created
                            .replace((worker.handle(), record.sequence))
                            .is_none()
                    );
                }
                BrowserEvent::WorkerDestroyed(handle) => {
                    let (expected, sequence) =
                        created.expect("even a failed initial run publishes its version creation");
                    assert_eq!(handle, expected);
                    assert!(record.sequence > sequence);
                    break;
                }
                _ => {}
            }
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "failed install facts: {observed:?}; snapshot: {:?}",
        browser.subscribe().unwrap().0.workers
    );
    assert!(browser.subscribe().unwrap().0.workers.is_empty());
    service.shutdown();
    while let Ok(record) = events.try_recv() {
        assert!(!matches!(record.event, BrowserEvent::WorkerDestroyed(_)));
    }
    server.shutdown().await;
}

#[tokio::test]
async fn native_shared_worker_network_completes_before_retirement_without_devtools() {
    shared_worker_network_before_close(
        "onconnect=()=>fetch('data:text/plain,native-worker-network').then(r=>r.text()).then(()=>close())",
        "data:text/plain,native-worker-network",
    ).await;
}

#[tokio::test]
async fn native_shared_worker_request_pause_releases_when_policy_document_closes() {
    shared_worker_pause_survives_observer_retirement(false).await;
}

#[tokio::test]
async fn native_shared_worker_response_pause_releases_when_policy_document_closes() {
    shared_worker_pause_survives_observer_retirement(true).await;
}

async fn shared_worker_pause_survives_observer_retirement(response_stage: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let script = "data:text/javascript,const ports=[];onconnect=async e=>{const p=e.ports[0];ports.push(p);p.start();if(ports.length===2){const r=await fetch('data:text/plain,native-release');const text=await r.text();for(const port of ports)port.postMessage(text);}};";
        let html = format!(
            "<!doctype html><script>const worker=new SharedWorker({script:?},'native-pause');worker.port.onmessage=e=>document.title=e.data;worker.port.start();</script>"
        );
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes()).await.unwrap();
        }
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    context
        .install_web_contents_fetch_interception_policy(
            contents,
            true,
            Some(crate::page::SubresourceResourceType::Fetch),
        )
        .unwrap();
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(&context, contents, &url).await;
    let (peer, _) = context.create_web_contents(Default::default()).unwrap();
    let peer_document = navigate(&context, peer, &url).await;
    let mut pause = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let BrowserEvent::WorkerFetchPaused(pause) = events.recv().await.unwrap().event {
                break pause;
            }
        }
    })
    .await
    .expect("the real SharedWorker must pause without any Protocol consumer");
    assert_eq!(pause.document, document);
    assert!(matches!(
        pause.pause.stage(),
        crate::page::RendererWorkerFetchStage::Request(_)
    ));
    if response_stage {
        context
            .start_worker_fetch_decision(
                pause,
                crate::page::WorkerFetchDecision::ContinueRequest {
                    url: None,
                    method: None,
                    body: None,
                    headers: None,
                    intercept_response: true,
                    handle_auth_requests: false,
                },
            )
            .unwrap()
            .wait()
            .await
            .unwrap();
        pause = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::WorkerFetchPaused(pause) = events.recv().await.unwrap().event {
                    break pause;
                }
            }
        })
        .await
        .expect("the response stage must be owned by the physical Worker");
        assert!(matches!(
            pause.pause.stage(),
            crate::page::RendererWorkerFetchStage::Response(_)
        ));
    }
    let snapshot = browser.subscribe().unwrap().0;
    assert_eq!(snapshot.worker_fetch_pauses, vec![pause.clone()]);
    assert!(matches!(
        pause.worker,
        crate::browser::WorkerHandle::Shared { .. }
    ));
    context
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert!(
        !pause.pause.is_available(),
        "retained snapshots must not keep retired policy authority alive"
    );
    assert!(
        context
            .start_worker_fetch_decision(
                pause.clone(),
                crate::page::WorkerFetchDecision::Fail("stale".into())
            )
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            assert_eq!(context.document_handle(peer).unwrap(), Some(peer_document));
            let (index, history) = context.navigation_history_snapshot(peer).unwrap();
            if history[index].title == "native-release" {
                break;
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("the surviving client's script must receive the released response");
    let snapshot = browser.subscribe().unwrap().0;
    assert!(snapshot.worker_fetch_pauses.is_empty());
    assert!(
        snapshot
            .workers
            .iter()
            .any(|worker| worker.handle() == pause.worker)
    );
    assert!(snapshot.network_requests.iter().any(|request| request.owner == crate::browser::NetworkOwner::Worker(pause.worker)
        && matches!(&request.state, crate::browser::NetworkRequestState::Recorded(record) if record.url().as_str() == "data:text/plain,native-release")));
    service.shutdown();
    server.await.unwrap();
}

#[tokio::test]
async fn native_shared_worker_network_xhr_success_and_fetch_failure_without_devtools() {
    for (script, url, success) in [
        (
            "onconnect=()=>{const x = new XMLHttpRequest(); x.open('GET', 'data:text/plain,native-xhr-body'); x.onload=()=>close(); x.send();}",
            "data:text/plain,native-xhr-body",
            true,
        ),
        (
            "onconnect=()=>fetch('http://127.0.0.1:1/native-worker-failure').catch(()=>close())",
            "http://127.0.0.1:1/native-worker-failure",
            false,
        ),
    ] {
        let occurrence = shared_worker_network_before_close(script, url).await;
        let crate::page::RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
        else {
            unreachable!();
        };
        let crate::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) = item.as_ref()
        else {
            unreachable!();
        };
        assert_eq!(
            matches!(
                record.outcome(),
                crate::page::SubresourceNetworkOutcome::Success { .. }
            ),
            success
        );
    }
}

async fn shared_worker_network_before_close(
    script: &str,
    url: &str,
) -> crate::browser::NetworkOccurrence {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let script = format!("data:text/javascript,{script}");
    navigate(&context, contents, &format!("data:text/html,<script>globalThis.worker = new SharedWorker({script:?}, 'native-network')</script>")).await;
    let mut worker = None;
    let mut completed = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(created @ crate::browser::WorkerSnapshot::Shared { .. }) => {
                    assert!(worker.replace(created.handle()).is_none());
                }
                BrowserEvent::NetworkRequestCompleted(occurrence) => {
                    if matches!(&occurrence.renderer.item, crate::page::RendererNetworkOutputItem::Resource(item)
                        if matches!(item.as_ref(), crate::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(result)
                            if result.url().as_str() == url)) {
                        completed.push(occurrence);
                    }
                }
                BrowserEvent::WorkerDestroyed(closed) if Some(closed) == worker => break,
                _ => {}
            }
        }
    }).await.expect("the real worker completes fetch and closes without a Protocol observer");
    assert!(worker.is_some());
    assert_eq!(
        completed.len(),
        1,
        "worker retirement must not erase an uncommitted Network fact for {url}"
    );
    let worker = worker.unwrap();
    assert_eq!(
        completed[0].owner,
        crate::browser::NetworkOwner::Worker(worker)
    );
    let crate::browser::WorkerHandle::Shared { instance, .. } = worker else {
        unreachable!();
    };
    assert_eq!(
        completed[0].renderer.source,
        crate::page::RendererNetworkSource::Worker(crate::page::RendererWorkerIdentity::Shared(
            instance
        ))
    );
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
    completed.pop().unwrap()
}

#[tokio::test]
async fn native_shared_worker_membership_and_retirement_do_not_require_devtools() {
    use crate::browser::{WorkerHandle, WorkerSnapshot};
    for retirement in ["worker", "context", "browser"] {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (context, contents) = context_with_contents(&service);
        let (_, mut events) = browser.subscribe().unwrap();
        navigate(&context, contents, "data:text/html,<script>globalThis.worker = new SharedWorker('data:text/javascript,onconnect = () => {}', 'native-membership')</script>").await;
        let created = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = events.recv().await.unwrap();
                if let BrowserEvent::WorkerCreated(WorkerSnapshot::Shared { context: id, info }) =
                    &record.event
                    && *id == context.id()
                    && info.name == "native-membership"
                {
                    break record;
                }
            }
        })
        .await
        .expect("Browser must observe the real worker without a DevTools transport");
        let BrowserEvent::WorkerCreated(worker) = &created.event else {
            unreachable!()
        };
        assert_eq!(browser.subscribe().unwrap().0.workers, vec![worker.clone()]);
        let WorkerHandle::Shared { instance, .. } = worker.handle() else {
            unreachable!()
        };
        match retirement {
            "worker" => assert!(context.close_shared_worker(instance)),
            "context" => assert!(context.remove().unwrap()),
            "browser" => service.shutdown(),
            _ => unreachable!(),
        }
        let destroyed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = events.recv().await.unwrap();
                if record.event == BrowserEvent::WorkerDestroyed(worker.handle()) {
                    break record;
                }
            }
        })
        .await
        .expect("retirement must publish its exact native Worker occurrence");
        assert!(destroyed.sequence > created.sequence);
        if retirement != "browser" {
            // An owner boundary also drains callbacks queued by physical close.
            assert!(browser.subscribe().unwrap().0.workers.is_empty());
            service.shutdown();
        }
        while let Ok(record) = events.try_recv() {
            assert_ne!(
                record.event,
                BrowserEvent::WorkerDestroyed(worker.handle()),
                "late renderer close must not publish a duplicate destruction"
            );
        }
    }
}

#[tokio::test]
async fn native_service_worker_network_is_owned_by_each_real_run_without_devtools() {
    use crate::browser::{
        NetworkOwner, ServiceWorkerCommand, ServiceWorkerExecution, WorkerSnapshot,
    };
    use crate::page::{
        RendererNetworkOutputItem, RendererNetworkSource, RendererWorkerIdentity,
        ScriptNetworkOutputItem, SubresourceNetworkOutcome,
    };
    let server = FixtureServer::spawn().await.unwrap();
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, &server.url("/native-worker-network/")).await;
    let mut previous_run = None;
    let mut version = None;
    for attempt in 0..2 {
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::NetworkRequestCompleted(occurrence) = events.recv().await.unwrap().event
                    && matches!(&occurrence.renderer.item, RendererNetworkOutputItem::Resource(item)
                        if matches!(item.as_ref(), ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                            if record.url().as_str() == server.url("/native-worker-network/probe"))) {
                    break occurrence;
                }
            }
        }).await.expect("a real Service Worker fetch commits without Protocol consumption");
        let RendererNetworkSource::Worker(RendererWorkerIdentity::Service {
            version: current_version,
            run,
        }) = &completed.renderer.source
        else {
            panic!("Service Worker response must retain its exact physical run");
        };
        assert_ne!(Some(run), previous_run.as_ref());
        if let Some(version) = version {
            assert_eq!(*current_version, version);
        }
        version = Some(*current_version);
        previous_run = Some(run.clone());
        assert_eq!(
            completed.owner,
            NetworkOwner::Worker(crate::browser::WorkerHandle::Service {
                context: context.id(),
                version: *current_version
            })
        );
        let snapshot = browser.subscribe().unwrap().0;
        let retained = snapshot
            .network_requests
            .iter()
            .find(|request| request.renderer_source == completed.renderer.source)
            .unwrap();
        assert_eq!(retained.owner, completed.owner);
        let crate::browser::NetworkRequestState::Recorded(record) = &retained.state else {
            panic!("the complete-only producer must not invent a Started phase");
        };
        let SubresourceNetworkOutcome::Success { response_body, .. } = record.outcome() else {
            panic!("the real fixture response must succeed");
        };
        assert_eq!(
            response_body.clone_body_bytes(),
            b"native worker network body"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if browser.subscribe().unwrap().0.workers.iter().any(|snapshot| matches!(snapshot, WorkerSnapshot::Service { worker, .. }
                    if worker.info.version_id == *current_version
                        && worker.info.status == crate::page::RendererServiceWorkerVersionStatus::Activated
                        && matches!(worker.execution, ServiceWorkerExecution::Running(_))
                        && worker.execution.active_run() == Some(run))) { break; }
                events.recv().await.unwrap();
            }
        }).await.expect("install and activation settle after the first fetch");
        context
            .execute_service_worker_command(ServiceWorkerCommand::StopVersion {
                version_id: *current_version,
            })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut stopped = false;
            let mut closed = false;
            while !stopped || !closed {
                match events.recv().await.unwrap().event {
                    BrowserEvent::WorkerUpdated(WorkerSnapshot::Service { worker, .. })
                        if worker.info.version_id == *current_version
                            && worker.execution == ServiceWorkerExecution::Stopped =>
                    {
                        stopped = true
                    }
                    BrowserEvent::NetworkSourceClosed { source, .. }
                        if source == completed.renderer.source.identity() =>
                    {
                        closed = true
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the exact native execution and its Network retention must retire");
        assert!(
            browser
                .subscribe()
                .unwrap()
                .0
                .network_requests
                .iter()
                .all(|request| request.renderer_source != completed.renderer.source)
        );
        if attempt == 0 {
            context
                .execute_service_worker_command(ServiceWorkerCommand::Start {
                    scope: server.url("/native-worker-network/").parse().unwrap(),
                })
                .unwrap();
        }
    }
    service.shutdown();
    server.shutdown().await;
}

#[tokio::test]
async fn native_dedicated_worker_network_completes_before_retirement_without_devtools() {
    dedicated_worker_network_before_close(
        "fetch('data:text/plain,native-dedicated-network').then(r=>r.text()).then(()=>close())",
        &[("data:text/plain,native-dedicated-network", true)],
    )
    .await;
}

#[tokio::test]
async fn native_nested_worker_network_keeps_each_physical_owner_without_devtools() {
    let child = "data:text/javascript,fetch('data:text/plain,native-nested-network').then(r=>r.text()).then(()=>{postMessage('done');close()})";
    dedicated_worker_network_before_close(
        &format!("globalThis.child=new Worker({child:?}, {{name:'native-nested-network'}});child.onmessage=()=>fetch('data:text/plain,native-parent-network').then(r=>r.text()).then(()=>close())"),
        &[("data:text/plain,native-nested-network", true), ("data:text/plain,native-parent-network", true)],
    ).await;
}

#[tokio::test]
async fn native_dedicated_worker_network_xhr_success_and_fetch_failure_without_devtools() {
    dedicated_worker_network_before_close(
        "const x=new XMLHttpRequest(); x.onload=()=>close(); x.open('GET','data:text/plain,native-dedicated-xhr'); x.send()",
        &[("data:text/plain,native-dedicated-xhr", true)],
    ).await;
    dedicated_worker_network_before_close(
        "fetch('http://127.0.0.1:1/native-dedicated-failure').catch(()=>close())",
        &[("http://127.0.0.1:1/native-dedicated-failure", false)],
    )
    .await;
}

async fn dedicated_worker_network_before_close(script: &str, urls: &[(&str, bool)]) {
    use crate::browser::{NetworkOwner, WorkerHandle, WorkerSnapshot};
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let script = format!("data:text/javascript,{script}");
    navigate(&context, contents, &format!("data:text/html,<script>globalThis.worker=new Worker({script:?}, {{name:'native-dedicated-network'}})</script>")).await;
    let mut root = None;
    let mut created = Vec::new();
    let mut completed = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(worker @ WorkerSnapshot::Dedicated { .. }) => {
                    let handle = worker.handle();
                    let WorkerSnapshot::Dedicated { worker, .. } = worker else { unreachable!() };
                    if worker.info.name == "native-dedicated-network" {
                        assert!(root.replace(handle).is_none());
                    }
                    assert!(!created.contains(&handle));
                    created.push(handle);
                }
                BrowserEvent::NetworkRequestCompleted(occurrence) => {
                    if matches!(&occurrence.renderer.item, crate::page::RendererNetworkOutputItem::Resource(item)
                        if matches!(item.as_ref(), crate::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(result)
                            if urls.iter().any(|(url, _)| *url == result.url().as_str()))) {
                        completed.push(occurrence);
                    }
                }
                BrowserEvent::WorkerDestroyed(closed) if Some(closed) == root => break,
                _ => {}
            }
        }
    }).await.expect("the real Dedicated Worker finishes its own and nested requests before closing");
    assert!(root.is_some());
    assert_eq!(
        completed.len(),
        urls.len(),
        "retirement must follow the native Network facts"
    );
    assert_eq!(
        created.len(),
        urls.len(),
        "every physical Worker has native membership"
    );
    let mut owners = std::collections::HashSet::new();
    for occurrence in completed {
        let crate::page::RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
        else {
            unreachable!()
        };
        let crate::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) = item.as_ref()
        else {
            unreachable!()
        };
        let expected = urls
            .iter()
            .find(|(url, _)| *url == record.url().as_str())
            .unwrap()
            .1;
        assert_eq!(
            matches!(
                record.outcome(),
                crate::page::SubresourceNetworkOutcome::Success { .. }
            ),
            expected
        );
        let NetworkOwner::Worker(worker @ WorkerHandle::Dedicated { .. }) = occurrence.owner else {
            panic!("Dedicated Worker Network must not be donated to its parent Page");
        };
        assert!(created.contains(&worker));
        assert!(
            owners.insert(worker),
            "the nested request cannot reuse its parent's identity"
        );
    }
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
}

#[tokio::test]
async fn native_dedicated_worker_membership_and_document_retirement_need_no_devtools() {
    for retirement in ["worker", "document", "contents", "context", "browser"] {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (context, contents) = context_with_contents(&service);
        let (_, mut events) = browser.subscribe().unwrap();
        navigate(&context, contents, "data:text/html,<script>globalThis.worker = new Worker('data:text/javascript,onmessage = () => {}', {name: 'native-dedicated'})</script>").await;
        let created = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = events.recv().await.unwrap();
                if let BrowserEvent::WorkerCreated(worker) = record.event {
                    break (worker, record.sequence);
                }
            }
        })
        .await
        .expect("DedicatedWorker creation must reach Browser without DevTools");
        assert_eq!(browser.subscribe().unwrap().0.workers.len(), 1);
        let crate::browser::WorkerHandle::Dedicated { instance, .. } = created.0.handle() else {
            panic!("DedicatedWorker must have its own physical identity");
        };
        let script = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let BrowserEvent::WorkerUpdated(worker) = events.recv().await.unwrap().event
                    && worker.handle() == created.0.handle()
                    && let crate::browser::WorkerSnapshot::Dedicated { worker, .. } = worker
                {
                    break worker
                        .main_script
                        .expect("completion stores the main-script fact");
                }
            }
        })
        .await
        .expect("main-script completion must be native too");
        assert!(matches!(
            script.outcome,
            crate::page::RendererDedicatedWorkerMainScriptOutcome::Loaded(_)
        ));
        let snapshot = browser.subscribe().unwrap().0;
        let crate::browser::WorkerSnapshot::Dedicated { worker, .. } = &snapshot.workers[0] else {
            unreachable!()
        };
        assert!(std::sync::Arc::ptr_eq(
            worker.main_script.as_ref().unwrap(),
            &script
        ));
        match retirement {
            "worker" => assert!(context.close_dedicated_worker(instance)),
            "document" => {
                navigate(&context, contents, "data:text/html,replacement").await;
            }
            "contents" => {
                browser
                    .close_web_contents(contents)
                    .unwrap()
                    .close_async()
                    .await;
            }
            "context" => {
                assert!(context.remove().unwrap());
            }
            "browser" => service.shutdown(),
            _ => unreachable!(),
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let record = events.recv().await.unwrap();
                if record.event == BrowserEvent::WorkerDestroyed(created.0.handle()) {
                    assert!(record.sequence > created.1);
                    break;
                }
            }
        })
        .await
        .expect("exact DedicatedWorker retirement must reach Browser");
        if retirement != "browser" {
            assert!(browser.subscribe().unwrap().0.workers.is_empty());
            service.shutdown();
        }
        while let Ok(record) = events.try_recv() {
            assert_ne!(
                record.event,
                BrowserEvent::WorkerDestroyed(created.0.handle())
            );
        }
    }
}

#[tokio::test]
async fn native_nested_worker_execution_retires_with_each_parent_boundary_without_devtools() {
    use crate::browser::{NetworkOwner, WorkerHandle, WorkerSnapshot};
    for retirement in ["worker", "document", "contents", "context", "browser"] {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (context, contents) = context_with_contents(&service);
        let (_, mut events) = browser.subscribe().unwrap();
        let child = "data:text/javascript,fetch('data:text/plain,nested-ready').then(r=>r.text()).then(()=>postMessage('ready'))";
        let script = format!(
            "data:text/javascript,globalThis.child=new Worker({child:?},{{name:'child'}});child.onmessage=()=>fetch('data:text/plain,parent-ready').then(r=>r.text())"
        );
        navigate(&context, contents, &format!("data:text/html,<script>globalThis.worker=new Worker({script:?},{{name:'parent'}})</script>")).await;
        let mut created = std::collections::HashSet::new();
        let mut parent = None;
        let mut child_owner = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match events.recv().await.unwrap().event {
                    BrowserEvent::WorkerCreated(worker @ WorkerSnapshot::Dedicated { .. }) => {
                        let handle = worker.handle();
                        let WorkerSnapshot::Dedicated { worker, .. } = worker else { unreachable!() };
                        assert!(created.insert(handle));
                        if worker.info.name == "parent" { parent = Some(handle); }
                        else { assert_eq!(worker.info.name, "child"); child_owner = Some(worker.info.owner); }
                    }
                    BrowserEvent::NetworkRequestCompleted(occurrence) if matches!(
                        &occurrence.renderer.item, crate::page::RendererNetworkOutputItem::Resource(item)
                        if matches!(item.as_ref(), crate::page::ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                            if record.url().as_str() == "data:text/plain,parent-ready")) => {
                        assert_eq!(occurrence.owner, NetworkOwner::Worker(parent.unwrap()));
                        break;
                    }
                    _ => {}
                }
            }
        }).await.expect("both real Worker threads must execute before the retirement trigger");
        let parent = parent.unwrap();
        let WorkerHandle::Dedicated { instance, .. } = parent else {
            unreachable!()
        };
        assert_eq!(created.len(), 2);
        assert_eq!(
            child_owner,
            Some(crate::page::RendererDedicatedWorkerOwner::Worker(
                crate::page::RendererWorkerIdentity::Dedicated(instance),
            ))
        );
        match retirement {
            "worker" => assert!(context.close_dedicated_worker(instance)),
            "document" => {
                navigate(&context, contents, "data:text/html,replacement").await;
            }
            "contents" => {
                browser
                    .close_web_contents(contents)
                    .unwrap()
                    .close_async()
                    .await;
            }
            "context" => {
                assert!(context.remove().unwrap());
            }
            "browser" => service.shutdown(),
            _ => unreachable!(),
        }
        let mut retired = std::collections::HashSet::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while retired.len() != created.len() {
                if let BrowserEvent::WorkerDestroyed(handle) = events.recv().await.unwrap().event {
                    assert!(created.contains(&handle));
                    assert!(retired.insert(handle));
                }
            }
        })
        .await
        .expect("terminating a parent must terminate its live nested execution too");
        if retirement != "browser" {
            assert!(browser.subscribe().unwrap().0.workers.is_empty());
            service.shutdown();
        }
        while let Ok(record) = events.try_recv() {
            assert!(
                !matches!(record.event, BrowserEvent::WorkerDestroyed(handle) if retired.contains(&handle))
            );
        }
    }
}

#[tokio::test]
async fn native_short_lived_dedicated_worker_preserves_occurrences_without_devtools() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, "data:text/html,<script>globalThis.worker = new Worker('data:text/javascript,close()')</script>").await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut created = None;
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(worker) => {
                    assert!(
                        created
                            .replace((worker.handle(), record.sequence))
                            .is_none()
                    );
                }
                BrowserEvent::WorkerDestroyed(handle) => {
                    let (expected, sequence) = created.expect("creation precedes destruction");
                    assert_eq!(handle, expected);
                    assert!(record.sequence > sequence);
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("a transient DedicatedWorker cannot disappear between native observations");
    assert!(browser.subscribe().unwrap().0.workers.is_empty());
    service.shutdown();
}

#[tokio::test]
async fn native_dedicated_worker_failed_script_retires_after_its_completion_fact() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, "data:text/html,<script>globalThis.worker = new Worker('ftp://example.test/worker.js')</script>").await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut created = None;
        let mut completed = None;
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(worker) => {
                    assert!(created.replace((worker.handle(), record.sequence)).is_none());
                }
                BrowserEvent::WorkerUpdated(worker) => {
                    let (handle, sequence) = created.expect("script completion follows creation");
                    assert_eq!(worker.handle(), handle);
                    assert!(record.sequence > sequence);
                    let crate::browser::WorkerSnapshot::Dedicated { worker, .. } = worker else { unreachable!() };
                    let script = worker.main_script.unwrap();
                    assert!(matches!(&script.outcome,
                        crate::page::RendererDedicatedWorkerMainScriptOutcome::Failed { error_message, .. }
                        if !error_message.is_empty()));
                    assert!(completed.replace(record.sequence).is_none());
                }
                BrowserEvent::WorkerDestroyed(handle) => {
                    assert_eq!(handle, created.unwrap().0);
                    assert!(record.sequence > completed.expect("failure must be observable before retirement"));
                    break;
                }
                _ => {}
            }
        }
    }).await.expect("failed Worker script must complete and retire without DevTools");
    assert!(browser.subscribe().unwrap().0.workers.is_empty());
    service.shutdown();
}

#[tokio::test]
async fn native_short_lived_shared_worker_preserves_both_occurrences_without_devtools() {
    use crate::browser::WorkerSnapshot;
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    navigate(&context, contents, "data:text/html,<script>globalThis.worker = new SharedWorker('data:text/javascript,onconnect = () => close()', 'native-short-lived')</script>").await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut created = None;
        loop {
            let record = events.recv().await.unwrap();
            match record.event {
                BrowserEvent::WorkerCreated(
                    worker @ WorkerSnapshot::Shared { context: id, .. },
                ) if id == context.id() => {
                    assert!(
                        created
                            .replace((worker.handle(), record.sequence))
                            .is_none()
                    );
                }
                BrowserEvent::WorkerDestroyed(handle) => {
                    let (expected, sequence) =
                        created.expect("destruction must follow the actual creation");
                    assert_eq!(handle, expected);
                    assert!(record.sequence > sequence);
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("a transient Worker cannot disappear between native observations");
    assert!(browser.subscribe().unwrap().0.workers.is_empty());
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
            BrowserEvent::DocumentLifecycleChanged(_) | BrowserEvent::DocumentTitleChanged(_) => {}
            BrowserEvent::NavigationResponseChanged(request) => {
                assert_eq!(request.web_contents, document.web_contents());
                if request.document == document.id() {
                    assert!(started, "a response cannot precede its admission");
                }
            }
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
        let document = commit_navigation(
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
    let document = commit_navigation(
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
        match event.event {
            BrowserEvent::DocumentLifecycleChanged(_) | BrowserEvent::DocumentTitleChanged(_) => {}
            BrowserEvent::NavigationResponseChanged(request) => {
                assert_eq!(request.web_contents, contents);
                assert!([first.id(), second.id()].contains(&request.document));
            }
            _ => panic!("unexpected event after exact Document commit: {event:?}"),
        }
    }
    let _provider = browser.register_document_decision_provider().unwrap();
    let pending = context
        .navigate_document(
            contents,
            crate::browser::web_contents::NavigationRequestInterception::new(
                Url::parse("data:text/html,pending").unwrap(),
                "GET".into(),
                None,
                Vec::new(),
                NavigationRequestLoadPolicy::BrowserInitiated,
            ),
        )
        .unwrap();
    let request = pending.request();
    let renderer = loop {
        if let Some(paused) = context.navigation_decision(contents).unwrap() {
            assert_eq!(paused.permit.navigation(), request.navigation);
            if let crate::browser::NavigationDecisionStage::PreparedDocument { renderer, .. } =
                paused.stage
            {
                break renderer;
            }
            assert!(
                context
                    .resolve_navigation_decision(
                        contents,
                        paused.permit,
                        crate::browser::NavigationDecision::Continue
                    )
                    .unwrap()
            );
        }
        events.recv().await.unwrap();
    };
    assert_eq!(
        browser.document_for_renderer(renderer),
        Some(DocumentHandle::new(contents, request.document))
    );
    drop(pending);
    assert!(
        context
            .navigation_retains(contents, request.navigation)
            .unwrap(),
        "dropping an observation cannot cancel its Browser-owned work"
    );
    browser
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    assert!(browser.document_commit_snapshot(second).is_err());
    assert_eq!(browser.document_for_renderer(renderer), None);
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
async fn native_activation_survives_outgoing_document_retirement() {
    activation_survives_document_retirement(false).await;
}

#[tokio::test]
async fn native_activation_survives_selected_document_retirement() {
    activation_survives_document_retirement(true).await;
}

async fn activation_survives_document_retirement(retire_selected: bool) {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, first) = context_with_contents(&service);
    let first_document = navigate(&context, first, "data:text/html,first").await;
    let (peer, _) = context.create_web_contents(Default::default()).unwrap();
    let peer_document = navigate(&context, peer, "data:text/html,peer").await;
    browser
        .activate_web_contents(first)
        .unwrap()
        .wait()
        .await
        .unwrap();
    let retired = if retire_selected {
        peer_document
    } else {
        first_document
    };
    let policy = context
        .start_document_policy_update(
            retired,
            crate::browser::DocumentPolicyUpdate::CpuThrottlingRate(2.0),
        )
        .unwrap()
        .wait()
        .await;
    let lifetime = context.observe_document_lifetime(retired).unwrap();
    let (_, mut events) = browser.subscribe().unwrap();
    let (activation, closed) = browser
        .execute(move |browser| {
            let activation = browser.activate_web_contents(peer).unwrap();
            // The spawned activation completion cannot run until this owner turn
            // ends. Retire the real Document here, before either surface result
            // can be applied, without a timing delay or production test hook.
            let context = browser.context_mut(first.context()).unwrap();
            let retirement = context.retire_document(retired.web_contents()).unwrap();
            assert!(context.document(retired).is_err());
            let (closed, completion) = oneshot::channel();
            tokio::task::spawn_local(async move {
                retirement.close().await;
                let _ = closed.send(());
            });
            (activation, completion)
        })
        .unwrap();
    let result = activation.wait().await;
    closed.await.unwrap();
    assert_eq!(lifetime.wait().await, DocumentRetirement::Superseded);
    let event =
        result.expect("retired surface work cannot fail a committed WebContents activation");
    assert_eq!(
        event.event,
        BrowserEvent::WebContentsActivated {
            web_contents: peer,
            previous: Some(first),
        }
    );
    let selection = context.selected_web_contents_snapshot().unwrap();
    assert_eq!(selection.web_contents, peer);
    assert_eq!(selection.sequence, event.sequence);
    let mut occurrences = 0;
    while let Ok(observed) = events.try_recv() {
        if observed == event {
            occurrences += 1;
        }
    }
    assert_eq!(
        occurrences, 1,
        "activation publishes one committed occurrence"
    );

    let replacement = navigate(
        &context,
        retired.web_contents(),
        "data:text/html,replacement",
    )
    .await;
    assert_ne!(replacement, retired);
    assert_eq!(context.selected_web_contents_handle(), Some(peer));
    assert_eq!(
        context.finish_document_policy_update(policy),
        Err("Document changed".into()),
        "ordinary Document policy completion must still reject a replacement"
    );
    assert!(context.start_capture_document_snapshot(retired).is_err());
    assert_eq!(
        context.document_handle(retired.web_contents()).unwrap(),
        Some(replacement)
    );
    service.shutdown();
}

#[tokio::test]
async fn native_activation_completion_preserves_a_later_selection() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, first) = context_with_contents(&service);
    let first_document = navigate(&context, first, "data:text/html,first").await;
    let (peer, _) = context.create_web_contents(Default::default()).unwrap();
    let peer_document = navigate(&context, peer, "data:text/html,peer").await;
    browser
        .activate_web_contents(first)
        .unwrap()
        .wait()
        .await
        .unwrap();
    let (earlier, later) = browser
        .execute(move |browser| {
            let earlier = browser.activate_web_contents(peer).unwrap();
            let later = browser.activate_web_contents(first).unwrap();
            (earlier, later)
        })
        .unwrap();
    let later = later.wait().await.unwrap();
    let earlier = earlier.wait().await.unwrap();
    assert!(earlier.sequence < later.sequence);
    assert_eq!(
        earlier.event,
        BrowserEvent::WebContentsActivated {
            web_contents: peer,
            previous: Some(first)
        }
    );
    assert_eq!(
        later.event,
        BrowserEvent::WebContentsActivated {
            web_contents: first,
            previous: Some(peer)
        }
    );
    assert_eq!(
        context.selected_web_contents_snapshot().unwrap(),
        crate::browser::WebContentsSelection {
            web_contents: first,
            sequence: later.sequence,
        }
    );
    for (document, expected) in [
        (first_document, "false,true"),
        (peer_document, "true,false"),
    ] {
        assert_eq!(
            context
                .evaluate_document_expression_for_test(
                    document,
                    "[document.hidden, document.hasFocus()].join(',')",
                    false
                )
                .await
                .unwrap()["value"],
            expected,
            "late completion must not replay an earlier visibility update"
        );
    }
    service.shutdown();
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
        let runtime = context.read_live(crate::browser::BrowserContext::worker_runtime_for_test);
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
    let (_, mut navigation_events) = service.handle().subscribe().unwrap();
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
    let navigation = waiter.request().navigation;
    let fetching = tokio::spawn(waiter.wait());
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
    let result = fetching.await.unwrap();
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
    assert!(
        !context
            .navigation_retains(contents, navigation)
            .unwrap_or(false),
        "a late fetch completion must not restore its retired navigation"
    );
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
