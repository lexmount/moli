use super::*;
use crate::browser::{
    BrowserContextStoragePartitionHandles, BrowserService, DocumentHandle, DocumentRetirement,
    StoragePartitionKind, WebContentsCreation,
};
use moli_test_support::FixtureServer;
use tokio::{io::AsyncReadExt, net::TcpListener};

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
    assert_eq!(committed.web_contents, contents.id());
    assert_eq!(committed.navigation, navigation);
    committed
        .post_response_continuation
        .expect("Browser commit must release the native DocumentCommit boundary without DevTools")
        .release();
    committed.retirement.close().await;
    // Dropping the unused inspection endpoint must not retire the Document.
    drop(committed.inspection_endpoint);
    context.document_handle(contents).unwrap().unwrap()
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
