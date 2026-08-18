//! Stable per-page turn scheduling state.
//!
//! The scheduler is owned by `RendererOwnerLocalPageSlot`, so its trigger and
//! source arbitration survive individual command and lifecycle futures.
//! Deadline residence belongs to the owner-wide timer index, not this
//! scheduler. Every ordinary source exposes a typed ready descriptor. Due
//! timers are scheduler-visible descriptors whose exact payload remains in
//! the Document/realm-bound timer heap.
//!
//! The container is page-owned, but executable semantics remain
//! document-owned. A trigger is only a page-scoped liveness hint: it never
//! identifies or authorizes work for the current document. Typed queued work
//! must carry and validate its exact execution-context/document identity.

use crate::page_task_queue::{
    RendererOwnerWakeSource, RendererPageReadyDescriptor, RendererPageTaskSourceKind,
};
use std::time::Instant;

use super::RendererOwnerRuntimeActivitySource;
use super::page_entry_residence::{
    RendererPageEntryCheckout, RendererPageEntryResidenceSlot, RendererPageEntryRestore,
};

#[derive(Debug)]
pub(super) struct PageTurnScheduler<Entry> {
    residence: RendererPageEntryResidenceSlot<Entry>,
    scheduled_trigger: Option<PageTurnTrigger>,
    arbitration: PageTaskArbitrationState,
    turn_class_arbitration: PageTurnClassArbitrationState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageTurnTrigger {
    /// A page-scoped hint that a producer may have made work ready. The queued
    /// typed payload, not this trigger, owns execution identity.
    Producer { source: RendererOwnerWakeSource },
    /// A page-scoped hint that the scheduler's derived deadline is due. The
    /// selected timer/task still has to validate its exact owner.
    Deadline,
}

impl PageTurnTrigger {
    pub(super) const fn producer(source: RendererOwnerWakeSource) -> Self {
        Self::Producer { source }
    }

    pub(super) const fn producer_source(self) -> Option<RendererOwnerWakeSource> {
        match self {
            Self::Producer { source, .. } => Some(source),
            Self::Deadline => None,
        }
    }
}

const MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE: usize = 8;
const MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS: usize = 8;

#[derive(Debug, Default)]
struct PageTaskArbitrationState {
    last_selected_source: Option<RendererPageTaskSourceKind>,
    consecutive_from_source: usize,
    fairness_cursor: Option<RendererPageTaskSourceKind>,
}

impl PageTaskArbitrationState {
    fn break_consecutive_run(&mut self) {
        self.last_selected_source = None;
        self.consecutive_from_source = 0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageTurnClass {
    Ordinary,
    DocumentLifecycle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentLifecycleClassReadiness {
    Absent,
    Available,
    RunnableContinuation,
    ReadyMainParserScriptContinuation,
}

impl DocumentLifecycleClassReadiness {
    pub(super) fn from_resident_state(
        has_pending_document_lifecycle_turn: bool,
        owner_turn_is_runnable: bool,
        has_ready_main_parser_script_continuation: bool,
    ) -> Self {
        assert!(
            !(owner_turn_is_runnable || has_ready_main_parser_script_continuation)
                || has_pending_document_lifecycle_turn,
            "a runnable lifecycle continuation must retain its exact resident"
        );
        match (
            has_pending_document_lifecycle_turn,
            owner_turn_is_runnable,
            has_ready_main_parser_script_continuation,
        ) {
            (true, _, true) => Self::ReadyMainParserScriptContinuation,
            (true, true, false) => Self::RunnableContinuation,
            (true, false, false) => Self::Available,
            (false, _, _) => Self::Absent,
        }
    }

    const fn is_available(self) -> bool {
        !matches!(self, Self::Absent)
    }
}

#[derive(Debug, Default)]
struct PageTurnClassArbitrationState {
    last_selected_class: Option<PageTurnClass>,
    consecutive_from_class: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageTurnAdmission {
    EnqueueOwnerTurn,
    AlreadyScheduled,
    Retired,
}

/// Whether an ordinary page-owner source has more work ready after one
/// bounded action. Source-specific terminal effects deliberately do not live
/// here; all migrated ordinary sources share this scheduling contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOwnerTurnReadiness {
    Runnable,
    Blocked {
        reason: PageOwnerBlockedReason,
        deadline: Option<Instant>,
    },
    Idle,
}

/// Why an ordinary Page turn cannot currently select another action.
///
/// This is intentionally source-neutral. A descriptor can be durable but
/// temporarily ineligible because its exact Document state has not reached
/// the source's execution boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOwnerBlockedReason {
    NoEligibleSource,
}

/// The one internal continuation admitted after an ordinary page-owner turn.
///
/// The source that produced a runnable outcome remains part of the typed
/// action, so this enum cannot accidentally authorize two producer families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOwnerNextTurn {
    Ordinary,
    DocumentLifecycle,
    None,
}

impl PageOwnerTurnReadiness {
    pub(crate) fn next_turn(self, has_pending_document_lifecycle_turn: bool) -> PageOwnerNextTurn {
        match (self, has_pending_document_lifecycle_turn) {
            (Self::Runnable, _) => PageOwnerNextTurn::Ordinary,
            (Self::Blocked { .. } | Self::Idle, true) => PageOwnerNextTurn::DocumentLifecycle,
            (Self::Blocked { .. } | Self::Idle, false) => PageOwnerNextTurn::None,
        }
    }
}

/// Semantic result of exactly one ordinary Page action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageOwnerTurnOutcome<Action> {
    pub(crate) action: Action,
}

impl<Action> PageOwnerTurnOutcome<Action> {
    pub(crate) const fn new(action: Action) -> Self {
        Self { action }
    }

    pub(crate) fn map_action<Mapped>(
        self,
        map: impl FnOnce(Action) -> Mapped,
    ) -> PageOwnerTurnOutcome<Mapped> {
        PageOwnerTurnOutcome {
            action: map(self.action),
        }
    }
}

#[derive(Debug)]
pub(super) enum ScheduledPageTurnCheckout<Entry> {
    Turn {
        entry: Entry,
        trigger: PageTurnTrigger,
    },
    NotScheduled,
    Busy,
    Retired,
}

impl<Entry> PageTurnScheduler<Entry> {
    pub(super) fn new(entry: Entry) -> Self {
        Self {
            residence: RendererPageEntryResidenceSlot::new(entry),
            scheduled_trigger: None,
            arbitration: PageTaskArbitrationState::default(),
            turn_class_arbitration: PageTurnClassArbitrationState::default(),
        }
    }

    /// Select which executable Page-turn class may consume an admitted wake.
    ///
    /// A wake supplies only a preferred class: it never authorizes work. If
    /// both ordinary tasks and an exact-Document lifecycle resident remain
    /// available, neither class may run more than a bounded number of turns
    /// without yielding once to the other. Ordinary source-head ordering and
    /// fairness remain a separate, nested arbitration step.
    pub(super) fn select_turn_class(
        &mut self,
        trigger: PageTurnTrigger,
        has_eligible_ordinary_task: bool,
        document_lifecycle: DocumentLifecycleClassReadiness,
    ) -> Option<PageTurnClass> {
        if matches!(
            document_lifecycle,
            DocumentLifecycleClassReadiness::RunnableContinuation
                | DocumentLifecycleClassReadiness::ReadyMainParserScriptContinuation
        ) {
            return Some(self.record_selected_turn_class(PageTurnClass::DocumentLifecycle));
        }
        let preferred = if trigger.producer_source()
            == Some(RendererOwnerWakeSource::Runtime(
                RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
            )) {
            PageTurnClass::DocumentLifecycle
        } else {
            PageTurnClass::Ordinary
        };
        let class_is_available = |class| match class {
            PageTurnClass::Ordinary => has_eligible_ordinary_task,
            PageTurnClass::DocumentLifecycle => document_lifecycle.is_available(),
        };
        let alternate = alternate_turn_class(preferred);
        let primary = if class_is_available(preferred) {
            preferred
        } else if class_is_available(alternate) {
            alternate
        } else {
            return None;
        };
        let should_yield = self.turn_class_arbitration.last_selected_class == Some(primary)
            && self.turn_class_arbitration.consecutive_from_class
                >= MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS
            && class_is_available(alternate_turn_class(primary));
        let selected = if should_yield {
            alternate_turn_class(primary)
        } else {
            primary
        };
        Some(self.record_selected_turn_class(selected))
    }

    /// Select work while page creation owns an exact lifecycle reply target.
    ///
    /// A runnable lifecycle resident gets the first opportunity regardless of
    /// which producer wake was admitted. The owner then gives a displaced
    /// ordinary source one turn before lifecycle arbitration resumes. This
    /// keeps post-target work behind the reply boundary without starving
    /// resource tasks needed to reach that boundary.
    pub(super) fn select_lifecycle_turn(
        &mut self,
        reconsider_displaced_ordinary: bool,
        has_eligible_ordinary_task: bool,
        document_lifecycle: DocumentLifecycleClassReadiness,
    ) -> Option<PageTurnClass> {
        let selected = if matches!(
            document_lifecycle,
            DocumentLifecycleClassReadiness::RunnableContinuation
                | DocumentLifecycleClassReadiness::ReadyMainParserScriptContinuation
        ) {
            PageTurnClass::DocumentLifecycle
        } else if reconsider_displaced_ordinary && has_eligible_ordinary_task {
            PageTurnClass::Ordinary
        } else if document_lifecycle.is_available() {
            PageTurnClass::DocumentLifecycle
        } else if has_eligible_ordinary_task {
            PageTurnClass::Ordinary
        } else {
            return None;
        };
        Some(self.record_selected_turn_class(selected))
    }

    fn record_selected_turn_class(&mut self, selected: PageTurnClass) -> PageTurnClass {
        if self.turn_class_arbitration.last_selected_class == Some(selected) {
            self.turn_class_arbitration.consecutive_from_class = self
                .turn_class_arbitration
                .consecutive_from_class
                .saturating_add(1);
        } else {
            self.turn_class_arbitration.last_selected_class = Some(selected);
            self.turn_class_arbitration.consecutive_from_class = 1;
        }
        if matches!(selected, PageTurnClass::DocumentLifecycle) {
            self.arbitration.break_consecutive_run();
        }
        selected
    }

    /// Select one eligible source head. The oldest runnable head wins under
    /// ordinary load, preserving the migration's observable ordering where it
    /// is already constrained by tests. After a bounded run from one source,
    /// the next ready source is chosen by a source-level round-robin cursor.
    /// This is a scheduler fairness policy, not an HTML cross-source ordering
    /// guarantee.
    pub(super) fn select_ready_descriptor(
        &mut self,
        descriptors: impl IntoIterator<Item = RendererPageReadyDescriptor>,
    ) -> Option<RendererPageReadyDescriptor> {
        let descriptors = descriptors.into_iter().collect::<Vec<_>>();
        let oldest = descriptors.iter().copied().min_by_key(|descriptor| {
            (
                descriptor.runnable_since(),
                // A timer became runnable at its deadline. Preserve the
                // existing `deadline <= immediate_ready_at` boundary when the
                // timestamps are exactly equal; immediate sources still use
                // their global enqueue order among themselves.
                descriptor.enqueue_order().unwrap_or(0),
                descriptor.source_kind(),
            )
        })?;
        let oldest_source = oldest.source_kind();
        let should_apply_fairness = self.arbitration.last_selected_source == Some(oldest_source)
            && self.arbitration.consecutive_from_source >= MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE
            && descriptors
                .iter()
                .any(|descriptor| descriptor.source_kind() != oldest_source);
        let selected = if should_apply_fairness {
            let cursor = self.arbitration.fairness_cursor.unwrap_or(oldest_source);
            let selected_source = next_ready_source_after(&descriptors, cursor, oldest_source)
                .expect("fairness override requires another ready Page source");
            self.arbitration.fairness_cursor = Some(selected_source);
            descriptors
                .iter()
                .copied()
                .filter(|descriptor| descriptor.source_kind() == selected_source)
                .min_by_key(|descriptor| {
                    (
                        descriptor.runnable_since(),
                        descriptor.enqueue_order().unwrap_or(0),
                    )
                })
                .expect("selected fairness source must retain its ready descriptor")
        } else {
            oldest
        };
        let selected_source = selected.source_kind();
        if self.arbitration.last_selected_source == Some(selected_source) {
            self.arbitration.consecutive_from_source =
                self.arbitration.consecutive_from_source.saturating_add(1);
        } else {
            self.arbitration.last_selected_source = Some(selected_source);
            self.arbitration.consecutive_from_source = 1;
        }
        Some(selected)
    }

    pub(super) fn resident(&self) -> Option<&Entry> {
        self.residence.resident()
    }

    pub(super) fn resident_mut(&mut self) -> Option<&mut Entry> {
        self.residence.resident_mut()
    }

    pub(super) fn checkout(&mut self) -> RendererPageEntryCheckout<Entry> {
        self.residence.checkout()
    }

    pub(super) fn restore(&mut self, entry: Entry) -> RendererPageEntryRestore<Entry> {
        self.residence.restore(entry)
    }

    pub(super) fn request_retirement(&mut self) -> Option<Entry> {
        self.scheduled_trigger = None;
        self.residence.request_retirement()
    }

    pub(super) fn is_retiring(&self) -> bool {
        self.residence.is_retiring()
    }

    /// Admits one concrete producer/deadline trigger and reports whether the
    /// owner must enqueue its turn. The owner consumes page triggers one at a
    /// time while a page turn is pending, so an already scheduled trigger is
    /// not replaced or reclassified here.
    pub(super) fn admit_turn(&mut self, trigger: PageTurnTrigger) -> PageTurnAdmission {
        if self.is_retiring() {
            return PageTurnAdmission::Retired;
        }
        if self.scheduled_trigger.is_some() {
            return PageTurnAdmission::AlreadyScheduled;
        }
        self.scheduled_trigger = Some(trigger);
        PageTurnAdmission::EnqueueOwnerTurn
    }

    #[cfg(test)]
    pub(super) fn take_scheduled_trigger(&mut self) -> Option<PageTurnTrigger> {
        self.scheduled_trigger.take()
    }

    /// Atomically consumes one admitted page turn and checks out its resident
    /// entry. A busy checkout leaves the admission intact for diagnosis; the
    /// serialized owner treats `Busy` at this boundary as an invariant failure
    /// instead of retrying indefinitely.
    pub(super) fn checkout_scheduled_turn(&mut self) -> ScheduledPageTurnCheckout<Entry> {
        if self.scheduled_trigger.is_none() {
            return ScheduledPageTurnCheckout::NotScheduled;
        }
        match self.residence.checkout() {
            RendererPageEntryCheckout::Entry(entry) => ScheduledPageTurnCheckout::Turn {
                entry,
                trigger: self
                    .scheduled_trigger
                    .take()
                    .expect("scheduled page turn must retain its trigger until checkout"),
            },
            RendererPageEntryCheckout::Busy => ScheduledPageTurnCheckout::Busy,
            RendererPageEntryCheckout::Retired => ScheduledPageTurnCheckout::Retired,
        }
    }
}

fn next_ready_source_after(
    descriptors: &[RendererPageReadyDescriptor],
    cursor: RendererPageTaskSourceKind,
    excluded: RendererPageTaskSourceKind,
) -> Option<RendererPageTaskSourceKind> {
    let cursor_index = RendererPageTaskSourceKind::ALL
        .iter()
        .position(|source| *source == cursor)
        .expect("Page task fairness cursor must name a scheduler source");
    (1..=RendererPageTaskSourceKind::ALL.len())
        .map(|offset| {
            RendererPageTaskSourceKind::ALL
                [(cursor_index + offset) % RendererPageTaskSourceKind::ALL.len()]
        })
        .find(|source| {
            *source != excluded
                && descriptors
                    .iter()
                    .any(|descriptor| descriptor.source_kind() == *source)
        })
}

const fn alternate_turn_class(class: PageTurnClass) -> PageTurnClass {
    match class {
        PageTurnClass::Ordinary => PageTurnClass::DocumentLifecycle,
        PageTurnClass::DocumentLifecycle => PageTurnClass::Ordinary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document_runtime::DomHandle,
        frame_owner_model::{
            ChildDocumentModuleFetchTarget, DocumentId, FrameDocumentTaskOwner, FrameRealmId,
            FrameSchedulerLaneId, LocalWindowId,
        },
        native_bridge::{
            OwnerDispatchScope, RuntimeObservableContextToken, WindowDocumentOwner,
            WindowDocumentTaskTarget, WindowExecutionContextAccessPolicy,
            WindowExecutionContextIdentity, WindowExecutionContextOwner,
        },
        page_resource_completion::RendererPageResourceCompletionOwner,
        page_task_queue::{
            RendererPageBroadcastChannelDeliveryOwner, RendererPageChildFrameTaskOwner,
            RendererPageChildFrameTaskTarget, RendererPageChildModuleDependencyFetchStartOwner,
            RendererPageChildModuleScriptTerminalOwner,
            RendererPageChildRealmMaterializationTarget,
            RendererPageDedicatedWorkerClientEventOwner, RendererPageDomManipulationOwner,
            RendererPageFileReadingOwner, RendererPageHistoryTraversalOwner,
            RendererPageHistoryTraversalTaskId, RendererPageHistoryTraversalTaskKind,
            RendererPageIndexedDbTaskOwner, RendererPageMediaElementEventOwner,
            RendererPageMessagePortDeliveryOwner, RendererPageMiscPlatformApiOwner,
            RendererPageNavigationAndTraversalHead, RendererPageNetworkingOwner,
            RendererPageOpfsTaskId, RendererPageOpfsTaskOwner, RendererPageRenderingUpdateHead,
            RendererPageRenderingUpdateOwner, RendererPageRenderingUpdateTaskId,
            RendererPageRenderingUpdateTaskKind, RendererPageSharedWorkerClientEventOwner,
            RendererPageUserInteractionOwner, RendererPageWebCryptoTaskId,
            RendererPageWebCryptoTaskOwner, RendererPageWebSocketOwner,
            RendererPageWebSocketReadiness, RendererPageWindowMessageOwner,
            RendererPageWindowMessageTaskId,
        },
        resource_ready::RendererPageTaskReadyMetadata,
        runtime::{RendererDocumentToken, RendererOwnerRuntimeActivitySource},
    };
    use moli_shared_worker::{
        SharedWorkerConnectAction, SharedWorkerDescriptor, SharedWorkerKey, SharedWorkerRegistry,
        SharedWorkerSameSiteCookies,
    };
    use moli_storage_key::{MoliStorageKey, StoragePartitionRelation};

    fn ready_metadata(ready_at: Instant, order: u64) -> RendererPageTaskReadyMetadata {
        RendererPageTaskReadyMetadata { ready_at, order }
    }

    fn producer_trigger(source: RendererOwnerWakeSource) -> PageTurnTrigger {
        PageTurnTrigger::producer(source)
    }

    fn dynamic_import_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::DynamicImportOwnerAction {
            ready: ready_metadata(ready_at, order),
        }
    }

    fn broadcast_channel_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::DomManipulation {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageDomManipulationOwner::BroadcastChannel(
                RendererPageBroadcastChannelDeliveryOwner::new(
                    RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                    WindowExecutionContextIdentity::new(
                        WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                        OwnerDispatchScope::Top,
                        RuntimeObservableContextToken::from_raw(11),
                        WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                    ),
                ),
            ),
        }
    }

    fn user_interaction_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::UserInteraction {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageUserInteractionOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(order),
                        LocalWindowId(order + 1),
                        DocumentId(order + 2),
                    )),
                    OwnerDispatchScope::Top,
                ),
            ),
        }
    }

    fn file_reading_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::FileReading {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageFileReadingOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(order),
                        LocalWindowId(order + 1),
                        DocumentId(order + 2),
                    )),
                    OwnerDispatchScope::Top,
                ),
            ),
        }
    }

    fn misc_platform_api_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::MiscPlatformApi {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageMiscPlatformApiOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(order),
                        LocalWindowId(order + 1),
                        DocumentId(order + 2),
                    )),
                    OwnerDispatchScope::Top,
                ),
            ),
        }
    }

    fn modulepreload_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ModulepreloadStart {
            ready: ready_metadata(ready_at, order),
        }
    }

    fn history_traversal_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        let execution_context = WindowExecutionContextIdentity::new(
            WindowExecutionContextOwner::Frame(LocalWindowId(7)),
            OwnerDispatchScope::Top,
            RuntimeObservableContextToken::from_raw(11),
            WindowExecutionContextAccessPolicy::EnforceWebOrigin,
        );
        RendererPageReadyDescriptor::NavigationAndTraversal {
            ready: ready_metadata(ready_at, order),
            head: RendererPageNavigationAndTraversalHead::HistoryTraversal {
                owner: RendererPageHistoryTraversalOwner::new(
                    RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                    execution_context,
                    crate::native_bridge::WindowTaskTarget::new(
                        OwnerDispatchScope::Top,
                        execution_context.owner(),
                    ),
                ),
                task_id: RendererPageHistoryTraversalTaskId::from_raw(order),
                kind: RendererPageHistoryTraversalTaskKind::SameDocument,
            },
        }
    }

    fn rendering_update_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::RenderingUpdate {
            ready: ready_metadata(ready_at, order),
            head: RendererPageRenderingUpdateHead::new(
                RendererPageRenderingUpdateOwner::new(
                    RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                    WindowDocumentTaskTarget::new(
                        WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                            FrameSchedulerLaneId(order),
                            LocalWindowId(order + 1),
                            DocumentId(order + 2),
                        )),
                        OwnerDispatchScope::Top,
                    ),
                ),
                RendererPageRenderingUpdateTaskId::from_raw(order),
                RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
            ),
        }
    }

    fn media_element_event_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::MediaElementEvent {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageMediaElementEventOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowDocumentTaskTarget::new(
                    WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(order),
                        LocalWindowId(order + 1),
                        DocumentId(order + 2),
                    )),
                    OwnerDispatchScope::Top,
                ),
            ),
        }
    }

    fn dedicated_worker_client_event_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::DedicatedWorkerClientEvent {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageDedicatedWorkerClientEventOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
                crate::types::DedicatedWorkerId::new(order),
            ),
        }
    }

    fn shared_worker_client_event_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        let registry = SharedWorkerRegistry::<()>::default();
        let key = SharedWorkerKey::new(
            MoliStorageKey::new(
                "https://page-scheduler.test".to_owned(),
                "https://page-scheduler.test".to_owned(),
                None,
                StoragePartitionRelation::FirstParty,
            ),
            "https://page-scheduler.test/shared-worker.js".to_owned(),
            "scheduler".to_owned(),
            SharedWorkerSameSiteCookies::All,
        );
        let client_id = match registry.connect(key, SharedWorkerDescriptor::default()) {
            SharedWorkerConnectAction::StartLoading { client_id, .. } => client_id,
            other => panic!("fresh SharedWorker registry should allocate one client: {other:?}"),
        };
        RendererPageReadyDescriptor::SharedWorkerClientEvent {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageSharedWorkerClientEventOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
                client_id,
            ),
        }
    }

    fn service_worker_internal_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ServiceWorkerInternal {
            ready: ready_metadata(ready_at, order),
            root_document: RendererDocumentToken::new_for_testing(
                crate::PageId::new_for_testing(1),
                1,
            ),
        }
    }

    fn service_worker_client_message_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ServiceWorkerClientMessage {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageServiceWorkerClientMessageOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                crate::types::ServiceWorkerWindowClientTarget {
                    client_id:
                        crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(
                            order,
                        ),
                    document_owner: crate::native_bridge::WindowDocumentOwner::for_test(order + 1),
                },
            ),
        }
    }

    fn window_message_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::WindowMessage {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageWindowMessageOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                crate::native_bridge::WindowTaskTarget::new(
                    crate::native_bridge::OwnerDispatchScope::Top,
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                ),
            ),
            task_id: RendererPageWindowMessageTaskId::from_raw(order),
        }
    }

    fn webcrypto_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::WebCryptoTask {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageWebCryptoTaskOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
                RendererPageWebCryptoTaskId::new(order),
            ),
        }
    }

    fn indexed_db_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::IndexedDbTask {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageIndexedDbTaskOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
            ),
        }
    }

    fn opfs_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::OpfsTask {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageOpfsTaskOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
                RendererPageOpfsTaskId::new(order),
            ),
        }
    }

    fn child_realm_materialization_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ChildFrameTask {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageChildFrameTaskOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                RendererPageChildFrameTaskTarget::RealmMaterialization(
                    RendererPageChildRealmMaterializationTarget::new(
                        DomHandle::new(order as usize + 100),
                        FrameDocumentTaskOwner::new(
                            FrameSchedulerLaneId(order),
                            LocalWindowId(order + 1),
                            DocumentId(order + 2),
                        ),
                    ),
                ),
            ),
        }
    }

    fn message_port_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::MessagePortDelivery {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageMessagePortDeliveryOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                WindowExecutionContextIdentity::new(
                    WindowExecutionContextOwner::Frame(LocalWindowId(7)),
                    OwnerDispatchScope::Top,
                    RuntimeObservableContextToken::from_raw(11),
                    WindowExecutionContextAccessPolicy::EnforceWebOrigin,
                ),
            ),
            port_id: order,
        }
    }

    fn resource_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::Networking {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageNetworkingOwner::ResourceCompletion(
                RendererPageResourceCompletionOwner::child_document(
                    RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                    DomHandle::new(99),
                    FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(99),
                        LocalWindowId(100),
                        DocumentId(101),
                    ),
                ),
            ),
        }
    }

    fn websocket_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::WebSocket {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageWebSocketOwner::new_for_test(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                order,
            ),
            readiness: RendererPageWebSocketReadiness::Ready,
        }
    }

    fn v8_foreground_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::V8ForegroundTask {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageV8ForegroundTaskOwner::new(
                crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(
                    1,
                )),
            ),
        }
    }

    fn module_reaction_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ModuleReaction {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageModuleReactionOwner::new(
                crate::runtime::RendererDocumentToken::new_for_testing(
                    crate::PageId::new_for_testing(1),
                    1,
                ),
                crate::page_task_queue::RendererPageModuleReactionTarget::DocumentModuleScript {
                    document_owner: FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(1),
                        LocalWindowId(2),
                        DocumentId(3),
                    ),
                },
            ),
        }
    }

    fn internal_loading_descriptor(ready_at: Instant, order: u64) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::InternalLoading {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageInternalLoadingOwner::new(
                crate::runtime::RendererDocumentToken::new_for_testing(
                    crate::PageId::new_for_testing(1),
                    1,
                ),
                FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(order),
                    LocalWindowId(order + 1),
                    DocumentId(order + 2),
                ),
            ),
        }
    }

    fn child_modulepreload_event_action_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ChildModulepreloadEventAction {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageChildModulepreloadEventActionOwner::new(
                crate::runtime::RendererDocumentToken::new_for_testing(
                    crate::PageId::new_for_testing(1),
                    1,
                ),
                FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(order),
                    LocalWindowId(order + 1),
                    DocumentId(order + 2),
                ),
                crate::frame_owner_model::FrameRealmId(order as i64 + 3),
            ),
        }
    }

    fn child_module_dependency_fetch_start_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ChildModuleDependencyFetchStart {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageChildModuleDependencyFetchStartOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                ChildDocumentModuleFetchTarget::new(
                    DomHandle::new(order as usize + 100),
                    FrameDocumentTaskOwner::new(
                        FrameSchedulerLaneId(order),
                        LocalWindowId(order + 1),
                        DocumentId(order + 2),
                    ),
                    FrameRealmId(order as i64 + 3),
                ),
            ),
        }
    }

    fn child_module_script_terminal_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::ChildModuleScriptTerminal {
            ready: ready_metadata(ready_at, order),
            owner: RendererPageChildModuleScriptTerminalOwner::new(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(order),
                    LocalWindowId(order + 1),
                    DocumentId(order + 2),
                ),
                FrameRealmId(order as i64 + 3),
            ),
        }
    }

    fn main_document_runtime_descriptor(
        ready_at: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        RendererPageReadyDescriptor::MainDocumentRuntime {
            ready: ready_metadata(ready_at, order),
            owner: crate::page_task_queue::RendererPageMainDocumentRuntimeOwner::new_for_test(
                RendererDocumentToken::new_for_testing(crate::PageId::new_for_testing(1), 1),
                FrameDocumentTaskOwner::new(
                    FrameSchedulerLaneId(order),
                    LocalWindowId(order + 1),
                    DocumentId(order + 2),
                ),
            ),
        }
    }

    fn descriptor_for_source(
        source: RendererPageTaskSourceKind,
        runnable_since: Instant,
        order: u64,
    ) -> RendererPageReadyDescriptor {
        match source {
            RendererPageTaskSourceKind::ActionWindow => RendererPageReadyDescriptor::ActionWindow {
                deadline: runnable_since,
            },
            RendererPageTaskSourceKind::Timer => RendererPageReadyDescriptor::Timer {
                deadline: runnable_since,
            },
            RendererPageTaskSourceKind::DomManipulation => {
                broadcast_channel_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::UserInteraction => {
                user_interaction_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::FileReading => {
                file_reading_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::MiscPlatformApi => {
                misc_platform_api_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::NavigationAndTraversal => {
                history_traversal_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::RenderingUpdate => {
                rendering_update_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::MediaElementEvent => {
                media_element_event_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::DedicatedWorkerClientEvent => {
                dedicated_worker_client_event_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::SharedWorkerClientEvent => {
                shared_worker_client_event_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ServiceWorkerInternal => {
                service_worker_internal_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ServiceWorkerClientMessage => {
                service_worker_client_message_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::WebCryptoTask => {
                webcrypto_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::IndexedDbTask => {
                indexed_db_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::OpfsTask => opfs_descriptor(runnable_since, order),
            RendererPageTaskSourceKind::InternalLoading => {
                internal_loading_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::MainDocumentRuntime => {
                main_document_runtime_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ChildModuleDependencyFetchStart => {
                child_module_dependency_fetch_start_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ChildModuleScriptTerminal => {
                child_module_script_terminal_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ChildModulepreloadEventAction => {
                child_modulepreload_event_action_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ChildFrameTask => {
                child_realm_materialization_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::V8ForegroundTask => {
                v8_foreground_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ModuleReaction => {
                module_reaction_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::WindowMessage => {
                window_message_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::MessagePortDelivery => {
                message_port_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::DynamicImportOwnerAction => {
                dynamic_import_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::ModulepreloadStart => {
                modulepreload_descriptor(runnable_since, order)
            }
            RendererPageTaskSourceKind::Networking => resource_descriptor(runnable_since, order),
        }
    }

    #[test]
    fn admitted_trigger_is_not_reclassified_before_its_turn() {
        let mut scheduler = PageTurnScheduler::new(7_u8);

        assert_eq!(
            scheduler.admit_turn(PageTurnTrigger::Deadline),
            PageTurnAdmission::EnqueueOwnerTurn
        );
        assert_eq!(
            scheduler.admit_turn(producer_trigger(RendererOwnerWakeSource::Runtime(
                RendererOwnerRuntimeActivitySource::Timer,
            ))),
            PageTurnAdmission::AlreadyScheduled
        );
        assert_eq!(
            scheduler.take_scheduled_trigger(),
            Some(PageTurnTrigger::Deadline)
        );
        assert_eq!(scheduler.take_scheduled_trigger(), None);
    }

    #[test]
    fn command_checkout_keeps_an_admitted_trigger() {
        let mut scheduler = PageTurnScheduler::new(7_u8);
        let trigger = producer_trigger(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::Timer,
        ));
        assert_eq!(
            scheduler.admit_turn(trigger),
            PageTurnAdmission::EnqueueOwnerTurn
        );

        assert_eq!(scheduler.checkout(), RendererPageEntryCheckout::Entry(7));
        assert_eq!(scheduler.take_scheduled_trigger(), Some(trigger));
        assert_eq!(scheduler.restore(7), RendererPageEntryRestore::Restored);
    }

    #[test]
    fn busy_scheduled_checkout_preserves_the_admitted_trigger() {
        let mut scheduler = PageTurnScheduler::new(7_u8);
        assert_eq!(scheduler.checkout(), RendererPageEntryCheckout::Entry(7));
        let trigger = producer_trigger(RendererOwnerWakeSource::V8ForegroundTask);
        assert_eq!(
            scheduler.admit_turn(trigger),
            PageTurnAdmission::EnqueueOwnerTurn
        );

        assert!(matches!(
            scheduler.checkout_scheduled_turn(),
            ScheduledPageTurnCheckout::Busy
        ));
        assert_eq!(scheduler.restore(7), RendererPageEntryRestore::Restored);
        let ScheduledPageTurnCheckout::Turn {
            entry,
            trigger: actual_trigger,
        } = scheduler.checkout_scheduled_turn()
        else {
            panic!("restored entry must retain its admitted producer turn");
        };
        assert_eq!(entry, 7);
        assert_eq!(actual_trigger, trigger);
    }

    #[test]
    fn retirement_discards_scheduled_turn_state() {
        let mut scheduler = PageTurnScheduler::new(7_u8);
        assert_eq!(
            scheduler.admit_turn(PageTurnTrigger::Deadline),
            PageTurnAdmission::EnqueueOwnerTurn
        );

        assert_eq!(scheduler.request_retirement(), Some(7));
        assert_eq!(scheduler.take_scheduled_trigger(), None);
        assert_eq!(
            scheduler.admit_turn(producer_trigger(RendererOwnerWakeSource::V8ForegroundTask)),
            PageTurnAdmission::Retired
        );
    }

    #[test]
    fn page_local_admission_state_is_independent() {
        let mut first = PageTurnScheduler::new(1_u8);
        let mut second = PageTurnScheduler::new(2_u8);

        assert_eq!(
            first.admit_turn(PageTurnTrigger::Deadline),
            PageTurnAdmission::EnqueueOwnerTurn
        );
        assert_eq!(
            second.admit_turn(producer_trigger(RendererOwnerWakeSource::WindowMessageTask)),
            PageTurnAdmission::EnqueueOwnerTurn
        );

        assert!(matches!(
            first.checkout_scheduled_turn(),
            ScheduledPageTurnCheckout::Turn {
                entry: 1,
                trigger: PageTurnTrigger::Deadline,
            }
        ));
        let ScheduledPageTurnCheckout::Turn { entry, trigger } = second.checkout_scheduled_turn()
        else {
            panic!("second Page must retain its independent producer turn");
        };
        assert_eq!(entry, 2);
        assert_eq!(
            trigger.producer_source(),
            Some(RendererOwnerWakeSource::WindowMessageTask)
        );
    }

    #[test]
    fn blocked_ordinary_work_never_self_admits_a_page_turn() {
        let blocked = PageOwnerTurnReadiness::Blocked {
            reason: PageOwnerBlockedReason::NoEligibleSource,
            deadline: Some(Instant::now()),
        };

        assert_eq!(blocked.next_turn(false), PageOwnerNextTurn::None);
    }

    #[test]
    fn exact_document_lifecycle_remains_next_when_ordinary_work_blocks() {
        let blocked = PageOwnerTurnReadiness::Blocked {
            reason: PageOwnerBlockedReason::NoEligibleSource,
            deadline: None,
        };

        assert_eq!(
            blocked.next_turn(true),
            PageOwnerNextTurn::DocumentLifecycle
        );
    }

    #[test]
    #[should_panic(expected = "a runnable lifecycle continuation must retain its exact resident")]
    fn ready_parser_script_continuation_without_lifecycle_resident_is_rejected() {
        let _ = DocumentLifecycleClassReadiness::from_resident_state(false, false, true);
    }

    #[test]
    fn runnable_lifecycle_continuation_precedes_an_ordinary_event_wake() {
        let mut scheduler = PageTurnScheduler::new(());
        let ordinary_trigger = producer_trigger(RendererOwnerWakeSource::DomManipulationTask);

        assert_eq!(
            scheduler.select_turn_class(
                ordinary_trigger,
                true,
                DocumentLifecycleClassReadiness::RunnableContinuation,
            ),
            Some(PageTurnClass::DocumentLifecycle),
            "an already-runnable lifecycle chain must finish its next bounded action before unrelated ordinary work"
        );
    }

    #[test]
    fn sustained_ordinary_turns_yield_to_exact_document_lifecycle() {
        let mut scheduler = PageTurnScheduler::new(());
        let trigger = producer_trigger(RendererOwnerWakeSource::SchedulerContinuation);

        for _ in 0..MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS {
            assert_eq!(
                scheduler.select_turn_class(
                    trigger,
                    true,
                    DocumentLifecycleClassReadiness::Available,
                ),
                Some(PageTurnClass::Ordinary)
            );
        }
        assert_eq!(
            scheduler.select_turn_class(trigger, true, DocumentLifecycleClassReadiness::Available,),
            Some(PageTurnClass::DocumentLifecycle)
        );
    }

    #[test]
    fn ready_sealed_parser_script_continuation_precedes_an_ordinary_event_wake() {
        let mut scheduler = PageTurnScheduler::new(());
        let ordinary_trigger = producer_trigger(RendererOwnerWakeSource::DomManipulationTask);

        assert_eq!(
            scheduler.select_turn_class(
                ordinary_trigger,
                true,
                DocumentLifecycleClassReadiness::ReadyMainParserScriptContinuation,
            ),
            Some(PageTurnClass::DocumentLifecycle),
            "a sealed parser defer/module continuation has Chromium's internal-script priority"
        );
    }

    #[test]
    fn sustained_lifecycle_turns_yield_to_ordinary_page_work() {
        let mut scheduler = PageTurnScheduler::new(());
        let trigger = producer_trigger(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
        ));

        for _ in 0..MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS {
            assert_eq!(
                scheduler.select_turn_class(
                    trigger,
                    true,
                    DocumentLifecycleClassReadiness::Available,
                ),
                Some(PageTurnClass::DocumentLifecycle)
            );
        }
        assert_eq!(
            scheduler.select_turn_class(trigger, true, DocumentLifecycleClassReadiness::Available,),
            Some(PageTurnClass::Ordinary)
        );
    }

    #[test]
    fn lifecycle_target_boundary_yields_only_to_explicitly_displaced_work() {
        let mut scheduler = PageTurnScheduler::new(());

        for _ in 0..=MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS {
            assert_eq!(
                scheduler.select_lifecycle_turn(
                    false,
                    true,
                    DocumentLifecycleClassReadiness::Available,
                ),
                Some(PageTurnClass::DocumentLifecycle)
            );
        }
        assert_eq!(
            scheduler
                .select_lifecycle_turn(true, true, DocumentLifecycleClassReadiness::Available,),
            Some(PageTurnClass::Ordinary)
        );
        assert_eq!(
            scheduler.select_lifecycle_turn(false, true, DocumentLifecycleClassReadiness::Absent,),
            Some(PageTurnClass::Ordinary)
        );
        assert_eq!(
            scheduler.select_lifecycle_turn(
                true,
                true,
                DocumentLifecycleClassReadiness::ReadyMainParserScriptContinuation,
            ),
            Some(PageTurnClass::DocumentLifecycle)
        );
    }

    #[test]
    fn admitted_wake_cannot_select_an_unavailable_turn_class() {
        let mut scheduler = PageTurnScheduler::new(());
        let lifecycle_trigger = producer_trigger(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
        ));

        assert_eq!(
            scheduler.select_turn_class(
                lifecycle_trigger,
                true,
                DocumentLifecycleClassReadiness::Absent,
            ),
            Some(PageTurnClass::Ordinary)
        );
        assert_eq!(
            scheduler.select_turn_class(
                producer_trigger(RendererOwnerWakeSource::SchedulerContinuation),
                false,
                DocumentLifecycleClassReadiness::Available,
            ),
            Some(PageTurnClass::DocumentLifecycle)
        );
        assert_eq!(
            scheduler.select_turn_class(
                lifecycle_trigger,
                false,
                DocumentLifecycleClassReadiness::Absent,
            ),
            None
        );
    }

    #[test]
    fn lifecycle_turn_breaks_an_ordinary_source_consecutive_run() {
        let mut scheduler = PageTurnScheduler::new(());
        let base = Instant::now();
        let descriptors = || {
            [
                dynamic_import_descriptor(base, 1),
                modulepreload_descriptor(base + std::time::Duration::from_millis(1), 2),
            ]
        };
        for _ in 0..MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE {
            assert_eq!(
                scheduler
                    .select_ready_descriptor(descriptors())
                    .expect("dominant ordinary source should remain ready")
                    .source_kind(),
                RendererPageTaskSourceKind::DynamicImportOwnerAction
            );
        }
        assert_eq!(
            scheduler.select_turn_class(
                producer_trigger(RendererOwnerWakeSource::Runtime(
                    RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
                )),
                true,
                DocumentLifecycleClassReadiness::Available,
            ),
            Some(PageTurnClass::DocumentLifecycle)
        );
        assert_eq!(
            scheduler
                .select_ready_descriptor(descriptors())
                .expect("ordinary arbitration should resume after lifecycle")
                .source_kind(),
            RendererPageTaskSourceKind::DynamicImportOwnerAction
        );
    }

    #[test]
    fn turn_class_fairness_history_is_isolated_per_page_scheduler() {
        let trigger = producer_trigger(RendererOwnerWakeSource::SchedulerContinuation);
        let mut first = PageTurnScheduler::new(1_u8);
        let mut second = PageTurnScheduler::new(2_u8);
        for _ in 0..MAX_CONSECUTIVE_PAGE_TURNS_PER_CLASS {
            let _ =
                first.select_turn_class(trigger, true, DocumentLifecycleClassReadiness::Available);
        }

        assert_eq!(
            first.select_turn_class(trigger, true, DocumentLifecycleClassReadiness::Available,),
            Some(PageTurnClass::DocumentLifecycle)
        );
        assert_eq!(
            second.select_turn_class(trigger, true, DocumentLifecycleClassReadiness::Available,),
            Some(PageTurnClass::Ordinary)
        );
    }

    #[test]
    fn due_timer_uses_runnable_time_and_precedes_newer_immediate_work() {
        let mut scheduler = PageTurnScheduler::new(());
        let base = Instant::now();
        let selected = scheduler
            .select_ready_descriptor([
                dynamic_import_descriptor(base + std::time::Duration::from_millis(2), 1),
                RendererPageReadyDescriptor::Timer { deadline: base },
            ])
            .expect("one ready descriptor should be selected");

        assert_eq!(selected.source_kind(), RendererPageTaskSourceKind::Timer);
    }

    #[test]
    fn immediate_sources_use_enqueue_order_when_ready_times_match() {
        let mut scheduler = PageTurnScheduler::new(());
        let ready_at = Instant::now();
        let selected = scheduler
            .select_ready_descriptor([
                modulepreload_descriptor(ready_at, 2),
                dynamic_import_descriptor(ready_at, 1),
            ])
            .expect("one immediate descriptor should be selected");

        assert_eq!(
            selected.source_kind(),
            RendererPageTaskSourceKind::DynamicImportOwnerAction
        );
    }

    #[test]
    fn timer_wins_an_exact_runnable_time_tie_with_immediate_work() {
        let mut scheduler = PageTurnScheduler::new(());
        let runnable_at = Instant::now();
        let selected = scheduler
            .select_ready_descriptor([
                dynamic_import_descriptor(runnable_at, 1),
                RendererPageReadyDescriptor::Timer {
                    deadline: runnable_at,
                },
            ])
            .expect("one tied descriptor should be selected");

        assert_eq!(selected.source_kind(), RendererPageTaskSourceKind::Timer);
    }

    #[test]
    fn oldest_runnable_head_can_win_from_every_page_source() {
        let base = Instant::now();
        for expected in RendererPageTaskSourceKind::ALL {
            let descriptors =
                RendererPageTaskSourceKind::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, source)| {
                        let runnable_since = if source == expected {
                            base
                        } else {
                            base + std::time::Duration::from_millis(index as u64 + 1)
                        };
                        descriptor_for_source(source, runnable_since, index as u64 + 1)
                    });
            let selected = PageTurnScheduler::new(())
                .select_ready_descriptor(descriptors)
                .expect("each source matrix must retain a runnable head");
            assert_eq!(
                selected.source_kind(),
                expected,
                "the oldest runnable head from {expected:?} must be selectable"
            );
        }
    }

    #[test]
    fn fairness_bound_rotates_across_all_waiting_sources() {
        let mut scheduler = PageTurnScheduler::new(());
        let base = Instant::now();
        let candidates = || {
            [
                dynamic_import_descriptor(base, 1),
                modulepreload_descriptor(base + std::time::Duration::from_millis(1), 2),
                resource_descriptor(base + std::time::Duration::from_millis(2), 3),
                RendererPageReadyDescriptor::Timer {
                    deadline: base + std::time::Duration::from_millis(3),
                },
                broadcast_channel_descriptor(base + std::time::Duration::from_millis(4), 4),
                window_message_descriptor(base + std::time::Duration::from_millis(5), 5),
                message_port_descriptor(base + std::time::Duration::from_millis(6), 6),
            ]
        };
        for expected_override in [
            RendererPageTaskSourceKind::ModulepreloadStart,
            RendererPageTaskSourceKind::Networking,
            RendererPageTaskSourceKind::Timer,
            RendererPageTaskSourceKind::DomManipulation,
            RendererPageTaskSourceKind::WindowMessage,
            RendererPageTaskSourceKind::MessagePortDelivery,
        ] {
            for _ in 0..MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE {
                assert_eq!(
                    scheduler
                        .select_ready_descriptor(candidates())
                        .expect("dominant source should remain ready")
                        .source_kind(),
                    RendererPageTaskSourceKind::DynamicImportOwnerAction
                );
            }
            assert_eq!(
                scheduler
                    .select_ready_descriptor(candidates())
                    .expect("fairness override should select another source")
                    .source_kind(),
                expected_override
            );
        }
    }

    #[test]
    fn fairness_override_keeps_oldest_head_within_a_shared_source_kind() {
        let mut scheduler = PageTurnScheduler::new(());
        let base = Instant::now();
        let candidates = || {
            [
                dynamic_import_descriptor(base, 1),
                websocket_descriptor(base + std::time::Duration::from_millis(2), 3),
                resource_descriptor(base + std::time::Duration::from_millis(1), 2),
            ]
        };
        for _ in 0..MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE {
            assert_eq!(
                scheduler
                    .select_ready_descriptor(candidates())
                    .expect("dominant source should remain ready")
                    .source_kind(),
                RendererPageTaskSourceKind::DynamicImportOwnerAction
            );
        }

        assert_eq!(
            scheduler
                .select_ready_descriptor(candidates())
                .expect("fairness override should select Networking"),
            resource_descriptor(base + std::time::Duration::from_millis(1), 2),
            "separate physical sources sharing Networking must preserve their oldest head"
        );
    }

    #[test]
    fn fairness_history_is_isolated_per_page_scheduler() {
        let base = Instant::now();
        let candidates = || {
            [
                dynamic_import_descriptor(base, 1),
                modulepreload_descriptor(base + std::time::Duration::from_millis(1), 2),
            ]
        };
        let mut first = PageTurnScheduler::new(1_u8);
        let mut second = PageTurnScheduler::new(2_u8);
        for _ in 0..MAX_CONSECUTIVE_PAGE_TASKS_PER_SOURCE {
            let _ = first.select_ready_descriptor(candidates());
        }

        assert_eq!(
            first
                .select_ready_descriptor(candidates())
                .expect("first Page should apply its fairness bound")
                .source_kind(),
            RendererPageTaskSourceKind::ModulepreloadStart
        );
        assert_eq!(
            second
                .select_ready_descriptor(candidates())
                .expect("second Page should have independent arbitration state")
                .source_kind(),
            RendererPageTaskSourceKind::DynamicImportOwnerAction
        );
    }
}
