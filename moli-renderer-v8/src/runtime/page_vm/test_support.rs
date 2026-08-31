use std::{
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use url::Url;

use crate::{
    PageId,
    dom::native::DomHost,
    frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction,
    local_executor::JsLocalExecutor,
    network::ResourceRequestClient,
    page_task_queue::{
        RendererOwnerWake, RendererOwnerWakeSender, RendererPageModulepreloadStartTestSource,
        RendererPageResourceCompletionTestSource,
    },
    parser::HtmlParser,
    script_vm::ScriptVm,
};

use super::{PageSelectedTaskTestSelector, PageVm, PageVmEnvConfig, PageVmRuntimeHooks};

static NEXT_TEST_PAGE_ID: AtomicU64 = AtomicU64::new(1);

impl PageVm {
    /// Keep the owner of a standalone request client alive for exactly this
    /// low-level PageVm fixture.
    ///
    /// Production PageVms obtain the same residence from their BrowserContext
    /// owner root. This hook exists only for test factories that return a bare
    /// PageVm after cloning a handle from a locally constructed client owner.
    pub(crate) fn retain_standalone_request_client_owner_for_test(
        &mut self,
        owner: crate::network::ResourceRequestClientOwner,
    ) {
        assert!(
            self.runtime_hooks
                ._standalone_request_client_owner
                .is_none(),
            "a standalone PageVm fixture must retain only one request-client owner"
        );
        self.runtime_hooks._standalone_request_client_owner = Some(std::rc::Rc::new(owner));
    }

    /// Claim the oldest eligible stable task and execute it through the same
    /// selected-task dispatcher used by the production owner loop.
    ///
    /// This helper supplies the named owner-local scope but deliberately does
    /// not model `PageTurnScheduler` admission or fairness. Tests that need
    /// those contracts must use the owner-loop integration harness.
    pub(crate) async fn run_one_oldest_ready_page_task_on_owner_lane_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        let sources = self.page_task_executor_sources_for_test();
        let task = if let Some(task) =
            sources.take_oldest_scheduler_task_for_executor_test(|descriptor| {
                self.page_ready_descriptor_is_eligible(descriptor)
            }) {
            task
        } else if let Some(crate::page_task_queue::RendererPageReadyDescriptor::Timer {
            deadline,
            selection,
        }) = self.due_page_timer_ready_descriptor(
            crate::page_task_queue::RendererPageTimerSelection::AnyReady,
        ) {
            crate::page_task_queue::RendererPageSchedulerTask::Timer {
                deadline,
                selection,
            }
        } else {
            return Ok(false);
        };
        self.apply_selected_page_scheduler_task_on_owner_lane_for_test(task, loader.clone())
            .await?;
        Ok(true)
    }
}

/// Minimal PageVm fixture for WebAPI tests that need real Page task routes.
///
/// It executes production typed tasks and lane executors, but deliberately has
/// no owner-local Page slot. It therefore cannot prove scheduler admission,
/// liveness, cross-source ordering, or fairness; those contracts belong to the
/// owner-loop integration harness.
pub(crate) struct PageVmTaskExecutorTestHarness {
    page_vm: PageVm,
    page_resource_completion_source: RendererPageResourceCompletionTestSource,
    owner_wake_rx: tokio::sync::mpsc::UnboundedReceiver<RendererOwnerWake>,
    selected_task_local_set: tokio::task::LocalSet,
    _browser_context_owner: Option<crate::runtime::RendererBrowserContextRuntimeOwner>,
}

impl PageVmTaskExecutorTestHarness {
    pub(crate) fn has_ready_dom_manipulation_family_for_test(
        &self,
        family: super::PageDomManipulationTestFamily,
    ) -> bool {
        self.page_vm
            .has_ready_dom_manipulation_family_for_test(family)
    }

    pub(crate) fn new(document_url: Url, loader: &ResourceRequestClient) -> Self {
        let owner = crate::runtime::RendererBrowserContextRuntime::new();
        let mut harness =
            Self::new_with_browser_context_runtime(document_url, loader, owner.handle());
        harness._browser_context_owner = Some(owner);
        harness
    }

    pub(crate) fn new_with_browser_context_runtime(
        document_url: Url,
        loader: &ResourceRequestClient,
        browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    ) -> Self {
        let dom_host = DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
            document_url,
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        ));
        Self::new_with_dom_host_and_browser_context_runtime(
            dom_host,
            loader,
            browser_context_runtime,
        )
    }

    /// Build the same task-executor fixture around an already parsed or
    /// streaming parser `DomHost`.
    ///
    /// This keeps parser-focused tests on their exact DOM input while still
    /// installing the production Page-owned task routes and selected-task
    /// dispatcher. It must not be used as evidence for owner-slot admission or
    /// scheduler fairness, which this fixture deliberately does not model.
    pub(crate) fn new_with_dom_host(dom_host: DomHost, loader: &ResourceRequestClient) -> Self {
        let owner = crate::runtime::RendererBrowserContextRuntime::new();
        let mut harness =
            Self::new_with_dom_host_and_browser_context_runtime(dom_host, loader, owner.handle());
        harness._browser_context_owner = Some(owner);
        harness
    }

    fn new_with_dom_host_and_browser_context_runtime(
        dom_host: DomHost,
        loader: &ResourceRequestClient,
        browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
    ) -> Self {
        let _js_runtime = crate::JsRuntime::initialize();
        let page_id = PageId::new_for_testing(NEXT_TEST_PAGE_ID.fetch_add(1, Ordering::Relaxed));
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(page_id),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_and_browser_context_without_owner_reservation_for_test(
                owner_wake,
                browser_context_runtime,
            );
        let page_vm = PageVm::new(
            page_id,
            JsLocalExecutor::new(),
            loader,
            &minimal_test_page_vm_env_config(),
            runtime_hooks,
            dom_host,
            Instant::now(),
        )
        .expect("PageVm task-executor test fixture should initialize");
        let page_resource_completion_source = page_vm.page_resource_completion_queue();
        Self {
            page_vm,
            page_resource_completion_source,
            owner_wake_rx,
            selected_task_local_set: tokio::task::LocalSet::new(),
            _browser_context_owner: None,
        }
    }

    /// Execute the oldest eligible stable Page task through the same selected
    /// task dispatcher used by the production owner-loop.
    ///
    /// The fixture has no PageTurnScheduler, so this preserves enqueue order
    /// but does not claim to cover fairness or owner admission. Crucially, it
    /// does exercise every family's production exact-owner authorization and
    /// terminal application instead of duplicating those rules in tests.
    pub(crate) async fn run_one_oldest_ready_page_task_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(
                self.page_vm
                    .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader),
            )
            .await
    }

    /// Execute one exact Networking resource terminal through the production
    /// selected-task dispatcher and its unique task-end coordinator.
    ///
    /// Tests that only inspect the resource owner's body effect may use
    /// `apply_one_page_resource_terminal_owner_admission`. Complete Page
    /// workflows must use this method so event-dispatch checkpoints and
    /// post-checkpoint follow-ups cannot drift from production.
    pub(crate) async fn run_one_page_resource_completion_selected_task_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ResourceCompletion,
                loader,
            ))
            .await
    }

    /// Advance Page timers only through the production selected-task
    /// dispatcher.
    ///
    /// `PageVmTaskExecutorTestHarness` dereferences to `ScriptVm` for
    /// synchronous domain setup. Without this explicit proxy, method
    /// resolution would select the standalone ScriptVm timer helper and
    /// silently bypass Page task completion.
    pub(crate) async fn advance_timers_until_deadline_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<()> {
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_millis(3_200))
            .unwrap_or_else(std::time::Instant::now);
        for _ in 0..10_000 {
            if let Some(crate::page_task_queue::RendererPageReadyDescriptor::Timer {
                deadline,
                selection,
            }) = self.page_vm.due_page_timer_ready_descriptor(
                crate::page_task_queue::RendererPageTimerSelection::AnyReady,
            ) {
                self.selected_task_local_set
                    .run_until(
                        self.page_vm
                            .apply_selected_page_scheduler_task_on_owner_lane_for_test(
                                crate::page_task_queue::RendererPageSchedulerTask::Timer {
                                    deadline,
                                    selection,
                                },
                                loader.clone(),
                            ),
                    )
                    .await?;
                continue;
            }
            let Some(ms_to_next) = self.page_vm.vm().ms_to_next_timeout() else {
                return Ok(());
            };
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            let sleep_for = std::time::Duration::from_millis(ms_to_next)
                .min(deadline.saturating_duration_since(now));
            if sleep_for.is_zero() {
                continue;
            }
            tokio::time::sleep(sleep_for).await;
        }
        anyhow::bail!("Page timer test executor exceeded its bounded 10,000-turn budget")
    }

    /// Drain only tasks that are already resident in the production Page
    /// sources.
    ///
    /// This is setup support for semantic tests, not ordering evidence: tests
    /// that assert a family boundary must use an exact selector instead. The
    /// bound prevents a reentrant producer from turning fixture setup into an
    /// unbounded generic wait driver.
    pub(crate) async fn drain_ready_page_task_executor_turns_for_setup(
        &mut self,
        loader: &ResourceRequestClient,
        max_tasks: usize,
    ) -> anyhow::Result<usize> {
        let mut completed = 0;
        while completed < max_tasks
            && self
                .run_one_oldest_ready_page_task_executor_turn(loader)
                .await?
        {
            completed += 1;
        }
        Ok(completed)
    }

    pub(crate) async fn run_one_dom_manipulation_task_executor_turn(
        &mut self,
        family: super::PageDomManipulationTestFamily,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::DomManipulation(family),
                loader,
            ))
            .await
    }

    pub(crate) async fn run_one_media_element_event_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::MediaElementEvent,
                loader,
            ))
            .await
    }

    pub(crate) async fn run_one_rendering_update_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,
                loader,
            ))
            .await
    }

    /// Return a producer bound to the fixture's current root Document.
    ///
    /// Complete ServiceWorker callback workflows must publish through this
    /// typed route and then execute `run_one_service_worker_internal_task_*`.
    /// Low-level body tests use the dedicated body support instead.
    pub(crate) fn current_service_worker_task_sender_for_test(
        &self,
    ) -> crate::page_task_queue::RendererPageServiceWorkerTaskSender {
        let root_document = self.page_vm.document_lifecycle.identity().document;
        self.page_vm
            .service_worker_task_sender_for_root_for_test(root_document)
    }

    /// Execute one ServiceWorker internal-default task through the production
    /// exact-root arbiter and selected-task completion coordinator.
    pub(crate) async fn run_one_service_worker_internal_task_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ServiceWorkerInternal,
                loader,
            ))
            .await
    }

    /// Execute one exact child-frame semantic family through the production
    /// selected-task dispatcher.
    ///
    /// Unlike `run_next_child_frame_semantic_turn`, this helper does not
    /// consume an earlier family as test setup. It is therefore suitable for
    /// assertions that a particular child task is or is not currently
    /// runnable.
    pub(crate) async fn run_one_child_frame_task_executor_turn(
        &mut self,
        turn: crate::frame_owner_model::ChildFrameSemanticTurnKind,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        let selector = match turn {
            ChildFrameSemanticTurnKind::RealmMaterialization => {
                PageSelectedTaskTestSelector::ChildRealmMaterialization
            }
            ChildFrameSemanticTurnKind::NavigationCommit => {
                PageSelectedTaskTestSelector::ChildNavigationCommit
            }
            ChildFrameSemanticTurnKind::DocumentLifecycle => {
                PageSelectedTaskTestSelector::ChildDocumentLifecycle
            }
            ChildFrameSemanticTurnKind::DocumentScriptReady => {
                PageSelectedTaskTestSelector::ChildDocumentScriptReady
            }
            ChildFrameSemanticTurnKind::HostLoad => PageSelectedTaskTestSelector::ChildHostLoad,
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad => {
                PageSelectedTaskTestSelector::ChildClassicScriptSourceLoad
            }
            ChildFrameSemanticTurnKind::ParserModuleRootStart => {
                PageSelectedTaskTestSelector::ChildParserModuleRootStart
            }
        };
        self.selected_task_local_set
            .run_until(
                self.page_vm
                    .run_exact_selected_page_task_for_test(selector, loader),
            )
            .await
    }

    pub(crate) async fn run_one_child_module_script_terminal_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildModuleScriptTerminal,
                loader,
            ))
            .await
    }

    /// Drain ready child-frame semantic prerequisites without consuming work
    /// from another Page task family.
    ///
    /// This setup helper exists for tests that intentionally queue a
    /// WindowMessage before a child navigation commits. The production owner
    /// scheduler gives the child transition its own arbitration; the
    /// standalone harness only has oldest-ready selection, so using its
    /// generic setup drain would execute the message against the initial
    /// empty LocalWindow. Every consumed task still runs through the
    /// production selected-task dispatcher and completion coordinator.
    pub(crate) async fn drain_ready_child_frame_task_executor_turns_for_setup(
        &mut self,
        loader: &ResourceRequestClient,
        max_tasks: usize,
    ) -> anyhow::Result<usize> {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        const CHILD_SETUP_ORDER: [ChildFrameSemanticTurnKind; 7] = [
            ChildFrameSemanticTurnKind::RealmMaterialization,
            ChildFrameSemanticTurnKind::NavigationCommit,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            ChildFrameSemanticTurnKind::HostLoad,
            ChildFrameSemanticTurnKind::ParserModuleRootStart,
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
        ];

        let mut completed = 0;
        while completed < max_tasks {
            let mut progressed = false;
            for turn in CHILD_SETUP_ORDER {
                if self
                    .run_one_child_frame_task_executor_turn(turn, loader)
                    .await?
                {
                    completed += 1;
                    progressed = true;
                    break;
                }
            }
            if !progressed {
                break;
            }
        }
        Ok(completed)
    }

    pub(crate) async fn run_one_text_track_networking_task_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::TextTrackNetworking,
                loader,
            ))
            .await
    }

    pub(crate) async fn run_one_broadcast_channel_delivery_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.run_one_dom_manipulation_task_executor_turn(
            super::PageDomManipulationTestFamily::BroadcastChannel,
            loader,
        )
        .await
    }

    pub(crate) async fn run_one_webcrypto_task_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::WebCryptoTask,
                loader,
            ))
            .await
    }

    pub(crate) fn run_one_child_realm_materialization_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<crate::page_task_queue::PageChildRealmMaterializationTurnOutcome>>
    {
        self.page_vm.run_child_realm_materialization_body_for_test()
    }

    pub(crate) fn run_one_child_navigation_commit_body_for_test(
        &mut self,
    ) -> anyhow::Result<Option<crate::page_task_queue::PageChildNavigationCommitTurnOutcome>> {
        self.page_vm.run_child_navigation_commit_body_for_test()
    }

    pub(crate) async fn wait_for_v8_foreground_task_arrival(&mut self) -> bool {
        while let Some(wake) = self.owner_wake_rx.recv().await {
            if wake.source_for_test()
                == crate::page_task_queue::RendererOwnerWakeSource::V8ForegroundTask
            {
                return true;
            }
        }
        false
    }

    /// Advance exactly one production child semantic action. Realm
    /// materialization is returned as its own visible turn; callers must not
    /// assume that test setup consumes it implicitly.
    pub(crate) async fn run_next_child_frame_semantic_turn(
        &mut self,
    ) -> Option<crate::frame_owner_model::ChildFrameSemanticTurnKind> {
        self.selected_task_local_set
            .run_until(
                self.page_vm
                    .run_next_child_frame_task_source_for_semantic_test(),
            )
            .await
    }

    pub(crate) async fn run_one_window_message_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::WindowMessage,
                loader,
            ))
            .await
    }

    pub(crate) async fn run_one_history_traversal_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::HistoryTraversal,
                loader,
            ))
            .await
    }

    pub(crate) async fn run_one_user_interaction_executor_turn(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> anyhow::Result<bool> {
        self.selected_task_local_set
            .run_until(self.page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::UserInteraction,
                loader,
            ))
            .await
    }

    pub(crate) async fn apply_pending_broadcast_channel_delivery_tasks(
        &mut self,
        loader: &ResourceRequestClient,
        max_tasks: usize,
    ) -> anyhow::Result<usize> {
        let mut completed = 0;
        while self
            .run_one_broadcast_channel_delivery_executor_turn(loader)
            .await?
        {
            completed += 1;
            assert!(
                completed <= max_tasks,
                "BroadcastChannel executor fixture should settle within {max_tasks} tasks"
            );
        }
        Ok(completed)
    }

    /// Apply one terminal only when the test observes the boundary between
    /// network settlement and a later typed Page successor.
    ///
    /// This is not a Page-task executor. Complete behavior tests must use
    /// `run_one_page_resource_completion_selected_task_executor_turn`.
    pub(crate) fn apply_one_page_resource_terminal_owner_admission(
        &mut self,
    ) -> anyhow::Result<bool> {
        self.page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(
                &mut self.page_resource_completion_source,
            )
            .map(|outcome| outcome.is_some())
    }

    pub(crate) fn modulepreload_start_test_source(
        &self,
    ) -> RendererPageModulepreloadStartTestSource {
        self.page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
    }

    /// Take one task produced through the real stable Page route for a narrow
    /// routing-shape test.
    ///
    /// Some low-level native-module tests deliberately carry V8 globals from a
    /// synthetic isolate and therefore must not execute the payload. Production
    /// application and owner-loop liveness are covered by PageVm and owner
    /// integration tests using isolate-local payloads.
    pub(crate) fn take_dynamic_import_owner_action_for_routing_test(
        &mut self,
    ) -> Option<FrameDocumentDynamicImportTerminalPreparedAction> {
        self.page_vm
            .page_task_executor_sources_for_test()
            .dynamic_import_owner_action()
            .pop_front()
            .map(|(_, task)| task.into_action())
    }

    /// Wait for either the harness owner wake or the Page's initial runtime
    /// wake. A wake only asks the fixture to re-check stable sources; it never
    /// authorizes terminal admission or task execution.
    pub(crate) async fn wait_for_task_executor_work_arrival(&mut self) -> bool {
        tokio::select! {
            wake = self.owner_wake_rx.recv() => wake.is_some(),
            arrived = self.page_vm.wait_for_initial_page_runtime_wake() => arrived,
        }
    }
}

impl Deref for PageVmTaskExecutorTestHarness {
    type Target = ScriptVm;

    fn deref(&self) -> &Self::Target {
        self.page_vm.vm()
    }
}

impl DerefMut for PageVmTaskExecutorTestHarness {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.page_vm.vm_mut()
    }
}

fn minimal_test_page_vm_env_config() -> PageVmEnvConfig {
    PageVmEnvConfig {
        web_storage: crate::RendererWebStorageHandles::ephemeral(),
        root_frame_id: None,
        main_document_commit: None,
        top_level_storage_key: None,
        document_start_scripts: Vec::new(),
        runtime_bindings: Vec::new(),
        runtime_inspector_session_restore_snapshots: Vec::new(),
        runtime_isolated_worlds: Vec::new(),
        permission_overrides: Vec::new(),
        extra_http_headers: Vec::new(),
        navigator_identity: Default::default(),
        document_policy_container: Default::default(),
        document_default_language: None,
        document_last_modified: None,
        locale_override: None,
        timezone_override: None,
        script_execution_disabled: false,
        bypass_content_security_policy: false,
        cpu_throttling_rate: 1.0,
        emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
        idle_override: None,
        viewport_surface: None,
        network_offline: false,
        blocked_url_patterns: Vec::new(),
        indexed_db_manager: None,
        storage_bucket_store: None,
        fetch_subresource_interception_enabled: false,
        fetch_subresource_interception_resource_type: None,
        layout_policy: crate::real_layout_test_policy(),
        wpt_extensions_enabled: false,
        navigation_bootstrap_entry: None,
        reserved_service_worker_client_id: None,
    }
}
