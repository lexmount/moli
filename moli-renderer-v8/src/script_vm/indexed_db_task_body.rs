use anyhow::Result;

use super::ScriptVm;
use crate::{
    page_task_queue::{RendererPageIndexedDbTaskKind, RendererPageIndexedDbTaskOwner},
    runtime::AuthorizedCurrentPageIndexedDbTask,
};

/// Observable result of applying one authorized IndexedDB scheduler body.
///
/// This type deliberately stops at the V8/task body boundary. It does not
/// encode whether or when the surrounding HTML task performs its microtask
/// checkpoint; the selected Page-task dispatcher owns that decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IndexedDbTaskBodyEffect {
    /// The exact realm-local task was consumed. The body may have dispatched
    /// request, transaction, or blocked-open callbacks.
    Applied,
    /// The stable Page ticket was current, but its realm-local payload had
    /// already been removed. The selected ticket still represents an ordinary
    /// task boundary, but no callback body ran.
    CurrentOwnerHadNoPendingTask,
}

/// Result of retiring one stale IndexedDB scheduler ticket.
///
/// Stale cleanup never owns a task checkpoint. This result only records
/// whether an exact realm-local payload was removed, so callers do not have to
/// reinterpret a transport-level boolean as lifecycle policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IndexedDbStaleTaskCleanupEffect {
    RemovedRealmLocalPayload,
    RealmOrPayloadAlreadyAbsent,
}

impl ScriptVm {
    pub(crate) fn indexed_db_task_owner_is_current(
        &self,
        owner: RendererPageIndexedDbTaskOwner,
    ) -> bool {
        self._context_host
            .borrow()
            .window_execution_context_identity_is_current(owner.execution_context())
    }

    fn indexed_db_context_ptr(
        &self,
        owner: RendererPageIndexedDbTaskOwner,
    ) -> Option<*const v8::Global<v8::Context>> {
        self.page_runtime_observable_contexts()
            .into_iter()
            .find(|context| context.context_token == owner.execution_context().realm_token())
            .map(|context| context.context)
    }

    /// Executes one exact Page-side IndexedDB task after Page-root and Window
    /// realm authorization. The local task id is consumed by identity rather
    /// than by queue head so a stale alias can never steal another realm's
    /// task from a shared V8 context.
    pub(crate) fn apply_current_indexed_db_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageIndexedDbTask,
    ) -> Result<IndexedDbTaskBodyEffect> {
        let task = authorization.into_task();
        let owner = task.owner();
        let Some(context_ptr) = self.indexed_db_context_ptr(owner) else {
            return Ok(IndexedDbTaskBodyEffect::CurrentOwnerHadNoPendingTask);
        };
        let execution_context = owner.execution_context();
        let kind = task.kind();
        let applied = self.with_context_scope_by_ptr(context_ptr, move |scope, host_ptr| {
            let previous_dispatch_scope = execution_context.dispatch_scope().enter(scope);
            let applied = match kind {
                RendererPageIndexedDbTaskKind::RuntimeQueue(task_id) => {
                    crate::context_bootstrap::flush_indexed_db_task_by_id(scope, task_id)
                }
                RendererPageIndexedDbTaskKind::DrainConnectionRequests => {
                    unsafe { &*host_ptr }.finish_indexed_db_connection_drain(execution_context);
                    crate::context_bootstrap::flush_indexed_db_connection_requests(
                        scope,
                        Some(execution_context),
                    );
                    true
                }
            };
            execution_context
                .dispatch_scope()
                .defer_restore(scope, previous_dispatch_scope);
            Ok(applied)
        })?;
        Ok(if applied {
            IndexedDbTaskBodyEffect::Applied
        } else {
            IndexedDbTaskBodyEffect::CurrentOwnerHadNoPendingTask
        })
    }

    /// Removes same-Page stale local state without executing callbacks. A task
    /// from an older PageVm namespace must never use its reused local id to
    /// touch the replacement PageVm.
    pub(crate) fn discard_stale_indexed_db_task(
        &mut self,
        owner: RendererPageIndexedDbTaskOwner,
        kind: RendererPageIndexedDbTaskKind,
    ) -> Result<IndexedDbStaleTaskCleanupEffect> {
        let Some(context_ptr) = self.indexed_db_context_ptr(owner) else {
            return Ok(IndexedDbStaleTaskCleanupEffect::RealmOrPayloadAlreadyAbsent);
        };
        let execution_context = owner.execution_context();
        let removed = self.with_context_scope_by_ptr(context_ptr, move |scope, host_ptr| {
            Ok(match kind {
                RendererPageIndexedDbTaskKind::RuntimeQueue(task_id) => {
                    crate::context_bootstrap::discard_indexed_db_task_by_id(scope, task_id)
                }
                RendererPageIndexedDbTaskKind::DrainConnectionRequests => {
                    unsafe { &*host_ptr }.finish_indexed_db_connection_drain(execution_context);
                    true
                }
            })
        })?;
        Ok(if removed {
            IndexedDbStaleTaskCleanupEffect::RemovedRealmLocalPayload
        } else {
            IndexedDbStaleTaskCleanupEffect::RealmOrPayloadAlreadyAbsent
        })
    }
}
