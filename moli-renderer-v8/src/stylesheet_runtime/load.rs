use std::sync::{Arc, OnceLock};

use crate::frame_owner_model::{DocumentLinkEventOwner, MainDocumentStyleLoadEventBinding};
use crate::module_runtime::{NativeModulepreloadFetchStart, NativeModulepreloadLinkClient};
use crate::style_engine::OwnerStyleSheetSource;
use crate::stylesheet_blocking::{
    StylesheetBlockingOperation, StylesheetFetch, StylesheetFetchOptions, StylesheetFetchTerminal,
};
use crate::types::SubresourceResourceType;

use super::{ConnectedStyleImportRoot, DomHandle, Url};

/// Immutable element class captured when a connected stylesheet operation is
/// created.
///
/// Async completion must not re-read the live DOM to choose a task source: the
/// owner can be detached or replaced before its terminal arrives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConnectedStyleEventElementKind {
    Style,
    Link,
}

/// One accepted style/link load event task.
///
/// Once processing has completed and this task is queued, later owner
/// invalidation can revoke source-install authority but must not erase the
/// already-posted event. Keeping the result on the task also allows multiple
/// completed processings for the same element to coexist without an
/// owner-keyed last-write-wins result map.
#[derive(Debug, Clone)]
pub(crate) struct ReadyConnectedStyleLoad {
    successful: bool,
    operation: ReadyConnectedStyleLoadOperation,
}

#[derive(Debug, Clone)]
pub(in crate::document_runtime) enum ReadyConnectedStyleLoadOperation {
    Connected(Arc<ConnectedLoadOperation>),
    StylesheetLink(Arc<StylesheetLinkClient>),
    NativeModulepreload(Arc<NativeModulepreloadLinkClient>),
}

impl ReadyConnectedStyleLoad {
    #[cfg(test)]
    pub(crate) fn for_owner(
        owner: DomHandle,
        successful: bool,
        element_kind: ConnectedStyleEventElementKind,
    ) -> Self {
        Self::for_operation(
            ConnectedLoadOperation::new_with_load_event_binding(
                owner,
                element_kind,
                ConnectedLoadParameters::ImmediateOwnerProcessing,
                None,
                None,
            ),
            successful,
        )
    }

    pub(in crate::document_runtime) fn for_operation(
        operation: Arc<ConnectedLoadOperation>,
        successful: bool,
    ) -> Self {
        Self {
            successful,
            operation: ReadyConnectedStyleLoadOperation::Connected(operation),
        }
    }

    pub(in crate::document_runtime) fn for_stylesheet_link(
        load: Arc<StylesheetLinkClient>,
        successful: bool,
    ) -> Self {
        Self {
            successful,
            operation: ReadyConnectedStyleLoadOperation::StylesheetLink(load),
        }
    }

    fn for_native_modulepreload(
        client: Arc<NativeModulepreloadLinkClient>,
        successful: bool,
    ) -> Self {
        Self {
            successful,
            operation: ReadyConnectedStyleLoadOperation::NativeModulepreload(client),
        }
    }

    pub(crate) fn owner(&self) -> DomHandle {
        match &self.operation {
            ReadyConnectedStyleLoadOperation::Connected(operation) => operation.owner,
            ReadyConnectedStyleLoadOperation::StylesheetLink(load) => load.owner(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(client) => client.owner(),
        }
    }

    pub(crate) fn successful(&self) -> bool {
        self.successful
    }

    pub(crate) fn load_event_binding(&self) -> Option<MainDocumentStyleLoadEventBinding> {
        match &self.operation {
            ReadyConnectedStyleLoadOperation::Connected(operation) => {
                operation.load_event_binding()
            }
            ReadyConnectedStyleLoadOperation::StylesheetLink(load) => load.load_event_binding(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(_) => None,
        }
    }

    pub(crate) fn element_kind(&self) -> ConnectedStyleEventElementKind {
        match &self.operation {
            ReadyConnectedStyleLoadOperation::Connected(operation) => operation.element_kind(),
            ReadyConnectedStyleLoadOperation::StylesheetLink(_)
            | ReadyConnectedStyleLoadOperation::NativeModulepreload(_) => {
                ConnectedStyleEventElementKind::Link
            }
        }
    }

    pub(in crate::document_runtime) fn operation(&self) -> &ReadyConnectedStyleLoadOperation {
        &self.operation
    }
}

/// One modulepreload link event accepted by the exact connected-owner state
/// machine but not yet published to the Page task source.
///
/// Consuming this value creates the immutable ready event without touching the
/// Document load gate. The network client is never mutated during the
/// transition.
#[derive(Debug)]
pub(crate) struct PendingNativeModulepreloadLinkEvent {
    client: Arc<NativeModulepreloadLinkClient>,
    successful: bool,
}

impl PendingNativeModulepreloadLinkEvent {
    pub(in crate::document_runtime) fn new(
        client: Arc<NativeModulepreloadLinkClient>,
        successful: bool,
    ) -> Self {
        Self { client, successful }
    }

    pub(crate) fn client(&self) -> &Arc<NativeModulepreloadLinkClient> {
        &self.client
    }

    pub(crate) fn into_ready_event(self) -> ReadyConnectedStyleLoad {
        ReadyConnectedStyleLoad::for_native_modulepreload(self.client, self.successful)
    }
}

/// Result of registering one connected modulepreload link with the module
/// map. A terminal is present only when the module map was already terminal.
#[derive(Debug)]
pub(crate) struct NativeModulepreloadLinkFetchOutcome {
    fetch_start: NativeModulepreloadFetchStart,
    pending_event: Option<PendingNativeModulepreloadLinkEvent>,
}

impl NativeModulepreloadLinkFetchOutcome {
    pub(in crate::document_runtime) fn new(
        fetch_start: NativeModulepreloadFetchStart,
        pending_event: Option<PendingNativeModulepreloadLinkEvent>,
    ) -> Self {
        Self {
            fetch_start,
            pending_event,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        NativeModulepreloadFetchStart,
        Option<PendingNativeModulepreloadLinkEvent>,
    ) {
        (self.fetch_start, self.pending_event)
    }
}

/// Lifecycle work that must be committed outside `DocumentRuntime` before a
/// connected style/link owner is processed.
///
/// Preparing this value only reads the DOM. `ScriptVm` commits it while
/// safely borrowing `JsContextHost`, then gives the resulting admission back
/// to `DocumentRuntime` synchronously. No task or event-loop turn may run
/// between those phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectedStyleLoadEventPlan {
    LoadDelaying { element: DomHandle },
    NonBlockingModulepreload { element: DomHandle },
}

impl ConnectedStyleLoadEventPlan {
    pub(in crate::document_runtime) fn load_delaying(element: DomHandle) -> Self {
        Self::LoadDelaying { element }
    }

    pub(in crate::document_runtime) fn non_blocking_modulepreload(element: DomHandle) -> Self {
        Self::NonBlockingModulepreload { element }
    }
}

/// Lifecycle authority committed for one connected style/link plan.
///
/// `modulepreload` links take the identity-only variant, which captures the
/// exact Document and element without touching the Document load gate. This
/// classification is independent of request validity, so error outcomes also
/// remain non-load-delaying. All other owners retain the stylesheet
/// load-event lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectedStyleLoadEventAdmission {
    LoadDelaying(MainDocumentStyleLoadEventBinding),
    NonBlockingModulepreload(DocumentLinkEventOwner),
}

impl ConnectedStyleLoadEventAdmission {
    pub(in crate::document_runtime) fn matches_plan(
        self,
        plan: ConnectedStyleLoadEventPlan,
    ) -> bool {
        match (self, plan) {
            (
                Self::LoadDelaying(binding),
                ConnectedStyleLoadEventPlan::LoadDelaying { element },
            ) => binding.element() == element,
            (
                Self::NonBlockingModulepreload(owner),
                ConnectedStyleLoadEventPlan::NonBlockingModulepreload { element },
            ) => owner.element() == element,
            (
                Self::LoadDelaying(_),
                ConnectedStyleLoadEventPlan::NonBlockingModulepreload { .. },
            )
            | (
                Self::NonBlockingModulepreload(_),
                ConnectedStyleLoadEventPlan::LoadDelaying { .. },
            ) => false,
        }
    }

    pub(in crate::document_runtime) fn load_event_binding(
        self,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        match self {
            Self::LoadDelaying(binding) => Some(binding),
            Self::NonBlockingModulepreload(_) => None,
        }
    }
}

#[derive(Debug)]
pub(in crate::document_runtime) struct QueuedConnectedStyleLoad {
    owner: DomHandle,
    inline_source: Option<Arc<OwnerStyleSheetSource>>,
    event_admission: Option<ConnectedStyleLoadEventAdmission>,
}

impl QueuedConnectedStyleLoad {
    pub(in crate::document_runtime) fn new(
        owner: DomHandle,
        inline_source: Option<Arc<OwnerStyleSheetSource>>,
        event_admission: Option<ConnectedStyleLoadEventAdmission>,
    ) -> Arc<Self> {
        Arc::new(Self {
            owner,
            inline_source,
            event_admission,
        })
    }

    pub(in crate::document_runtime) fn owner(&self) -> DomHandle {
        self.owner
    }

    pub(in crate::document_runtime) fn inline_source(&self) -> Option<&Arc<OwnerStyleSheetSource>> {
        self.inline_source.as_ref()
    }

    pub(in crate::document_runtime) fn event_admission(
        &self,
    ) -> Option<ConnectedStyleLoadEventAdmission> {
        self.event_admission
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::document_runtime) enum ConnectedLoadParameters {
    ImmediateOwnerProcessing,
    StyleImports {
        source: InlineStyleImportSource,
        urls: Vec<Url>,
        roots: Vec<ConnectedStyleImportRoot>,
    },
    PreloadLikeLink {
        url: Url,
        options: Arc<ConnectedLinkReadinessFetchOptions>,
    },
}

/// Exact processing identity for one inline `<style>` import graph.
///
/// Linked stylesheets have a fetch-owned graph lifecycle and deliberately do
/// not use an owner-bound `ConnectedLoadOperation`.
#[derive(Debug, Clone)]
pub(in crate::document_runtime) struct InlineStyleImportSource(Arc<OwnerStyleSheetSource>);

impl InlineStyleImportSource {
    pub(in crate::document_runtime) fn new(source: Arc<OwnerStyleSheetSource>) -> Self {
        Self(source)
    }

    pub(in crate::document_runtime) fn owner(&self) -> DomHandle {
        self.0.owner()
    }

    pub(in crate::document_runtime) fn source(&self) -> &Arc<OwnerStyleSheetSource> {
        &self.0
    }
}

impl PartialEq for InlineStyleImportSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for InlineStyleImportSource {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::document_runtime) struct ConnectedLinkReadinessFetchOptions {
    pub(in crate::document_runtime) resource_type: SubresourceResourceType,
    pub(in crate::document_runtime) request_resource_type: Option<moli_fetch::RequestResourceType>,
    pub(in crate::document_runtime) script_fetch_metadata:
        Option<crate::planning::ScriptFetchMetadata>,
    pub(in crate::document_runtime) request_mode: moli_fetch::RequestMode,
    pub(in crate::document_runtime) credentials_mode: moli_fetch::RequestCredentialsMode,
    pub(in crate::document_runtime) fetch_priority_hint: Option<moli_fetch::FetchPriorityHint>,
    pub(in crate::document_runtime) link_preload: bool,
    pub(in crate::document_runtime) link_fetch_options: StylesheetFetchOptions,
}

/// Identity of one connected style/link processing operation.
///
/// The parameters describe what was captured when processing started, but an
/// asynchronous completion belongs to an owner only when this exact object is
/// still installed in the owner's canonical runtime state. This deliberately
/// prevents same-URL A -> B -> A completions from being accepted by value.
#[derive(Debug)]
pub(crate) struct ConnectedLoadOperation {
    pub(in crate::document_runtime) owner: DomHandle,
    element_kind: ConnectedStyleEventElementKind,
    pub(in crate::document_runtime) parameters: ConnectedLoadParameters,
    pub(super) blocking_operation: Option<StylesheetBlockingOperation>,
    load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
}

impl ConnectedLoadOperation {
    #[cfg(test)]
    pub(in crate::document_runtime) fn new_for_test(
        owner: DomHandle,
        element_kind: ConnectedStyleEventElementKind,
        parameters: ConnectedLoadParameters,
        blocking_operation: Option<StylesheetBlockingOperation>,
    ) -> Arc<Self> {
        Self::new_with_load_event_binding(owner, element_kind, parameters, blocking_operation, None)
    }

    pub(in crate::document_runtime) fn new_with_load_event_binding(
        owner: DomHandle,
        element_kind: ConnectedStyleEventElementKind,
        parameters: ConnectedLoadParameters,
        blocking_operation: Option<StylesheetBlockingOperation>,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
    ) -> Arc<Self> {
        Arc::new(Self {
            owner,
            element_kind,
            parameters,
            blocking_operation,
            load_event_binding,
        })
    }

    pub(in crate::document_runtime) const fn element_kind(&self) -> ConnectedStyleEventElementKind {
        self.element_kind
    }

    pub(in crate::document_runtime) fn load_event_binding(
        &self,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        self.load_event_binding
    }

    pub(in crate::document_runtime) fn ptr_eq(left: &Arc<Self>, right: &Arc<Self>) -> bool {
        Arc::ptr_eq(left, right)
    }

    pub(in crate::document_runtime) fn matches_processing(
        &self,
        parameters: &ConnectedLoadParameters,
        blocking_operation: Option<&StylesheetBlockingOperation>,
    ) -> bool {
        self.parameters == *parameters
            && match (&self.blocking_operation, blocking_operation) {
                (Some(current), Some(candidate)) => current.ptr_eq(candidate),
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Debug)]
pub(crate) struct StylesheetLinkClient {
    owner: DomHandle,
    request_url: Url,
    fetch: StylesheetFetch,
    role: StylesheetLinkClientRole,
    load_event_binding: OnceLock<MainDocumentStyleLoadEventBinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StylesheetLinkClientRole {
    Install,
    Preload,
}

impl StylesheetLinkClient {
    pub(super) fn new(owner: DomHandle, request_url: Url, fetch: StylesheetFetch) -> Arc<Self> {
        Self::new_with_load_event_binding(owner, request_url, fetch, None)
    }

    pub(super) fn new_with_load_event_binding(
        owner: DomHandle,
        request_url: Url,
        fetch: StylesheetFetch,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
    ) -> Arc<Self> {
        Self::new_with_role(
            owner,
            request_url,
            fetch,
            StylesheetLinkClientRole::Install,
            load_event_binding,
        )
    }

    pub(super) fn new_preload_with_load_event_binding(
        owner: DomHandle,
        request_url: Url,
        fetch: StylesheetFetch,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
    ) -> Arc<Self> {
        Self::new_with_role(
            owner,
            request_url,
            fetch,
            StylesheetLinkClientRole::Preload,
            load_event_binding,
        )
    }

    fn new_with_role(
        owner: DomHandle,
        request_url: Url,
        fetch: StylesheetFetch,
        role: StylesheetLinkClientRole,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
    ) -> Arc<Self> {
        let binding = OnceLock::new();
        if let Some(load_event_binding) = load_event_binding {
            binding
                .set(load_event_binding)
                .expect("a new stylesheet link client has no load event binding");
        }
        Arc::new(Self {
            owner,
            request_url,
            fetch,
            role,
            load_event_binding: binding,
        })
    }

    pub(crate) fn owner(&self) -> DomHandle {
        self.owner
    }

    pub(crate) fn request_url(&self) -> &Url {
        &self.request_url
    }

    pub(crate) fn fetch(&self) -> &StylesheetFetch {
        &self.fetch
    }

    pub(crate) fn installs_stylesheet(&self) -> bool {
        self.role == StylesheetLinkClientRole::Install
    }

    pub(in crate::document_runtime) fn load_event_binding(
        &self,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        self.load_event_binding.get().copied()
    }

    pub(super) fn bind_load_event(&self, binding: MainDocumentStyleLoadEventBinding) -> bool {
        match self.load_event_binding.set(binding) {
            Ok(()) => true,
            Err(candidate) => self.load_event_binding.get() == Some(&candidate),
        }
    }

    pub(in crate::document_runtime) fn ptr_eq(left: &Arc<Self>, right: &Arc<Self>) -> bool {
        Arc::ptr_eq(left, right)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StylesheetLinkClientTerminal {
    load: Arc<StylesheetLinkClient>,
    terminal: Arc<StylesheetFetchTerminal>,
}

impl StylesheetLinkClientTerminal {
    pub(super) fn new(
        load: Arc<StylesheetLinkClient>,
        terminal: Arc<StylesheetFetchTerminal>,
    ) -> Self {
        Self { load, terminal }
    }

    pub(crate) fn load(&self) -> &Arc<StylesheetLinkClient> {
        &self.load
    }

    pub(crate) fn terminal(&self) -> &Arc<StylesheetFetchTerminal> {
        &self.terminal
    }
}
