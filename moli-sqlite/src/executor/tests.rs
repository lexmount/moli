use std::{sync::mpsc, time::Duration};

use super::*;

fn config() -> ExecutorConfig {
    ExecutorConfig { workers: 2 }
}

fn submit(handle: &ExecutorHandle, key: u64, task: impl FnOnce() + Send + 'static) {
    handle.submit(key, task).unwrap();
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
        submit(&handle, 42, move || sender.send(value).unwrap());
    }
    executor.shutdown().unwrap();
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );

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
    submit(&handle, 0, move || {
        let _ = started.send(());
        let _ = wait.recv();
    });
    start.recv_timeout(Duration::from_secs(5)).unwrap();
    let (sender, receiver) = mpsc::channel();
    submit(&handle, 1, move || {
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
fn independent_transactions_can_wait_between_commands_on_the_same_worker() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    let connections = Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    for key in [0, 2] {
        let connections = connections.clone();
        let sender = sender.clone();
        submit(&handle, key, move || {
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
    submit(&handle, 0, move || {
        let connection = owners.lock().remove(&0).unwrap();
        connection.execute_batch("COMMIT").unwrap();
    });
    submit(&handle, 2, move || {
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

    let barrier = Arc::new(std::sync::Barrier::new(2));
    let gate = barrier.clone();
    let (sender, receiver) = mpsc::channel();
    let submitter = thread::spawn(move || {
        gate.wait();
        handle.submit(0, move || sender.send(()).unwrap())
    });
    barrier.wait();
    executor.shutdown().unwrap();
    match submitter.join().unwrap() {
        Ok(()) => receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        Err(ExecutorError::Closed) => assert!(receiver.recv().is_err()),
        Err(error) => panic!("unexpected submit error: {error}"),
    }
}

#[test]
fn shutdown_finalizers_run_on_their_workers() {
    let mut executor = Executor::new(config()).unwrap();

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
}

#[test]
fn accepted_commands_progress_before_earlier_results_are_consumed() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for value in 0..8 {
        let sender = sender.clone();
        handle
            .submit(0, move || sender.send((value, vec![0; 512])).unwrap())
            .unwrap();
    }
    let mut results = Vec::new();
    for expected in 0..8 {
        let (value, data) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(value, expected);
        results.push(data);
    }

    executor.shutdown().unwrap();
    assert_eq!(results.len(), 8);
    assert!(results.iter().all(|bytes| bytes.len() == 512));
    drop(results);
}

#[test]
fn accepted_work_keeps_database_fifo() {
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
        handle
            .submit(0, move || sender.send(value).unwrap())
            .unwrap();
    }
    assert!(receiver.try_recv().is_err());
    drop(guard);
    executor.shutdown().unwrap();
    assert_eq!(
        receiver.try_iter().collect::<Vec<_>>(),
        (0..16).collect::<Vec<_>>()
    );
}

#[test]
fn shutdown_drains_accepted_jobs_while_results_are_retained() {
    let mut executor = Executor::new(config()).unwrap();
    let handle = executor.handle();
    let (sender, receiver) = mpsc::channel();
    for _ in 0..8 {
        let sender = sender.clone();
        handle
            .submit(0, move || sender.send(vec![0; 128]).unwrap())
            .unwrap();
    }
    executor.shutdown().unwrap();

    let results = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(results.len(), 8);
    assert!(results.iter().all(|bytes| bytes.len() == 128));
}
