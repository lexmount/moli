use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[cfg(test)]
use super::RendererPageOwnedTaskSourcesTestHarness;
use super::{
    RendererPageOwnedTaskSources, RendererPageTaskProducerRoutes,
    dynamic_import_owner_action::RendererPageDynamicImportOwnerActionSender,
    main_document_runtime::RendererPageMainDocumentRuntimeRouteClosed,
    modulepreload_start::RendererPageModulepreloadStartSender,
    resource_completions::RendererOwnerWakeSender,
};
use crate::page_resource_completion::RendererPageResourceCompletionSender;
use crate::page_task_queue::RendererPageChildFrameTaskSender;
use crate::page_task_queue::RendererPageChildModuleDependencyFetchStartSender;
use crate::page_task_queue::RendererPageChildModuleScriptTerminalSender;
use crate::page_task_queue::RendererPageChildModulepreloadEventActionSender;
use crate::page_task_queue::RendererPageDedicatedWorkerClientEventSender;
use crate::page_task_queue::RendererPageFileReadingSender;
use crate::page_task_queue::RendererPageIndexedDbTaskSender;
use crate::page_task_queue::RendererPageInternalLoadingSender;
use crate::page_task_queue::RendererPageMainParserContinuationSender;
use crate::page_task_queue::RendererPageMediaElementEventSender;
use crate::page_task_queue::RendererPageMessagePortDeliverySender;
use crate::page_task_queue::RendererPageMiscPlatformApiSender;
use crate::page_task_queue::RendererPageModuleReactionSender;
use crate::page_task_queue::RendererPageOpfsTaskSender;
use crate::page_task_queue::RendererPageServiceWorkerTaskSender;
use crate::page_task_queue::RendererPageSharedWorkerClientEventSender;
use crate::page_task_queue::RendererPageStylesheetTaskSender;
use crate::page_task_queue::RendererPageTextTrackLoadSender;
use crate::page_task_queue::RendererPageV8ForegroundTaskSender;
use crate::page_task_queue::RendererPageWebCryptoTaskSender;
use crate::page_task_queue::RendererPageWebSocketSender;
use crate::page_task_queue::RendererPageWindowMessageSender;
use crate::page_task_queue::RendererWorkerHostBridgeEventSender;
use crate::page_task_queue::{
    RendererPageDomManipulationSender, RendererPageMainDocumentRuntimeProducer,
    RendererPageMainDocumentRuntimeSender, RendererPageNavigationAndTraversalSender,
    RendererPageRenderingUpdateSender, RendererPageUserInteractionSender,
};

/// Complete producer capability set installed on an owner-attached PageVm.
///
/// A PageVm must never observe a partially bound set: all typed routes are
/// created together by the stable Page source and become available only after
/// the owner has reserved the document isolate.
pub(crate) struct RendererPageTaskProducerSenders {
    js_context: RendererPageJsContextTaskSenders,
    main_document_runtime: RendererPageMainDocumentRuntimeSender,
    resource_completion: RendererPageResourceCompletionSender,
    main_parser_continuation: RendererPageMainParserContinuationSender,
    stylesheet: RendererPageStylesheetTaskSender,
    service_worker: RendererPageServiceWorkerTaskSender,
}

/// Complete PageVm-stamped capability bundle installed in `JsContextHost`.
///
/// Keeping this bundle intact prevents positional sender tuples and makes a
/// partially installed Window task surface unrepresentable. Resource
/// completion stays separate because it is a constructor dependency of the
/// native host rather than a route read from this capability cell.
pub(crate) struct RendererPageJsContextTaskSenders {
    modulepreload_start: RendererPageModulepreloadStartSender,
    dynamic_import_owner_action: RendererPageDynamicImportOwnerActionSender,
    dom_manipulation: RendererPageDomManipulationSender,
    user_interaction: RendererPageUserInteractionSender,
    file_reading: RendererPageFileReadingSender,
    misc_platform_api: RendererPageMiscPlatformApiSender,
    navigation_and_traversal: RendererPageNavigationAndTraversalSender,
    rendering_update: RendererPageRenderingUpdateSender,
    media_element_event: RendererPageMediaElementEventSender,
    text_track_load: RendererPageTextTrackLoadSender,
    dedicated_worker_client_event: RendererPageDedicatedWorkerClientEventSender,
    shared_worker_client_event: RendererPageSharedWorkerClientEventSender,
    worker_host_bridge: RendererWorkerHostBridgeEventSender,
    webcrypto_task: RendererPageWebCryptoTaskSender,
    indexed_db_task: RendererPageIndexedDbTaskSender,
    opfs_task: RendererPageOpfsTaskSender,
    internal_loading: RendererPageInternalLoadingSender,
    child_module_dependency_fetch_start: RendererPageChildModuleDependencyFetchStartSender,
    child_module_script_terminal: RendererPageChildModuleScriptTerminalSender,
    child_modulepreload_event_action: RendererPageChildModulepreloadEventActionSender,
    child_frame_task: RendererPageChildFrameTaskSender,
    module_reaction: RendererPageModuleReactionSender,
    window_message: RendererPageWindowMessageSender,
    message_port_delivery: RendererPageMessagePortDeliverySender,
    websocket: RendererPageWebSocketSender,
}

impl RendererPageTaskProducerSenders {
    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererPageJsContextTaskSenders,
        RendererPageMainDocumentRuntimeSender,
        RendererPageResourceCompletionSender,
        RendererPageMainParserContinuationSender,
        RendererPageStylesheetTaskSender,
        RendererPageServiceWorkerTaskSender,
    ) {
        (
            self.js_context,
            self.main_document_runtime,
            self.resource_completion,
            self.main_parser_continuation,
            self.stylesheet,
            self.service_worker,
        )
    }
}

impl RendererPageJsContextTaskSenders {
    pub(crate) fn modulepreload_start(&self) -> &RendererPageModulepreloadStartSender {
        &self.modulepreload_start
    }

    pub(crate) fn dynamic_import_owner_action(
        &self,
    ) -> &RendererPageDynamicImportOwnerActionSender {
        &self.dynamic_import_owner_action
    }

    pub(crate) fn dom_manipulation(&self) -> &RendererPageDomManipulationSender {
        &self.dom_manipulation
    }

    pub(crate) fn user_interaction(&self) -> &RendererPageUserInteractionSender {
        &self.user_interaction
    }

    pub(crate) fn file_reading(&self) -> &RendererPageFileReadingSender {
        &self.file_reading
    }

    pub(crate) fn misc_platform_api(&self) -> &RendererPageMiscPlatformApiSender {
        &self.misc_platform_api
    }

    pub(crate) fn navigation_and_traversal(&self) -> &RendererPageNavigationAndTraversalSender {
        &self.navigation_and_traversal
    }

    pub(crate) fn rendering_update(&self) -> &RendererPageRenderingUpdateSender {
        &self.rendering_update
    }

    pub(crate) fn media_element_event(&self) -> &RendererPageMediaElementEventSender {
        &self.media_element_event
    }

    pub(crate) fn text_track_load(&self) -> &RendererPageTextTrackLoadSender {
        &self.text_track_load
    }

    pub(crate) fn dedicated_worker_client_event(
        &self,
    ) -> &RendererPageDedicatedWorkerClientEventSender {
        &self.dedicated_worker_client_event
    }

    pub(crate) fn shared_worker_client_event(&self) -> &RendererPageSharedWorkerClientEventSender {
        &self.shared_worker_client_event
    }

    pub(crate) fn worker_host_bridge(&self) -> &RendererWorkerHostBridgeEventSender {
        &self.worker_host_bridge
    }

    pub(crate) fn webcrypto_task(&self) -> &RendererPageWebCryptoTaskSender {
        &self.webcrypto_task
    }

    pub(crate) fn indexed_db_task(&self) -> &RendererPageIndexedDbTaskSender {
        &self.indexed_db_task
    }

    pub(crate) fn opfs_task(&self) -> &RendererPageOpfsTaskSender {
        &self.opfs_task
    }

    pub(crate) fn internal_loading(&self) -> &RendererPageInternalLoadingSender {
        &self.internal_loading
    }

    pub(crate) fn child_module_dependency_fetch_start(
        &self,
    ) -> &RendererPageChildModuleDependencyFetchStartSender {
        &self.child_module_dependency_fetch_start
    }

    pub(crate) fn child_module_script_terminal(
        &self,
    ) -> &RendererPageChildModuleScriptTerminalSender {
        &self.child_module_script_terminal
    }

    pub(crate) fn child_modulepreload_event_action(
        &self,
    ) -> &RendererPageChildModulepreloadEventActionSender {
        &self.child_modulepreload_event_action
    }

    pub(crate) fn child_frame_task(&self) -> &RendererPageChildFrameTaskSender {
        &self.child_frame_task
    }

    pub(crate) fn module_reaction(&self) -> &RendererPageModuleReactionSender {
        &self.module_reaction
    }

    pub(crate) fn window_message(&self) -> &RendererPageWindowMessageSender {
        &self.window_message
    }

    pub(crate) fn message_port_delivery(&self) -> &RendererPageMessagePortDeliverySender {
        &self.message_port_delivery
    }

    /// Exact-Document WebSocket ingress capability for this Window.
    ///
    /// WebSocket network events are Page tasks, not generic resource
    /// completions. Keeping this sender in the atomically installed
    /// JavaScript-context capability set prevents a resource-only or
    /// direct-result host from accidentally acquiring WebSocket authority.
    pub(crate) fn websocket(&self) -> &RendererPageWebSocketSender {
        &self.websocket
    }
}

#[derive(Debug, Default)]
struct PageRuntimeWakeState {
    pending: AtomicUsize,
    next_top_level_navigation_handoff_id: AtomicU64,
    notify: tokio::sync::Notify,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PageRuntimeWakeSignal {
    state: Arc<PageRuntimeWakeState>,
}

impl PageRuntimeWakeSignal {
    pub(crate) fn send(&self) {
        self.state.pending.fetch_add(1, Ordering::Release);
        self.state.notify.notify_one();
    }

    pub(super) fn take_ready(&self) -> bool {
        self.state.pending.swap(0, Ordering::AcqRel) != 0
    }

    fn next_top_level_navigation_handoff(&self) -> super::RendererTopLevelNavigationHandoff {
        let request_id = self
            .state
            .next_top_level_navigation_handoff_id
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        super::RendererTopLevelNavigationHandoff::new(request_id)
    }

    pub(crate) async fn wait(&self) {
        loop {
            if self.take_ready() {
                return;
            }
            self.state.notify.notified().await;
        }
    }

    pub(super) fn clear(&self) {
        self.state.pending.store(0, Ordering::Release);
    }
}

/// Page-lifetime carrier for runtime-owned task sources and their wake route.
///
/// Cross-document navigation replaces document task queues, but it does not
/// replace the page isolate or stable Page task sources. Keeping this carrier
/// stable prevents navigation from unregistering the isolate, dropping V8
/// foreground work, or moving exact-Document tasks into a replacement queue.
#[derive(Clone)]
pub(crate) struct PageRuntimeTaskSource {
    wake: PageRuntimeWakeSignal,
    owner_wake: Option<RendererOwnerWakeSender>,
    page_task_producer_routes: Rc<RefCell<Option<RendererPageTaskProducerRoutes>>>,
}

impl std::fmt::Debug for PageRuntimeTaskSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageRuntimeTaskSource")
            .field(
                "has_bound_page_task_routes",
                &self.page_task_producer_routes.borrow().is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl PageRuntimeTaskSource {
    pub(crate) fn new(owner_wake: Option<RendererOwnerWakeSender>) -> Self {
        Self {
            wake: PageRuntimeWakeSignal::default(),
            owner_wake,
            page_task_producer_routes: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn identity_key(&self) -> usize {
        Rc::as_ptr(&self.page_task_producer_routes) as usize
    }

    pub(crate) fn v8_foreground_task_sender(&self) -> Option<RendererPageV8ForegroundTaskSender> {
        self.page_task_producer_routes
            .borrow()
            .as_ref()
            .map(RendererPageTaskProducerRoutes::v8_foreground_task_sender)
    }

    pub(crate) fn bound_task_producer_senders(
        &self,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<RendererPageTaskProducerSenders> {
        let routes = self.page_task_producer_routes.borrow();
        let routes = routes.as_ref()?;
        Some(RendererPageTaskProducerSenders {
            js_context: RendererPageJsContextTaskSenders {
                modulepreload_start: routes.modulepreload_start_sender(root_document),
                dynamic_import_owner_action: routes
                    .dynamic_import_owner_action_sender(root_document),
                dom_manipulation: routes.dom_manipulation_sender(root_document),
                user_interaction: routes.user_interaction_sender(root_document),
                file_reading: routes.file_reading_sender(root_document),
                misc_platform_api: routes.misc_platform_api_sender(root_document),
                navigation_and_traversal: routes.navigation_and_traversal_sender(root_document),
                rendering_update: routes.rendering_update_sender(root_document),
                media_element_event: routes.media_element_event_sender(root_document),
                text_track_load: routes.text_track_load_sender(root_document),
                dedicated_worker_client_event: routes
                    .dedicated_worker_client_event_sender(root_document),
                shared_worker_client_event: routes.shared_worker_client_event_sender(root_document),
                worker_host_bridge: routes.worker_host_bridge_event_sender(root_document),
                webcrypto_task: routes.webcrypto_task_sender(root_document),
                indexed_db_task: routes.indexed_db_task_sender(root_document),
                opfs_task: routes.opfs_task_sender(root_document),
                internal_loading: routes.internal_loading_sender(root_document),
                child_module_dependency_fetch_start: routes
                    .child_module_dependency_fetch_start_sender(root_document),
                child_module_script_terminal: routes
                    .child_module_script_terminal_sender(root_document),
                child_modulepreload_event_action: routes
                    .child_modulepreload_event_action_sender(root_document),
                child_frame_task: routes.child_frame_task_sender(root_document),
                module_reaction: routes.module_reaction_sender(root_document),
                window_message: routes.window_message_sender(root_document),
                message_port_delivery: routes.message_port_delivery_sender(root_document),
                websocket: routes.websocket_sender(root_document),
            },
            main_document_runtime: routes.main_document_runtime_sender(root_document),
            resource_completion: routes.resource_completion_sender(),
            main_parser_continuation: routes.main_parser_continuation_sender(root_document),
            stylesheet: routes.stylesheet_task_sender(root_document),
            service_worker: routes.service_worker_task_sender(root_document),
        })
    }

    #[cfg(test)]
    pub(crate) fn dynamic_import_owner_action_sender(
        &self,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> Option<RendererPageDynamicImportOwnerActionSender> {
        self.owner_wake.as_ref()?;
        self.page_task_producer_routes
            .borrow()
            .as_ref()
            .map(|routes| routes.dynamic_import_owner_action_sender(root_document))
    }

    pub(crate) fn bind_page_task_producer_routes(
        &self,
        routes: RendererPageTaskProducerRoutes,
    ) -> anyhow::Result<()> {
        let mut bound_routes = self.page_task_producer_routes.borrow_mut();
        anyhow::ensure!(
            bound_routes.is_none(),
            "Page runtime task source already has stable Page producer routes"
        );
        *bound_routes = Some(routes);
        Ok(())
    }

    pub(crate) fn resource_completion_sender(
        &self,
    ) -> Option<RendererPageResourceCompletionSender> {
        self.page_task_producer_routes
            .borrow()
            .as_ref()
            .map(RendererPageTaskProducerRoutes::resource_completion_sender)
    }

    pub(crate) fn owner_attached_page_source_wakes(
        &self,
    ) -> Option<(PageRuntimeWakeSignal, RendererOwnerWakeSender)> {
        Some((self.wake.clone(), self.owner_wake.clone()?))
    }

    pub(crate) fn page_task_producer_routes_match(
        &self,
        sources: &RendererPageOwnedTaskSources,
    ) -> bool {
        self.page_task_producer_routes
            .borrow()
            .as_ref()
            .is_some_and(|routes| sources.routes_match(routes))
    }

    #[cfg(test)]
    fn page_source_owner_wake_for_test(&self) -> RendererOwnerWakeSender {
        self.owner_wake.clone().unwrap_or_else(|| {
            let (wake_tx, _wake_rx) = tokio::sync::mpsc::unbounded_channel();
            RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(
                    u64::MAX,
                )),
            )
        })
    }

    #[cfg(test)]
    fn owner_attached_runtime_page_task_sender_for_test(
        &self,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> RuntimePageTaskSender {
        let routes = self.page_task_producer_routes.borrow();
        let routes = routes
            .as_ref()
            .expect("standalone ScriptVm tests must use bound production Page task routes");
        self.owner_attached_runtime_page_task_sender(
            routes.main_document_runtime_sender(root_document),
            routes.main_parser_continuation_sender(root_document),
            routes.stylesheet_task_sender(root_document),
            routes.service_worker_task_sender(root_document),
        )
    }

    pub(crate) fn owner_attached_runtime_page_task_sender(
        &self,
        main_document_runtime: RendererPageMainDocumentRuntimeSender,
        main_parser_continuation: RendererPageMainParserContinuationSender,
        stylesheet: RendererPageStylesheetTaskSender,
        service_worker: RendererPageServiceWorkerTaskSender,
    ) -> RuntimePageTaskSender {
        RuntimePageTaskSender::owner_attached(
            self.wake.clone(),
            self.owner_wake.clone(),
            main_document_runtime,
            main_parser_continuation,
            stylesheet,
            service_worker,
            #[cfg(test)]
            self.v8_foreground_task_sender(),
        )
    }

    pub(crate) fn page_runtime_wake_sender(&self) -> PageRuntimeWakeSender {
        PageRuntimeWakeSender::new(self.wake.clone()).with_owner_wake(self.owner_wake.clone())
    }

    pub(crate) async fn wait(&self) {
        self.wake.wait().await;
    }

    pub(crate) fn clear(&self) {
        self.wake.clear();
    }
}

/// Explicit residence for low-level tests that need a production Page route
/// and its unique consumer without constructing an owner-local Page slot.
///
/// The consumer deliberately lives here rather than in [`PageRuntimeTaskSource`].
/// That keeps the production carrier identical in test and non-test builds and
/// makes the fixture responsible for retaining both halves of the route.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct RendererPageTaskTestResidence {
    // Standalone PageVm tests have no owner runtime. Keep that fixture-only
    // runtime beside the test task residence instead of leaking it into
    // ScriptVm/PageVm production state.
    standalone_runtime: Option<Arc<tokio::runtime::Runtime>>,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    runtime_source: PageRuntimeTaskSource,
    task_sources: RendererPageOwnedTaskSourcesTestHarness,
    root_document: crate::runtime::RendererDocumentToken,
    owner_wake_rx: Option<
        Rc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<super::RendererOwnerWake>>>,
    >,
}

#[cfg(test)]
impl std::fmt::Debug for RendererPageTaskTestResidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererPageTaskTestResidence")
            .field("runtime_source", &self.runtime_source)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl RendererPageTaskTestResidence {
    pub(crate) fn new(owner_wake: Option<RendererOwnerWakeSender>) -> Self {
        let (standalone_runtime, resource_task_runner) =
            match crate::network::RendererResourceTaskRunner::from_current_tokio() {
                Ok(task_runner) => (None, task_runner),
                Err(_) => {
                    let runtime = Arc::new(
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .expect("standalone Page task fixture runtime should build"),
                    );
                    let task_runner = crate::network::RendererResourceTaskRunner::from_tokio_handle(
                        runtime.handle().clone(),
                    );
                    (Some(runtime), task_runner)
                }
            };
        let (owner_wake, owner_wake_rx) = match owner_wake {
            Some(owner_wake) => (Some(owner_wake), None),
            None => {
                let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
                let owner_wake = RendererOwnerWakeSender::new(
                    wake_tx,
                    crate::runtime::RendererPageToken::new_for_testing(
                        crate::PageId::new_for_testing(u64::MAX),
                    ),
                );
                (
                    Some(owner_wake),
                    Some(Rc::new(tokio::sync::Mutex::new(wake_rx))),
                )
            }
        };
        let runtime_source = PageRuntimeTaskSource::new(owner_wake);
        let (owned_sources, routes) = RendererPageOwnedTaskSources::new(
            runtime_source.wake.clone(),
            runtime_source.page_source_owner_wake_for_test(),
        );
        runtime_source
            .bind_page_task_producer_routes(routes)
            .expect("fresh test Page residence must accept its producer routes");
        let root_document = crate::runtime::RendererDocumentToken::new_for_testing(
            crate::PageId::new_for_testing(runtime_source.identity_key() as u64),
            1,
        );
        Self {
            standalone_runtime,
            resource_task_runner,
            runtime_source,
            task_sources: RendererPageOwnedTaskSourcesTestHarness::new(owned_sources),
            root_document,
            owner_wake_rx,
        }
    }

    pub(crate) fn with_owner_runtime<T>(&self, operation: impl FnOnce() -> T) -> T {
        if let Some(runtime) = self.standalone_runtime.as_ref() {
            let _guard = runtime.enter();
            operation()
        } else {
            operation()
        }
    }

    pub(crate) fn resource_task_runner(&self) -> crate::network::RendererResourceTaskRunner {
        self.resource_task_runner.clone()
    }

    pub(crate) fn runtime_source(&self) -> PageRuntimeTaskSource {
        self.runtime_source.clone()
    }

    pub(crate) fn task_sources(&self) -> RendererPageOwnedTaskSourcesTestHarness {
        self.task_sources.clone()
    }

    pub(crate) const fn root_document(&self) -> crate::runtime::RendererDocumentToken {
        self.root_document
    }

    pub(crate) async fn wait_for_owner_task_arrival(&self) -> bool {
        let Some(owner_wake_rx) = self.owner_wake_rx.as_ref() else {
            return false;
        };
        owner_wake_rx.lock().await.recv().await.is_some()
    }

    pub(crate) fn owner_attached_runtime_page_task_sender(&self) -> RuntimePageTaskSender {
        self.runtime_source
            .owner_attached_runtime_page_task_sender_for_test(self.root_document)
    }

    pub(crate) fn service_worker_task_sender_for_root(
        &self,
        root_document: crate::runtime::RendererDocumentToken,
    ) -> RendererPageServiceWorkerTaskSender {
        let senders = self
            .runtime_source
            .bound_task_producer_senders(root_document)
            .expect("test Page residence must retain its producer routes");
        let (_, _, _, _, _, service_worker) = senders.into_parts();
        service_worker
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PageRuntimeWakeSender {
    wake: PageRuntimeWakeSignal,
    owner_wake: Option<RendererOwnerWakeSender>,
    main_document_runtime: Option<RendererPageMainDocumentRuntimeSender>,
}

impl PageRuntimeWakeSender {
    fn new(wake: PageRuntimeWakeSignal) -> Self {
        Self {
            wake,
            owner_wake: None,
            main_document_runtime: None,
        }
    }

    fn with_owner_wake(mut self, owner_wake: Option<RendererOwnerWakeSender>) -> Self {
        self.owner_wake = owner_wake;
        self
    }

    fn with_main_document_runtime(
        mut self,
        sender: Option<RendererPageMainDocumentRuntimeSender>,
    ) -> Self {
        self.main_document_runtime = sender;
        self
    }

    pub(crate) fn has_main_document_runtime_route(&self) -> bool {
        self.main_document_runtime.is_some()
    }

    #[cfg(test)]
    pub(crate) fn send_wake(&self) -> Result<(), std::convert::Infallible> {
        self.wake.send();
        Ok(())
    }

    pub(crate) fn send_document_lifecycle_wake(&self) -> Result<(), std::convert::Infallible> {
        self.wake.send();
        if let Some(owner_wake) = &self.owner_wake {
            owner_wake.signal_document_lifecycle_turn();
        }
        Ok(())
    }

    pub(crate) fn bind_main_document_runtime_continuation(
        &self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> MainDocumentRuntimeContinuationSender {
        match &self.main_document_runtime {
            Some(sender) => MainDocumentRuntimeContinuationSender::OwnerAttached(
                sender.bind_producer(document_owner),
            ),
            None => panic!("owner-attached Page runtime continuation sender is required"),
        }
    }
}

/// Narrow ScriptVm-to-owner capability for one exact top-level navigation.
/// It intentionally carries neither generic Page wake authority nor a main
/// Document runtime task route.
#[derive(Debug, Clone)]
pub(crate) struct RendererTopLevelNavigationHandoffSender {
    wake: PageRuntimeWakeSignal,
    owner_wake: Option<RendererOwnerWakeSender>,
}

impl RendererTopLevelNavigationHandoffSender {
    fn new(wake: PageRuntimeWakeSignal, owner_wake: Option<RendererOwnerWakeSender>) -> Self {
        Self { wake, owner_wake }
    }

    pub(crate) fn next_handoff(&self) -> super::RendererTopLevelNavigationHandoff {
        self.wake.next_top_level_navigation_handoff()
    }

    /// Hand one exact request directly to the Page owner without manufacturing
    /// a Page scheduler task; the descriptor already occupies the ScriptVm's
    /// unique pending-navigation slot.
    pub(crate) fn send(&self, handoff: super::RendererTopLevelNavigationHandoff) -> bool {
        self.owner_wake
            .as_ref()
            .is_some_and(|owner_wake| owner_wake.signal_top_level_navigation_handoff(handoff))
    }
}

#[derive(Clone, Debug)]
pub(crate) enum MainDocumentRuntimeContinuationSender {
    OwnerAttached(RendererPageMainDocumentRuntimeProducer),
}

impl MainDocumentRuntimeContinuationSender {
    pub(crate) fn send_runtime_script_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        match self {
            Self::OwnerAttached(sender) => sender.send_runtime_script_continuation(),
        }
    }

    pub(crate) fn send_runtime_module_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        match self {
            Self::OwnerAttached(sender) => sender.send_runtime_module_continuation(),
        }
    }

    pub(crate) fn send_parser_owned_module_continuation(
        &self,
    ) -> Result<(), RendererPageMainDocumentRuntimeRouteClosed> {
        match self {
            Self::OwnerAttached(sender) => sender.send_parser_owned_module_continuation(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PageTaskSender {
    main_document_runtime: RendererPageMainDocumentRuntimeSender,
}

impl PageTaskSender {
    fn owner_attached(main_document_runtime: RendererPageMainDocumentRuntimeSender) -> Self {
        Self {
            main_document_runtime,
        }
    }

    pub(crate) fn has_main_document_runtime_route(&self) -> bool {
        true
    }

    pub(crate) fn bind_main_document_runtime_producer(
        &self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> RendererPageMainDocumentRuntimeProducer {
        self.main_document_runtime.bind_producer(document_owner)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimePageTaskSender {
    wake: PageRuntimeWakeSignal,
    top_level_navigation_owner_wake: Option<RendererOwnerWakeSender>,
    main_document_runtime: RendererPageMainDocumentRuntimeSender,
    main_parser_continuation: RendererPageMainParserContinuationSender,
    stylesheet: RendererPageStylesheetTaskSender,
    service_worker: RendererPageServiceWorkerTaskSender,
    #[cfg(test)]
    v8_foreground_task_sender: Option<RendererPageV8ForegroundTaskSender>,
}

impl RuntimePageTaskSender {
    fn owner_attached(
        wake: PageRuntimeWakeSignal,
        top_level_navigation_owner_wake: Option<RendererOwnerWakeSender>,
        main_document_runtime: RendererPageMainDocumentRuntimeSender,
        main_parser_continuation: RendererPageMainParserContinuationSender,
        stylesheet: RendererPageStylesheetTaskSender,
        service_worker: RendererPageServiceWorkerTaskSender,
        #[cfg(test)] v8_foreground_task_sender: Option<RendererPageV8ForegroundTaskSender>,
    ) -> Self {
        Self {
            wake,
            top_level_navigation_owner_wake,
            main_document_runtime,
            main_parser_continuation,
            stylesheet,
            service_worker,
            #[cfg(test)]
            v8_foreground_task_sender,
        }
    }

    pub(crate) fn page_task_sender(&self) -> PageTaskSender {
        PageTaskSender::owner_attached(self.main_document_runtime.clone())
    }

    pub(crate) fn page_runtime_wake_sender(&self) -> PageRuntimeWakeSender {
        PageRuntimeWakeSender::new(self.wake.clone())
            .with_main_document_runtime(Some(self.main_document_runtime.clone()))
    }

    pub(crate) fn top_level_navigation_handoff_sender(
        &self,
    ) -> RendererTopLevelNavigationHandoffSender {
        RendererTopLevelNavigationHandoffSender::new(
            self.wake.clone(),
            self.top_level_navigation_owner_wake.clone(),
        )
    }

    pub(crate) fn service_worker_task_sender(&self) -> RendererPageServiceWorkerTaskSender {
        self.service_worker.clone()
    }

    pub(crate) fn main_parser_continuation_sender(
        &self,
    ) -> RendererPageMainParserContinuationSender {
        self.main_parser_continuation.clone()
    }

    /// Page-scoped stylesheet task routes required by every live Document.
    ///
    /// This capability is installed with the other owner-attached runtime
    /// routes. A ScriptVm therefore cannot be constructed in a "local
    /// stylesheet completion queue" mode that production never uses.
    pub(crate) fn stylesheet_task_sender(&self) -> RendererPageStylesheetTaskSender {
        self.stylesheet.clone()
    }

    #[cfg(test)]
    pub(crate) fn v8_foreground_task_sender(&self) -> RendererPageV8ForegroundTaskSender {
        self.v8_foreground_task_sender
            .clone()
            .expect("Page isolate bootstrap requires a bound V8 foreground-task source")
    }
}

#[cfg(test)]
mod navigation_handoff_tests {
    use super::*;

    #[test]
    fn replacement_runtime_senders_share_page_lifetime_navigation_identity() {
        let wake = PageRuntimeWakeSignal::default();
        let first_document = RendererTopLevelNavigationHandoffSender::new(wake.clone(), None);
        let replacement_document = RendererTopLevelNavigationHandoffSender::new(wake, None);

        let first = first_document.next_handoff();
        let second = replacement_document.next_handoff();

        assert_ne!(
            first, second,
            "cross-Document ScriptVm replacement must not restart navigation handoff identity"
        );
    }

    #[test]
    fn navigation_handoff_sender_publishes_typed_owner_wake() {
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token =
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(17));
        let sender = RendererTopLevelNavigationHandoffSender::new(
            PageRuntimeWakeSignal::default(),
            Some(RendererOwnerWakeSender::new(wake_tx, token)),
        );
        let handoff = sender.next_handoff();

        assert!(sender.send(handoff));
        assert!(matches!(
            wake_rx.try_recv(),
            Ok(crate::page_task_queue::RendererOwnerWake::TopLevelNavigationHandoff {
                token: actual_token,
                handoff: actual_handoff,
            }) if actual_token == token && actual_handoff == handoff
        ));
    }
}
