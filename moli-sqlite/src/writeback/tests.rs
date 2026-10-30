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
    client
        .record(Batch::put(1, 10, &control), FailureAdmission::Retain)
        .unwrap();
    client.request_flush();
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    client
        .record(Batch::put(1, 20, &control), FailureAdmission::Retain)
        .unwrap();
    let other = Arc::new(Control::default());
    second
        .client()
        .record(Batch::put(2, 42, &other), FailureAdmission::Retain)
        .unwrap();
    second.client().flush().unwrap();
    assert_eq!(value(&second.client(), 2), 42);
    release_tx.send(()).unwrap();
    assert_eq!(value(&client, 1), 10);
    client.flush().unwrap();
    assert_eq!(value(&client, 1), 20);
    assert_eq!(control.commits.load(Ordering::Acquire), 2);
    first.close().unwrap();
    assert!(
        client
            .record(Batch::put(1, 99, &control), FailureAdmission::Retain)
            .is_err()
    );
    second
        .client()
        .record(Batch::put(2, 43, &other), FailureAdmission::Retain)
        .unwrap();
    second.close().unwrap();
}
#[test]
fn failed_batch_merges_before_newer_changes_and_retry_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime::new().unwrap();
    let owner = owner(&runtime, root.path().join("failure.sqlite"));
    let client = owner.client();
    let control = Arc::new(Control::default());
    control.fail.store(true, Ordering::Release);
    client
        .record(Batch::put(1, 10, &control), FailureAdmission::Retain)
        .unwrap();
    assert!(client.flush().is_err());
    let called = AtomicBool::new(false);
    assert!(
        client
            .accept(FailureAdmission::Reject, || {
                called.store(true, Ordering::Release);
                Ok::<_, WritebackError>(((), Batch::put(1, 999, &control)))
            })
            .is_err()
    );
    assert!(!called.load(Ordering::Acquire));
    client
        .record(Batch::put(1, 20, &control), FailureAdmission::Retain)
        .unwrap();
    client
        .record(Batch::put(2, 30, &control), FailureAdmission::Retain)
        .unwrap();
    assert!(client.failure().is_some());
    client.flush().unwrap();
    assert!(client.failure().is_none());
    assert_eq!(value(&client, 1), 20);
    assert_eq!(value(&client, 2), 30);
    let retained = client.clone();
    thread::spawn(move || owner.close().unwrap())
        .join()
        .unwrap();
    assert!(retained.query(|_| Ok(())).is_err());
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
            accepted: 0,
            committed: 0,
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
    let task = tokio::spawn(run(connection, receiver, shared, runtime.handle(), policy));
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
    client
        .record(Batch::put(1, 10, &control), FailureAdmission::Retain)
        .unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(2500)).await;
    client
        .record(Batch::put(1, 20, &control), FailureAdmission::Retain)
        .unwrap();
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
        client
            .record(Batch::put(1, value, &control), FailureAdmission::Retain)
            .unwrap();
    }
    assert_eq!(client.shared.pending.lock().batch.values.len(), 1);
    done_rx.await.unwrap();
    assert_eq!(tokio::time::Instant::now(), start);
    client.shared.pending.lock().closed = true;
    client.sender.send(Message::Close).unwrap();
    task.await.unwrap().unwrap();
}
