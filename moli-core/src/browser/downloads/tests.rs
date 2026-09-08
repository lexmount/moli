use super::*;
use moli_fetch::FetchCancelHandle;
use tokio::sync::{mpsc, oneshot};

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "moli-download-service-{}",
            naming::generate_download_guid().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn policy(&self) -> DownloadPolicy {
        DownloadPolicy {
            behavior: DownloadBehavior::Allow,
            download_path: Some(self.0.to_str().unwrap().to_owned()),
        }
    }

    fn files(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct BodyProducer {
    chunks: mpsc::UnboundedSender<Vec<u8>>,
    completion: oneshot::Sender<anyhow::Result<()>>,
    cancel: FetchCancelHandle,
}

fn stream() -> (DownloadBody, BodyProducer) {
    let (chunks, receiver) = mpsc::unbounded_channel();
    let (completion, finished) = oneshot::channel();
    let cancel = FetchCancelHandle::new();
    let response = StreamingRawResponse::new(
        Url::parse("https://download.test/report.txt").unwrap(),
        200,
        Vec::new(),
        None,
        Vec::new(),
        false,
        Vec::new(),
        receiver,
        cancel.clone(),
        finished,
    );
    (
        DownloadBody::Streaming(Box::new(response)),
        BodyProducer {
            chunks,
            completion,
            cancel,
        },
    )
}

fn start(
    manager: &mut DownloadManager,
    directory: &TestDirectory,
    body: DownloadBody,
) -> DownloadObservation {
    manager
        .start_response(
            &directory.policy(),
            Url::parse("https://download.test/report.txt").unwrap(),
            Vec::new(),
            body,
        )
        .unwrap()
        .unwrap()
}

async fn wait_for(
    observation: &mut DownloadObservation,
    predicate: impl Fn(&DownloadSnapshot) -> bool,
) -> DownloadSnapshot {
    let mut snapshot = observation.snapshot();
    while !predicate(&snapshot) {
        snapshot = observation
            .next_update()
            .await
            .expect("transfer must publish the expected state before closing");
    }
    snapshot
}

async fn terminal(observation: &mut DownloadObservation) -> DownloadSnapshot {
    wait_for(observation, |snapshot| {
        snapshot.state != DownloadState::Active
    })
    .await
}

#[tokio::test]
async fn buffered_download_and_artifact_read_need_no_frontend() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let mut observation = start(
        &mut manager,
        &directory,
        DownloadBody::Buffered(b"artifact".to_vec()),
    );
    assert!(matches!(
        manager.read_artifact(observation.guid()),
        Some(Err(DownloadAccessError::InProgress))
    ));
    let snapshot = terminal(&mut observation).await;
    assert_eq!(
        snapshot.state,
        DownloadState::Completed {
            artifact_path: directory.0.join("report.txt")
        }
    );
    assert_eq!(snapshot.received_bytes, 8);
    assert_eq!(
        manager.cancel(observation.guid()),
        Some(Err(DownloadAccessError::AlreadyTerminal))
    );
    assert_eq!(
        manager
            .read_artifact(observation.guid())
            .unwrap()
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"artifact"
    );
    assert_eq!(
        observation.snapshot(),
        snapshot,
        "terminal cancellation must not change the artifact record"
    );
    assert_eq!(directory.files(), [directory.0.join("report.txt")]);
    assert!(manager.cancel("unknown").is_none());
    assert!(manager.read_artifact("unknown").is_none());
}

#[tokio::test]
async fn dropping_observation_does_not_cancel_a_download() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let observation = start(&mut manager, &directory, body);
    let mut monitor = observation.clone();
    drop(observation);
    producer.chunks.send(b"after detach".to_vec()).unwrap();
    drop(producer.chunks);
    producer.completion.send(Ok(())).unwrap();
    assert!(matches!(
        terminal(&mut monitor).await.state,
        DownloadState::Completed { .. }
    ));
    assert!(!producer.cancel.is_cancelled());
    assert_eq!(
        manager
            .read_artifact(monitor.guid())
            .unwrap()
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"after detach"
    );
}

#[tokio::test]
async fn context_retirement_cancels_a_stalled_body_and_removes_its_partial_before_terminal() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let mut observation = start(&mut manager, &directory, body);
    producer.chunks.send(b"partial".to_vec()).unwrap();
    wait_for(&mut observation, |snapshot| snapshot.received_bytes == 7).await;
    assert_eq!(directory.files().len(), 1);
    drop(manager);
    let snapshot = terminal(&mut observation).await;
    assert_eq!(snapshot.state, DownloadState::Canceled);
    assert_eq!(snapshot.received_bytes, 7);
    assert!(producer.cancel.is_cancelled());
    assert!(producer.chunks.is_closed());
    assert!(directory.files().is_empty());
    assert!(
        DownloadManager::default()
            .read_artifact(observation.guid())
            .is_none(),
        "a new Context must not inherit the retired Context's registry"
    );
}

#[tokio::test]
async fn explicit_cancel_terminates_a_stalled_body_without_waiting_for_network_completion() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let mut observation = start(&mut manager, &directory, body);
    producer.chunks.send(b"partial".to_vec()).unwrap();
    wait_for(&mut observation, |snapshot| snapshot.received_bytes == 7).await;
    assert_eq!(manager.cancel(observation.guid()), Some(Ok(())));
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Canceled
    );
    assert_eq!(
        manager.cancel(observation.guid()),
        Some(Err(DownloadAccessError::AlreadyTerminal))
    );
    assert!(matches!(
        manager.read_artifact(observation.guid()),
        Some(Err(DownloadAccessError::NoArtifact))
    ));
    assert!(producer.cancel.is_cancelled());
    assert!(directory.files().is_empty());
}

#[tokio::test]
async fn same_filename_downloads_have_independent_partial_file_ownership() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (first_body, first) = stream();
    let (second_body, second) = stream();
    let mut first_observation = start(&mut manager, &directory, first_body);
    let mut second_observation = start(&mut manager, &directory, second_body);
    first.chunks.send(b"discard".to_vec()).unwrap();
    second.chunks.send(b"keep".to_vec()).unwrap();
    wait_for(&mut first_observation, |snapshot| {
        snapshot.received_bytes == 7
    })
    .await;
    wait_for(&mut second_observation, |snapshot| {
        snapshot.received_bytes == 4
    })
    .await;
    let files = directory.files();
    assert_eq!(files.len(), 2);
    assert_ne!(files[0], files[1]);
    manager.cancel(first_observation.guid()).unwrap().unwrap();
    assert_eq!(
        terminal(&mut first_observation).await.state,
        DownloadState::Canceled
    );
    assert_eq!(
        directory.files().len(),
        1,
        "cancel must preserve the other transfer's partial"
    );
    assert!(first.cancel.is_cancelled());
    second.chunks.send(b" me".to_vec()).unwrap();
    drop(second.chunks);
    second.completion.send(Ok(())).unwrap();
    assert!(matches!(
        terminal(&mut second_observation).await.state,
        DownloadState::Completed { .. }
    ));
    assert_eq!(
        manager
            .read_artifact(second_observation.guid())
            .unwrap()
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"keep me"
    );
    assert_eq!(directory.files(), [directory.0.join("report.txt")]);
}

#[tokio::test]
async fn slow_observer_retains_start_metadata_and_terminal_state_without_blocking_transfer() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let mut slow = start(&mut manager, &directory, body);
    let mut monitor = slow.clone();
    for _ in 0..256 {
        producer.chunks.send(vec![1; 64]).unwrap();
    }
    drop(producer.chunks);
    producer.completion.send(Ok(())).unwrap();
    let final_state = terminal(&mut monitor).await;
    assert_eq!(final_state.received_bytes, 256 * 64);
    assert!(matches!(final_state.state, DownloadState::Completed { .. }));
    assert_eq!(slow.snapshot(), final_state);
    assert_eq!(
        final_state.metadata.unwrap().suggested_filename,
        "report.txt"
    );
    assert!(slow.next_update().await.is_none());
}

#[tokio::test]
async fn failed_stream_preserves_existing_artifact_and_cleans_partial() {
    let directory = TestDirectory::new();
    std::fs::write(directory.0.join("report.txt"), b"previous").unwrap();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let mut observation = start(&mut manager, &directory, body);
    producer.chunks.send(b"incomplete".to_vec()).unwrap();
    drop(producer.chunks);
    producer
        .completion
        .send(Err(anyhow::anyhow!("truncated body")))
        .unwrap();
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Canceled
    );
    assert_eq!(
        std::fs::read(directory.0.join("report.txt")).unwrap(),
        b"previous"
    );
    assert_eq!(directory.files(), [directory.0.join("report.txt")]);
}

#[tokio::test]
async fn admission_freezes_policy_and_guid_naming() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let mut policy = directory.policy();
    policy.behavior = DownloadBehavior::AllowAndName;
    let mut observation = manager
        .start_response(
            &policy,
            Url::parse("https://download.test/report.txt").unwrap(),
            Vec::new(),
            DownloadBody::Buffered(b"saved".to_vec()),
        )
        .unwrap()
        .unwrap();
    policy.behavior = DownloadBehavior::Deny;
    policy.download_path = None;
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Completed {
            artifact_path: directory.0.join(observation.guid())
        }
    );
    assert!(
        manager
            .start_response(
                &policy,
                Url::parse("https://download.test/denied").unwrap(),
                Vec::new(),
                DownloadBody::Buffered(Vec::new())
            )
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cancellation_interrupts_body_completion_after_all_chunks_arrived() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let (body, producer) = stream();
    let mut observation = start(&mut manager, &directory, body);
    producer.chunks.send(b"body".to_vec()).unwrap();
    drop(producer.chunks);
    wait_for(&mut observation, |snapshot| snapshot.received_bytes == 4).await;
    // Deliberately retain the producer's completion sender: neither EOF nor
    // the last chunk is permission to publish an artifact as completed.
    assert_eq!(manager.cancel(observation.guid()), Some(Ok(())));
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Canceled
    );
    assert!(producer.cancel.is_cancelled());
    assert!(directory.files().is_empty());
}

#[tokio::test]
async fn context_retirement_before_first_poll_cannot_publish_a_buffered_artifact() {
    let directory = TestDirectory::new();
    let mut manager = DownloadManager::default();
    let mut observation = start(
        &mut manager,
        &directory,
        DownloadBody::Buffered(b"canceled".to_vec()),
    );
    drop(manager);
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Canceled
    );
    assert!(directory.files().is_empty());
}

#[tokio::test]
async fn network_download_runs_without_a_frontend_and_uses_response_filename() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let directory = TestDirectory::new();
    let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/from-url.txt", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Disposition: attachment; filename=from-response.txt\r\nConnection: close\r\n\r\nbody").await.unwrap();
        socket.shutdown().await.unwrap();
        request
    });
    let mut manager = DownloadManager::default();
    let mut observation = manager
        .start_request(
            &directory.policy(),
            client.clone(),
            Request::get(&url).unwrap(),
            Some("from-hint.txt".into()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        observation.snapshot().metadata.unwrap().suggested_filename,
        "from-hint.txt"
    );
    let snapshot = terminal(&mut observation).await;
    assert_eq!(
        snapshot.state,
        DownloadState::Completed {
            artifact_path: directory.0.join("from-response.txt")
        }
    );
    assert_eq!(snapshot.received_bytes, 4);
    assert_eq!(
        snapshot.metadata.unwrap().suggested_filename,
        "from-hint.txt",
        "the early start identity must survive coalesced progress"
    );
    assert!(
        server
            .await
            .unwrap()
            .starts_with(b"GET /from-url.txt HTTP/1.1\r\n")
    );
    assert_eq!(
        manager
            .read_artifact(observation.guid())
            .unwrap()
            .unwrap()
            .await
            .unwrap()
            .unwrap(),
        b"body"
    );
}

#[tokio::test]
async fn context_retirement_cancels_network_download_before_response_headers() {
    use tokio::io::AsyncReadExt;

    let directory = TestDirectory::new();
    let client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/stalled", listener.local_addr().unwrap());
    let (requested, request_arrived) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        requested.send(()).unwrap();
        socket.read(&mut [0_u8; 1]).await.unwrap()
    });
    let mut manager = DownloadManager::default();
    let mut observation = manager
        .start_request(
            &directory.policy(),
            client.clone(),
            Request::get(&url).unwrap(),
            None,
        )
        .unwrap()
        .unwrap();
    request_arrived.await.unwrap();
    drop(manager);
    assert_eq!(
        terminal(&mut observation).await.state,
        DownloadState::Canceled
    );
    assert!(directory.files().is_empty());
    assert_eq!(
        server.await.unwrap(),
        0,
        "Context retirement must close the waiting transport, not only its observation"
    );
}
