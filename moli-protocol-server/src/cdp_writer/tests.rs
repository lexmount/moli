use serde_json::json;

use super::*;

fn test_sink() -> (
    CdpSocketSink,
    mpsc::Receiver<SocketWriterCommand>,
    tokio::sync::watch::Receiver<SocketCloseSignal>,
) {
    test_sink_with_limits(
        1,
        DEFAULT_MAX_PENDING_WRITER_BYTES,
        DEFAULT_MAX_WRITER_MESSAGE_BYTES,
    )
}

fn test_sink_with_limits(
    queue_capacity: usize,
    max_pending_bytes: usize,
    max_message_bytes: usize,
) -> (
    CdpSocketSink,
    mpsc::Receiver<SocketWriterCommand>,
    tokio::sync::watch::Receiver<SocketCloseSignal>,
) {
    let (output_tx, output_rx) = mpsc::channel(queue_capacity);
    let (close_tx, close_rx) = tokio::sync::watch::channel(SocketCloseSignal::Open);
    (
        CdpSocketSink {
            output_tx,
            close_tx,
            pending_byte_budget: PendingByteBudget::new(max_pending_bytes),
            max_message_bytes,
        },
        output_rx,
        close_rx,
    )
}

#[test]
fn page_sink_overflow_requests_frontend_close_without_waiting() {
    let (sink, _output_rx, close_rx) = test_sink();
    assert!(sink.enqueue_owned_message(json!({ "id": 1, "result": {} })));
    assert!(!sink.enqueue_owned_message(json!({ "id": 2, "result": {} })));
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::ImmediateClose);
}

#[test]
fn graceful_close_is_queued_after_existing_output() {
    let (sink, mut output_rx, close_rx) = test_sink_with_limits(
        2,
        DEFAULT_MAX_PENDING_WRITER_BYTES,
        DEFAULT_MAX_WRITER_MESSAGE_BYTES,
    );
    assert!(sink.enqueue_owned_message(json!({ "id": 1, "result": {} })));

    sink.close_after_flush();

    assert_eq!(*close_rx.borrow(), SocketCloseSignal::Open);
    assert!(matches!(
        output_rx.try_recv(),
        Ok(SocketWriterCommand::Output(_))
    ));
    assert!(matches!(
        output_rx.try_recv(),
        Ok(SocketWriterCommand::CloseAfterFlush)
    ));
}

#[tokio::test]
async fn flush_confirmation_is_ordered_after_output_and_propagates_failure() {
    let (sink, mut output_rx, _close_rx) = test_sink_with_limits(
        2,
        DEFAULT_MAX_PENDING_WRITER_BYTES,
        DEFAULT_MAX_WRITER_MESSAGE_BYTES,
    );
    assert!(sink.enqueue_owned_message(json!({"id": 1, "result": {}})));
    let pending = tokio::spawn(async move { sink.flush().await });
    assert!(matches!(
        output_rx.recv().await,
        Some(SocketWriterCommand::Output(_))
    ));
    let Some(SocketWriterCommand::Flush(completed)) = output_rx.recv().await else {
        panic!("flush must be queued after preceding output");
    };
    assert!(
        !pending.is_finished(),
        "queue admission is not a flush confirmation"
    );
    completed
        .send(false)
        .expect("flush waiter remains attached");
    assert!(!pending.await.expect("flush task"));
}

#[test]
fn peer_close_signal_has_priority_over_immediate_close() {
    let (sink, _output_rx, close_rx) = test_sink();

    sink.peer_close_received();
    sink.close();

    assert_eq!(
        *close_rx.borrow(),
        SocketCloseSignal::PeerCloseReceived,
        "an immediate close must not suppress the automatic peer-close reply"
    );

    let (sink, _output_rx, close_rx) = test_sink();
    sink.close();
    sink.peer_close_received();
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::PeerCloseReceived);
}

#[test]
fn pending_byte_budget_closes_only_the_overflowing_sink() {
    let message = json!({ "id": 1, "result": { "value": "bounded" } });
    let message_bytes = to_string_with_limit(&message, usize::MAX)
        .expect("serialize test message")
        .len();
    let (sink, output_rx, close_rx) = test_sink_with_limits(2, message_bytes, message_bytes);

    assert!(sink.enqueue_owned_message(message.clone()));
    assert_eq!(sink.pending_byte_budget.current(), message_bytes);
    assert!(!sink.enqueue_owned_message(message));
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::ImmediateClose);
    assert_eq!(sink.pending_byte_budget.current(), message_bytes);

    drop(output_rx);
    assert_eq!(sink.pending_byte_budget.current(), 0);
}

#[test]
fn oversized_single_message_is_rejected_before_queue_allocation() {
    let message = json!({ "id": 1, "result": { "value": "too-large" } });
    let message_bytes = serde_json::to_string(&message)
        .expect("serialize test message")
        .len();
    let (sink, mut output_rx, close_rx) =
        test_sink_with_limits(1, message_bytes, message_bytes - 1);

    assert!(!sink.enqueue_owned_message(message));
    assert!(output_rx.try_recv().is_err());
    assert_eq!(sink.pending_byte_budget.current(), 0);
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::ImmediateClose);
}

#[test]
fn owned_outer_html_response_reuses_the_html_allocation() {
    let mut outer_html = String::with_capacity(256);
    outer_html.push_str("<main title=\"quoted\">line 1\\line 2\n中文</main>");
    let outer_html_pointer = outer_html.as_ptr();
    let expected = json!({
        "id": 7,
        "result": { "outerHTML": outer_html.clone() },
        "sessionId": "child-\"session",
    });
    let message = Value::Object(serde_json::Map::from_iter([
        ("id".to_owned(), Value::from(7)),
        (
            "result".to_owned(),
            Value::Object(serde_json::Map::from_iter([(
                "outerHTML".to_owned(),
                Value::String(outer_html),
            )])),
        ),
        (
            "sessionId".to_owned(),
            Value::String("child-\"session".to_owned()),
        ),
    ]));
    let (sink, mut output_rx, close_rx) = test_sink();

    assert!(sink.enqueue_owned_message(message));
    let SocketWriterCommand::Output(envelope) = output_rx
        .try_recv()
        .expect("owned outerHTML response should be queued")
    else {
        panic!("owned outerHTML response queued a close command");
    };

    assert_eq!(envelope.message.as_ptr(), outer_html_pointer);
    assert_eq!(
        serde_json::from_str::<Value>(&envelope.message).expect("queued response JSON"),
        expected
    );
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::Open);
}

#[test]
fn owned_outer_html_response_without_session_matches_generic_json() {
    let message = json!({
        "id": 8,
        "result": { "outerHTML": "<body>no session</body>" },
    });
    let expected = serde_json::to_string(&message).expect("generic response JSON");
    let (sink, mut output_rx, close_rx) = test_sink();

    assert!(sink.enqueue_owned_message(message));
    let SocketWriterCommand::Output(envelope) = output_rx
        .try_recv()
        .expect("outerHTML response should be queued")
    else {
        panic!("outerHTML response queued a close command");
    };

    assert_eq!(envelope.message, expected);
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::Open);
}

#[test]
fn oversized_owned_outer_html_response_is_rejected() {
    let message = json!({
        "id": 8,
        "result": { "outerHTML": "<body title=\"quoted\">large</body>" },
    });
    let message_bytes = serde_json::to_string(&message)
        .expect("serialize outerHTML response")
        .len();
    let (sink, mut output_rx, close_rx) =
        test_sink_with_limits(1, message_bytes, message_bytes - 1);

    assert!(!sink.enqueue_owned_message(message));
    assert!(output_rx.try_recv().is_err());
    assert_eq!(sink.pending_byte_budget.current(), 0);
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::ImmediateClose);
}

#[test]
fn owned_non_outer_html_message_uses_the_generic_serializer() {
    let message = json!({
        "id": 9,
        "result": { "value": "ordinary" },
        "sessionId": "child-session",
    });
    let expected = serde_json::to_string(&message).expect("generic response JSON");
    let (sink, mut output_rx, close_rx) = test_sink();

    assert!(sink.enqueue_owned_message(message));
    let SocketWriterCommand::Output(envelope) = output_rx
        .try_recv()
        .expect("generic response should be queued")
    else {
        panic!("generic response queued a close command");
    };

    assert_eq!(envelope.message, expected);
    assert_eq!(*close_rx.borrow(), SocketCloseSignal::Open);
}
