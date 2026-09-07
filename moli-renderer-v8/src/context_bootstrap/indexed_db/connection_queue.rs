//! One connection-admission queue for each storage-key/database-name pair.
//!
//! Like Chromium's ConnectionCoordinator, admission is unconditional: even an
//! open at the current version, or a delete with no live connection, waits for
//! earlier calls. Version checks happen at the head, never against an admission-
//! time snapshot. A running upgrade keeps its head until its open result is
//! dispatched; closing a connection alone cannot let the next request overtake.
//!
//! The Page shares this coordinator across its exact Window/realm owners.
//! Workers use the same implementation with a context-local coordinator. V8
//! callbacks and scheduler publication always happen outside the RefCell borrow.

use super::*;
use crate::native_bridge::WindowExecutionContextIdentity;
use std::{cell::RefCell, rc::Rc};

mod order;
use order::ConnectionQueue;
pub(crate) use order::ConnectionRequestId;

pub(crate) type SharedIndexedDbConnectionQueue = Rc<RefCell<IndexedDbConnectionQueue>>;

#[derive(Clone, Copy)]
pub(super) enum ConnectionOperation {
    Open(Option<u64>),
    Delete,
}

#[derive(Clone, Copy)]
enum ConnectionRequestState {
    NotStarted,
    WaitingForConnections {
        old_version: u64,
        new_version: Option<u64>,
    },
    WaitingForResult,
}

struct ConnectionRequest {
    request: v8::Global<v8::Object>,
    owner: IndexedDbExecutionOwner,
    storage_scope: IndexedDbStorageScope,
    name: String,
    operation: ConnectionOperation,
    state: ConnectionRequestState,
    manager: Option<WeakIndexedDbManager>,
    provisional_database: Option<DatabaseHandle>,
    upgrade_transaction: Option<TransactionHandle>,
}

impl Drop for ConnectionRequest {
    fn drop(&mut self) {
        // Realm retirement can cancel the head while its upgrade is pending.
        // Only un-delivered opens own these handles. Normal terminal delivery
        // disarms them before removal, transferring the connection to script.
        if self.provisional_database.is_none() && self.upgrade_transaction.is_none() {
            return;
        }
        let Some(manager) = self
            .manager
            .as_ref()
            .and_then(WeakIndexedDbManager::upgrade)
        else {
            return;
        };
        let mut manager = manager.lock();
        if let Some(transaction) = self.upgrade_transaction.take() {
            let _ = manager.abort_transaction(transaction);
        }
        if let Some(database) = self.provisional_database.take() {
            let _ = manager.close_database(database);
        }
    }
}

#[derive(Default)]
pub(crate) struct IndexedDbConnectionQueue {
    requests: ConnectionQueue<ConnectionRequest>,
    local_drain_scheduled: bool,
}

impl IndexedDbConnectionQueue {
    pub(crate) fn waiting_owner(&self, key: &str) -> Option<WindowExecutionContextIdentity> {
        let (_, request) = self.requests.head_for_key(key)?;
        matches!(
            request.state,
            ConnectionRequestState::WaitingForConnections { .. }
        )
        .then(|| request.owner.execution_context())
        .flatten()
    }

    pub(crate) fn retire_matching(
        &mut self,
        matches: impl Fn(WindowExecutionContextIdentity) -> bool,
    ) -> Vec<WindowExecutionContextIdentity> {
        self.requests
            .retire(|request| request.owner.execution_context().is_some_and(&matches))
            .into_iter()
            .filter_map(|id| self.requests.head(id)?.owner.execution_context())
            .collect()
    }
}

fn connection_queue(scope: &mut v8::PinScope<'_, '_>) -> SharedIndexedDbConnectionQueue {
    if let Some(host) = crate::util::context_host_from_global_bridge(scope) {
        // Clone just the independent owner before making any V8 call.
        return host.indexed_db_connection_queue();
    }
    let context = scope.get_current_context();
    if let Some(queue) = context.get_slot::<RefCell<IndexedDbConnectionQueue>>() {
        return queue;
    }
    let queue = Rc::new(RefCell::new(IndexedDbConnectionQueue::default()));
    let _ = context.set_slot(queue.clone());
    queue
}

pub(super) fn enqueue_connection_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    storage_scope: IndexedDbStorageScope,
    name: String,
    operation: ConnectionOperation,
) {
    let owner = indexed_db_typed_execution_owner(scope, request)
        .expect("connection request must retain its accepting owner");
    let key = database_registry_key(storage_scope.storage_key(), &name);
    let entry = ConnectionRequest {
        request: v8::Global::new(scope, request),
        owner,
        storage_scope,
        name,
        operation,
        state: ConnectionRequestState::NotStarted,
        manager: weak_indexed_db_manager_for_context(scope),
        provisional_database: None,
        upgrade_transaction: None,
    };
    let (id, is_head) = connection_queue(scope)
        .borrow_mut()
        .requests
        .push(key, entry);
    bind_indexed_db_connection_request(scope, request, id);
    if is_head {
        schedule_connection_drain(scope, owner);
    }
}

fn schedule_connection_drain(scope: &mut v8::PinScope<'_, '_>, owner: IndexedDbExecutionOwner) {
    if let Some(host) = crate::util::context_host_from_global_bridge(scope) {
        let owner = owner
            .execution_context()
            .expect("Page connection request has an exact owner");
        host.schedule_indexed_db_connection_drains([owner]);
        return;
    }
    let queue = connection_queue(scope);
    {
        let mut queue = queue.borrow_mut();
        if queue.local_drain_scheduled {
            return;
        }
        queue.local_drain_scheduled = true;
    }
    let task = v8::Object::new(scope);
    register_indexed_db_task(
        scope,
        task,
        IndexedDbTaskKind::DrainConnectionRequests,
        None,
    );
    enqueue_indexed_db_task(scope, task);
}

pub(super) fn wake_connection_requests(scope: &mut v8::PinScope<'_, '_>, key: &str) {
    let queue = connection_queue(scope);
    let owner = queue
        .borrow()
        .requests
        .head_for_key(key)
        .and_then(|(_, request)| {
            matches!(
                request.state,
                ConnectionRequestState::WaitingForConnections { .. }
            )
            .then_some(request.owner)
        });
    if let Some(owner) = owner {
        schedule_connection_drain(scope, owner);
    }
}

pub(super) fn retain_provisional_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    database: DatabaseHandle,
    transaction: Option<TransactionHandle>,
) {
    let Some(id) = indexed_db_connection_request_id(scope, request) else {
        return;
    };
    let queue = connection_queue(scope);
    if let Some(entry) = queue.borrow_mut().requests.head_mut(id) {
        entry.provisional_database = Some(database);
        entry.upgrade_transaction = transaction;
    }
}

pub(super) fn finish_connection_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) {
    let Some(id) = take_indexed_db_connection_request_id(scope, request) else {
        return;
    };
    let successful = object_hidden_value(scope, request, INDEXED_DB_REQUEST_ERROR_SLOT)
        .is_some_and(|error| error.is_null());
    let queue = connection_queue(scope);
    let (completed, next_owner) = {
        let mut queue = queue.borrow_mut();
        let Some((mut completed, next)) = queue.requests.finish(id) else {
            return; // A retired owner must not finish a replacement head.
        };
        if successful {
            completed.provisional_database = None;
            completed.upgrade_transaction = None;
        }
        let next_owner =
            next.and_then(|next| queue.requests.head(next).map(|request| request.owner));
        (completed, next_owner)
    };
    // An error before wrapper construction still owns backend handles. Drop
    // them before waking the next head, but outside the coordinator borrow.
    drop(completed);
    if let Some(owner) = next_owner {
        schedule_connection_drain(scope, owner);
    }
}

pub(crate) fn flush_indexed_db_connection_requests(
    scope: &mut v8::PinScope<'_, '_>,
    owner: Option<WindowExecutionContextIdentity>,
) {
    let queue = connection_queue(scope);
    let heads = {
        let mut queue = queue.borrow_mut();
        queue.local_drain_scheduled = false;
        queue
            .requests
            .heads()
            .filter_map(|(id, request)| {
                (request.owner.execution_context() == owner
                    && !matches!(request.state, ConnectionRequestState::WaitingForResult))
                .then_some(id)
            })
            .collect::<Vec<_>>()
    };
    for id in heads {
        advance_connection_request(scope, &queue, id);
    }
}

fn advance_connection_request(
    scope: &mut v8::PinScope<'_, '_>,
    queue: &SharedIndexedDbConnectionQueue,
    id: ConnectionRequestId,
) {
    let work = {
        let mut queue = queue.borrow_mut();
        let Some(entry) = queue.requests.head_mut(id) else {
            return;
        };
        let state = entry.state;
        if matches!(state, ConnectionRequestState::WaitingForResult) {
            return;
        }
        entry.state = ConnectionRequestState::WaitingForResult;
        ConnectionRequestWork {
            request: v8::Local::new(scope, &entry.request),
            owner: entry.owner,
            storage_scope: entry.storage_scope.clone(),
            name: entry.name.clone(),
            operation: entry.operation,
            state,
        }
    };
    let owner = work.owner;
    let previous_owner = owner.dispatch_scope().enter(scope);
    advance_connection_request_in_owner(scope, queue, id, work);
    owner.dispatch_scope().defer_restore(scope, previous_owner);
}

// A short-lived snapshot releases the queue borrow before any V8 callback can
// close a connection, enqueue another request, or retire this exact owner.
struct ConnectionRequestWork<'s> {
    request: v8::Local<'s, v8::Object>,
    owner: IndexedDbExecutionOwner,
    storage_scope: IndexedDbStorageScope,
    name: String,
    operation: ConnectionOperation,
    state: ConnectionRequestState,
}

fn advance_connection_request_in_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    queue: &SharedIndexedDbConnectionQueue,
    id: ConnectionRequestId,
    work: ConnectionRequestWork<'s>,
) {
    let ConnectionRequestWork {
        request,
        owner,
        storage_scope,
        name,
        operation,
        state,
    } = work;
    if let Err(error) = validate_storage_bucket_scope(scope, &storage_scope) {
        let error = request_error_object(scope, &error);
        store_request_error(scope, request, error);
        return;
    }
    let key = database_registry_key(storage_scope.storage_key(), &name);
    match state {
        ConnectionRequestState::NotStarted => {
            let version = match with_indexed_db_manager(scope, |manager| {
                manager.database_version(storage_scope.storage_key(), &name)
            }) {
                Ok(version) => version.unwrap_or(0),
                Err(error) => {
                    let error = request_error_object(scope, &error);
                    store_request_error(scope, request, error);
                    return;
                }
            };
            let new_version = match operation {
                ConnectionOperation::Open(requested) => Some(requested.unwrap_or(version.max(1))),
                ConnectionOperation::Delete => None,
            };
            let needs_exclusive_connection =
                new_version.is_none_or(|requested| requested > version);
            if needs_exclusive_connection && has_open_database_connections_for_key(scope, &key) {
                if let Some(entry) = queue.borrow_mut().requests.head_mut(id) {
                    entry.state = ConnectionRequestState::WaitingForConnections {
                        old_version: version,
                        new_version,
                    };
                }
                dispatch_version_change_to_open_connections(scope, &key, version, new_version);
                // Checking blocked in a subsequent task lets each dispatched
                // versionchange callback's microtasks close its connection.
                schedule_connection_drain(scope, owner);
                return;
            }
        }
        ConnectionRequestState::WaitingForConnections {
            old_version,
            new_version,
        } => {
            if has_open_database_connections_for_key(scope, &key) {
                if let Some(entry) = queue.borrow_mut().requests.head_mut(id) {
                    entry.state = state;
                }
                if !object_bool_property(scope, request, INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT)
                    .unwrap_or(false)
                {
                    set_indexed_db_slot_value(
                        scope,
                        request,
                        INDEXED_DB_REQUEST_BLOCKED_DISPATCHED_SLOT,
                        v8::Boolean::new(scope, true).into(),
                    );
                    let _ = dispatch_version_change_event(
                        scope,
                        request,
                        "blocked",
                        old_version,
                        new_version,
                    );
                }
                return;
            }
        }
        ConnectionRequestState::WaitingForResult => return,
    }
    match operation {
        ConnectionOperation::Open(version) => {
            execute_open_request(scope, request, storage_scope, name, version)
        }
        ConnectionOperation::Delete => {
            execute_delete_database_request(scope, request, storage_scope, name)
        }
    }
}
