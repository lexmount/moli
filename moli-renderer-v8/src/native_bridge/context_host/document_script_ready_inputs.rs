use super::JsContextHost;
use crate::{
    document_script_scheduler::{
        DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyDispatchOwnerMismatch,
        FrameDocumentClassicScriptSchedulerWork, FrameDocumentReadyActionRoute,
        FrameDocumentScriptSchedulerStore,
    },
    frame_owner_model::{
        FrameDocumentOwner, FrameDocumentRealmBoundScriptWork, FrameDocumentScriptReadyTaskWork,
        FrameDocumentScriptWorkAdmission, FrameDocumentTaskOwner, FrameDocumentUnboundScriptWork,
    },
    page_task_queue::{
        RendererPageChildDocumentScriptReadyTarget, RendererPageChildDocumentScriptReadyTaskId,
    },
};

#[derive(Debug)]
struct PendingChildDocumentScriptReadyTask {
    target: RendererPageChildDocumentScriptReadyTarget,
    work: FrameDocumentScriptReadyTaskWork,
}

/// PageVm-local payload residence for scheduler-visible child script tasks.
///
/// Ordering lives exclusively in the stable `ChildFrameTask` source. This
/// ledger only binds an opaque task id to the V8/DOM-bearing payload created by
/// the same PageVm, so replacement ids cannot authorize another generation's
/// work.
#[derive(Debug)]
pub(super) struct ChildDocumentScriptReadyTaskLedger {
    next_task_id: u64,
    pending: std::collections::HashMap<
        RendererPageChildDocumentScriptReadyTaskId,
        PendingChildDocumentScriptReadyTask,
    >,
}

impl Default for ChildDocumentScriptReadyTaskLedger {
    fn default() -> Self {
        Self {
            next_task_id: 1,
            pending: std::collections::HashMap::new(),
        }
    }
}

impl ChildDocumentScriptReadyTaskLedger {
    fn allocate_task_id(&mut self) -> RendererPageChildDocumentScriptReadyTaskId {
        let task_id = RendererPageChildDocumentScriptReadyTaskId::new(self.next_task_id);
        self.next_task_id = self
            .next_task_id
            .checked_add(1)
            .expect("child DocumentScriptReady task id overflow");
        task_id
    }

    fn insert(
        &mut self,
        target: RendererPageChildDocumentScriptReadyTarget,
        work: FrameDocumentScriptReadyTaskWork,
    ) {
        let replaced = self.pending.insert(
            target.task_id(),
            PendingChildDocumentScriptReadyTask { target, work },
        );
        debug_assert!(replaced.is_none(), "fresh child script id must be unique");
    }

    fn pending(
        &self,
        task_id: RendererPageChildDocumentScriptReadyTaskId,
    ) -> Option<&PendingChildDocumentScriptReadyTask> {
        self.pending.get(&task_id)
    }

    fn remove(
        &mut self,
        task_id: RendererPageChildDocumentScriptReadyTaskId,
    ) -> Option<PendingChildDocumentScriptReadyTask> {
        self.pending.remove(&task_id)
    }

    fn remove_exact(
        &mut self,
        target: RendererPageChildDocumentScriptReadyTarget,
    ) -> Option<FrameDocumentScriptReadyTaskWork> {
        if self.pending(target.task_id())?.target != target {
            return None;
        }
        self.remove(target.task_id()).map(|pending| pending.work)
    }

    fn remove_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Vec<FrameDocumentScriptReadyTaskWork> {
        let task_ids = self
            .pending
            .iter()
            .filter_map(|(task_id, pending)| {
                (pending.target.document_owner() == owner).then_some(*task_id)
            })
            .collect::<Vec<_>>();
        task_ids
            .into_iter()
            .filter_map(|task_id| self.remove(task_id).map(|pending| pending.work))
            .collect()
    }

    fn remove_child_handle(
        &mut self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Vec<FrameDocumentScriptReadyTaskWork> {
        let task_ids = self
            .pending
            .iter()
            .filter_map(|(task_id, pending)| {
                (pending.target.child_handle() == Some(child_handle)).then_some(*task_id)
            })
            .collect::<Vec<_>>();
        task_ids
            .into_iter()
            .filter_map(|task_id| self.remove(task_id).map(|pending| pending.work))
            .collect()
    }

    fn has_owner(&self, owner: FrameDocumentTaskOwner) -> bool {
        self.pending
            .values()
            .any(|pending| pending.target.document_owner() == owner)
    }
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn child_document_script_schedulers(&self) -> &FrameDocumentScriptSchedulerStore {
        &self.child_document_script_schedulers
    }

    pub(crate) fn child_document_script_schedulers_mut(
        &mut self,
    ) -> &mut FrameDocumentScriptSchedulerStore {
        &mut self.child_document_script_schedulers
    }

    fn child_document_script_ready_target_is_current(
        &self,
        target: RendererPageChildDocumentScriptReadyTarget,
    ) -> bool {
        if self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(target.document_owner())
            != Some(target.realm_id())
        {
            return false;
        }
        target.child_handle().is_none_or(|child_handle| {
            self.frame_owner_store
                .child_document_task_owner_is_current(child_handle, target.document_owner())
        })
    }

    fn queue_child_document_script_ready_task_for_realm(
        &mut self,
        work: FrameDocumentScriptReadyTaskWork,
        realm_id: crate::frame_owner_model::FrameRealmId,
    ) -> bool {
        let route = work.route();
        let owner = route.task_owner();
        if self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner)
            != Some(realm_id)
        {
            return false;
        }
        if route
            .optional_realm_id()
            .is_some_and(|expected| expected != realm_id)
            || route.child_handle().is_some_and(|child_handle| {
                !self
                    .frame_owner_store
                    .child_document_task_owner_is_current(child_handle, owner)
            })
        {
            return false;
        }

        let task_id = self.child_document_script_ready_tasks.allocate_task_id();
        let target = RendererPageChildDocumentScriptReadyTarget::new(
            route.child_handle(),
            owner,
            realm_id,
            task_id,
        );
        self.child_document_script_ready_tasks.insert(target, work);
        if self
            .page_child_frame_task_sender()
            .send_document_script_ready(target)
            .is_ok()
        {
            return true;
        }

        if let Some(work) = self.child_document_script_ready_tasks.remove_exact(target) {
            self.settle_child_document_script_ready_task_without_execution(work);
        }
        tracing::debug!(
            ?target,
            "retired child DocumentScriptReady payload after stable route closure"
        );
        false
    }

    fn queue_materialized_child_document_script_ready_task(
        &mut self,
        work: FrameDocumentScriptReadyTaskWork,
    ) -> bool {
        let owner = work.route().task_owner();
        let Some(realm_id) = self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(owner)
        else {
            return false;
        };
        self.queue_child_document_script_ready_task_for_realm(work, realm_id)
    }

    /// Move every currently-runnable scheduler action into the single stable
    /// child-frame source. Current owners without a materialized realm retain
    /// their work in the scheduler store; stale owners are consumed rather
    /// than blocking later Documents.
    pub(crate) fn admit_runnable_child_document_script_tasks(&mut self) -> usize {
        let mut admitted = 0;
        loop {
            let frame_owner_store = &self.frame_owner_store;
            let dispatch = self
                .child_document_script_schedulers
                .take_next_ready_dispatch_matching::<FrameDocumentReadyActionRoute>(
                    |document_owner| {
                        let Some(owner) = frame_owner_store
                            .current_document_task_owner_for_document_owner(document_owner)
                        else {
                            return true;
                        };
                        frame_owner_store
                            .current_materialized_realm_id_for_document_task_owner(owner)
                            .is_some()
                    },
                );
            let Some(dispatch) = dispatch else {
                break;
            };
            let dispatch = match dispatch {
                Ok(dispatch) => dispatch,
                Err(mismatch) => {
                    report_child_document_ready_owner_mismatch(mismatch);
                    continue;
                }
            };
            let route = *dispatch.route();
            if !self
                .frame_owner_store
                .frame_document_ready_route_task_is_current(&route)
            {
                tracing::debug!(
                    document_owner = ?route.document_owner(),
                    child_handle = ?route.child_handle(),
                    task_owner = ?route.task_owner(),
                    realm_id = ?route.optional_realm_id(),
                    "retired stale child DocumentScriptReady action during typed admission"
                );
                continue;
            }
            let (work, _) = dispatch.into_action_and_route();
            if !self.queue_materialized_child_document_script_ready_task(
                FrameDocumentScriptReadyTaskWork::Scheduler(work),
            ) {
                break;
            }
            admitted += 1;
        }
        admitted
    }

    pub(crate) fn current_pending_child_document_script_ready_target(
        &self,
        expected: RendererPageChildDocumentScriptReadyTarget,
    ) -> Option<RendererPageChildDocumentScriptReadyTarget> {
        let pending = self
            .child_document_script_ready_tasks
            .pending(expected.task_id())?;
        (pending.target == expected
            && self.child_document_script_ready_target_is_current(pending.target))
        .then_some(pending.target)
    }

    pub(crate) fn has_pending_child_document_script_ready_task_for_owner(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.child_document_script_ready_tasks.has_owner(owner)
    }

    pub(crate) fn take_pending_child_document_script_ready_task(
        &mut self,
        target: RendererPageChildDocumentScriptReadyTarget,
    ) -> Option<FrameDocumentScriptReadyTaskWork> {
        self.child_document_script_ready_tasks.remove_exact(target)
    }

    pub(crate) fn discard_pending_child_document_script_ready_task(
        &mut self,
        task_id: RendererPageChildDocumentScriptReadyTaskId,
    ) -> bool {
        let Some(pending) = self.child_document_script_ready_tasks.remove(task_id) else {
            return false;
        };
        self.settle_child_document_script_ready_task_without_execution(pending.work);
        true
    }

    pub(crate) fn retire_child_document_script_ready_tasks_for_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        let retired = self.child_document_script_ready_tasks.remove_owner(owner);
        let retired_count = retired.len();
        for work in retired {
            self.settle_child_document_script_ready_task_without_execution(work);
        }
        if retired_count != 0 {
            self.page_child_frame_task_sender().signal_reconsideration();
        }
        retired_count
    }

    pub(crate) fn retire_child_document_script_ready_tasks_for_handle(
        &mut self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> usize {
        let retired = self
            .child_document_script_ready_tasks
            .remove_child_handle(child_handle);
        let retired_count = retired.len();
        for work in retired {
            self.settle_child_document_script_ready_task_without_execution(work);
        }
        if retired_count != 0 {
            self.page_child_frame_task_sender().signal_reconsideration();
        }
        retired_count
    }

    /// Admit child parser-ready input produced inside a native callback into
    /// the durable scheduler store and, once its realm is executable, the
    /// stable `ChildFrameTask` source.
    pub(crate) fn push_child_document_script_ready_input(
        &mut self,
        work: FrameDocumentClassicScriptSchedulerWork,
    ) -> bool {
        let route = work.dispatch_route();
        let owner = route.task_owner();
        let Some(child_handle) = route.child_handle() else {
            return false;
        };
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, owner)
        {
            return false;
        }
        self.child_document_script_schedulers
            .notify_parser_classic_next_owner_action(work);
        if self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(owner)
            .is_none()
        {
            let _ = self.request_child_frame_realm_materialization_for_owner(child_handle, owner);
        }
        self.admit_runnable_child_document_script_tasks();
        true
    }

    /// Append exact-Document script work behind its realm-materialization
    /// prerequisite in the single stable child-frame FIFO.
    pub(crate) fn queue_child_document_script_work_with_realm_prerequisite(
        &mut self,
        work: FrameDocumentUnboundScriptWork,
    ) -> Option<FrameDocumentScriptWorkAdmission> {
        let child_handle = work.child_handle();
        let owner = work.owner();
        if self.current_child_document_task_owner(child_handle) != Some(owner) {
            return None;
        }

        let reserved_realm_id = self.frame_owner_store.ensure_child_realm(child_handle)?;
        if self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner)
            != Some(reserved_realm_id)
            || !work.can_materialize_in_realm(reserved_realm_id)
        {
            return None;
        }

        let request =
            self.request_child_frame_realm_materialization_for_owner(child_handle, owner)?;
        let realm_id = request.realm_id();
        debug_assert_eq!(realm_id, reserved_realm_id);
        let admission = match request {
            crate::frame_owner_model::FrameRealmMaterializationRequest::AlreadyMaterialized {
                ..
            } => FrameDocumentScriptWorkAdmission::Runnable,
            crate::frame_owner_model::FrameRealmMaterializationRequest::NewlyQueued { .. }
            | crate::frame_owner_model::FrameRealmMaterializationRequest::AlreadyQueued {
                ..
            } => FrameDocumentScriptWorkAdmission::QueuedBehindRealm,
        };
        let work = work.bind_to_realm(realm_id);
        self.queue_child_document_script_ready_task_for_realm(work.into(), realm_id)
            .then_some(admission)
    }

    /// Admit script work whose exact child realm has just become executable.
    /// Lifecycle work is already resident in the stable ChildFrameTask FIFO
    /// behind its realm-materialization prerequisite and needs no handoff.
    pub(crate) fn admit_child_realm_dependent_work_after_materialization(
        &mut self,
        child_handle: crate::document_runtime::DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        if let Some(work) =
            self.take_child_classic_script_scheduler_work_for_current_document(child_handle)
        {
            self.child_document_script_schedulers
                .notify_parser_classic_next_owner_action(work);
        }
        let _ = self.admit_next_child_parser_deferred_script_if_ready(child_handle, owner);
        self.admit_runnable_child_document_script_tasks();
    }

    fn settle_child_document_script_ready_task_without_execution(
        &mut self,
        work: FrameDocumentScriptReadyTaskWork,
    ) {
        let FrameDocumentScriptReadyTaskWork::DocumentScriptExecution(work) = work else {
            // Scheduler-owned parser/module state is retired together with its
            // exact Document. It must not be reinserted into another queue.
            return;
        };
        match *work {
            FrameDocumentRealmBoundScriptWork::ExternalClassic(work) => {
                let _ = self.settle_child_async_classic_script_load_delay(
                    work.child_handle,
                    work.owner,
                    work.load_delay,
                );
            }
            FrameDocumentRealmBoundScriptWork::JavascriptUrl(work) => {
                let _ = self.drop_child_javascript_url_document_script(&work);
            }
        }
    }
}

fn report_child_document_ready_owner_mismatch(
    mismatch: DocumentScriptReadyDispatchOwnerMismatch<
        FrameDocumentOwner,
        FrameDocumentReadyActionRoute,
    >,
) {
    let route = mismatch.route();
    tracing::debug!(
        queued_owner = ?mismatch.queued_owner(),
        payload_owner = ?mismatch.payload_owner(),
        child_handle = ?route.child_handle(),
        task_owner = ?route.task_owner(),
        realm_id = ?route.optional_realm_id(),
        requires_realm = route.requires_realm(),
        script_handle = ?route.script_handle(),
        "dropping child document script ready work queued under mismatched owner"
    );
}
