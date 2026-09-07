use super::{
    JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};
use moli_indexeddb::DatabaseHandle;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
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
    scheduled_drains: Vec<WindowExecutionContextIdentity>,
}

#[derive(Default)]
pub(super) struct IndexedDbContextState {
    open_connections: RefCell<BTreeMap<DatabaseHandle, IndexedDbOpenConnection>>,
    connection_queue: crate::context_bootstrap::SharedIndexedDbConnectionQueue,
    pending_connection_drains: RefCell<HashSet<WindowExecutionContextIdentity>>,
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

    fn unregister_open_connection(
        &self,
        handle: DatabaseHandle,
    ) -> (bool, Vec<WindowExecutionContextIdentity>) {
        let Some(connection) = self.open_connections.borrow_mut().remove(&handle) else {
            return (false, Vec::new());
        };
        (
            true,
            self.connection_queue
                .borrow()
                .waiting_owner(&connection.database_key)
                .into_iter()
                .collect(),
        )
    }

    fn reserve_connection_drains(
        &self,
        execution_contexts: impl IntoIterator<Item = WindowExecutionContextIdentity>,
    ) -> Vec<WindowExecutionContextIdentity> {
        let mut pending = self.pending_connection_drains.borrow_mut();
        execution_contexts
            .into_iter()
            .filter(|execution_context| pending.insert(*execution_context))
            .collect()
    }

    fn finish_connection_drain(&self, execution_context: WindowExecutionContextIdentity) {
        self.pending_connection_drains
            .borrow_mut()
            .remove(&execution_context);
    }

    fn retire_matching(
        &self,
        should_retire: impl Fn(WindowExecutionContextIdentity) -> bool,
    ) -> IndexedDbContextRetirement {
        let next_owners = self
            .connection_queue
            .borrow_mut()
            .retire_matching(&should_retire);
        self.pending_connection_drains
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
        let mut scheduled_drains = next_owners.into_iter().collect::<HashSet<_>>();
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

    fn retire_owner(&self, owner: WindowExecutionContextOwner) -> IndexedDbContextRetirement {
        self.retire_matching(|candidate| candidate.owner() == owner)
    }
}

impl JsContextHost {
    pub(crate) fn schedule_indexed_db_connection_drains(
        &self,
        execution_contexts: impl IntoIterator<Item = WindowExecutionContextIdentity>,
    ) -> usize {
        let drains = self
            .indexed_db_context_tasks
            .reserve_connection_drains(execution_contexts);
        let mut scheduled = 0;
        for execution_context in drains {
            if self
                .page_indexed_db_task_sender()
                .send(
                    execution_context,
                    crate::page_task_queue::RendererPageIndexedDbTaskKind::DrainConnectionRequests,
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
                    .finish_connection_drain(execution_context);
                self.indexed_db_context_tasks
                    .connection_queue
                    .borrow_mut()
                    .retire_matching(|_| true);
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

    pub(crate) fn indexed_db_connection_queue(
        &self,
    ) -> crate::context_bootstrap::SharedIndexedDbConnectionQueue {
        self.indexed_db_context_tasks.connection_queue.clone()
    }

    pub(crate) fn unregister_indexed_db_open_connection(&self, handle: DatabaseHandle) -> bool {
        let (removed, drain_contexts) = self
            .indexed_db_context_tasks
            .unregister_open_connection(handle);
        self.schedule_indexed_db_connection_drains(drain_contexts);
        removed
    }

    pub(crate) fn finish_indexed_db_connection_drain(
        &self,
        execution_context: WindowExecutionContextIdentity,
    ) {
        self.indexed_db_context_tasks
            .finish_connection_drain(execution_context);
    }

    pub(super) fn retire_indexed_db_context(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> IndexedDbContextRetirement {
        let mut retirement = self.indexed_db_context_tasks.retire_context(context_token);
        let drain_contexts = std::mem::take(&mut retirement.scheduled_drains);
        self.schedule_indexed_db_connection_drains(drain_contexts);
        self.signal_page_indexed_db_task_reconsideration_if_installed();
        retirement
    }

    pub(super) fn retire_indexed_db_owner(
        &self,
        owner: WindowExecutionContextOwner,
    ) -> IndexedDbContextRetirement {
        let mut retirement = self.indexed_db_context_tasks.retire_owner(owner);
        let drain_contexts = std::mem::take(&mut retirement.scheduled_drains);
        self.schedule_indexed_db_connection_drains(drain_contexts);
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
    fn connection_drain_is_coalesced_per_context() {
        let state = IndexedDbContextState::default();
        let context = identity(21);

        assert_eq!(state.reserve_connection_drains([context]), vec![context]);
        assert!(state.reserve_connection_drains([context]).is_empty());
        state.finish_connection_drain(context);
        assert_eq!(state.reserve_connection_drains([context]), vec![context]);
    }

    #[test]
    fn popup_owner_retirement_preserves_opener_state_in_the_shared_realm() {
        let state = IndexedDbContextState::default();
        let opener = identity(31);
        let popup = popup_identity(7, 9, opener.realm_token().as_u64());
        assert_eq!(
            state.reserve_connection_drains([opener, popup]),
            vec![opener, popup]
        );

        let retirement = state.retire_owner(popup.owner());
        assert!(retirement.retired_connections.is_empty());
        assert!(state.reserve_connection_drains([opener]).is_empty());
        assert_eq!(state.reserve_connection_drains([popup]), vec![popup]);
    }
}
