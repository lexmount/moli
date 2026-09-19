use std::fmt;

use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::{ParserPendingScriptId, ParserPendingScriptKey};
use crate::document_task_lane::DocumentRealmTask;
use crate::module_runtime::{
    ModuleEntryId, ModuleGraphFetchedSource, ModuleImportPhase, ModuleMapKey,
    NativeDynamicImportSingleModuleClient, NativeModuleGraphFetchRequest,
    NativeModuleScriptSingleModuleClient, NativeModuleSingleFetchRequest,
};
use crate::planning::{PreparedScript, ScriptFetchMetadata};

use super::lifecycle_tasks::{ChildDocumentModuleScriptLoadDelay, DocumentLinkEventOwner};
use super::module_graph::{
    FrameDocumentDynamicImportTerminalPreparedAction, FrameDocumentModuleTerminalQueueFollowup,
};
use super::records::{
    FrameDocumentOwner, FrameDocumentTaskOwner, FrameRealmId, FrameRequestId, FrameRequestKind,
};

/// Exact PageVm-local execution target captured when a child module fetch is
/// started.
///
/// The stable Page resource queue adds the root renderer Document namespace.
/// This target keeps the remaining child/document/realm identity atomic, so a
/// consumer cannot authorize execution by splicing routing and owner fields
/// from independent protocol-attribution records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ChildDocumentModuleFetchTarget {
    child_handle: DomHandle,
    task_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
}

impl ChildDocumentModuleFetchTarget {
    pub(crate) fn new(
        child_handle: DomHandle,
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> Self {
        Self {
            child_handle,
            task_owner,
            realm_id,
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn task_owner(self) -> FrameDocumentTaskOwner {
        self.task_owner
    }

    pub(crate) fn realm_id(self) -> FrameRealmId {
        self.realm_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FrameDocumentModuleClientEntryId(u32);

impl FrameDocumentModuleClientEntryId {
    pub(crate) fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(crate) fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentModuleFetchDisposition {
    StartedFetch(FrameDocumentModuleClientEntryId),
    JoinedFetching(FrameDocumentModuleClientEntryId),
    AlreadyFetched(FrameDocumentModuleClientEntryId),
    AlreadyFailed(FrameDocumentModuleClientEntryId),
    AlreadyLinked(FrameDocumentModuleClientEntryId),
}

impl FrameDocumentModuleFetchDisposition {
    #[cfg(test)]
    pub(crate) fn entry_id(self) -> FrameDocumentModuleClientEntryId {
        match self {
            Self::StartedFetch(entry_id)
            | Self::JoinedFetching(entry_id)
            | Self::AlreadyFetched(entry_id)
            | Self::AlreadyFailed(entry_id)
            | Self::AlreadyLinked(entry_id) => entry_id,
        }
    }
}

/// Result of consuming one exact-owner static dependency fetch-start task.
///
/// A failed Document request admission is settled through a dependency
/// failure terminal when its document modulator still exists. Otherwise the
/// disposition records whether this action started the network request or
/// joined existing module-map state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentModuleDependencyFetchStartOutcome {
    RequestAdmissionUnavailable {
        terminal_followup: FrameDocumentModuleTerminalQueueFollowup,
    },
    ClientAccepted {
        disposition: FrameDocumentModuleFetchDisposition,
    },
}

impl FrameDocumentModuleDependencyFetchStartOutcome {
    #[cfg(test)]
    pub(crate) const fn fetch_was_scheduled(self) -> bool {
        matches!(
            self,
            Self::ClientAccepted {
                disposition: FrameDocumentModuleFetchDisposition::StartedFetch(_),
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FrameDocumentModuleClientId(u64);

impl FrameDocumentModuleClientId {
    pub(crate) fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleClientRegistration {
    entry_id: FrameDocumentModuleClientEntryId,
    client_id: FrameDocumentModuleClientId,
    fetch_disposition: FrameDocumentModuleFetchDisposition,
}

impl FrameDocumentModuleClientRegistration {
    pub(crate) fn new(
        entry_id: FrameDocumentModuleClientEntryId,
        client_id: FrameDocumentModuleClientId,
        fetch_disposition: FrameDocumentModuleFetchDisposition,
    ) -> Self {
        Self {
            entry_id,
            client_id,
            fetch_disposition,
        }
    }

    pub(crate) fn entry_id(&self) -> FrameDocumentModuleClientEntryId {
        self.entry_id
    }

    pub(crate) fn client_id(&self) -> FrameDocumentModuleClientId {
        self.client_id
    }

    pub(crate) fn fetch_disposition(&self) -> FrameDocumentModuleFetchDisposition {
        self.fetch_disposition
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameDocumentParserRootModuleClient {
    script: PreparedScript,
    pending_script_key: ParserPendingScriptKey,
    script_handle: DomHandle,
    base_url: url::Url,
    fetch_metadata: ScriptFetchMetadata,
    source_is_external: bool,
    load_delay_token: ChildDocumentModuleScriptLoadDelay,
}

impl FrameDocumentParserRootModuleClient {
    pub(crate) fn new(
        pending_script_key: ParserPendingScriptKey,
        script: PreparedScript,
        script_handle: DomHandle,
        base_url: url::Url,
        fetch_metadata: ScriptFetchMetadata,
        source_is_external: bool,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
    ) -> Self {
        Self {
            pending_script_key,
            script,
            script_handle,
            base_url,
            fetch_metadata,
            source_is_external,
            load_delay_token: load_delay_token.into(),
        }
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn pending_script_id(
        &self,
        owner: FrameDocumentOwner,
    ) -> ParserPendingScriptId<FrameDocumentOwner> {
        ParserPendingScriptId::from_key(owner, self.pending_script_key)
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.script_handle
    }

    pub(crate) fn base_url(&self) -> &url::Url {
        &self.base_url
    }

    pub(crate) fn fetch_metadata(&self) -> &ScriptFetchMetadata {
        &self.fetch_metadata
    }

    pub(crate) fn source_is_external(&self) -> bool {
        self.source_is_external
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.load_delay_token
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FrameDocumentParserModuleRootStartKind {
    ExternalFetch { key: ModuleMapKey },
    LoadedSource(crate::module_runtime::ModuleSource),
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentParserModuleRootStartPayload {
    child_handle: DomHandle,
    client: FrameDocumentParserRootModuleClient,
    kind: FrameDocumentParserModuleRootStartKind,
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentParserModuleRootStartTask {
    owner: FrameDocumentTaskOwner,
    payload: FrameDocumentParserModuleRootStartPayload,
}

impl FrameDocumentParserModuleRootStartTask {
    pub(crate) fn from_root_start_parts(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        client: FrameDocumentParserRootModuleClient,
        kind: FrameDocumentParserModuleRootStartKind,
    ) -> Self {
        Self {
            owner,
            payload: FrameDocumentParserModuleRootStartPayload {
                child_handle,
                client,
                kind,
            },
        }
    }

    pub(crate) fn from_parser_script_parts(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
        script_handle: DomHandle,
        script: PreparedScript,
        load_delay_token: impl Into<ChildDocumentModuleScriptLoadDelay>,
    ) -> Self {
        assert_eq!(script.kind, crate::types::ScriptKind::Module);
        assert_eq!(pending_script_id.owner(), owner.document_owner());
        assert_eq!(pending_script_id.script_node_id(), script.node_id);
        assert_eq!(pending_script_id.parser_position(), script.position);
        let source_is_external = script.source_kind == crate::types::ScriptSourceKind::External;
        debug_assert_eq!(
            source_is_external,
            !matches!(script.source, crate::planning::ScriptSource::Inline(_)),
            "prepared parser module source kind must agree with its source payload"
        );
        let kind = match &script.source {
            crate::planning::ScriptSource::External => {
                FrameDocumentParserModuleRootStartKind::ExternalFetch {
                    key: ModuleMapKey::from_script_url(script.url.clone()),
                }
            }
            crate::planning::ScriptSource::Loaded(source)
            | crate::planning::ScriptSource::Inline(source) => {
                FrameDocumentParserModuleRootStartKind::LoadedSource(
                    crate::module_runtime::ModuleSource::text(source.clone()),
                )
            }
            crate::planning::ScriptSource::LoadedBinary { bytes, .. } => {
                FrameDocumentParserModuleRootStartKind::LoadedSource(
                    crate::module_runtime::ModuleSource::binary(bytes.clone()),
                )
            }
        };
        let client = FrameDocumentParserRootModuleClient::new(
            pending_script_id.key(),
            script.clone(),
            script_handle,
            script.base_url.clone(),
            script.fetch_metadata.clone(),
            source_is_external,
            load_delay_token,
        );
        assert_eq!(
            client.pending_script_id(owner.document_owner()),
            pending_script_id
        );
        Self::from_root_start_parts(child_handle, owner, client, kind)
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    fn payload(&self) -> &FrameDocumentParserModuleRootStartPayload {
        &self.payload
    }

    pub(crate) fn child_handle(&self) -> DomHandle {
        self.payload().child_handle
    }

    pub(crate) fn kind(&self) -> &FrameDocumentParserModuleRootStartKind {
        &self.payload().kind
    }

    pub(crate) fn client(&self) -> &FrameDocumentParserRootModuleClient {
        &self.payload().client
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        self.payload().client.script()
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<FrameDocumentOwner> {
        self.client()
            .pending_script_id(self.owner().document_owner())
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        DomHandle,
        FrameDocumentTaskOwner,
        FrameDocumentParserRootModuleClient,
        FrameDocumentParserModuleRootStartKind,
    ) {
        (
            self.payload.child_handle,
            self.owner,
            self.payload.client,
            self.payload.kind,
        )
    }

    pub(crate) fn into_external_fetch_start(
        self,
        realm_id: FrameRealmId,
        start: FrameDocumentModuleFetchClientStart,
    ) -> FrameDocumentParserModuleRootFetchStart {
        debug_assert_eq!(self.owner.document_owner(), start.owner());
        FrameDocumentParserModuleRootFetchStart {
            child_handle: self.payload.child_handle,
            owner: self.owner,
            script_handle: self.payload.client.script_handle(),
            script: self.payload.client.script().clone(),
            realm_id,
            start,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FrameDocumentParserModuleRootFetchStart {
    pub(crate) child_handle: DomHandle,
    pub(crate) owner: FrameDocumentTaskOwner,
    pub(crate) script_handle: DomHandle,
    pub(crate) script: PreparedScript,
    pub(crate) realm_id: FrameRealmId,
    pub(crate) start: FrameDocumentModuleFetchClientStart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentStaticDependencyModuleClient {
    parent_entry_id: ModuleEntryId,
    parent_key: ModuleMapKey,
    specifier: String,
    phase: ModuleImportPhase,
    tree_client: moli_module_script_tree::SingleModuleClientToken,
}

impl FrameDocumentStaticDependencyModuleClient {
    pub(crate) fn new(
        parent_entry_id: ModuleEntryId,
        parent_key: ModuleMapKey,
        specifier: String,
        phase: ModuleImportPhase,
        tree_client: moli_module_script_tree::SingleModuleClientToken,
    ) -> Self {
        Self {
            parent_entry_id,
            parent_key,
            specifier,
            phase,
            tree_client,
        }
    }

    pub(crate) fn parent_entry_id(&self) -> ModuleEntryId {
        self.parent_entry_id
    }

    pub(crate) fn parent_key(&self) -> &ModuleMapKey {
        &self.parent_key
    }

    pub(crate) fn specifier(&self) -> &str {
        &self.specifier
    }

    pub(crate) fn phase(&self) -> ModuleImportPhase {
        self.phase
    }

    pub(crate) fn tree_client(&self) -> moli_module_script_tree::SingleModuleClientToken {
        self.tree_client
    }
}

#[derive(Clone)]
pub(crate) struct FrameDocumentModuleDependencyFetchPayload {
    dependency_key: ModuleMapKey,
    client: FrameDocumentStaticDependencyModuleClient,
    reservation: FrameDocumentModuleClientReservation,
    fetch_request: NativeModuleGraphFetchRequest,
}

pub(crate) type FrameDocumentModuleDependencyFetchTask = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    FrameDocumentModuleDependencyFetchPayload,
>;

impl fmt::Debug for FrameDocumentModuleDependencyFetchPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameDocumentModuleDependencyFetchPayload")
            .field("dependency_key", &self.dependency_key)
            .field("client", &self.client)
            .field("reservation", &self.reservation)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FrameDocumentModuleDependencyFetchPayload {
    fn eq(&self, other: &Self) -> bool {
        self.dependency_key == other.dependency_key
            && self.client == other.client
            && self.reservation == other.reservation
    }
}

impl Eq for FrameDocumentModuleDependencyFetchPayload {}

impl FrameDocumentModuleDependencyFetchTask {
    pub(crate) fn from_dependency_fetch_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        dependency_key: ModuleMapKey,
        client: FrameDocumentStaticDependencyModuleClient,
        reservation: FrameDocumentModuleClientReservation,
        fetch_request: NativeModuleGraphFetchRequest,
    ) -> Self {
        Self::new(
            owner,
            realm_id,
            FrameDocumentModuleDependencyFetchPayload {
                dependency_key,
                client,
                reservation,
                fetch_request,
            },
        )
    }

    pub(crate) fn dependency_key(&self) -> &ModuleMapKey {
        &self.payload().dependency_key
    }

    pub(crate) fn client(&self) -> &FrameDocumentStaticDependencyModuleClient {
        &self.payload().client
    }

    pub(crate) fn reservation(&self) -> &FrameDocumentModuleClientReservation {
        &self.payload().reservation
    }

    pub(crate) fn fetch_request(&self) -> &NativeModuleGraphFetchRequest {
        &self.payload().fetch_request
    }
}

#[derive(Clone)]
pub(crate) struct FrameDocumentModuleDependencyTerminalPayload {
    request_key: ModuleMapKey,
    client: FrameDocumentStaticDependencyModuleClient,
    fetch_request: NativeModuleGraphFetchRequest,
    result: FrameDocumentModuleFetchTerminalResult,
}

pub(crate) type FrameDocumentModuleDependencyTerminalWork = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    FrameDocumentModuleDependencyTerminalPayload,
>;

impl fmt::Debug for FrameDocumentModuleDependencyTerminalPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameDocumentModuleDependencyTerminalPayload")
            .field("request_key", &self.request_key)
            .field("client", &self.client)
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl FrameDocumentModuleDependencyTerminalWork {
    pub(crate) fn from_fetch_task_result(
        task: FrameDocumentModuleDependencyFetchTask,
        result: FrameDocumentModuleFetchTerminalResult,
    ) -> Self {
        let (owner, realm_id, payload) = task.into_parts();
        Self::new(
            owner,
            realm_id,
            FrameDocumentModuleDependencyTerminalPayload {
                request_key: payload.dependency_key,
                client: payload.client,
                fetch_request: payload.fetch_request,
                result,
            },
        )
    }

    pub(crate) fn into_terminal_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        ModuleMapKey,
        FrameDocumentStaticDependencyModuleClient,
        NativeModuleGraphFetchRequest,
        FrameDocumentModuleFetchTerminalResult,
    ) {
        let (owner, realm_id, payload) = self.into_parts();
        (
            owner,
            realm_id,
            payload.request_key,
            payload.client,
            payload.fetch_request,
            payload.result,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadFetchPayload {
    client: FrameDocumentModulepreloadLinkClient,
    request: NativeModuleSingleFetchRequest,
}

/// Modulepreload work discovered before its child default realm is ready.
///
/// The parser can establish the exact Document owner before any V8 realm has
/// been materialized.  Keep that owner authoritative while the existing realm
/// materialization source runs; if a semantic realm already existed at
/// discovery time, preserve it as an additional replacement guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadWorkAwaitingRealm {
    expected_realm_id: Option<FrameRealmId>,
    client: FrameDocumentModulepreloadLinkClient,
    kind: FrameDocumentModulepreloadWorkAwaitingRealmKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FrameDocumentModulepreloadWorkAwaitingRealmKind {
    FetchStart(Box<NativeModuleSingleFetchRequest>),
    LinkError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentModulepreloadMaterializedWork {
    FetchStart(Box<FrameDocumentModulepreloadFetchTask>),
    LinkError(FrameDocumentModulepreloadTerminalWork),
}

impl FrameDocumentModulepreloadWorkAwaitingRealm {
    pub(crate) fn fetch_start(
        expected_realm_id: Option<FrameRealmId>,
        client: FrameDocumentModulepreloadLinkClient,
        request: NativeModuleSingleFetchRequest,
    ) -> Self {
        Self {
            expected_realm_id,
            client,
            kind: FrameDocumentModulepreloadWorkAwaitingRealmKind::FetchStart(Box::new(request)),
        }
    }

    pub(crate) fn link_error(
        expected_realm_id: Option<FrameRealmId>,
        client: FrameDocumentModulepreloadLinkClient,
    ) -> Self {
        Self {
            expected_realm_id,
            client,
            kind: FrameDocumentModulepreloadWorkAwaitingRealmKind::LinkError,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.client.owner()
    }

    pub(crate) fn expected_realm_id(&self) -> Option<FrameRealmId> {
        self.expected_realm_id
    }

    pub(crate) fn bind_first_established_realm(&mut self, realm_id: FrameRealmId) {
        if self.expected_realm_id.is_none() {
            self.expected_realm_id = Some(realm_id);
        }
    }

    pub(crate) fn child_handle(&self) -> DomHandle {
        self.client.child_handle()
    }

    pub(crate) fn link_handle(&self) -> DomHandle {
        self.client.link_handle()
    }

    pub(crate) fn request(&self) -> Option<&NativeModuleSingleFetchRequest> {
        match &self.kind {
            FrameDocumentModulepreloadWorkAwaitingRealmKind::FetchStart(request) => {
                Some(request.as_ref())
            }
            FrameDocumentModulepreloadWorkAwaitingRealmKind::LinkError => None,
        }
    }

    pub(crate) fn into_materialized_work(
        self,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentModulepreloadMaterializedWork> {
        if self
            .expected_realm_id
            .is_some_and(|expected| expected != realm_id)
        {
            return None;
        }
        Some(match self.kind {
            FrameDocumentModulepreloadWorkAwaitingRealmKind::FetchStart(request) => {
                FrameDocumentModulepreloadMaterializedWork::FetchStart(Box::new(
                    FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
                        realm_id,
                        self.client,
                        *request,
                    ),
                ))
            }
            FrameDocumentModulepreloadWorkAwaitingRealmKind::LinkError => {
                FrameDocumentModulepreloadMaterializedWork::LinkError(
                    FrameDocumentModulepreloadTerminalWork::from_link_error_parts(
                        realm_id,
                        self.client,
                    ),
                )
            }
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadLinkClient {
    child_handle: DomHandle,
    event_owner: DocumentLinkEventOwner,
}

impl FrameDocumentModulepreloadLinkClient {
    pub(crate) fn new(
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        link_handle: DomHandle,
    ) -> Self {
        Self {
            child_handle,
            event_owner: DocumentLinkEventOwner::new(owner, link_handle),
        }
    }

    pub(crate) fn child_handle(self) -> DomHandle {
        self.child_handle
    }

    pub(crate) fn owner(self) -> FrameDocumentTaskOwner {
        self.event_owner.owner()
    }

    pub(crate) fn link_handle(self) -> DomHandle {
        self.event_owner.element()
    }
}

pub(crate) type FrameDocumentModulepreloadFetchTask =
    DocumentRealmTask<FrameDocumentTaskOwner, FrameRealmId, FrameDocumentModulepreloadFetchPayload>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentModuleTerminalWarning {
    ParserRootTerminalWithoutOwnerWork {
        key: ModuleMapKey,
        successful: bool,
        parser_root_client_count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleTerminalWarningRecord {
    task_owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    warning: FrameDocumentModuleTerminalWarning,
}

#[derive(Debug, Default)]
pub(crate) struct FrameDocumentModuleTerminalBatch {
    terminal_batches: Vec<FrameDocumentModuleScriptTerminalBatchTask>,
    modulepreload_terminal_works: Vec<FrameDocumentModulepreloadTerminalWork>,
    dynamic_import_owner_actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    warnings: Vec<FrameDocumentModuleTerminalWarningRecord>,
}

impl FrameDocumentModuleTerminalWarningRecord {
    pub(crate) fn new(
        task_owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        warning: FrameDocumentModuleTerminalWarning,
    ) -> Self {
        Self {
            task_owner,
            realm_id,
            warning,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        FrameDocumentModuleTerminalWarning,
    ) {
        (self.task_owner, self.realm_id, self.warning)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FrameDocumentModuleScriptTerminalTask {
    ParserRoot(Box<FrameDocumentParserRootTerminalWork>),
    SingleModule(FrameDocumentModuleScriptTerminalWork),
    Dependency(Box<FrameDocumentModuleDependencyTerminalWork>),
}

impl FrameDocumentModuleScriptTerminalTask {
    pub(crate) fn parser_root(work: FrameDocumentParserRootTerminalWork) -> Self {
        Self::ParserRoot(Box::new(work))
    }

    pub(crate) fn single_module(work: FrameDocumentModuleScriptTerminalWork) -> Self {
        Self::SingleModule(work)
    }

    pub(crate) fn dependency(work: FrameDocumentModuleDependencyTerminalWork) -> Self {
        Self::Dependency(Box::new(work))
    }
}

pub(crate) type FrameDocumentModuleScriptTerminalBatchTask = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    Vec<FrameDocumentModuleScriptTerminalTask>,
>;

impl FrameDocumentModuleTerminalBatch {
    pub(crate) fn push_terminal_batch(&mut self, task: FrameDocumentModuleScriptTerminalBatchTask) {
        self.terminal_batches.push(task);
    }

    pub(crate) fn push_module_script_terminals(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        tasks: Vec<FrameDocumentModuleScriptTerminalTask>,
    ) {
        if !tasks.is_empty() {
            self.push_terminal_batch(FrameDocumentModuleScriptTerminalBatchTask::new(
                owner, realm_id, tasks,
            ));
        }
    }

    pub(crate) fn push_modulepreload_terminal_work(
        &mut self,
        work: FrameDocumentModulepreloadTerminalWork,
    ) {
        self.modulepreload_terminal_works.push(work);
    }

    pub(crate) fn push_dynamic_import_owner_action(
        &mut self,
        action: FrameDocumentDynamicImportTerminalPreparedAction,
    ) {
        self.dynamic_import_owner_actions.push(action);
    }

    pub(crate) fn push_warning(&mut self, warning: FrameDocumentModuleTerminalWarningRecord) {
        self.warnings.push(warning);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<FrameDocumentModuleScriptTerminalBatchTask>,
        Vec<FrameDocumentModulepreloadTerminalWork>,
        Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
        Vec<FrameDocumentModuleTerminalWarningRecord>,
    ) {
        (
            self.terminal_batches,
            self.modulepreload_terminal_works,
            self.dynamic_import_owner_actions,
            self.warnings,
        )
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.terminal_batches.is_empty()
            && self.modulepreload_terminal_works.is_empty()
            && self.dynamic_import_owner_actions.is_empty()
            && self.warnings.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.terminal_batches.len()
    }

    #[cfg(test)]
    pub(crate) fn into_modulepreload_terminal_works(
        self,
    ) -> Vec<FrameDocumentModulepreloadTerminalWork> {
        self.modulepreload_terminal_works
    }

    #[cfg(test)]
    pub(crate) fn into_dynamic_import_owner_actions(
        self,
    ) -> Vec<FrameDocumentDynamicImportTerminalPreparedAction> {
        self.dynamic_import_owner_actions
    }

    #[cfg(test)]
    pub(crate) fn into_warnings(self) -> Vec<FrameDocumentModuleTerminalWarningRecord> {
        self.warnings
    }
}

#[cfg(test)]
impl IntoIterator for FrameDocumentModuleTerminalBatch {
    type Item = FrameDocumentModuleScriptTerminalBatchTask;
    type IntoIter = std::vec::IntoIter<FrameDocumentModuleScriptTerminalBatchTask>;

    fn into_iter(self) -> Self::IntoIter {
        self.terminal_batches.into_iter()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadTerminalPayload {
    key: Option<ModuleMapKey>,
    client: FrameDocumentModulepreloadLinkClient,
    successful: bool,
}

pub(crate) type FrameDocumentModulepreloadTerminalWork = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    FrameDocumentModulepreloadTerminalPayload,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadEventAction {
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    key: Option<ModuleMapKey>,
    client: FrameDocumentModulepreloadLinkClient,
    successful: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentModulepreloadTerminalOutcome {
    event_dispatched: bool,
    event_dispatch_failed: bool,
}

impl FrameDocumentModulepreloadTerminalOutcome {
    pub(crate) fn event_dispatched() -> Self {
        Self {
            event_dispatched: true,
            ..Self::default()
        }
    }

    pub(crate) fn event_dispatch_failed() -> Self {
        Self {
            event_dispatch_failed: true,
            ..Self::default()
        }
    }

    pub(crate) fn event_was_dispatched(self) -> bool {
        self.event_dispatched
    }

    #[cfg(test)]
    pub(crate) fn event_dispatch_was_failed(self) -> bool {
        self.event_dispatch_failed
    }
}

impl FrameDocumentModulepreloadTerminalWork {
    pub(crate) fn from_terminal_parts(
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: FrameDocumentModulepreloadLinkClient,
        successful: bool,
    ) -> Self {
        Self::new(
            client.owner(),
            realm_id,
            FrameDocumentModulepreloadTerminalPayload {
                key: Some(key),
                client,
                successful,
            },
        )
    }

    pub(crate) fn from_link_error_parts(
        realm_id: FrameRealmId,
        client: FrameDocumentModulepreloadLinkClient,
    ) -> Self {
        Self::new(
            client.owner(),
            realm_id,
            FrameDocumentModulepreloadTerminalPayload {
                key: None,
                client,
                successful: false,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn key(&self) -> Option<&ModuleMapKey> {
        self.payload().key.as_ref()
    }

    pub(crate) fn link_handle(&self) -> DomHandle {
        self.payload().client.link_handle()
    }

    pub(crate) fn client(&self) -> FrameDocumentModulepreloadLinkClient {
        self.payload().client
    }

    pub(crate) fn successful(&self) -> bool {
        self.payload().successful
    }

    pub(crate) fn into_terminal_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        Option<ModuleMapKey>,
        FrameDocumentModulepreloadLinkClient,
        bool,
    ) {
        let (owner, realm_id, payload) = self.into_parts();
        (
            owner,
            realm_id,
            payload.key,
            payload.client,
            payload.successful,
        )
    }

    pub(crate) fn into_event_action(self) -> FrameDocumentModulepreloadEventAction {
        let (owner, realm_id, key, client, successful) = self.into_terminal_parts();
        debug_assert_eq!(owner, client.owner());
        FrameDocumentModulepreloadEventAction {
            owner,
            realm_id,
            key,
            client,
            successful,
        }
    }
}

impl FrameDocumentModulepreloadEventAction {
    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) fn key(&self) -> Option<&ModuleMapKey> {
        self.key.as_ref()
    }

    pub(crate) fn link_handle(&self) -> DomHandle {
        self.client.link_handle()
    }

    pub(crate) fn successful(&self) -> bool {
        self.successful
    }
}

pub(crate) trait FrameDocumentModulepreloadEventActionHooks {
    fn dispatch_modulepreload_event(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        link_handle: DomHandle,
        successful: bool,
    ) -> Result<(), String>;

    fn record_modulepreload_event_dispatch_failed(
        &mut self,
        action: &FrameDocumentModulepreloadEventAction,
        error: &str,
    );
}

pub(crate) struct FrameDocumentModulepreloadEventActionRunner<Hooks> {
    hooks: Hooks,
}

impl<Hooks> FrameDocumentModulepreloadEventActionRunner<Hooks>
where
    Hooks: FrameDocumentModulepreloadEventActionHooks,
{
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }

    #[cfg(test)]
    pub(crate) fn into_hooks(self) -> Hooks {
        self.hooks
    }

    pub(crate) fn run_event_action(
        &mut self,
        action: FrameDocumentModulepreloadEventAction,
    ) -> FrameDocumentModulepreloadTerminalOutcome {
        match self.hooks.dispatch_modulepreload_event(
            action.owner(),
            action.realm_id(),
            action.link_handle(),
            action.successful(),
        ) {
            Ok(()) => FrameDocumentModulepreloadTerminalOutcome::event_dispatched(),
            Err(error) => {
                self.hooks
                    .record_modulepreload_event_dispatch_failed(&action, &error);
                FrameDocumentModulepreloadTerminalOutcome::event_dispatch_failed()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleScriptTerminalPayload {
    key: ModuleMapKey,
    client: NativeModuleScriptSingleModuleClient,
}

pub(crate) type FrameDocumentModuleScriptTerminalWork = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    FrameDocumentModuleScriptTerminalPayload,
>;

impl FrameDocumentModuleScriptTerminalWork {
    pub(crate) fn from_terminal_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: NativeModuleScriptSingleModuleClient,
    ) -> Self {
        Self::new(
            owner,
            realm_id,
            FrameDocumentModuleScriptTerminalPayload { key, client },
        )
    }

    #[cfg(test)]
    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.payload().key
    }

    #[cfg(test)]
    pub(crate) fn client(&self) -> NativeModuleScriptSingleModuleClient {
        self.payload().client
    }

    pub(crate) fn into_terminal_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        ModuleMapKey,
        NativeModuleScriptSingleModuleClient,
    ) {
        let (owner, realm_id, payload) = self.into_parts();
        (owner, realm_id, payload.key, payload.client)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentDynamicImportTerminalPayload {
    key: ModuleMapKey,
    client: NativeDynamicImportSingleModuleClient,
}

pub(crate) type FrameDocumentDynamicImportTerminalWork = DocumentRealmTask<
    FrameDocumentTaskOwner,
    FrameRealmId,
    FrameDocumentDynamicImportTerminalPayload,
>;

impl FrameDocumentDynamicImportTerminalWork {
    pub(crate) fn from_terminal_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: NativeDynamicImportSingleModuleClient,
    ) -> Self {
        Self::new(
            owner,
            realm_id,
            FrameDocumentDynamicImportTerminalPayload { key, client },
        )
    }

    pub(crate) fn into_terminal_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        ModuleMapKey,
        NativeDynamicImportSingleModuleClient,
    ) {
        let (owner, realm_id, payload) = self.into_parts();
        (owner, realm_id, payload.key, payload.client)
    }
}

impl FrameDocumentModulepreloadFetchTask {
    pub(crate) fn from_modulepreload_fetch_parts(
        realm_id: FrameRealmId,
        client: FrameDocumentModulepreloadLinkClient,
        request: NativeModuleSingleFetchRequest,
    ) -> Self {
        Self::new(
            client.owner(),
            realm_id,
            FrameDocumentModulepreloadFetchPayload { client, request },
        )
    }

    pub(crate) fn link_handle(&self) -> DomHandle {
        self.payload().client.link_handle()
    }

    pub(crate) fn target(&self) -> ChildDocumentModuleFetchTarget {
        ChildDocumentModuleFetchTarget::new(
            self.payload().client.child_handle(),
            self.owner(),
            self.realm_id(),
        )
    }

    pub(crate) fn client(&self) -> FrameDocumentModulepreloadLinkClient {
        self.payload().client
    }

    pub(crate) fn request(&self) -> &NativeModuleSingleFetchRequest {
        &self.payload().request
    }

    pub(crate) fn into_request(self) -> NativeModuleSingleFetchRequest {
        self.into_payload().request
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleFetchClientStart {
    owner: FrameDocumentOwner,
    request_id: FrameRequestId,
    request_kind: FrameRequestKind,
    key: ModuleMapKey,
    registration: FrameDocumentModuleClientRegistration,
}

impl FrameDocumentModuleFetchClientStart {
    pub(crate) fn new(
        owner: FrameDocumentOwner,
        request_id: FrameRequestId,
        request_kind: FrameRequestKind,
        key: ModuleMapKey,
        registration: FrameDocumentModuleClientRegistration,
    ) -> Self {
        Self {
            owner,
            request_id,
            request_kind,
            key,
            registration,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        self.owner
    }

    pub(crate) fn request_id(&self) -> FrameRequestId {
        self.request_id
    }

    pub(crate) fn request_kind(&self) -> FrameRequestKind {
        self.request_kind
    }

    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.key
    }

    pub(crate) fn registration(&self) -> FrameDocumentModuleClientRegistration {
        self.registration
    }

    pub(crate) fn entry_id(&self) -> FrameDocumentModuleClientEntryId {
        self.registration.entry_id()
    }

    pub(crate) fn fetch_disposition(&self) -> FrameDocumentModuleFetchDisposition {
        self.registration.fetch_disposition()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleClientReservation {
    owner: FrameDocumentOwner,
    key: ModuleMapKey,
    registration: FrameDocumentModuleClientRegistration,
}

impl FrameDocumentModuleClientReservation {
    pub(crate) fn new(
        owner: FrameDocumentOwner,
        key: ModuleMapKey,
        registration: FrameDocumentModuleClientRegistration,
    ) -> Self {
        Self {
            owner,
            key,
            registration,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentOwner {
        self.owner
    }

    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.key
    }

    pub(crate) fn registration(&self) -> FrameDocumentModuleClientRegistration {
        self.registration
    }

    pub(crate) fn entry_id(&self) -> FrameDocumentModuleClientEntryId {
        self.registration.entry_id()
    }

    pub(crate) fn client_id(&self) -> FrameDocumentModuleClientId {
        self.registration.client_id()
    }

    pub(crate) fn fetch_disposition(&self) -> FrameDocumentModuleFetchDisposition {
        self.registration.fetch_disposition()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FrameDocumentParserRootTerminalClient {
    parser_root: FrameDocumentParserRootModuleClient,
}

impl FrameDocumentParserRootTerminalClient {
    pub(crate) fn new(client: FrameDocumentParserRootModuleClient) -> Self {
        Self {
            parser_root: client,
        }
    }

    #[cfg(test)]
    pub(crate) fn parser_root_payload(&self) -> &FrameDocumentParserRootModuleClient {
        &self.parser_root
    }

    pub(crate) fn into_parser_root_payload(self) -> FrameDocumentParserRootModuleClient {
        self.parser_root
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameDocumentModuleFetchTerminalResult {
    Fetched(ModuleGraphFetchedSource),
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct FrameDocumentParserRootTerminalPayload {
    key: ModuleMapKey,
    client: FrameDocumentParserRootTerminalClient,
    result: FrameDocumentModuleFetchTerminalResult,
}

pub(crate) type FrameDocumentParserRootTerminalWork =
    DocumentRealmTask<FrameDocumentTaskOwner, FrameRealmId, FrameDocumentParserRootTerminalPayload>;

impl FrameDocumentParserRootTerminalWork {
    pub(crate) fn from_terminal_parts(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        client: FrameDocumentParserRootTerminalClient,
        result: FrameDocumentModuleFetchTerminalResult,
    ) -> Self {
        Self::new(
            owner,
            realm_id,
            FrameDocumentParserRootTerminalPayload {
                key,
                client,
                result,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn client(&self) -> &FrameDocumentParserRootTerminalClient {
        &self.payload().client
    }

    #[cfg(test)]
    pub(crate) fn parser_root_payload(&self) -> &FrameDocumentParserRootModuleClient {
        self.client().parser_root_payload()
    }

    #[cfg(test)]
    pub(crate) fn result(&self) -> &FrameDocumentModuleFetchTerminalResult {
        &self.payload().result
    }

    pub(crate) fn into_terminal_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        FrameRealmId,
        ModuleMapKey,
        FrameDocumentParserRootModuleClient,
        FrameDocumentModuleFetchTerminalResult,
    ) {
        let (owner, realm_id, payload) = self.into_parts();
        (
            owner,
            realm_id,
            payload.key,
            payload.client.into_parser_root_payload(),
            payload.result,
        )
    }
}
