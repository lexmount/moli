use std::{sync::mpsc, time::Duration};

use super::*;

fn config() -> ExecutorConfig {
    ExecutorConfig {
        workers: 2,
        max_in_flight_bytes: 1024,
    }
}

fn submit(handle: &ExecutorHandle, key: u64, task: impl FnOnce(InFlightData) + Send + 'static) {
    handle.try_reserve(16).unwrap().submit(key, task).unwrap();
}

struct ReleaseOnDrop(Option<mpsc::Sender<()>>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[test]
fn same_database_jobs_are_fifo_and_shutdown_drains_every_accepted_job() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for value in 0..8 {
        let sender = sender.clone();
        submit(&handle, 42, move |_| sender.send(value).unwrap());
    }
    executor.shutdown().unwrap();
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert_eq!(handle.in_flight_bytes(), 0);
    assert!(matches!(
        handle.submit(0, || {}),
        Err(ExecutorError::Closed)
    ));
    executor.shutdown().unwrap();
}

#[test]
fn another_worker_progresses_while_one_worker_is_busy() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (release, wait) = mpsc::channel();
    let release_guard = ReleaseOnDrop(Some(release));
    let (started, start) = mpsc::channel();
    submit(&handle, 0, move |_| {
        let _ = started.send(());
        let _ = wait.recv();
    });
    start.recv_timeout(Duration::from_secs(5)).unwrap();
    let (sender, receiver) = mpsc::channel();
    submit(&handle, 1, move |_| {
        sender
            .send(thread::current().name().unwrap().to_owned())
            .unwrap()
    });
    let result = receiver.recv_timeout(Duration::from_secs(5));
    drop(release_guard);
    executor.shutdown().unwrap();
    assert_eq!(result.unwrap(), "moli-sqlite-1");
}

#[test]
fn undelivered_results_retain_bytes_without_starving_cleanup() {
    let mut settings = config();
    settings.max_in_flight_bytes = 32;
    let mut executor = Executor::new(settings).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    handle
        .try_reserve(32)
        .unwrap()
        .submit(0, move |data| sender.send(data).unwrap())
        .unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(result.reserved_bytes(), 32);
    assert_eq!(handle.in_flight_bytes(), 32);
    assert!(matches!(
        handle.try_reserve(1),
        Err(ExecutorError::Capacity)
    ));
    let (sender, receiver) = mpsc::channel();
    for value in 0..1024 {
        let sender = sender.clone();
        handle
            .submit(0, move || sender.send(value).unwrap())
            .unwrap();
    }
    for value in 0..1024 {
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            value
        );
    }
    assert_eq!(handle.in_flight_bytes(), 32);
    drop(result);
    executor.shutdown().unwrap();
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn canceled_and_closed_submissions_release_their_capacity() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let canceled = handle.try_reserve(24).unwrap();
    drop(canceled);
    assert_eq!(handle.in_flight_bytes(), 0);
    let late = handle.try_reserve(24).unwrap();
    executor.shutdown().unwrap();
    assert!(matches!(
        late.submit(0, |_| panic!("closed submission must not run")),
        Err(ExecutorError::Closed)
    ));
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn byte_budget_rejects_overflow_and_allows_zero_byte_work_without_a_task_limit() {
    let executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    assert!(matches!(
        handle.try_reserve(usize::MAX),
        Err(ExecutorError::Capacity)
    ));
    let first = handle.try_reserve(1023).unwrap();
    assert!(matches!(
        handle.try_reserve(2),
        Err(ExecutorError::Capacity)
    ));
    let second = handle.try_reserve(1).unwrap();
    let held: Vec<_> = (0..1024).map(|_| handle.try_reserve(0).unwrap()).collect();
    assert_eq!(handle.in_flight_bytes(), 1024);
    drop((first, second, held));
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn independent_transactions_can_wait_between_commands_on_the_same_worker() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    let connections = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    for key in [0, 2] {
        let connections = connections.clone();
        let sender = sender.clone();
        submit(&handle, key, move |_| {
            let connection = crate::open_private_memory(Default::default()).unwrap();
            connection
                .execute_batch("CREATE TABLE saved(value); BEGIN; INSERT INTO saved VALUES(1);")
                .unwrap();
            connections.lock().insert(key, connection);
            sender.send(key).unwrap();
        });
    }
    // Both transactions have started although neither has committed yet.
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    let owners = connections.clone();
    submit(&handle, 0, move |_| {
        let connection = owners.lock().remove(&0).unwrap();
        connection.execute_batch("COMMIT").unwrap();
    });
    submit(&handle, 2, move |_| {
        let connection = connections.lock().remove(&2).unwrap();
        connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM saved", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    });
    executor.shutdown().unwrap();
}

#[test]
fn shutdown_racing_with_submit_never_drops_an_accepted_task() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let permit = handle.try_reserve(1).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let gate = barrier.clone();
    let (sender, receiver) = mpsc::channel();
    let submitter = thread::spawn(move || {
        gate.wait();
        permit.submit(0, move |_| sender.send(()).unwrap())
    });
    barrier.wait();
    executor.shutdown().unwrap();
    match submitter.join().unwrap() {
        Ok(()) => receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        Err(ExecutorError::Closed) => assert!(receiver.recv().is_err()),
        Err(error) => panic!("unexpected submit error: {error}"),
    }
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn shutdown_finalizers_run_on_workers_when_all_data_bytes_are_retained() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let held = handle.try_reserve(config().max_in_flight_bytes).unwrap();
    let (sender, receiver) = mpsc::channel();
    executor
        .shutdown_with(move |lane| {
            sender
                .send((lane, thread::current().name().unwrap().to_owned()))
                .unwrap()
        })
        .unwrap();
    let mut finished: Vec<_> = receiver.try_iter().collect();
    finished.sort();
    assert_eq!(
        finished,
        [(0, "moli-sqlite-0".into()), (1, "moli-sqlite-1".into())]
    );
    assert_eq!(handle.in_flight_bytes(), config().max_in_flight_bytes);
    drop(held);
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn reservation_growth_is_atomic_and_shrink_releases_bytes_after_close() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let mut permit = handle.try_reserve(16).unwrap();
    assert!(permit.belongs_to(&handle));
    permit.try_resize(1024).unwrap();
    assert_eq!(handle.in_flight_bytes(), 1024);
    assert!(matches!(
        permit.try_resize(1025),
        Err(ExecutorError::Capacity)
    ));
    assert_eq!(permit.reserved_bytes(), 1024);
    executor.shutdown().unwrap();
    permit.try_resize(8).unwrap();
    assert!(matches!(permit.try_resize(16), Err(ExecutorError::Closed)));
    assert_eq!(handle.in_flight_bytes(), 8);
    drop(permit);
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn accepted_commands_progress_before_earlier_results_are_consumed() {
    let mut settings = config();
    settings.max_in_flight_bytes = 4096;
    let mut executor = Executor::new(settings).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for value in 0..8 {
        let sender = sender.clone();
        handle
            .try_reserve(512)
            .unwrap()
            .submit(0, move |data| sender.send((value, data)).unwrap())
            .unwrap();
    }
    let mut results = Vec::new();
    for expected in 0..8 {
        let (value, data) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, expected);
        results.push(data);
    }
    assert_eq!(handle.in_flight_bytes(), 4096);
    assert!(matches!(
        handle.try_reserve(1),
        Err(ExecutorError::Capacity)
    ));
    executor.shutdown().unwrap();
    drop(results);
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn data_and_lifecycle_work_share_database_fifo() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (release, wait) = mpsc::channel();
    let guard = ReleaseOnDrop(Some(release));
    handle
        .submit(0, move || {
            let _ = wait.recv();
        })
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    for value in 0..16 {
        let sender = sender.clone();
        if value % 2 == 0 {
            handle
                .try_reserve(16)
                .unwrap()
                .submit(0, move |_| sender.send(value).unwrap())
                .unwrap();
        } else {
            handle
                .submit(0, move || sender.send(value).unwrap())
                .unwrap();
        }
    }
    assert!(receiver.try_recv().is_err());
    drop(guard);
    executor.shutdown().unwrap();
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        (0..16).collect::<Vec<_>>()
    );
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn shutdown_drains_accepted_jobs_while_results_are_retained() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for _ in 0..8 {
        let sender = sender.clone();
        handle
            .try_reserve(128)
            .unwrap()
            .submit(0, move |data| sender.send(data).unwrap())
            .unwrap();
    }
    executor.shutdown().unwrap();
    assert_eq!(handle.in_flight_bytes(), 1024);
    drop(receiver.try_iter().collect::<Vec<_>>());
    assert_eq!(handle.in_flight_bytes(), 0);
}

#[test]
fn retained_results_can_resize_without_execution_slot_bookkeeping() {
    let mut settings = config();
    settings.max_in_flight_bytes = 4096;
    let mut executor = Executor::new(settings).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for _ in 0..2 {
        let sender = sender.clone();
        handle
            .try_reserve(512)
            .unwrap()
            .submit(0, move |data| sender.send(data).unwrap())
            .unwrap();
    }
    let mut retained = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let next = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(handle.in_flight_bytes(), 1024);
    retained.try_resize(1024).unwrap();
    assert_eq!(handle.in_flight_bytes(), 1536);
    drop((retained, next));
    executor.shutdown().unwrap();
    assert_eq!(handle.in_flight_bytes(), 0);
}
