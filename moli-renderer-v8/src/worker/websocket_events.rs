//! One worker-owned ingress slot and at most one backpressured WebSocket frame.
//! The exact socket's pull resumes delivery; other worker sources stay runnable.

use std::cell::Cell;
use std::rc::Rc;

use moli_websocket::Event;
use tokio::sync::{Notify, mpsc};

#[derive(Default)]
struct PullState {
    socket_id: Cell<Option<u64>>,
    requested: Cell<bool>,
    wake: Notify,
}

#[derive(Clone, Default)]
pub(super) struct WorkerWebSocketPulls(Rc<PullState>);

impl WorkerWebSocketPulls {
    fn signal(&self, socket_id: u64) {
        if self.0.socket_id.get() == Some(socket_id) && !self.0.requested.replace(true) {
            self.0.wake.notify_one();
        }
    }
}

pub(crate) fn signal_worker_websocket_stream_pull(scope: &v8::PinScope<'_, '_>, socket_id: u64) {
    if let Some(pulls) = scope.get_slot::<WorkerWebSocketPulls>() {
        pulls.signal(socket_id);
    }
}

pub(super) struct WorkerWebSocketEvents {
    ingress: mpsc::Receiver<Event>,
    parked: Option<Event>,
    pulls: WorkerWebSocketPulls,
}

impl WorkerWebSocketEvents {
    pub(super) fn new() -> (mpsc::Sender<Event>, Self) {
        let (sender, ingress) = mpsc::channel(1);
        (
            sender,
            Self {
                ingress,
                parked: None,
                pulls: WorkerWebSocketPulls::default(),
            },
        )
    }

    pub(super) fn pulls(&self) -> WorkerWebSocketPulls {
        self.pulls.clone()
    }

    pub(super) fn park(&mut self, event: Event) {
        assert!(
            self.parked.is_none(),
            "worker WebSocket frame already parked"
        );
        assert!(matches!(
            event,
            Event::TextMessage { .. } | Event::BinaryMessage { .. }
        ));
        self.pulls.0.socket_id.set(Some(event.socket_id()));
        self.pulls.0.requested.set(false);
        self.parked = Some(event);
    }

    /// A losing select! branch retains both the parked frame and pull demand.
    /// No event leaves this source before recv returns it to the worker loop.
    pub(super) async fn recv(&mut self) -> Option<Event> {
        if self.parked.is_some() {
            loop {
                if self.pulls.0.requested.replace(false) {
                    self.pulls.0.socket_id.set(None);
                    return self.parked.take();
                }
                self.pulls.0.wake.notified().await;
            }
        }
        self.ingress.recv().await
    }
}

impl Drop for WorkerWebSocketEvents {
    fn drop(&mut self) {
        self.pulls.0.socket_id.set(None);
        self.pulls.0.requested.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Waker};

    fn frame(socket_id: u64, byte: u8) -> Event {
        Event::BinaryMessage {
            socket_id,
            data: vec![byte; 128],
        }
    }

    fn payload(event: &Event) -> &[u8] {
        let Event::BinaryMessage { data, .. } = event else {
            panic!("binary frame");
        };
        data
    }

    #[tokio::test]
    async fn matching_pull_retains_frame_allocation_and_bounded_ingress_order() {
        let (sender, mut source) = WorkerWebSocketEvents::new();
        let pulls = source.pulls();
        let first = frame(7, 1);
        let original_allocation = payload(&first).as_ptr();
        sender.send(first).await.unwrap();
        let received = source.recv().await.unwrap();
        source.park(received);
        sender.send(frame(8, 2)).await.unwrap();
        assert!(matches!(
            sender.try_send(frame(9, 3)),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        pulls.signal(8);
        {
            let mut receive = pin!(source.recv());
            assert!(
                receive
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        for _ in 0..100 {
            pulls.signal(7);
        }
        let returned = source.recv().await.unwrap();
        assert_eq!(payload(&returned).as_ptr(), original_allocation);
        assert_eq!(returned.socket_id(), 7);
        assert_eq!(payload(&returned), &[1; 128]);
        assert_eq!(source.recv().await.unwrap().socket_id(), 8);
        sender.send(frame(9, 3)).await.unwrap();
        assert_eq!(source.recv().await.unwrap().socket_id(), 9);
    }

    #[tokio::test]
    async fn cancellation_preserves_parked_frame_and_pending_pull() {
        let (_sender, mut source) = WorkerWebSocketEvents::new();
        let pulls = source.pulls();
        source.park(frame(7, 1));
        {
            let mut receive = pin!(source.recv());
            assert!(
                receive
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        pulls.signal(7);
        tokio::select! {
            biased;
            () = std::future::ready(()) => {},
            _ = source.recv() => panic!("the other worker source must win"),
        }
        assert_eq!(source.recv().await.unwrap().socket_id(), 7);
        // An unused wake permit cannot resume a different parked socket.
        source.park(frame(8, 2));
        let mut receive = pin!(source.recv());
        assert!(
            receive
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }

    #[tokio::test]
    async fn parked_frame_leaves_parent_messages_runnable() {
        let (_sender, mut source) = WorkerWebSocketEvents::new();
        source.park(frame(7, 1));
        let (parent_tx, mut parent_rx) = mpsc::unbounded_channel();
        parent_tx.send("parent message").unwrap();
        tokio::select! {
            message = parent_rx.recv() => assert_eq!(message, Some("parent message")),
            _ = source.recv() => panic!("backpressured frame must remain parked"),
        }
    }

    #[tokio::test]
    async fn dropping_source_releases_blocked_transport_producer() {
        let (sender, mut source) = WorkerWebSocketEvents::new();
        let pulls = source.pulls();
        source.park(frame(7, 1));
        sender.send(frame(8, 2)).await.unwrap();
        let third = frame(9, 3);
        let original_allocation = payload(&third).as_ptr();
        let mut send = pin!(sender.send(third));
        assert!(
            send.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(source);
        let unsent = send.await.unwrap_err().0;
        assert_eq!(payload(&unsent).as_ptr(), original_allocation);
        assert!(sender.is_closed());
        pulls.signal(7);
        assert_eq!(pulls.0.socket_id.get(), None);
        assert!(!pulls.0.requested.get());
    }
}
