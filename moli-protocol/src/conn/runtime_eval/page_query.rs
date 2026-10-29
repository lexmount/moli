use anyhow::{Context as _, Result};
use moli_core::page::{CompletedPageCommand, Page, PendingPageCommand};

use super::{CdpConnection, CommandOwnerScope, RuntimePageScope};
use crate::conn::ProtocolWorkflowDriver;

/// One admitted Native query. Both its inputs and its original Page authority
/// leave the state turn before waiting; completion cannot use a successor Page.
pub(crate) struct PreparedRuntimePageQuery<T> {
    scope: RuntimePageScope,
    pending: Option<PendingPageCommand>,
    operation: &'static str,
    finish: Box<dyn FnOnce(&mut Page, CompletedPageCommand) -> Result<T>>,
}

impl<T: 'static> PreparedRuntimePageQuery<T> {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        operation: &'static str,
        start: impl FnOnce(&mut Page) -> Result<PendingPageCommand>,
        finish: impl FnOnce(&mut Page, CompletedPageCommand) -> Result<T> + 'static,
    ) -> Result<Self> {
        let scope = RuntimePageScope::capture(conn, owner)?;
        let pending = start(conn.loaded_page_mut_for_protocol_access_for_owner(scope.owner())?)
            .context(operation)?;
        Ok(Self {
            scope,
            pending: Some(pending),
            operation,
            finish: Box::new(finish),
        })
    }

    pub(crate) fn prepare_resident(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        operation: &'static str,
        start: impl FnOnce(&mut Page) -> Result<PendingPageCommand>,
        finish: impl FnOnce(&mut Page, CompletedPageCommand) -> Result<T> + 'static,
    ) -> Result<Self> {
        let scope = RuntimePageScope::capture(conn, owner)?;
        let pending = start(scope.page_mut(conn)?).context(operation)?;
        Ok(Self {
            scope,
            pending: Some(pending),
            operation,
            finish: Box::new(finish),
        })
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<T> {
        let pending = self.pending.take().expect("one query owns one Native wait");
        let completed = pending.wait().await.context(self.operation)?;
        if let Some(fence) = completed.renderer_output_predecessor() {
            fence.merge_into_same_stream_tail(predecessor);
        }
        driver.with_state(move |state| state.finish_runtime_page_query(self, completed))?
    }

    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        completed: CompletedPageCommand,
    ) -> Result<T> {
        (self.finish)(self.scope.page_mut(conn)?, completed).context(self.operation)
    }
}

/// Runtime's node operations reuse the DOM domain's existing finite native
/// continuations, with the original Page authority across every wait.
pub(crate) struct PreparedRuntimeDomQuery {
    scope: RuntimePageScope,
    pending: Option<crate::domains::dom::PendingDomCommandDispatch>,
}

impl PreparedRuntimeDomQuery {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        command: crate::automation::AutomationCommand,
    ) -> Result<Self, crate::automation::DevToolsError> {
        use crate::automation::{DevToolsError, DevToolsErrorKind};
        let scope = RuntimePageScope::capture(conn, &owner)
            .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))?;
        let pending = crate::domains::dom::start_runtime_dom_query(conn, scope.owner(), command)?
            .ok_or_else(|| {
            DevToolsError::new(DevToolsErrorKind::Internal, MissingRuntimeDomCommand)
        })?;
        Ok(Self {
            scope,
            pending: Some(pending),
        })
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<crate::automation::AutomationResult, crate::automation::DevToolsError> {
        use crate::automation::{DevToolsError, DevToolsErrorKind};
        loop {
            let completed = Box::pin(
                self.pending
                    .take()
                    .expect("one DOM query owns its Native continuation")
                    .wait(),
            )
            .await;
            if let Some(fence) = completed.renderer_output_predecessor() {
                fence.merge_into_same_stream_tail(predecessor);
            }
            let (query, step) = driver
                .with_state(move |state| state.finish_runtime_dom_query(self, completed))
                .map_err(|error| DevToolsError::new(DevToolsErrorKind::Internal, error))??;
            self = query;
            match step {
                crate::domains::dom::DevToolsDomCommandTaskStep::Pending(pending) => {
                    self.pending = Some(*pending)
                }
                crate::domains::dom::DevToolsDomCommandTaskStep::Complete(result) => {
                    return *result;
                }
            }
        }
    }

    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        completed: crate::domains::dom::CompletedDomCommandDispatch,
    ) -> Result<
        (Self, crate::domains::dom::DevToolsDomCommandTaskStep),
        crate::automation::DevToolsError,
    > {
        self.scope.page_mut(conn).map_err(|error| {
            crate::automation::DevToolsError::new(
                crate::automation::DevToolsErrorKind::Internal,
                error,
            )
        })?;
        let step = crate::domains::dom::complete_pending_dom_command_result(conn, completed);
        Ok((self, step))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("MissingDomCommand")]
struct MissingRuntimeDomCommand;
