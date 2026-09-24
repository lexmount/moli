#[tokio::test]
async fn streaming_body_failure_preserves_source_for_renderer_and_navigation() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (chunks_tx, chunks_rx) = tokio::sync::mpsc::unbounded_channel();
            chunks_tx.send(b"partial body".to_vec()).unwrap();
            drop(chunks_tx);
            let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
            finish_tx
                .send(Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "typed body transport failure",
                ))
                .context("transport completion failed")))
                .unwrap();
            let response = moli_fetch::StreamingRawResponse::new(
                url::Url::parse("https://example.test/document").unwrap(),
                200,
                Vec::new(),
                None,
                Vec::new(),
                false,
                Vec::new(),
                chunks_rx,
                moli_fetch::FetchCancelHandle::new(),
                finish_rx,
            );
            let (body_tx, mut body_rx) = tokio::sync::mpsc::channel(1);
            let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
            let (owner, _operations) = tokio::sync::mpsc::unbounded_channel();
            let request = crate::browser::NavigationRequest {
                web_contents: crate::browser::WebContentsHandle::new(
                    crate::browser::BrowserContextId::allocate(),
                    crate::browser::WebContentsId::allocate(),
                ),
                navigation: crate::browser::NavigationId::allocate(),
                document: crate::browser::DocumentId::allocate(),
            };
            let writer = std::rc::Rc::new(super::NativeBodyWriter::new(&owner, request, response));
            let capture = super::spawn_streaming_body_capture(writer, None, body_tx, completion_tx);
            assert_eq!(body_rx.recv().await.unwrap(), b"partial body");
            assert!(body_rx.recv().await.is_none());
            let navigation_error = capture
                .finish()
                .await
                .expect_err("partial response must fail navigation capture");
            let renderer_error = completion_rx
                .await
                .unwrap()
                .expect_err("renderer must receive the transport failure");
            for error in [&navigation_error, &renderer_error] {
                let cause = error
                    .root_cause()
                    .downcast_ref::<std::io::Error>()
                    .expect("shared body failure must retain the typed I/O cause");
                assert_eq!(cause.kind(), std::io::ErrorKind::ConnectionReset);
                assert!(format!("{error:#}").contains("transport completion failed"));
                assert!(format!("{error:#}").contains("failed to read page body from stream"));
            }
            assert!(std::ptr::eq(
                navigation_error.root_cause(),
                renderer_error.root_cause()
            ));
        })
        .await;
}

#[tokio::test]
async fn cancelled_body_capture_retains_join_error() {
    let task = tokio::spawn(std::future::pending::<anyhow::Result<super::CapturedBody>>());
    task.abort();
    let error = super::NativeBodyCapture {
        pump: Some(task),
        writer: None,
    }
    .finish()
    .await
    .expect_err("cancelled capture must fail");
    assert!(
        error
            .downcast_ref::<tokio::task::JoinError>()
            .expect("capture must preserve the task failure type")
            .is_cancelled()
    );
    assert!(format!("{error:#}").contains("main document body capture task failed"));
}

use crate::browser::{NavigationNetworkError, NavigationNetworkErrorKind};
use url::Url;
const OFFLINE_ERROR_TEXT: &str = "net::ERR_INTERNET_DISCONNECTED";
#[test]
fn error_page_failure_uses_typed_request_after_context() {
    let unreachable_url = Url::parse("https://overridden.example/unreachable").unwrap();
    let request_headers = vec![("x-request".to_owned(), "overridden-request".to_owned())];
    let error = anyhow::Error::new(NavigationNetworkError {
        kind: NavigationNetworkErrorKind::InternetDisconnected,
        unreachable_url: unreachable_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: request_headers.clone().into(),
    })
    .context("failed to load document")
    .context("failed to continue intercepted navigation");
    let (failure, _) = super::navigation_fetch_failure(
        error,
        Url::parse("https://original.example/document").unwrap(),
    )
    .unwrap();
    let request = failure.request.unwrap();
    assert_eq!(request.current_url(), &unreachable_url);
    assert_eq!(request.request_method(), "POST");
    assert_eq!(
        request.request_headers(),
        &moli_fetch::RequestHeaders::from_utf8(request_headers)
    );
    assert_eq!(failure.error.unreachable_url, unreachable_url);
    assert_eq!(failure.error.error_text, OFFLINE_ERROR_TEXT);
}

#[test]
fn navigation_error_boundary_does_not_classify_display_text_as_network_failure() {
    for error in [
        anyhow::anyhow!(OFFLINE_ERROR_TEXT),
        anyhow::anyhow!(OFFLINE_ERROR_TEXT)
            .context("failed to load document")
            .context("failed to continue intercepted navigation"),
    ] {
        let expected_chain = format!("{error:#}");
        let error = super::navigation_fetch_failure(
            error,
            Url::parse("https://original.example/document").unwrap(),
        )
        .expect_err("a matching display string must remain a non-network error");
        assert!(error.downcast_ref::<NavigationNetworkError>().is_none());
        assert_eq!(format!("{error:#}"), expected_chain);
        assert_eq!(error.root_cause().to_string(), OFFLINE_ERROR_TEXT);
    }
}
