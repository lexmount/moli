use super::*;

#[tokio::test]
async fn native_receive_storage_is_shared_across_idle_probes_and_transferred_to_chunks() {
    let runtime = CurlWebSocketRuntime::new().unwrap();
    let mut connections = Vec::new();
    let mut peers = Vec::new();
    let mut writers = Vec::new();
    for _ in 0..3 {
        let (write, writes) = std::sync::mpsc::channel::<u8>();
        let (url, peer) = server(move |mut stream| {
            upgrade(&mut stream, &[]);
            while let Ok(byte) = writes.recv_timeout(DEADLINE) {
                stream.write_all(&[0x82, 1, byte]).unwrap();
            }
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        let mut connection = runtime.connect(CurlWebSocketRequest::new(url)).unwrap();
        opened(&mut connection).await;
        connection.sender().set_reading(true);
        timeout(
            DEADLINE,
            connection.sender().control.read_waiting.notified(),
        )
        .await
        .unwrap();
        connections.push(connection);
        peers.push(peer);
        writers.push(write);
    }
    let allocations = |connections: &[CurlWebSocketConnection]| {
        connections
            .iter()
            .map(|connection| {
                connection
                    .sender()
                    .control
                    .receive_allocations
                    .load(Ordering::Acquire)
            })
            .sum::<usize>()
    };
    assert_eq!(
        allocations(&connections),
        1,
        "idle connections must share the owner's spare receive storage"
    );

    let mut retained = Vec::new();
    for byte in 0..12 {
        let index = usize::from(byte) % connections.len();
        writers[index].send(byte).unwrap();
        let CurlWebSocketEvent::Chunk { data, .. } = event(&mut connections[index]).await else {
            panic!("expected payload");
        };
        retained.push(data);
        timeout(
            DEADLINE,
            connections[index].sender().control.read_waiting.notified(),
        )
        .await
        .unwrap();
        // Success transfers the previous allocation; the following AGAIN only
        // needs one replacement. Keep all prior chunks alive to detect aliasing.
        assert_eq!(allocations(&connections), usize::from(byte) + 2);
        for (expected, data) in retained.iter().enumerate() {
            assert_eq!(data, &[expected as u8]);
        }
    }
    drop(writers);
    drop(connections);
    for peer in peers {
        peer.join().unwrap();
    }
}
