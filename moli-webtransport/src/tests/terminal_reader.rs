use super::*;

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
