use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::{
    DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyActionRoute,
    FrameDocumentClassicScriptSchedulerWork, FrameDocumentModuleScriptReadyWork,
    FrameDocumentReadyActionRoute, FrameDocumentScriptReadyWork,
};
use crate::frame_owner_model::{
    ChildDocumentAsyncClassicScriptLoadDelay, FrameDocumentNavigationLoadBinding,
    FrameDocumentOwner, FrameDocumentOwnerTransition, FrameDocumentScriptElementEventKind,
    FrameDocumentTaskOwner, FrameRealmId, FrameScriptJob,
};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingChildExternalClassicDocumentScript {
    pub(crate) child_handle: DomHandle,
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) realm_id: Option<FrameRealmId>,
    pub(crate) script_handle: DomHandle,
    pub(crate) load_delay: ChildDocumentAsyncClassicScriptLoadDelay,
    pub(crate) source_result: Result<String, String>,
    pub(crate) script_url: Url,
    pub(crate) script_base_url: Url,
}

impl DocumentScriptReadyActionRoute<FrameDocumentOwner>
    for PendingChildExternalClassicDocumentScript
{
    fn payload_document_owner(&self) -> FrameDocumentOwner {
        self.owner.document_owner()
    }
}

impl DocumentScriptReadyActionDispatchRoute<FrameDocumentReadyActionRoute>
    for PendingChildExternalClassicDocumentScript
{
    fn dispatch_route(&self) -> FrameDocumentReadyActionRoute {
        FrameDocumentReadyActionRoute::from_frame_document_parts(
            Some(self.child_handle),
            self.owner,
            self.realm_id,
            self.realm_id.is_some(),
            self.script_handle,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingChildJavascriptUrlDocumentScript {
    pub(crate) child_handle: DomHandle,
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) realm_id: Option<FrameRealmId>,
    pub(crate) navigation_load: FrameDocumentNavigationLoadBinding,
    pub(crate) url: Url,
    pub(crate) source: String,
    pub(crate) preserve_window_event_state: bool,
    pub(crate) dispatch_load_on_no_string_completion: bool,
}

/// Exact-Document script work produced before its child realm is executable.
///
/// A prebootstrapped context is already a valid LocalWindow execution context,
/// but it is not yet registered in the ScriptVm realm store. The producer
/// first publishes the realm-materialization prerequisite, then
/// binds this work to that reserved realm and appends it to the same stable
/// child-frame FIFO. It therefore stays durable without becoming executable
/// before the prerequisite turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameDocumentUnboundScriptWork {
    ExternalClassic(PendingChildExternalClassicDocumentScript),
    JavascriptUrl(PendingChildJavascriptUrlDocumentScript),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameDocumentRealmBoundScriptWork {
    ExternalClassic(PendingChildExternalClassicDocumentScript),
    JavascriptUrl(PendingChildJavascriptUrlDocumentScript),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDocumentScriptWorkAdmission {
    QueuedBehindRealm,
    Runnable,
}

impl FrameDocumentScriptWorkAdmission {
    pub(crate) const fn is_runnable(self) -> bool {
        matches!(self, Self::Runnable)
    }
}

impl FrameDocumentUnboundScriptWork {
    pub(crate) fn child_handle(&self) -> DomHandle {
        match self {
            Self::ExternalClassic(work) => work.child_handle,
            Self::JavascriptUrl(work) => work.child_handle,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        match self {
            Self::ExternalClassic(work) => work.owner,
            Self::JavascriptUrl(work) => work.owner,
        }
    }

    pub(crate) fn expected_realm_id(&self) -> Option<FrameRealmId> {
        match self {
            Self::ExternalClassic(work) => work.realm_id,
            Self::JavascriptUrl(work) => work.realm_id,
        }
    }

    pub(crate) fn can_materialize_in_realm(&self, realm_id: FrameRealmId) -> bool {
        self.expected_realm_id()
            .is_none_or(|expected| expected == realm_id)
    }

    pub(crate) fn bind_to_realm(self, realm_id: FrameRealmId) -> FrameDocumentRealmBoundScriptWork {
        debug_assert!(self.can_materialize_in_realm(realm_id));
        match self {
            Self::ExternalClassic(mut work) => {
                work.realm_id = Some(realm_id);
                FrameDocumentRealmBoundScriptWork::ExternalClassic(work)
            }
            Self::JavascriptUrl(mut work) => {
                work.realm_id = Some(realm_id);
                FrameDocumentRealmBoundScriptWork::JavascriptUrl(work)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum PendingChildDocumentScriptExecutionWork {
    ExternalClassic(PendingChildExternalClassicDocumentScript),
    JavascriptUrl(PendingChildJavascriptUrlDocumentScript),
    ModuleScript(Box<FrameDocumentModuleScriptReadyWork>),
}

/// One exact child-Document script task bound to the realm that must authorize
/// its eventual execution.
///
/// The stable Page source owns FIFO and authorization metadata. This payload
/// stays in the PageVm-local Host ledger because it may contain V8/DOM-bound
/// state; it is claimed only by the selected ChildFrameTask turn.
#[derive(Debug)]
pub(crate) enum FrameDocumentScriptReadyTaskWork {
    Scheduler(FrameDocumentScriptReadyWork),
    DocumentScriptExecution(Box<FrameDocumentRealmBoundScriptWork>),
}

impl FrameDocumentScriptReadyTaskWork {
    pub(crate) fn route(&self) -> FrameDocumentReadyActionRoute {
        match self {
            Self::Scheduler(work) => work.dispatch_route(),
            Self::DocumentScriptExecution(work) => match work.as_ref() {
                FrameDocumentRealmBoundScriptWork::ExternalClassic(work) => work.dispatch_route(),
                FrameDocumentRealmBoundScriptWork::JavascriptUrl(work) => {
                    FrameDocumentReadyActionRoute::from_frame_document_parts(
                        Some(work.child_handle),
                        work.owner,
                        work.realm_id,
                        true,
                        work.child_handle,
                    )
                }
            },
        }
    }
}

impl From<FrameDocumentRealmBoundScriptWork> for FrameDocumentScriptReadyTaskWork {
    fn from(work: FrameDocumentRealmBoundScriptWork) -> Self {
        Self::DocumentScriptExecution(Box::new(work))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentExternalClassicScriptExecutionTarget {
    child_handle: DomHandle,
    task_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    script_handle: DomHandle,
    load_delay: ChildDocumentAsyncClassicScriptLoadDelay,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentExternalClassicScriptExecutionAction {
    target: FrameDocumentExternalClassicScriptExecutionTarget,
    execution: FrameDocumentExternalClassicScriptExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentJavascriptUrlScriptExecutionTarget {
    child_handle: DomHandle,
    task_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    navigation_load: FrameDocumentNavigationLoadBinding,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentJavascriptUrlScriptExecutionAction {
    target: FrameDocumentJavascriptUrlScriptExecutionTarget,
    job: FrameScriptJob,
    url: Url,
    preserve_window_event_state: bool,
    dispatch_load_on_no_string_completion: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum FrameDocumentExternalClassicScriptExecution {
    ScriptJob(Box<FrameScriptJob>),
    SourceFailure { message: String },
}

#[derive(Debug)]
pub(crate) enum FrameDocumentScriptExecutionWork {
    ExternalClassic(FrameDocumentExternalClassicScriptExecutionAction),
    JavascriptUrl(Box<FrameDocumentJavascriptUrlScriptExecutionAction>),
    ModuleScript(Box<FrameDocumentModuleScriptReadyWork>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameDocumentScriptExecutionResult {
    ExternalClassic(FrameDocumentExternalClassicExecutionResult),
    JavascriptUrl(FrameDocumentJavascriptUrlExecutionResult),
    ModuleScript(
        crate::document_script_scheduler::FrameModuleScriptRunOutcome<
            crate::document_script_scheduler::DocumentScriptExecutionOutcome,
        >,
    ),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDocumentScriptPrepareFollowup {
    ExternalClassic(FrameDocumentExternalClassicPrepareFollowup),
    JavascriptUrl(FrameDocumentJavascriptUrlPrepareFollowup),
    ModuleScript(crate::document_script_scheduler::DocumentScriptExecutionOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameDocumentScriptExecutionFollowup {
    ExternalClassic(FrameDocumentExternalClassicPostExecutionAction),
    JavascriptUrl(FrameDocumentJavascriptUrlPostExecutionAction),
    ModuleScript(
        crate::document_script_scheduler::FrameModuleScriptRunOutcome<
            crate::document_script_scheduler::DocumentScriptExecutionOutcome,
        >,
    ),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDocumentExternalClassicPrepareSkipReason {
    RealmMaterializationFailed,
    MissingCurrentRealm,
    StaleRealm {
        expected: FrameRealmId,
        current: FrameRealmId,
    },
    ExecutionActionUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentExternalClassicPrepareFollowup {
    prepared_execution_action: bool,
    skip_reason: Option<FrameDocumentExternalClassicPrepareSkipReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentExternalClassicExecutionFollowup {
    attempted_script_job: bool,
    failed_script_job: bool,
    source_failed: bool,
    script_event_dispatched: bool,
    lifecycle_followup_queued: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentExternalClassicExecutionResult {
    target: FrameDocumentExternalClassicScriptExecutionTarget,
    attempted_script_job: bool,
    failed_script_job: bool,
    source_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentExternalClassicPostExecutionAction {
    target: FrameDocumentExternalClassicScriptExecutionTarget,
    attempted_script_job: bool,
    failed_script_job: bool,
    source_failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDocumentJavascriptUrlPrepareSkipReason {
    RealmMaterializationFailed,
    MissingCurrentRealm,
    StaleRealm {
        expected: FrameRealmId,
        current: FrameRealmId,
    },
    ExecutionActionUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentJavascriptUrlPrepareFollowup {
    prepared_execution_action: bool,
    skip_reason: Option<FrameDocumentJavascriptUrlPrepareSkipReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameDocumentJavascriptUrlExecutionResult {
    target: FrameDocumentJavascriptUrlScriptExecutionTarget,
    url: Url,
    attempted_script_job: bool,
    completion: FrameDocumentJavascriptUrlCompletion,
    preserve_window_event_state: bool,
    dispatch_load_on_no_string_completion: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameDocumentJavascriptUrlCompletion {
    String(String),
    NonString,
    FailedScriptJob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameDocumentJavascriptUrlPostExecutionAction {
    target: FrameDocumentJavascriptUrlScriptExecutionTarget,
    url: Url,
    attempted_script_job: bool,
    completion: FrameDocumentJavascriptUrlCompletion,
    preserve_window_event_state: bool,
    dispatch_load_on_no_string_completion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentJavascriptUrlExecutionFollowup {
    attempted_script_job: bool,
    failed_script_job: bool,
    string_completion_committed: bool,
    lifecycle_followup_queued: bool,
}

#[derive(Debug)]
pub(crate) struct FrameDocumentJavascriptUrlPostExecutionApplication {
    pub(crate) attempted_script_job: bool,
    pub(crate) failed_script_job: bool,
    pub(crate) string_completion_committed: bool,
    pub(crate) lifecycle_followup_queued: bool,
    pub(crate) initial_classic_ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    pub(crate) owner_transition: Option<FrameDocumentOwnerTransition>,
}

impl PendingChildExternalClassicDocumentScript {
    pub(crate) fn execution_target(
        &self,
        realm_id: FrameRealmId,
    ) -> FrameDocumentExternalClassicScriptExecutionTarget {
        FrameDocumentExternalClassicScriptExecutionTarget::new(
            self.child_handle,
            self.owner,
            realm_id,
            self.script_handle,
            self.load_delay,
        )
    }
}

impl PendingChildJavascriptUrlDocumentScript {
    pub(crate) fn execution_target(
        &self,
        realm_id: FrameRealmId,
    ) -> FrameDocumentJavascriptUrlScriptExecutionTarget {
        FrameDocumentJavascriptUrlScriptExecutionTarget::new(
            self.child_handle,
            self.owner,
            realm_id,
            self.navigation_load,
        )
    }
}

impl FrameDocumentExternalClassicScriptExecutionTarget {
    pub(crate) fn new(
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
        load_delay: ChildDocumentAsyncClassicScriptLoadDelay,
    ) -> Self {
        Self {
            child_handle,
            task_owner,
            realm_id,
            script_handle,
            load_delay,
        }
    }

    pub(crate) fn child_handle(&self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        self.task_owner.document_owner()
    }

    pub(crate) fn task_owner(&self) -> FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.script_handle
    }

    pub(crate) fn load_delay(&self) -> ChildDocumentAsyncClassicScriptLoadDelay {
        self.load_delay
    }
}

impl FrameDocumentExternalClassicScriptExecutionAction {
    pub(crate) fn new(
        target: FrameDocumentExternalClassicScriptExecutionTarget,
        execution: FrameDocumentExternalClassicScriptExecution,
    ) -> Self {
        Self { target, execution }
    }

    pub(crate) fn target(&self) -> FrameDocumentExternalClassicScriptExecutionTarget {
        self.target
    }

    pub(crate) fn into_execution(self) -> FrameDocumentExternalClassicScriptExecution {
        self.execution
    }
}

impl FrameDocumentJavascriptUrlScriptExecutionTarget {
    pub(crate) fn new(
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        navigation_load: FrameDocumentNavigationLoadBinding,
    ) -> Self {
        Self {
            child_handle,
            task_owner,
            realm_id,
            navigation_load,
        }
    }

    pub(crate) fn child_handle(&self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        self.task_owner.document_owner()
    }

    pub(crate) fn task_owner(&self) -> FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) fn navigation_load(&self) -> FrameDocumentNavigationLoadBinding {
        self.navigation_load
    }
}

impl FrameDocumentJavascriptUrlScriptExecutionAction {
    pub(crate) fn new(
        target: FrameDocumentJavascriptUrlScriptExecutionTarget,
        job: FrameScriptJob,
        url: Url,
        preserve_window_event_state: bool,
        dispatch_load_on_no_string_completion: bool,
    ) -> Self {
        Self {
            target,
            job,
            url,
            preserve_window_event_state,
            dispatch_load_on_no_string_completion,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FrameDocumentJavascriptUrlScriptExecutionTarget,
        FrameScriptJob,
        Url,
        bool,
        bool,
    ) {
        (
            self.target,
            self.job,
            self.url,
            self.preserve_window_event_state,
            self.dispatch_load_on_no_string_completion,
        )
    }
}

impl FrameDocumentExternalClassicScriptExecution {
    pub(crate) fn script_job(job: FrameScriptJob) -> Self {
        Self::ScriptJob(Box::new(job))
    }

    pub(crate) fn source_failure(message: String) -> Self {
        Self::SourceFailure { message }
    }
}

impl FrameDocumentScriptExecutionWork {
    pub(crate) fn external_classic(
        action: FrameDocumentExternalClassicScriptExecutionAction,
    ) -> Self {
        Self::ExternalClassic(action)
    }

    pub(crate) fn javascript_url(action: FrameDocumentJavascriptUrlScriptExecutionAction) -> Self {
        Self::JavascriptUrl(Box::new(action))
    }

    pub(crate) fn module_script(work: FrameDocumentModuleScriptReadyWork) -> Self {
        Self::ModuleScript(Box::new(work))
    }
}

impl FrameDocumentScriptPrepareFollowup {
    pub(crate) fn made_progress(&self) -> bool {
        match self {
            Self::ExternalClassic(followup) => followup.made_progress(),
            Self::JavascriptUrl(followup) => followup.made_progress(),
            Self::ModuleScript(outcome) => outcome.made_progress(),
        }
    }
}

impl FrameDocumentExternalClassicPrepareFollowup {
    pub(crate) fn prepared_execution_action() -> Self {
        Self {
            prepared_execution_action: true,
            skip_reason: None,
        }
    }

    pub(crate) fn skipped(reason: FrameDocumentExternalClassicPrepareSkipReason) -> Self {
        Self {
            prepared_execution_action: false,
            skip_reason: Some(reason),
        }
    }

    pub(crate) fn made_progress(&self) -> bool {
        self.prepared_execution_action || self.skip_reason.is_some()
    }
}

impl FrameDocumentExternalClassicExecutionFollowup {
    pub(crate) fn new(
        attempted_script_job: bool,
        failed_script_job: bool,
        source_failed: bool,
        script_event_dispatched: bool,
        lifecycle_followup_queued: bool,
    ) -> Self {
        Self {
            attempted_script_job,
            failed_script_job,
            source_failed,
            script_event_dispatched,
            lifecycle_followup_queued,
        }
    }

    pub(crate) fn made_progress(&self) -> bool {
        self.attempted_script_job
            || self.source_failed
            || self.script_event_dispatched
            || self.lifecycle_followup_queued
    }

    pub(crate) fn script_or_event_was_dispatched(&self) -> bool {
        self.attempted_script_job || self.script_event_dispatched
    }
}

impl FrameDocumentExternalClassicExecutionResult {
    pub(crate) fn new(
        target: FrameDocumentExternalClassicScriptExecutionTarget,
        attempted_script_job: bool,
        failed_script_job: bool,
        source_failed: bool,
    ) -> Self {
        Self {
            target,
            attempted_script_job,
            failed_script_job,
            source_failed,
        }
    }

    pub(crate) fn into_post_execution_action(
        self,
    ) -> FrameDocumentExternalClassicPostExecutionAction {
        FrameDocumentExternalClassicPostExecutionAction {
            target: self.target,
            attempted_script_job: self.attempted_script_job,
            failed_script_job: self.failed_script_job,
            source_failed: self.source_failed,
        }
    }
}

impl FrameDocumentExternalClassicPostExecutionAction {
    pub(crate) fn target(&self) -> FrameDocumentExternalClassicScriptExecutionTarget {
        self.target
    }

    pub(crate) fn attempted_script_job(&self) -> bool {
        self.attempted_script_job
    }

    pub(crate) fn failed_script_job(&self) -> bool {
        self.failed_script_job
    }

    pub(crate) fn source_failed(&self) -> bool {
        self.source_failed
    }

    pub(crate) fn event_kind(&self) -> FrameDocumentScriptElementEventKind {
        if self.failed_script_job || self.source_failed {
            FrameDocumentScriptElementEventKind::Error
        } else {
            FrameDocumentScriptElementEventKind::Load
        }
    }
}

impl FrameDocumentJavascriptUrlPrepareFollowup {
    pub(crate) fn prepared_execution_action() -> Self {
        Self {
            prepared_execution_action: true,
            skip_reason: None,
        }
    }

    pub(crate) fn skipped(reason: FrameDocumentJavascriptUrlPrepareSkipReason) -> Self {
        Self {
            prepared_execution_action: false,
            skip_reason: Some(reason),
        }
    }

    pub(crate) fn made_progress(&self) -> bool {
        self.prepared_execution_action || self.skip_reason.is_some()
    }
}

impl FrameDocumentJavascriptUrlExecutionResult {
    pub(crate) fn new(
        target: FrameDocumentJavascriptUrlScriptExecutionTarget,
        url: Url,
        attempted_script_job: bool,
        completion: FrameDocumentJavascriptUrlCompletion,
        preserve_window_event_state: bool,
        dispatch_load_on_no_string_completion: bool,
    ) -> Self {
        Self {
            target,
            url,
            attempted_script_job,
            completion,
            preserve_window_event_state,
            dispatch_load_on_no_string_completion,
        }
    }

    pub(crate) fn into_post_execution_action(
        self,
    ) -> FrameDocumentJavascriptUrlPostExecutionAction {
        FrameDocumentJavascriptUrlPostExecutionAction {
            target: self.target,
            url: self.url,
            attempted_script_job: self.attempted_script_job,
            completion: self.completion,
            preserve_window_event_state: self.preserve_window_event_state,
            dispatch_load_on_no_string_completion: self.dispatch_load_on_no_string_completion,
        }
    }
}

impl FrameDocumentJavascriptUrlPostExecutionAction {
    pub(crate) fn target(&self) -> FrameDocumentJavascriptUrlScriptExecutionTarget {
        self.target
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn attempted_script_job(&self) -> bool {
        self.attempted_script_job
    }

    pub(crate) fn failed_script_job(&self) -> bool {
        self.completion.failed_script_job()
    }

    pub(crate) fn completion(&self) -> &FrameDocumentJavascriptUrlCompletion {
        &self.completion
    }

    pub(crate) fn preserve_window_event_state(&self) -> bool {
        self.preserve_window_event_state
    }

    pub(crate) fn dispatch_load_on_no_string_completion(&self) -> bool {
        self.dispatch_load_on_no_string_completion
    }
}

impl FrameDocumentJavascriptUrlCompletion {
    pub(crate) fn failed_script_job(&self) -> bool {
        matches!(self, Self::FailedScriptJob)
    }
}

impl FrameDocumentJavascriptUrlExecutionFollowup {
    pub(crate) fn new(
        attempted_script_job: bool,
        failed_script_job: bool,
        string_completion_committed: bool,
        lifecycle_followup_queued: bool,
    ) -> Self {
        Self {
            attempted_script_job,
            failed_script_job,
            string_completion_committed,
            lifecycle_followup_queued,
        }
    }

    pub(crate) fn made_progress(&self) -> bool {
        self.attempted_script_job
            || self.failed_script_job
            || self.string_completion_committed
            || self.lifecycle_followup_queued
    }

    pub(crate) fn script_was_attempted(&self) -> bool {
        self.attempted_script_job
    }
}
