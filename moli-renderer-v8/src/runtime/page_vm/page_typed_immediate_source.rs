use std::fmt::Debug;

use super::{PageOwnerTurnOutcome, PageVm};
use crate::{
    page_task_queue::{
        PageWindowDocumentTaskTurnAction, RendererPageReadyDescriptor, RendererPageTimerSelection,
        RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    },
    runtime::RendererDocumentToken,
    script_vm::ScriptVm,
};

/// Proof that the Page arbiter matched a scheduler task against its exact
/// PageVm namespace and Window/Document ledger slot.
///
/// The wrapper is shared by exact Window/Document task families. Its
/// constructor is confined to the PageVm arbiter; V8 executors can only unwrap
/// a capability that has crossed that boundary.
pub(crate) struct AuthorizedCurrentWindowDocumentTask<T>(T);

impl<T> AuthorizedCurrentWindowDocumentTask<T> {
    pub(in crate::runtime::page_vm) fn new(task: T) -> Self {
        Self(task)
    }

    pub(crate) fn into_task(self) -> T {
        self.0
    }

    #[cfg(test)]
    pub(crate) fn new_for_executor_test(task: T) -> Self {
        Self(task)
    }
}

/// Stale result shared by exact Window/Document task families.
///
/// A Host-local id may be retired only when the queued task belongs to the
/// currently resident PageVm namespace. This prevents an old stable task from
/// consuming a naturally reused id after PageVm replacement.
pub(in crate::runtime::page_vm) struct StaleWindowDocumentTaskAdmission<T> {
    current_owner: Option<RendererPageWindowDocumentTaskOwner<T>>,
    may_discard_local_payload: bool,
}

impl<T: Copy> StaleWindowDocumentTaskAdmission<T> {
    pub(in crate::runtime::page_vm) const fn current_owner(
        &self,
    ) -> Option<RendererPageWindowDocumentTaskOwner<T>> {
        self.current_owner
    }

    pub(in crate::runtime::page_vm) const fn may_discard_local_payload(&self) -> bool {
        self.may_discard_local_payload
    }
}

impl PageVm {
    pub(in crate::runtime) fn document_lifecycle_dom_owner_is_current(
        &self,
        owner: crate::page_task_queue::RendererPageDomManipulationOwner,
    ) -> bool {
        use crate::page_task_queue::{
            RendererPageChildFrameTaskTarget, RendererPageDomManipulationOwner,
        };
        let root_document = self.document_lifecycle.identity().document;
        match owner {
            RendererPageDomManipulationOwner::MainDocumentLifecycle(owner) => {
                owner.root_document == root_document
                    && self.vm().current_main_document_task_owner() == Some(owner.body.owner())
            }
            RendererPageDomManipulationOwner::ChildDocumentLifecycle(owner) => {
                let RendererPageChildFrameTaskTarget::DocumentLifecycle(target) = owner.target()
                else {
                    unreachable!("child lifecycle DOM task must retain its typed target");
                };
                owner.root_document() == root_document
                    && self.vm().current_child_document_lifecycle_target(target) == Some(target)
            }
            RendererPageDomManipulationOwner::ChildHostLoad(owner) => {
                let RendererPageChildFrameTaskTarget::HostLoad(target) = owner.target() else {
                    unreachable!("child load DOM task must retain its typed target");
                };
                owner.root_document() == root_document
                    && self.vm().current_child_host_load_target(target) == Some(target)
            }
            _ => false,
        }
    }

    pub(in crate::runtime::page_vm) fn authorize_current_window_document_task<
        T,
        K: Eq,
        O: Copy + Eq,
    >(
        &self,
        task: T,
        owner: RendererPageWindowDocumentTaskOwner<O>,
        kind: K,
        current: Option<(RendererPageWindowDocumentTaskOwner<O>, K)>,
    ) -> Result<AuthorizedCurrentWindowDocumentTask<T>, StaleWindowDocumentTaskAdmission<O>> {
        if current.as_ref() == Some(&(owner, kind)) {
            return Ok(AuthorizedCurrentWindowDocumentTask::new(task));
        }
        Err(StaleWindowDocumentTaskAdmission {
            current_owner: current.map(|(owner, _)| owner),
            may_discard_local_payload: owner.root_document()
                == self.document_lifecycle.identity().document,
        })
    }

    pub(super) fn apply_selected_window_document_task<I, K, E, T>(
        &mut self,
        task: RendererPageWindowDocumentTask<I, K, T>,
        lookup: impl FnOnce(
            &ScriptVm,
            I,
            RendererDocumentToken,
        ) -> Option<(RendererPageWindowDocumentTaskOwner<T>, K)>,
        apply: impl FnOnce(
            &mut ScriptVm,
            AuthorizedCurrentWindowDocumentTask<RendererPageWindowDocumentTask<I, K, T>>,
        ) -> anyhow::Result<E>,
        discard: impl FnOnce(&mut ScriptVm, I) -> bool,
        stale_effect: impl FnOnce(Option<RendererPageWindowDocumentTaskOwner<T>>) -> E,
    ) -> anyhow::Result<PageOwnerTurnOutcome<PageWindowDocumentTaskTurnAction<I, K, E, T>>>
    where
        I: Copy + Debug,
        K: Copy + Eq + Debug,
        T: Copy + Eq + Debug,
    {
        let owner = task.owner();
        let task_id = task.task_id();
        let kind = task.kind();
        let current = lookup(
            self.vm(),
            task_id,
            self.document_lifecycle.identity().document,
        );
        let target_effect =
            match self.authorize_current_window_document_task(task, owner, kind, current) {
                Ok(authorization) => apply(self.vm_mut(), authorization)?,
                Err(stale) => {
                    if stale.may_discard_local_payload() {
                        discard(self.vm_mut(), task_id);
                    }
                    let current_owner = stale.current_owner();
                    tracing::debug!(
                        ?owner,
                        ?current_owner,
                        ?task_id,
                        ?kind,
                        "discarded stale Window/Document task"
                    );
                    stale_effect(current_owner)
                }
            };
        Ok(PageOwnerTurnOutcome::new(
            PageWindowDocumentTaskTurnAction {
                owner,
                task_id,
                kind,
                target_effect,
            },
        ))
    }

    /// Source-local eligibility for a descriptor already visible to the Page
    /// scheduler. This query may gate execution on current Document state, but
    /// it must not compare or reorder competing source heads.
    pub(in crate::runtime) fn page_ready_descriptor_is_eligible(
        &mut self,
        descriptor: RendererPageReadyDescriptor,
    ) -> bool {
        match descriptor {
            RendererPageReadyDescriptor::DomManipulation {
                owner:
                    crate::page_task_queue::RendererPageDomManipulationOwner::ChildDocumentLifecycle(
                        owner,
                    ),
                ..
            } => {
                if owner.root_document() != self.document_lifecycle.identity().document {
                    return true;
                }
                let crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentLifecycle(
                    target,
                ) = owner.target()
                else {
                    unreachable!("child lifecycle DOM carrier must retain its exact target");
                };
                // A current task retains its DOM FIFO position while its
                // separately queued realm prerequisite runs. It is not stale
                // merely because that realm has not materialized yet.
                !self.vm().child_document_lifecycle_waits_for_realm(target)
            }
            #[cfg(any(test, feature = "devtools"))]
            RendererPageReadyDescriptor::ActionWindow { .. } => true,
            RendererPageReadyDescriptor::DomManipulation { .. }
            | RendererPageReadyDescriptor::UserInteraction { .. }
            | RendererPageReadyDescriptor::FileReading { .. }
            | RendererPageReadyDescriptor::MiscPlatformApi { .. }
            | RendererPageReadyDescriptor::IntersectionObserver { .. }
            | RendererPageReadyDescriptor::FontLoading { .. }
            | RendererPageReadyDescriptor::DeviceSensor { .. }
            | RendererPageReadyDescriptor::CanvasBlobSerialization { .. }
            | RendererPageReadyDescriptor::DedicatedWorkerClientEvent { .. }
            | RendererPageReadyDescriptor::SharedWorkerClientEvent { .. }
            | RendererPageReadyDescriptor::ServiceWorkerInternal { .. }
            | RendererPageReadyDescriptor::ServiceWorkerClientMessage { .. }
            | RendererPageReadyDescriptor::BitmapTask { .. }
            | RendererPageReadyDescriptor::WebLocksTask { .. }
            | RendererPageReadyDescriptor::WebCryptoTask { .. }
            | RendererPageReadyDescriptor::IndexedDbTask { .. }
            | RendererPageReadyDescriptor::OpfsTask { .. }
            | RendererPageReadyDescriptor::InternalLoading { .. }
            | RendererPageReadyDescriptor::MainDocumentRuntime { .. }
            | RendererPageReadyDescriptor::NavigationAndTraversal { .. }
            | RendererPageReadyDescriptor::RenderingUpdate { .. }
            | RendererPageReadyDescriptor::MediaElementEvent { .. }
            | RendererPageReadyDescriptor::ChildModuleDependencyFetchStart { .. }
            | RendererPageReadyDescriptor::ChildFrameTask { .. }
            | RendererPageReadyDescriptor::V8ForegroundTask { .. }
            | RendererPageReadyDescriptor::ModuleReaction { .. }
            | RendererPageReadyDescriptor::MessagePortDelivery { .. }
            | RendererPageReadyDescriptor::DynamicImportOwnerAction { .. }
            | RendererPageReadyDescriptor::ModulepreloadStart { .. }
            | RendererPageReadyDescriptor::Networking { .. }
            | RendererPageReadyDescriptor::Timer { .. } => true,
            RendererPageReadyDescriptor::WebSocket {
                owner, readiness, ..
            } => {
                matches!(
                    readiness,
                    crate::page_task_queue::RendererPageWebSocketReadiness::Ready
                ) || owner.root_document() != self.document_lifecycle.identity().document
            }
            RendererPageReadyDescriptor::ChildModuleScriptTerminal { owner, .. } => {
                self.page_child_module_script_terminal_is_eligible_for_owner_turn(owner)
            }
            RendererPageReadyDescriptor::ChildModulepreloadEventAction { owner, .. } => {
                self.page_child_modulepreload_event_action_is_eligible_for_owner_turn(owner)
            }
            RendererPageReadyDescriptor::WindowMessage { owner, task_id, .. } => {
                self.page_window_message_is_eligible_for_owner_turn(owner, task_id)
            }
        }
    }

    pub(in crate::runtime) fn due_page_timer_ready_descriptor(
        &self,
        selection: RendererPageTimerSelection,
    ) -> Option<RendererPageReadyDescriptor> {
        self.vm()
            .next_ready_timeout_deadline(selection)
            .map(|deadline| RendererPageReadyDescriptor::Timer {
                deadline,
                selection,
            })
    }
}
