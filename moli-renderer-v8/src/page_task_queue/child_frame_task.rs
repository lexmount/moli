use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{
        FrameDocumentClassicScriptSourceLoadTask, FrameDocumentLifecycleAction,
        FrameDocumentLoadDeliveryAdmission, FrameDocumentLoadDeliveryTask,
        FrameDocumentParserModuleRootStartTask, FrameDocumentTaskOwner, FrameRealmId,
    },
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::{PageOwnerTurnOutcome, RendererDocumentToken},
};

use super::{RendererOwnerWakeSender, RendererPageChildRealmMaterializationTarget};

/// PageVm-local ledger key for one child document-script execution task.
///
/// The stable Page source owns ordering and exact authorization metadata. The
/// V8/DOM payload remains in the PageVm that created this id and can only be
/// claimed with the complete target below.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageChildDocumentScriptReadyTaskId(u64);

impl RendererPageChildDocumentScriptReadyTaskId {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildDocumentScriptReadyTarget {
    child_handle: Option<DomHandle>,
    document_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    task_id: RendererPageChildDocumentScriptReadyTaskId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildParserModuleRootStartTarget {
    child_handle: DomHandle,
    document_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildClassicScriptSourceLoadTarget {
    child_handle: DomHandle,
    document_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    script_handle: DomHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildDocumentLifecycleTarget {
    action: FrameDocumentLifecycleAction,
    realm_id: FrameRealmId,
}

impl RendererPageChildDocumentLifecycleTarget {
    pub(crate) const fn new(action: FrameDocumentLifecycleAction, realm_id: FrameRealmId) -> Self {
        Self { action, realm_id }
    }

    pub(crate) const fn action(self) -> FrameDocumentLifecycleAction {
        self.action
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.action.child_handle()
    }

    pub(crate) fn document_owner(self) -> FrameDocumentTaskOwner {
        self.action.owner()
    }

    pub(crate) const fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildHostLoadTarget {
    admission: FrameDocumentLoadDeliveryAdmission,
}

impl RendererPageChildHostLoadTarget {
    pub(crate) const fn new(admission: FrameDocumentLoadDeliveryAdmission) -> Self {
        Self { admission }
    }

    pub(crate) const fn admission(self) -> FrameDocumentLoadDeliveryAdmission {
        self.admission
    }

    pub(crate) const fn task(self) -> FrameDocumentLoadDeliveryTask {
        self.admission.task()
    }

    #[cfg(test)]
    pub(crate) const fn child_handle(self) -> DomHandle {
        self.task().child_handle
    }

    #[cfg(test)]
    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.task().owner
    }
}

impl RendererPageChildClassicScriptSourceLoadTarget {
    pub(crate) const fn new(
        child_handle: DomHandle,
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
    ) -> Self {
        Self {
            child_handle,
            document_owner,
            realm_id,
            script_handle,
        }
    }

    pub(crate) const fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }

    pub(crate) const fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) const fn script_handle(self) -> DomHandle {
        self.script_handle
    }
}

impl RendererPageChildParserModuleRootStartTarget {
    pub(crate) const fn new(
        child_handle: DomHandle,
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> Self {
        Self {
            child_handle,
            document_owner,
            realm_id,
        }
    }

    pub(crate) const fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }

    pub(crate) const fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }
}

impl RendererPageChildDocumentScriptReadyTarget {
    pub(crate) const fn new(
        child_handle: Option<DomHandle>,
        document_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        task_id: RendererPageChildDocumentScriptReadyTaskId,
    ) -> Self {
        Self {
            child_handle,
            document_owner,
            realm_id,
            task_id,
        }
    }

    pub(crate) const fn child_handle(self) -> Option<DomHandle> {
        self.child_handle
    }

    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }

    pub(crate) const fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) const fn task_id(self) -> RendererPageChildDocumentScriptReadyTaskId {
        self.task_id
    }
}

/// Family-local child-frame action selected by the Page scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageChildFrameTaskTarget {
    RealmMaterialization(RendererPageChildRealmMaterializationTarget),
    DocumentLifecycle(RendererPageChildDocumentLifecycleTarget),
    DocumentScriptReady(RendererPageChildDocumentScriptReadyTarget),
    HostLoad(RendererPageChildHostLoadTarget),
    ParserModuleRootStart(RendererPageChildParserModuleRootStartTarget),
    ClassicScriptSourceLoad(RendererPageChildClassicScriptSourceLoadTarget),
}

impl RendererPageChildFrameTaskTarget {
    #[cfg(test)]
    pub(crate) fn child_handle(self) -> Option<DomHandle> {
        match self {
            Self::RealmMaterialization(target) => Some(target.child_handle()),
            Self::DocumentLifecycle(target) => Some(target.child_handle()),
            Self::DocumentScriptReady(target) => target.child_handle(),
            Self::HostLoad(target) => Some(target.child_handle()),
            Self::ParserModuleRootStart(target) => Some(target.child_handle()),
            Self::ClassicScriptSourceLoad(target) => Some(target.child_handle()),
        }
    }

    #[cfg(test)]
    pub(crate) fn document_owner(self) -> FrameDocumentTaskOwner {
        match self {
            Self::RealmMaterialization(target) => target.document_owner(),
            Self::DocumentLifecycle(target) => target.document_owner(),
            Self::DocumentScriptReady(target) => target.document_owner(),
            Self::HostLoad(target) => target.document_owner(),
            Self::ParserModuleRootStart(target) => target.document_owner(),
            Self::ClassicScriptSourceLoad(target) => target.document_owner(),
        }
    }
}

/// PageVm namespace plus one exact child-frame action target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildFrameTaskOwner {
    root_document: RendererDocumentToken,
    target: RendererPageChildFrameTaskTarget,
}

impl RendererPageChildFrameTaskOwner {
    pub(crate) const fn new(
        root_document: RendererDocumentToken,
        target: RendererPageChildFrameTaskTarget,
    ) -> Self {
        Self {
            root_document,
            target,
        }
    }

    pub(crate) const fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) const fn target(self) -> RendererPageChildFrameTaskTarget {
        self.target
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageChildFrameTask {
    owner: RendererPageChildFrameTaskOwner,
    payload: RendererPageChildFrameTaskPayload,
}

#[derive(Debug)]
enum RendererPageChildFrameTaskPayload {
    None,
    ParserModuleRootStart(Box<FrameDocumentParserModuleRootStartTask>),
    ClassicScriptSourceLoad(Box<FrameDocumentClassicScriptSourceLoadTask>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildDocumentScriptReadyTargetEffect {
    /// One exact classic or module task entered V8 or dispatched its terminal
    /// event. Any module error-handling checkpoint has already happened inside
    /// the module algorithm; the selected Page-task dispatcher still owns the
    /// enclosing HTML task's callback completion.
    AppliedScriptOrEventToCurrentOwner { made_progress: bool },
    /// One exact task settled scheduler/parser state without entering script
    /// or event code. It still owns the ordinary task-end checkpoint, but must
    /// not flush unrelated callback follow-up work.
    AppliedWithoutScriptOrEventToCurrentOwner { made_progress: bool },
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildFrameTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildDocumentScriptReadyTurnAction {
    pub(crate) owner: RendererPageChildFrameTaskOwner,
    pub(crate) task_id: RendererPageChildDocumentScriptReadyTaskId,
    pub(crate) target_effect: PageChildDocumentScriptReadyTargetEffect,
}

pub(crate) type PageChildDocumentScriptReadyTurnOutcome =
    PageOwnerTurnOutcome<PageChildDocumentScriptReadyTurnAction>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildDocumentLifecycleTargetEffect {
    EventDispatchedToCurrentOwner,
    ConsumedCurrentOwnerWithoutEvent,
    FailedForCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildFrameTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildDocumentLifecycleTurnAction {
    pub(crate) owner: RendererPageChildFrameTaskOwner,
    pub(crate) target_effect: PageChildDocumentLifecycleTargetEffect,
}

pub(crate) type PageChildDocumentLifecycleTurnOutcome =
    PageOwnerTurnOutcome<PageChildDocumentLifecycleTurnAction>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildHostLoadTargetEffect {
    CallbackDispatchedToCurrentOwner,
    ConsumedCurrentOwnerWithoutCallback,
    FailedForCurrentOwner,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildFrameTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildHostLoadTurnAction {
    pub(crate) owner: RendererPageChildFrameTaskOwner,
    pub(crate) target_effect: PageChildHostLoadTargetEffect,
}

pub(crate) type PageChildHostLoadTurnOutcome = PageOwnerTurnOutcome<PageChildHostLoadTurnAction>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildParserModuleRootStartTargetEffect {
    /// The exact current child Document consumed its root-start reservation.
    /// The body may compile an inline root, start or join an external graph,
    /// or publish a typed failure successor, but it never evaluates module code
    /// or dispatches a callback.
    ConsumedByCurrentOwner,
    /// The claim no longer names the current root Page/child Document/realm.
    /// Stale retirement is bookkeeping, not a task in the current realm.
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildFrameTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildParserModuleRootStartTurnAction {
    pub(crate) owner: RendererPageChildFrameTaskOwner,
    pub(crate) target_effect: PageChildParserModuleRootStartTargetEffect,
}

pub(crate) type PageChildParserModuleRootStartTurnOutcome =
    PageOwnerTurnOutcome<PageChildParserModuleRootStartTurnAction>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageChildClassicScriptSourceLoadTargetEffect {
    /// The exact current child request entered the owner network bridge. The
    /// fetched source remains a later Networking terminal and no script or
    /// event ran in this body.
    NetworkRequestStartedForCurrentOwner,
    /// The exact current request could not enter the network bridge and ran
    /// its typed pre-start failure path. Any parser/script successor remains a
    /// separately selected task.
    RejectedBeforeNetworkStartForCurrentOwner,
    /// The claim no longer names the current root Page/child Document/realm.
    /// Retiring the old reservation is not a task in the replacement realm.
    DiscardedStaleOwner {
        current_owner: Option<RendererPageChildFrameTaskOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageChildClassicScriptSourceLoadTurnAction {
    pub(crate) owner: RendererPageChildFrameTaskOwner,
    pub(crate) target_effect: PageChildClassicScriptSourceLoadTargetEffect,
}

pub(crate) type PageChildClassicScriptSourceLoadTurnOutcome =
    PageOwnerTurnOutcome<PageChildClassicScriptSourceLoadTurnAction>;

impl RendererPageChildFrameTask {
    fn new(owner: RendererPageChildFrameTaskOwner) -> Self {
        debug_assert!(!matches!(
            owner.target(),
            RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
                | RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
        ));
        Self {
            owner,
            payload: RendererPageChildFrameTaskPayload::None,
        }
    }

    fn new_parser_module_root_start(
        root_document: RendererDocumentToken,
        target: RendererPageChildParserModuleRootStartTarget,
        task: FrameDocumentParserModuleRootStartTask,
    ) -> Self {
        assert_eq!(
            target.child_handle(),
            task.child_handle(),
            "parser module root target and payload must name the same child"
        );
        assert_eq!(
            target.document_owner(),
            task.owner(),
            "parser module root target and payload must name the same Document owner"
        );
        Self {
            owner: RendererPageChildFrameTaskOwner::new(
                root_document,
                RendererPageChildFrameTaskTarget::ParserModuleRootStart(target),
            ),
            payload: RendererPageChildFrameTaskPayload::ParserModuleRootStart(Box::new(task)),
        }
    }

    fn new_classic_script_source_load(
        root_document: RendererDocumentToken,
        target: RendererPageChildClassicScriptSourceLoadTarget,
        task: FrameDocumentClassicScriptSourceLoadTask,
    ) -> Self {
        assert_eq!(
            target.child_handle(),
            task.child_handle(),
            "classic source-load target and payload must name the same child"
        );
        assert_eq!(
            target.document_owner(),
            task.owner(),
            "classic source-load target and payload must name the same Document owner"
        );
        assert_eq!(
            target.realm_id(),
            task.realm_id(),
            "classic source-load target and payload must name the same realm"
        );
        assert_eq!(
            target.script_handle(),
            task.client().metadata().script_handle(),
            "classic source-load target and payload must name the same script"
        );
        Self {
            owner: RendererPageChildFrameTaskOwner::new(
                root_document,
                RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(target),
            ),
            payload: RendererPageChildFrameTaskPayload::ClassicScriptSourceLoad(Box::new(task)),
        }
    }

    pub(crate) const fn owner(&self) -> RendererPageChildFrameTaskOwner {
        self.owner
    }

    pub(crate) fn into_parser_module_root_start_task(
        self,
    ) -> FrameDocumentParserModuleRootStartTask {
        match self.payload {
            RendererPageChildFrameTaskPayload::ParserModuleRootStart(task) => *task,
            RendererPageChildFrameTaskPayload::None
            | RendererPageChildFrameTaskPayload::ClassicScriptSourceLoad(_) => {
                unreachable!("parser-module-root executor received a payload-free child task")
            }
        }
    }

    pub(crate) fn into_classic_script_source_load_task(
        self,
    ) -> FrameDocumentClassicScriptSourceLoadTask {
        match self.payload {
            RendererPageChildFrameTaskPayload::ClassicScriptSourceLoad(task) => *task,
            RendererPageChildFrameTaskPayload::None
            | RendererPageChildFrameTaskPayload::ParserModuleRootStart(_) => {
                unreachable!("classic-source executor received another child task payload")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageChildFrameTaskRouteClosed;

#[derive(Debug)]
pub(crate) struct RendererPageChildParserModuleRootStartRouteClosed(
    Box<FrameDocumentParserModuleRootStartTask>,
);

impl RendererPageChildParserModuleRootStartRouteClosed {
    pub(crate) fn into_task(self) -> FrameDocumentParserModuleRootStartTask {
        *self.0
    }
}

#[derive(Debug)]
pub(crate) struct RendererPageChildClassicScriptSourceLoadRouteClosed(
    Box<FrameDocumentClassicScriptSourceLoadTask>,
);

impl RendererPageChildClassicScriptSourceLoadRouteClosed {
    pub(crate) fn into_task(self) -> FrameDocumentClassicScriptSourceLoadTask {
        *self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageChildFrameTaskRoute {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageChildFrameTask>, ChildFrameTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageChildFrameTaskRoute {
    pub(crate) fn sender(
        &self,
        root_document: RendererDocumentToken,
    ) -> RendererPageChildFrameTaskSender {
        RendererPageChildFrameTaskSender {
            task_route: self.task_route.clone(),
            owner_wake: self.owner_wake.clone(),
            root_document,
        }
    }

    fn same_route_as(&self, source: &RendererPageChildFrameTaskSource) -> bool {
        self.task_route.same_source_as(&source.source)
    }
}

/// PageVm-stamped producer for every scheduler-visible child-frame task kind.
#[derive(Clone, Debug)]
pub(crate) struct RendererPageChildFrameTaskSender {
    task_route:
        OwnerReadyTaskRoute<ReadyPageTask<RendererPageChildFrameTask>, ChildFrameTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
    root_document: RendererDocumentToken,
}

impl RendererPageChildFrameTaskSender {
    pub(crate) fn send_realm_materialization(
        &self,
        target: RendererPageChildRealmMaterializationTarget,
    ) -> Result<(), RendererPageChildFrameTaskRouteClosed> {
        self.send(RendererPageChildFrameTaskTarget::RealmMaterialization(
            target,
        ))
    }

    pub(crate) fn send_document_script_ready(
        &self,
        target: RendererPageChildDocumentScriptReadyTarget,
    ) -> Result<(), RendererPageChildFrameTaskRouteClosed> {
        self.send(RendererPageChildFrameTaskTarget::DocumentScriptReady(
            target,
        ))
    }

    pub(crate) fn send_document_lifecycle(
        &self,
        target: RendererPageChildDocumentLifecycleTarget,
    ) -> Result<(), RendererPageChildFrameTaskRouteClosed> {
        self.send(RendererPageChildFrameTaskTarget::DocumentLifecycle(target))
    }

    pub(crate) fn send_host_load(
        &self,
        target: RendererPageChildHostLoadTarget,
    ) -> Result<(), RendererPageChildFrameTaskRouteClosed> {
        self.send(RendererPageChildFrameTaskTarget::HostLoad(target))
    }

    pub(crate) fn send_parser_module_root_start(
        &self,
        target: RendererPageChildParserModuleRootStartTarget,
        task: FrameDocumentParserModuleRootStartTask,
    ) -> Result<(), RendererPageChildParserModuleRootStartRouteClosed> {
        let task = RendererPageChildFrameTask::new_parser_module_root_start(
            self.root_document,
            target,
            task,
        );
        if let Err(error) = self
            .task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
        {
            let (_, task) = error.0.into_parts();
            return Err(RendererPageChildParserModuleRootStartRouteClosed(Box::new(
                task.into_parser_module_root_start_task(),
            )));
        }
        Ok(())
    }

    pub(crate) fn send_classic_script_source_load(
        &self,
        target: RendererPageChildClassicScriptSourceLoadTarget,
        task: FrameDocumentClassicScriptSourceLoadTask,
    ) -> Result<(), RendererPageChildClassicScriptSourceLoadRouteClosed> {
        let task = RendererPageChildFrameTask::new_classic_script_source_load(
            self.root_document,
            target,
            task,
        );
        if let Err(error) = self
            .task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(task))
        {
            let (_, task) = error.0.into_parts();
            return Err(RendererPageChildClassicScriptSourceLoadRouteClosed(
                Box::new(task.into_classic_script_source_load_task()),
            ));
        }
        Ok(())
    }

    fn send(
        &self,
        target: RendererPageChildFrameTaskTarget,
    ) -> Result<(), RendererPageChildFrameTaskRouteClosed> {
        let owner = RendererPageChildFrameTaskOwner::new(self.root_document, target);
        self.task_route
            .send_and_signal_if_newly_ready(ReadyPageTask::new(RendererPageChildFrameTask::new(
                owner,
            )))
            .map_err(|_| RendererPageChildFrameTaskRouteClosed)
    }

    /// Publish admission after a child owner transition changes head
    /// eligibility without creating a duplicate task.
    pub(crate) fn signal_reconsideration(&self) {
        self.owner_wake.signal_child_frame_task();
    }
}

#[derive(Clone, Debug)]
struct ChildFrameTaskReadySignal {
    owner_wake: RendererOwnerWakeSender,
}

impl OwnerTaskReadySignal for ChildFrameTaskReadySignal {
    fn signal_ready(&self) {
        self.owner_wake.signal_child_frame_task();
    }
}

/// Unique Page-lifetime consumer for the child-frame task family.
#[derive(Debug)]
pub(crate) struct RendererPageChildFrameTaskSource {
    source:
        OwnerReadyTaskSource<ReadyPageTask<RendererPageChildFrameTask>, ChildFrameTaskReadySignal>,
    owner_wake: RendererOwnerWakeSender,
}

impl RendererPageChildFrameTaskSource {
    pub(crate) fn new(owner_wake: RendererOwnerWakeSender) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(ChildFrameTaskReadySignal {
                owner_wake: owner_wake.clone(),
            }),
            owner_wake,
        }
    }

    pub(crate) fn route(&self) -> RendererPageChildFrameTaskRoute {
        RendererPageChildFrameTaskRoute {
            task_route: self.source.route(),
            owner_wake: self.owner_wake.clone(),
        }
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.source.front().map(ReadyPageTask::metadata)
    }

    pub(crate) fn next_ready_owner(&mut self) -> Option<RendererPageChildFrameTaskOwner> {
        self.source.front().map(|ready| ready.value().owner())
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(RendererPageTaskReadyMetadata, RendererPageChildFrameTask)> {
        self.source.pop_front().map(ReadyPageTask::into_parts)
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        !self.source.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.source.clear_local();
    }

    pub(crate) fn route_matches(&self, route: &RendererPageChildFrameTaskRoute) -> bool {
        route.same_route_as(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        dom::NodeId,
        frame_owner_model::{
            DocumentId, DocumentLoadDelayTokenId, FrameSchedulerLaneId, LocalWindowId,
            frame_document_classic_script_source_load_client_action,
        },
        parser_script::{
            action::ParserPendingClassicScriptSourceLoadClientAction,
            payload::ParserClassicScriptMetadata,
        },
        planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
        runtime::RendererPageToken,
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };

    fn root_document() -> RendererDocumentToken {
        RendererDocumentToken::new_for_testing(PageId::new_for_testing(7), 11)
    }

    fn parser_root_task(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> FrameDocumentParserModuleRootStartTask {
        let script_url = url::Url::parse("https://child-frame-source.test/root.mjs")
            .expect("parser module URL should parse");
        let script = PreparedScript {
            position: 3,
            node_id: NodeId::new(41),
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleDefer,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: script_url.clone(),
            base_url: script_url.clone(),
            initiator_url: script_url,
            host_script_handle: None,
        };
        let pending_script_id = crate::document_script_scheduler::ParserPendingScriptId::new(
            owner.document_owner(),
            &script,
        );
        FrameDocumentParserModuleRootStartTask::from_parser_script_parts(
            child_handle,
            owner,
            pending_script_id,
            DomHandle::new(41),
            script,
            DocumentLoadDelayTokenId(53),
        )
    }

    fn inline_parser_root_task(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> FrameDocumentParserModuleRootStartTask {
        let document_url = url::Url::parse("https://child-frame-source.test/document")
            .expect("inline parser module document URL should parse");
        let script = PreparedScript {
            position: 3,
            node_id: NodeId::new(43),
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleDefer,
            source_kind: ScriptSourceKind::Inline,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::Inline("export const value = 1;".to_owned()),
            url: document_url.clone(),
            base_url: document_url.clone(),
            initiator_url: document_url,
            host_script_handle: None,
        };
        let pending_script_id = crate::document_script_scheduler::ParserPendingScriptId::new(
            owner.document_owner(),
            &script,
        );
        FrameDocumentParserModuleRootStartTask::from_parser_script_parts(
            child_handle,
            owner,
            pending_script_id,
            DomHandle::new(43),
            script,
            DocumentLoadDelayTokenId(61),
        )
    }

    fn classic_source_load_task(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
    ) -> FrameDocumentClassicScriptSourceLoadTask {
        let script_url = url::Url::parse("https://child-frame-source.test/parser-classic.js")
            .expect("classic script URL should parse");
        let client = frame_document_classic_script_source_load_client_action(
            child_handle,
            owner.document_owner(),
            ParserPendingClassicScriptSourceLoadClientAction::new(
                ParserClassicScriptMetadata::new(script_handle, 7),
                &script_url,
            ),
        );
        FrameDocumentClassicScriptSourceLoadTask::from_source_load_client(owner, realm_id, client)
    }

    fn child_source() -> RendererPageChildFrameTaskSource {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        RendererPageChildFrameTaskSource::new(RendererOwnerWakeSender::new(
            wake_tx,
            RendererPageToken::new_for_testing(root_document().page_id),
        ))
    }

    #[test]
    fn parser_root_payload_follows_its_realm_prerequisite_in_family_fifo() {
        let mut source = child_source();
        let sender = source.route().sender(root_document());
        let child_handle = DomHandle::new(17);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(19),
            LocalWindowId(23),
            DocumentId(29),
        );
        let realm_id = FrameRealmId(31);

        sender
            .send_realm_materialization(RendererPageChildRealmMaterializationTarget::new(
                child_handle,
                owner,
            ))
            .expect("realm prerequisite should enter the child-frame family");
        sender
            .send_parser_module_root_start(
                RendererPageChildParserModuleRootStartTarget::new(child_handle, owner, realm_id),
                parser_root_task(child_handle, owner),
            )
            .expect("parser root should enter the child-frame family");

        let (_, realm_task) = source.pop_front().expect("realm task should be first");
        assert!(matches!(
            realm_task.owner().target(),
            RendererPageChildFrameTaskTarget::RealmMaterialization(_)
        ));
        let (_, parser_task) = source.pop_front().expect("parser root should be second");
        assert_eq!(
            parser_task.owner().target(),
            RendererPageChildFrameTaskTarget::ParserModuleRootStart(
                RendererPageChildParserModuleRootStartTarget::new(child_handle, owner, realm_id,)
            )
        );
        let parser_task = parser_task.into_parser_module_root_start_task();
        assert_eq!(parser_task.child_handle(), child_handle);
        assert_eq!(parser_task.owner(), owner);
        assert!(!source.has_ready_task());
    }

    #[test]
    fn inline_parser_root_remains_loaded_work_in_the_typed_family() {
        let child_handle = DomHandle::new(67);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(71),
            LocalWindowId(73),
            DocumentId(79),
        );
        let task = inline_parser_root_task(child_handle, owner);

        assert!(matches!(
            task.kind(),
            crate::frame_owner_model::FrameDocumentParserModuleRootStartKind::LoadedSource(
                crate::module_runtime::ModuleSource::Text(source)
            ) if source == "export const value = 1;"
        ));
        assert_eq!(task.child_handle(), child_handle);
        assert_eq!(task.owner(), owner);
    }

    #[test]
    fn classic_fetch_start_precedes_its_execution_realm_in_family_fifo() {
        let mut source = child_source();
        let sender = source.route().sender(root_document());
        let child_handle = DomHandle::new(83);
        let script_handle = DomHandle::new(89);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(97),
            LocalWindowId(101),
            DocumentId(103),
        );
        let realm_id = FrameRealmId(107);
        let task = classic_source_load_task(child_handle, owner, realm_id, script_handle);

        sender
            .send_classic_script_source_load(
                RendererPageChildClassicScriptSourceLoadTarget::new(
                    child_handle,
                    owner,
                    realm_id,
                    script_handle,
                ),
                task,
            )
            .expect("classic source start should enter the child-frame family");
        sender
            .send_realm_materialization(RendererPageChildRealmMaterializationTarget::new(
                child_handle,
                owner,
            ))
            .expect("execution realm should enter the child-frame family");

        let (_, classic_task) = source
            .pop_front()
            .expect("classic source start should be first");
        assert_eq!(
            classic_task.owner().target(),
            RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(
                RendererPageChildClassicScriptSourceLoadTarget::new(
                    child_handle,
                    owner,
                    realm_id,
                    script_handle,
                )
            )
        );
        let classic_task = classic_task.into_classic_script_source_load_task();
        assert_eq!(classic_task.owner(), owner);
        assert_eq!(classic_task.realm_id(), realm_id);
        assert_eq!(classic_task.child_handle(), child_handle);
        assert_eq!(
            classic_task.client().metadata().script_handle(),
            script_handle
        );
        assert_eq!(
            classic_task.client().script_url().as_str(),
            "https://child-frame-source.test/parser-classic.js"
        );
        let (_, realm_task) = source.pop_front().expect("realm task should be second");
        assert!(matches!(
            realm_task.owner().target(),
            RendererPageChildFrameTaskTarget::RealmMaterialization(_)
        ));
        assert!(!source.has_ready_task());
    }

    #[test]
    fn closed_child_frame_route_returns_the_exact_parser_root_payload() {
        let source = child_source();
        let sender = source.route().sender(root_document());
        let child_handle = DomHandle::new(37);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(41),
            LocalWindowId(43),
            DocumentId(47),
        );
        let task = parser_root_task(child_handle, owner);
        let expected_pending_script = task.pending_script_id();
        drop(source);

        let returned = sender
            .send_parser_module_root_start(
                RendererPageChildParserModuleRootStartTarget::new(
                    child_handle,
                    owner,
                    FrameRealmId(59),
                ),
                task,
            )
            .expect_err("closed child-frame route must reject parser root")
            .into_task();
        assert_eq!(returned.child_handle(), child_handle);
        assert_eq!(returned.owner(), owner);
        assert_eq!(returned.pending_script_id(), expected_pending_script);
    }

    #[test]
    fn closed_child_frame_route_returns_the_exact_classic_source_payload() {
        let source = child_source();
        let sender = source.route().sender(root_document());
        let child_handle = DomHandle::new(109);
        let script_handle = DomHandle::new(113);
        let owner = FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(127),
            LocalWindowId(131),
            DocumentId(137),
        );
        let realm_id = FrameRealmId(139);
        let task = classic_source_load_task(child_handle, owner, realm_id, script_handle);
        drop(source);

        let returned = sender
            .send_classic_script_source_load(
                RendererPageChildClassicScriptSourceLoadTarget::new(
                    child_handle,
                    owner,
                    realm_id,
                    script_handle,
                ),
                task,
            )
            .expect_err("closed child-frame route must reject classic source start")
            .into_task();
        assert_eq!(returned.child_handle(), child_handle);
        assert_eq!(returned.owner(), owner);
        assert_eq!(returned.realm_id(), realm_id);
        assert_eq!(returned.client().metadata().script_handle(), script_handle);
    }
}
