use super::*;

impl ScriptVm {
    pub(crate) fn resync_child_browsing_contexts(&mut self) {
        child_host_load::ChildHostLoadOwner::new(self).resync_child_browsing_contexts();
    }

    #[cfg(test)]
    pub(crate) async fn run_child_frame_task_source_once_for_test(
        &mut self,
        turn: impl Into<crate::frame_owner_model::ChildFrameSemanticTurnKind>,
    ) -> bool {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        match turn.into() {
            ChildFrameSemanticTurnKind::RealmMaterialization => self
                .run_child_realm_materialization_body_for_test()
                .expect("typed child realm-materialization executor turn should succeed"),
            ChildFrameSemanticTurnKind::DocumentScriptReady => self
                .run_child_document_script_ready_body_for_test()
                .await
                .expect("typed child DocumentScriptReady executor turn should succeed")
                .is_some_and(ChildDocumentScriptReadyRunOutcome::made_progress),
            ChildFrameSemanticTurnKind::NavigationCommit => self
                .run_next_child_navigation_commit_body_for_test()
                .expect("typed child navigation-commit body should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::DocumentLifecycle => self
                .run_child_document_lifecycle_body_for_test()
                .expect("typed child lifecycle executor turn should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::HostLoad => self
                .run_child_host_load_body_for_test()
                .expect("typed child HostLoad executor turn should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad => self
                .run_child_classic_source_load_body_for_test()
                .expect("typed child classic source-load body should succeed")
                .is_some(),
            ChildFrameSemanticTurnKind::ParserModuleRootStart => self
                .run_child_parser_module_root_start_body_for_test()
                .expect("typed child parser module root body should succeed")
                .is_some(),
        }
    }

    #[cfg(test)]
    pub(crate) fn has_ready_child_frame_semantic_turn_for_test(
        &self,
        expected: crate::frame_owner_model::ChildFrameSemanticTurnKind,
    ) -> bool {
        use crate::{
            frame_owner_model::ChildFrameSemanticTurnKind,
            page_task_queue::{
                RendererPageChildFrameTaskTarget, RendererPageDomManipulationOwner,
                RendererPageReadyDescriptor,
            },
        };

        if expected == ChildFrameSemanticTurnKind::HostLoad {
            return self
                ._page_task_residence_for_executor_test
                .as_ref()
                .expect("semantic fixture must retain its sources")
                .task_sources()
                .has_scheduler_task_for_executor_test(|descriptor| {
                    matches!(
                        descriptor,
                        RendererPageReadyDescriptor::DomManipulation {
                            owner: RendererPageDomManipulationOwner::ChildHostLoad(_),
                            ..
                        }
                    )
                });
        }

        if expected == ChildFrameSemanticTurnKind::DocumentLifecycle {
            return self._page_task_residence_for_executor_test.as_ref().expect("semantic fixture must retain its sources").task_sources().has_scheduler_task_for_executor_test(|descriptor| {
                matches!(descriptor,
                    RendererPageReadyDescriptor::DomManipulation { owner: RendererPageDomManipulationOwner::ChildDocumentLifecycle(_), .. }
                ) || matches!(descriptor,
                    RendererPageReadyDescriptor::ChildFrameTask { owner, .. }
                        if matches!(owner.target(), RendererPageChildFrameTaskTarget::DocumentLifecycle(_))
                )
            });
        }

        let Some(target) = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .and_then(|residence| residence.task_sources().next_child_frame_task_target())
        else {
            return false;
        };
        matches!(
            (expected, target),
            (
                ChildFrameSemanticTurnKind::RealmMaterialization,
                RendererPageChildFrameTaskTarget::RealmMaterialization(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                RendererPageChildFrameTaskTarget::DocumentScriptReady(_)
            ) | (
                ChildFrameSemanticTurnKind::HostLoad,
                RendererPageChildFrameTaskTarget::HostLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
                RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ParserModuleRootStart,
                RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
            )
        )
    }

    /// Run one child semantic executor turn from the production stable
    /// residences used by low-level ScriptVm fixtures.
    #[cfg(test)]
    pub(crate) async fn run_next_child_frame_semantic_turn_for_test(
        &mut self,
    ) -> Option<crate::frame_owner_model::ChildFrameSemanticTurnKind> {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        if self
            .run_child_realm_materialization_body_for_test()
            .expect("child realm materialization prerequisite should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::RealmMaterialization);
        }
        if self
            .run_next_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::NavigationCommit);
        }
        if matches!(
            self._page_task_residence_for_executor_test
                .as_ref()
                .expect("child fixture must retain its sources")
                .task_sources()
                .next_child_semantic_task_target(),
            Some(crate::page_task_queue::RendererPageChildFrameTaskTarget::DocumentLifecycle(_))
        ) && self
            .run_child_document_lifecycle_body_for_test()
            .expect("typed child lifecycle executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::DocumentLifecycle);
        }
        if self
            .run_child_document_script_ready_body_for_test()
            .await
            .expect("typed child DocumentScriptReady executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::DocumentScriptReady);
        }
        if self
            .run_child_host_load_body_for_test()
            .expect("typed child HostLoad executor turn should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::HostLoad);
        }
        if self
            .run_child_parser_module_root_start_body_for_test()
            .expect("typed child parser module root body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::ParserModuleRootStart);
        }
        if self
            .run_child_classic_source_load_body_for_test()
            .expect("typed child classic source-load body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad);
        }
        None
    }

    pub(super) fn apply_child_parser_module_root_fetch_completion_to_owner(
        &mut self,
        completion: crate::types::ChildParserModuleRootFetchCompletion,
    ) -> crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup {
        child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_parser_root_fetch_completion(completion)
    }

    pub(super) fn apply_child_module_dependency_fetch_completion_to_owner(
        &mut self,
        completion: crate::types::ChildModuleDependencyFetchCompletion,
    ) -> crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup {
        child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_dependency_fetch_completion(completion)
    }

    #[cfg(test)]
    pub(crate) fn drain_pending_child_frame_work_for_test(&mut self) {
        expect_ready_child_frame_owner_source_future_for_test(
            self.drain_pending_child_frame_work_for_test_on_owner_sources(),
        );
    }

    #[cfg(test)]
    pub(super) async fn drain_pending_child_frame_work_for_test_on_owner_sources(&mut self) {
        // Direct ScriptVm semantic tests have no owner loop, so advance the
        // same production sources one turn at a time. Never jump a dependent
        // child action over its realm prerequisite in the stable family FIFO.
        for _ in 0..128 {
            if self
                .run_child_realm_materialization_body_for_test()
                .expect("test child realm owner turn should complete")
            {
                continue;
            }
            if self
                .run_next_child_navigation_commit_body_for_test()
                .expect("test child navigation-commit body should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_document_lifecycle_body_for_test()
                .expect("test child lifecycle owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_document_script_ready_body_for_test()
                .await
                .expect("test child script owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_host_load_body_for_test()
                .expect("test child HostLoad owner turn should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_parser_module_root_start_body_for_test()
                .expect("test child parser-root body should complete")
                .is_some()
            {
                continue;
            }
            if self
                .run_child_classic_source_load_body_for_test()
                .expect("test child classic source body should complete")
                .is_some()
            {
                continue;
            }
            if self.run_child_module_script_terminal_body_for_test() {
                continue;
            }
            return;
        }
        panic!("test child owner-source drain exceeded its finite turn budget");
    }

    pub(crate) fn has_pending_child_document_lifecycle(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_child_document_lifecycle()
    }

    #[cfg(test)]
    pub(crate) fn has_pending_lightweight_popup_document_loads(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_lightweight_popup_document_loads()
    }

    pub(crate) fn has_pending_lightweight_popup_resource_loads(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_lightweight_popup_resource_loads()
    }

    pub(super) fn sync_child_browsing_context_records(&mut self) {
        self.resync_child_browsing_contexts();
        self.apply_pending_child_document_owner_retirements();
        self.prune_stale_child_default_execution_contexts();
    }

    #[cfg(test)]
    pub(crate) fn has_pending_child_navigation_commit_for_test(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_child_navigation_commit_task()
    }

    pub(crate) fn has_pending_location_navigation(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_location_navigation()
    }

    pub(super) fn has_planned_form_navigation_to(
        &self,
        target: crate::native_bridge::OwnerDispatchScope,
    ) -> bool {
        self._context_host
            .borrow()
            .has_planned_form_navigation_to(target)
    }

    pub(super) fn pending_location_navigation_source_document(
        &self,
    ) -> Option<crate::runtime::RendererDocumentLifecycleIdentity> {
        self._context_host
            .borrow()
            .pending_location_navigation_source_document()
    }

    pub(crate) fn pending_location_navigation_kind(
        &self,
    ) -> Option<crate::native_bridge::PendingLocationNavigationKind> {
        self._context_host
            .borrow()
            .pending_location_navigation_kind()
    }

    pub(crate) fn has_pending_javascript_location_navigation(&self) -> bool {
        self.pending_location_navigation_kind()
            == Some(crate::native_bridge::PendingLocationNavigationKind::JavascriptUrl)
    }

    pub(crate) fn pending_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<crate::runtime::RendererRuntimeCommandCausalIdentity> {
        self._context_host
            .borrow()
            .pending_location_navigation_runtime_command_cause()
    }

    pub(crate) fn pending_location_navigation_handoff(
        &self,
    ) -> Option<crate::page_task_queue::RendererTopLevelNavigationHandoff> {
        self._context_host
            .borrow()
            .pending_location_navigation_handoff()
    }

    pub(crate) fn child_browsing_context_frame_tree_snapshot(
        &mut self,
    ) -> Vec<crate::native_bridge::ChildBrowsingContextFrameSnapshot> {
        self._context_host
            .borrow_mut()
            .child_browsing_context_frame_tree_snapshot()
    }

    pub(crate) fn top_document_storage_key_snapshot(&mut self) -> String {
        self._context_host
            .borrow_mut()
            .top_document_storage_context()
            .storage_key()
            .serialized_storage_key()
    }

    pub(crate) fn child_browsing_context_frame_tree_snapshot_for_protocol(
        &mut self,
    ) -> Vec<crate::protocol_types::ChildFrameTreeSnapshot> {
        fn convert(
            snapshot: crate::native_bridge::ChildBrowsingContextFrameSnapshot,
        ) -> crate::protocol_types::ChildFrameTreeSnapshot {
            crate::protocol_types::ChildFrameTreeSnapshot {
                frame_id: snapshot.frame_id,
                loader_id: snapshot.loader_id,
                name: snapshot.name,
                owner_element_id: snapshot.owner_element_id,
                url: snapshot.url,
                storage_key: snapshot.storage_key,
                security_origin_inherited: snapshot.security_origin_inherited,
                security_origin_opaque: snapshot.security_origin_opaque,
                child_frames: snapshot.child_frames.into_iter().map(convert).collect(),
            }
        }

        self.child_browsing_context_frame_tree_snapshot()
            .into_iter()
            .map(convert)
            .collect()
    }

    pub(crate) fn child_browsing_context_owner_node_id_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<crate::dom::NodeId> {
        self._context_host
            .borrow()
            .child_browsing_context_owner_node_id_by_frame_id(frame_id)
    }

    pub(crate) fn child_browsing_context_frame_id_by_owner_node_id(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<String> {
        self._context_host
            .borrow()
            .child_browsing_context_frame_id_by_owner_node_id(owner_node_id)
    }

    pub(crate) fn child_browsing_context_is_same_origin_with_top(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> bool {
        self._context_host
            .borrow()
            .child_browsing_context_is_same_origin_with_top(owner_node_id)
    }

    pub(crate) fn child_browsing_context_has_opaque_origin(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> bool {
        self._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(owner_node_id)
    }

    pub(crate) fn child_browsing_context_current_url(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<url::Url> {
        self._context_host
            .borrow()
            .child_browsing_context_current_url(owner_node_id)
    }

    pub(crate) fn child_browsing_context_parent_frame_id(
        &self,
        owner_node_id: crate::dom::NodeId,
    ) -> Option<String> {
        self._context_host
            .borrow()
            .child_browsing_context_parent_frame_id(owner_node_id)
    }

    pub(crate) fn child_browsing_context_document_handle_by_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<crate::dom::NodeId> {
        let host = self._context_host.borrow();
        let child_handle = host.child_browsing_context_owner_node_id_by_frame_id(frame_id)?;
        host.child_browsing_context_document_handle(child_handle)
    }

    pub(crate) fn live_child_document_handles_in_snapshot_order(
        &self,
    ) -> Vec<(String, crate::dom::NodeId, crate::dom::NodeId)> {
        let host = self._context_host.borrow();
        host.child_browsing_context_handles_in_document_order()
            .into_iter()
            .filter_map(|child_handle| {
                let frame_id = host.frame_owner_frame_id_for_child_handle(child_handle)?.0;
                let document_handle = host.child_browsing_context_document_handle(child_handle)?;
                Some((frame_id, child_handle, document_handle))
            })
            .collect()
    }

    pub(crate) fn detached_child_browsing_context_document_snapshots_for_dom_snapshot(
        &mut self,
        top_frame_id: &str,
    ) -> Vec<crate::native_bridge::DetachedChildBrowsingContextDocumentSnapshot> {
        self._context_host
            .borrow_mut()
            .detached_child_browsing_context_document_snapshots_for_dom_snapshot(top_frame_id)
    }

    pub(crate) fn child_browsing_context_document_snapshot_by_frame_id(
        &mut self,
        frame_id: &str,
    ) -> Option<crate::native_bridge::ChildBrowsingContextDocumentSnapshot> {
        self._context_host
            .borrow_mut()
            .child_browsing_context_document_snapshot_by_frame_id(frame_id)
    }

    pub(super) fn top_level_navigation_history(&self) -> crate::runtime::RendererNavigationHistory {
        self._context_host.borrow().top_level_navigation_history()
    }

    pub(crate) fn take_pending_location_navigation_with_seed(
        &mut self,
    ) -> Option<crate::native_bridge::PendingLocationNavigation> {
        self._context_host
            .borrow_mut()
            .take_pending_location_navigation()
    }

    pub(crate) fn take_pending_location_navigation_of_kind(
        &mut self,
        kind: crate::native_bridge::PendingLocationNavigationKind,
    ) -> Option<crate::native_bridge::PendingLocationNavigation> {
        self._context_host
            .borrow_mut()
            .take_pending_location_navigation_of_kind(kind)
    }

    pub(crate) fn take_pending_document_location_navigation(
        &mut self,
    ) -> Option<crate::native_bridge::PendingLocationNavigation> {
        let source_url = self.document_runtime.document_url().clone();
        let pending = self.take_pending_location_navigation_of_kind(
            crate::native_bridge::PendingLocationNavigationKind::Document,
        )?;
        self.restore_top_level_location_runtime_state(&source_url);
        Some(pending)
    }

    /// Moves one browser-owned location request into the active concrete
    /// renderer output sink.
    ///
    /// The request remains renderer-local until lifecycle/command arbitration
    /// proves that the browser owns the navigation. At that exact boundary we
    /// consume it once, freeze its source Document and command cause, and stop
    /// relying on protocol to pull mutable Page state in a later turn.
    pub(crate) fn publish_pending_document_location_navigation(&mut self) -> anyhow::Result<bool> {
        let Some(pending) = self.take_pending_document_location_navigation() else {
            return Ok(false);
        };
        let source_document = pending.source_document.ok_or_else(|| {
            anyhow::anyhow!(
                "Page-owned location navigation was produced without an exact source Document"
            )
        })?;
        let runtime_command_cause = pending.runtime_command_cause;
        let action = crate::runtime::RendererOwnerAction::TopLevelLocationNavigation(
            crate::runtime::RendererDocumentSourcedTopLevelLocationNavigation::
                new_with_request_and_runtime_command_cause(
                    source_document,
                    pending.url.to_string(),
                    pending.request_method,
                    pending.request_body,
                    pending.request_headers,
                    pending.browser_navigation_kind,
                    runtime_command_cause.clone(),
                ).with_navigation_history(pending.entry_seed.map(|seed| self.top_level_navigation_history().request(seed))),
        );
        anyhow::ensure!(
            self._context_host
                .borrow()
                .append_owner_action_with_cause(runtime_command_cause, action),
            "browser-owned location navigation requires an active renderer output sink"
        );
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn take_pending_top_level_history_traversal(
        &mut self,
    ) -> Option<crate::runtime::RendererPendingTopLevelHistoryTraversal> {
        self._context_host
            .borrow_mut()
            .take_pending_top_level_history_traversal()
    }

    pub(crate) fn prepare_top_level_meta_refresh_navigation(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLifecycleFollowup> {
        if self.has_pending_location_navigation()
            || !self
                ._context_host
                .borrow()
                .current_main_document_load_has_dispatched(owner)
        {
            return None;
        }
        let scheduled = self
            .document_runtime
            .finish_top_level_meta_refresh_load(owner)?;
        let delay_ms = scheduled.navigation.delay_ms;
        let url = scheduled.navigation.url.clone();
        let ready_at = scheduled.ready_at;
        debug_assert_eq!(scheduled.owner, owner);
        tracing::debug!(?owner, %url, delay_ms, ?ready_at, "prepared post-load top-level meta refresh task");
        let (task, ready_at) = scheduled.into_internal_loading_task();
        Some(MainDocumentLifecycleFollowup::ScheduleInternalLoading { task, ready_at })
    }

    pub(crate) fn schedule_page_internal_loading_task(
        &self,
        task: crate::page_task_queue::PageOwnedInternalLoadingTask,
        ready_at: Instant,
    ) -> anyhow::Result<()> {
        self._context_host
            .borrow()
            .page_internal_loading_sender()
            .schedule_at(task, ready_at)
            .map_err(|_| anyhow::anyhow!("live Page internal-loading source closed"))
    }

    pub(crate) fn run_page_owned_internal_loading_task(
        &mut self,
        task: crate::page_task_queue::PageOwnedInternalLoadingTask,
    ) -> crate::page_task_queue::PageOwnedInternalLoadingTaskEffect {
        let crate::page_task_queue::PageOwnedInternalLoadingTask::MetaRefreshNavigation(task) =
            task;
        let owner = task.owner();
        let delay_ms = task.delay_ms();
        let url = task.into_url();
        let scheduler_owned_task = self
            .document_runtime
            .consume_top_level_meta_refresh_navigation(owner, delay_ms, &url);
        if !scheduler_owned_task {
            tracing::debug!(
                ?owner,
                %url,
                "discarded top-level meta refresh because its scheduler ownership no longer matches"
            );
            return crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated;
        }
        if self.has_pending_location_navigation() {
            // Consumption deliberately happens before this check. A competing
            // navigation that has already started supersedes the refresh, so
            // the posted task is retired rather than retried against a later
            // Document or after the competing navigation is handed off.
            tracing::debug!(
                ?owner,
                %url,
                "retired top-level meta refresh because a competing navigation is already pending"
            );
            return crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated;
        }
        tracing::debug!(?owner, %url, delay_ms, "activating post-load top-level meta refresh navigation");
        let activated = self
            .with_default_context_scope(|scope, _host_ptr| {
                Ok(crate::context_bootstrap::navigate_top_level_meta_refresh(
                    scope, &url, delay_ms,
                ))
            })
            .unwrap_or(false);
        if activated {
            crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated
        } else {
            crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationNotActivated
        }
    }

    pub(crate) fn apply_parser_stream_mutation_effects_to_live_dom_host_in_default_context(
        &mut self,
        effects: DomMutationEffects,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_parser_stream_mutation_effects_to_live_dom_host(scope, host_ptr, effects);
            Ok(())
        })
    }

    pub(crate) fn apply_parser_dom_mutation_to_live_dom_host_in_default_context(
        &mut self,
        mutation: crate::parser::ParserDomMutation,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .apply_parser_dom_mutation_to_live_dom_host(scope, host_ptr, mutation);
            Ok(())
        })
    }

    pub(crate) fn initialize_parser_added_body_window_handlers_in_default_context(
        &mut self,
        handlers: crate::native_bridge::element::ParserAddedBodyWindowHandlers,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            handlers.initialize(scope, host_ptr);
            Ok(())
        })
    }

    pub(crate) fn sync_selectedcontents_after_parser_option_finished_in_default_context(
        &mut self,
        option: NativeNodeId,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            unsafe { &mut *host_ptr }
                .sync_selectedcontents_after_parser_option_finished(scope, host_ptr, option);
            Ok(())
        })
    }

    pub(crate) fn apply_parser_created_null_registry_associations_in_default_context(
        &mut self,
        handles: &[NativeNodeId],
    ) -> Result<()> {
        self.with_default_context_scope(|_scope, host_ptr| {
            custom_elements::apply_parser_created_null_registry_associations(host_ptr, handles);
            Ok(())
        })
    }

    pub(crate) fn construct_parser_custom_element_handoff(
        &mut self,
        handoff: &crate::parser::ParserCustomElementConstructionHandoff,
    ) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            let _ = custom_elements::construct_parser_created_autonomous_element_from_handoff(
                scope, host_ptr, handoff,
            );
            Ok(())
        })
    }

    pub(crate) fn flush_parser_custom_element_handoff_replacements(&mut self) -> Result<()> {
        self.with_default_context_scope(|scope, host_ptr| {
            custom_elements::flush_parser_custom_element_handoff_replacements(scope, host_ptr);
            Ok(())
        })
    }

    pub(crate) fn install_session_history_position(
        &mut self,
        position: moli_session_history::SessionHistoryPosition,
    ) {
        let _ = self.with_default_context_scope(|scope, _runtime_ptr| {
            crate::context_bootstrap::install_session_history_position(scope, position);
            Ok(())
        });
    }

    #[cfg(test)]
    pub(crate) fn install_navigation_bootstrap_entry(
        &mut self,
        entry_seed: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    ) {
        self.install_navigation_bootstrap_from_history(entry_seed, None);
    }

    pub(super) fn install_navigation_bootstrap_from_history(
        &mut self,
        entry_seed: Option<super::native_bridge::NavigationHistoryEntrySeed>,
        source: Option<crate::runtime::RendererNavigationHistory>,
    ) {
        let Some(entry_seed) = entry_seed else {
            return;
        };
        let _ = self.with_default_context_scope(|scope, runtime_ptr| {
            if let Some(source) = &source {
                unsafe { &mut *runtime_ptr }
                    .restore_top_level_navigation_history(source, &entry_seed);
            }
            crate::context_bootstrap::install_navigation_bootstrap_entry(scope, &entry_seed);
            Ok(())
        });
    }
}
