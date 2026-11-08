use super::*;

#[test]
fn creation_io_errors_retry_the_same_generation_at_each_publication_boundary() {
    for boundary in [
        Boundary::MarkerSynced,
        Boundary::Allocated,
        Boundary::CreatingSynced,
    ] {
        let directory = Directory::new();
        let root = directory.0.join("idb-lite");
        let mut service = Service::on_disk(root.clone(), config()).unwrap();
        service.catalog.as_ref().unwrap().interrupt_at(boundary);
        let (sender, receiver) = mpsc::channel();
        let first = service
            .resolve(OWNER.to_vec(), "retry".into(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        assert!(matches!(
            receive(&receiver).result,
            Err(IndexedDbError::Io(_))
        ));
        assert!(first.state.failure.lock().is_none());
        let again = resolve(&service, OWNER, "retry");
        assert_eq!(again.id(), first.id());
        assert_eq!(again.identity().generation, first.identity().generation);
        seed(&again);
        verify_record(&again);
        service.shutdown().unwrap();
        let recovered = Service::on_disk(root, config()).unwrap();
        verify_record(&resolve(&recovered, OWNER, "retry"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn fd_exhaustion_during_a_burst_recovers_after_normal_idle_maintenance() {
    const CHILD: &str = "MOLI_TEST_IDB_LOW_FD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new("sh")
            .args(["-c", "ulimit -n 64; exec \"$@\"", "idb-low-fd-child"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "sqlite::service::tests::catalog_tests::open_retry_tests::fd_exhaustion_during_a_burst_recovers_after_normal_idle_maintenance", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "low-FD child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let directory = Directory::new();
    let service = Service::on_disk(directory.0.join("idb-lite"), config()).unwrap();
    let mut pending = Vec::new();
    for index in 0..80 {
        let name = format!("burst-{index}");
        let (sender, receiver) = mpsc::channel();
        let client = service
            .resolve(OWNER.to_vec(), name.clone(), move |reply| {
                let _ = sender.send(reply);
            })
            .unwrap();
        pending.push((name, client, receiver));
    }
    let mut failed = Vec::new();
    let mut opened = Vec::new();
    for (name, client, receiver) in pending {
        match receive(&receiver).result {
            Ok(CommandResult::Metadata(_)) => opened.push(client),
            Err(IndexedDbError::Io(_)) => {
                assert!(
                    client.state.failure.lock().is_none(),
                    "temporary I/O must not poison registration"
                );
                failed.push((name, client));
            }
            other => panic!("unexpected burst result: {other:?}"),
        }
    }
    assert!(!failed.is_empty(), "the burst must exercise the FD limit");
    assert!(!opened.is_empty());
    // Observe the real maintenance task. Never reap manually or advance its
    // idle clock; no Web connections or transactions keep these files active.
    let deadline = Instant::now() + Duration::from_secs(15);
    while opened.iter().any(physical_connection) {
        assert!(
            Instant::now() < deadline,
            "normal maintenance did not release the descriptors"
        );
        std::thread::yield_now();
    }
    for (name, original) in failed.into_iter().take(4) {
        let retry = resolve(&service, OWNER, &name);
        assert_eq!(retry.id(), original.id());
        assert_eq!(retry.identity().generation, original.identity().generation);
        seed(&retry);
        verify_record(&retry);
    }
}
