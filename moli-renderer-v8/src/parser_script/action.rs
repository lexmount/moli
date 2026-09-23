use crate::document_runtime::DomHandle;
use crate::parser_script::payload::{
    ParserClassicScriptMetadata, ParserClassicScriptSourceFailure,
    ParserClassicScriptSourceIdentity, ParserClassicScriptSourceResult,
    ParserExecutableClassicScript, ParserPreparedClassicScript, ParserReadyClassicScript,
};
use crate::types::SharedNavigationResponseResult;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserPendingClassicScriptReady {
    script: ParserReadyClassicScript,
    kind: ParserPendingClassicScriptReadyKind,
}

impl ParserPendingClassicScriptReady {
    pub(crate) fn new(
        script: ParserReadyClassicScript,
        kind: ParserPendingClassicScriptReadyKind,
    ) -> Self {
        Self { script, kind }
    }

    pub(crate) fn script(&self) -> &ParserReadyClassicScript {
        &self.script
    }

    pub(crate) fn kind(&self) -> ParserPendingClassicScriptReadyKind {
        self.kind
    }

    pub(crate) fn execution_for_script(
        &self,
        script_handle: DomHandle,
    ) -> Option<ParserPendingClassicScriptExecution> {
        if self.script.script_handle() != script_handle {
            return None;
        }
        Some(ParserPendingClassicScriptExecution {
            metadata: self.script.metadata(),
            ready_kind: self.kind,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParserPendingClassicScriptReadyAction<'a> {
    ready: &'a ParserPendingClassicScriptReady,
}

impl<'a> ParserPendingClassicScriptReadyAction<'a> {
    pub(crate) fn new(ready: &'a ParserPendingClassicScriptReady) -> Self {
        Self { ready }
    }

    #[cfg(test)]
    pub(crate) fn ready(&self) -> &'a ParserPendingClassicScriptReady {
        self.ready
    }

    pub(crate) fn ready_script(&self) -> &'a ParserReadyClassicScript {
        self.ready.script()
    }

    pub(crate) fn ready_kind(&self) -> ParserPendingClassicScriptReadyKind {
        self.ready.kind()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParserPendingClassicScriptExecution {
    pub(crate) metadata: ParserClassicScriptMetadata,
    pub(crate) ready_kind: ParserPendingClassicScriptReadyKind,
}

pub(crate) struct ParserPendingClassicScriptBeginExecutionAction {
    execution: ParserPendingClassicScriptExecution,
    executable_script: ParserExecutableClassicScript,
}

impl ParserPendingClassicScriptBeginExecutionAction {
    pub(crate) fn new(
        execution: ParserPendingClassicScriptExecution,
        executable_script: ParserExecutableClassicScript,
    ) -> Self {
        Self {
            execution,
            executable_script,
        }
    }

    pub(crate) fn execution(&self) -> ParserPendingClassicScriptExecution {
        self.execution
    }

    pub(crate) fn into_executable_script(self) -> ParserExecutableClassicScript {
        self.executable_script
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ParserClassicScriptExecutionStart<Action, Completion> {
    Execute(Box<Action>),
    Complete(Box<Completion>),
    Dropped,
}

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptBeginExecutionAction<Target> {
    target: Target,
    execution: ParserPendingClassicScriptExecution,
    executable_script: ParserExecutableClassicScript,
}

impl<Target> ParserClassicScriptBeginExecutionAction<Target> {
    pub(crate) fn new(
        target: Target,
        execution: ParserPendingClassicScriptExecution,
        executable_script: ParserExecutableClassicScript,
    ) -> Self {
        Self {
            target,
            execution,
            executable_script,
        }
    }

    pub(crate) fn from_pending_begin_execution_action(
        target: Target,
        action: ParserPendingClassicScriptBeginExecutionAction,
    ) -> Self {
        let execution = action.execution();
        Self::new(target, execution, action.into_executable_script())
    }

    pub(crate) fn script_url(&self) -> &Url {
        self.executable_script.script_url()
    }

    pub(crate) fn source_kind(&self) -> crate::types::ScriptSourceKind {
        self.executable_script.source_kind()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Target,
        ParserPendingClassicScriptExecution,
        ParserExecutableClassicScript,
    ) {
        (self.target, self.execution, self.executable_script)
    }
}

pub(crate) struct ParserPendingClassicScriptDisposedReadyAction {
    execution: ParserPendingClassicScriptExecution,
}

impl ParserPendingClassicScriptDisposedReadyAction {
    pub(crate) fn new(execution: ParserPendingClassicScriptExecution) -> Self {
        Self { execution }
    }

    pub(crate) fn execution(&self) -> ParserPendingClassicScriptExecution {
        self.execution
    }
}

pub(crate) struct ParserPendingClassicScriptFinishedExecutionAction {
    execution: ParserPendingClassicScriptExecution,
}

impl ParserPendingClassicScriptFinishedExecutionAction {
    pub(crate) fn new(execution: ParserPendingClassicScriptExecution) -> Self {
        Self { execution }
    }

    pub(crate) fn execution(&self) -> ParserPendingClassicScriptExecution {
        self.execution
    }
}

pub(crate) struct ParserPendingClassicScriptSourceFailureAction {
    failure: ParserClassicScriptSourceFailure,
}

impl ParserPendingClassicScriptSourceFailureAction {
    pub(crate) fn new(failure: ParserClassicScriptSourceFailure) -> Self {
        Self { failure }
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.failure.script_handle()
    }

    pub(crate) fn into_failure(self) -> ParserClassicScriptSourceFailure {
        self.failure
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParserPendingClassicScriptSourceLoadCandidate<'a> {
    metadata: ParserClassicScriptMetadata,
    script_url: &'a Url,
}

impl<'a> ParserPendingClassicScriptSourceLoadCandidate<'a> {
    pub(crate) fn new(metadata: ParserClassicScriptMetadata, script_url: &'a Url) -> Self {
        Self {
            metadata,
            script_url,
        }
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn script_url(&self) -> &'a Url {
        self.script_url
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ParserPendingClassicScriptSourceLoadClientAction<'a> {
    metadata: ParserClassicScriptMetadata,
    script_url: &'a Url,
}

impl<'a> ParserPendingClassicScriptSourceLoadClientAction<'a> {
    pub(crate) fn new(metadata: ParserClassicScriptMetadata, script_url: &'a Url) -> Self {
        Self {
            metadata,
            script_url,
        }
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn script_url(&self) -> &'a Url {
        self.script_url
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptSourceLoadClientAction<Target> {
    target: Target,
    metadata: ParserClassicScriptMetadata,
    script_url: Url,
}

impl<Target> ParserClassicScriptSourceLoadClientAction<Target> {
    pub(crate) fn new(
        target: Target,
        metadata: ParserClassicScriptMetadata,
        script_url: Url,
    ) -> Self {
        Self {
            target,
            metadata,
            script_url,
        }
    }

    pub(crate) fn from_pending_source_load_client_action(
        target: Target,
        client: ParserPendingClassicScriptSourceLoadClientAction<'_>,
    ) -> Self {
        Self::new(target, client.metadata(), client.script_url().clone())
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserPendingClassicScriptSourceLoadRequest {
    source_identity: ParserClassicScriptSourceIdentity,
    input: ParserPreparedClassicScript,
}

impl ParserPendingClassicScriptSourceLoadRequest {
    pub(crate) fn new(
        source_identity: ParserClassicScriptSourceIdentity,
        input: ParserPreparedClassicScript,
    ) -> Self {
        Self {
            source_identity,
            input,
        }
    }

    #[cfg(test)]
    pub(crate) fn source_identity(&self) -> ParserClassicScriptSourceIdentity {
        self.source_identity
    }

    #[cfg(test)]
    pub(crate) fn input(&self) -> &ParserPreparedClassicScript {
        &self.input
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserClassicScriptSourceIdentity,
        ParserPreparedClassicScript,
    ) {
        (self.source_identity, self.input)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptSourceLoadRequestAction<Target> {
    target: Target,
    request: ParserPendingClassicScriptSourceLoadRequest,
}

impl<Target> ParserClassicScriptSourceLoadRequestAction<Target> {
    pub(crate) fn new(
        target: Target,
        request: ParserPendingClassicScriptSourceLoadRequest,
    ) -> Self {
        Self { target, request }
    }

    pub(crate) fn from_pending_source_load_action(
        target: Target,
        action: ParserPendingClassicScriptSourceLoadAction,
    ) -> Self {
        Self::new(target, action.into_request())
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    #[cfg(test)]
    pub(crate) fn source_load_request(&self) -> &ParserPendingClassicScriptSourceLoadRequest {
        &self.request
    }

    pub(crate) fn into_parts(self) -> (Target, ParserPendingClassicScriptSourceLoadRequest) {
        (self.target, self.request)
    }
}

pub(crate) struct ParserPendingClassicScriptSourceLoadAction {
    request: ParserPendingClassicScriptSourceLoadRequest,
}

impl ParserPendingClassicScriptSourceLoadAction {
    pub(crate) fn new(request: ParserPendingClassicScriptSourceLoadRequest) -> Self {
        Self { request }
    }

    pub(crate) fn into_request(self) -> ParserPendingClassicScriptSourceLoadRequest {
        self.request
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserPendingClassicScriptSourceLoadCompletionRecord {
    source_identity: ParserClassicScriptSourceIdentity,
}

impl ParserPendingClassicScriptSourceLoadCompletionRecord {
    pub(crate) fn new(source_identity: ParserClassicScriptSourceIdentity) -> Self {
        Self { source_identity }
    }

    pub(crate) fn source_identity(&self) -> ParserClassicScriptSourceIdentity {
        self.source_identity
    }

    pub(crate) fn into_source_result(
        self,
        result: Result<String, String>,
    ) -> ParserClassicScriptSourceResult {
        ParserClassicScriptSourceResult::from_identity_result(self.source_identity, result)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserPendingClassicScriptSourceLoadCompletionAction<SourceLoadOwner> {
    source_load_owner: Option<SourceLoadOwner>,
    record: ParserPendingClassicScriptSourceLoadCompletionRecord,
}

impl<SourceLoadOwner> ParserPendingClassicScriptSourceLoadCompletionAction<SourceLoadOwner> {
    pub(crate) fn new(
        source_load_owner: Option<SourceLoadOwner>,
        record: ParserPendingClassicScriptSourceLoadCompletionRecord,
    ) -> Self {
        Self {
            source_load_owner,
            record,
        }
    }

    pub(crate) fn source_load_owner(&self) -> Option<SourceLoadOwner>
    where
        SourceLoadOwner: Copy,
    {
        self.source_load_owner
    }

    pub(crate) fn source_identity(&self) -> ParserClassicScriptSourceIdentity {
        self.record.source_identity()
    }

    pub(crate) fn into_record(self) -> ParserPendingClassicScriptSourceLoadCompletionRecord {
        self.record
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptSourceLoadCompletionAction<Target> {
    target: Target,
    record: ParserPendingClassicScriptSourceLoadCompletionRecord,
}

impl<Target> ParserClassicScriptSourceLoadCompletionAction<Target> {
    pub(crate) fn new(
        target: Target,
        record: ParserPendingClassicScriptSourceLoadCompletionRecord,
    ) -> Self {
        Self { target, record }
    }

    pub(crate) fn from_pending_source_load_completion_action<SourceLoadOwner>(
        target: Target,
        action: ParserPendingClassicScriptSourceLoadCompletionAction<SourceLoadOwner>,
    ) -> Self {
        Self::new(target, action.into_record())
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    pub(crate) fn into_parts(
        self,
    ) -> (Target, ParserPendingClassicScriptSourceLoadCompletionRecord) {
        (self.target, self.record)
    }
}

pub(crate) struct ParserPendingClassicScriptSourceResultAction<'a> {
    notification: ParserPendingClassicScriptNotification,
    network_result: Option<&'a SharedNavigationResponseResult>,
    network_record_urls: Option<ParserClassicScriptNetworkRecordUrls>,
}

impl<'a> ParserPendingClassicScriptSourceResultAction<'a> {
    pub(crate) fn new(
        notification: ParserPendingClassicScriptNotification,
        network_result: Option<&'a SharedNavigationResponseResult>,
        network_record_urls: Option<ParserClassicScriptNetworkRecordUrls>,
    ) -> Self {
        Self {
            notification,
            network_result,
            network_record_urls,
        }
    }

    pub(crate) fn notification(&self) -> ParserPendingClassicScriptNotification {
        self.notification
    }

    pub(crate) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result
    }

    pub(crate) fn network_record_urls(&self) -> Option<&ParserClassicScriptNetworkRecordUrls> {
        self.network_record_urls.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptNetworkRecordUrls {
    initiator_url: Url,
    script_url: Url,
}

impl ParserClassicScriptNetworkRecordUrls {
    pub(crate) fn from_prepared_script(script: &crate::planning::PreparedScript) -> Self {
        Self {
            initiator_url: script.initiator_url.clone(),
            script_url: script.url.clone(),
        }
    }

    pub(crate) fn initiator_url(&self) -> &Url {
        &self.initiator_url
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParserClassicScriptRunnerStep {
    Ready(ParserPendingClassicScriptReady),
    SourceFailed(ParserClassicScriptSourceFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParserClassicScriptNextOwnerAction<Ready, SourceFailure> {
    Ready(Ready),
    SourceFailed(SourceFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptReadyAction<Target> {
    target: Target,
    ready: ParserReadyClassicScript,
    ready_kind: ParserPendingClassicScriptReadyKind,
}

impl<Target> ParserClassicScriptReadyAction<Target> {
    pub(crate) fn new(
        target: Target,
        ready: ParserReadyClassicScript,
        ready_kind: ParserPendingClassicScriptReadyKind,
    ) -> Self {
        Self {
            target,
            ready,
            ready_kind,
        }
    }

    pub(crate) fn from_pending_ready_action(
        target: Target,
        ready: ParserPendingClassicScriptReadyAction<'_>,
    ) -> Self {
        Self::new(target, ready.ready_script().clone(), ready.ready_kind())
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    pub(crate) fn map_target<NextTarget>(
        self,
        target: NextTarget,
    ) -> ParserClassicScriptReadyAction<NextTarget> {
        ParserClassicScriptReadyAction {
            target,
            ready: self.ready,
            ready_kind: self.ready_kind,
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_kind(&self) -> ParserPendingClassicScriptReadyKind {
        self.ready_kind
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.ready.script_handle()
    }

    pub(crate) fn script_url(&self) -> &Url {
        self.ready.script_url()
    }

    #[cfg(test)]
    pub(crate) fn start_line(&self) -> u64 {
        self.ready.start_line()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptSourceFailureAction<Target, ScriptElementEvent> {
    target: Target,
    failure: ParserClassicScriptSourceFailure,
    script_element_event: Option<ScriptElementEvent>,
}

impl<Target, ScriptElementEvent>
    ParserClassicScriptSourceFailureAction<Target, ScriptElementEvent>
{
    pub(crate) fn new(
        target: Target,
        failure: ParserClassicScriptSourceFailure,
        script_element_event: Option<ScriptElementEvent>,
    ) -> Self {
        Self {
            target,
            failure,
            script_element_event,
        }
    }

    pub(crate) fn from_pending_source_failure_action(
        target: Target,
        action: ParserPendingClassicScriptSourceFailureAction,
        script_element_event: Option<ScriptElementEvent>,
    ) -> Self {
        Self::new(target, action.into_failure(), script_element_event)
    }

    pub(crate) fn target(&self) -> &Target {
        &self.target
    }

    pub(crate) fn map_target<NextTarget>(
        self,
        target: NextTarget,
    ) -> ParserClassicScriptSourceFailureAction<NextTarget, ScriptElementEvent> {
        ParserClassicScriptSourceFailureAction {
            target,
            failure: self.failure,
            script_element_event: self.script_element_event,
        }
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.failure.script_handle()
    }

    pub(crate) fn script_url(&self) -> &Url {
        self.failure.script_url()
    }

    pub(crate) fn error(&self) -> &str {
        self.failure.error()
    }

    #[cfg(test)]
    pub(crate) fn script_element_event(&self) -> Option<ScriptElementEvent>
    where
        ScriptElementEvent: Copy,
    {
        self.script_element_event
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Target,
        ParserClassicScriptSourceFailure,
        Option<ScriptElementEvent>,
    ) {
        (self.target, self.failure, self.script_element_event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptCompletionAction<Target, ScriptElementEvent> {
    target: Target,
    script_element_event: Option<ScriptElementEvent>,
}

/// Parser-owned classic completion policy chosen when the PendingScript is
/// accepted. Main and child adapters apply different concrete continuations,
/// but they must agree on whether completion resumes a paused parser or
/// releases an after-parsing ordered slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserClassicScriptScheduling {
    ParserBlocking,
    Deferred,
}

impl<Target, ScriptElementEvent> ParserClassicScriptCompletionAction<Target, ScriptElementEvent> {
    pub(crate) fn new(target: Target, script_element_event: Option<ScriptElementEvent>) -> Self {
        Self {
            target,
            script_element_event,
        }
    }

    pub(crate) fn from_pending_finished_execution_action(
        target: Target,
        _action: ParserPendingClassicScriptFinishedExecutionAction,
        script_element_event: Option<ScriptElementEvent>,
    ) -> Self {
        Self::new(target, script_element_event)
    }

    pub(crate) fn from_pending_disposed_ready_action(
        target: Target,
        action: ParserPendingClassicScriptDisposedReadyAction,
    ) -> Self {
        let _execution = action.execution();
        Self::new(target, None)
    }

    pub(crate) fn into_parts(self) -> (Target, Option<ScriptElementEvent>) {
        (self.target, self.script_element_event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserPendingClassicScriptReadyKind {
    ParserConnected,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParserPendingClassicScriptNotification {
    SourceReady,
    SourceFailed,
}

#[cfg(test)]
mod tests {
    use super::{
        ParserClassicScriptCompletionAction, ParserClassicScriptReadyAction,
        ParserClassicScriptSourceFailureAction, ParserPendingClassicScriptDisposedReadyAction,
        ParserPendingClassicScriptExecution, ParserPendingClassicScriptFinishedExecutionAction,
        ParserPendingClassicScriptReadyKind, ParserPendingClassicScriptSourceFailureAction,
    };
    use crate::{
        document_runtime::DomHandle,
        parser_script::payload::{
            ParserClassicScriptMetadata, ParserClassicScriptSourceFailure, ParserReadyClassicScript,
        },
    };
    use url::Url;

    #[test]
    fn parser_classic_script_completion_action_carries_target_and_event() {
        let action = ParserClassicScriptCompletionAction::new("document-1", Some("load"));
        assert_eq!(action.into_parts(), ("document-1", Some("load")));
    }

    #[test]
    fn parser_classic_script_completion_action_allows_no_event() {
        let action =
            ParserClassicScriptCompletionAction::<_, &'static str>::new("document-1", None);
        assert_eq!(action.into_parts(), ("document-1", None));
    }

    #[test]
    fn parser_classic_script_completion_action_accepts_pending_finished_execution() {
        let execution = ParserPendingClassicScriptExecution {
            metadata: ParserClassicScriptMetadata::new(DomHandle::new(14), 8),
            ready_kind: ParserPendingClassicScriptReadyKind::External,
        };
        let pending = ParserPendingClassicScriptFinishedExecutionAction::new(execution);
        let action = ParserClassicScriptCompletionAction::from_pending_finished_execution_action(
            "document-2",
            pending,
            Some("load"),
        );

        assert_eq!(action.into_parts(), ("document-2", Some("load")));
    }

    #[test]
    fn parser_classic_script_completion_action_accepts_pending_disposed_ready() {
        let execution = ParserPendingClassicScriptExecution {
            metadata: ParserClassicScriptMetadata::new(DomHandle::new(15), 9),
            ready_kind: ParserPendingClassicScriptReadyKind::ParserConnected,
        };
        let pending = ParserPendingClassicScriptDisposedReadyAction::new(execution);
        let action =
            ParserClassicScriptCompletionAction::<_, &'static str>::from_pending_disposed_ready_action(
                "document-3",
                pending,
            );

        assert_eq!(action.into_parts(), ("document-3", None));
    }

    #[test]
    fn parser_classic_script_ready_action_carries_target_and_ready_payload() {
        let script_url = Url::parse("https://example.test/ready.js").expect("script url");
        let ready = ParserReadyClassicScript::new(
            ParserClassicScriptMetadata::new(DomHandle::new(11), 7),
            script_url.clone(),
        );
        let action = ParserClassicScriptReadyAction::new(
            "document-1",
            ready,
            ParserPendingClassicScriptReadyKind::External,
        );

        assert_eq!(action.target(), &"document-1");
        assert_eq!(action.script_handle(), DomHandle::new(11));
        assert_eq!(action.script_url(), &script_url);
        assert_eq!(
            action.ready_kind(),
            ParserPendingClassicScriptReadyKind::External
        );
        assert_eq!(action.start_line(), 7);
    }

    #[test]
    fn parser_classic_script_ready_action_can_remap_target_after_claim() {
        let script_url = Url::parse("https://example.test/ready.js").expect("script url");
        let ready = ParserReadyClassicScript::new(
            ParserClassicScriptMetadata::new(DomHandle::new(11), 7),
            script_url.clone(),
        );
        let action = ParserClassicScriptReadyAction::new(
            "queued-owner",
            ready,
            ParserPendingClassicScriptReadyKind::ParserConnected,
        );

        let action = action.map_target("materialized-owner");

        assert_eq!(action.target(), &"materialized-owner");
        assert_eq!(action.script_handle(), DomHandle::new(11));
        assert_eq!(action.script_url(), &script_url);
        assert_eq!(
            action.ready_kind(),
            ParserPendingClassicScriptReadyKind::ParserConnected
        );
        assert_eq!(action.start_line(), 7);
    }

    #[test]
    fn parser_classic_script_source_failure_action_carries_target_failure_and_event() {
        let script_url = Url::parse("https://example.test/source-failure.js").expect("script url");
        let failure = ParserClassicScriptSourceFailure {
            metadata: ParserClassicScriptMetadata::new(DomHandle::new(12), 5),
            script_url: script_url.clone(),
            error: "network failure".to_owned(),
            prepared_script: None,
            source_network_result: None,
        };
        let action =
            ParserClassicScriptSourceFailureAction::new("document-1", failure, Some("error"));

        assert_eq!(action.target(), &"document-1");
        assert_eq!(action.script_handle(), DomHandle::new(12));
        assert_eq!(action.script_url(), &script_url);
        assert_eq!(action.error(), "network failure");
        assert_eq!(action.script_element_event(), Some("error"));
    }

    #[test]
    fn parser_classic_script_source_failure_action_can_remap_target() {
        let script_url =
            Url::parse("https://example.test/remapped-source-failure.js").expect("script url");
        let failure = ParserClassicScriptSourceFailure {
            metadata: ParserClassicScriptMetadata::new(DomHandle::new(12), 5),
            script_url: script_url.clone(),
            error: "network failure".to_owned(),
            prepared_script: None,
            source_network_result: None,
        };
        let action =
            ParserClassicScriptSourceFailureAction::new("queued-owner", failure, Some("error"));

        let action = action.map_target("materialized-owner");

        assert_eq!(action.target(), &"materialized-owner");
        assert_eq!(action.script_handle(), DomHandle::new(12));
        assert_eq!(action.script_url(), &script_url);
        assert_eq!(action.error(), "network failure");
        assert_eq!(action.script_element_event(), Some("error"));
    }

    #[test]
    fn parser_classic_script_source_failure_action_accepts_pending_failure() {
        let script_url =
            Url::parse("https://example.test/pending-source-failure.js").expect("script url");
        let failure = ParserClassicScriptSourceFailure {
            metadata: ParserClassicScriptMetadata::new(DomHandle::new(13), 6),
            script_url: script_url.clone(),
            error: "blocked".to_owned(),
            prepared_script: None,
            source_network_result: None,
        };
        let pending = ParserPendingClassicScriptSourceFailureAction::new(failure);
        assert_eq!(pending.script_handle(), DomHandle::new(13));

        let action = ParserClassicScriptSourceFailureAction::from_pending_source_failure_action(
            "document-2",
            pending,
            Some("error"),
        );

        assert_eq!(action.target(), &"document-2");
        assert_eq!(action.script_handle(), DomHandle::new(13));
        assert_eq!(action.script_url(), &script_url);
        assert_eq!(action.error(), "blocked");
        assert_eq!(action.script_element_event(), Some("error"));
    }
}
