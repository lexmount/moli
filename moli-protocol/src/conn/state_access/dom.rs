use anyhow::Result;
use moli_core::page::{CompletedPageCommand, Page, PendingPageCommand};

use super::ProtocolWorkflowState;
use crate::conn::{PreparedRuntimeDomQuery, PreparedRuntimePageQuery, RuntimePageScope};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_dom_storage_child_query(
        self,
        operation: &crate::domains::dom_storage::PreparedDomStorageOperation,
    ) -> Result<PreparedRuntimePageQuery<Vec<moli_core::page::ChildFrameTreeSnapshot>>> {
        operation.prepare_child_query(self.connection)
    }

    pub(crate) fn finish_dom_storage_operation(
        self,
        operation: crate::domains::dom_storage::PreparedDomStorageOperation,
        storage_key: String,
    ) -> crate::domains::command_output::CommandOutputPlan {
        operation.finish(self.connection, &storage_key)
    }

    pub(crate) fn prepare_dom_page_query<T: 'static>(
        self,
        scope: &RuntimePageScope,
        start: impl FnOnce(&mut Page) -> Result<PendingPageCommand>,
        finish: impl FnOnce(&mut Page, CompletedPageCommand) -> Result<T> + 'static,
    ) -> Result<PreparedRuntimePageQuery<T>> {
        anyhow::ensure!(
            scope.can_start_operation(self.connection),
            crate::conn::NoDocumentLoaded
        );
        PreparedRuntimePageQuery::prepare(
            self.connection,
            scope.owner(),
            "DOM query failed",
            start,
            finish,
        )
    }

    pub(crate) fn prepare_scoped_dom_query(
        self,
        scope: &RuntimePageScope,
        command: crate::automation::AutomationCommand,
    ) -> Result<PreparedRuntimeDomQuery, crate::automation::DevToolsError> {
        if !scope.can_start_operation(self.connection) {
            return Err(crate::automation::DevToolsError::new(
                crate::automation::DevToolsErrorKind::Internal,
                crate::conn::NoDocumentLoaded,
            ));
        }
        PreparedRuntimeDomQuery::prepare(self.connection, scope.owner().clone(), command)
    }

    pub(crate) fn register_scoped_dom_object(
        self,
        scope: &RuntimePageScope,
        value: serde_json::Value,
        group: Option<String>,
    ) -> Result<()> {
        anyhow::ensure!(
            scope.can_start_operation(self.connection),
            crate::conn::NoDocumentLoaded
        );
        if let Some(group) = group {
            self.connection
                .register_runtime_remote_object_ids_from_value_for_owner_with_group(
                    scope.owner(),
                    &value,
                    &group,
                );
        } else {
            self.connection
                .register_runtime_remote_object_ids_from_value_for_owner(scope.owner(), &value);
        }
        Ok(())
    }
}
