use super::*;
use std::sync::{Barrier, atomic::AtomicUsize};

#[test]
fn database_fifo_drains_without_consuming_its_completions() {
    let mut service = Service::new(config()).unwrap();
    let client = open(&service, "fifo", Location::Memory);
    let held = hold_database(&client);
    let (sender, receiver) = mpsc::channel();
    for value in 0..32 {
        let sender = sender.clone();
        client
            .owner
            .submit(
                move |owner| {
                    let config = owner.config;
                    let entry = owner.entry()?;
                    entry.connect(config)?;
                    let result: i64 = entry.database.as_ref().unwrap().connection.query_row(
                        "SELECT ?1",
                        [value],
                        |row| row.get(0),
                    )?;
                    Ok(result)
                },
                move |result| {
                    let _ = sender.send(result);
                },
            )
            .unwrap();
    }
    drop(sender);
    drop(held);
    service.shutdown().unwrap();
    let values = receiver.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(values, (0..32).collect::<Vec<_>>());
}

#[test]
fn another_database_runs_while_a_blocking_command_waits() {
    let service = Service::new(config()).unwrap();
    let slow = open(&service, "slow", Location::Memory);
    let fast = open(&service, "fast", Location::Memory);
    let held = hold_database(&slow);
    with_database(&fast, |database| {
        let value: i64 = database
            .connection
            .query_row("SELECT 42", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 42);
    });
    drop(held);
}

#[test]
fn logical_connection_registration_follows_accepted_commands() {
    let service = Service::new(config()).unwrap();
    let client = open(&service, "register-fifo", Location::Memory);
    let held = hold_database(&client);
    let (sender, receiver) = mpsc::channel();
    client
        .open_connection(Arc::new(|| {}), move |reply| {
            let _ = sender.send(reply);
        })
        .unwrap();
    assert!(!client.has_open_connections());
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(held);
    let CommandResult::Connection(connection) = receive(&receiver).result.unwrap() else {
        panic!("connection receipt");
    };
    assert!(client.has_open_connections());
    assert!(!connection.is_closed());
}

#[test]
fn a_panicked_database_rejects_queued_work_without_replay_and_other_databases_continue() {
    let mut service = Service::new(config()).unwrap();
    let client = open(&service, "failed", Location::Memory);
    initialize(&client);
    let connection = client.open_connection_for_test(Arc::new(|| {})).unwrap();
    let other = open(&service, "survivor", Location::Memory);
    let held = hold_database(&client);
    let (sender, receiver) = mpsc::channel();
    let failed = sender.clone();
    client
        .owner
        .submit::<()>(
            |owner| {
                assert!(owner.entry()?.database.is_some());
                panic!("injected database command panic");
            },
            move |result| {
                let _ = failed.send(result);
            },
        )
        .unwrap();
    let executed = Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        let count = executed.clone();
        let sender = sender.clone();
        client
            .owner
            .submit(
                move |_| {
                    count.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
                move |result| {
                    let _ = sender.send(result);
                },
            )
            .unwrap();
    }
    drop(sender);
    drop(held);
    for _ in 0..4 {
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .is_err()
        );
    }
    assert_eq!(executed.load(Ordering::Relaxed), 0);
    assert!(client.is_retired());
    assert!(connection.is_forced_closed());
    with_database(&other, |database| {
        let value: i64 = database
            .connection
            .query_row("SELECT 7", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, 7);
    });
    assert!(service.shutdown().is_err());
}

#[test]
fn shutdown_orders_its_stop_after_every_accepted_concurrent_submission() {
    let mut service = Service::new(config()).unwrap();
    let client = open(&service, "shutdown-race", Location::Memory);
    let accepted = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(5));
    let submitters: Vec<_> = (0..4)
        .map(|_| {
            let owner = client.owner.clone();
            let accepted = accepted.clone();
            let completed = completed.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                for index in 0..129 {
                    if index == 1 {
                        // Every producer has accepted work before shutdown
                        // races the remaining submissions.
                        barrier.wait();
                    }
                    let completed = completed.clone();
                    if owner
                        .submit(
                            |owner| {
                                owner.entry()?;
                                Ok(())
                            },
                            move |result| {
                                result.unwrap();
                                completed.fetch_add(1, Ordering::Relaxed);
                            },
                        )
                        .is_err()
                    {
                        break;
                    }
                    accepted.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    barrier.wait();
    service.shutdown().unwrap();
    for submitter in submitters {
        submitter.join().unwrap();
    }
    assert!(accepted.load(Ordering::Relaxed) >= 4);
    assert_eq!(
        completed.load(Ordering::Relaxed),
        accepted.load(Ordering::Relaxed)
    );
    assert!(client.owner.submit(|_| Ok(()), |_| {}).is_err());
}
