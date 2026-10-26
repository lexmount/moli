use super::*;
use crate::{StreamIoError, WriteError};

#[tokio::test]
async fn reading_buffered_fin_after_reset_observation_releases_the_native_reader() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let (buffered, ready) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let (mut send, mut recv) = session.accept_bi().await.unwrap();
            let mut first = [0];
            recv.read_exact(&mut first).await.unwrap();
            send.write_all(&[7]).await.unwrap();
            send.finish().unwrap();
            assert_eq!(send.stopped().await.unwrap(), None);
            // FIN acknowledgement proves all bytes reached the receiver.
            buffered.send(()).unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (mut send, mut recv) = session.open_bi().await.unwrap();
        send.write_all(&[42]).await.unwrap();
        ready.await.unwrap();
        std::future::poll_fn(|cx| {
            let mut reset = std::pin::pin!(recv.received_reset());
            assert!(reset.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        let mut bytes = [0; 16];
        assert_eq!(recv.read(&mut bytes).await.unwrap(), Some(1));
        assert_eq!(bytes[0], 7);
        assert!(recv.read(&mut bytes).await.unwrap().is_none());
        // No further packet is needed to remove the cancelled observer's waker.
        drop(recv);
        session.cancel();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn invalid_reset_code_is_not_reported_as_a_clean_fin() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            // Use raw QUIC to send a code outside WebTransport's HTTP/3 range.
            let (mut send, mut recv) = quinn::Connection::accept_bi(&session).await.unwrap();
            let mut header_and_data = [0; 4];
            recv.read_exact(&mut header_and_data).await.unwrap();
            assert_eq!(header_and_data, [0x40, 0x41, 0, 42]);
            send.reset(quinn::VarInt::from_u32(1)).unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (send, recv) = session.open_bi().await.unwrap();
        let (send, sending) = send.into_io();
        let (recv, receiving) = recv.into_io();
        let sending = tokio::spawn(sending);
        let receiving = tokio::spawn(receiving);
        send.write_all(vec![42]).await.unwrap();
        assert!(matches!(
            recv.closed().await,
            Err(StreamIoError::Transport(crate::ReadError::ClosedStream))
        ));
        assert!(matches!(
            recv.read().await,
            Err(StreamIoError::Transport(crate::ReadError::ClosedStream))
        ));
        receiving.await.unwrap();
        session.cancel();
        sending.await.unwrap();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn peer_stop_and_reset_are_observed_without_read_or_write_demand() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let (mut send, mut recv) = session.accept_bi().await.unwrap();
            let mut first = [0];
            recv.read_exact(&mut first).await.unwrap();
            assert_eq!(first, [42]);
            recv.stop(0).unwrap();
            send.reset(u32::MAX).unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (send, recv) = session.open_bi().await.unwrap();
        let (send, sending) = send.into_io();
        let (recv, receiving) = recv.into_io();
        let sending = tokio::spawn(sending);
        let receiving = tokio::spawn(receiving);
        send.write_all(vec![42]).await.unwrap();
        assert!(matches!(
            send.closed().await,
            Err(StreamIoError::Transport(WriteError::Stopped(0)))
        ));
        // No read was requested from this endpoint.
        assert!(matches!(
            recv.closed().await,
            Err(StreamIoError::Transport(crate::ReadError::Reset(u32::MAX)))
        ));
        assert!(matches!(
            recv.read().await,
            Err(StreamIoError::Transport(crate::ReadError::Reset(u32::MAX)))
        ));
        sending.await.unwrap();
        receiving.await.unwrap();
        session.cancel();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stream_drivers_preserve_bytes_and_fin_across_multiple_requests() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let (mut send, mut recv) = session.accept_bi().await.unwrap();
            let bytes = recv.read_to_end(256 * 1024).await.unwrap();
            send.write_all(&bytes).await.unwrap();
            send.finish().unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (send, recv) = session.open_bi().await.unwrap();
        let (send, sending) = send.into_io();
        let (recv, receiving) = recv.into_io();
        let sending = tokio::spawn(sending);
        let receiving = tokio::spawn(receiving);
        let expected: Vec<_> = (0..131_071).map(|index| index as u8).collect();
        for bytes in expected.chunks(8191) {
            send.write_all(bytes.to_vec()).await.unwrap();
        }
        send.finish().await.unwrap();
        send.closed().await.unwrap();
        let mut received = Vec::new();
        while let Some(bytes) = recv.read().await.unwrap() {
            received.extend(bytes);
        }
        assert_eq!(received, expected);
        recv.closed().await.unwrap();
        assert!(recv.read().await.unwrap().is_none());
        sending.await.unwrap();
        receiving.await.unwrap();
        session.cancel();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn local_reset_interrupts_a_flow_control_blocked_write() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let (seen, ready) = tokio::sync::oneshot::channel();
        let (reset, reset_received) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let mut recv = session.accept_uni().await.unwrap();
            let mut first = [0];
            recv.read_exact(&mut first).await.unwrap();
            seen.send(()).unwrap();
            assert_eq!(recv.received_reset().await.unwrap(), Some(71));
            reset.send(()).unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (send, sending) = session.open_uni().await.unwrap().into_io();
        let sending = tokio::spawn(sending);
        let writing = {
            let send = send.clone();
            tokio::spawn(async move { send.write_all(vec![9; 16 * 1024 * 1024]).await })
        };
        ready.await.unwrap();
        assert!(
            !writing.is_finished(),
            "peer has not consumed enough to release stream credit"
        );
        send.reset(71);
        assert!(matches!(
            writing.await.unwrap(),
            Err(StreamIoError::Cancelled)
        ));
        assert!(matches!(send.closed().await, Err(StreamIoError::Cancelled)));
        sending.await.unwrap();
        reset_received.await.unwrap();
        session.cancel();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_close_observer_does_not_keep_an_unreachable_sender_alive() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut server = fixture.server();
        let (seen, ready) = tokio::sync::oneshot::channel();
        let (reset, reset_received) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let session = server.accept().await.unwrap().ok().await.unwrap();
            let mut recv = session.accept_uni().await.unwrap();
            let mut first = [0];
            recv.read_exact(&mut first).await.unwrap();
            seen.send(()).unwrap();
            assert_eq!(recv.received_reset().await.unwrap(), Some(0));
            reset.send(()).unwrap();
            session.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner
            .connector()
            .connect(fixture.options.clone())
            .await
            .unwrap();
        let (send, sending) = session.open_uni().await.unwrap().into_io();
        let sending = tokio::spawn(sending);
        send.write_all(vec![7]).await.unwrap();
        ready.await.unwrap();
        let closed = send.closed();
        drop(send);
        assert!(matches!(closed.await, Err(StreamIoError::Cancelled)));
        sending.await.unwrap();
        reset_received.await.unwrap();
        session.cancel();
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
