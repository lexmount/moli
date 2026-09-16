use super::{
    JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};
use moli_indexeddb::DatabaseHandle;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
};

struct IndexedDbOpenConnection {
    execution_context: WindowExecutionContextIdentity,
    context: v8::Global<v8::Context>,
    database: v8::Global<v8::Object>,
    database_key: String,
}

pub(crate) struct IndexedDbOpenConnectionSnapshot {
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) database: v8::Global<v8::Object>,
}

#[derive(Default)]
pub(super) struct IndexedDbContextRetirement {
    pub(super) retired_connections: Vec<DatabaseHandle>,
    retired_requests: Vec<crate::context_bootstrap::ConnectionRequestHandle>,
    scheduled_drains: Vec<WindowExecutionContextIdentity>,
}

impl IndexedDbContextRetirement {
    pub(super) fn finish(self, manager: Option<&crate::WeakIndexedDbManager>) {
        if let Some(manager) = manager {
            let _ = manager.close_database_handles(self.retired_connections);
        }
        // A successor can run on another worker immediately after its wake.
        // Release queue positions only after closing the retired connections.
        for request in self.retired_requests {
            request.finish();
        }
    }
}

#[derive(Default)]
pub(super) struct IndexedDbContextState {
    connection_requests: RefCell<
        HashMap<
            WindowExecutionContextIdentity,
            Vec<crate::context_bootstrap::ConnectionRequestHandle>,
        >,
    >,
    open_connections: RefCell<BTreeMap<DatabaseHandle, IndexedDbOpenConnection>>,
    blocked_contexts: RefCell<HashMap<(String, WindowExecutionContextIdentity), usize>>,
    pending_blocked_drains: RefCell<HashSet<WindowExecutionContextIdentity>>,
}

impl IndexedDbContextState {
    fn register_open_connection(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        execution_context: WindowExecutionContextIdentity,
        handle: DatabaseHandle,
        database_key: String,
        database: v8::Local<'_, v8::Object>,
    ) {
        let previous = self.open_connections.borrow_mut().insert(
            handle,
            IndexedDbOpenConnection {
                execution_context,
                context: v8::Global::new(scope, scope.get_current_context()),
                database: v8::Global::new(scope, database),
                database_key,
            },
        );
        assert!(
            previous.is_none(),
            "IndexedDB database handle registered more than once"
        );
    }

    fn open_connection_snapshots(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        database_key: &str,
    ) -> Vec<IndexedDbOpenConnectionSnapshot> {
        self.open_connections
            .borrow()
            .values()
            .filter(|connection| connection.database_key == database_key)
            .map(|connection| IndexedDbOpenConnectionSnapshot {
                execution_context: connection.execution_context,
                context: v8::Global::new(scope, v8::Local::new(scope, &connection.context)),
                database: v8::Global::new(scope, v8::Local::new(scope, &connection.database)),
            })
            .collect()
    }

    fn register_blocked_context(
        &self,
        database_key: String,
        execution_context: WindowExecutionContextIdentity,
    ) {
        let mut blocked_contexts = self.blocked_contexts.borrow_mut();
        let count = blocked_contexts
            .entry((database_key, execution_context))
            .or_default();
        *count = count
            .checked_add(1)
            .expect("IndexedDB blocked request count overflow");
    }

    fn unregister_blocked_context(
        &self,
        database_key: &str,
        execution_context: WindowExecutionContextIdentity,
    ) {
        let key = (database_key.to_owned(), execution_context);
        let mut blocked_contexts = self.blocked_contexts.borrow_mut();
        let Some(count) = blocked_contexts.get_mut(&key) else {
            return;
        };
        assert!(*count > 0, "IndexedDB blocked request count underflow");
        *count -= 1;
        if *count == 0 {
            blocked_contexts.remove(&key);
        }
    }

    fn blocked_contexts_for_key(&self, database_key: &str) -> Vec<WindowExecutionContextIdentity> {
        self.blocked_contexts
            .borrow()
            .keys()
            .filter_map(|(candidate_key, execution_context)| {
                (candidate_key == database_key).then_some(*execution_context)
            })
            .collect()
    }

    fn unregister_open_connection(
        &self,
        handle: DatabaseHandle,
    ) -> (bool, Vec<WindowExecutionContextIdentity>) {
        let Some(connection) = self.open_connections.borrow_mut().remove(&handle) else {
            return (false, Vec::new());
        };
        (
            true,
            self.blocked_contexts_for_key(&connection.database_key),
        )
    }

    fn reserve_blocked_drains(
        &self,
        execution_contexts: impl IntoIterator<Item = WindowExecutionContextIdentity>,
    ) -> Vec<WindowExecutionContextIdentity> {
        let mut pending = self.pending_blocked_drains.borrow_mut();
        execution_contexts
            .into_iter()
            .filter(|execution_context| pending.insert(*execution_context))
            .collect()
    }

    fn finish_blocked_drain(&self, execution_context: WindowExecutionContextIdentity) {
        self.pending_blocked_drains
            .borrow_mut()
            .remove(&execution_context);
    }

    fn retire_matching(
        &self,
        should_retire: impl Fn(WindowExecutionContextIdentity) -> bool,
    ) -> IndexedDbContextRetirement {
        let mut retired_requests = Vec::new();
        self.connection_requests
            .borrow_mut()
            .retain(|owner, requests| {
                if !should_retire(*owner) {
                    return true;
                }
                retired_requests.append(requests);
                false
            });
        self.blocked_contexts
            .borrow_mut()
            .retain(|(_, candidate), _| !should_retire(*candidate));
        self.pending_blocked_drains
            .borrow_mut()
            .retain(|candidate| !should_retire(*candidate));

        let retired_connections = self
            .open_connections
            .borrow()
            .iter()
            .filter_map(|(handle, connection)| {
                should_retire(connection.execution_context).then_some(*handle)
            })
            .collect::<Vec<_>>();
        let mut scheduled_drains = HashSet::new();
        for handle in &retired_connections {
            let (_, drain_contexts) = self.unregister_open_connection(*handle);
            scheduled_drains.extend(drain_contexts);
        }

        IndexedDbContextRetirement {
            retired_connections,
            retired_requests,
            scheduled_drains: scheduled_drains.into_iter().collect(),
        }
    }

    fn retire_context(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> IndexedDbContextRetirement {
        self.retire_matching(|candidate| candidate.realm_token() == context_token)
    }

    fn retire_owner(&self, owner: WindowExecutionContextOwner) -> IndexedDbContextRetirement {
        self.retire_matching(|candidate| candidate.owner() == owner)
    }
}

impl JsContextHost {
    pub(crate) fn register_indexed_db_connection_request(
        &self,
        owner: WindowExecutionContextIdentity,
        request: crate::context_bootstrap::ConnectionRequestHandle,
    ) {
        self.indexed_db_context_tasks
            .connection_requests
            .borrow_mut()
            .entry(owner)
            .or_default()
            .push(request);
    }

    pub(crate) fn unregister_indexed_db_connection_request(
        &self,
        owner: WindowExecutionContextIdentity,
        id: u64,
    ) {
        let mut requests = self
            .indexed_db_context_tasks
            .connection_requests
            .borrow_mut();
        if let Some(owned) = requests.get_mut(&owner) {
            owned.retain(|request| request.id() != id);
            if owned.is_empty() {
                requests.remove(&owner);
            }
        }
    }

    fn schedule_indexed_db_blocked_drains(
        &self,
        execution_contexts: impl IntoIterator<Item = WindowExecutionContextIdentity>,
    ) -> usize {
        let drains = self
            .indexed_db_context_tasks
            .reserve_blocked_drains(execution_contexts);
        let mut scheduled = 0;
        for execution_context in drains {
            if self
                .page_indexed_db_task_sender()
                .send(
                    execution_context,
                    crate::page_task_queue::RendererPageIndexedDbTaskKind::DrainBlockedOpenRequests,
                )
                .is_ok()
            {
                scheduled += 1;
            } else {
                // The Page consumer has retired. Release the coalescing
                // reservation instead of retaining a drain that no scheduler
                // can ever execute; teardown must not fall back to legacy
                // dispatch.
                self.indexed_db_context_tasks
                    .finish_blocked_drain(execution_context);
            }
        }
        scheduled
    }

    pub(crate) fn register_indexed_db_open_connection(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        execution_context: WindowExecutionContextIdentity,
        handle: DatabaseHandle,
        database_key: String,
        database: v8::Local<'_, v8::Object>,
    ) {
        self.indexed_db_context_tasks.register_open_connection(
            scope,
            execution_context,
            handle,
            database_key,
            database,
        );
    }

    pub(crate) fn indexed_db_open_connection_snapshots(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        database_key: &str,
    ) -> Vec<IndexedDbOpenConnectionSnapshot> {
        self.indexed_db_context_tasks
            .open_connection_snapshots(scope, database_key)
    }

    pub(crate) fn unregister_indexed_db_open_connection(&self, handle: DatabaseHandle) -> bool {
        let (removed, drain_contexts) = self
            .indexed_db_context_tasks
            .unregister_open_connection(handle);
        self.schedule_indexed_db_blocked_drains(drain_contexts);
        removed
    }

    pub(crate) fn register_indexed_db_blocked_context(
        &self,
        database_key: String,
        execution_context: WindowExecutionContextIdentity,
    ) {
        self.indexed_db_context_tasks
            .register_blocked_context(database_key, execution_context);
    }

    pub(crate) fn unregister_indexed_db_blocked_context(
        &self,
        database_key: &str,
        execution_context: WindowExecutionContextIdentity,
    ) {
        self.indexed_db_context_tasks
            .unregister_blocked_context(database_key, execution_context);
    }

    pub(crate) fn finish_indexed_db_blocked_drain(
        &self,
        execution_context: WindowExecutionContextIdentity,
    ) {
        self.indexed_db_context_tasks
            .finish_blocked_drain(execution_context);
    }

    pub(super) fn retire_indexed_db_context(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> IndexedDbContextRetirement {
        let mut retirement = self.indexed_db_context_tasks.retire_context(context_token);
        let drain_contexts = std::mem::take(&mut retirement.scheduled_drains);
        self.schedule_indexed_db_blocked_drains(drain_contexts);
        self.signal_page_indexed_db_task_reconsideration_if_installed();
        retirement
    }

    pub(super) fn retire_indexed_db_owner(
        &self,
        owner: WindowExecutionContextOwner,
    ) -> IndexedDbContextRetirement {
        let mut retirement = self.indexed_db_context_tasks.retire_owner(owner);
        let drain_contexts = std::mem::take(&mut retirement.scheduled_drains);
        self.schedule_indexed_db_blocked_drains(drain_contexts);
        self.signal_page_indexed_db_task_reconsideration_if_installed();
        retirement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_db_retirement_closes_connections_before_waking_other_event_loops() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let manager = crate::new_indexed_db_manager(None).unwrap();
        let weak = crate::downgrade_indexed_db_manager(&manager);
        let opened = manager
            .lock()
            .open(moli_indexeddb::OpenOptions {
                origin: "origin".to_owned(),
                name: "db".to_owned(),
                version: None,
            })
            .unwrap();
        let queues = manager.lock().connection_request_queues();
        let first = queues.enqueue("origin", "db", Arc::new(|| {}));
        let woken = Arc::new(AtomicBool::new(false));
        let second = queues.enqueue("origin", "db", {
            let weak = weak.clone();
            let woken = woken.clone();
            Arc::new(move || {
                assert!(
                    weak.upgrade()
                        .unwrap()
                        .lock()
                        .close_database(opened.database)
                        .is_err(),
                    "the retired connection must be closed before a successor can execute"
                );
                woken.store(true, Ordering::SeqCst);
            })
        });
        IndexedDbContextRetirement {
            retired_connections: vec![opened.database],
            retired_requests: vec![first.handle().clone()],
            ..Default::default()
        }
        .finish(Some(&weak));
        assert!(woken.load(Ordering::SeqCst));
        assert!(second.handle().is_head());
    }

    fn identity(raw: u64) -> WindowExecutionContextIdentity {
        WindowExecutionContextIdentity::new(
            super::super::WindowExecutionContextOwner::Frame(
                crate::frame_owner_model::LocalWindowId(raw),
            ),
            super::super::OwnerDispatchScope::Top,
            RuntimeObservableContextToken::from_raw(raw),
            super::super::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        )
    }

    fn popup_identity(
        popup_id: u64,
        local_window_id: u64,
        shared_realm: u64,
    ) -> WindowExecutionContextIdentity {
        WindowExecutionContextIdentity::new(
            super::super::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: super::super::LightweightPopupLocalWindowId::new(local_window_id),
            },
            super::super::OwnerDispatchScope::LightweightPopup(popup_id),
            RuntimeObservableContextToken::from_raw(shared_realm),
            super::super::WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        )
    }

    #[test]
    fn blocked_drain_is_coalesced_per_context() {
        let state = IndexedDbContextState::default();
        let context = identity(21);

        assert_eq!(state.reserve_blocked_drains([context]), vec![context]);
        assert!(state.reserve_blocked_drains([context]).is_empty());
        state.finish_blocked_drain(context);
        assert_eq!(state.reserve_blocked_drains([context]), vec![context]);
    }

    #[test]
    fn popup_owner_retirement_preserves_opener_state_in_the_shared_realm() {
        let state = IndexedDbContextState::default();
        let opener = identity(31);
        let popup = popup_identity(7, 9, opener.realm_token().as_u64());
        state.register_blocked_context("shared".to_owned(), opener);
        state.register_blocked_context("shared".to_owned(), popup);
        assert_eq!(
            state.reserve_blocked_drains([opener, popup]),
            vec![opener, popup]
        );

        let retirement = state.retire_owner(popup.owner());
        assert!(retirement.retired_connections.is_empty());
        assert_eq!(state.blocked_contexts_for_key("shared"), vec![opener]);
        assert!(state.reserve_blocked_drains([opener]).is_empty());
        assert_eq!(state.reserve_blocked_drains([popup]), vec![popup]);
    }
}
