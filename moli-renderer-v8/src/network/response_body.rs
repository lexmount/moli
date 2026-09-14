use super::{
    RendererResourceTaskRunner, ResourceBodyResponse, ResourceResponseFailure,
    ResourceResponseHead, ResourceResponseStream,
};
use moli_fetch::{ResponseHead, StreamingRawResponse};
use moli_page_types::{SubresourceResponseBodyRead, SubresourceResponseBodySource};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll, Waker},
};

/// Queue delivery only. These callbacks never execute script or read a body.
/// Completion stays with the physical producer, including when its VM retires.
pub(crate) trait ResourceResponseConsumer: Send {
    fn task_runner(&self) -> RendererResourceTaskRunner;
    fn is_cancelled(&self) -> bool {
        false
    }
    fn detach(&mut self);
    fn response_started(&mut self, head: ResponseHead);
    fn data_received(&mut self, bytes: Vec<u8>);
    fn complete(
        self: Box<Self>,
        result: Result<ResourceBodyResponse, ResourceResponseFailure>,
        network_error_text: Option<String>,
    );
    fn discard(self: Box<Self>);
}

/// One physical response, one spool, and one consumer. HTTP polls only while
/// holding the input lock; controlled responses push under the same lock. A
/// decision can synchronously cancel and retain queued bytes without waiting
/// for a body command, and a physical terminal queues completion immediately.
pub(crate) struct ResourceResponseBody {
    pub(crate) resource: Arc<ResourceResponseStream>,
    input: parking_lot::Mutex<BodyInput>,
    changed: tokio::sync::Notify,
}

struct BodyInput {
    state: BodyInputState,
    transport: Option<Waker>,
    consumer: Option<Box<dyn ResourceResponseConsumer>>,
}

enum BodyInputState {
    Streaming(Box<StreamingRawResponse>),
    Controlled {
        cancel: moli_fetch::FetchCancelHandle,
        discard: Box<dyn FnOnce() + Send>,
    },
    Finished {
        result: Result<(), String>,
        cancel: Option<moli_fetch::FetchCancelHandle>,
        network_error_text: Option<String>,
    },
    Discarded,
}

impl ResourceResponseBody {
    fn new(resource: Arc<ResourceResponseStream>, state: BodyInputState) -> Arc<Self> {
        Arc::new(Self {
            resource,
            input: parking_lot::Mutex::new(BodyInput {
                state,
                transport: None,
                consumer: None,
            }),
            changed: Default::default(),
        })
    }

    pub(crate) fn controlled(
        resource: Arc<ResourceResponseStream>,
        head: ResourceResponseHead,
        cancel: moli_fetch::FetchCancelHandle,
        discard: impl FnOnce() + Send + 'static,
    ) -> Arc<Self> {
        resource.response_started(head);
        Self::new(
            resource,
            BodyInputState::Controlled {
                cancel,
                discard: Box::new(discard),
            },
        )
    }

    pub(crate) fn completed(
        resource: Arc<ResourceResponseStream>,
        response: ResourceBodyResponse,
        status_text: Option<String>,
    ) -> Arc<Self> {
        resource.pause_completed_response(response, status_text);
        Self::new(
            resource,
            BodyInputState::Finished {
                result: Ok(()),
                cancel: None,
                network_error_text: None,
            },
        )
    }

    pub(crate) fn streaming(
        resource: Arc<ResourceResponseStream>,
        response: StreamingRawResponse,
        status_text: Option<String>,
    ) -> Arc<Self> {
        resource.response_started(ResourceResponseHead {
            status_text,
            head: response.head(),
            network_request_headers: None,
        });
        Self::new(resource, BodyInputState::Streaming(Box::new(response)))
    }

    pub(crate) fn start(self: &Arc<Self>, runner: &RendererResourceTaskRunner) {
        if !matches!(self.input.lock().state, BodyInputState::Streaming(_)) {
            return;
        }
        let body = self.clone();
        runner.spawn(async move { std::future::poll_fn(|cx| body.poll_transport(cx)).await });
    }

    fn poll_transport(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut input = self.input.lock();
        loop {
            let BodyInputState::Streaming(response) = &mut input.state else {
                return Poll::Ready(());
            };
            match response.poll_next_chunk(cx) {
                Poll::Ready(Some(bytes)) => self.append(&mut input, bytes),
                Poll::Ready(None) => {
                    let result = match response.poll_finish(cx) {
                        Poll::Ready(result) => result.map_err(|error| format!("{error:#}")),
                        Poll::Pending => {
                            input.transport = Some(cx.waker().clone());
                            return Poll::Pending;
                        }
                    };
                    drop(input);
                    self.complete(result, None);
                    return Poll::Ready(());
                }
                Poll::Pending => {
                    input.transport = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        }
    }

    fn append(&self, input: &mut BodyInput, bytes: Vec<u8>) {
        self.resource.data_received(&bytes);
        if let Some(consumer) = &mut input.consumer {
            consumer.data_received(bytes);
        }
        self.changed.notify_waiters();
    }

    pub(crate) fn data_received(&self, bytes: Vec<u8>) {
        let mut input = self.input.lock();
        if matches!(input.state, BodyInputState::Controlled { .. }) {
            self.append(&mut input, bytes);
        }
    }

    pub(crate) fn complete(&self, result: Result<(), String>, network_error_text: Option<String>) {
        let mut input = self.input.lock();
        if matches!(
            input.state,
            BodyInputState::Discarded | BodyInputState::Finished { .. }
        ) {
            return;
        }
        let cancel = match &input.state {
            BodyInputState::Streaming(response) => Some(response.cancellation_handle()),
            BodyInputState::Controlled { cancel, .. } => Some(cancel.clone()),
            BodyInputState::Finished { .. } | BodyInputState::Discarded => unreachable!(),
        };
        self.resource.finish_response();
        input.state = BodyInputState::Finished {
            result: result.clone(),
            cancel,
            network_error_text: network_error_text.clone(),
        };
        let consumer = input.consumer.take();
        let result = result
            .map(|()| {
                self.resource
                    .finish_response()
                    .expect("physical response head")
            })
            .map_err(|message| self.resource.failure(message));
        drop(input);
        self.changed.notify_waiters();
        if let Some(consumer) = consumer {
            consumer.complete(result, network_error_text);
        }
    }

    pub(crate) fn head(&self) -> ResponseHead {
        self.resource.head().head.clone()
    }

    fn is_cancelled(&self) -> bool {
        match &self.input.lock().state {
            BodyInputState::Streaming(response) => response.cancellation_handle().is_cancelled(),
            BodyInputState::Controlled { cancel, .. } => cancel.is_cancelled(),
            BodyInputState::Discarded => true,
            BodyInputState::Finished { cancel, .. } => cancel
                .as_ref()
                .is_some_and(moli_fetch::FetchCancelHandle::is_cancelled),
        }
    }

    pub(crate) fn body_source(self: &Arc<Self>) -> SubresourceResponseBodySource {
        SubresourceResponseBodySource::Streaming(Arc::new(BodyReadView(Arc::downgrade(self))))
    }

    pub(crate) async fn read(&self, offset: usize, size: usize) -> Result<(Vec<u8>, bool), String> {
        loop {
            let changed = self.changed.notified();
            let mut changed = std::pin::pin!(changed);
            changed.as_mut().enable();
            {
                let input = self.input.lock();
                if matches!(input.state, BodyInputState::Discarded) {
                    return Err("response has been discarded".into());
                }
                let bytes = self.resource.read_received(offset, size)?;
                if let BodyInputState::Finished { result: Ok(()), .. } = &input.state {
                    let length = self
                        .resource
                        .finish_response()
                        .expect("physical response head")
                        .body
                        .len();
                    let eof = offset.saturating_add(bytes.len()) >= length;
                    return Ok((bytes, eof));
                }
                if !bytes.is_empty() || size == 0 {
                    return Ok((bytes, false));
                }
                if let BodyInputState::Finished { result, .. } = &input.state {
                    return result.clone().map(|()| (bytes, true));
                }
            }
            changed.await;
        }
    }

    pub(crate) fn resume(&self, mut consumer: Box<dyn ResourceResponseConsumer>) {
        if self.is_cancelled() || consumer.is_cancelled() {
            self.discard();
            consumer.complete(
                Err(self
                    .resource
                    .failure(crate::network_host::ABORTED_ERROR_TEXT.into())),
                None,
            );
            return;
        }
        let mut input = self.input.lock();
        self.resource.accept_response(None, None);
        consumer.response_started(self.head());
        let mut offset = 0;
        loop {
            let bytes = match self.resource.read_received(offset, 64 * 1024) {
                Ok(bytes) => bytes,
                Err(message) => {
                    drop(input);
                    self.discard();
                    consumer.complete(Err(self.resource.failure(message)), None);
                    return;
                }
            };
            if bytes.is_empty() {
                break;
            }
            offset += bytes.len();
            consumer.data_received(bytes);
        }
        if let BodyInputState::Finished {
            result,
            network_error_text,
            ..
        } = &input.state
        {
            let network_error_text = network_error_text.clone();
            let result = result
                .clone()
                .map(|()| {
                    self.resource
                        .finish_response()
                        .expect("physical response head")
                })
                .map_err(|message| self.resource.failure(message));
            drop(input);
            consumer.complete(result, network_error_text);
        } else {
            assert!(input.consumer.is_none(), "one response consumer");
            input.consumer = Some(consumer);
        }
    }

    pub(crate) fn discard(&self) {
        let mut input = self.input.lock();
        let release = match std::mem::replace(&mut input.state, BodyInputState::Discarded) {
            BodyInputState::Streaming(mut response) => {
                response.cancellation_handle().cancel();
                while let Some(bytes) = response.try_next_chunk() {
                    self.resource.data_received(&bytes);
                }
                None
            }
            BodyInputState::Controlled { cancel, discard } => {
                cancel.cancel();
                Some(discard)
            }
            BodyInputState::Finished { cancel, .. } => {
                if let Some(cancel) = cancel {
                    cancel.cancel();
                }
                None
            }
            BodyInputState::Discarded => None,
        };
        let consumer = input.consumer.take();
        if let Some(transport) = input.transport.take() {
            transport.wake();
        }
        drop(input);
        if let Some(release) = release {
            release();
        }
        self.changed.notify_waiters();
        if let Some(consumer) = consumer {
            consumer.complete(
                Err(self
                    .resource
                    .failure(crate::network_host::ABORTED_ERROR_TEXT.into())),
                None,
            );
        }
    }
}

impl Drop for ResourceResponseBody {
    fn drop(&mut self) {
        self.discard();
    }
}

struct BodyReadView(Weak<ResourceResponseBody>);
impl SubresourceResponseBodyRead for BodyReadView {
    fn retained_memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
    }
    fn read(
        &self,
        offset: usize,
        size: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(Vec<u8>, bool), String>> + Send + '_>> {
        Box::pin(async move {
            let body = self.0.upgrade().ok_or("response is no longer available")?;
            body.read(offset, size).await
        })
    }
}

pub(crate) struct PausedResourceResponse {
    pub(crate) body: Arc<ResourceResponseBody>,
    consumer: Option<Box<dyn ResourceResponseConsumer>>,
}
impl PausedResourceResponse {
    pub(crate) fn new(
        body: Arc<ResourceResponseBody>,
        consumer: Box<dyn ResourceResponseConsumer>,
    ) -> Self {
        body.start(&consumer.task_runner());
        Self {
            body,
            consumer: Some(consumer),
        }
    }
    pub(crate) fn discard(mut self) -> ResponseHead {
        let head = self.body.head();
        self.body.discard();
        self.consumer.take().expect("response consumer").discard();
        head
    }
    pub(crate) fn resume(mut self, status: Option<u16>, headers: Option<Vec<(String, String)>>) {
        self.body.resource.accept_response(status, headers);
        self.body
            .resume(self.consumer.take().expect("response consumer"));
    }
}
impl fmt::Debug for PausedResourceResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PausedResourceResponse")
            .field("head", &self.body.head())
            .finish_non_exhaustive()
    }
}
impl Drop for PausedResourceResponse {
    fn drop(&mut self) {
        if let Some(mut consumer) = self.consumer.take() {
            consumer.detach();
            self.body.resume(consumer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response() -> (
        Arc<ResourceResponseBody>,
        tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        tokio::sync::oneshot::Sender<anyhow::Result<()>>,
        moli_fetch::FetchCancelHandle,
    ) {
        let resource = ResourceResponseStream::unobserved_for_test();
        resource.configure_interception(true, false);
        let head = crate::network_host::local_url_response(&url::Url::parse("data:,held").unwrap())
            .unwrap()
            .head();
        let (chunks, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let cancel = moli_fetch::FetchCancelHandle::new();
        let response =
            StreamingRawResponse::new_with_head(head, receiver, cancel.clone(), completion);
        (
            ResourceResponseBody::streaming(resource, response, None),
            chunks,
            finished,
            cancel,
        )
    }

    #[tokio::test]
    async fn cancellation_wakes_a_body_read_and_retains_only_received_bytes() {
        let (body, chunks, _finished, cancel) = response();
        body.start(&RendererResourceTaskRunner::from_current_tokio().unwrap());
        chunks.send(vec![0, 128]).unwrap();
        assert_eq!(body.read(0, 2).await.unwrap().0, [0, 128]);
        let mut read = std::pin::pin!(body.read(2, 2));
        assert!(
            read.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        chunks.send(vec![255, 65]).unwrap();
        body.discard();
        assert!(cancel.is_cancelled());
        assert!(read.await.is_err());
        let ResourceResponseFailure::PartialBody { body, .. } =
            body.resource.failure("retired".into())
        else {
            panic!("retain physical response")
        };
        assert_eq!(body.clone_body_bytes(), [0, 128, 255, 65]);
    }

    #[tokio::test]
    async fn a_debugger_read_preserves_the_spooled_body_for_continuation() {
        let (body, chunks, finished, _) = response();
        body.resource
            .set_body_writer_for_test(moli_page_types::SubresourceResponseBodyWriter::new(1));
        body.start(&RendererResourceTaskRunner::from_current_tokio().unwrap());
        chunks.send(vec![0, 128]).unwrap();
        assert_eq!(body.read(0, 1).await.unwrap().0, [0]);
        chunks.send(vec![255, 65]).unwrap();
        drop(chunks);
        finished.send(Ok(())).unwrap();
        assert_eq!(
            body.body_source()
                .materialize_bytes_limited(4)
                .await
                .unwrap(),
            [0, 128, 255, 65]
        );
        use crate::types::AsyncSubresourceFetchEvent;
        let mut queue = crate::page_task_queue::RendererResourceCompletionTestHarness::new();
        let receiver = crate::network_host::ResourceFetchReceiver::new(
            RendererResourceTaskRunner::from_current_tokio().unwrap(),
            queue.sender(),
            7,
            body.head().final_url,
            body.resource.clone(),
        );
        receiver.pause(body, true).resume(Some(206), None);
        let Some(AsyncSubresourceFetchEvent::StreamingStarted(started)) =
            queue.pop_next_async_subresource_event()
        else {
            panic!("continue must deliver the accepted head first")
        };
        assert_eq!(started.internal_id, 7);
        assert_eq!(started.head.status, 206);
        let Some(AsyncSubresourceFetchEvent::StreamingChunk(chunk)) =
            queue.pop_next_async_subresource_event()
        else {
            panic!("continue must replay the actual spool")
        };
        assert_eq!(chunk.body_source_id, started.body_source_id);
        assert_eq!(chunk.bytes, [0, 128, 255, 65]);
        let Some(AsyncSubresourceFetchEvent::TransportStreamingFinished {
            body_source_id,
            completion,
        }) = queue.pop_next_async_subresource_event()
        else {
            panic!("complete the original response after replay")
        };
        assert_eq!(body_source_id, started.body_source_id);
        let complete = completion.complete_for_test();
        assert_eq!(complete.internal_id, 7);
        assert_eq!(
            complete.result.unwrap().body.clone_body_bytes(),
            [0, 128, 255, 65]
        );
        assert!(queue.pop_next_async_subresource_event().is_none());
    }

    #[tokio::test]
    async fn observation_body_views_do_not_retain_response_ownership() {
        let (body, _chunks, _finished, cancel) = response();
        let view = body.body_source();
        drop(body);
        assert!(cancel.is_cancelled());
        assert!(view.read(0, 1).await.is_err());
    }
}
