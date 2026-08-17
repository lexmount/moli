use super::PageVm;
use crate::page_task_queue::{RendererPageReadyDescriptor, RendererPageWindowDocumentTaskOwner};

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
pub(in crate::runtime::page_vm) struct StaleWindowDocumentTaskAdmission {
    current_owner: Option<RendererPageWindowDocumentTaskOwner>,
    may_discard_local_payload: bool,
}

impl StaleWindowDocumentTaskAdmission {
    pub(in crate::runtime::page_vm) const fn current_owner(
        &self,
    ) -> Option<RendererPageWindowDocumentTaskOwner> {
        self.current_owner
    }

    pub(in crate::runtime::page_vm) const fn may_discard_local_payload(&self) -> bool {
        self.may_discard_local_payload
    }
}

impl PageVm {
    pub(in crate::runtime::page_vm) fn authorize_current_window_document_task<T, K: Eq>(
        &self,
        task: T,
        owner: RendererPageWindowDocumentTaskOwner,
        kind: K,
        current: Option<(RendererPageWindowDocumentTaskOwner, K)>,
    ) -> Result<AuthorizedCurrentWindowDocumentTask<T>, StaleWindowDocumentTaskAdmission> {
        if current.as_ref() == Some(&(owner, kind)) {
            return Ok(AuthorizedCurrentWindowDocumentTask::new(task));
        }
        Err(StaleWindowDocumentTaskAdmission {
            current_owner: current.map(|(owner, _)| owner),
            may_discard_local_payload: owner.root_document()
                == self.document_lifecycle.identity().document,
        })
    }

    /// Source-local eligibility for a descriptor already visible to the Page
    /// scheduler. This query may gate execution on current Document state, but
    /// it must not compare or reorder competing source heads.
    pub(in crate::runtime) fn page_ready_descriptor_is_eligible(
        &mut self,
        descriptor: RendererPageReadyDescriptor,
    ) -> bool {
        match descriptor {
            RendererPageReadyDescriptor::ActionWindow { .. }
            | RendererPageReadyDescriptor::DomManipulation { .. }
            | RendererPageReadyDescriptor::UserInteraction { .. }
            | RendererPageReadyDescriptor::FileReading { .. }
            | RendererPageReadyDescriptor::MiscPlatformApi { .. }
            | RendererPageReadyDescriptor::DedicatedWorkerClientEvent { .. }
            | RendererPageReadyDescriptor::SharedWorkerClientEvent { .. }
            | RendererPageReadyDescriptor::ServiceWorkerInternal { .. }
            | RendererPageReadyDescriptor::ServiceWorkerClientMessage { .. }
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
    ) -> Option<RendererPageReadyDescriptor> {
        self.vm()
            .has_ready_timeout()
            .then(|| self.vm().next_timeout_deadline())
            .flatten()
            .map(|deadline| RendererPageReadyDescriptor::Timer { deadline })
    }
}
