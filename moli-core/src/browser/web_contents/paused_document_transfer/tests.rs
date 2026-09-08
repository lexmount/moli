fn failing_streamed_document(
    navigation: &super::super::NavigationRequestInterception,
) -> super::DocumentBodySource {
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::unbounded_channel();
    chunks_tx.send(b"partial document".to_vec()).unwrap();
    drop(chunks_tx);
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    completion_tx
        .send(Err(anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "typed response body failure",
        ))
        .context("response transport failed")))
        .unwrap();
    let response = moli_fetch::StreamingRawResponse::new(
        navigation.requested_url.clone(),
        200,
        Vec::new(),
        None,
        Vec::new(),
        false,
        Vec::new(),
        chunks_rx,
        moli_fetch::FetchCancelHandle::new(),
        completion_rx,
    );
    super::DocumentBodySource::StreamingRaw {
        requested_url: navigation.requested_url.clone(),
        request_method: navigation.method.clone(),
        request_headers: navigation.headers.clone(),
        response,
        network_observation_journal: Default::default(),
    }
}

#[tokio::test]
async fn fetch_body_materialization_preserves_error_type_and_request_identity() {
    let navigation = navigation_fixture();
    let (error, body) = failing_streamed_document(&navigation)
        .materialize_body_limited_async(1024)
        .await
        .expect_err("partial transport failure must fail body materialization");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("materialization must preserve the I/O cause")
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert!(format!("{error:#}").contains("response transport failed"));
    assert!(format!("{error:#}").contains("failed to read page body from stream"));
    let super::DocumentBodySource::CapturedRaw {
        requested_url,
        request_method,
        request_headers,
        ..
    } = body
    else {
        panic!("failed streamed materialization must return its captured source");
    };
    assert_eq!(requested_url, navigation.requested_url);
    assert_eq!(request_method, navigation.method);
    assert_eq!(request_headers, navigation.headers);
}

#[tokio::test]
async fn fetch_body_stream_read_preserves_error_type_and_paused_transfer() {
    let navigation = navigation_fixture();
    let body = failing_streamed_document(&navigation);
    let transfer = super::PausedDocumentTransfer::pending(navigation.policy, body)
        .open_body_stream("stream-typed-error".to_owned())
        .expect("streamed response should open")
        .transfer;
    let (transfer, error) = transfer
        .read_body_stream_async(None)
        .await
        .expect_err("partial transport failure must fail the stream read");
    assert!(
        transfer.body_stream_offset().is_some(),
        "failure must retain the paused stream"
    );
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("stream read must preserve the I/O cause")
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert!(format!("{error:#}").contains("response transport failed"));
    assert!(format!("{error:#}").contains("failed to read page body from stream"));
}

fn navigation_fixture() -> super::super::NavigationRequestInterception {
    super::super::NavigationRequestInterception::new(
        "https://original.example/document".parse().unwrap(),
        "POST".into(),
        None,
        moli_fetch::RequestHeaders::from_bytes(vec![("x-request".into(), vec![0xe9, 0xff])]),
        crate::browser::NavigationRequestLoadPolicy::BrowserInitiated,
    )
}
