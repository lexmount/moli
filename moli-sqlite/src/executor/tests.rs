use std::{sync::mpsc, time::Duration};

use super::*;

fn config() -> ExecutorConfig {
    ExecutorConfig {
        workers: 2,
        data: Capacity {
            tasks: 8,
            bytes: 1024,
        },
        control: Capacity {
            tasks: 2,
            bytes: 128,
        },
    }
}

fn submit(handle: &ExecutorHandle, key: u64, task: impl FnOnce(TaskPermit) + Send + 'static) {
    handle
        .try_reserve(TaskKind::Data, 16)
        .unwrap()
        .submit(key, task)
        .unwrap();
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
    assert_eq!(handle.usage(), QueueUsage::default());
    assert!(matches!(
        handle.try_reserve(TaskKind::Control, 0),
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
fn undelivered_results_retain_bytes_and_cannot_starve_control_work() {
    let mut settings = config();
    settings.data = Capacity {
        tasks: 1,
        bytes: 32,
    };
    let mut executor = Executor::new(settings).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    handle
        .try_reserve(TaskKind::Data, 32)
        .unwrap()
        .submit(0, move |permit| {
            assert!(sender.send(permit).is_ok());
        })
        .unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(result.reserved_bytes(), 32);
    assert_eq!(handle.usage().data_bytes, 32);
    assert!(matches!(
        handle.try_reserve(TaskKind::Data, 0),
        Err(ExecutorError::Capacity)
    ));
    let (sender, receiver) = mpsc::channel();
    handle
        .try_reserve(TaskKind::Control, 16)
        .unwrap()
        .submit(0, move |_| sender.send(()).unwrap())
        .unwrap();
    receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(result);
    executor.shutdown().unwrap();
    assert_eq!(handle.usage(), QueueUsage::default());
}

#[test]
fn canceled_and_closed_submissions_release_their_capacity() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let canceled = handle.try_reserve(TaskKind::Data, 24).unwrap();
    drop(canceled);
    assert_eq!(handle.usage(), QueueUsage::default());
    let late = handle.try_reserve(TaskKind::Data, 24).unwrap();
    executor.shutdown().unwrap();
    assert!(matches!(
        late.submit(0, |_| panic!("closed submission must not run")),
        Err(ExecutorError::Closed)
    ));
    assert_eq!(handle.usage(), QueueUsage::default());
}

#[test]
fn byte_budget_rejects_overflow_and_control_has_its_own_limit() {
    let executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    assert!(matches!(
        handle.try_reserve(TaskKind::Data, usize::MAX),
        Err(ExecutorError::Capacity)
    ));
    let first = handle.try_reserve(TaskKind::Control, 127).unwrap();
    assert!(matches!(
        handle.try_reserve(TaskKind::Control, 2),
        Err(ExecutorError::Capacity)
    ));
    let second = handle.try_reserve(TaskKind::Control, 1).unwrap();
    assert!(matches!(
        handle.try_reserve(TaskKind::Control, 0),
        Err(ExecutorError::Capacity)
    ));
    let data = handle.try_reserve(TaskKind::Data, 1024).unwrap();
    drop((first, second, data));
    assert_eq!(handle.usage(), QueueUsage::default());
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
    let permit = handle.try_reserve(TaskKind::Data, 1).unwrap();
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
    assert_eq!(handle.usage(), QueueUsage::default());
}

#[test]
fn shutdown_finalizers_run_on_workers_even_when_all_permits_are_retained() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let mut held = Vec::new();
    for _ in 0..config().data.tasks {
        held.push(handle.try_reserve(TaskKind::Data, 0).unwrap());
    }
    for _ in 0..config().control.tasks {
        held.push(handle.try_reserve(TaskKind::Control, 0).unwrap());
    }
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
    drop(held);
    assert_eq!(handle.usage(), QueueUsage::default());
}

#[test]
fn reservation_growth_is_atomic_and_shrink_releases_bytes_after_close() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let mut permit = handle.try_reserve(TaskKind::Data, 16).unwrap();
    assert!(permit.belongs_to(&handle));
    assert_eq!(permit.kind(), TaskKind::Data);
    permit.try_resize(1024).unwrap();
    assert_eq!(handle.usage().data_bytes, 1024);
    assert!(matches!(
        permit.try_resize(1025),
        Err(ExecutorError::Capacity)
    ));
    assert_eq!(permit.reserved_bytes(), 1024);
    executor.shutdown().unwrap();
    permit.try_resize(8).unwrap();
    assert!(matches!(permit.try_resize(16), Err(ExecutorError::Closed)));
    assert_eq!(handle.usage().data_bytes, 8);
    drop(permit);
    assert_eq!(handle.usage(), QueueUsage::default());
}
