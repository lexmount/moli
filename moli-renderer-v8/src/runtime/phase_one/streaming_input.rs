use anyhow::{Result, anyhow};
use tokio::sync::mpsc;

use crate::page_task_queue::RendererPageMainParserContinuationProducer;

use super::streaming::RawDocumentBodySource;

const STREAMING_DOCUMENT_INPUT_BUFFERED_EVENTS: usize = 1;

/// One durable parser-input fact produced by the main Document response.
///
/// These values are not HTML tasks. They make the already-resident parser
/// continuation runnable and are consumed only by that continuation.
pub(super) enum StreamingDocumentInputEvent {
    Chunks(Vec<Vec<u8>>),
    Finished(std::result::Result<(), String>),
}

#[derive(Clone)]
struct StreamingDocumentInputSender {
    tx: mpsc::Sender<StreamingDocumentInputEvent>,
    parser_continuation: RendererPageMainParserContinuationProducer,
}

impl StreamingDocumentInputSender {
    async fn receiver_closed(&self) {
        self.tx.closed().await;
    }

    async fn send(&self, event: StreamingDocumentInputEvent) -> bool {
        if self.tx.send(event).await.is_err() {
            return false;
        }
        // The payload is resident before the owner is notified. A merged or
        // delayed wake therefore cannot lose parser input.
        self.parser_continuation.request().is_ok()
    }
}

/// Stable, bounded residence for raw main-Document input while the PageVm is
/// parked in the owner-local Page slot.
pub(super) struct StreamingDocumentInputSource {
    rx: mpsc::Receiver<StreamingDocumentInputEvent>,
}

impl StreamingDocumentInputSource {
    pub(super) fn bridge(
        mut raw_body: RawDocumentBodySource,
        parser_continuation: RendererPageMainParserContinuationProducer,
        task_runner: crate::network::RendererResourceTaskRunner,
    ) -> Self {
        let (tx, rx) = mpsc::channel(STREAMING_DOCUMENT_INPUT_BUFFERED_EVENTS);
        let sender = StreamingDocumentInputSender {
            tx,
            parser_continuation,
        };
        task_runner.spawn(async move {
            loop {
                let next_chunk = tokio::select! {
                    _ = sender.receiver_closed() => return,
                    next_chunk = raw_body.next_chunk() => next_chunk,
                };
                let Some(chunk) = next_chunk else {
                    break;
                };
                let mut chunks = vec![chunk];
                while let Some(chunk) = raw_body.try_next_chunk() {
                    chunks.push(chunk);
                }
                if !sender
                    .send(StreamingDocumentInputEvent::Chunks(chunks))
                    .await
                {
                    return;
                }
            }
            let terminal = tokio::select! {
                _ = sender.receiver_closed() => return,
                terminal = raw_body.finish() => terminal.map_err(|error| error.to_string()),
            };
            let _ = sender
                .send(StreamingDocumentInputEvent::Finished(terminal))
                .await;
        });
        Self { rx }
    }

    pub(super) fn has_ready_input(&mut self) -> bool {
        !self.rx.is_empty()
    }

    pub(super) fn try_next(&mut self) -> Result<Option<StreamingDocumentInputEvent>> {
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(anyhow!(
                "streaming main-Document input producer exited without a terminal event"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::phase_one::streaming::ExternalRawDocumentBodyStream;
    use crate::{
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        page_task_queue::{
            PageRuntimeWakeSignal, RendererOwnerWake, RendererOwnerWakeSender,
            RendererPageMainParserContinuationSender, RendererPageNetworkingSource,
            RendererPageNetworkingTask,
        },
        runtime::{RendererDocumentToken, RendererPageToken},
    };
    use moli_fetch::{FetchCancelHandle, StreamingRawResponse};

    fn pending_fetch_body() -> (
        tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        tokio::sync::oneshot::Sender<Result<()>>,
        FetchCancelHandle,
        RawDocumentBodySource,
    ) {
        let (body_tx, body_rx) = tokio::sync::mpsc::unbounded_channel();
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        let cancel_handle = FetchCancelHandle::new();
        let response = StreamingRawResponse::new(
            url::Url::parse("https://streaming-input.test/").expect("test URL"),
            200,
            Vec::new(),
            None,
            Vec::new(),
            false,
            Vec::new(),
            body_rx,
            cancel_handle.clone(),
            completion_rx,
        );
        (
            body_tx,
            completion_tx,
            cancel_handle,
            RawDocumentBodySource::FetchResponse(Box::new(response)),
        )
    }

    fn continuation_fixture(
        page_id: u64,
    ) -> (
        RendererPageNetworkingSource,
        RendererPageMainParserContinuationProducer,
        tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
    ) {
        let root_document =
            RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(page_id), 1);
        let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let networking = RendererPageNetworkingSource::new_owner_attached(
            PageRuntimeWakeSignal::default(),
            RendererOwnerWakeSender::new(
                wake_tx,
                RendererPageToken::new_for_testing(root_document.page_id),
            ),
        );
        let continuation =
            RendererPageMainParserContinuationSender::new(networking.route(), root_document)
                .bind_producer(FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(1),
                    LocalWindowId(2),
                    DocumentId(3),
                ));
        (networking, continuation, wake_rx)
    }

    #[tokio::test]
    async fn body_payloads_are_resident_before_their_owner_wakes() {
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        let (mut networking, continuation, mut wake_rx) = continuation_fixture(92);
        let mut source = StreamingDocumentInputSource::bridge(
            RawDocumentBodySource::External(raw_body),
            continuation.clone(),
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("Tokio test owns the streaming input runner"),
        );

        body_tx
            .send(b"first".to_vec())
            .await
            .expect("first test body chunk should send");
        body_tx
            .send(b"second".to_vec())
            .await
            .expect("second test body chunk should send");
        let chunk_wake = wake_rx
            .recv()
            .await
            .expect("stored body chunk should wake its owner");
        assert_eq!(
            chunk_wake.source_for_test(),
            crate::page_task_queue::RendererOwnerWakeSource::NetworkingTask
        );
        assert!(source.has_ready_input());
        let (_, task) = networking
            .pop_front_task()
            .expect("body payload should queue one parser continuation");
        let RendererPageNetworkingTask::MainParserContinuation(task) = task else {
            panic!("body payload must notify the parser through Networking");
        };
        let _ = task.into_owner();
        assert!(matches!(
            source.try_next().expect("chunk read should succeed"),
            Some(StreamingDocumentInputEvent::Chunks(chunks))
                if chunks == [b"first".to_vec(), b"second".to_vec()]
        ));

        drop(body_tx);
        completion_tx
            .send(Ok(()))
            .expect("test body terminal should send");
        let terminal_wake = wake_rx
            .recv()
            .await
            .expect("stored body terminal should wake its owner");
        assert_eq!(
            terminal_wake.source_for_test(),
            crate::page_task_queue::RendererOwnerWakeSource::NetworkingTask
        );
        let (_, task) = networking
            .pop_front_task()
            .expect("body terminal should queue one parser continuation");
        let RendererPageNetworkingTask::MainParserContinuation(task) = task else {
            panic!("body terminal must notify the parser through Networking");
        };
        let _ = task.into_owner();
        assert!(matches!(
            source.try_next().expect("terminal read should succeed"),
            Some(StreamingDocumentInputEvent::Finished(Ok(())))
        ));
    }

    #[tokio::test]
    async fn dropping_input_source_cancels_fetch_waiting_for_another_body_chunk() {
        let (body_tx, completion_tx, cancel_handle, raw_body) = pending_fetch_body();
        let (_networking, continuation, _wake_rx) = continuation_fixture(93);
        let source = StreamingDocumentInputSource::bridge(
            raw_body,
            continuation,
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("Tokio test owns the streaming input runner"),
        );

        drop(source);

        tokio::time::timeout(std::time::Duration::from_secs(1), body_tx.closed())
            .await
            .expect("dropping the input source should release the raw body receiver");
        assert!(
            cancel_handle.is_cancelled(),
            "dropping the raw response should cancel its fetch"
        );
        assert!(
            completion_tx.send(Ok(())).is_err(),
            "the detached bridge must not retain the fetch completion receiver"
        );
    }

    #[tokio::test]
    async fn dropping_input_source_cancels_fetch_waiting_for_final_completion() {
        let (body_tx, mut completion_tx, cancel_handle, raw_body) = pending_fetch_body();
        drop(body_tx);
        let (_networking, continuation, _wake_rx) = continuation_fixture(94);
        let source = StreamingDocumentInputSource::bridge(
            raw_body,
            continuation,
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("Tokio test owns the streaming input runner"),
        );

        drop(source);

        tokio::time::timeout(std::time::Duration::from_secs(1), completion_tx.closed())
            .await
            .expect("dropping the input source should release the fetch completion receiver");
        assert!(
            cancel_handle.is_cancelled(),
            "dropping the raw response should cancel a fetch awaiting terminal completion"
        );
    }
}
