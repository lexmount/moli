//! Each connection has one ingress slot and at most one parked frame.
//! A stalled readable stops only its own receiver; other sockets remain fair.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::poll_fn;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use moli_websocket::Event;
use tokio::sync::mpsc;

struct SocketEvents {
    ingress: mpsc::Receiver<Event>,
    parked: Option<Event>,
    retry_ready: bool,
}

#[derive(Default)]
struct EventState {
    sockets: HashMap<u64, SocketEvents>,
    order: VecDeque<u64>,
    wake: Option<Waker>,
    closed: bool,
}

#[derive(Clone)]
pub(super) struct WorkerWebSocketIngress(Rc<RefCell<EventState>>);

impl WorkerWebSocketIngress {
    /// Only the per-connection sender crosses to the transport runtime.
    pub(super) fn register(&self, socket_id: u64) -> Option<mpsc::Sender<Event>> {
        let (sender, wake) = {
            let mut state = self.0.borrow_mut();
            if state.closed || state.sockets.contains_key(&socket_id) {
                return None;
            }
            let (sender, ingress) = mpsc::channel(1);
            state.sockets.insert(
                socket_id,
                SocketEvents {
                    ingress,
                    parked: None,
                    retry_ready: false,
                },
            );
            state.order.push_back(socket_id);
            (sender, state.wake.take())
        };
        if let Some(wake) = wake {
            wake.wake();
        }
        Some(sender)
    }

    fn resume(&self, socket_id: u64) {
        let wake = {
            let mut state = self.0.borrow_mut();
            let Some(socket) = state.sockets.get_mut(&socket_id) else {
                return;
            };
            if socket.parked.is_none() || socket.retry_ready {
                return;
            }
            socket.retry_ready = true;
            state.wake.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}

pub(crate) fn signal_worker_websocket_stream_pull(scope: &v8::PinScope<'_, '_>, socket_id: u64) {
    if let Some(ingress) = scope.get_slot::<WorkerWebSocketIngress>() {
        ingress.resume(socket_id);
    }
}

/// Closing/cancelling must release a parked delivery without another read.
/// Dispatch still observes that frame once, preserving network trace ordering;
/// the stream's close-requested state prevents enqueueing it into the readable.
pub(crate) fn signal_worker_websocket_stream_close(scope: &v8::PinScope<'_, '_>, socket_id: u64) {
    if let Some(ingress) = scope.get_slot::<WorkerWebSocketIngress>() {
        ingress.resume(socket_id);
    }
}

pub(super) struct WorkerWebSocketEvents {
    ingress: WorkerWebSocketIngress,
}

impl WorkerWebSocketEvents {
    pub(super) fn new() -> (WorkerWebSocketIngress, Self) {
        let ingress = WorkerWebSocketIngress(Rc::default());
        (ingress.clone(), Self { ingress })
    }

    pub(super) fn signals(&self) -> WorkerWebSocketIngress {
        self.ingress.clone()
    }

    pub(super) fn park(&mut self, event: Event) {
        assert!(matches!(
            event,
            Event::TextMessage { .. } | Event::BinaryMessage { .. }
        ));
        let mut state = self.ingress.0.borrow_mut();
        let socket = state
            .sockets
            .get_mut(&event.socket_id())
            .expect("registered WebSocket");
        assert!(socket.parked.is_none(), "WebSocket frame already parked");
        socket.retry_ready = false;
        socket.parked = Some(event);
    }

    /// Polling borrows the registry only for this call, including when pending.
    /// Losing select! branches retain parked frames and their retry demand.
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        let mut state = self.ingress.0.borrow_mut();
        if state.closed {
            return Poll::Ready(None);
        }
        state.wake = Some(cx.waker().clone());
        let count = state.order.len();
        for _ in 0..count {
            let socket_id = state.order.pop_front().expect("registered socket order");
            let socket = state
                .sockets
                .get_mut(&socket_id)
                .expect("registered WebSocket");
            let event = if socket.parked.is_some() {
                if socket.retry_ready {
                    socket.retry_ready = false;
                    Poll::Ready(socket.parked.take())
                } else {
                    Poll::Pending
                }
            } else {
                socket.ingress.poll_recv(cx)
            };
            match event {
                Poll::Ready(Some(event)) => {
                    assert_eq!(event.socket_id(), socket_id, "WebSocket receiver owner");
                    if matches!(event, Event::Close { .. }) {
                        state.sockets.remove(&socket_id);
                    } else {
                        state.order.push_back(socket_id);
                    }
                    return Poll::Ready(Some(event));
                }
                Poll::Ready(None) => {
                    state.sockets.remove(&socket_id);
                }
                Poll::Pending => state.order.push_back(socket_id),
            }
        }
        Poll::Pending
    }

    pub(super) async fn recv(&mut self) -> Option<Event> {
        poll_fn(|cx| self.poll_next(cx)).await
    }
}

impl Drop for WorkerWebSocketEvents {
    fn drop(&mut self) {
        let mut state = self.ingress.0.borrow_mut();
        state.closed = true;
        state.sockets.clear();
        state.order.clear();
        state.wake = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::task::Wake;

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
    async fn backpressured_socket_does_not_block_another_socket() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let first = ingress.register(7).unwrap();
        let second = ingress.register(8).unwrap();
        first.send(frame(7, 1)).await.unwrap();
        let event = source.recv().await.unwrap();
        let allocation = payload(&event).as_ptr();
        source.park(event);
        first.send(frame(7, 2)).await.unwrap();
        assert!(matches!(
            first.try_send(frame(7, 3)),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        second.send(frame(8, 4)).await.unwrap();
        let other = tokio::time::timeout(std::time::Duration::from_secs(1), source.recv())
            .await
            .expect("socket 8 must progress without a pull from socket 7")
            .unwrap();
        assert_eq!(other.socket_id(), 8);
        for _ in 0..100 {
            ingress.resume(7);
        }
        let resumed = source.recv().await.unwrap();
        assert_eq!(payload(&resumed).as_ptr(), allocation);
        assert_eq!(payload(&resumed), &[1; 128]);
        assert_eq!(payload(&source.recv().await.unwrap()), &[2; 128]);
    }

    #[tokio::test]
    async fn parked_sockets_resume_independently_and_preserve_order() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let first = ingress.register(7).unwrap();
        let second = ingress.register(8).unwrap();
        first.send(frame(7, 1)).await.unwrap();
        let event = source.recv().await.unwrap();
        source.park(event);
        second.send(frame(8, 2)).await.unwrap();
        let event = source.recv().await.unwrap();
        source.park(event);
        first.send(frame(7, 3)).await.unwrap();
        second.send(frame(8, 4)).await.unwrap();
        ingress.resume(8);
        assert_eq!(payload(&source.recv().await.unwrap()), &[2; 128]);
        assert_eq!(payload(&source.recv().await.unwrap()), &[4; 128]);
        ingress.resume(7);
        assert_eq!(payload(&source.recv().await.unwrap()), &[1; 128]);
        assert_eq!(payload(&source.recv().await.unwrap()), &[3; 128]);
    }

    #[tokio::test]
    async fn cancelling_select_preserves_frame_and_pending_retry() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let _sender = ingress.register(7).unwrap();
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
        ingress.resume(7);
        tokio::select! {
            biased;
            () = std::future::ready(()) => {},
            _ = source.recv() => panic!("other worker source must win"),
        }
        assert_eq!(source.recv().await.unwrap().socket_id(), 7);
        source.park(frame(7, 2));
        let mut receive = pin!(source.recv());
        assert!(
            receive
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }

    struct CountWakes(AtomicUsize);
    impl Wake for CountWakes {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn registering_a_socket_wakes_a_pending_receive() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let _first = ingress.register(7).unwrap();
        source.park(frame(7, 1));
        let count = Arc::new(CountWakes(AtomicUsize::new(0)));
        let waker = Waker::from(count.clone());
        let mut receive = pin!(source.recv());
        assert!(
            receive
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        let second = ingress.register(8).unwrap();
        assert!(count.0.load(Ordering::Relaxed) > 0);
        second.send(frame(8, 2)).await.unwrap();
        assert_eq!(receive.await.unwrap().socket_id(), 8);
    }

    #[tokio::test]
    async fn closing_socket_removes_its_receiver() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let sender = ingress.register(7).unwrap();
        sender
            .send(Event::Close {
                socket_id: 7,
                code: 1000,
                reason: String::new(),
                was_clean: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            source.recv().await,
            Some(Event::Close { socket_id: 7, .. })
        ));
        assert!(sender.is_closed());
        assert!(ingress.0.borrow().sockets.is_empty());
        assert!(ingress.0.borrow().order.is_empty());
    }

    #[tokio::test]
    async fn dropping_source_releases_all_blocked_producers() {
        let (ingress, mut source) = WorkerWebSocketEvents::new();
        let sender = ingress.register(7).unwrap();
        let other = ingress.register(8).unwrap();
        source.park(frame(7, 1));
        sender.send(frame(7, 2)).await.unwrap();
        let third = frame(7, 3);
        let allocation = payload(&third).as_ptr();
        let mut send = pin!(sender.send(third));
        assert!(
            send.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        drop(source);
        assert_eq!(payload(&send.await.unwrap_err().0).as_ptr(), allocation);
        assert!(sender.is_closed() && other.is_closed());
        assert!(ingress.register(9).is_none());
        ingress.resume(7);
        assert!(ingress.0.borrow().sockets.is_empty());
    }
}
