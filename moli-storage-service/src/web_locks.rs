//! Bucket-scoped Web Locks coordination. Endpoints enqueue owned native events;
//! neither the coordinator nor its queues contain renderer or JavaScript state.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    sync::Arc,
};

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::StorageBucketLocator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebLockMode {
    Exclusive,
    Shared,
}

/// Invalid combinations such as shared+steal cannot reach the coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WebLockRequestKind {
    Wait(WebLockMode),
    IfAvailable(WebLockMode),
    Steal,
}

impl WebLockRequestKind {
    fn mode(self) -> WebLockMode {
        match self {
            Self::Wait(mode) | Self::IfAvailable(mode) => mode,
            Self::Steal => WebLockMode::Exclusive,
        }
    }
}

/// Never reused within a partition, including after a client is destroyed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WebLockRequestId(u64);

impl WebLockRequestId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebLockInfo {
    // DOMString names are sequences of code units, not Unicode scalar values.
    pub name: Vec<u16>,
    pub mode: WebLockMode,
    pub client_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WebLockSnapshot {
    pub held: Vec<WebLockInfo>,
    pub pending: Vec<WebLockInfo>,
}

#[derive(Debug)]
pub enum WebLockEvent {
    Granted(WebLockRequestId),
    Unavailable(WebLockRequestId),
    Stolen(WebLockRequestId),
    Released(WebLockRequestId),
    Snapshot(WebLockRequestId, WebLockSnapshot),
}

impl WebLockEvent {
    pub fn request_id(&self) -> WebLockRequestId {
        match self {
            Self::Granted(id)
            | Self::Unavailable(id)
            | Self::Stolen(id)
            | Self::Released(id)
            | Self::Snapshot(id, _) => *id,
        }
    }
}

/// A thread-safe native task route. Implementations must only enqueue the event
/// on their owner's event loop, without running JavaScript or blocking.
pub trait WebLockEventSink: Send + Sync + 'static {
    fn send(&self, event: WebLockEvent);
}

impl<F: Fn(WebLockEvent) + Send + Sync + 'static> WebLockEventSink for F {
    fn send(&self, event: WebLockEvent) {
        self(event);
    }
}

#[derive(Clone, Default)]
pub struct WebLocks(Arc<Mutex<State>>);

impl fmt::Debug for WebLocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.0.lock();
        f.debug_struct("WebLocks")
            .field("clients", &state.clients.len())
            .field("buckets", &state.buckets.len())
            .finish()
    }
}

#[derive(Default)]
struct State {
    next_id: u64,
    clients: HashMap<u64, Client>,
    buckets: HashMap<StorageBucketLocator, Bucket>,
    outbox: VecDeque<(Arc<dyn WebLockEventSink>, WebLockEvent)>,
    delivering: bool,
}

struct Client {
    bucket: StorageBucketLocator,
    exposed_id: String,
    sink: Arc<dyn WebLockEventSink>,
}

#[derive(Default)]
struct Bucket {
    held: Vec<Record>,
    queues: IndexMap<Vec<u16>, VecDeque<Record>>,
}

struct Record {
    id: WebLockRequestId,
    client: u64,
    name: Vec<u16>,
    mode: WebLockMode,
}

impl State {
    fn allocate(&mut self) -> u64 {
        self.next_id = self.next_id.checked_add(1).expect("Web Locks id overflow");
        self.next_id
    }

    fn event(&mut self, client: u64, event: WebLockEvent) {
        if let Some(client) = self.clients.get(&client) {
            self.outbox.push_back((client.sink.clone(), event));
        }
    }

    fn process_queue(&mut self, bucket: &mut Bucket, name: &[u16]) {
        let Some(queue) = bucket.queues.get_mut(name) else {
            return;
        };
        while let Some(request) = queue.front() {
            let blocked = bucket.held.iter().any(|held| {
                held.name == name
                    && (held.mode == WebLockMode::Exclusive
                        || request.mode == WebLockMode::Exclusive)
            });
            if blocked {
                break;
            }
            let request = queue.pop_front().expect("queue has a front");
            self.event(request.client, WebLockEvent::Granted(request.id));
            bucket.held.push(request);
        }
        if queue.is_empty() {
            bucket.queues.shift_remove(name);
        }
    }

    fn info(&self, record: &Record) -> WebLockInfo {
        WebLockInfo {
            name: record.name.clone(),
            mode: record.mode,
            client_id: self.clients[&record.client].exposed_id.clone(),
        }
    }
}

impl WebLocks {
    pub fn connect(
        &self,
        bucket: StorageBucketLocator,
        exposed_client_id: String,
        sink: Arc<dyn WebLockEventSink>,
    ) -> WebLockClient {
        let mut state = self.0.lock();
        let id = state.allocate();
        state.clients.insert(
            id,
            Client {
                bucket,
                exposed_id: exposed_client_id,
                sink,
            },
        );
        WebLockClient(Arc::new(ClientLease {
            coordinator: self.clone(),
            id,
        }))
    }

    fn mutate(&self, operation: impl FnOnce(&mut State)) {
        {
            let mut state = self.0.lock();
            operation(&mut state);
            if state.delivering {
                return;
            }
            state.delivering = true;
        }
        // Serialize delivery across threads without calling endpoints under the
        // coordinator mutex. Reentrant submissions append after earlier events.
        let mut drain = DeliveryGuard {
            coordinator: self,
            complete: false,
        };
        loop {
            let next = {
                let mut state = self.0.lock();
                let next = state.outbox.pop_front();
                if next.is_none() {
                    state.delivering = false;
                    drain.complete = true;
                }
                next
            };
            let Some((sink, event)) = next else {
                break;
            };
            sink.send(event);
        }
    }

    fn close_client(&self, client_id: u64) {
        self.mutate(|state| {
            let Some(client) = state.clients.remove(&client_id) else {
                return;
            };
            let Some(mut bucket) = state.buckets.remove(&client.bucket) else {
                return;
            };
            let mut affected = Vec::new();
            bucket.held.retain(|held| {
                if held.client == client_id {
                    affected.push(held.name.clone());
                    false
                } else {
                    true
                }
            });
            for (name, queue) in &mut bucket.queues {
                if queue.iter().any(|request| request.client == client_id) {
                    queue.retain(|request| request.client != client_id);
                    affected.push(name.clone());
                }
            }
            for name in affected {
                state.process_queue(&mut bucket, &name);
            }
            if !bucket.held.is_empty() || !bucket.queues.is_empty() {
                state.buckets.insert(client.bucket, bucket);
            }
        });
    }
}

struct DeliveryGuard<'a> {
    coordinator: &'a WebLocks,
    complete: bool,
}

impl Drop for DeliveryGuard<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.coordinator.0.lock().delivering = false;
        }
    }
}

struct ClientLease {
    coordinator: WebLocks,
    id: u64,
}

impl Drop for ClientLease {
    fn drop(&mut self) {
        self.coordinator.close_client(self.id);
    }
}

/// A client lifetime, shared by its Window/Worker's managers and requests.
/// Dropping its final owner aborts requests and releases every held lock.
#[derive(Clone)]
pub struct WebLockClient(Arc<ClientLease>);

impl WebLockClient {
    pub fn prepare_request(&self, name: Vec<u16>, kind: WebLockRequestKind) -> WebLockRequest {
        let id = WebLockRequestId(self.0.coordinator.0.lock().allocate());
        WebLockRequest {
            client: self.clone(),
            record: Record {
                id,
                client: self.0.id,
                name,
                mode: kind.mode(),
            },
            kind,
        }
    }

    pub fn prepare_query(&self) -> WebLockQuery {
        WebLockQuery {
            client: self.clone(),
            id: WebLockRequestId(self.0.coordinator.0.lock().allocate()),
        }
    }

    /// Release precisely this client's lease. A late callback from a stolen
    /// lock cannot release its replacement or settle the stolen request.
    pub fn release(&self, id: WebLockRequestId) {
        self.remove(id, true);
    }

    /// Cancel a request that has not entered its callback, including a grant
    /// whose callback task is queued. The renderer preserves signal.reason.
    pub fn cancel(&self, id: WebLockRequestId) {
        self.remove(id, false);
    }

    fn remove(&self, id: WebLockRequestId, released: bool) {
        self.0.coordinator.mutate(|state| {
            let Some(client) = state.clients.get(&self.0.id) else {
                return;
            };
            let locator = client.bucket.clone();
            let Some(mut bucket) = state.buckets.remove(&locator) else {
                return;
            };
            let mut affected = None;
            if let Some(index) = bucket
                .held
                .iter()
                .position(|held| held.id == id && held.client == self.0.id)
            {
                affected = Some(bucket.held.remove(index).name);
                if released {
                    state.event(self.0.id, WebLockEvent::Released(id));
                }
            } else if !released {
                for (name, queue) in &mut bucket.queues {
                    if let Some(index) = queue
                        .iter()
                        .position(|request| request.id == id && request.client == self.0.id)
                    {
                        queue.remove(index);
                        affected = Some(name.clone());
                        break;
                    }
                }
            }
            if let Some(name) = affected {
                state.process_queue(&mut bucket, &name);
            }
            if !bucket.held.is_empty() || !bucket.queues.is_empty() {
                state.buckets.insert(locator, bucket);
            }
        });
    }
}

/// Reserve before storing renderer state; submit only after that state exists.
/// This single-use value prevents duplicate submission or reuse of an old ID.
pub struct WebLockRequest {
    client: WebLockClient,
    record: Record,
    kind: WebLockRequestKind,
}

impl WebLockRequest {
    pub fn id(&self) -> WebLockRequestId {
        self.record.id
    }

    pub fn submit(self) {
        self.client.0.coordinator.mutate(|state| {
            let locator = state.clients[&self.client.0.id].bucket.clone();
            let mut bucket = state.buckets.remove(&locator).unwrap_or_default();
            let name = self.record.name.clone();
            let queue = bucket.queues.entry(name.clone()).or_default();
            match self.kind {
                WebLockRequestKind::Steal => {
                    bucket.held.retain(|held| {
                        if held.name == name {
                            state.event(held.client, WebLockEvent::Stolen(held.id));
                            false
                        } else {
                            true
                        }
                    });
                    queue.push_front(self.record);
                }
                WebLockRequestKind::IfAvailable(mode) => {
                    let available = queue.is_empty()
                        && !bucket.held.iter().any(|held| {
                            held.name == name
                                && (mode == WebLockMode::Exclusive
                                    || held.mode == WebLockMode::Exclusive)
                        });
                    if available {
                        queue.push_back(self.record);
                    } else {
                        state.event(
                            self.record.client,
                            WebLockEvent::Unavailable(self.record.id),
                        );
                    }
                }
                WebLockRequestKind::Wait(_) => queue.push_back(self.record),
            }
            state.process_queue(&mut bucket, &name);
            if !bucket.held.is_empty() || !bucket.queues.is_empty() {
                state.buckets.insert(locator, bucket);
            }
        });
    }
}

pub struct WebLockQuery {
    client: WebLockClient,
    id: WebLockRequestId,
}

impl WebLockQuery {
    pub fn id(&self) -> WebLockRequestId {
        self.id
    }

    pub fn submit(self) {
        self.client.0.coordinator.mutate(|state| {
            let locator = &state.clients[&self.client.0.id].bucket;
            let snapshot = state
                .buckets
                .get(locator)
                .map(|bucket| WebLockSnapshot {
                    held: bucket
                        .held
                        .iter()
                        .map(|record| state.info(record))
                        .collect(),
                    pending: bucket
                        .queues
                        .values()
                        .flatten()
                        .map(|record| state.info(record))
                        .collect(),
                })
                .unwrap_or_default();
            state.event(self.client.0.id, WebLockEvent::Snapshot(self.id, snapshot));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};

    fn bucket(name: &str) -> StorageBucketLocator {
        StorageBucketLocator::Default {
            storage_key: name.to_owned(),
        }
    }

    fn client(
        locks: &WebLocks,
        bucket: StorageBucketLocator,
        name: &str,
    ) -> (WebLockClient, Receiver<WebLockEvent>) {
        let (tx, rx) = mpsc::channel();
        let client = locks.connect(
            bucket,
            name.to_owned(),
            Arc::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        (client, rx)
    }

    fn request(client: &WebLockClient, name: &[u16], kind: WebLockRequestKind) -> WebLockRequestId {
        let request = client.prepare_request(name.to_vec(), kind);
        let id = request.id();
        request.submit();
        id
    }

    fn granted(rx: &Receiver<WebLockEvent>, expected: WebLockRequestId) {
        assert!(matches!(rx.try_recv().unwrap(), WebLockEvent::Granted(id) if id == expected));
    }

    fn released(rx: &Receiver<WebLockEvent>, expected: WebLockRequestId) {
        assert!(matches!(rx.try_recv().unwrap(), WebLockEvent::Released(id) if id == expected));
    }

    fn snapshot(client: &WebLockClient, rx: &Receiver<WebLockEvent>) -> WebLockSnapshot {
        let query = client.prepare_query();
        let id = query.id();
        query.submit();
        let WebLockEvent::Snapshot(actual, snapshot) = rx.try_recv().unwrap() else {
            panic!("snapshot event");
        };
        assert_eq!(actual, id);
        snapshot
    }

    #[test]
    fn shared_locks_do_not_overtake_a_queued_writer() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let name = [42];
        let a = request(
            &client,
            &name,
            WebLockRequestKind::Wait(WebLockMode::Shared),
        );
        let b = request(
            &client,
            &name,
            WebLockRequestKind::Wait(WebLockMode::Shared),
        );
        granted(&rx, a);
        granted(&rx, b);
        let writer = request(
            &client,
            &name,
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        let reader = request(
            &client,
            &name,
            WebLockRequestKind::Wait(WebLockMode::Shared),
        );
        assert!(rx.try_recv().is_err());
        client.release(a);
        released(&rx, a);
        assert!(rx.try_recv().is_err());
        client.release(b);
        released(&rx, b);
        granted(&rx, writer);
        client.release(writer);
        released(&rx, writer);
        granted(&rx, reader);
        client.release(reader);
        released(&rx, reader);
        assert_eq!(snapshot(&client, &rx), WebLockSnapshot::default());
    }

    #[test]
    fn if_available_is_not_queued_and_cannot_jump_a_waiting_writer() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let a = request(&client, &[], WebLockRequestKind::Wait(WebLockMode::Shared));
        granted(&rx, a);
        let writer = request(
            &client,
            &[],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        let unavailable = request(
            &client,
            &[],
            WebLockRequestKind::IfAvailable(WebLockMode::Shared),
        );
        assert!(
            matches!(rx.try_recv().unwrap(), WebLockEvent::Unavailable(id) if id == unavailable)
        );
        let state = snapshot(&client, &rx);
        assert_eq!(state.held.len(), 1);
        assert_eq!(state.pending.len(), 1);
        client.release(a);
        released(&rx, a);
        granted(&rx, writer);
    }

    #[test]
    fn stealing_rejects_holders_and_precedes_existing_waiters() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let a = request(&client, &[1], WebLockRequestKind::Wait(WebLockMode::Shared));
        let b = request(&client, &[1], WebLockRequestKind::Wait(WebLockMode::Shared));
        granted(&rx, a);
        granted(&rx, b);
        let queued = request(
            &client,
            &[1],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        let stolen = request(&client, &[1], WebLockRequestKind::Steal);
        assert!(matches!(rx.try_recv().unwrap(), WebLockEvent::Stolen(id) if id == a));
        assert!(matches!(rx.try_recv().unwrap(), WebLockEvent::Stolen(id) if id == b));
        granted(&rx, stolen);
        client.release(a);
        client.release(b);
        assert!(rx.try_recv().is_err());
        client.release(stolen);
        released(&rx, stolen);
        granted(&rx, queued);
    }

    #[test]
    fn canceling_a_queued_writer_immediately_grants_the_next_shared_group() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let a = request(&client, &[1], WebLockRequestKind::Wait(WebLockMode::Shared));
        granted(&rx, a);
        let writer = request(
            &client,
            &[1],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        let b = request(&client, &[1], WebLockRequestKind::Wait(WebLockMode::Shared));
        let c = request(&client, &[1], WebLockRequestKind::Wait(WebLockMode::Shared));
        client.cancel(writer);
        granted(&rx, b);
        granted(&rx, c);
        assert_eq!(snapshot(&client, &rx).held.len(), 3);
    }

    #[test]
    fn canceling_a_queued_callback_releases_the_granted_lease() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let a = request(
            &client,
            &[1],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        let b = request(
            &client,
            &[1],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        client.cancel(a);
        // The renderer can already have the first grant queued. Its abort
        // algorithm discards that callback; coordination grants the successor.
        granted(&rx, a);
        granted(&rx, b);
        client.release(a);
        assert!(rx.try_recv().is_err());
        assert_eq!(snapshot(&client, &rx).held.len(), 1);
    }

    #[test]
    fn client_teardown_releases_holders_and_removes_its_queued_requests() {
        let locks = WebLocks::default();
        let (a, arx) = client(&locks, bucket("origin"), "a");
        let (b, brx) = client(&locks, bucket("origin"), "b");
        let first = request(&a, &[1], WebLockRequestKind::Wait(WebLockMode::Exclusive));
        granted(&arx, first);
        request(&a, &[1], WebLockRequestKind::Wait(WebLockMode::Exclusive));
        let next = request(&b, &[1], WebLockRequestKind::Wait(WebLockMode::Exclusive));
        drop(a);
        granted(&brx, next);
        let state = snapshot(&b, &brx);
        assert!(state.pending.is_empty());
        assert_eq!(state.held[0].client_id, "b");
        drop(b);
        assert!(locks.0.lock().buckets.is_empty());
        assert!(locks.0.lock().clients.is_empty());
    }

    #[test]
    fn partition_and_named_bucket_namespaces_are_independent() {
        let partitions = [
            crate::StorageService::in_memory(),
            crate::StorageService::in_memory(),
        ];
        let namespaces = [
            bucket("a"),
            bucket("b"),
            StorageBucketLocator::Named {
                storage_key: "a".to_owned(),
                bucket_id: crate::StorageBucketId::new(1).unwrap(),
            },
        ];
        let mut keep_alive = Vec::new();
        for partition in &partitions {
            for namespace in &namespaces {
                let (client, rx) = client(partition.web_locks(), namespace.clone(), "client");
                let id = request(
                    &client,
                    &[1],
                    WebLockRequestKind::Wait(WebLockMode::Exclusive),
                );
                granted(&rx, id);
                keep_alive.push(client);
            }
        }
    }

    #[test]
    fn names_preserve_null_and_unpaired_utf16_code_units() {
        let locks = WebLocks::default();
        let (client, rx) = client(&locks, bucket("origin"), "client");
        let names = [
            vec![],
            vec![0],
            vec![0xd800],
            vec![0xdc00],
            vec![0xfffd],
            vec![0xffff],
        ];
        for name in &names {
            let id = request(
                &client,
                name,
                WebLockRequestKind::Wait(WebLockMode::Exclusive),
            );
            granted(&rx, id);
        }
        assert_eq!(
            snapshot(&client, &rx)
                .held
                .into_iter()
                .map(|info| info.name)
                .collect::<Vec<_>>(),
            names
        );
    }

    #[test]
    fn sinks_can_reenter_the_coordinator_without_deadlocking() {
        let locks = WebLocks::default();
        let nested = locks.clone();
        let (tx, rx) = mpsc::channel();
        let owner = locks.connect(
            bucket("origin"),
            "owner".to_owned(),
            Arc::new(move |event| {
                let temporary =
                    nested.connect(bucket("other"), "nested".to_owned(), Arc::new(|_| {}));
                drop(temporary);
                tx.send(event).unwrap();
            }),
        );
        let id = request(
            &owner,
            &[1],
            WebLockRequestKind::Wait(WebLockMode::Exclusive),
        );
        granted(&rx, id);
        owner.release(id);
        released(&rx, id);
    }
}
