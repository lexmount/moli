use super::*;
use crate::Runtime;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    thread,
};
use tokio::sync::oneshot;

#[derive(Default)]
struct Control {
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
    fail: AtomicBool,
    attempts: AtomicUsize,
    attempted: Mutex<Option<oneshot::Sender<()>>>,
    commits: AtomicUsize,
    committed: Mutex<Option<oneshot::Sender<()>>>,
}
#[derive(Default)]
struct Batch {
    values: BTreeMap<i64, i64>,
    control: Option<Arc<Control>>,
}
impl Batch {
    fn put(key: i64, value: i64, control: &Arc<Control>) -> Self {
        Self {
            values: BTreeMap::from([(key, value)]),
            control: Some(control.clone()),
        }
    }
}
impl WriteBatch for Batch {
    fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    fn merge(&mut self, newer: Self) {
        self.values.extend(newer.values);
        self.control = newer.control.or(self.control.take());
    }
    fn write(&self, connection: &mut Connection) -> Result<()> {
        let control = self.control.as_ref().unwrap();
        control.attempts.fetch_add(1, Ordering::AcqRel);
        if let Some(attempted) = control.attempted.lock().take() {
            let _ = attempted.send(());
        }
        let gate = control.gate.lock().take();
        if let Some((started, release)) = gate {
            started.send(()).unwrap();
            release.recv().unwrap();
        }
        if control.fail.swap(false, Ordering::AcqRel) {
            return Err(WritebackError::new("injected failure"));
        }
        let tx = connection.transaction()?;
        for (key, value) in &self.values {
            tx.execute("INSERT INTO data VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",rusqlite::params![key,value])?;
        }
        tx.commit()?;
        control.commits.fetch_add(1, Ordering::AcqRel);
        if let Some(done) = control.committed.lock().take() {
            let _ = done.send(());
        }
        Ok(())
    }
}
fn initialize(connection: &mut Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE data(key INTEGER PRIMARY KEY,value INTEGER)")?;
    Ok(())
}
fn long_policy() -> WritebackPolicy {
    WritebackPolicy {
        delay: Duration::from_secs(3600),
        early_commit_operations: None,
    }
}
fn owner(runtime: &Runtime, path: PathBuf) -> WritebackOwner<Batch> {
    WritebackOwner::open(
        path,
        OpenMode::Create,
        runtime.handle(),
        long_policy(),
        initialize,
    )
    .unwrap()
}
fn value(client: &WritebackClient<Batch>, key: i64) -> i64 {
    client
        .query(move |connection| {
            Ok(connection.query_row("SELECT value FROM data WHERE key=?1", [key], |r| r.get(0))?)
        })
        .unwrap()
}

#[test]
fn an_unused_deferred_store_never_opens_even_when_flushed_or_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unused/store.sqlite");
    let runtime = Runtime::new().unwrap();
    let owner = WritebackOwner::<Batch>::deferred(
        path.clone(),
        OpenMode::Create,
        runtime.handle(),
        long_policy(),
        |_| panic!("unused store must not initialize"),
    )
    .unwrap();
    let client = owner.client();
    client.ensure_open().unwrap();
    client.request_flush();
    client.flush().unwrap();
    owner.close().unwrap();
    assert!(client.query(|_| Ok(())).is_err());
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn concurrent_first_queries_initialize_one_reused_connection() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let initialized = Arc::new(AtomicUsize::new(0));
    let count = initialized.clone();
    let owner = WritebackOwner::<Batch>::deferred(
        root.path().join("first.sqlite"),
        OpenMode::Create,
        runtime.handle(),
        long_policy(),
        move |connection| {
            count.fetch_add(1, Ordering::SeqCst);
            initialize(connection)?;
            // A temporary table identifies this connection, not just the file.
            connection.execute_batch(
                "CREATE TEMP TABLE session(value); INSERT INTO session VALUES (42)",
            )?;
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(initialized.load(Ordering::SeqCst), 0);
    thread::scope(|scope| {
        for _ in 0..8 {
            let client = owner.client();
            scope.spawn(move || {
                assert_eq!(
                    client
                        .query(|connection| Ok(connection.query_row(
                            "SELECT value FROM session",
                            [],
                            |r| r.get::<_, i64>(0)
                        )?))
                        .unwrap(),
                    42
                );
            });
        }
    });
    assert_eq!(initialized.load(Ordering::SeqCst), 1);
    owner.close().unwrap();
}

#[test]
fn deferred_initialization_errors_reach_queries_flush_and_close() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let owner = WritebackOwner::<Batch>::deferred(
        root.path().join("broken.sqlite"),
        OpenMode::Create,
        runtime.handle(),
        long_policy(),
        |_| Err(WritebackError::new("injected schema failure")),
    )
    .unwrap();
    let client = owner.client();
    client.flush().unwrap();
    for _ in 0..2 {
        assert_eq!(
            client
                .query::<()>(|_| panic!("query must not run"))
                .unwrap_err()
                .to_string(),
            "injected schema failure"
        );
    }
    assert_eq!(
        client.flush().unwrap_err().to_string(),
        "injected schema failure"
    );
    assert_eq!(
        owner.close().unwrap_err().to_string(),
        "injected schema failure"
    );
}

#[test]
fn a_first_write_opens_and_close_commits_without_a_query() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("write.sqlite");
    let runtime = Runtime::new().unwrap();
    let owner = WritebackOwner::<Batch>::deferred(
        path.clone(),
        OpenMode::Create,
        runtime.handle(),
        long_policy(),
        initialize,
    )
    .unwrap();
    let control = Arc::new(Control::default());
    owner.client().record(Batch::put(1, 42, &control)).unwrap();
    assert!(!path.exists());
    owner.close().unwrap();
    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM data WHERE key=1", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        42
    );
}

#[test]
fn an_in_flight_acknowledgement_cannot_clear_a_newer_batch_and_another_file_can_progress() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let first = owner(&runtime, root.path().join("first.sqlite"));
    let second = owner(&runtime, root.path().join("second.sqlite"));
    let control = Arc::new(Control::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    *control.gate.lock() = Some((started_tx, release_rx));
    let client = first.client();
    client.record(Batch::put(1, 10, &control)).unwrap();
    client.request_flush();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    client.record(Batch::put(1, 20, &control)).unwrap();
    let other = Arc::new(Control::default());
    second.client().record(Batch::put(2, 42, &other)).unwrap();
    second.client().flush().unwrap();
    assert_eq!(value(&second.client(), 2), 42);
    release_tx.send(()).unwrap();
    assert_eq!(value(&client, 1), 10);
    client.flush().unwrap();
    assert_eq!(value(&client, 1), 20);
    assert_eq!(control.commits.load(Ordering::Acquire), 2);
    first.close().unwrap();
    assert!(client.record(Batch::put(1, 99, &control)).is_err());
    second.client().record(Batch::put(2, 43, &other)).unwrap();
    second.close().unwrap();
}
#[test]
fn discarded_failure_does_not_retry_or_block_new_changes() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let owner = owner(&runtime, root.path().join("failure.sqlite"));
    let client = owner.client();
    let control = Arc::new(Control::default());
    control.fail.store(true, Ordering::Release);
    client.record(Batch::put(1, 10, &control)).unwrap();
    assert!(client.flush().is_err());
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    assert!(client.flush().is_err());
    assert_eq!(
        control.attempts.load(Ordering::Acquire),
        1,
        "flush reports the discarded batch error without replaying it"
    );
    client.record(Batch::put(2, 30, &control)).unwrap();
    client.flush().unwrap();
    assert!(client.failure().is_none());
    assert_eq!(value(&client, 2), 30);
    assert_eq!(
        client
            .query(|connection| Ok(connection.query_row(
                "SELECT count(*) FROM data WHERE key=1",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
    let retained = client.clone();
    thread::spawn(move || owner.close().unwrap())
        .join()
        .unwrap();
    assert!(retained.query(|_| Ok(())).is_err());
}

#[test]
fn queued_flush_commits_pending_edits_before_the_following_query() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let owner = owner(&runtime, root.path().join("flush-order.sqlite"));
    let client = owner.client();
    let control = Arc::new(Control::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let blocked_client = client.clone();
    let blocked = thread::spawn(move || {
        blocked_client.query(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    client.record(Batch::put(1, 10, &control)).unwrap();
    let (reply, receipt) = mpsc::channel();
    client.sender.send(Message::Flush { reply }).unwrap();
    client.record(Batch::put(2, 20, &control)).unwrap();
    release_tx.send(()).unwrap();
    blocked.join().unwrap().unwrap();
    receipt
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(value(&client, 1), 10);
    assert_eq!(value(&client, 2), 20);
    assert_eq!(control.commits.load(Ordering::Acquire), 1);
    owner.close().unwrap();
    assert_eq!(control.commits.load(Ordering::Acquire), 1);
}
fn virtual_client(
    runtime: &Runtime,
    policy: WritebackPolicy,
) -> (WritebackClient<Batch>, tokio::task::JoinHandle<Result<()>>) {
    let mut connection = Connection::open_in_memory().unwrap();
    initialize(&mut connection).unwrap();
    let shared = Arc::new(Shared {
        pending: Mutex::new(Pending {
            batch: Batch::default(),
            operations: 0,
            first_dirty: None,
            failure: None,
            closed: false,
        }),
        changed: Notify::new(),
    });
    let (sender, receiver) = async_mpsc::unbounded_channel();
    let client = WritebackClient {
        shared: shared.clone(),
        sender,
        runtime: runtime.handle(),
    };
    let task = tokio::spawn(run(
        LazyConnection::Open(connection),
        receiver,
        shared,
        runtime.handle(),
        policy,
    ));
    (client, task)
}
#[tokio::test(start_paused = true)]
async fn the_first_dirty_deadline_is_not_extended_by_later_writes() {
    let runtime = Runtime::new().unwrap();
    let (client, task) = virtual_client(
        &runtime,
        WritebackPolicy {
            delay: Duration::from_secs(5),
            early_commit_operations: None,
        },
    );
    let control = Arc::new(Control::default());
    let (done_tx, done_rx) = oneshot::channel();
    *control.committed.lock() = Some(done_tx);
    client.record(Batch::put(1, 10, &control)).unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(2500)).await;
    client.record(Batch::put(1, 20, &control)).unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(2499)).await;
    tokio::task::yield_now().await;
    assert_eq!(control.commits.load(Ordering::Acquire), 0);
    tokio::time::advance(Duration::from_millis(1)).await;
    done_rx.await.unwrap();
    assert_eq!(control.commits.load(Ordering::Acquire), 1);
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    task.await.unwrap().unwrap();
}
#[tokio::test(start_paused = true)]
async fn early_commit_counts_accepted_operations_even_when_the_same_key_is_coalesced() {
    let runtime = Runtime::new().unwrap();
    let (client, task) = virtual_client(
        &runtime,
        WritebackPolicy {
            delay: Duration::from_secs(30),
            early_commit_operations: Some(512),
        },
    );
    let control = Arc::new(Control::default());
    let (done_tx, done_rx) = oneshot::channel();
    *control.committed.lock() = Some(done_tx);
    let start = tokio::time::Instant::now();
    for value in 0..512 {
        client.record(Batch::put(1, value, &control)).unwrap();
    }
    assert_eq!(client.shared.pending.lock().batch.values.len(), 1);
    done_rx.await.unwrap();
    assert_eq!(tokio::time::Instant::now(), start);
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    task.await.unwrap().unwrap();
}

struct Barrier(oneshot::Sender<()>);
impl Query for Barrier {
    fn run(self: Box<Self>, _: &mut LazyConnection) {
        let _ = self.0.send(());
    }
}
async fn barrier(client: &WritebackClient<Batch>) {
    let (tx, rx) = oneshot::channel();
    client
        .sender
        .send(Message::Query(Box::new(Barrier(tx))))
        .unwrap();
    rx.await.unwrap();
}
fn next_attempt(control: &Control) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    *control.attempted.lock() = Some(tx);
    rx
}
fn prevent_time_auto_advance() -> tokio::task::JoinHandle<()> {
    // Blocking SQL runs on another runtime. Keep virtual time under the test's
    // control while awaiting those workers, instead of skipping normal deadlines.
    tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    })
}

#[tokio::test(start_paused = true)]
async fn a_discarded_failure_does_not_rearm_idle_timers_or_shorten_new_dirty_delay() {
    let awake = prevent_time_auto_advance();
    let runtime = Runtime::new().unwrap();
    let (client, task) = virtual_client(
        &runtime,
        WritebackPolicy {
            delay: Duration::from_secs(30),
            early_commit_operations: Some(512),
        },
    );
    let control = Arc::new(Control::default());
    control.fail.store(true, Ordering::Release);
    let attempted = next_attempt(&control);
    client.record(Batch::put(1, 10, &control)).unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(30)).await;
    attempted.await.unwrap();
    barrier(&client).await;
    assert!(client.failure().is_some());
    tokio::time::advance(Duration::from_secs(3600)).await;
    barrier(&client).await;
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    client.record(Batch::put(2, 20, &control)).unwrap();
    barrier(&client).await;
    let attempted = next_attempt(&control);
    tokio::time::advance(Duration::from_millis(29999)).await;
    barrier(&client).await;
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    tokio::time::advance(Duration::from_millis(1)).await;
    attempted.await.unwrap();
    barrier(&client).await;
    assert_eq!(control.commits.load(Ordering::Acquire), 1);
    assert!(client.failure().is_none());
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    task.await.unwrap().unwrap();
    awake.abort();
}

#[tokio::test(start_paused = true)]
async fn a_failed_early_commit_resets_its_operation_count_and_allows_the_next_batch() {
    let awake = prevent_time_auto_advance();
    let runtime = Runtime::new().unwrap();
    let (client, task) = virtual_client(
        &runtime,
        WritebackPolicy {
            delay: Duration::from_secs(30),
            early_commit_operations: Some(512),
        },
    );
    let control = Arc::new(Control::default());
    control.fail.store(true, Ordering::Release);
    let attempted = next_attempt(&control);
    for value in 0..512 {
        client.record(Batch::put(1, value, &control)).unwrap();
    }
    attempted.await.unwrap();
    barrier(&client).await;
    assert!(client.failure().is_some());
    for value in 0..511 {
        client.record(Batch::put(2, value, &control)).unwrap();
    }
    barrier(&client).await;
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    let attempted = next_attempt(&control);
    let now = tokio::time::Instant::now();
    client.record(Batch::put(2, 511, &control)).unwrap();
    attempted.await.unwrap();
    barrier(&client).await;
    assert_eq!(tokio::time::Instant::now(), now);
    assert_eq!(control.attempts.load(Ordering::Acquire), 2);
    assert!(client.failure().is_none());
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    task.await.unwrap().unwrap();
    awake.abort();
}

#[tokio::test(start_paused = true)]
async fn closing_after_a_discarded_failure_reports_it_without_replaying_the_batch() {
    let awake = prevent_time_auto_advance();
    let runtime = Runtime::new().unwrap();
    let (client, task) = virtual_client(
        &runtime,
        WritebackPolicy {
            delay: Duration::from_secs(30),
            early_commit_operations: Some(1),
        },
    );
    let control = Arc::new(Control::default());
    control.fail.store(true, Ordering::Release);
    let attempted = next_attempt(&control);
    client.record(Batch::put(1, 10, &control)).unwrap();
    attempted.await.unwrap();
    barrier(&client).await;
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(control.attempts.load(Ordering::Acquire), 1);
    awake.abort();
}
