use super::{JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity};
use moli_indexeddb::{
    DatabaseHandle,
    sqlite::service::{DatabaseConnection, WeakDatabaseConnection},
};
use std::{cell::RefCell, collections::BTreeMap};

struct IndexedDbOpenConnection {
    execution_context: WindowExecutionContextIdentity,
    connection: WeakDatabaseConnection,
}

/// This index only routes owner retirement. Upgrade/delete notification and
/// transaction admission belong to the native SQLite connection registry.
#[derive(Default)]
pub(super) struct IndexedDbContextState {
    open_connections: RefCell<BTreeMap<DatabaseHandle, IndexedDbOpenConnection>>,
}

impl IndexedDbContextState {
    fn register_open_connection(
        &self,
        execution_context: WindowExecutionContextIdentity,
        connection: &DatabaseConnection,
    ) {
        let mut connections = self.open_connections.borrow_mut();
        connections.retain(|_, entry| entry.connection.is_alive());
        let previous = connections.insert(
            connection.handle(),
            IndexedDbOpenConnection {
                execution_context,
                connection: connection.downgrade(),
            },
        );
        assert!(
            previous.is_none(),
            "IndexedDB database handle registered more than once"
        );
    }

    fn retire_matching(
        &self,
        should_retire: impl Fn(WindowExecutionContextIdentity) -> bool,
    ) -> usize {
        let retired = self
            .open_connections
            .borrow_mut()
            .extract_if(.., |_, entry| should_retire(entry.execution_context))
            .map(|(_, entry)| entry.connection)
            .collect::<Vec<_>>();
        let count = retired.len();
        for connection in retired {
            if let Some(connection) = connection.upgrade() {
                // Retirement uses the connection's pre-reserved native control
                // task. No V8 handles or manager lock are needed during teardown.
                let _ = connection.retire();
            }
        }
        count
    }
}

impl JsContextHost {
    pub(crate) fn register_indexed_db_open_connection(
        &self,
        execution_context: WindowExecutionContextIdentity,
        connection: &DatabaseConnection,
    ) {
        self.indexed_db_context_tasks
            .register_open_connection(execution_context, connection);
    }

    pub(crate) fn unregister_indexed_db_open_connection(&self, handle: DatabaseHandle) {
        self.indexed_db_context_tasks
            .open_connections
            .borrow_mut()
            .remove(&handle);
    }

    pub(super) fn retire_indexed_db_context(
        &self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let retired = self
            .indexed_db_context_tasks
            .retire_matching(|owner| owner.realm_token() == context_token);
        self.signal_page_indexed_db_task_reconsideration_if_installed();
        retired
    }
}
