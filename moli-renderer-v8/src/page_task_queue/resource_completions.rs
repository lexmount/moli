use crate::module_script_continuation::MainParserDeferredClassicSourceLoadCompletion;
use crate::page_resource_completion::{
    MainDynamicImportGraphFetchCompletion, MainModulepreloadFetchCompletion,
    MainParserDeferredClassicSourceNetworkAttribution, MainParserModuleGraphFetchCompletion,
    MainRuntimeModuleGraphFetchCompletion, RendererPageResourceCompletion,
    RendererPageResourceCompletionSender, RendererResourceCompletionRouteClosed,
};
use crate::runtime::{
    RendererDocumentLifecycleIdentity, RendererDocumentToken, RendererOwnerRuntimeActivitySource,
    RendererPageToken, RendererRuntimeInspectorResponsePublication,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent,
    ChildBlockingStylesheetLoadCompletion, ChildClassicScriptLoadCompletion,
    ChildDocumentLoadCompletion, ChildDynamicImportFetchCompletion,
    ChildModuleDependencyFetchCompletion, ChildModulepreloadFetchCompletion,
    ChildParserModuleRootFetchCompletion, DocumentWriteExternalScriptLoadCompletion,
    PopupClassicScriptLoadCompletion, PopupDocumentLoadCompletion,
};

#[derive(Debug, Clone)]
pub(crate) struct RendererOwnerWakeSender {
    tx: tokio::sync::mpsc::UnboundedSender<RendererOwnerWake>,
    token: RendererPageToken,
}

/// Identity of one top-level location-navigation request handed from a
/// renderer producer to the Page owner.
///
/// The pending navigation descriptor remains in the ScriptVm's single slot.
/// This identity only authorizes the owner to claim that exact slot value; a
/// delayed wake cannot accidentally start a later replacement request.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct RendererTopLevelNavigationHandoff(u64);

impl RendererTopLevelNavigationHandoff {
    pub(crate) const fn new(request_id: u64) -> Self {
        Self(request_id)
    }
}

#[derive(Debug)]
pub(crate) enum RendererOwnerWake {
    Page {
        token: RendererPageToken,
        source: RendererOwnerWakeSource,
    },
    PostResponseDocumentLifecycle {
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    },
    /// The browser-side commit response has crossed its release boundary.
    /// Parser work parked at DocumentCommit may now resume without racing
    /// ahead of target installation, old-Page retirement, or the response.
    CommittedDocumentParserUnblocked { token: RendererPageToken },
    /// A concrete late Inspector response that became ready inside a Page
    /// owner turn. It is committed only after that turn has published all
    /// protocol-visible output, and never schedules or executes Page work.
    RuntimeInspectorResponsePublication {
        token: RendererPageToken,
        publication: RendererRuntimeInspectorResponsePublication,
    },
    /// A renderer producer installed one exact request in the Page's pending
    /// top-level navigation slot. Unlike `ProducedActivityOutput`, this is an
    /// internal execution handoff rather than a capture-only wake.
    TopLevelNavigationHandoff {
        token: RendererPageToken,
        handoff: RendererTopLevelNavigationHandoff,
    },
    /// The stable Page view either committed or rejected one exact
    /// replacement PageVm identity. This wake only re-admits commands parked
    /// on that identity; ordinary Page activity cannot satisfy the condition.
    ReplacementDocumentViewSettled {
        token: RendererPageToken,
        vm_creation_id: u64,
    },
}

impl RendererOwnerWake {
    pub(crate) fn page(token: RendererPageToken, source: RendererOwnerWakeSource) -> Self {
        Self::Page { token, source }
    }

    pub(crate) fn post_response_document_lifecycle(
        token: RendererPageToken,
        document: RendererDocumentLifecycleIdentity,
    ) -> Self {
        Self::PostResponseDocumentLifecycle { token, document }
    }

    pub(crate) fn committed_document_parser_unblocked(token: RendererPageToken) -> Self {
        Self::CommittedDocumentParserUnblocked { token }
    }

    pub(crate) fn runtime_inspector_response_publication(
        token: RendererPageToken,
        publication: RendererRuntimeInspectorResponsePublication,
    ) -> Self {
        Self::RuntimeInspectorResponsePublication { token, publication }
    }

    pub(crate) fn top_level_navigation_handoff(
        token: RendererPageToken,
        handoff: RendererTopLevelNavigationHandoff,
    ) -> Self {
        Self::TopLevelNavigationHandoff { token, handoff }
    }

    pub(crate) fn replacement_document_view_settled(
        token: RendererPageToken,
        vm_creation_id: u64,
    ) -> Self {
        Self::ReplacementDocumentViewSettled {
            token,
            vm_creation_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn page_id(&self) -> crate::PageId {
        match self {
            Self::Page { token, .. }
            | Self::PostResponseDocumentLifecycle { token, .. }
            | Self::CommittedDocumentParserUnblocked { token }
            | Self::RuntimeInspectorResponsePublication { token, .. }
            | Self::TopLevelNavigationHandoff { token, .. }
            | Self::ReplacementDocumentViewSettled { token, .. } => token.page_id(),
        }
    }

    #[cfg(test)]
    pub(crate) fn source_for_test(&self) -> RendererOwnerWakeSource {
        match self {
            Self::Page { source, .. } => *source,
            Self::PostResponseDocumentLifecycle { .. } => RendererOwnerWakeSource::Runtime(
                RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
            ),
            Self::CommittedDocumentParserUnblocked { .. } => panic!(
                "a committed-Document parser release is an owner continuation gate, not a Page source wake"
            ),
            Self::RuntimeInspectorResponsePublication { .. } => {
                panic!("a concrete Runtime response publication is not a Page scheduling wake")
            }
            Self::TopLevelNavigationHandoff { .. } => panic!(
                "a top-level navigation handoff is an execution handoff, not a Page source wake"
            ),
            Self::ReplacementDocumentViewSettled { .. } => panic!(
                "a replacement Document settlement is a command admission fact, not a Page source wake"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum RendererOwnerWakeSource {
    /// A bounded Page turn left ordinary work runnable. This wake requests a
    /// fresh arbitration pass and deliberately carries no work identity.
    SchedulerContinuation,
    /// A task entered the Page's shared HTML networking source. The queued
    /// payload owns execution identity; this is only an admission hint and
    /// cannot itself create a scheduler task.
    NetworkingTask,
    /// A parser-discovered async script source terminal entered the stable
    /// parse-time Document-script source. While phase one is parked, only its
    /// exact continuation may consume this payload; the wake must not admit a
    /// second generic Page turn.
    ParseTimeDocumentScriptWork,
    /// A task entered the Page's HTML DOM-manipulation source. The typed
    /// payload owns execution authority; APIs sharing this source also share
    /// FIFO and scheduler fairness.
    DomManipulationTask,
    /// A task entered the Page's HTML user-interaction source. The typed
    /// payload owns exact Document authority; this wake is admission-only.
    UserInteractionTask,
    /// A callback task entered the Page's HTML file-reading source. The exact
    /// reader request and callback payload remain in the creating Host.
    FileReadingTask,
    /// A callback entered the exact Window/Document miscellaneous-platform
    /// task source. The Host-local payload owns callback Realm identity.
    MiscPlatformApiTask,
    /// One exact Window/realm history traversal entered the stable HTML
    /// history-traversal task source.
    NavigationAndTraversalTask,
    /// One exact Document acquired pending work in the HTML rendering task
    /// source. The queued payload, not this admission hint, owns authority.
    RenderingUpdateTask,
    /// One exact Window/Document media-element event entered its stable HTML
    /// task source. The queued payload owns execution authority.
    MediaElementEventTask,
    /// A DedicatedWorker wrapper event entered its exact Window-realm source.
    /// The bound producer owns worker identity; this wake is admission-only.
    DedicatedWorkerClientEvent,
    /// A SharedWorker client error/terminal event entered its exact
    /// Window-realm source. The bound producer owns client identity.
    SharedWorkerClientEvent,
    /// A browser-context ServiceWorker internal callback entered the exact
    /// root-Document source owned by one PageVm.
    ServiceWorkerInternalTask,
    /// A ServiceWorker `message` entered the exact Window-client source.
    ServiceWorkerClientMessage,
    /// A completed WebCrypto operation entered its exact Page/Window task
    /// source. The queued task, not this admission hint, owns identity.
    WebCryptoTask,
    /// A Page-side IndexedDB task entered its exact Window-realm source.
    IndexedDbTask,
    /// An OPFS owner completion entered its exact Window-realm source.
    OpfsTask,
    /// One exact main-Document HTML internal-loading task entered its stable
    /// Page source.
    InternalLoadingTask,
    /// One exact main-Document runtime/script action entered the stable
    /// internal-continue-script-loading source.
    MainDocumentRuntimeTask,
    /// One exact child Document/realm static-module dependency fetch start
    /// entered its stable Page source.
    ChildModuleDependencyFetchStart,
    /// One exact child Document/realm module-map terminal fanout entered its
    /// stable Page source.
    ChildModuleScriptTerminal,
    /// One exact child Document/realm modulepreload load/error action entered
    /// its stable Page source.
    ChildModulepreloadEventAction,
    /// One exact child-frame task entered the shared Page-owned family.
    ChildFrameTask,
    /// V8 posted one foreground continuation for the stable Page isolate.
    /// The queued task retains its exact isolate-registration generation.
    V8ForegroundTask,
    /// A module Promise callback produced one exact-Document/realm host
    /// continuation. The queued reaction owns execution identity.
    ModuleReaction,
    /// A Window.postMessage task entered its Page-owned exact-LocalWindow
    /// source, or a blocked head became eligible after realm materialization.
    WindowMessageTask,
    /// A page-side MessagePort attachment has one exact-realm delivery
    /// opportunity ready for the Page scheduler.
    MessagePortDelivery,
    /// An exact-Document/realm dynamic-import owner action entered its stable
    /// Page source. This wake only requests scheduling; it is not protocol
    /// activity.
    DynamicImportOwnerAction,
    /// An exact-target child modulepreload start entered the stable Page
    /// runnable source. This is an internal scheduling hint, not a protocol
    /// observable runtime activity source.
    ModulepreloadStart,
    Runtime(RendererOwnerRuntimeActivitySource),
}

impl RendererOwnerWakeSender {
    pub(crate) fn new(
        tx: tokio::sync::mpsc::UnboundedSender<RendererOwnerWake>,
        token: RendererPageToken,
    ) -> Self {
        Self { tx, token }
    }

    pub(crate) fn signal_scheduler_continuation(&self) {
        self.signal_source(RendererOwnerWakeSource::SchedulerContinuation);
    }

    pub(crate) fn signal_networking_task(&self) {
        self.signal_source(RendererOwnerWakeSource::NetworkingTask);
    }

    pub(crate) fn signal_parse_time_document_script_work(&self) {
        self.signal_source(RendererOwnerWakeSource::ParseTimeDocumentScriptWork);
    }

    pub(crate) fn signal_dom_manipulation_task(&self) {
        self.signal_source(RendererOwnerWakeSource::DomManipulationTask);
    }

    pub(crate) fn signal_user_interaction_task(&self) {
        self.signal_source(RendererOwnerWakeSource::UserInteractionTask);
    }

    pub(crate) fn signal_file_reading_task(&self) {
        self.signal_source(RendererOwnerWakeSource::FileReadingTask);
    }

    pub(crate) fn signal_misc_platform_api_task(&self) {
        self.signal_source(RendererOwnerWakeSource::MiscPlatformApiTask);
    }

    pub(crate) fn signal_navigation_and_traversal_task(&self) {
        self.signal_source(RendererOwnerWakeSource::NavigationAndTraversalTask);
    }

    pub(crate) fn signal_rendering_update_task(&self) {
        self.signal_source(RendererOwnerWakeSource::RenderingUpdateTask);
    }

    pub(crate) fn signal_media_element_event_task(&self) {
        self.signal_source(RendererOwnerWakeSource::MediaElementEventTask);
    }

    pub(crate) fn signal_dedicated_worker_client_event(&self) {
        self.signal_source(RendererOwnerWakeSource::DedicatedWorkerClientEvent);
    }

    pub(crate) fn signal_shared_worker_client_event(&self) {
        self.signal_source(RendererOwnerWakeSource::SharedWorkerClientEvent);
    }

    pub(crate) fn signal_service_worker_internal_task(&self) {
        self.signal_source(RendererOwnerWakeSource::ServiceWorkerInternalTask);
    }

    pub(crate) fn signal_service_worker_client_message(&self) {
        self.signal_source(RendererOwnerWakeSource::ServiceWorkerClientMessage);
    }

    pub(crate) fn signal_webcrypto_task(&self) {
        self.signal_source(RendererOwnerWakeSource::WebCryptoTask);
    }

    pub(crate) fn signal_indexed_db_task(&self) {
        self.signal_source(RendererOwnerWakeSource::IndexedDbTask);
    }

    pub(crate) fn signal_opfs_task(&self) {
        self.signal_source(RendererOwnerWakeSource::OpfsTask);
    }

    pub(crate) fn signal_internal_loading_task(&self) {
        self.signal_source(RendererOwnerWakeSource::InternalLoadingTask);
    }

    pub(crate) fn signal_main_document_runtime_task(&self) {
        self.signal_source(RendererOwnerWakeSource::MainDocumentRuntimeTask);
    }

    pub(crate) fn signal_child_module_dependency_fetch_start(&self) {
        self.signal_source(RendererOwnerWakeSource::ChildModuleDependencyFetchStart);
    }

    pub(crate) fn signal_child_module_script_terminal(&self) {
        self.signal_source(RendererOwnerWakeSource::ChildModuleScriptTerminal);
    }

    pub(crate) fn signal_child_modulepreload_event_action(&self) {
        self.signal_source(RendererOwnerWakeSource::ChildModulepreloadEventAction);
    }

    pub(crate) fn signal_child_frame_task(&self) {
        self.signal_source(RendererOwnerWakeSource::ChildFrameTask);
    }

    pub(crate) fn signal_v8_foreground_task(&self) {
        self.signal_source(RendererOwnerWakeSource::V8ForegroundTask);
    }

    pub(crate) fn signal_module_reaction(&self) {
        self.signal_source(RendererOwnerWakeSource::ModuleReaction);
    }

    pub(crate) fn signal_window_message_task(&self) {
        self.signal_source(RendererOwnerWakeSource::WindowMessageTask);
    }

    pub(crate) fn signal_message_port_delivery(&self) {
        self.signal_source(RendererOwnerWakeSource::MessagePortDelivery);
    }

    pub(crate) fn token(&self) -> RendererPageToken {
        self.token
    }

    pub(crate) fn signal_modulepreload_start(&self) {
        self.signal_source(RendererOwnerWakeSource::ModulepreloadStart);
    }

    pub(crate) fn signal_dynamic_import_owner_action(&self) {
        self.signal_source(RendererOwnerWakeSource::DynamicImportOwnerAction);
    }

    pub(crate) fn signal_document_lifecycle_turn(&self) {
        self.signal_source(RendererOwnerWakeSource::Runtime(
            RendererOwnerRuntimeActivitySource::DocumentLifecycleTurn,
        ));
    }

    pub(crate) fn signal_top_level_navigation_handoff(
        &self,
        handoff: RendererTopLevelNavigationHandoff,
    ) -> bool {
        self.tx
            .send(RendererOwnerWake::top_level_navigation_handoff(
                self.token, handoff,
            ))
            .is_ok()
    }
    pub(crate) fn defer_runtime_inspector_response_publication(
        &self,
        publication: RendererRuntimeInspectorResponsePublication,
    ) -> Result<(), crate::runtime::RendererRuntimeInspectorAsyncCompletion> {
        match self
            .tx
            .send(RendererOwnerWake::runtime_inspector_response_publication(
                self.token,
                publication,
            )) {
            Ok(()) => Ok(()),
            Err(error) => {
                let RendererOwnerWake::RuntimeInspectorResponsePublication { publication, .. } =
                    error.0
                else {
                    unreachable!("runtime response send must return its exact wake payload")
                };
                publication.commit(None)
            }
        }
    }

    fn signal_source(&self, source: RendererOwnerWakeSource) {
        let _ = self.tx.send(RendererOwnerWake::page(self.token, source));
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RendererResourceCompletionSender {
    page_completion_route: Option<RendererPageResourceCompletionRoute>,
}

#[derive(Debug, Clone)]
struct RendererPageResourceCompletionRoute {
    sender: RendererPageResourceCompletionSender,
    root_document: RendererDocumentToken,
}

impl RendererResourceCompletionSender {
    pub(crate) fn for_page_scheduler(
        page_completion_sender: RendererPageResourceCompletionSender,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            page_completion_route: Some(RendererPageResourceCompletionRoute {
                sender: page_completion_sender,
                root_document,
            }),
        }
    }

    /// Capability used by ServiceWorker interception paths whose actual
    /// result travels through a dedicated oneshot channel.
    ///
    /// It deliberately owns no Page route. If an error path accidentally
    /// attempts a Page completion, the typed send returns `RouteClosed`.
    pub(crate) fn direct_completion_only() -> Self {
        Self {
            page_completion_route: None,
        }
    }

    /// Narrow constructor for tests that exercise one production Networking
    /// resource route without constructing unrelated Page producers.
    ///
    /// The resource terminal still enters the real typed source and carries
    /// the exact root-Document stamp without acquiring any unrelated Page
    /// producer capability.
    #[cfg(test)]
    pub(crate) fn for_page_resource_test(
        page_completion_sender: RendererPageResourceCompletionSender,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            page_completion_route: Some(RendererPageResourceCompletionRoute {
                sender: page_completion_sender,
                root_document,
            }),
        }
    }

    fn send_page_completion(
        &self,
        make_completion: impl FnOnce(RendererDocumentToken) -> RendererPageResourceCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        let route = self
            .page_completion_route
            .as_ref()
            .ok_or(RendererResourceCompletionRouteClosed)?;
        route.sender.send(make_completion(route.root_document))
    }

    pub(crate) fn send_async_subresource(
        &self,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_async_subresource_event(AsyncSubresourceFetchEvent::Completion(Box::new(
            completion,
        )))
    }

    pub(crate) fn send_async_subresource_event(
        &self,
        event: AsyncSubresourceFetchEvent,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::async_subresource(root_document, event)
        })
    }

    pub(crate) fn send_document_write_external_script(
        &self,
        completion: DocumentWriteExternalScriptLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::document_write_external_script(
                root_document,
                completion,
            )
        })
    }

    pub(crate) fn send_main_parser_deferred_classic_source_load(
        &self,
        completion: MainParserDeferredClassicSourceLoadCompletion,
        network_attribution: MainParserDeferredClassicSourceNetworkAttribution,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::main_parser_deferred_classic_source(
                root_document,
                completion,
                network_attribution,
            )
        })
    }

    /// Sends one exact-PendingScript main parser module fetch terminal.
    ///
    /// All Page producers use the stable typed queue. A missing or closed route
    /// is terminal and never falls back to the legacy aggregate.
    pub(crate) fn send_main_parser_module_graph_fetch(
        &self,
        completion: MainParserModuleGraphFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::main_parser_module_graph_fetch(
                root_document,
                completion,
            )
        })
    }

    /// Sends one exact dynamic-script-owner main runtime module fetch terminal.
    ///
    /// All Page producers use the stable typed queue. A missing or closed route
    /// is terminal and never falls back to the legacy aggregate.
    pub(crate) fn send_main_runtime_module_graph_fetch(
        &self,
        completion: MainRuntimeModuleGraphFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::main_runtime_module_graph_fetch(
                root_document,
                completion,
            )
        })
    }

    /// Sends one exact main-Document dynamic-import graph terminal.
    ///
    /// A missing or closed Page route is terminal. Dynamic-import completions
    /// never fall back to the requester/order-based legacy aggregate.
    pub(crate) fn send_main_dynamic_import_graph_fetch(
        &self,
        completion: MainDynamicImportGraphFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::main_dynamic_import_graph_fetch(
                root_document,
                completion,
            )
        })
    }

    /// Sends one exact-Document module-map modulepreload fetch terminal.
    ///
    /// All Page producers use the stable typed queue. A missing or closed route
    /// is terminal and never falls back to the legacy aggregate.
    pub(crate) fn send_main_modulepreload_fetch(
        &self,
        completion: MainModulepreloadFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::main_modulepreload_fetch(root_document, completion)
        })
    }

    pub(crate) fn send_child_classic_script(
        &self,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_classic_script(root_document, completion)
        })
    }

    pub(crate) fn send_child_blocking_stylesheet(
        &self,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_blocking_stylesheet(root_document, completion)
        })
    }

    pub(crate) fn send_child_parser_module_root_fetch(
        &self,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_parser_module_root_fetch(
                root_document,
                completion,
            )
        })
    }

    pub(crate) fn send_child_module_dependency_fetch(
        &self,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_module_dependency_fetch(root_document, completion)
        })
    }

    /// Sends one exact-target child `modulepreload` terminal.
    ///
    /// All Page producers use the stable typed queue. A missing or closed route
    /// is terminal and never falls back to the legacy aggregate.
    pub(crate) fn send_child_modulepreload_fetch(
        &self,
        completion: ChildModulepreloadFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_modulepreload_fetch(root_document, completion)
        })
    }

    /// Sends one exact-target child dynamic-import fetch terminal.
    ///
    /// All Page producers use the stable typed queue. A missing or closed route
    /// is terminal and never falls back to the legacy aggregate.
    pub(crate) fn send_child_dynamic_import_fetch(
        &self,
        completion: ChildDynamicImportFetchCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_dynamic_import_fetch(root_document, completion)
        })
    }

    pub(crate) fn send_child_document(
        &self,
        completion: ChildDocumentLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::child_document_load(root_document, completion)
        })
    }

    pub(crate) fn send_popup_document(
        &self,
        completion: PopupDocumentLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::popup_document_load(root_document, completion)
        })
    }

    pub(crate) fn send_popup_classic_script(
        &self,
        completion: PopupClassicScriptLoadCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.send_page_completion(|root_document| {
            RendererPageResourceCompletion::popup_classic_script(root_document, completion)
        })
    }
}

/// Low-level resource-terminal fixture retaining the production Networking
/// source and its unique consumer.
///
/// Tests may claim resource heads directly, but there is no alternate
/// completion queue and no conversion back into a generic aggregate. Other
/// Page capabilities, including WebSocket ingress, are deliberately absent
/// from this resource-specific facade.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct RendererResourceCompletionTestHarness {
    page_residence: crate::page_task_queue::RendererPageTaskTestResidence,
    root_document: RendererDocumentToken,
    page_wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
}

#[cfg(test)]
impl RendererResourceCompletionTestHarness {
    pub(crate) fn new() -> Self {
        let (page_wake_tx, page_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            page_wake_tx,
            RendererPageToken::new_for_testing(crate::PageId::new_for_testing(1)),
        );
        let page_residence =
            crate::page_task_queue::RendererPageTaskTestResidence::new(Some(owner_wake.clone()));
        let root_document = page_residence.root_document();
        Self {
            page_residence,
            root_document,
            page_wake_rx,
        }
    }

    pub(crate) fn sender(&self) -> RendererResourceCompletionSender {
        let senders = self
            .page_residence
            .runtime_source()
            .bound_task_producer_senders(self.root_document)
            .expect("test Page residence must expose all typed producer routes");
        let (_, _, page_completion_sender, _, _, _) = senders.into_parts();
        RendererResourceCompletionSender::for_page_scheduler(
            page_completion_sender,
            self.root_document,
        )
    }

    pub(crate) fn pop_next_page_completion(&mut self) -> Option<RendererPageResourceCompletion> {
        let task = self
            .page_residence
            .task_sources()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                        owner:
                            crate::page_task_queue::RendererPageNetworkingOwner::ResourceCompletion(
                                _
                            ),
                        ..
                    }
                )
            })?;
        let crate::page_task_queue::RendererPageSchedulerTask::Networking(
            crate::page_task_queue::RendererPageNetworkingTask::ResourceCompletion(completion),
        ) = task
        else {
            panic!("resource networking descriptor dequeued a different task variant")
        };
        Some(*completion)
    }

    pub(crate) fn pop_next_page_terminal(
        &mut self,
    ) -> Option<crate::page_resource_completion::RendererPageResourceTerminal> {
        self.pop_next_page_completion()
            .map(RendererPageResourceCompletion::into_terminal)
    }

    pub(crate) fn pop_next_async_subresource_event(
        &mut self,
    ) -> Option<AsyncSubresourceFetchEvent> {
        let terminal = self.pop_next_page_terminal()?;
        let crate::page_resource_completion::RendererPageResourceTerminal::AsyncSubresource {
            event,
        } = terminal
        else {
            panic!("expected async-subresource terminal, got {terminal:?}");
        };
        Some(*event)
    }

    pub(crate) fn has_ready_completion(&mut self) -> bool {
        self.page_residence
            .task_sources()
            .has_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                        owner:
                            crate::page_task_queue::RendererPageNetworkingOwner::ResourceCompletion(
                                _
                            ),
                        ..
                    }
                )
            })
    }

    pub(crate) async fn wait_for_arrival_without_timeout(&mut self) -> bool {
        loop {
            if self.has_ready_completion() {
                // A Page wake is only an admission hint. Once the stable
                // source exposes work, retaining that hint would let the next
                // test wait return before any new task was published.
                while self.page_wake_rx.try_recv().is_ok() {}
                return true;
            }
            if self.page_wake_rx.recv().await.is_none() {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_module_script_tree as module_tree;
    use url::Url;

    use super::{
        RendererOwnerWake, RendererOwnerWakeSender, RendererOwnerWakeSource,
        RendererResourceCompletionSender,
    };
    use crate::document_script_scheduler::{ParserPendingScriptId, ParserPendingScriptKey};
    use crate::dynamic_script_owner::DynamicScriptOwnerId;
    use crate::frame_owner_model::{
        ChildDocumentModuleFetchTarget, DocumentId, FrameDocumentModuleClientEntryId,
        FrameDocumentModuleClientId, FrameDocumentModuleClientRegistration,
        FrameDocumentModuleClientReservation, FrameDocumentModuleDependencyFetchTask,
        FrameDocumentModuleFetchDisposition, FrameDocumentStaticDependencyModuleClient,
        FrameDocumentTaskOwner, FrameRealmId, FrameRequestId, FrameSchedulerLaneId, LocalWindowId,
    };
    use crate::module_runtime::{
        DynamicModuleImportOwner, ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource,
        ModuleImportPhase, ModuleKind, ModuleMapKey, ModuleSource, NativeModuleGraphFetchRequest,
    };
    use crate::module_script_continuation::{
        MainParserDeferredClassicSourceLoadCompletion, MainParserDocumentOwner,
    };
    use crate::page_resource_completion::{
        MainDynamicImportGraphFetchCompletion, MainDynamicImportGraphFetchTarget,
        MainModuleFetchNetworkAttribution, MainModulepreloadFetchCompletion,
        MainModulepreloadFetchTarget, MainParserDeferredClassicSourceNetworkAttribution,
        MainParserModuleGraphFetchCompletion, MainParserModuleGraphFetchTarget,
        MainRuntimeModuleGraphFetchCompletion, MainRuntimeModuleGraphFetchTarget,
        RendererPageResourceCompletionOwner, RendererPageResourceTerminal,
    };
    use crate::page_task_queue::RendererPageNetworkingSource;
    use crate::planning::PreparedScriptSourceLoadOutcome;
    use crate::runtime::{RendererDocumentToken, RendererPageToken};
    use crate::types::{
        AsyncSubresourceFetchCompletion, ChildBlockingStylesheetLoadCompletion,
        ChildClassicScriptLoadCompletion, ChildClassicScriptNetworkAttribution,
        ChildDocumentLoadCompletion, ChildDocumentLoadOutcome, ChildDynamicImportFetchCompletion,
        ChildModuleDependencyFetchCompletion, ChildModuleFetchNetworkAttribution,
        ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion,
        DocumentWriteExternalScriptLoadCompletion, LoadedChildDocument,
        PopupDocumentLoadCompletion, PopupDocumentLoadOutcome,
    };

    fn owner_attached_page_queue(
        token: RendererPageToken,
        wake_tx: tokio::sync::mpsc::UnboundedSender<RendererOwnerWake>,
    ) -> RendererPageNetworkingSource {
        let owner_wake = RendererOwnerWakeSender::new(wake_tx, token);
        RendererPageNetworkingSource::new_owner_attached(
            crate::page_task_queue::PageRuntimeWakeSignal::default(),
            owner_wake,
        )
    }

    #[tokio::test]
    async fn typed_resource_route_emits_non_authorizing_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let page_id = crate::PageId::new_for_testing(72);
        let mut page_queue =
            owner_attached_page_queue(RendererPageToken::new_for_testing(page_id), wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(page_id, 1),
        );

        sender
            .send_document_write_external_script(
                DocumentWriteExternalScriptLoadCompletion::for_test(1),
            )
            .expect("migrated terminal should enter its Page queue");
        let migrated = wake_rx
            .recv()
            .await
            .expect("migrated terminal should wake the Page owner");
        assert_eq!(
            migrated.source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );
        assert!(
            page_queue.pop_front_task().is_some(),
            "a second Networking admission wake requires the first task to be consumed"
        );

        sender
            .send_async_subresource(async_subresource_completion(2))
            .expect("async-subresource terminal should enter Networking");
        let async_subresource = wake_rx
            .recv()
            .await
            .expect("async-subresource terminal should wake the Page owner");
        assert_eq!(
            async_subresource.source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );
        assert!(page_queue.pop_front_task().is_some());
    }

    #[test]
    fn direct_completion_capability_rejects_page_routes_without_panicking() {
        let sender = RendererResourceCompletionSender::direct_completion_only();
        assert!(
            sender
                .send_document_write_external_script(
                    DocumentWriteExternalScriptLoadCompletion::for_test(91),
                )
                .is_err(),
            "direct-result ServiceWorker paths must not silently acquire a Page terminal route"
        );
    }

    fn async_subresource_completion(internal_id: u64) -> AsyncSubresourceFetchCompletion {
        AsyncSubresourceFetchCompletion {
            internal_id,
            request_url: Url::parse("https://example.test/api").unwrap(),
            request_method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            result: Err("test".to_owned()),
        }
    }

    fn document_write_completion(load_id: u64) -> DocumentWriteExternalScriptLoadCompletion {
        DocumentWriteExternalScriptLoadCompletion::for_test(load_id)
    }

    fn main_parser_deferred_completion(
        owner: FrameDocumentTaskOwner,
        parser_position: usize,
    ) -> MainParserDeferredClassicSourceLoadCompletion {
        let pending_script_id = ParserPendingScriptId::from_key(
            MainParserDocumentOwner::new(owner),
            ParserPendingScriptKey::from_parts_for_test(
                parser_position,
                moli_dom::NodeId::new(parser_position + 1),
            ),
        );
        MainParserDeferredClassicSourceLoadCompletion::new(
            pending_script_id,
            PreparedScriptSourceLoadOutcome {
                source_result: Ok(format!("window.defer{parser_position} = true")),
                source_bytes: None,
                network_result: None,
            },
        )
    }

    fn main_parser_deferred_network_attribution(
        parser_position: usize,
    ) -> MainParserDeferredClassicSourceNetworkAttribution {
        MainParserDeferredClassicSourceNetworkAttribution::new(
            Url::parse("https://example.test/document").unwrap(),
            Url::parse(&format!("https://example.test/defer-{parser_position}.js")).unwrap(),
        )
    }

    fn main_parser_module_target(
        owner: FrameDocumentTaskOwner,
        parser_position: usize,
        load_id: u64,
    ) -> MainParserModuleGraphFetchTarget {
        MainParserModuleGraphFetchTarget::new(
            ParserPendingScriptId::from_key(
                MainParserDocumentOwner::new(owner),
                ParserPendingScriptKey::from_parts_for_test(
                    parser_position,
                    moli_dom::NodeId::new(parser_position + 101),
                ),
            ),
            load_id,
        )
    }

    fn main_parser_module_completion(
        target: MainParserModuleGraphFetchTarget,
    ) -> MainParserModuleGraphFetchCompletion {
        let request_url = Url::parse(&format!(
            "https://example.test/parser-module-{}.mjs",
            target.load_id()
        ))
        .unwrap();
        MainParserModuleGraphFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text("export default 1;".to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(
                Url::parse("https://example.test/document").unwrap(),
                request_url,
            ),
        )
    }

    fn main_runtime_module_target(
        owner: FrameDocumentTaskOwner,
        dynamic_script_owner_id: u64,
        load_id: u64,
    ) -> MainRuntimeModuleGraphFetchTarget {
        MainRuntimeModuleGraphFetchTarget::new(
            owner,
            DynamicScriptOwnerId::from_u64(dynamic_script_owner_id),
            load_id,
        )
    }

    fn main_runtime_module_completion(
        target: MainRuntimeModuleGraphFetchTarget,
    ) -> MainRuntimeModuleGraphFetchCompletion {
        let request_url = Url::parse(&format!(
            "https://example.test/runtime-module-{}.mjs",
            target.load_id()
        ))
        .unwrap();
        MainRuntimeModuleGraphFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text("export default 1;".to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(
                Url::parse("https://example.test/document").unwrap(),
                request_url,
            ),
        )
    }

    fn main_dynamic_import_target(load_id: u64) -> MainDynamicImportGraphFetchTarget {
        MainDynamicImportGraphFetchTarget::new(
            DynamicModuleImportOwner::main_for_test_parts(31, 37, 41),
            load_id,
        )
    }

    fn main_dynamic_import_completion(
        target: MainDynamicImportGraphFetchTarget,
    ) -> MainDynamicImportGraphFetchCompletion {
        let request_url = Url::parse(&format!(
            "https://example.test/dynamic-import-{}.mjs",
            target.load_id()
        ))
        .unwrap();
        MainDynamicImportGraphFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text("export default 1;".to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(
                Url::parse("https://example.test/document").unwrap(),
                request_url,
            ),
        )
    }

    fn main_modulepreload_target(
        owner: FrameDocumentTaskOwner,
        load_id: u64,
    ) -> MainModulepreloadFetchTarget {
        MainModulepreloadFetchTarget::new(owner, load_id)
    }

    fn main_modulepreload_completion(
        target: MainModulepreloadFetchTarget,
    ) -> MainModulepreloadFetchCompletion {
        let request_url = Url::parse(&format!(
            "https://example.test/modulepreload-{}.mjs",
            target.load_id()
        ))
        .unwrap();
        MainModulepreloadFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                request_url.clone(),
                false,
                ModuleSource::text("export default 1;".to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(
                Url::parse("https://example.test/document").unwrap(),
                request_url,
            ),
        )
    }

    fn frame_document_task_owner(document_id: u64) -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(3),
            LocalWindowId(5),
            DocumentId(document_id),
        )
    }

    fn child_blocking_stylesheet_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildBlockingStylesheetLoadCompletion {
        ChildBlockingStylesheetLoadCompletion {
            child_handle,
            owner,
            signature: crate::DocumentBlockingStylesheetSignature::ParserCreatedStyleImport {
                urls: Vec::new(),
            },
            network_results: Vec::new(),
        }
    }

    fn child_classic_script_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildClassicScriptLoadCompletion {
        ChildClassicScriptLoadCompletion {
            owner,
            load_id: 17,
            handle: child_handle,
            script_handle: moli_dom::native::NativeNodeId::new(19),
            result: Ok("globalThis.childClassic = true".to_owned()),
            network_result: None,
            network_attribution: ChildClassicScriptNetworkAttribution {
                frame_id: Some("child-frame".to_owned()),
                document_url: Url::parse("https://example.test/child").unwrap(),
                request_url: Url::parse("https://example.test/child.js").unwrap(),
            },
        }
    }

    fn child_module_network_attribution(request_url: Url) -> ChildModuleFetchNetworkAttribution {
        ChildModuleFetchNetworkAttribution::parser(
            Some("child-module-frame".to_owned()),
            Url::parse("https://example.test/child-module-document").unwrap(),
            request_url,
        )
    }

    fn child_modulepreload_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildModulepreloadFetchCompletion {
        let request_url = Url::parse("https://example.test/child-modulepreload.js").unwrap();
        ChildModulepreloadFetchCompletion::new(
            ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(59)),
            83,
            Err("modulepreload fetch failed for route test".to_owned()),
            None,
            child_module_network_attribution(request_url),
        )
    }

    fn child_dynamic_import_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildDynamicImportFetchCompletion {
        let request_url = Url::parse("https://example.test/child-dynamic-import.js").unwrap();
        ChildDynamicImportFetchCompletion::new(
            ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(59)),
            89,
            Err("dynamic import fetch failed for route test".to_owned()),
            None,
            ChildModuleFetchNetworkAttribution::dynamic_import(
                Some("child-module-frame".to_owned()),
                Url::parse("https://example.test/child-module-document").unwrap(),
                request_url,
            ),
        )
    }

    fn child_parser_module_root_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildParserModuleRootFetchCompletion {
        let request_url = Url::parse("https://example.test/child-root.js").unwrap();
        ChildParserModuleRootFetchCompletion::new(
            ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(59)),
            FrameRequestId(61),
            ModuleMapKey::java_script(request_url.clone()),
            Err("root fetch failed for route test".to_owned()),
            None,
            child_module_network_attribution(request_url),
        )
    }

    fn child_module_dependency_completion(
        child_handle: moli_dom::native::NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> ChildModuleDependencyFetchCompletion {
        let parent_url = Url::parse("https://example.test/child-root.js").unwrap();
        let dependency_url = Url::parse("https://example.test/child-dependency.js").unwrap();
        let parent_key = ModuleMapKey::java_script(parent_url.clone());
        let dependency_key = ModuleMapKey::java_script(dependency_url.clone());
        let parent_entry_id = ModuleEntryId::from_raw(67);
        let tree_client = module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(71),
            sequence: 73,
        };
        let client = FrameDocumentStaticDependencyModuleClient::new(
            parent_entry_id,
            parent_key.clone(),
            "./child-dependency.js".to_owned(),
            ModuleImportPhase::Evaluation,
            tree_client,
        );
        let entry_id = FrameDocumentModuleClientEntryId::from_raw(79);
        let reservation = FrameDocumentModuleClientReservation::new(
            owner.document_owner(),
            dependency_key.clone(),
            FrameDocumentModuleClientRegistration::new(
                entry_id,
                FrameDocumentModuleClientId::from_raw(83),
                FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
            ),
        );
        let task = FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
            owner,
            FrameRealmId(59),
            dependency_key.clone(),
            client,
            reservation,
            NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
                dependency_url.clone(),
                parent_url,
                ModuleFetchMetadata::default(),
                ModuleKind::JavaScript,
                tree_client,
                dependency_key,
                parent_key,
                parent_entry_id,
                "./child-dependency.js".to_owned(),
                ModuleImportPhase::Evaluation,
            ),
        );
        ChildModuleDependencyFetchCompletion::new(
            child_handle,
            FrameRequestId(89),
            task,
            Err("dependency fetch failed for route test".to_owned()),
            None,
            child_module_network_attribution(dependency_url),
        )
    }

    fn child_document_completion(load_id: u64) -> ChildDocumentLoadCompletion {
        ChildDocumentLoadCompletion::for_test(
            load_id,
            moli_dom::NodeId::new(7),
            Ok(ChildDocumentLoadOutcome::Loaded(Box::new(
                LoadedChildDocument {
                    final_url: Url::parse("https://example.test/child").unwrap(),
                    policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
                    content_type: Some("text/html".to_owned()),
                    character_set: "UTF-8".to_owned(),
                    document_network: None,
                    markup: "<!doctype html><main>child</main>".to_owned(),
                },
            ))),
        )
    }

    fn popup_document_completion(load_id: u64) -> PopupDocumentLoadCompletion {
        PopupDocumentLoadCompletion::new(
            crate::native_bridge::LightweightPopupDocumentFetchTarget::for_test(
                load_id,
                crate::native_bridge::LightweightPopupNavigationTaskToken::for_test(
                    crate::native_bridge::LightweightPopupDocumentOwner::new(
                        9,
                        crate::native_bridge::LightweightPopupDocumentId::new(1),
                    ),
                    1,
                ),
            ),
            Ok(PopupDocumentLoadOutcome::Loaded(Box::new(
                LoadedChildDocument {
                    final_url: Url::parse("https://example.test/popup").unwrap(),
                    policy_container: crate::document_runtime::DocumentPolicyContainer::default(),
                    content_type: Some("text/html".to_owned()),
                    character_set: "UTF-8".to_owned(),
                    document_network: None,
                    markup: "<!doctype html><main>popup</main>".to_owned(),
                },
            ))),
        )
    }

    #[test]
    fn page_scheduler_route_stamps_exact_owner() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(19));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 7);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let owner = frame_document_task_owner(7);

        sender
            .send_main_parser_deferred_classic_source_load(
                main_parser_deferred_completion(owner, 11),
                main_parser_deferred_network_attribution(11),
            )
            .expect("typed parser-deferred completion should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("page queue should retain the typed completion");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::main_document(root_document, owner)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::MainParserDeferredClassicSource { .. }
        ));
        let wake = wake_rx
            .try_recv()
            .expect("one typed enqueue should publish one page wake");
        assert_eq!(wake.page_id(), token.page_id());
        assert!(
            wake_rx.try_recv().is_err(),
            "producer emitted a duplicate wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_exact_main_parser_module_target_and_one_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(20));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 8);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let target = main_parser_module_target(frame_document_task_owner(8), 17, 29);

        sender
            .send_main_parser_module_graph_fetch(main_parser_module_completion(target))
            .expect("typed main parser module terminal should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("stable Page queue should retain parser module terminal");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                root_document,
                target,
            )
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::MainParserModuleGraphFetch { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed enqueue should wake Page")
                .page_id(),
            token.page_id()
        );
        assert!(wake_rx.try_recv().is_err(), "typed enqueue woke Page twice");
    }

    #[test]
    fn closed_main_parser_module_page_route_has_no_legacy_fallback_or_phantom_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(24));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(token.page_id(), 2),
        );
        drop(page_queue);

        let target = main_parser_module_target(frame_document_task_owner(2), 19, 31);
        assert!(
            sender
                .send_main_parser_module_graph_fetch(main_parser_module_completion(target))
                .is_err(),
            "closed stable route must reject parser module terminal"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "failed parser module enqueue must not publish a Page wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_exact_main_runtime_module_target_and_one_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(25));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 11);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let target = main_runtime_module_target(frame_document_task_owner(9), 13, 41);

        sender
            .send_main_runtime_module_graph_fetch(main_runtime_module_completion(target))
            .expect("typed main runtime module terminal should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("stable Page queue should retain runtime module terminal");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
                root_document,
                target,
            )
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::MainRuntimeModuleGraphFetch { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed enqueue should wake Page")
                .page_id(),
            token.page_id()
        );
        assert!(wake_rx.try_recv().is_err(), "typed enqueue woke Page twice");
    }

    #[test]
    fn closed_main_runtime_module_page_route_has_no_legacy_fallback_or_phantom_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(26));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(token.page_id(), 12),
        );
        drop(page_queue);

        let target = main_runtime_module_target(frame_document_task_owner(10), 17, 43);
        assert!(
            sender
                .send_main_runtime_module_graph_fetch(main_runtime_module_completion(target))
                .is_err(),
            "closed stable route must reject runtime module terminal"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "failed runtime module enqueue must not publish a Page wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_exact_main_dynamic_import_target_and_networking_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(29));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 15);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let target = main_dynamic_import_target(59);

        sender
            .send_main_dynamic_import_graph_fetch(main_dynamic_import_completion(target))
            .expect("typed main dynamic-import terminal should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("stable Page queue should retain dynamic-import terminal");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::main_dynamic_import_graph_fetch(
                root_document,
                target,
            )
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::MainDynamicImportGraphFetch { .. }
        ));
        let wake = wake_rx
            .try_recv()
            .expect("typed dynamic-import enqueue should wake Page");
        assert_eq!(wake.page_id(), token.page_id());
        assert_eq!(
            wake.source_for_test(),
            RendererOwnerWakeSource::NetworkingTask
        );
        assert!(wake_rx.try_recv().is_err(), "typed enqueue woke Page twice");
    }

    #[test]
    fn closed_main_dynamic_import_route_has_no_legacy_fallback_or_phantom_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(30));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(token.page_id(), 16),
        );
        drop(page_queue);

        let target = main_dynamic_import_target(61);
        assert!(
            sender
                .send_main_dynamic_import_graph_fetch(main_dynamic_import_completion(target))
                .is_err(),
            "closed stable route must reject dynamic-import terminal"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "failed dynamic-import enqueue must not publish a phantom Page wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_exact_main_modulepreload_target_and_one_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(27));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 13);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let target = main_modulepreload_target(frame_document_task_owner(12), 53);

        sender
            .send_main_modulepreload_fetch(main_modulepreload_completion(target))
            .expect("typed main modulepreload terminal should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("stable Page queue should retain modulepreload terminal");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::main_modulepreload_fetch(root_document, target)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::MainModulepreloadFetch { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed enqueue should wake Page")
                .page_id(),
            token.page_id()
        );
        assert!(wake_rx.try_recv().is_err(), "typed enqueue woke Page twice");
    }

    #[test]
    fn closed_main_modulepreload_page_route_has_no_legacy_fallback_or_phantom_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(28));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(token.page_id(), 14),
        );
        drop(page_queue);

        let target = main_modulepreload_target(frame_document_task_owner(13), 59);
        assert!(
            sender
                .send_main_modulepreload_fetch(main_modulepreload_completion(target))
                .is_err(),
            "closed stable route must reject modulepreload terminal"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "failed modulepreload enqueue must not publish a Page wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_document_write_target_and_emits_one_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(21));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 9);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let completion = document_write_completion(13);
        let target = completion.target();

        sender
            .send_document_write_external_script(completion)
            .expect("typed document.write completion should enqueue");
        let (_, completion) = page_queue
            .pop_front()
            .expect("stable Page queue should retain document.write completion");
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::document_write_external_script(
                root_document,
                target,
            )
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::DocumentWriteExternalScript { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed enqueue should wake Page")
                .page_id(),
            token.page_id()
        );
        assert!(wake_rx.try_recv().is_err(), "typed enqueue woke Page twice");
    }

    #[test]
    fn closed_document_write_page_route_rejects_after_typed_route_closes() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(22));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            RendererDocumentToken::new_for_testing(token.page_id(), 1),
        );
        drop(page_queue);

        assert!(
            sender
                .send_document_write_external_script(document_write_completion(17))
                .is_err(),
            "closed stable route must reject the typed terminal"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "failed enqueue must not publish a runnable Page wake"
        );
    }

    #[test]
    fn stable_page_route_accepts_late_old_and_replacement_document_terminals() {
        let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(23));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let old_root_document = RendererDocumentToken::new_for_testing(token.page_id(), 1);
        let replacement_root_document = RendererDocumentToken::new_for_testing(token.page_id(), 2);
        let stable_route = page_queue.sender();
        let old_document_sender = RendererResourceCompletionSender::for_page_resource_test(
            stable_route.clone(),
            old_root_document,
        );
        let replacement_document_sender = RendererResourceCompletionSender::for_page_resource_test(
            stable_route.clone(),
            replacement_root_document,
        );
        assert!(
            stable_route.same_route_as(&page_queue.sender()),
            "cross-Document runtime hooks must preserve the Page-owned channel"
        );
        let colliding_local_owner = frame_document_task_owner(0);

        replacement_document_sender
            .send_main_parser_deferred_classic_source_load(
                main_parser_deferred_completion(colliding_local_owner, 2),
                main_parser_deferred_network_attribution(2),
            )
            .unwrap();
        old_document_sender
            .send_main_parser_deferred_classic_source_load(
                main_parser_deferred_completion(colliding_local_owner, 1),
                main_parser_deferred_network_attribution(1),
            )
            .unwrap();

        let first = page_queue.pop_front().unwrap().1;
        let second = page_queue.pop_front().unwrap().1;
        assert_eq!(
            first.owner(),
            RendererPageResourceCompletionOwner::main_document(
                replacement_root_document,
                colliding_local_owner,
            )
        );
        assert_eq!(
            second.owner(),
            RendererPageResourceCompletionOwner::main_document(
                old_root_document,
                colliding_local_owner,
            ),
            "the old Document producer must remain connected and retain its namespace after replacement"
        );
        assert!(!page_queue.has_ready_completion());
    }

    #[test]
    fn page_scheduler_route_preserves_child_document_owner() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(29));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 5);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let child_handle = moli_dom::native::NativeNodeId::new(41);
        let owner = frame_document_task_owner(43);

        sender
            .send_child_blocking_stylesheet(child_blocking_stylesheet_completion(
                child_handle,
                owner,
            ))
            .expect("typed child stylesheet completion should enqueue");
        let completion = page_queue
            .pop_front()
            .expect("page queue should retain the child completion")
            .1;
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::child_document(root_document, child_handle, owner,)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::ChildBlockingStylesheet { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed child enqueue should publish one page wake")
                .page_id(),
            token.page_id()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "producer emitted a duplicate wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_child_classic_owner() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(31));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 6);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let child_handle = moli_dom::native::NativeNodeId::new(47);
        let owner = frame_document_task_owner(53);

        sender
            .send_child_classic_script(child_classic_script_completion(child_handle, owner))
            .expect("typed child classic completion should enqueue");
        let completion = page_queue
            .pop_front()
            .expect("page queue should retain the child classic completion")
            .1;
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::child_document(root_document, child_handle, owner,)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::ChildClassicScript { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("typed child classic enqueue should publish one page wake")
                .page_id(),
            token.page_id()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "producer emitted a duplicate wake"
        );
    }

    #[test]
    fn closed_page_route_does_not_fall_back_to_legacy_child_classic_queue() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(32));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let page_route = page_queue.sender();
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 7);
        let sender =
            RendererResourceCompletionSender::for_page_resource_test(page_route, root_document);
        drop(page_queue);

        let result = sender.send_child_classic_script(child_classic_script_completion(
            moli_dom::native::NativeNodeId::new(49),
            frame_document_task_owner(55),
        ));

        assert!(
            result.is_err(),
            "a retired Page route must report closure to its producer"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "a rejected terminal must not publish a phantom owner wake"
        );
    }

    #[test]
    fn page_scheduler_route_preserves_exact_child_navigation_target() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(33));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 11);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let completion = child_document_completion(59);
        let target = completion.target();

        sender
            .send_child_document(completion)
            .expect("typed child navigation terminal should enqueue");
        let completion = page_queue
            .pop_front()
            .expect("stable Page queue should retain the child navigation terminal")
            .1;
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::child_document_navigation(root_document, target,)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::ChildDocumentLoad { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted child navigation terminal should publish one Page wake")
                .page_id(),
            token.page_id()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "one accepted child navigation terminal must not publish a duplicate wake"
        );
    }

    #[test]
    fn closed_page_route_does_not_fall_back_to_legacy_child_document_queue() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(34));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let page_route = page_queue.sender();
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 13);
        let sender =
            RendererResourceCompletionSender::for_page_resource_test(page_route, root_document);
        drop(page_queue);

        let result = sender.send_child_document(child_document_completion(61));

        assert!(
            result.is_err(),
            "retired Page route must report closure to the child navigation producer"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "rejected child navigation terminal must not publish a phantom owner wake"
        );
    }

    #[test]
    fn page_scheduler_route_preserves_exact_popup_target() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(35));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 17);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let completion = popup_document_completion(63);
        let target = completion.target();

        sender
            .send_popup_document(completion)
            .expect("typed popup terminal should enqueue");
        let completion = page_queue
            .pop_front()
            .expect("stable Page queue should retain the popup terminal")
            .1;
        assert_eq!(
            completion.owner(),
            RendererPageResourceCompletionOwner::popup_document_load(root_document, target)
        );
        assert!(matches!(
            completion.terminal(),
            RendererPageResourceTerminal::PopupDocumentLoad { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted popup terminal should publish one Page wake")
                .page_id(),
            token.page_id()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "one accepted popup terminal must not publish a duplicate wake"
        );
    }

    #[test]
    fn closed_page_route_does_not_fall_back_to_legacy_popup_queue() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(36));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let page_route = page_queue.sender();
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 19);
        let sender =
            RendererResourceCompletionSender::for_page_resource_test(page_route, root_document);
        drop(page_queue);

        let result = sender.send_popup_document(popup_document_completion(67));

        assert!(
            result.is_err(),
            "retired Page route must report closure to the popup producer"
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "rejected popup terminal must not publish a phantom owner wake"
        );
    }

    #[test]
    fn page_scheduler_route_stamps_child_module_owners() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(37));
        let mut page_queue = owner_attached_page_queue(token, wake_tx);
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 11);
        let sender = RendererResourceCompletionSender::for_page_resource_test(
            page_queue.sender(),
            root_document,
        );
        let child_handle = moli_dom::native::NativeNodeId::new(97);
        let owner = frame_document_task_owner(101);

        sender
            .send_child_parser_module_root_fetch(child_parser_module_root_completion(
                child_handle,
                owner,
            ))
            .expect("typed child module root completion should enqueue");
        sender
            .send_child_module_dependency_fetch(child_module_dependency_completion(
                child_handle,
                owner,
            ))
            .expect("typed child module dependency completion should enqueue");
        sender
            .send_child_modulepreload_fetch(child_modulepreload_completion(child_handle, owner))
            .expect("typed child modulepreload completion should enqueue");
        sender
            .send_child_dynamic_import_fetch(child_dynamic_import_completion(child_handle, owner))
            .expect("typed child dynamic-import completion should enqueue");
        let root = page_queue
            .pop_front()
            .expect("page queue should retain the root completion")
            .1;
        let dependency = page_queue
            .pop_front()
            .expect("page queue should retain the dependency completion")
            .1;
        let modulepreload = page_queue
            .pop_front()
            .expect("page queue should retain the modulepreload completion")
            .1;
        let dynamic_import = page_queue
            .pop_front()
            .expect("page queue should retain the dynamic-import completion")
            .1;
        let expected_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(59)),
        );
        assert_eq!(root.owner(), expected_owner);
        assert_eq!(dependency.owner(), expected_owner);
        assert_eq!(modulepreload.owner(), expected_owner);
        assert_eq!(dynamic_import.owner(), expected_owner);
        assert!(matches!(
            root.terminal(),
            RendererPageResourceTerminal::ChildParserModuleRootFetch { .. }
        ));
        assert!(matches!(
            dependency.terminal(),
            RendererPageResourceTerminal::ChildModuleDependencyFetch { .. }
        ));
        assert!(matches!(
            modulepreload.terminal(),
            RendererPageResourceTerminal::ChildModulepreloadFetch { .. }
        ));
        assert!(matches!(
            dynamic_import.terminal(),
            RendererPageResourceTerminal::ChildDynamicImportFetch { .. }
        ));
        assert_eq!(
            wake_rx
                .try_recv()
                .expect("the first accepted module terminal should publish one Page wake")
                .page_id(),
            token.page_id()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "terminals appended while the Page source is ready must share one wake"
        );
        assert!(!page_queue.has_ready_completion());
    }

    #[test]
    fn closed_page_route_does_not_fall_back_to_legacy_child_module_queues() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(41));
        let page_queue = owner_attached_page_queue(token, wake_tx);
        let page_route = page_queue.sender();
        let root_document = RendererDocumentToken::new_for_testing(token.page_id(), 13);
        let sender =
            RendererResourceCompletionSender::for_page_resource_test(page_route, root_document);
        drop(page_queue);
        let child_handle = moli_dom::native::NativeNodeId::new(103);
        let owner = frame_document_task_owner(107);

        assert!(
            sender
                .send_child_parser_module_root_fetch(child_parser_module_root_completion(
                    child_handle,
                    owner,
                ))
                .is_err()
        );
        assert!(
            sender
                .send_child_module_dependency_fetch(child_module_dependency_completion(
                    child_handle,
                    owner,
                ))
                .is_err()
        );
        assert!(
            sender
                .send_child_modulepreload_fetch(child_modulepreload_completion(child_handle, owner))
                .is_err()
        );
        assert!(
            sender
                .send_child_dynamic_import_fetch(child_dynamic_import_completion(
                    child_handle,
                    owner,
                ))
                .is_err()
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "rejected module terminals must not publish phantom owner wakes"
        );
    }

    #[test]
    fn async_subresource_rejects_a_sender_without_a_page_route() {
        let sender = RendererResourceCompletionSender::direct_completion_only();

        assert!(
            sender
                .send_async_subresource(async_subresource_completion(11))
                .is_err(),
            "typed async-subresource completion requires a stable Page route"
        );
    }
}
