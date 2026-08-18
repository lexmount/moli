use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_module_script_tree as module_tree;

#[cfg(test)]
use super::host::DynamicScriptBatch;
use super::{
    host::{
        FailedDynamicScript, ModuleFailurePolicy, QueuedScriptFailureKind, RuntimeScriptAdmission,
        RuntimeScriptAdmissionPayload,
    },
    page_task_queue::MainDocumentRuntimeContinuationSender,
    planning::{
        PreparedScript, PreparedScriptSourceLoadOutcome,
        load_prepared_script_source_outcome_with_document_character_set,
        load_service_worker_aware_external_script_source_outcome,
        prepared_script_with_loaded_source,
    },
    types::{
        ScriptErrorConstructorKind, ScriptKind, ScriptMode, ScriptSourceKind,
        SharedNavigationResponseResult,
    },
};
use crate::frame_owner_model::MainDocumentScriptLoadDelayLease;
use crate::module_script_continuation::{
    ModuleScriptContinuation, ModuleScriptEvaluationContinuation,
    ModuleScriptEvaluationReactionState, ModuleScriptEvaluationUpdate,
};
use crate::network::{RendererResourceTaskRunner, ResourceRequestClient};
use moli_owner_queue::{OwnerTaskSource, OwnerWakeQueue};
use url::Url;

mod script_lanes;

use script_lanes::DynamicScriptLanes;

fn enqueue_runtime_script_continuation_once(
    continuation_tx: &Arc<Mutex<Option<MainDocumentRuntimeContinuationSender>>>,
    continuation_turn_queued: &Arc<AtomicBool>,
) -> bool {
    let Some(task_tx) = continuation_tx.lock().clone() else {
        return false;
    };
    if continuation_turn_queued
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    if task_tx.send_runtime_script_continuation().is_ok() {
        true
    } else {
        continuation_turn_queued.store(false, Ordering::Release);
        false
    }
}

#[derive(Debug)]
pub(super) struct DynamicScriptOwner {
    script_lanes: DynamicScriptLanes,
    followup_work: OwnerTaskSource<DynamicScriptRunnable>,
    next_id: u64,
    next_ready_order: u64,
    in_flight_loads: usize,
    owner_event_tx: tokio::sync::mpsc::UnboundedSender<DynamicScriptOwnerEvent>,
    continuation_tx: Arc<Mutex<Option<MainDocumentRuntimeContinuationSender>>>,
    continuation_turn_queued: Arc<AtomicBool>,
    load_delay_bindings: HashMap<DynamicScriptOwnerId, MainDocumentScriptLoadDelayLease>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DynamicScriptOwnerId(u64);

impl DynamicScriptOwnerId {
    fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[cfg(test)]
    pub(crate) fn from_u64(raw: u64) -> Self {
        Self(raw)
    }
}

/// One selected runtime script together with the exact Window-load lease
/// transferred from `DynamicScriptOwner` to its stable page-owned task.
///
/// Once this claim exists, terminal settlement must consume the claim itself;
/// it must not look the lease up through `DynamicScriptOwnerId` again. This
/// keeps the work residence and its lifecycle ownership in the same value even
/// when another runtime path temporarily moves `DynamicScriptOwner` aside.
#[derive(Debug)]
pub(crate) struct DynamicScriptPageTaskClaim {
    id: DynamicScriptOwnerId,
    load_delay_binding: MainDocumentScriptLoadDelayLease,
}

impl DynamicScriptPageTaskClaim {
    pub(crate) fn id(&self) -> DynamicScriptOwnerId {
        self.id
    }

    pub(crate) fn owner(&self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.load_delay_binding.owner()
    }

    pub(crate) fn into_parts(self) -> (DynamicScriptOwnerId, MainDocumentScriptLoadDelayLease) {
        (self.id, self.load_delay_binding)
    }
}

#[derive(Debug)]
struct DynamicScriptEntry {
    id: DynamicScriptOwnerId,
    script: PreparedScript,
    ready_state: DynamicScriptReadyState,
}

#[derive(Debug)]
enum DynamicScriptReadyState {
    Loading,
    Ready {
        order: u64,
        source_network_result: Option<SharedNavigationResponseResult>,
    },
    SuspendedModuleScriptGraph {
        wait: DynamicModuleScriptGraphWait,
        continuation: Option<Box<ModuleScriptContinuation>>,
    },
    ReadyModuleScriptGraph {
        order: u64,
        continuation: Box<ModuleScriptContinuation>,
    },
    SuspendedModuleScriptEvaluation {
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    },
    ReadyModuleScriptEvaluation {
        order: u64,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    },
    Failed {
        failure: DynamicScriptFailure,
        order: u64,
    },
}

#[derive(Debug)]
enum DynamicModuleScriptGraphWait {
    PendingFetch {
        load_ids: Vec<u64>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
    },
}

#[derive(Debug, Clone)]
struct DynamicScriptFailure {
    message: String,
    kind: DynamicScriptFailureKind,
    module_failure_policy: Option<ModuleFailurePolicy>,
    source_network_result: Option<SharedNavigationResponseResult>,
    error_constructor: Option<ScriptErrorConstructorKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DynamicScriptFailureKind {
    Immediate,
    ModuleFetch,
    ModuleResolve,
    ModuleInstantiate,
}

impl DynamicScriptFailure {
    fn with_kind(
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
    ) -> Self {
        Self {
            message,
            kind,
            module_failure_policy,
            source_network_result: None,
            error_constructor: None,
        }
    }

    fn with_source_network_result(
        mut self,
        source_network_result: Option<SharedNavigationResponseResult>,
    ) -> Self {
        self.source_network_result = source_network_result;
        self
    }

    fn with_error_constructor(
        mut self,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Self {
        self.error_constructor = error_constructor;
        self
    }

    fn is_deferrable_module(&self) -> bool {
        self.kind.is_deferrable_module()
    }

    #[cfg(test)]
    fn is_pending_module_script_graph_completion(&self) -> bool {
        self.is_deferrable_module()
            && matches!(
                self.module_failure_policy,
                Some(
                    ModuleFailurePolicy::ModuleTreeLoadFailure | ModuleFailurePolicy::GraphFailure
                )
            )
    }
}

impl DynamicScriptFailureKind {
    pub(super) fn is_deferrable_module(self) -> bool {
        matches!(
            self,
            DynamicScriptFailureKind::ModuleFetch
                | DynamicScriptFailureKind::ModuleResolve
                | DynamicScriptFailureKind::ModuleInstantiate
        )
    }
}

#[derive(Debug)]
pub(super) struct DynamicScriptLoadCompletion {
    id: DynamicScriptOwnerId,
    outcome: PreparedScriptSourceLoadOutcome,
}

#[derive(Debug)]
pub(super) enum DynamicScriptOwnerEvent {
    Completion(DynamicScriptLoadCompletion),
}

#[derive(Debug, Default)]
pub(super) struct DynamicScriptOwnerEventSource {
    events: OwnerWakeQueue<DynamicScriptOwnerEvent>,
}

impl DynamicScriptOwnerEventSource {
    pub(super) fn sender(&self) -> tokio::sync::mpsc::UnboundedSender<DynamicScriptOwnerEvent> {
        self.events.sender()
    }

    pub(super) fn drain_ready(&mut self) -> VecDeque<DynamicScriptOwnerEvent> {
        let mut events = VecDeque::new();
        self.events.try_drain_into(&mut events);
        events
    }
}

#[derive(Clone, Debug)]
struct DynamicScriptOwnerEventSender {
    tx: tokio::sync::mpsc::UnboundedSender<DynamicScriptOwnerEvent>,
    continuation_tx: Arc<Mutex<Option<MainDocumentRuntimeContinuationSender>>>,
    continuation_turn_queued: Arc<AtomicBool>,
}

impl DynamicScriptOwnerEventSender {
    fn send(
        &self,
        event: DynamicScriptOwnerEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<DynamicScriptOwnerEvent>> {
        self.tx.send(event)?;
        enqueue_runtime_script_continuation_once(
            &self.continuation_tx,
            &self.continuation_turn_queued,
        );
        Ok(())
    }
}

#[cfg(test)]
impl Default for DynamicScriptOwner {
    fn default() -> Self {
        let events = DynamicScriptOwnerEventSource::default();
        Self::with_event_sender(events.sender())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicScriptQueueKind {
    InOrder,
    ImportMapInOrder,
    ModuleInOrder,
    Async,
}

impl DynamicScriptQueueKind {
    fn for_mode(mode: ScriptMode) -> Self {
        match mode {
            ScriptMode::InOrder => Self::InOrder,
            ScriptMode::ImportMapInOrder => Self::ImportMapInOrder,
            ScriptMode::ModuleInOrder => Self::ModuleInOrder,
            ScriptMode::Async => Self::Async,
            ScriptMode::Normal | ScriptMode::Defer | ScriptMode::ModuleDefer => {
                unreachable!("runtime scripts must use a dynamic-owner queue mode")
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum DynamicScriptRunnable {
    Execute {
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        source_network_result: Option<SharedNavigationResponseResult>,
    },
    ContinueModuleScriptGraph {
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    },
    ContinueModuleScriptEvaluation {
        id: DynamicScriptOwnerId,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    },
    DispatchError {
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
        source_network_result: Option<SharedNavigationResponseResult>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    },
}

/// One exact runtime-script error terminal that is already the authoritative
/// next candidate in `DynamicScriptOwner`.
///
/// This is deliberately narrower than `DynamicScriptRunnable`: a selected
/// module/resource action may synchronously settle only graph failures carried
/// by that action. It must not use the terminal as a license to drain an
/// unrelated ready script, load event, or module continuation.
#[derive(Debug)]
pub(super) struct DynamicScriptFailureTerminal {
    pub(super) id: DynamicScriptOwnerId,
    pub(super) script: PreparedScript,
    pub(super) message: String,
    pub(super) kind: DynamicScriptFailureKind,
    pub(super) module_failure_policy: Option<ModuleFailurePolicy>,
    pub(super) source_network_result: Option<SharedNavigationResponseResult>,
    pub(super) error_constructor: Option<ScriptErrorConstructorKind>,
}

#[derive(Debug)]
pub(super) enum DynamicModuleScriptContinuationWork {
    Graph {
        continuation: Box<ModuleScriptContinuation>,
    },
    Evaluation {
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    },
}

#[derive(Debug)]
pub(super) enum DynamicScriptOwnerPoll {
    Work(Box<DynamicScriptRunnable>),
    Idle,
    StalledWithoutInflightLoads,
}

#[derive(Clone)]
pub(crate) struct DynamicScriptServiceWorkerContext {
    pub(crate) browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    pub(crate) client_id: crate::service_worker_runtime::ServiceWorkerClientId,
    pub(crate) document_url: Url,
}

impl DynamicScriptOwner {
    pub(super) fn with_event_sender(
        owner_event_tx: tokio::sync::mpsc::UnboundedSender<DynamicScriptOwnerEvent>,
    ) -> Self {
        Self {
            script_lanes: DynamicScriptLanes::default(),
            followup_work: OwnerTaskSource::default(),
            next_id: 0,
            next_ready_order: 0,
            in_flight_loads: 0,
            owner_event_tx,
            continuation_tx: Arc::default(),
            continuation_turn_queued: Arc::default(),
            load_delay_bindings: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn enqueue_batch(&mut self, batch: DynamicScriptBatch) {
        self.enqueue_batch_with_optional_load_delay_binding(batch, |_, _| None);
    }

    #[cfg(test)]
    fn enqueue_batch_with_optional_load_delay_binding(
        &mut self,
        batch: DynamicScriptBatch,
        mut bind_load_delay: impl FnMut(
            DynamicScriptOwnerId,
            &PreparedScript,
        ) -> Option<MainDocumentScriptLoadDelayLease>,
    ) {
        for script in batch.in_order {
            let id = self.reserve_script_id();
            let binding = bind_load_delay(id, &script);
            self.enqueue_inline_script_with_id_for_test(
                id,
                DynamicScriptQueueKind::InOrder,
                script,
                binding,
            );
        }
        for script in batch.importmap_in_order {
            let id = self.reserve_script_id();
            let binding = bind_load_delay(id, &script);
            self.enqueue_inline_script_with_id_for_test(
                id,
                DynamicScriptQueueKind::ImportMapInOrder,
                script,
                binding,
            );
        }
        for script in batch.module_in_order {
            let id = self.reserve_script_id();
            let binding = bind_load_delay(id, &script);
            self.enqueue_inline_script_with_id_for_test(
                id,
                DynamicScriptQueueKind::ModuleInOrder,
                script,
                binding,
            );
        }
        for script in batch.async_scripts {
            let id = self.reserve_script_id();
            let binding = bind_load_delay(id, &script);
            self.enqueue_inline_script_with_id_for_test(
                id,
                DynamicScriptQueueKind::Async,
                script,
                binding,
            );
        }
        for failed in batch.failed_scripts {
            let id = self.reserve_script_id();
            let binding = bind_load_delay(id, &failed.script);
            self.enqueue_failed_script_with_binding(id, failed, binding);
        }
        self.refresh_followup_work();
    }

    pub(super) fn enqueue_admission(
        &mut self,
        loader: &ResourceRequestClient,
        task_runner: RendererResourceTaskRunner,
        admission: RuntimeScriptAdmission,
        document_character_set: Option<&str>,
        service_worker_context: Option<&DynamicScriptServiceWorkerContext>,
    ) {
        let (payload, binding) = admission.into_parts();
        let id = self.reserve_script_id();
        match payload {
            RuntimeScriptAdmissionPayload::Script(script) => {
                let queue_kind = DynamicScriptQueueKind::for_mode(script.mode);
                self.enqueue_script_with_id(
                    loader,
                    task_runner,
                    id,
                    queue_kind,
                    script,
                    document_character_set,
                    service_worker_context,
                    Some(binding),
                );
            }
            RuntimeScriptAdmissionPayload::Failed(failed) => {
                self.enqueue_failed_script_with_binding(id, failed, Some(binding));
            }
        }
        self.refresh_followup_work();
    }

    pub(super) fn next_runnable_script(&mut self) -> Option<DynamicScriptRunnable> {
        self.refresh_followup_work();
        self.followup_work.pop_front()
    }

    pub(super) fn take_ready_module_script_continuation(
        &mut self,
    ) -> Option<DynamicModuleScriptContinuationWork> {
        let runnable = self.next_runnable_script()?;
        match runnable {
            DynamicScriptRunnable::ContinueModuleScriptGraph { continuation, .. } => {
                Some(DynamicModuleScriptContinuationWork::Graph { continuation })
            }
            DynamicScriptRunnable::ContinueModuleScriptEvaluation { evaluation, .. } => {
                Some(DynamicModuleScriptContinuationWork::Evaluation { evaluation })
            }
            other => {
                self.restore_followup_task_front(other);
                None
            }
        }
    }

    pub(super) fn has_ready_module_script_continuation(&mut self) -> bool {
        self.refresh_followup_work();
        matches!(
            self.followup_work.front(),
            Some(
                DynamicScriptRunnable::ContinueModuleScriptGraph { .. }
                    | DynamicScriptRunnable::ContinueModuleScriptEvaluation { .. }
            )
        )
    }

    pub(super) fn has_immediately_runnable_work(&mut self) -> bool {
        self.refresh_followup_work();
        !self.followup_work.is_empty()
    }

    /// Whether any lane exposes a runnable candidate under the caller's
    /// current lifecycle policy.
    ///
    /// Ordered queues expose only their shared earliest head. The async queue
    /// exposes every ready entry. A gated ordered head therefore blocks later
    /// ordered scripts without hiding an unrelated ready async script.
    pub(super) fn has_immediately_runnable_work_matching(
        &mut self,
        mut predicate: impl FnMut(&PreparedScript) -> bool,
    ) -> bool {
        self.refresh_followup_work_matching(&mut predicate);
        !self.followup_work.is_empty()
    }

    /// Select one currently admitted lane candidate without moving rejected
    /// work to a competing queue.
    pub(super) fn next_runnable_script_matching(
        &mut self,
        mut predicate: impl FnMut(&PreparedScript) -> bool,
    ) -> Option<DynamicScriptRunnable> {
        self.refresh_followup_work_matching(&mut predicate);
        self.followup_work.pop_front()
    }

    /// Takes the next error terminal only when it belongs to the currently
    /// selected owner action.
    ///
    /// Normal lane arbitration still decides which task is next. If an
    /// unrelated script or continuation is ahead of every `action_owner_id`,
    /// this returns `None` and leaves that work in its authoritative lane.
    pub(super) fn take_runnable_failure_terminal_for_action(
        &mut self,
        action_owner_ids: &[DynamicScriptOwnerId],
    ) -> Option<DynamicScriptFailureTerminal> {
        self.refresh_followup_work();
        let is_owned_terminal = matches!(
            self.followup_work.front(),
            Some(DynamicScriptRunnable::DispatchError { id, .. })
                if action_owner_ids.contains(id)
        );
        if !is_owned_terminal {
            return None;
        }
        let Some(DynamicScriptRunnable::DispatchError {
            id,
            script,
            message,
            kind,
            module_failure_policy,
            source_network_result,
            error_constructor,
        }) = self.followup_work.pop_front()
        else {
            unreachable!("owned error terminal changed after an immutable front check")
        };
        Some(DynamicScriptFailureTerminal {
            id,
            script,
            message,
            kind,
            module_failure_policy,
            source_network_result,
            error_constructor,
        })
    }

    /// Retires one selected runtime script and transfers its exact load-delay
    /// lease to the script action that is completing the observable terminal.
    pub(super) fn finish_script_terminal(
        &mut self,
        id: DynamicScriptOwnerId,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        let _ = self.remove_entry_by_id(id);
        let lease = self.load_delay_bindings.remove(&id);
        self.refresh_followup_work();
        lease
    }

    /// Transfers terminal responsibility for one already-selected non-module
    /// runtime script to its stable page-owned execution task.
    pub(super) fn claim_page_owned_execution(
        &mut self,
        id: DynamicScriptOwnerId,
    ) -> Option<DynamicScriptPageTaskClaim> {
        let load_delay_binding = self.load_delay_bindings.remove(&id)?;
        self.refresh_followup_work();
        Some(DynamicScriptPageTaskClaim {
            id,
            load_delay_binding,
        })
    }

    pub(super) fn restore_page_owned_execution_claim(&mut self, claim: DynamicScriptPageTaskClaim) {
        let (id, load_delay_binding) = claim.into_parts();
        let previous = self.load_delay_bindings.insert(id, load_delay_binding);
        assert!(
            previous.is_none(),
            "restored page-owned runtime script claim must not replace another lease"
        );
        self.refresh_followup_work();
    }

    #[cfg(test)]
    pub(super) fn note_script_failed_with_kind(
        &mut self,
        id: DynamicScriptOwnerId,
        script: &PreparedScript,
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) {
        self.note_script_failed_with_kind_and_error_constructor(
            id,
            script,
            message,
            kind,
            module_failure_policy,
            error_constructor,
        );
    }

    pub(super) fn note_script_failed_with_kind_and_error_constructor(
        &mut self,
        id: DynamicScriptOwnerId,
        script: &PreparedScript,
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) {
        assert!(
            script.host_script_handle.is_some(),
            "runtime dynamic script should carry host handle before failure dispatch planning"
        );
        let failure = DynamicScriptFailure::with_kind(message, kind, module_failure_policy)
            .with_error_constructor(error_constructor);
        if failure.is_deferrable_module() {
            self.note_script_failed_in_queue_or_enqueue(id, script.clone(), failure);
            self.refresh_followup_work();
            return;
        }
        self.remove_entry_by_id(id);
        self.requeue_script_failure_front(id, script.clone(), failure);
        self.refresh_followup_work();
    }

    pub(super) fn note_module_script_graph_fetch_suspended(
        &mut self,
        id: DynamicScriptOwnerId,
        load_ids: Vec<u64>,
        joined_clients: Vec<module_tree::SingleModuleClientToken>,
        continuation: Box<ModuleScriptContinuation>,
    ) {
        debug_assert!(
            !load_ids.is_empty() || !joined_clients.is_empty(),
            "suspended module graph fetch should carry at least one load id or joined client"
        );
        let continuation = match self.merge_module_script_graph_pending_fetches(
            id,
            &load_ids,
            &joined_clients,
            continuation,
        ) {
            Ok(()) => {
                self.refresh_followup_work();
                return;
            }
            Err(continuation) => continuation,
        };
        let script = continuation.script.clone();
        self.note_module_script_graph_suspended(
            id,
            DynamicModuleScriptGraphWait::PendingFetch {
                load_ids,
                joined_clients,
            },
            script,
            Some(continuation),
        );
    }

    pub(super) fn restore_module_script_graph_pending_continuation(
        &mut self,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) -> bool {
        let restored = Self::restore_module_script_graph_pending_continuation_in_queue(
            &mut self.script_lanes.in_order,
            id,
            continuation,
        )
        .or_else(|continuation| {
            Self::restore_module_script_graph_pending_continuation_in_queue(
                &mut self.script_lanes.importmap_in_order,
                id,
                continuation,
            )
        })
        .or_else(|continuation| {
            Self::restore_module_script_graph_pending_continuation_in_queue(
                &mut self.script_lanes.module_in_order,
                id,
                continuation,
            )
        })
        .or_else(|continuation| {
            Self::restore_module_script_graph_pending_continuation_in_queue(
                &mut self.script_lanes.async_scripts,
                id,
                continuation,
            )
        })
        .is_ok();
        if restored {
            self.refresh_followup_work();
        }
        restored
    }

    fn note_module_script_graph_suspended(
        &mut self,
        id: DynamicScriptOwnerId,
        wait: DynamicModuleScriptGraphWait,
        script: PreparedScript,
        continuation: Option<Box<ModuleScriptContinuation>>,
    ) {
        debug_assert_eq!(
            script.kind,
            ScriptKind::Module,
            "only module scripts should suspend on native module graph work"
        );
        let ready_state =
            DynamicScriptReadyState::SuspendedModuleScriptGraph { wait, continuation };
        match self.update_entry_state_by_id(id, ready_state) {
            Ok(()) => {
                self.refresh_followup_work();
            }
            Err(ready_state) => {
                let DynamicScriptReadyState::SuspendedModuleScriptGraph { wait, continuation } =
                    *ready_state
                else {
                    unreachable!("unexpected dynamic script ready state returned")
                };
                self.push_entry_front(DynamicScriptEntry {
                    id,
                    script,
                    ready_state: DynamicScriptReadyState::SuspendedModuleScriptGraph {
                        wait,
                        continuation,
                    },
                });
                self.refresh_followup_work();
            }
        }
    }

    fn merge_module_script_graph_pending_fetches(
        &mut self,
        id: DynamicScriptOwnerId,
        load_ids: &[u64],
        joined_clients: &[module_tree::SingleModuleClientToken],
        continuation: Box<ModuleScriptContinuation>,
    ) -> Result<(), Box<ModuleScriptContinuation>> {
        Self::merge_module_script_graph_pending_fetches_in_queue(
            &mut self.script_lanes.in_order,
            id,
            load_ids,
            joined_clients,
            continuation,
        )
        .or_else(|continuation| {
            Self::merge_module_script_graph_pending_fetches_in_queue(
                &mut self.script_lanes.importmap_in_order,
                id,
                load_ids,
                joined_clients,
                continuation,
            )
        })
        .or_else(|continuation| {
            Self::merge_module_script_graph_pending_fetches_in_queue(
                &mut self.script_lanes.module_in_order,
                id,
                load_ids,
                joined_clients,
                continuation,
            )
        })
        .or_else(|continuation| {
            Self::merge_module_script_graph_pending_fetches_in_queue(
                &mut self.script_lanes.async_scripts,
                id,
                load_ids,
                joined_clients,
                continuation,
            )
        })
    }

    fn merge_module_script_graph_pending_fetches_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        id: DynamicScriptOwnerId,
        new_load_ids: &[u64],
        new_joined_clients: &[module_tree::SingleModuleClientToken],
        continuation: Box<ModuleScriptContinuation>,
    ) -> Result<(), Box<ModuleScriptContinuation>> {
        let Some(entry) = queue.iter_mut().find(|entry| entry.id == id) else {
            return Err(continuation);
        };
        let DynamicScriptReadyState::SuspendedModuleScriptGraph {
            wait:
                DynamicModuleScriptGraphWait::PendingFetch {
                    load_ids: stored_load_ids,
                    joined_clients: stored_joined_clients,
                },
            continuation: stored_continuation,
        } = &mut entry.ready_state
        else {
            return Err(continuation);
        };
        for load_id in new_load_ids {
            if !stored_load_ids.contains(load_id) {
                stored_load_ids.push(*load_id);
            }
        }
        for client in new_joined_clients {
            if !stored_joined_clients.contains(client) {
                stored_joined_clients.push(*client);
            }
        }
        *stored_continuation = Some(continuation);
        Ok(())
    }

    fn restore_module_script_graph_pending_continuation_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) -> Result<(), Box<ModuleScriptContinuation>> {
        let Some(entry) = queue.iter_mut().find(|entry| entry.id == id) else {
            return Err(continuation);
        };
        let DynamicScriptReadyState::SuspendedModuleScriptGraph {
            wait: DynamicModuleScriptGraphWait::PendingFetch { .. },
            continuation: stored_continuation,
        } = &mut entry.ready_state
        else {
            return Err(continuation);
        };
        *stored_continuation = Some(continuation);
        Ok(())
    }

    pub(super) fn note_module_script_graph_ready(
        &mut self,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) -> bool {
        let order = self.next_ready_order;
        let ready_state = DynamicScriptReadyState::ReadyModuleScriptGraph {
            order,
            continuation,
        };
        match self.update_entry_state_by_id(id, ready_state) {
            Ok(()) => {
                self.next_ready_order += 1;
                self.refresh_followup_work();
                true
            }
            Err(ready_state) => {
                let DynamicScriptReadyState::ReadyModuleScriptGraph {
                    order,
                    continuation,
                } = *ready_state
                else {
                    unreachable!("unexpected dynamic script ready state returned")
                };
                let script = continuation.script.clone();
                self.push_entry_front(DynamicScriptEntry {
                    id,
                    script,
                    ready_state: DynamicScriptReadyState::ReadyModuleScriptGraph {
                        order,
                        continuation,
                    },
                });
                self.next_ready_order += 1;
                self.refresh_followup_work();
                true
            }
        }
    }

    pub(super) fn note_module_script_evaluation_suspended(
        &mut self,
        id: DynamicScriptOwnerId,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    ) {
        let script = evaluation.script_continuation.script.clone();
        debug_assert_eq!(
            script.kind,
            ScriptKind::Module,
            "only module scripts should suspend on native module evaluation work"
        );
        let ready_state = DynamicScriptReadyState::SuspendedModuleScriptEvaluation { evaluation };
        match self.update_entry_state_by_id(id, ready_state) {
            Ok(()) => {
                self.refresh_followup_work();
            }
            Err(ready_state) => {
                let DynamicScriptReadyState::SuspendedModuleScriptEvaluation { evaluation } =
                    *ready_state
                else {
                    unreachable!("unexpected dynamic script ready state returned")
                };
                let entry = DynamicScriptEntry {
                    id,
                    script,
                    ready_state: DynamicScriptReadyState::SuspendedModuleScriptEvaluation {
                        evaluation,
                    },
                };
                self.push_entry_front(entry);
                self.refresh_followup_work();
            }
        }
    }

    #[cfg(test)]
    pub(super) fn note_module_script_evaluation_ready(
        &mut self,
        id: DynamicScriptOwnerId,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    ) -> bool {
        let order = self.next_ready_order;
        let ready_state =
            DynamicScriptReadyState::ReadyModuleScriptEvaluation { order, evaluation };
        match self.update_entry_state_by_id(id, ready_state) {
            Ok(()) => {
                self.next_ready_order += 1;
                self.refresh_followup_work();
                true
            }
            Err(ready_state) => {
                let DynamicScriptReadyState::ReadyModuleScriptEvaluation { order, evaluation } =
                    *ready_state
                else {
                    unreachable!("unexpected dynamic script ready state returned")
                };
                let script = evaluation.script_continuation.script.clone();
                self.push_entry_front(DynamicScriptEntry {
                    id,
                    script,
                    ready_state: DynamicScriptReadyState::ReadyModuleScriptEvaluation {
                        order,
                        evaluation,
                    },
                });
                self.next_ready_order += 1;
                self.refresh_followup_work();
                true
            }
        }
    }

    #[cfg(test)]
    pub(super) fn has_pending_module_script_evaluation(&mut self) -> bool {
        [
            &self.script_lanes.in_order,
            &self.script_lanes.importmap_in_order,
            &self.script_lanes.module_in_order,
            &self.script_lanes.async_scripts,
        ]
        .into_iter()
        .flat_map(|queue| queue.iter())
        .any(|entry| {
            matches!(
                entry.ready_state,
                DynamicScriptReadyState::SuspendedModuleScriptEvaluation { .. }
                    | DynamicScriptReadyState::ReadyModuleScriptEvaluation { .. }
            )
        })
    }

    #[cfg(test)]
    pub(super) fn has_pending_module_script_graph(&self) -> bool {
        [
            &self.script_lanes.in_order,
            &self.script_lanes.importmap_in_order,
            &self.script_lanes.module_in_order,
            &self.script_lanes.async_scripts,
        ]
        .into_iter()
        .flat_map(|queue| queue.iter())
        .any(Self::entry_has_pending_module_script_graph_completion)
    }

    pub(super) fn module_script_owner_id_for_pending_fetch(
        &self,
        load_id: u64,
    ) -> Option<DynamicScriptOwnerId> {
        [
            &self.script_lanes.in_order,
            &self.script_lanes.importmap_in_order,
            &self.script_lanes.module_in_order,
            &self.script_lanes.async_scripts,
        ]
        .into_iter()
        .flat_map(|queue| queue.iter())
        .find_map(|entry| {
            matches!(
                &entry.ready_state,
                DynamicScriptReadyState::SuspendedModuleScriptGraph {
                    wait: DynamicModuleScriptGraphWait::PendingFetch { load_ids, .. },
                    continuation: Some(_),
                } if load_ids.contains(&load_id)
            )
            .then_some(entry.id)
        })
    }

    #[cfg(test)]
    fn entry_has_pending_module_script_graph_completion(entry: &DynamicScriptEntry) -> bool {
        match &entry.ready_state {
            DynamicScriptReadyState::SuspendedModuleScriptGraph { .. }
            | DynamicScriptReadyState::ReadyModuleScriptGraph { .. } => true,
            // Graph/evaluation failures are the terminal error-dispatch phase of
            // a runtime-owned module graph. Keep them lifecycle-pending until
            // DynamicScriptOwner promotes the DispatchError task. Top-level
            // source-load failures are deferrable for ordering, but they are not
            // graph completions and must not block the module graph gate.
            DynamicScriptReadyState::Failed { failure, .. } => {
                failure.is_pending_module_script_graph_completion()
            }
            DynamicScriptReadyState::Loading
            | DynamicScriptReadyState::Ready { .. }
            | DynamicScriptReadyState::SuspendedModuleScriptEvaluation { .. }
            | DynamicScriptReadyState::ReadyModuleScriptEvaluation { .. } => false,
        }
    }

    pub(super) fn take_module_script_graph_pending_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptContinuation> {
        let result = Self::take_module_script_graph_pending_fetch_in_queue(
            &mut self.script_lanes.in_order,
            load_id,
        )
        .or_else(|| {
            Self::take_module_script_graph_pending_fetch_in_queue(
                &mut self.script_lanes.importmap_in_order,
                load_id,
            )
        })
        .or_else(|| {
            Self::take_module_script_graph_pending_fetch_in_queue(
                &mut self.script_lanes.module_in_order,
                load_id,
            )
        })
        .or_else(|| {
            Self::take_module_script_graph_pending_fetch_in_queue(
                &mut self.script_lanes.async_scripts,
                load_id,
            )
        });
        if result.is_some() {
            self.refresh_followup_work();
        }
        result.map(|boxed| *boxed)
    }

    pub(super) fn take_module_script_graph_pending_joined_client(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<ModuleScriptContinuation> {
        let result = Self::take_module_script_graph_pending_joined_client_in_queue(
            &mut self.script_lanes.in_order,
            client,
        )
        .or_else(|| {
            Self::take_module_script_graph_pending_joined_client_in_queue(
                &mut self.script_lanes.importmap_in_order,
                client,
            )
        })
        .or_else(|| {
            Self::take_module_script_graph_pending_joined_client_in_queue(
                &mut self.script_lanes.module_in_order,
                client,
            )
        })
        .or_else(|| {
            Self::take_module_script_graph_pending_joined_client_in_queue(
                &mut self.script_lanes.async_scripts,
                client,
            )
        });
        if result.is_some() {
            self.refresh_followup_work();
        }
        result.map(|boxed| *boxed)
    }

    pub(super) fn clear_module_script_graph_pending_waits(
        &mut self,
        id: DynamicScriptOwnerId,
    ) -> (Vec<u64>, Vec<module_tree::SingleModuleClientToken>) {
        let waits = Self::clear_module_script_graph_pending_waits_in_queue(
            &mut self.script_lanes.in_order,
            id,
        )
        .or_else(|| {
            Self::clear_module_script_graph_pending_waits_in_queue(
                &mut self.script_lanes.importmap_in_order,
                id,
            )
        })
        .or_else(|| {
            Self::clear_module_script_graph_pending_waits_in_queue(
                &mut self.script_lanes.module_in_order,
                id,
            )
        })
        .or_else(|| {
            Self::clear_module_script_graph_pending_waits_in_queue(
                &mut self.script_lanes.async_scripts,
                id,
            )
        })
        .unwrap_or_default();
        if !waits.0.is_empty() || !waits.1.is_empty() {
            self.refresh_followup_work();
        }
        waits
    }

    pub(super) fn mark_module_script_evaluation_fulfilled(
        &mut self,
        reaction_id: u64,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        let update = self.mark_module_script_evaluation_reaction(
            reaction_id,
            ModuleScriptEvaluationReactionState::Fulfilled,
        );
        if update.is_some() {
            self.refresh_followup_work();
        }
        update
    }

    pub(super) fn mark_module_script_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        let update = self.mark_module_script_evaluation_reaction(
            reaction_id,
            ModuleScriptEvaluationReactionState::Rejected {
                reason,
                error_constructor,
            },
        );
        if update.is_some() {
            self.refresh_followup_work();
        }
        update
    }

    /// Returns ready work or `Idle` without waiting for in-flight loads.
    ///
    /// The owning runtime must first apply every ready item from its
    /// `DynamicScriptOwnerEventSource`. Keeping that single-consumer source
    /// outside the shared owner state lets async waits proceed without
    /// holding a `RefMut<RuntimeScriptWorkState>`.
    pub(super) fn poll_nonblocking(&mut self) -> DynamicScriptOwnerPoll {
        if let Some(work) = self.next_runnable_script() {
            return DynamicScriptOwnerPoll::Work(Box::new(work));
        }
        if self.in_flight_loads == 0 && !self.is_idle() {
            return DynamicScriptOwnerPoll::StalledWithoutInflightLoads;
        }
        DynamicScriptOwnerPoll::Idle
    }

    pub(super) fn requeue_ready_script_front(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        source_network_result: Option<SharedNavigationResponseResult>,
    ) {
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::Ready {
                order: 0,
                source_network_result,
            },
        };
        self.push_entry_front(entry);
    }

    pub(super) fn requeue_module_script_graph_ready_front(
        &mut self,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) {
        let script = continuation.script.clone();
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::ReadyModuleScriptGraph {
                order: 0,
                continuation,
            },
        };
        self.push_entry_front(entry);
    }

    pub(super) fn requeue_module_script_evaluation_ready_front(
        &mut self,
        id: DynamicScriptOwnerId,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    ) {
        let script = evaluation.script_continuation.script.clone();
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::ReadyModuleScriptEvaluation {
                order: 0,
                evaluation,
            },
        };
        self.push_entry_front(entry);
    }

    #[cfg(test)]
    pub(super) fn requeue_failed_script_front(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
        source_network_result: Option<SharedNavigationResponseResult>,
    ) {
        self.requeue_failed_script_front_with_error_constructor(
            id,
            script,
            message,
            kind,
            module_failure_policy,
            source_network_result,
            error_constructor,
        );
    }

    pub(super) fn requeue_failed_script_front_with_error_constructor(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        message: String,
        kind: DynamicScriptFailureKind,
        module_failure_policy: Option<ModuleFailurePolicy>,
        source_network_result: Option<SharedNavigationResponseResult>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) {
        let failure = DynamicScriptFailure::with_kind(message, kind, module_failure_policy)
            .with_source_network_result(source_network_result)
            .with_error_constructor(error_constructor);
        self.requeue_script_failure_front(id, script, failure);
    }

    fn requeue_script_failure_front(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        failure: DynamicScriptFailure,
    ) {
        let _ = self.remove_entry_by_id(id);
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::Failed { failure, order: 0 },
        };
        self.push_entry_front(entry);
    }

    #[cfg(test)]
    pub(super) fn enqueue_loading_script_for_test(&mut self, script: PreparedScript) {
        let id = DynamicScriptOwnerId::new(self.next_id);
        self.next_id += 1;
        self.in_flight_loads += 1;
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::Loading,
        };
        match entry.script.mode {
            super::types::ScriptMode::InOrder => self.script_lanes.in_order.push_back(entry),
            super::types::ScriptMode::ImportMapInOrder => {
                self.script_lanes.importmap_in_order.push_back(entry)
            }
            super::types::ScriptMode::ModuleInOrder => {
                self.script_lanes.module_in_order.push_back(entry)
            }
            super::types::ScriptMode::Async => self.script_lanes.async_scripts.push_back(entry),
            super::types::ScriptMode::Normal
            | super::types::ScriptMode::Defer
            | super::types::ScriptMode::ModuleDefer => {
                unreachable!("test helper only supports dynamic-owner script modes")
            }
        }
    }

    /// Seed one already-ready runtime script with the same exact lifecycle
    /// capability required by production admission.
    ///
    /// This is deliberately not a parallel executor: selected Page-task tests
    /// use it only as producer state, then prove that the production callback
    /// completion publishes a typed runtime continuation. Tests that need an
    /// unresolved producer use `enqueue_loading_script_for_test` instead.
    #[cfg(test)]
    pub(super) fn enqueue_ready_script_with_load_delay_for_test(
        &mut self,
        script: PreparedScript,
        load_delay_binding: MainDocumentScriptLoadDelayLease,
    ) {
        let id = self.reserve_script_id();
        let queue_kind = DynamicScriptQueueKind::for_mode(script.mode);
        let ready_state = DynamicScriptReadyState::Ready {
            order: self.next_ready_order(),
            source_network_result: None,
        };
        self.insert_script_with_ready_state(
            id,
            queue_kind,
            script,
            ready_state,
            Some(load_delay_binding),
        );
        self.refresh_followup_work();
    }

    #[cfg(test)]
    pub(crate) fn pending_source_load_count_for_test(&self) -> usize {
        self.in_flight_loads
    }

    pub(super) fn is_idle(&mut self) -> bool {
        self.in_flight_loads == 0 && self.script_lanes.is_empty() && self.followup_work.is_empty()
    }

    pub(super) fn has_only_scripts_matching(
        &mut self,
        mut predicate: impl FnMut(&PreparedScript) -> bool,
    ) -> bool {
        self.promote_ready_work();

        self.script_lanes
            .in_order
            .iter()
            .all(|entry| predicate(&entry.script))
            && self
                .script_lanes
                .importmap_in_order
                .iter()
                .all(|entry| predicate(&entry.script))
            && self
                .script_lanes
                .module_in_order
                .iter()
                .all(|entry| predicate(&entry.script))
            && self
                .script_lanes
                .async_scripts
                .iter()
                .all(|entry| predicate(&entry.script))
            && self.followup_work.with_tasks_mut(|tasks| {
                tasks.iter().all(|task| match task {
                    DynamicScriptRunnable::Execute { script, .. }
                    | DynamicScriptRunnable::DispatchError { script, .. } => predicate(script),
                    DynamicScriptRunnable::ContinueModuleScriptGraph { continuation, .. } => {
                        predicate(&continuation.script)
                    }
                    DynamicScriptRunnable::ContinueModuleScriptEvaluation {
                        evaluation, ..
                    } => predicate(&evaluation.script_continuation.script),
                })
            })
    }

    pub(super) fn has_script_matching(
        &mut self,
        mut predicate: impl FnMut(&PreparedScript) -> bool,
    ) -> bool {
        self.promote_ready_work();

        self.script_lanes
            .in_order
            .iter()
            .any(|entry| predicate(&entry.script))
            || self
                .script_lanes
                .importmap_in_order
                .iter()
                .any(|entry| predicate(&entry.script))
            || self
                .script_lanes
                .module_in_order
                .iter()
                .any(|entry| predicate(&entry.script))
            || self
                .script_lanes
                .async_scripts
                .iter()
                .any(|entry| predicate(&entry.script))
            || self.followup_work.with_tasks_mut(|tasks| {
                tasks.iter().any(|task| match task {
                    DynamicScriptRunnable::Execute { script, .. }
                    | DynamicScriptRunnable::DispatchError { script, .. } => predicate(script),
                    DynamicScriptRunnable::ContinueModuleScriptGraph { continuation, .. } => {
                        predicate(&continuation.script)
                    }
                    DynamicScriptRunnable::ContinueModuleScriptEvaluation {
                        evaluation, ..
                    } => predicate(&evaluation.script_continuation.script),
                })
            })
    }

    fn enqueue_script_with_id(
        &mut self,
        loader: &ResourceRequestClient,
        task_runner: RendererResourceTaskRunner,
        id: DynamicScriptOwnerId,
        queue_kind: DynamicScriptQueueKind,
        script: PreparedScript,
        document_character_set: Option<&str>,
        service_worker_context: Option<&DynamicScriptServiceWorkerContext>,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) {
        let ready_state = match script.source_kind {
            ScriptSourceKind::External => {
                let tx = self.owner_event_sender();
                let loader = loader.clone();
                let script_for_load = script.clone();
                let document_character_set = document_character_set.map(str::to_owned);
                let service_worker_context = service_worker_context.cloned();
                let fetch_task_runner = task_runner.clone();
                self.in_flight_loads += 1;
                task_runner.spawn(async move {
                    let outcome = if let Some(context) = service_worker_context {
                        load_service_worker_aware_external_script_source_outcome(
                            &script_for_load,
                            &loader,
                            fetch_task_runner,
                            document_character_set.as_deref(),
                            None,
                            context.browser_context_runtime,
                            context.client_id,
                            context.document_url,
                        )
                        .await
                    } else {
                        load_prepared_script_source_outcome_with_document_character_set(
                            &script_for_load,
                            &loader,
                            document_character_set.as_deref(),
                            None,
                        )
                        .await
                    };
                    let _ = tx.send(DynamicScriptOwnerEvent::Completion(
                        DynamicScriptLoadCompletion { id, outcome },
                    ));
                });
                DynamicScriptReadyState::Loading
            }
            ScriptSourceKind::Inline => DynamicScriptReadyState::Ready {
                order: self.next_ready_order(),
                source_network_result: None,
            },
        };

        self.insert_script_with_ready_state(
            id,
            queue_kind,
            script,
            ready_state,
            load_delay_binding,
        );
    }

    #[cfg(test)]
    fn enqueue_inline_script_with_id_for_test(
        &mut self,
        id: DynamicScriptOwnerId,
        queue_kind: DynamicScriptQueueKind,
        script: PreparedScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) {
        assert_eq!(
            script.source_kind,
            ScriptSourceKind::Inline,
            "inline-only DynamicScriptOwner tests must inject external load completions explicitly"
        );
        let ready_state = DynamicScriptReadyState::Ready {
            order: self.next_ready_order(),
            source_network_result: None,
        };
        self.insert_script_with_ready_state(
            id,
            queue_kind,
            script,
            ready_state,
            load_delay_binding,
        );
    }

    fn insert_script_with_ready_state(
        &mut self,
        id: DynamicScriptOwnerId,
        queue_kind: DynamicScriptQueueKind,
        script: PreparedScript,
        ready_state: DynamicScriptReadyState,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) {
        if let Some(binding) = load_delay_binding {
            let previous = self.load_delay_bindings.insert(id, binding);
            debug_assert!(
                previous.is_none(),
                "new dynamic script owner id must not replace a lifecycle binding"
            );
        }

        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state,
        };
        match queue_kind {
            DynamicScriptQueueKind::InOrder => self.script_lanes.in_order.push_back(entry),
            DynamicScriptQueueKind::ImportMapInOrder => {
                self.script_lanes.importmap_in_order.push_back(entry)
            }
            DynamicScriptQueueKind::ModuleInOrder => {
                self.script_lanes.module_in_order.push_back(entry)
            }
            DynamicScriptQueueKind::Async => self.script_lanes.async_scripts.push_back(entry),
        }
    }

    fn enqueue_failed_script_with_binding(
        &mut self,
        id: DynamicScriptOwnerId,
        failed: FailedDynamicScript,
        load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
    ) {
        if let Some(binding) = load_delay_binding {
            let previous = self.load_delay_bindings.insert(id, binding);
            debug_assert!(
                previous.is_none(),
                "new failed dynamic script owner id must not replace a lifecycle binding"
            );
        }
        self.enqueue_failed_script_with_id(id, failed);
    }

    fn reserve_script_id(&mut self) -> DynamicScriptOwnerId {
        let id = DynamicScriptOwnerId::new(self.next_id);
        self.next_id += 1;
        id
    }

    fn enqueue_failed_script_with_id(
        &mut self,
        id: DynamicScriptOwnerId,
        failed: FailedDynamicScript,
    ) {
        let failure =
            Self::queued_script_failure(failed.failure_kind, &failed.script, failed.message);
        self.enqueue_script_failure_with_id(id, failed.script, failure);
    }

    fn queued_script_failure(
        failure_kind: QueuedScriptFailureKind,
        script: &PreparedScript,
        message: String,
    ) -> DynamicScriptFailure {
        match failure_kind {
            QueuedScriptFailureKind::Immediate => {
                DynamicScriptFailure::with_kind(message, DynamicScriptFailureKind::Immediate, None)
            }
            QueuedScriptFailureKind::ModuleTopLevelLoad => DynamicScriptFailure::with_kind(
                message,
                Self::module_load_failure_kind(
                    script,
                    crate::module_runtime::ModuleLoadStage::Fetch,
                ),
                Some(ModuleFailurePolicy::TopLevelLoadFailure),
            ),
        }
    }

    fn enqueue_script_failure_with_id(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        failure: DynamicScriptFailure,
    ) {
        let entry = DynamicScriptEntry {
            id,
            script,
            ready_state: DynamicScriptReadyState::Failed {
                failure,
                order: self.next_ready_order(),
            },
        };
        match entry.script.mode {
            super::types::ScriptMode::InOrder => self.script_lanes.in_order.push_back(entry),
            super::types::ScriptMode::ImportMapInOrder => {
                self.script_lanes.importmap_in_order.push_back(entry)
            }
            super::types::ScriptMode::ModuleInOrder => {
                self.script_lanes.module_in_order.push_back(entry)
            }
            super::types::ScriptMode::Async => self.script_lanes.async_scripts.push_back(entry),
            super::types::ScriptMode::Normal
            | super::types::ScriptMode::Defer
            | super::types::ScriptMode::ModuleDefer => {
                unreachable!("failed dynamic scripts must stay on dynamic owner lanes")
            }
        }
    }

    fn note_script_failed_in_queue_or_enqueue(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        failure: DynamicScriptFailure,
    ) {
        let order = self.next_ready_order;
        if self
            .update_entry_state_by_id(
                id,
                DynamicScriptReadyState::Failed {
                    failure: failure.clone(),
                    order,
                },
            )
            .is_ok()
        {
            self.next_ready_order += 1;
            return;
        }
        self.enqueue_script_failure_with_id(id, script, failure);
    }

    pub(super) fn legacy_message_failure_kind(
        script: &PreparedScript,
        message: &str,
    ) -> DynamicScriptFailureKind {
        let _ = (script, message);
        DynamicScriptFailureKind::Immediate
    }

    pub(super) fn module_load_failure_kind(
        script: &PreparedScript,
        stage: crate::module_runtime::ModuleLoadStage,
    ) -> DynamicScriptFailureKind {
        if !matches!(
            script.mode,
            super::types::ScriptMode::Async | super::types::ScriptMode::ModuleInOrder
        ) || script.kind != ScriptKind::Module
        {
            return DynamicScriptFailureKind::Immediate;
        }

        match stage {
            crate::module_runtime::ModuleLoadStage::Fetch => DynamicScriptFailureKind::ModuleFetch,
            crate::module_runtime::ModuleLoadStage::Resolve => {
                DynamicScriptFailureKind::ModuleResolve
            }
            crate::module_runtime::ModuleLoadStage::Instantiate => {
                DynamicScriptFailureKind::ModuleInstantiate
            }
            crate::module_runtime::ModuleLoadStage::Compile
            | crate::module_runtime::ModuleLoadStage::Evaluate => {
                DynamicScriptFailureKind::Immediate
            }
        }
    }

    fn owner_event_sender(&self) -> DynamicScriptOwnerEventSender {
        DynamicScriptOwnerEventSender {
            tx: self.owner_event_tx.clone(),
            continuation_tx: Arc::clone(&self.continuation_tx),
            continuation_turn_queued: Arc::clone(&self.continuation_turn_queued),
        }
    }

    pub(super) fn enable_continuation_enqueue(
        &mut self,
        sender: MainDocumentRuntimeContinuationSender,
    ) {
        *self.continuation_tx.lock() = Some(sender);
    }

    pub(super) fn disable_continuation_enqueue(&mut self) {
        *self.continuation_tx.lock() = None;
        self.continuation_turn_queued
            .store(false, Ordering::Release);
    }

    pub(super) fn begin_continuation_turn(&mut self) {
        self.continuation_turn_queued
            .store(false, Ordering::Release);
    }

    fn push_entry_front(&mut self, entry: DynamicScriptEntry) {
        match entry.script.mode {
            super::types::ScriptMode::InOrder => self.script_lanes.in_order.push_front(entry),
            super::types::ScriptMode::ImportMapInOrder => {
                self.script_lanes.importmap_in_order.push_front(entry)
            }
            super::types::ScriptMode::ModuleInOrder => {
                self.script_lanes.module_in_order.push_front(entry)
            }
            super::types::ScriptMode::Async => self.script_lanes.async_scripts.push_front(entry),
            super::types::ScriptMode::Normal
            | super::types::ScriptMode::Defer
            | super::types::ScriptMode::ModuleDefer => {
                unreachable!("requeued dynamic scripts must stay on dynamic owner lanes")
            }
        }
    }

    fn update_entry_state_by_id(
        &mut self,
        id: DynamicScriptOwnerId,
        ready_state: DynamicScriptReadyState,
    ) -> Result<(), Box<DynamicScriptReadyState>> {
        let mut ready_state = Some(ready_state);
        for queue in [
            &mut self.script_lanes.in_order,
            &mut self.script_lanes.importmap_in_order,
            &mut self.script_lanes.module_in_order,
            &mut self.script_lanes.async_scripts,
        ] {
            if let Some(entry) = queue.iter_mut().find(|entry| entry.id == id) {
                entry.ready_state = ready_state
                    .take()
                    .expect("dynamic ready state should be available");
                return Ok(());
            }
        }
        Err(Box::new(
            ready_state.expect("dynamic ready state should be available"),
        ))
    }

    fn remove_entry_by_id(&mut self, id: DynamicScriptOwnerId) -> Option<DynamicScriptEntry> {
        for queue in [
            &mut self.script_lanes.in_order,
            &mut self.script_lanes.importmap_in_order,
            &mut self.script_lanes.module_in_order,
            &mut self.script_lanes.async_scripts,
        ] {
            if let Some(index) = queue.iter().position(|entry| entry.id == id) {
                return queue.remove(index);
            }
        }
        None
    }

    fn take_module_script_graph_pending_fetch_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        load_id: u64,
    ) -> Option<Box<ModuleScriptContinuation>> {
        let index = queue.iter().position(|entry| {
            matches!(
                &entry.ready_state,
                DynamicScriptReadyState::SuspendedModuleScriptGraph {
                    wait: DynamicModuleScriptGraphWait::PendingFetch { load_ids, .. },
                    continuation,
                } if continuation.is_some() && load_ids.contains(&load_id)
            )
        })?;
        let (continuation, should_remove) = {
            let entry = queue
                .get_mut(index)
                .expect("matched pending fetch entry should exist");
            let DynamicScriptReadyState::SuspendedModuleScriptGraph {
                wait:
                    DynamicModuleScriptGraphWait::PendingFetch {
                        load_ids,
                        joined_clients,
                    },
                continuation,
            } = &mut entry.ready_state
            else {
                unreachable!("matched pending fetch state above")
            };
            let position = load_ids
                .iter()
                .position(|current| *current == load_id)
                .expect("matched pending fetch load id should exist");
            load_ids.remove(position);
            let continuation = continuation
                .take()
                .expect("matched pending fetch continuation should exist");
            (
                continuation,
                load_ids.is_empty() && joined_clients.is_empty(),
            )
        };
        if should_remove {
            let _ = queue.remove(index);
        };
        Some(continuation)
    }

    fn take_module_script_graph_pending_joined_client_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<Box<ModuleScriptContinuation>> {
        let index = queue.iter().position(|entry| {
            matches!(
                &entry.ready_state,
                DynamicScriptReadyState::SuspendedModuleScriptGraph {
                    wait: DynamicModuleScriptGraphWait::PendingFetch { joined_clients, .. },
                    continuation,
                } if continuation.is_some() && joined_clients.contains(&client)
            )
        })?;
        let (continuation, should_remove) = {
            let entry = queue
                .get_mut(index)
                .expect("matched pending joined fetch entry should exist");
            let DynamicScriptReadyState::SuspendedModuleScriptGraph {
                wait:
                    DynamicModuleScriptGraphWait::PendingFetch {
                        load_ids,
                        joined_clients,
                    },
                continuation,
            } = &mut entry.ready_state
            else {
                unreachable!("matched pending joined fetch state above")
            };
            let position = joined_clients
                .iter()
                .position(|current| *current == client)
                .expect("matched pending joined fetch client should exist");
            joined_clients.remove(position);
            let continuation = continuation
                .take()
                .expect("matched pending joined fetch continuation should exist");
            (
                continuation,
                load_ids.is_empty() && joined_clients.is_empty(),
            )
        };
        if should_remove {
            let _ = queue.remove(index);
        };
        Some(continuation)
    }

    fn clear_module_script_graph_pending_waits_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        id: DynamicScriptOwnerId,
    ) -> Option<(Vec<u64>, Vec<module_tree::SingleModuleClientToken>)> {
        let entry = queue.iter_mut().find(|entry| entry.id == id)?;
        let DynamicScriptReadyState::SuspendedModuleScriptGraph {
            wait:
                DynamicModuleScriptGraphWait::PendingFetch {
                    load_ids,
                    joined_clients,
                },
            continuation,
        } = &mut entry.ready_state
        else {
            return None;
        };
        *continuation = None;
        Some((std::mem::take(load_ids), std::mem::take(joined_clients)))
    }

    fn restore_followup_task_front(&mut self, task: DynamicScriptRunnable) {
        match task {
            DynamicScriptRunnable::Execute {
                id,
                script,
                source_network_result,
            } => self.requeue_ready_script_front(id, script, source_network_result),
            DynamicScriptRunnable::ContinueModuleScriptGraph { id, continuation } => {
                self.requeue_module_script_graph_ready_front(id, continuation)
            }
            DynamicScriptRunnable::ContinueModuleScriptEvaluation { id, evaluation } => {
                self.requeue_module_script_evaluation_ready_front(id, evaluation)
            }
            DynamicScriptRunnable::DispatchError {
                id,
                script,
                message,
                kind,
                module_failure_policy,
                source_network_result,
                error_constructor,
            } => {
                let failure = DynamicScriptFailure::with_kind(message, kind, module_failure_policy)
                    .with_source_network_result(source_network_result)
                    .with_error_constructor(error_constructor);
                if failure.is_deferrable_module() {
                    self.enqueue_script_failure_with_id(id, script, failure);
                    return;
                }
                self.requeue_script_failure_front(id, script, failure);
            }
        }
    }

    fn refresh_followup_work(&mut self) {
        self.refresh_followup_work_matching(&mut |_| true);
    }

    fn refresh_followup_work_matching(
        &mut self,
        predicate: &mut impl FnMut(&PreparedScript) -> bool,
    ) {
        let existing = self.followup_work.pop_front_local_only();
        debug_assert!(
            self.followup_work.is_empty_local_only(),
            "dynamic followup work should only cache one runnable item"
        );
        if let Some(task) = existing {
            self.restore_followup_task_front(task);
        }
        self.promote_ready_work_matching(predicate);
    }

    pub(super) fn apply_owner_event(&mut self, event: DynamicScriptOwnerEvent) {
        match event {
            DynamicScriptOwnerEvent::Completion(completion) => {
                self.in_flight_loads = self.in_flight_loads.saturating_sub(1);
                self.apply_completion(completion);
                self.refresh_followup_work();
            }
        }
    }

    fn apply_completion(&mut self, completion: DynamicScriptLoadCompletion) {
        let order = self.next_ready_order;
        if Self::apply_completion_to_queue(&mut self.script_lanes.in_order, &completion, order) {
            self.next_ready_order += 1;
            return;
        }
        if Self::apply_completion_to_queue(
            &mut self.script_lanes.importmap_in_order,
            &completion,
            order,
        ) {
            self.next_ready_order += 1;
            return;
        }
        if Self::apply_completion_to_queue(
            &mut self.script_lanes.module_in_order,
            &completion,
            order,
        ) {
            self.next_ready_order += 1;
            return;
        }
        if Self::apply_completion_to_queue(&mut self.script_lanes.async_scripts, &completion, order)
        {
            self.next_ready_order += 1;
        }
    }

    fn promote_ready_work(&mut self) {
        self.promote_ready_work_matching(&mut |_| true);
    }

    fn promote_ready_work_matching(&mut self, predicate: &mut impl FnMut(&PreparedScript) -> bool) {
        if !self.followup_work.is_empty() {
            return;
        }

        let entry = self.script_lanes.take_next_eligible(predicate);
        if let Some(work) = Self::take_entry(entry) {
            self.followup_work.enqueue_local(work);
        }
    }

    fn mark_module_script_evaluation_reaction(
        &mut self,
        reaction_id: u64,
        reaction_state: ModuleScriptEvaluationReactionState,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        for queue in [
            &mut self.script_lanes.in_order,
            &mut self.script_lanes.importmap_in_order,
            &mut self.script_lanes.module_in_order,
            &mut self.script_lanes.async_scripts,
        ] {
            if let Some(update) = Self::mark_module_script_evaluation_reaction_in_queue(
                queue,
                reaction_id,
                reaction_state.clone(),
                &mut self.next_ready_order,
            ) {
                return Some(update);
            }
        }
        None
    }

    fn mark_module_script_evaluation_reaction_in_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        reaction_id: u64,
        reaction_state: ModuleScriptEvaluationReactionState,
        next_ready_order: &mut u64,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        for entry in queue {
            let matches_reaction = match &entry.ready_state {
                DynamicScriptReadyState::SuspendedModuleScriptEvaluation { evaluation } => {
                    evaluation.reaction_id == reaction_id
                }
                _ => false,
            };
            if !matches_reaction {
                continue;
            }
            let old_state =
                std::mem::replace(&mut entry.ready_state, DynamicScriptReadyState::Loading);
            let DynamicScriptReadyState::SuspendedModuleScriptEvaluation { mut evaluation } =
                old_state
            else {
                unreachable!("checked suspended module evaluation state above")
            };
            evaluation.reaction_state = reaction_state;
            let update = ModuleScriptEvaluationUpdate {
                root_entry: evaluation.root_entry,
            };
            entry.ready_state = DynamicScriptReadyState::ReadyModuleScriptEvaluation {
                order: *next_ready_order,
                evaluation,
            };
            *next_ready_order += 1;
            return Some(update);
        }
        None
    }

    fn apply_completion_to_queue(
        queue: &mut VecDeque<DynamicScriptEntry>,
        completion: &DynamicScriptLoadCompletion,
        order: u64,
    ) -> bool {
        let Some(entry) = queue.iter_mut().find(|entry| entry.id == completion.id) else {
            return false;
        };
        match completion.outcome.source_result.clone() {
            Ok(source) => {
                entry.script = prepared_script_with_loaded_source(
                    entry.script.clone(),
                    source,
                    completion.outcome.source_bytes.clone(),
                );
                entry.ready_state = DynamicScriptReadyState::Ready {
                    order,
                    source_network_result: completion.outcome.network_result.clone(),
                };
            }
            Err(message) => {
                let failure = DynamicScriptFailure::with_kind(
                    message.clone(),
                    Self::module_load_failure_kind(
                        &entry.script,
                        crate::module_runtime::ModuleLoadStage::Fetch,
                    ),
                    Some(ModuleFailurePolicy::TopLevelLoadFailure),
                )
                .with_source_network_result(completion.outcome.network_result.clone());
                entry.ready_state = DynamicScriptReadyState::Failed { failure, order };
            }
        }
        true
    }

    fn take_entry(entry: Option<DynamicScriptEntry>) -> Option<DynamicScriptRunnable> {
        let entry = entry?;
        match entry.ready_state {
            DynamicScriptReadyState::Ready {
                source_network_result,
                ..
            } => Some(DynamicScriptRunnable::Execute {
                id: entry.id,
                script: entry.script,
                source_network_result,
            }),
            DynamicScriptReadyState::ReadyModuleScriptGraph { continuation, .. } => {
                Some(DynamicScriptRunnable::ContinueModuleScriptGraph {
                    id: entry.id,
                    continuation,
                })
            }
            DynamicScriptReadyState::ReadyModuleScriptEvaluation { evaluation, .. } => {
                Some(DynamicScriptRunnable::ContinueModuleScriptEvaluation {
                    id: entry.id,
                    evaluation,
                })
            }
            DynamicScriptReadyState::Failed { failure, .. } => {
                Some(DynamicScriptRunnable::DispatchError {
                    id: entry.id,
                    script: entry.script,
                    message: failure.message,
                    kind: failure.kind,
                    module_failure_policy: failure.module_failure_policy,
                    source_network_result: failure.source_network_result,
                    error_constructor: failure.error_constructor,
                })
            }
            DynamicScriptReadyState::Loading
            | DynamicScriptReadyState::SuspendedModuleScriptGraph { .. }
            | DynamicScriptReadyState::SuspendedModuleScriptEvaluation { .. } => None,
        }
    }

    fn next_ready_order(&mut self) -> u64 {
        let order = self.next_ready_order;
        self.next_ready_order += 1;
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        planning::ScriptSource,
        types::{ScriptKind, ScriptMode},
    };
    use url::Url;

    fn prepared_script(position: usize, mode: ScriptMode) -> PreparedScript {
        PreparedScript {
            position,
            node_id: crate::dom::NodeId::new(position + 1),
            kind: match mode {
                ScriptMode::ModuleInOrder => ScriptKind::Module,
                ScriptMode::ImportMapInOrder => ScriptKind::ImportMap,
                _ => ScriptKind::Classic,
            },
            mode,
            source_kind: ScriptSourceKind::Inline,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::Inline(String::new()),
            initiator_url: Url::parse("https://example.test/script.js")
                .expect("test url should parse"),
            base_url: Url::parse("https://example.test/script.js").expect("test url should parse"),
            url: Url::parse("https://example.test/script.js").expect("test url should parse"),
            host_script_handle: Some(format!("script-{position}")),
        }
    }

    fn prepared_module_script(position: usize, mode: ScriptMode) -> PreparedScript {
        let mut script = prepared_script(position, mode);
        script.kind = ScriptKind::Module;
        script
    }

    fn owner_id(raw: u64) -> DynamicScriptOwnerId {
        DynamicScriptOwnerId::from_u64(raw)
    }

    fn runtime_module_continuation(
        id: u64,
        position: usize,
        mode: ScriptMode,
    ) -> Box<ModuleScriptContinuation> {
        Box::new(ModuleScriptContinuation::new_runtime(
            prepared_module_script(position, mode),
            owner_id(id),
            0,
        ))
    }

    fn joined_module_client(sequence: u64) -> module_tree::SingleModuleClientToken {
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(1),
            sequence,
        }
    }

    fn runtime_module_evaluation(
        id: u64,
        position: usize,
        mode: ScriptMode,
    ) -> Box<ModuleScriptEvaluationContinuation> {
        Box::new(ModuleScriptEvaluationContinuation {
            script_continuation: *runtime_module_continuation(id, position, mode),
            root_entry: crate::module_runtime::ModuleEntryId::for_test(id as u32),
            reaction_id: id,
            reaction_state: ModuleScriptEvaluationReactionState::Pending,
            completion_applied_at_evaluation_start: false,
        })
    }

    fn loading_entry(id: u64, position: usize, mode: ScriptMode) -> DynamicScriptEntry {
        let mut script = prepared_script(position, mode);
        script.source_kind = ScriptSourceKind::External;
        script.source = ScriptSource::External;
        DynamicScriptEntry {
            id: owner_id(id),
            script,
            ready_state: DynamicScriptReadyState::Loading,
        }
    }

    fn load_completion_ok(id: u64, source: impl Into<String>) -> DynamicScriptLoadCompletion {
        DynamicScriptLoadCompletion {
            id: owner_id(id),
            outcome: PreparedScriptSourceLoadOutcome {
                source_result: Ok(source.into()),
                source_bytes: None,
                network_result: None,
            },
        }
    }

    fn load_completion_err(id: u64, error: impl Into<String>) -> DynamicScriptLoadCompletion {
        DynamicScriptLoadCompletion {
            id: owner_id(id),
            outcome: PreparedScriptSourceLoadOutcome {
                source_result: Err(error.into()),
                source_bytes: None,
                network_result: None,
            },
        }
    }

    #[test]
    fn suspended_module_script_graph_keeps_owner_non_idle_without_runnable_work() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(!owner.is_idle());
        assert!(!owner.has_immediately_runnable_work());
        assert!(matches!(
            owner.poll_nonblocking(),
            DynamicScriptOwnerPoll::StalledWithoutInflightLoads
        ));
    }

    #[test]
    fn ready_module_script_graph_surfaces_continue_work_with_owner_id() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.note_module_script_graph_ready(
            owner_id(7),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder)
        ));
        let Some(DynamicScriptRunnable::ContinueModuleScriptGraph { id, continuation }) =
            owner.next_runnable_script()
        else {
            panic!("expected ready module graph continuation work");
        };
        assert_eq!(id, owner_id(7));
        assert_eq!(continuation.script.kind, ScriptKind::Module);
    }

    #[test]
    fn module_continuation_readiness_is_exact_and_non_consuming() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([prepared_script(0, ScriptMode::Async)]),
            failed_scripts: VecDeque::new(),
        });

        assert!(owner.has_immediately_runnable_work());
        assert!(
            !owner.has_ready_module_script_continuation(),
            "a runnable classic script must not masquerade as module owner work"
        );

        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );
        assert!(owner.note_module_script_graph_ready(
            owner_id(7),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        ));

        assert!(owner.has_ready_module_script_continuation());
        assert!(owner.has_ready_module_script_continuation());
        assert!(matches!(
            owner.next_runnable_script(),
            Some(DynamicScriptRunnable::ContinueModuleScriptGraph { id, .. })
                if id == owner_id(7)
        ));
    }

    #[test]
    fn suspended_module_script_graph_pending_fetch_payload_is_owned_by_owner() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.has_pending_module_script_graph());
        assert!(owner.take_module_script_graph_pending_fetch(71).is_none());
        let continuation = owner
            .take_module_script_graph_pending_fetch(70)
            .expect("expected owner-owned pending graph continuation");
        assert_eq!(continuation.dynamic_script_owner_id(), Some(owner_id(7)));
        assert!(!owner.has_pending_module_script_graph());
    }

    #[test]
    fn suspended_module_script_graph_pending_fetches_share_one_continuation() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70, 71],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        let continuation = owner
            .take_module_script_graph_pending_fetch(70)
            .expect("first load id should take the active continuation");
        assert!(
            owner.has_pending_module_script_graph(),
            "remaining load id should keep the owner graph pending"
        );
        assert!(owner.take_module_script_graph_pending_fetch(71).is_none());
        assert!(
            owner.restore_module_script_graph_pending_continuation(
                owner_id(7),
                Box::new(continuation)
            )
        );
        let continuation = owner
            .take_module_script_graph_pending_fetch(71)
            .expect("restored continuation should be available for the remaining load id");
        assert_eq!(continuation.dynamic_script_owner_id(), Some(owner_id(7)));
        assert!(!owner.has_pending_module_script_graph());
    }

    #[test]
    fn pending_module_fetch_owner_query_is_exact_and_non_consuming() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![70, 71],
            Vec::new(),
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );
        owner.note_module_script_graph_fetch_suspended(
            owner_id(8),
            vec![80],
            Vec::new(),
            runtime_module_continuation(8, 1, ScriptMode::Async),
        );

        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(70),
            Some(owner_id(7))
        );
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(71),
            Some(owner_id(7))
        );
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(80),
            Some(owner_id(8))
        );
        assert_eq!(owner.module_script_owner_id_for_pending_fetch(81), None);
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(70),
            Some(owner_id(7)),
            "currentness lookup must not consume the exact pending fetch"
        );

        let continuation = owner
            .take_module_script_graph_pending_fetch(70)
            .expect("the queried fetch must remain consumable");
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(71),
            None,
            "a sibling fetch cannot be authorized while the shared continuation is checked out"
        );
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(80),
            Some(owner_id(8)),
            "checking out one owner must not hide another owner's pending fetch"
        );
        assert!(
            owner.restore_module_script_graph_pending_continuation(
                owner_id(7),
                Box::new(continuation)
            )
        );
        assert_eq!(
            owner.module_script_owner_id_for_pending_fetch(71),
            Some(owner_id(7))
        );
    }

    #[test]
    fn suspended_module_script_graph_joined_clients_share_one_continuation() {
        let mut owner = DynamicScriptOwner::default();
        let first_client = joined_module_client(70);
        let second_client = joined_module_client(71);
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            Vec::new(),
            vec![first_client, second_client],
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.has_pending_module_script_graph());
        let continuation = owner
            .take_module_script_graph_pending_joined_client(first_client)
            .expect("first joined client should take the active continuation");
        assert!(
            owner.has_pending_module_script_graph(),
            "remaining joined client should keep the owner graph pending"
        );
        assert!(
            owner
                .take_module_script_graph_pending_joined_client(second_client)
                .is_none()
        );
        assert!(
            owner.restore_module_script_graph_pending_continuation(
                owner_id(7),
                Box::new(continuation)
            )
        );
        let continuation = owner
            .take_module_script_graph_pending_joined_client(second_client)
            .expect("restored continuation should be available for remaining joined client");
        assert_eq!(continuation.dynamic_script_owner_id(), Some(owner_id(7)));
        assert!(!owner.has_pending_module_script_graph());
    }

    #[test]
    fn clear_module_script_graph_pending_waits_detaches_runtime_owner_wait_set() {
        let mut owner = DynamicScriptOwner::default();
        let first_client = joined_module_client(70);
        let second_client = joined_module_client(71);
        owner.note_module_script_graph_fetch_suspended(
            owner_id(7),
            vec![80, 81],
            vec![first_client, second_client],
            runtime_module_continuation(7, 0, ScriptMode::ModuleInOrder),
        );

        let (load_ids, joined_clients) = owner.clear_module_script_graph_pending_waits(owner_id(7));
        assert_eq!(load_ids, vec![80, 81]);
        assert_eq!(joined_clients, vec![first_client, second_client]);
        assert!(
            owner.take_module_script_graph_pending_fetch(80).is_none(),
            "cleared load id should no longer recover a graph continuation"
        );
        assert!(
            owner
                .take_module_script_graph_pending_joined_client(first_client)
                .is_none(),
            "cleared joined client should no longer recover a graph continuation"
        );
    }

    #[test]
    fn failed_suspended_module_script_graph_does_not_block_later_in_order_module() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::from([
                prepared_module_script(0, ScriptMode::ModuleInOrder),
                prepared_module_script(1, ScriptMode::ModuleInOrder),
            ]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });

        let Some(DynamicScriptRunnable::Execute {
            id: first_id,
            script: first_script,
            ..
        }) = owner.next_runnable_script()
        else {
            panic!("first module should execute");
        };
        owner.note_module_script_graph_fetch_suspended(
            first_id,
            vec![80],
            Vec::new(),
            runtime_module_continuation(first_id.0, 0, ScriptMode::ModuleInOrder),
        );
        let (_load_ids, _joined_clients) = owner.clear_module_script_graph_pending_waits(first_id);
        owner.requeue_failed_script_front(
            first_id,
            first_script,
            "graph failed".to_owned(),
            DynamicScriptFailureKind::ModuleFetch,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
            None,
        );

        assert!(
            owner.script_lanes.module_in_order.iter().all(|entry| {
                entry.id != first_id
                    || !matches!(
                        entry.ready_state,
                        DynamicScriptReadyState::SuspendedModuleScriptGraph { .. }
                    )
            }),
            "failed graph owner entry must not leave a stale suspended graph entry behind"
        );
        let Some(DynamicScriptRunnable::Execute {
            id: second_id,
            script,
            ..
        }) = owner.next_runnable_script()
        else {
            panic!("later in-order module should not be blocked by a cleared graph wait");
        };
        assert_ne!(second_id, first_id);
        assert_eq!(script.position, 1);
        assert_eq!(script.mode, ScriptMode::ModuleInOrder);
    }

    #[test]
    fn suspended_module_script_evaluation_keeps_owner_non_idle_without_runnable_work() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_evaluation_suspended(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(!owner.is_idle());
        assert!(!owner.has_immediately_runnable_work());
        assert!(matches!(
            owner.poll_nonblocking(),
            DynamicScriptOwnerPoll::StalledWithoutInflightLoads
        ));
    }

    #[test]
    fn ready_module_script_evaluation_surfaces_continue_work_with_owner_id() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_evaluation_suspended(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.note_module_script_evaluation_ready(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder)
        ));
        let Some(DynamicScriptRunnable::ContinueModuleScriptEvaluation { id, evaluation }) =
            owner.next_runnable_script()
        else {
            panic!("expected ready module evaluation continuation work");
        };
        assert_eq!(id, owner_id(7));
        assert_eq!(
            evaluation.script_continuation.script.kind,
            ScriptKind::Module
        );
    }

    #[test]
    fn taking_module_continuation_preserves_non_module_runnable_work() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::from([prepared_script(0, ScriptMode::InOrder)]),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });

        assert!(owner.take_ready_module_script_continuation().is_none());
        let Some(DynamicScriptRunnable::Execute { id, .. }) = owner.next_runnable_script() else {
            panic!("expected preserved execute runnable");
        };
        assert_eq!(id, owner_id(0));
    }

    #[test]
    fn taking_module_continuation_returns_owner_selected_continuation_kind() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_evaluation_suspended(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.note_module_script_evaluation_ready(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder)
        ));
        let Some(work) = owner.take_ready_module_script_continuation() else {
            panic!("expected ready module evaluation continuation work");
        };
        let DynamicModuleScriptContinuationWork::Evaluation { evaluation } = work else {
            panic!("expected ready module evaluation continuation work");
        };
        assert_eq!(evaluation.reaction_id, 7);
    }

    #[test]
    fn taking_module_continuation_preserves_owner_order_between_graph_and_evaluation() {
        let mut owner = DynamicScriptOwner::default();
        owner.note_module_script_graph_fetch_suspended(
            owner_id(8),
            vec![80],
            Vec::new(),
            runtime_module_continuation(8, 1, ScriptMode::ModuleInOrder),
        );
        owner.note_module_script_evaluation_suspended(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder),
        );

        assert!(owner.note_module_script_evaluation_ready(
            owner_id(7),
            runtime_module_evaluation(7, 0, ScriptMode::ModuleInOrder)
        ));
        assert!(owner.note_module_script_graph_ready(
            owner_id(8),
            runtime_module_continuation(8, 1, ScriptMode::ModuleInOrder)
        ));
        let Some(work) = owner.take_ready_module_script_continuation() else {
            panic!("expected owner-selected module continuation work");
        };
        let DynamicModuleScriptContinuationWork::Evaluation { evaluation } = work else {
            panic!("expected ready module evaluation continuation work");
        };
        assert_eq!(evaluation.reaction_id, 7);
    }

    #[test]
    fn module_in_order_lane_respects_global_append_order() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::from([prepared_script(0, ScriptMode::InOrder)]),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::from([prepared_script(1, ScriptMode::ModuleInOrder)]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first script should be ready");
        let second = owner
            .next_runnable_script()
            .expect("second script should be ready");
        let DynamicScriptRunnable::Execute { script: first, .. } = first else {
            panic!("first item should execute");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second item should execute");
        };

        assert_eq!(first.mode, ScriptMode::InOrder);
        assert_eq!(second.mode, ScriptMode::ModuleInOrder);
    }

    #[test]
    fn ready_async_can_overtake_later_loading_in_order_front() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([prepared_script(0, ScriptMode::Async)]),
            failed_scripts: VecDeque::new(),
        });

        let next = owner
            .next_runnable_script()
            .expect("async script should be ready");
        let DynamicScriptRunnable::Execute { script: next, .. } = next else {
            panic!("async script should execute");
        };
        assert_eq!(next.mode, ScriptMode::Async);
    }

    #[test]
    fn requeued_execute_work_preserves_owner_entry_id() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([prepared_script(0, ScriptMode::Async)]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("async script should be ready");
        let DynamicScriptRunnable::Execute {
            id,
            script,
            source_network_result,
        } = first
        else {
            panic!("async script should execute");
        };
        owner.requeue_ready_script_front(id, script, source_network_result);

        let second = owner
            .next_runnable_script()
            .expect("requeued async script should be ready");
        let DynamicScriptRunnable::Execute {
            id: requeued_id, ..
        } = second
        else {
            panic!("requeued async script should execute");
        };
        assert_eq!(requeued_id, id);
    }

    #[test]
    fn importmap_lane_respects_global_append_order_against_module_lane() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::from([prepared_script(0, ScriptMode::ImportMapInOrder)]),
            module_in_order: VecDeque::from([prepared_script(1, ScriptMode::ModuleInOrder)]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("importmap should be ready");
        let second = owner
            .next_runnable_script()
            .expect("module should be ready");
        let DynamicScriptRunnable::Execute { script: first, .. } = first else {
            panic!("first item should execute");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second item should execute");
        };

        assert_eq!(first.mode, ScriptMode::ImportMapInOrder);
        assert_eq!(first.kind, ScriptKind::ImportMap);
        assert_eq!(second.mode, ScriptMode::ModuleInOrder);
        assert_eq!(second.kind, ScriptKind::Module);
    }

    #[test]
    fn failed_importmap_lane_dispatches_error_before_later_module() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::from([prepared_script(1, ScriptMode::ModuleInOrder)]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::from([FailedDynamicScript {
                script: prepared_script(0, ScriptMode::ImportMapInOrder),
                message: "unsupported".to_owned(),
                failure_kind: QueuedScriptFailureKind::Immediate,
            }]),
        });

        let first = owner
            .next_runnable_script()
            .expect("failed importmap should be surfaced first");
        let second = owner
            .next_runnable_script()
            .expect("module should still execute after failed importmap");
        let DynamicScriptRunnable::DispatchError {
            script, message, ..
        } = first
        else {
            panic!("first item should dispatch error");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second item should execute");
        };

        assert_eq!(script.mode, ScriptMode::ImportMapInOrder);
        assert_eq!(script.kind, ScriptKind::ImportMap);
        assert_eq!(message, "unsupported");
        assert_eq!(second.mode, ScriptMode::ModuleInOrder);
    }

    #[test]
    fn failed_importmap_preempts_later_async_module() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([prepared_module_script(1, ScriptMode::Async)]),
            failed_scripts: VecDeque::from([FailedDynamicScript {
                script: prepared_script(0, ScriptMode::ImportMapInOrder),
                message: "external import maps are not supported".to_owned(),
                failure_kind: QueuedScriptFailureKind::Immediate,
            }]),
        });

        let first = owner
            .next_runnable_script()
            .expect("failed importmap should be surfaced first");
        let second = owner
            .next_runnable_script()
            .expect("later async module should run after the importmap failure");
        let DynamicScriptRunnable::DispatchError {
            script, message, ..
        } = first
        else {
            panic!("first item should dispatch importmap error");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second item should execute later async module");
        };

        assert_eq!(script.mode, ScriptMode::ImportMapInOrder);
        assert_eq!(message, "external import maps are not supported");
        assert_eq!(second.mode, ScriptMode::Async);
        assert_eq!(second.kind, ScriptKind::Module);
    }

    #[test]
    fn newly_arrived_failed_importmap_preempts_cached_later_module() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::from([prepared_script(1, ScriptMode::ModuleInOrder)]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::from([FailedDynamicScript {
                script: prepared_script(0, ScriptMode::ImportMapInOrder),
                message: "unsupported".to_owned(),
                failure_kind: QueuedScriptFailureKind::Immediate,
            }]),
        });

        let first = owner
            .next_runnable_script()
            .expect("failed importmap should preempt cached later module");
        let second = owner
            .next_runnable_script()
            .expect("later module should remain queued behind the failure");
        let DynamicScriptRunnable::DispatchError {
            script, message, ..
        } = first
        else {
            panic!("first item should dispatch error");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second item should execute");
        };

        assert_eq!(script.mode, ScriptMode::ImportMapInOrder);
        assert_eq!(script.kind, ScriptKind::ImportMap);
        assert_eq!(message, "unsupported");
        assert_eq!(second.mode, ScriptMode::ModuleInOrder);
    }

    #[test]
    fn completion_promotes_front_in_order_and_following_global_front_chain() {
        let mut owner = DynamicScriptOwner::default();
        let later_module = prepared_script(1, ScriptMode::ModuleInOrder);
        owner
            .script_lanes
            .in_order
            .push_back(loading_entry(0, 0, ScriptMode::InOrder));
        owner
            .script_lanes
            .module_in_order
            .push_back(DynamicScriptEntry {
                id: owner_id(1),
                script: later_module.clone(),
                ready_state: DynamicScriptReadyState::Ready {
                    order: 0,
                    source_network_result: None,
                },
            });

        assert!(
            owner.next_runnable_script().is_none(),
            "loading in-order front should block later module until completion"
        );

        owner.apply_completion(load_completion_ok(0, "window.frontLoaded = true;"));

        let first = owner
            .next_runnable_script()
            .expect("completed front script should promote into runnable owner work");
        let second = owner
            .next_runnable_script()
            .expect("later ready module should chain behind completed front");

        let DynamicScriptRunnable::Execute { script: first, .. } = first else {
            panic!("front script should execute");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("module should execute");
        };

        assert_eq!(first.mode, ScriptMode::InOrder);
        let ScriptSource::Loaded(source) = &first.source else {
            panic!("front script should carry loaded source");
        };
        assert_eq!(source, "window.frontLoaded = true;");
        assert_eq!(second.mode, ScriptMode::ModuleInOrder);
    }

    #[test]
    fn completion_promotes_ready_async_into_owner_work_queue() {
        let mut owner = DynamicScriptOwner::default();
        owner
            .script_lanes
            .async_scripts
            .push_back(loading_entry(0, 0, ScriptMode::Async));

        assert!(
            owner.next_runnable_script().is_none(),
            "external async should not be runnable before completion"
        );

        owner.apply_completion(load_completion_ok(0, "window.asyncLoaded = true;"));

        let runnable = owner
            .next_runnable_script()
            .expect("completion should enqueue async owner work immediately");
        let DynamicScriptRunnable::Execute {
            script: runnable, ..
        } = runnable
        else {
            panic!("completed async should execute");
        };

        assert_eq!(runnable.mode, ScriptMode::Async);
        let ScriptSource::Loaded(source) = &runnable.source else {
            panic!("async script should carry loaded source");
        };
        assert_eq!(source, "window.asyncLoaded = true;");
    }

    #[test]
    fn earlier_async_completion_can_overtake_later_in_order_completion() {
        let mut owner = DynamicScriptOwner::default();
        owner
            .script_lanes
            .in_order
            .push_back(loading_entry(0, 0, ScriptMode::InOrder));
        owner
            .script_lanes
            .async_scripts
            .push_back(loading_entry(1, 1, ScriptMode::Async));

        owner.apply_completion(load_completion_ok(1, "window.asyncLoaded = true;"));
        owner.apply_completion(load_completion_ok(0, "window.inOrderLoaded = true;"));

        let first = owner
            .next_runnable_script()
            .expect("earlier async completion should run first");
        let second = owner
            .next_runnable_script()
            .expect("in-order completion should remain runnable");
        let DynamicScriptRunnable::Execute { script: first, .. } = first else {
            panic!("first work item should execute");
        };
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute");
        };

        assert_eq!(first.mode, ScriptMode::Async);
        assert_eq!(second.mode, ScriptMode::InOrder);
    }

    #[test]
    fn module_source_load_failure_yields_later_module_without_message_pattern() {
        let mut owner = DynamicScriptOwner::default();
        owner.script_lanes.module_in_order.push_back(loading_entry(
            0,
            0,
            ScriptMode::ModuleInOrder,
        ));
        owner
            .script_lanes
            .module_in_order
            .push_back(DynamicScriptEntry {
                id: owner_id(1),
                script: prepared_module_script(1, ScriptMode::ModuleInOrder),
                ready_state: DynamicScriptReadyState::Ready {
                    order: 0,
                    source_network_result: None,
                },
            });

        owner.apply_completion(load_completion_err(0, "opaque source load failure"));
        assert!(
            !owner.has_pending_module_script_graph(),
            "top-level source load failure should not look like a pending graph"
        );

        let later = owner
            .next_runnable_script()
            .expect("later module should run before top-level module load error");
        let DynamicScriptRunnable::Execute { script: later, .. } = later else {
            panic!("later module should execute before source load failure");
        };
        assert_eq!(later.position, 1);

        let failure = owner
            .next_runnable_script()
            .expect("source load failure should remain queued");
        let DynamicScriptRunnable::DispatchError {
            script, message, ..
        } = failure
        else {
            panic!("source load failure should dispatch after later module");
        };
        assert_eq!(script.position, 0);
        assert_eq!(message, "opaque source load failure");
    }

    #[tokio::test]
    async fn completion_source_delivers_payload_before_nonblocking_poll() {
        let mut events = DynamicScriptOwnerEventSource::default();
        let mut owner = DynamicScriptOwner::with_event_sender(events.sender());
        owner
            .script_lanes
            .async_scripts
            .push_back(loading_entry(7, 0, ScriptMode::Async));
        owner.in_flight_loads = 1;
        let tx = owner.owner_event_sender().clone();
        tokio::spawn(async move {
            let _ = tx.send(DynamicScriptOwnerEvent::Completion(load_completion_ok(
                7,
                "console.log('ready');",
            )));
        });

        let event = events
            .events
            .recv()
            .await
            .expect("completion source should deliver the concrete payload");
        owner.apply_owner_event(event);
        let poll = owner.poll_nonblocking();
        let DynamicScriptOwnerPoll::Work(work) = poll else {
            panic!("completed async should surface directly as owner work");
        };
        let DynamicScriptRunnable::Execute { script, .. } = *work else {
            panic!("completed async should surface directly as owner work");
        };
        let ScriptSource::Loaded(source) = &script.source else {
            panic!("completed async should carry loaded source");
        };
        assert_eq!(script.mode, ScriptMode::Async);
        assert_eq!(source, "console.log('ready');");
    }

    #[test]
    fn external_load_uses_captured_runner_without_an_ambient_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("resource runner runtime");
        let task_runner = RendererResourceTaskRunner::from_tokio_handle(runtime.handle().clone());
        let request_client = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("request client");
        let mut events = DynamicScriptOwnerEventSource::default();
        let mut owner = DynamicScriptOwner::with_event_sender(events.sender());
        let mut script = prepared_script(0, ScriptMode::Async);
        script.source_kind = ScriptSourceKind::External;
        script.source = ScriptSource::External;
        script.url = Url::parse("data:text/javascript,globalThis.ready%3Dtrue")
            .expect("external data script URL");

        // This call deliberately happens outside a Tokio runtime. The load is
        // valid because its committed Document authority supplied the runner;
        // reverting to ambient `tokio::spawn()` would panic here.
        owner.enqueue_script_with_id(
            &request_client,
            task_runner,
            owner_id(7),
            DynamicScriptQueueKind::Async,
            script,
            None,
            None,
            None,
        );

        let event = runtime
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_secs(2), events.events.recv()).await
            })
            .expect("captured runner should complete the external load")
            .expect("external load should publish its completion");
        owner.apply_owner_event(event);

        assert!(matches!(
            owner.poll_nonblocking(),
            DynamicScriptOwnerPoll::Work(work)
                if matches!(*work, DynamicScriptRunnable::Execute { .. })
        ));
    }

    #[test]
    fn nonblocking_poll_reports_stall_without_inflight_loads() {
        let mut owner = DynamicScriptOwner::default();
        owner
            .script_lanes
            .async_scripts
            .push_back(loading_entry(7, 0, ScriptMode::Async));

        let poll = owner.poll_nonblocking();
        assert!(matches!(
            poll,
            DynamicScriptOwnerPoll::StalledWithoutInflightLoads
        ));
    }

    #[test]
    fn ready_completion_payload_is_applied_before_nonblocking_poll() {
        let mut events = DynamicScriptOwnerEventSource::default();
        let mut owner = DynamicScriptOwner::with_event_sender(events.sender());
        owner
            .script_lanes
            .async_scripts
            .push_back(loading_entry(7, 0, ScriptMode::Async));
        owner.in_flight_loads = 1;
        owner
            .owner_event_sender()
            .send(DynamicScriptOwnerEvent::Completion(load_completion_ok(
                7,
                "console.log('ready');",
            )))
            .expect("completion send should succeed");

        for event in events.drain_ready() {
            owner.apply_owner_event(event);
        }
        let poll = owner.poll_nonblocking();
        let DynamicScriptOwnerPoll::Work(work) = poll else {
            panic!("completed async should be visible to non-blocking poll");
        };
        let DynamicScriptRunnable::Execute { script, .. } = *work else {
            panic!("completed async should be visible to non-blocking poll");
        };
        let ScriptSource::Loaded(source) = &script.source else {
            panic!("completed async should carry loaded source");
        };
        assert_eq!(script.mode, ScriptMode::Async);
        assert_eq!(source, "console.log('ready');");
    }

    #[test]
    fn async_module_resolve_failure_yields_later_async_before_error() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first async module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        let message = "ModuleLinkFailed: module `/dep.mjs` does not export `default`".to_owned();
        let kind = DynamicScriptOwner::module_load_failure_kind(
            &first,
            crate::module_runtime::ModuleLoadStage::Resolve,
        );
        owner.note_script_failed_with_kind(
            first_id,
            &first,
            message,
            kind,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
        );
        assert!(
            owner.has_pending_module_script_graph(),
            "deferred graph failure should keep runtime-owned module work pending"
        );

        let second = owner
            .next_runnable_script()
            .expect("later async module should run before link failure report");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute later async module");
        };
        assert_eq!(second.position, 1);
        assert!(
            owner.has_pending_module_script_graph(),
            "deferred graph failure should remain pending until its error dispatch is promoted"
        );

        let failure = owner
            .next_runnable_script()
            .expect("deferred link failure should surface after later async module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            script,
            message,
            ..
        } = failure
        else {
            panic!("third work item should dispatch module link failure");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(script.position, 0);
        assert!(
            !owner.has_pending_module_script_graph(),
            "promoted graph failure should leave the pending module graph set"
        );
        assert!(
            message.starts_with("ModuleLinkFailed:"),
            "unexpected failure message: {message}"
        );
    }

    #[test]
    fn selected_action_failure_terminal_does_not_cross_an_unrelated_runnable_head() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            ..DynamicScriptBatch::default()
        });

        let DynamicScriptRunnable::Execute {
            id: failed_id,
            script: failed_script,
            ..
        } = owner
            .next_runnable_script()
            .expect("first async module should be ready")
        else {
            panic!("first work item should execute");
        };
        owner.note_script_failed_with_kind(
            failed_id,
            &failed_script,
            "resolve failed".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
        );

        assert!(
            owner
                .take_runnable_failure_terminal_for_action(&[failed_id])
                .is_none(),
            "the selected action must not drain a later runnable async module"
        );
        let DynamicScriptRunnable::Execute { script, .. } = owner
            .next_runnable_script()
            .expect("unrelated async module should remain runnable")
        else {
            panic!("unrelated head should remain an executable script");
        };
        assert_eq!(script.position, 1);
    }

    #[test]
    fn selected_action_settles_every_now_runnable_failure_in_its_fanout() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            ..DynamicScriptBatch::default()
        });

        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = owner
            .next_runnable_script()
            .expect("first async module should be ready")
        else {
            panic!("first work item should execute");
        };
        owner.note_script_failed_with_kind(
            first_id,
            &first,
            "first resolve failed".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
        );

        let DynamicScriptRunnable::Execute {
            id: second_id,
            script: second,
            ..
        } = owner
            .next_runnable_script()
            .expect("later async module should run before the first error")
        else {
            panic!("second work item should execute");
        };
        owner.note_script_failed_with_kind(
            second_id,
            &second,
            "second resolve failed".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
        );

        let action_ids = [first_id, second_id];
        let first_terminal = owner
            .take_runnable_failure_terminal_for_action(&action_ids)
            .expect("second failure should now be the lane head");
        let second_terminal = owner
            .take_runnable_failure_terminal_for_action(&action_ids)
            .expect("first failure should follow in the same action");
        assert_eq!(first_terminal.id, second_id);
        assert_eq!(second_terminal.id, first_id);
        assert!(
            owner
                .take_runnable_failure_terminal_for_action(&action_ids)
                .is_none(),
            "the action should stop after consuming its exact fanout"
        );
    }

    #[test]
    fn typed_module_graph_failure_yields_later_async_and_preserves_constructor() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first async module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        owner.note_script_failed_with_kind(
            first_id,
            &first,
            "opaque typed module graph failure".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            Some(ScriptErrorConstructorKind::SyntaxError),
        );

        let second = owner
            .next_runnable_script()
            .expect("typed graph failure should wait behind later async module");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute later async module");
        };
        assert_eq!(second.position, 1);

        let failure = owner
            .next_runnable_script()
            .expect("typed graph failure should surface after later async module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            message,
            error_constructor,
            ..
        } = failure
        else {
            panic!("third work item should dispatch typed graph failure");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(message, "opaque typed module graph failure");
        assert_eq!(
            error_constructor,
            Some(ScriptErrorConstructorKind::SyntaxError),
            "deferred dynamic module failures must retain their original error constructor"
        );
    }

    #[test]
    fn typed_module_graph_fetch_failure_yields_later_async_and_preserves_policy() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first async module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        owner.note_script_failed_with_kind(
            first_id,
            &first,
            "module dependency fetch failed".to_owned(),
            DynamicScriptFailureKind::ModuleFetch,
            Some(ModuleFailurePolicy::ModuleTreeLoadFailure),
            None,
        );
        assert!(
            owner.has_pending_module_script_graph(),
            "module graph fetch failure should stay pending until promoted"
        );

        let second = owner
            .next_runnable_script()
            .expect("typed graph fetch failure should wait behind later async module");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute later async module");
        };
        assert_eq!(second.position, 1);

        let failure = owner
            .next_runnable_script()
            .expect("typed graph fetch failure should surface after later async module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            message,
            module_failure_policy,
            ..
        } = failure
        else {
            panic!("third work item should dispatch typed graph fetch failure");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(message, "module dependency fetch failed");
        assert_eq!(
            module_failure_policy,
            Some(ModuleFailurePolicy::ModuleTreeLoadFailure)
        );
    }

    #[test]
    fn module_evaluation_failure_dispatches_before_later_async_module() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first async module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        owner.note_script_failed_with_kind(
            first_id,
            &first,
            "native module graph evaluation rejected".to_owned(),
            DynamicScriptFailureKind::Immediate,
            Some(ModuleFailurePolicy::EvaluationFailure),
            None,
        );

        let failure = owner
            .next_runnable_script()
            .expect("evaluation failure should dispatch before later module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            script,
            message,
            module_failure_policy,
            ..
        } = failure
        else {
            panic!("evaluation failure should be the next work item");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(script.position, 0);
        assert_eq!(message, "native module graph evaluation rejected");
        assert_eq!(
            module_failure_policy,
            Some(ModuleFailurePolicy::EvaluationFailure)
        );

        let second = owner
            .next_runnable_script()
            .expect("later async module should remain queued after evaluation failure");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute later async module");
        };
        assert_eq!(second.position, 1);
    }

    #[test]
    fn module_load_failure_kind_maps_graph_stages_without_message_text() {
        let script = prepared_module_script(0, ScriptMode::Async);

        assert_eq!(
            DynamicScriptOwner::module_load_failure_kind(
                &script,
                crate::module_runtime::ModuleLoadStage::Fetch
            ),
            DynamicScriptFailureKind::ModuleFetch
        );
        assert_eq!(
            DynamicScriptOwner::module_load_failure_kind(
                &script,
                crate::module_runtime::ModuleLoadStage::Resolve
            ),
            DynamicScriptFailureKind::ModuleResolve
        );
        assert_eq!(
            DynamicScriptOwner::module_load_failure_kind(
                &script,
                crate::module_runtime::ModuleLoadStage::Instantiate
            ),
            DynamicScriptFailureKind::ModuleInstantiate
        );
        assert_eq!(
            DynamicScriptOwner::module_load_failure_kind(
                &script,
                crate::module_runtime::ModuleLoadStage::Evaluate
            ),
            DynamicScriptFailureKind::Immediate
        );
    }

    #[test]
    fn legacy_message_failure_kind_does_not_parse_module_error_text() {
        let script = prepared_module_script(0, ScriptMode::Async);

        assert_eq!(
            DynamicScriptOwner::legacy_message_failure_kind(
                &script,
                "ModuleLinkFailed: module `/dep.mjs` does not export `default`",
            ),
            DynamicScriptFailureKind::Immediate
        );
    }

    #[test]
    fn in_order_module_resolve_failure_yields_later_module_before_error() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::from([
                prepared_module_script(0, ScriptMode::ModuleInOrder),
                prepared_module_script(1, ScriptMode::ModuleInOrder),
            ]),
            async_scripts: VecDeque::new(),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first in-order module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        let message = "ModuleLinkFailed: module `/dep.mjs` does not export `default`".to_owned();
        let kind = DynamicScriptOwner::module_load_failure_kind(
            &first,
            crate::module_runtime::ModuleLoadStage::Resolve,
        );
        owner.note_script_failed_with_kind(first_id, &first, message, kind, None, None);

        let second = owner
            .next_runnable_script()
            .expect("later in-order module should run before link failure report");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("second work item should execute later in-order module");
        };
        assert_eq!(second.position, 1);

        let failure = owner
            .next_runnable_script()
            .expect("deferred link failure should surface after later in-order module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            script,
            message,
            ..
        } = failure
        else {
            panic!("third work item should dispatch module link failure");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(script.position, 0);
        assert!(
            message.starts_with("ModuleLinkFailed:"),
            "unexpected failure message: {message}"
        );
    }

    #[test]
    fn async_module_evaluation_failure_keeps_error_ahead_of_later_async() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            in_order: VecDeque::new(),
            importmap_in_order: VecDeque::new(),
            module_in_order: VecDeque::new(),
            async_scripts: VecDeque::from([
                prepared_module_script(0, ScriptMode::Async),
                prepared_module_script(1, ScriptMode::Async),
            ]),
            failed_scripts: VecDeque::new(),
        });

        let first = owner
            .next_runnable_script()
            .expect("first async module should be ready");
        let DynamicScriptRunnable::Execute {
            id: first_id,
            script: first,
            ..
        } = first
        else {
            panic!("first work item should execute");
        };

        owner.note_script_failed_with_kind(
            first_id,
            &first,
            "tla-broken".to_owned(),
            DynamicScriptFailureKind::Immediate,
            None,
            None,
        );

        let failure = owner
            .next_runnable_script()
            .expect("evaluation failure should stay ahead of later async module");
        let DynamicScriptRunnable::DispatchError {
            id: failure_id,
            script,
            message,
            ..
        } = failure
        else {
            panic!("second work item should dispatch evaluation failure");
        };
        assert_eq!(failure_id, first_id);
        assert_eq!(script.position, 0);
        assert_eq!(message, "tla-broken");

        let second = owner
            .next_runnable_script()
            .expect("later async module should still remain queued");
        let DynamicScriptRunnable::Execute { script: second, .. } = second else {
            panic!("third work item should execute later async module");
        };
        assert_eq!(second.position, 1);
    }

    #[test]
    fn owner_terminal_failure_preserves_error_constructor() {
        let mut owner = DynamicScriptOwner::default();
        owner.enqueue_batch(DynamicScriptBatch {
            async_scripts: VecDeque::from([prepared_script(0, ScriptMode::Async)]),
            ..DynamicScriptBatch::default()
        });
        let DynamicScriptRunnable::Execute { id, script, .. } = owner
            .next_runnable_script()
            .expect("dynamic script should become executable")
        else {
            panic!("expected executable dynamic script");
        };

        owner.note_script_failed_with_kind_and_error_constructor(
            id,
            &script,
            "typed failure".to_owned(),
            DynamicScriptFailureKind::Immediate,
            None,
            Some(ScriptErrorConstructorKind::SyntaxError),
        );

        let DynamicScriptRunnable::DispatchError {
            error_constructor, ..
        } = owner
            .next_runnable_script()
            .expect("typed failure should become owner terminal work")
        else {
            panic!("expected dynamic script error dispatch");
        };
        assert_eq!(
            error_constructor,
            Some(ScriptErrorConstructorKind::SyntaxError)
        );
    }

    #[test]
    #[should_panic(
        expected = "runtime dynamic script should carry host handle before failure dispatch planning"
    )]
    fn failed_script_without_handle_is_rejected() {
        let mut owner = DynamicScriptOwner::default();
        let mut script = prepared_script(0, ScriptMode::Async);
        script.host_script_handle = None;
        owner.note_script_failed_with_kind(
            owner_id(0),
            &script,
            "boom".to_owned(),
            DynamicScriptFailureKind::Immediate,
            None,
            None,
        );
    }
}
