use super::*;
use moli_fetch::{FetchCancelHandle, NetworkObservationJournal, StreamingRawResponse};
use tokio::sync::{mpsc, oneshot};

fn navigation(token: &DocumentNavigationToken, id: &str) -> NavigationDispatchState {
    let mut navigation = pending_fetch_auth_navigation(id, Some("SID-A")).navigation;
    navigation.loader_id.clone_from(&token.loader_id);
    navigation.frame_id.clone_from(&token.target_id);
    navigation
}

fn request(token: &DocumentNavigationToken, id: &str) -> PendingFetchNavigation {
    PendingFetchNavigation {
        reserved_service_worker_client: None,
        interception: None,
        remaining_request_stages: Vec::new(),
        redirected_request_id: None,
        auth_retry_budget: Default::default(),
        fetch_request_id: id.to_owned(),
        interception_session_id: Some("SID-A".to_owned()),
        document_navigation_token: Some(token.clone()),
        navigation: navigation(token, id),
        request_cookie_report: None,
        intercept_response: false,
        response_stage_url_match_policy: ResponseStageUrlMatchPolicy::AlreadyMatched,
        auth_required_blocked_intercepts: Vec::new(),
    }
}

struct TestStream {
    body: DocumentBodySource,
    cancellation: FetchCancelHandle,
    chunks: mpsc::UnboundedSender<Vec<u8>>,
    completion: oneshot::Sender<anyhow::Result<()>>,
}

fn stream_body() -> TestStream {
    let (chunks, chunks_rx) = mpsc::unbounded_channel();
    let (completion, completion_rx) = oneshot::channel();
    let cancellation = FetchCancelHandle::new();
    let response = StreamingRawResponse::new_with_head(
        PendingFetchAuthNavigation::test_auth_response(test_url("body"))
            .response()
            .head(),
        chunks_rx,
        cancellation.clone(),
        completion_rx,
    );
    TestStream {
        body: DocumentBodySource::StreamingRaw {
            requested_url: test_url("body"),
            request_method: "GET".to_owned(),
            request_headers: Default::default(),
            response,
            network_observation_journal: NetworkObservationJournal::default(),
            body_progress_source: Default::default(),
        },
        cancellation,
        chunks,
        completion,
    }
}

#[test]
fn supersession_retires_all_document_pause_kinds_but_keeps_current_and_subresources() {
    for open_body in [false, true] {
        let mut runtime = TargetRuntimeSlot::default();
        let mut fetch = TargetFetchState::default();
        let request_token = runtime.start_document_navigation("TID".into(), "LID-request".into());
        fetch.register_pending_fetch_navigation_request(request(&request_token, "request"));
        let auth_token = runtime.start_document_navigation("TID".into(), "LID-auth".into());
        let mut auth = pending_fetch_auth_navigation("auth", Some("SID-other"));
        auth.document_navigation_token = Some(auth_token.clone());
        fetch.register_pending_fetch_auth_navigation("auth".into(), auth);
        let response_token = runtime.start_document_navigation("TID".into(), "LID-response".into());
        let stream = stream_body();
        fetch.register_pending_fetch_response_navigation(
            "response".into(),
            Some(response_token.clone()),
            navigation(&response_token, "response"),
            stream.body,
        );
        if open_body {
            fetch
                .open_pending_fetch_response_body_stream(&mut runtime, "response", "stream".into())
                .unwrap();
        }
        let current = runtime.start_document_navigation("TID".into(), "LID-current".into());
        fetch.register_pending_fetch_navigation_request(request(&current, "current"));
        fetch.register_pending_subresource_fetch_request(
            "xhr".into(),
            pending_subresource_fetch(1, Some("SID-A")),
        );

        let retired = fetch.take_superseded_document_navigations(&current);
        let tokens = retired
            .into_iter()
            .map(|(token, _)| token.unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(
            tokens,
            HashSet::from([request_token, auth_token, response_token])
        );
        assert_eq!(
            fetch.pending_fetch_request_ids,
            HashSet::from(["current".into(), "xhr".into()])
        );
        assert!(fetch.pending_fetch_response_transfers.is_empty());
        assert!(
            stream.cancellation.is_cancelled(),
            "retirement must cancel the transport as well as drop routing metadata"
        );
        assert!(fetch.pending_fetch_navigations.contains_key("current"));
        assert!(fetch.pending_subresource_fetches.contains_key("xhr"));
        assert!(
            fetch
                .take_superseded_document_navigations(&current)
                .is_empty(),
            "retirement is once-only"
        );
    }
}

#[tokio::test]
async fn late_body_read_success_and_failure_cannot_restore_superseded_interception() {
    for succeeds in [false, true] {
        let mut runtime = TargetRuntimeSlot::default();
        let mut fetch = TargetFetchState::default();
        let old = runtime.start_document_navigation("TID".into(), "LID-old".into());
        let stream = stream_body();
        runtime
            .page_slot_mut()
            .add_document_navigation_cancellation(&old, stream.cancellation.clone());
        fetch.register_pending_fetch_response_navigation(
            "response".into(),
            Some(old.clone()),
            navigation(&old, "response"),
            stream.body,
        );
        fetch
            .open_pending_fetch_response_body_stream(&mut runtime, "response", "stream".into())
            .unwrap();
        let PendingFetchResponseBodyStreamReadStart::Pending(read) =
            fetch.start_pending_fetch_response_body_stream_read("stream", None, None)
        else {
            panic!("body read should start");
        };
        assert!(matches!(
            fetch.start_pending_fetch_response_body_stream_read("stream", None, None),
            PendingFetchResponseBodyStreamReadStart::NotFound
        ));
        assert!(!fetch.close_pending_fetch_response_body_stream("stream"));
        assert!(
            fetch
                .pending_fetch_response_transfers
                .contains_request("response")
        );
        let current = runtime.start_document_navigation("TID".into(), "LID-current".into());
        assert!(stream.cancellation.is_cancelled());
        let retired = fetch.take_superseded_document_navigations(&current);
        assert_eq!(retired.len(), 1, "protocol still owns A during the read");
        assert_eq!(retired[0].0.as_ref(), Some(&old));
        runtime.finish_renderer_document_navigation(&old).unwrap();
        assert!(fetch.pending_fetch_response_transfers.is_empty());
        runtime
            .finish_renderer_document_navigation(&current)
            .unwrap();
        assert!(!runtime.renderer_document_navigation_is_suspended());
        if succeeds {
            stream.chunks.send(b"body".to_vec()).unwrap();
        }
        drop(stream.chunks);
        stream
            .completion
            .send(if succeeds {
                Ok(())
            } else {
                Err(moli_fetch::FetchCancelled.into())
            })
            .unwrap();
        let completed = read.wait().await;
        assert!(matches!(
            fetch.finish_pending_fetch_response_body_stream_read(&mut runtime, completed),
            PendingFetchResponseBodyStreamRead::NotFound
        ));
        assert!(fetch.pending_fetch_request_ids.is_empty());
        assert!(fetch.pending_fetch_response_transfers.is_empty());
        assert!(!runtime.renderer_document_navigation_is_suspended());
    }
}

#[tokio::test]
async fn buffered_body_read_keeps_navigation_registered_until_supersession() {
    for succeeds in [false, true] {
        let mut runtime = TargetRuntimeSlot::default();
        let mut fetch = TargetFetchState::default();
        let old = runtime.start_document_navigation("TID".into(), "LID-old".into());
        let stream = stream_body();
        fetch.register_pending_fetch_response_navigation(
            "response".into(),
            Some(old.clone()),
            navigation(&old, "response"),
            stream.body,
        );
        let read = fetch
            .start_pending_fetch_response_body_read("response")
            .unwrap();
        assert!(
            fetch
                .pending_fetch_response_transfers
                .contains_request("response")
        );
        assert!(fetch.pending_fetch_request_ids.contains("response"));
        assert!(
            fetch
                .start_pending_fetch_response_body_read("response")
                .is_none()
        );
        assert!(
            fetch
                .take_available_fetch_response_transfer("response")
                .is_none()
        );
        assert!(fetch.consume_pending_request_action("response").is_err());

        let current = runtime.start_document_navigation("TID".into(), "LID-current".into());
        let retired = fetch.take_superseded_document_navigations(&current);
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].0.as_ref(), Some(&old));
        assert!(
            stream.cancellation.is_cancelled(),
            "the registered read cancels its own transport"
        );
        assert!(fetch.pending_fetch_request_ids.is_empty());
        assert!(fetch.pending_fetch_response_transfers.is_empty());
        if succeeds {
            stream.chunks.send(b"body".to_vec()).unwrap();
        }
        drop(stream.chunks);
        stream
            .completion
            .send(if succeeds {
                Ok(())
            } else {
                Err(moli_fetch::FetchCancelled.into())
            })
            .unwrap();
        let completed = read.materialize(1024).await;
        assert_eq!(completed.result().is_ok(), succeeds);
        assert!(
            fetch
                .finish_pending_fetch_response_body_read(completed)
                .is_none()
        );
        assert!(
            fetch
                .take_superseded_document_navigations(&current)
                .is_empty()
        );
        assert!(
            runtime.renderer_document_navigation_is_suspended(),
            "A's read cannot release B"
        );
    }
}

#[tokio::test]
async fn clearing_requests_cancels_buffered_reads_and_rejects_late_completion() {
    let mut fetch = TargetFetchState::default();
    let mut runtime = TargetRuntimeSlot::default();
    let token = runtime.start_document_navigation("TID".into(), "LID".into());
    let stream = stream_body();
    fetch.register_pending_fetch_response_navigation(
        "response".into(),
        Some(token.clone()),
        navigation(&token, "response"),
        stream.body,
    );
    let read = fetch
        .start_pending_fetch_response_body_read("response")
        .unwrap();
    fetch.clear();
    assert!(stream.cancellation.is_cancelled());
    drop(stream.chunks);
    stream
        .completion
        .send(Err(moli_fetch::FetchCancelled.into()))
        .unwrap();
    assert!(
        fetch
            .finish_pending_fetch_response_body_read(read.materialize(1024).await)
            .is_none()
    );
    assert!(fetch.is_empty());
}

#[tokio::test]
async fn failing_a_request_can_take_and_cancel_its_in_progress_body_read() {
    let mut fetch = TargetFetchState::default();
    let mut runtime = TargetRuntimeSlot::default();
    let token = runtime.start_document_navigation("TID".into(), "LID".into());
    let stream = stream_body();
    fetch.register_pending_fetch_response_navigation(
        "response".into(),
        Some(token.clone()),
        navigation(&token, "response"),
        stream.body,
    );
    let read = fetch
        .start_pending_fetch_response_body_read("response")
        .unwrap();
    let transfer = fetch
        .take_fetch_response_transfer_for_cancellation("response")
        .unwrap();
    let (failed_token, _, result) = transfer.fail(moli_fetch::FetchCancelled.into());
    assert_eq!(failed_token, Some(token));
    assert!(result.unwrap_err().is::<moli_fetch::FetchCancelled>());
    assert!(stream.cancellation.is_cancelled());
    drop(stream.chunks);
    stream
        .completion
        .send(Err(moli_fetch::FetchCancelled.into()))
        .unwrap();
    assert!(
        fetch
            .finish_pending_fetch_response_body_read(read.materialize(1024).await)
            .is_none()
    );
    assert!(fetch.is_empty());
}

#[tokio::test]
async fn disabling_fetch_drains_in_progress_stream_reads_only_for_the_owner_session() {
    let mut fetch = TargetFetchState::default();
    let mut runtime = TargetRuntimeSlot::default();
    let token = runtime.start_document_navigation("TID".into(), "LID".into());
    let stream = stream_body();
    fetch.register_pending_fetch_response_navigation(
        "response".into(),
        Some(token.clone()),
        navigation(&token, "response"),
        stream.body,
    );
    fetch
        .open_pending_fetch_response_body_stream(&mut runtime, "response", "stream".into())
        .unwrap();
    let PendingFetchResponseBodyStreamReadStart::Pending(read) =
        fetch.start_pending_fetch_response_body_stream_read("stream", None, None)
    else {
        panic!("stream read must start");
    };
    assert!(
        fetch
            .drain_pending_requests_for_disable_session(Some("SID-other"))
            .response_transfers
            .is_empty()
    );
    assert!(!stream.cancellation.is_cancelled());
    let mut drained = fetch
        .drain_pending_requests_for_disable_session(Some("SID-A"))
        .response_transfers;
    assert_eq!(drained.len(), 1);
    assert_eq!(drained.pop().unwrap().into_navigation().0, Some(token));
    assert!(stream.cancellation.is_cancelled());
    assert!(fetch.is_empty());
    drop(stream.chunks);
    stream
        .completion
        .send(Err(moli_fetch::FetchCancelled.into()))
        .unwrap();
    assert!(matches!(
        fetch.finish_pending_fetch_response_body_stream_read(&mut runtime, read.wait().await),
        PendingFetchResponseBodyStreamRead::NotFound
    ));
}

#[tokio::test]
async fn late_read_cannot_overwrite_a_new_read_even_if_the_request_id_is_reused() {
    let mut transfers = PausedDocumentTransfers::default();
    let mut runtime = TargetRuntimeSlot::default();
    let token = runtime.start_document_navigation("TID".into(), "LID".into());
    let first = stream_body();
    transfers.register_pending_navigation(
        "response".into(),
        Some(token.clone()),
        navigation(&token, "first"),
        first.body,
    );
    let old = transfers.start_body_read("response").unwrap();
    first.chunks.send(b"old".to_vec()).unwrap();
    drop(first.chunks);
    first.completion.send(Ok(())).unwrap();
    let old = old.materialize(1024).await;
    transfers.clear();

    let second = stream_body();
    transfers.register_pending_navigation(
        "response".into(),
        Some(token.clone()),
        navigation(&token, "second"),
        second.body,
    );
    let current = transfers.start_body_read("response").unwrap();
    assert!(transfers.finish_body_read(old).is_none());
    assert!(!second.cancellation.is_cancelled());
    assert!(
        transfers.start_body_read("response").is_none(),
        "the second read still owns the body"
    );
    second.chunks.send(b"new".to_vec()).unwrap();
    drop(second.chunks);
    second.completion.send(Ok(())).unwrap();
    assert_eq!(
        transfers
            .finish_body_read(current.materialize(1024).await)
            .unwrap()
            .unwrap(),
        Some(b"new".to_vec())
    );
    assert!(transfers.get("response").unwrap().is_pending());
    assert!(
        !second.cancellation.is_cancelled(),
        "returning the body is not cancellation"
    );
}
