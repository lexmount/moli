use crate::{
    Event, FrameOpcode, spawn_standalone_connection, spawn_synthetic_connection, test_support::*,
};
use futures_util::{SinkExt, StreamExt};
use moli_file_api::{BlobData, BlobReadCompletion, BlobSource};
use parking_lot::Mutex;
use std::{fmt, io, ops::Range, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc, oneshot},
    time::{Duration, timeout},
};
use tokio_tungstenite::tungstenite::Message;

#[derive(Default)]
struct DeferredSource {
    pending: Mutex<Option<BlobReadCompletion>>,
    requested: Notify,
}

impl fmt::Debug for DeferredSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeferredSource")
    }
}

impl BlobSource for DeferredSource {
    fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        assert_eq!(range, 0..4);
        assert!(self.pending.lock().replace(complete).is_none());
        self.requested.notify_one();
    }
}

impl DeferredSource {
    fn blob(self: &Arc<Self>) -> BlobData {
        BlobData::from_source(self.clone(), 4)
    }
    async fn wait(&self) {
        timeout(Duration::from_secs(3), self.requested.notified())
            .await
            .unwrap();
    }
    fn complete(&self, bytes: io::Result<Vec<u8>>) {
        self.pending.lock().take().unwrap()(bytes);
    }
}

#[tokio::test]
async fn source_read_keeps_send_order_and_local_close_while_pong_still_progresses() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/blob", listener.local_addr().unwrap());
    let (ping_tx, ping_rx) = oneshot::channel();
    let (pong_tx, pong_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        ping_rx.await.unwrap();
        socket
            .send(Message::Ping(b"tick".to_vec().into()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Pong(b"tick".to_vec().into())
        );
        pong_tx.send(()).unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Binary(b"data".to_vec().into())
        );
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("tail".into())
        );
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
        socket.flush().await.unwrap();
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_standalone_connection(901, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    let source = Arc::new(DeferredSource::default());
    handle.send_blob(source.blob()).unwrap();
    handle.send_text("tail".into()).unwrap();
    source.wait().await;
    ping_tx.send(()).unwrap();
    timeout(Duration::from_secs(3), pong_rx)
        .await
        .unwrap()
        .unwrap();
    handle.close(Some(1000), String::new()).unwrap();
    assert_closing(&mut rx, 901).await;
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    source.complete(Ok(b"data".to_vec()));
    assert_send_completed(&mut rx, 901, FrameOpcode::Binary, 4).await;
    assert_send_completed(&mut rx, 901, FrameOpcode::Text, 4).await;
    assert_close(&mut rx, 901, 1000, "", true).await;
    server.await.unwrap();
}

#[tokio::test]
async fn peer_close_does_not_wait_for_a_source_read_or_publish_its_late_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/blob-close", listener.local_addr().unwrap());
    let (close_tx, close_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        close_rx.await.unwrap();
        socket.send(Message::Close(None)).await.unwrap();
        assert!(matches!(socket.next().await, Some(Ok(Message::Close(_)))));
    });
    let (tx, mut rx) = mpsc::channel(8);
    let handle = spawn_standalone_connection(902, url, Vec::new(), test_websocket_context(), tx);
    recv_open_event(&mut rx).await;
    let source = Arc::new(DeferredSource::default());
    handle.send_blob(source.blob()).unwrap();
    source.wait().await;
    close_tx.send(()).unwrap();
    assert_close(&mut rx, 902, 1005, "", true).await;
    assert!(handle.is_closed());
    source.complete(Ok(b"data".to_vec()));
    assert!(
        timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .is_none()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn source_error_or_invalid_length_closes_without_sending_later_messages() {
    for bytes in [
        Err(io::Error::other("injected body failure")),
        Ok(b"bad".to_vec()),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/blob-error", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert!(!matches!(
                socket.next().await,
                Some(Ok(Message::Binary(_) | Message::Text(_)))
            ));
        });
        let (tx, mut rx) = mpsc::channel(8);
        let handle =
            spawn_standalone_connection(903, url, Vec::new(), test_websocket_context(), tx);
        recv_open_event(&mut rx).await;
        let source = Arc::new(DeferredSource::default());
        handle.send_blob(source.blob()).unwrap();
        handle.send_text("later".into()).unwrap();
        source.wait().await;
        source.complete(bytes);
        assert!(
            matches!(timeout(Duration::from_secs(3),rx.recv()).await.unwrap(),Some(Event::Error{message,..}) if message.contains("Blob read"))
        );
        assert_close(&mut rx, 903, 1006, "", false).await;
        assert!(handle.is_closed());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn synthetic_source_preserves_fifo_and_peer_close_can_cancel_the_read() {
    for cancel in [false, true] {
        let (tx, mut rx) = mpsc::channel(8);
        let (handle, peer) =
            spawn_synthetic_connection(904, Vec::new().into(), 101, Vec::new().into(), tx);
        recv_open_event(&mut rx).await;
        let source = Arc::new(DeferredSource::default());
        handle.send_blob(source.blob()).unwrap();
        handle.send_text("tail".into()).unwrap();
        source.wait().await;
        peer.send_text("incoming during read".into()).unwrap();
        assert_text_message(&mut rx, 904, "incoming during read").await;
        if cancel {
            peer.close(Some(1000), String::new()).unwrap();
            assert_close(&mut rx, 904, 1000, "", true).await;
            source.complete(Ok(b"data".to_vec()));
        } else {
            handle.close(Some(1000), String::new()).unwrap();
            assert_closing(&mut rx, 904).await;
            source.complete(Ok(b"data".to_vec()));
            assert_send_completed(&mut rx, 904, FrameOpcode::Binary, 4).await;
            assert_send_completed(&mut rx, 904, FrameOpcode::Text, 4).await;
            assert_close(&mut rx, 904, 1000, "", true).await;
        }
        assert!(
            timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn rejected_source_does_not_read_and_cancel_does_not_wait_for_a_source() {
    for rejected in [false, true] {
        let (tx, mut rx) = mpsc::channel(8);
        let (handle, _) =
            spawn_synthetic_connection(905, Vec::new().into(), 101, Vec::new().into(), tx);
        recv_open_event(&mut rx).await;
        let source = Arc::new(DeferredSource::default());
        if rejected {
            let data = BlobData::from_source(source.clone(), crate::commands::MAX_QUEUED_BYTES + 1);
            assert_eq!(
                handle.send_blob(data),
                Err(crate::SendError::CapacityExceeded)
            );
            assert!(matches!(
                timeout(Duration::from_secs(3), rx.recv()).await.unwrap(),
                Some(Event::Error { .. })
            ));
            assert_close(&mut rx, 905, 1006, "", false).await;
            assert!(source.pending.lock().is_none());
        } else {
            handle.send_blob(source.blob()).unwrap();
            source.wait().await;
            handle.cancel();
            assert!(
                timeout(Duration::from_secs(3), rx.recv())
                    .await
                    .unwrap()
                    .is_none()
            );
            source.complete(Ok(b"data".to_vec()));
        }
        assert!(handle.is_closed());
    }
}
