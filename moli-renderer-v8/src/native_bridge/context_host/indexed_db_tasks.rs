use super::{JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity};
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
    version: u64,
}

pub(crate) struct IndexedDbOpenConnectionSnapshot {
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) context: v8::Global<v8::Context>,
    pub(crate) database: v8::Global<v8::Object>,
}

#[derive(Default)]
pub(super) struct IndexedDbContextRetirement {
    pub(super) retired_connections: Vec<DatabaseHandle>,
    scheduled_drains: Vec<WindowExecutionContextIdentity>,
}

#[derive(Default)]
pub(super) struct IndexedDbContextState {
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
        version: u64,
        database: v8::Local<'_, v8::Object>,
    ) {
        let previous = self.open_connections.borrow_mut().insert(
            handle,
            IndexedDbOpenConnection {
                execution_context,
                context: v8::Global::new(scope, scope.get_current_context()),
                database: v8::Global::new(scope, database),
                database_key,
                version,
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

    fn open_connection_version(&self, database_key: &str) -> Option<u64> {
        self.open_connections
            .borrow()
            .values()
            .filter_map(|connection| {
                (connection.database_key == database_key).then_some(connection.version)
            })
            .max()
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
            scheduled_drains: scheduled_drains.into_iter().collect(),
        }
    }

    fn retire_context(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> IndexedDbContextRetirement {
        self.retire_matching(|candidate| candidate.realm_token() == context_token)
    }
}

impl JsContextHost {
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
        version: u64,
        database: v8::Local<'_, v8::Object>,
    ) {
        self.indexed_db_context_tasks.register_open_connection(
            scope,
            execution_context,
            handle,
            database_key,
            version,
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

    pub(crate) fn indexed_db_open_connection_version(&self, database_key: &str) -> Option<u64> {
        self.indexed_db_context_tasks
            .open_connection_version(database_key)
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn blocked_drain_is_coalesced_per_context() {
        let state = IndexedDbContextState::default();
        let context = identity(21);

        assert_eq!(state.reserve_blocked_drains([context]), vec![context]);
        assert!(state.reserve_blocked_drains([context]).is_empty());
        state.finish_blocked_drain(context);
        assert_eq!(state.reserve_blocked_drains([context]), vec![context]);
    }
}
