use std::{future::Future, pin::Pin, sync::Arc};

use crate::Event;

type BoxEventSendFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;
type AsyncEventSink = dyn Fn(Event) -> BoxEventSendFuture + Send + Sync + 'static;

#[derive(Clone)]
pub struct EventSender {
    target: EventSenderTarget,
    after_send: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
}

#[derive(Clone)]
enum EventSenderTarget {
    Channel(tokio::sync::mpsc::Sender<Event>),
    AsyncSink(Arc<AsyncEventSink>),
}

impl EventSender {
    pub fn new(tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        Self {
            target: EventSenderTarget::Channel(tx),
            after_send: None,
        }
    }

    pub fn with_after_send(
        tx: tokio::sync::mpsc::Sender<Event>,
        after_send: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            target: EventSenderTarget::Channel(tx),
            after_send: Some(Arc::new(after_send)),
        }
    }

    /// Creates a sender backed by a typed async residence rather than a raw
    /// `Event` channel.
    ///
    /// The sink owns admission, backpressure, and wake publication as one
    /// operation. This is useful for embedders that must stamp an exact owner
    /// onto each accepted event before making it scheduler-visible.
    pub fn with_async_sink<F, Fut>(sink: F) -> Self
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        Self {
            target: EventSenderTarget::AsyncSink(Arc::new(move |event| Box::pin(sink(event)))),
            after_send: None,
        }
    }

    /// Creates an unavailable sender for a host context that can only return
    /// a result through a separate direct-completion channel.
    ///
    /// Keeping this as an explicit closed sink lets an accidentally reached
    /// WebSocket producer fail its normal async send instead of panicking in
    /// renderer capability lookup.
    pub fn closed() -> Self {
        Self::with_async_sink(|_event| async { false })
    }

    pub async fn send(&self, event: Event) -> bool {
        let sent = match &self.target {
            EventSenderTarget::Channel(tx) => tx.send(event).await.is_ok(),
            EventSenderTarget::AsyncSink(sink) => sink(event).await,
        };
        if !sent {
            return false;
        }
        if let Some(after_send) = &self.after_send {
            after_send();
        }
        true
    }
}

impl From<tokio::sync::mpsc::Sender<Event>> for EventSender {
    fn from(tx: tokio::sync::mpsc::Sender<Event>) -> Self {
        Self::new(tx)
    }
}

impl std::fmt::Debug for EventSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventSender")
            .field(
                "target",
                &match &self.target {
                    EventSenderTarget::Channel(_) => "channel",
                    EventSenderTarget::AsyncSink(_) => "async_sink",
                },
            )
            .field("has_after_send", &self.after_send.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct EventSinkClosed;

pub(crate) type EventResult<T = ()> = Result<T, EventSinkClosed>;

pub(crate) async fn send_event(event_tx: &EventSender, event: Event) -> EventResult {
    event_tx
        .send(event)
        .await
        .then_some(())
        .ok_or(EventSinkClosed)
}

pub(crate) async fn send_error_and_close(
    event_tx: &EventSender,
    socket_id: u64,
    message: String,
) -> EventResult {
    send_event(event_tx, Event::Error { socket_id, message }).await?;
    send_event(
        event_tx,
        Event::Close {
            socket_id,
            code: 1006,
            reason: String::new(),
            was_clean: false,
        },
    )
    .await
}
