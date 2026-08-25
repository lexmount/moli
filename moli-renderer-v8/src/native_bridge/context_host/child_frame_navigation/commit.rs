use super::super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextNavigationRequest,
    ChildBrowsingContextSnapshot, JsContextHost,
    child_documents::{ChildDocumentCommitResult, ChildDocumentCommitState},
};
use crate::native_bridge::context_host::child_documents::ChildDocumentNavigationInitiator;
use crate::{
    context_bootstrap::increment_top_level_history_length_for_runtime_owner,
    document_runtime::DomHandle,
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    frame_owner_model::{
        DocumentCreationKind, FrameDocumentInteractiveLifecycleAction,
        FrameDocumentJavascriptUrlPostExecutionApplication,
        FrameDocumentJavascriptUrlScriptExecutionAction,
        FrameDocumentJavascriptUrlScriptExecutionTarget, FrameDocumentTaskOwner,
        FrameLaneNavigationCommitTask, FrameNavigationCommitReservationResult, FrameRealmId,
        FrameScriptJobKind, PendingChildJavascriptUrlDocumentScript,
    },
};
use percent_encoding::percent_decode_str;
use url::Url;

pub(crate) struct ChildFrameNavigationCommitTaskRun {
    ready_work: Vec<FrameDocumentClassicScriptSchedulerWork>,
    parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
    owner_transition: Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
}

impl ChildFrameNavigationCommitTaskRun {
    fn new(
        ready_work: Vec<FrameDocumentClassicScriptSchedulerWork>,
        parser_stop_action: Option<FrameDocumentInteractiveLifecycleAction>,
        owner_transition: Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
    ) -> Self {
        Self {
            ready_work,
            parser_stop_action,
            owner_transition,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<FrameDocumentClassicScriptSchedulerWork>,
        Option<FrameDocumentInteractiveLifecycleAction>,
        Option<crate::frame_owner_model::FrameDocumentOwnerTransition>,
    ) {
        (
            self.ready_work,
            self.parser_stop_action,
            self.owner_transition,
        )
    }
}

impl JsContextHost {
    fn suppress_pending_child_host_load_before_navigation_commit(&mut self, handle: DomHandle) {
        let _ = self
            .frame_owner_store
            .retire_current_child_document_load_delivery_task_reservation(handle);
    }

    pub(crate) fn queue_child_browsing_context_navigation_commit(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.suppress_pending_child_host_load_before_navigation_commit(handle);
        let Some(owner) = self
            .frame_owner_store
            .current_child_frame_lane_task_owner(handle)
        else {
            return false;
        };
        let Some(navigation_load) = self.ensure_child_navigation_load(handle) else {
            tracing::warn!(
                ?handle,
                "refusing child navigation commit without a current document lifecycle owner"
            );
            return false;
        };
        let task = FrameLaneNavigationCommitTask {
            child_handle: handle,
            owner,
            navigation_load,
        };
        match self
            .frame_owner_store
            .reserve_current_child_navigation_commit_task(task)
        {
            FrameNavigationCommitReservationResult::Reserved => {}
            FrameNavigationCommitReservationResult::AlreadyReserved => return true,
            FrameNavigationCommitReservationResult::NotCurrent => return false,
        }
        if self
            .page_child_navigation_commit_sender()
            .send(task)
            .is_err()
        {
            let retired = self
                .frame_owner_store
                .retire_child_navigation_commit_task(task);
            debug_assert!(
                retired,
                "a closed child-navigation route must retire its exact reservation"
            );
            return false;
        }
        true
    }

    pub(crate) fn current_child_navigation_commit_task(
        &self,
        child_handle: DomHandle,
    ) -> Option<FrameLaneNavigationCommitTask> {
        self.frame_owner_store
            .current_child_navigation_commit_task(child_handle)
    }

    pub(crate) fn has_pending_child_navigation_commit_task(&self) -> bool {
        self.child_browsing_contexts.keys().any(|child_handle| {
            self.frame_owner_store
                .current_child_navigation_commit_task(*child_handle)
                .is_some()
        })
    }

    pub(crate) fn claim_child_navigation_commit_task(
        &mut self,
        expected: FrameLaneNavigationCommitTask,
    ) -> bool {
        self.frame_owner_store
            .claim_current_child_navigation_commit_task(expected)
    }

    pub(crate) fn retire_child_navigation_commit_task(
        &mut self,
        expected: FrameLaneNavigationCommitTask,
    ) -> bool {
        self.frame_owner_store
            .retire_child_navigation_commit_task(expected)
    }

    pub(in crate::native_bridge::context_host) fn retire_current_child_navigation_commit_task(
        &mut self,
        child_handle: DomHandle,
    ) -> bool {
        let Some(task) = self.current_child_navigation_commit_task(child_handle) else {
            return false;
        };
        self.retire_child_navigation_commit_task(task)
    }

    pub(crate) fn run_child_frame_navigation_commit_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: FrameLaneNavigationCommitTask,
    ) -> ChildFrameNavigationCommitTaskRun {
        if !self
            .frame_owner_store
            .child_frame_lane_task_owner_is_current(task.child_handle, task.owner)
            || !self.child_browsing_context_is_live(task.child_handle)
            || self.current_child_navigation_load(task.child_handle) != Some(task.navigation_load)
        {
            return ChildFrameNavigationCommitTaskRun::new(Vec::new(), None, None);
        }

        let handle = task.child_handle;
        let has_pending_navigation = self
            .child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_live_navigation())
            .is_some();
        let has_pending_attribute_bootstrap = self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.pending_attribute_bootstrap_commit());
        let result = if has_pending_navigation {
            self.commit_pending_child_browsing_context_navigation_if_needed(
                scope,
                handle,
                task.navigation_load,
            )
        } else if has_pending_attribute_bootstrap {
            self.commit_child_browsing_context_attribute_bootstrap_if_needed(
                scope,
                handle,
                task.navigation_load,
            )
        } else {
            None
        };

        let mut ready_work = Vec::new();
        let mut parser_stop_action = None;
        let mut owner_transition = None;
        if let Some(result) = result {
            owner_transition = result.owner_transition;
            self.queue_followup_after_child_navigation_commit(
                handle,
                result,
                &mut ready_work,
                &mut parser_stop_action,
            );
        } else if (has_pending_navigation || has_pending_attribute_bootstrap)
            && !self.child_browsing_context_has_pending_javascript_url_navigation(handle)
            && !self.child_document_load_is_pending(handle)
            && !self.child_browsing_context_has_pending_navigation_or_document_load(handle)
        {
            let _ = self
                .finish_child_frame_navigation_without_load_dispatch(handle, task.navigation_load);
            let _ = self.queue_child_document_complete_lifecycle_if_ready(handle);
        }
        ChildFrameNavigationCommitTaskRun::new(ready_work, parser_stop_action, owner_transition)
    }

    fn queue_followup_after_child_navigation_commit(
        &mut self,
        handle: DomHandle,
        result: ChildDocumentCommitResult,
        ready_work: &mut Vec<FrameDocumentClassicScriptSchedulerWork>,
        parser_stop_action: &mut Option<FrameDocumentInteractiveLifecycleAction>,
    ) {
        let produced_initial_classic_ready_work = result.initial_classic_ready_work.is_some();
        *parser_stop_action = result.parser_stop_action;
        if let Some(work) = result.initial_classic_ready_work {
            ready_work.push(work);
        }
        if result.state == ChildDocumentCommitState::Ready
            && !produced_initial_classic_ready_work
            && parser_stop_action.is_none()
        {
            let _ = self.queue_child_document_complete_lifecycle_if_ready(handle);
        }
    }

    pub(in crate::native_bridge::context_host) fn queue_child_browsing_context_navigation_to_url(
        &mut self,
        handle: DomHandle,
        url: &Url,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Url(url.clone()),
                true,
            )
            .is_none()
        {
            return false;
        }
        self.register_reserved_service_worker_child_client_for_navigation(handle, url);
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(in crate::native_bridge::context_host) fn queue_child_browsing_context_navigation_request(
        &mut self,
        handle: DomHandle,
        request: ChildBrowsingContextNavigationRequest,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "The navigation was canceled.".to_owned(),
        );
        let document_url = request.url.clone();
        if self
            .set_child_browsing_context_pending_navigation(
                handle,
                ChildBrowsingContextBootstrap::Request(request),
                true,
            )
            .is_none()
        {
            return false;
        }
        self.register_reserved_service_worker_child_client_for_navigation(handle, &document_url);
        self.queue_child_browsing_context_navigation_commit(handle)
    }

    pub(in crate::native_bridge::context_host) fn commit_child_browsing_context_attribute_bootstrap_if_needed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        navigation_load: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
    ) -> Option<ChildDocumentCommitResult> {
        let entry_snapshot = self.child_browsing_contexts.get(&handle).cloned()?;
        if !entry_snapshot.pending_attribute_bootstrap_commit() {
            return None;
        }
        if self.current_child_navigation_load(handle) != Some(navigation_load) {
            tracing::debug!(
                ?handle,
                ?navigation_load,
                "ignored stale child attribute navigation commit"
            );
            return None;
        }
        let attribute_bootstrap = entry_snapshot.attribute_bootstrap().clone();
        let url = Self::child_browsing_context_navigation_entry_url(&attribute_bootstrap)?;
        let is_attribute_about_blank_noop = matches!(
            attribute_bootstrap,
            ChildBrowsingContextBootstrap::AboutBlank
        ) || matches!(
            attribute_bootstrap,
            ChildBrowsingContextBootstrap::Url(ref target) if target.as_str() == "about:blank"
        );
        let live_bootstrap = entry_snapshot.live_bootstrap();
        let live_bootstrap_is_about_blank =
            matches!(live_bootstrap, ChildBrowsingContextBootstrap::AboutBlank);
        let is_initial_about_blank_commit = live_bootstrap_is_about_blank
            && entry_snapshot.navigation_seed_is_initial_about_blank_commit();
        let is_initial_attribute_target_seed = live_bootstrap_is_about_blank
            && entry_snapshot.navigation_seed_is_initial_attribute_target(&url);
        let is_javascript_attribute_target = matches!(
            attribute_bootstrap,
            ChildBrowsingContextBootstrap::Url(ref target) if target.scheme() == "javascript"
        );
        let preserve_delayed_initial_window_event_state = is_javascript_attribute_target
            && is_initial_about_blank_commit
            && self
                .dom_host()
                .node(handle)
                .is_some_and(|node| node.flags().parser_created());
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            if is_attribute_about_blank_noop {
                entry.clear_navigation_activation();
            } else if is_javascript_attribute_target {
                entry.apply_javascript_url_navigation_to_entry_seed();
            } else if is_initial_about_blank_commit {
                entry.apply_initial_attribute_target_navigation_entry(&url);
            } else if is_initial_attribute_target_seed {
                entry.mark_initial_attribute_target_navigation_activation();
            } else {
                entry.replace_navigation_in_entry_seed(&url);
            }
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.clear_pending_attribute_bootstrap_commit();
        }
        if let ChildBrowsingContextBootstrap::Url(url) = &attribute_bootstrap
            && url.scheme() == "javascript"
        {
            let dispatch_load_on_no_string_completion =
                entry_snapshot.navigation_seed_is_initial_about_blank_commit();
            let entry = self.child_browsing_contexts.get_mut(&handle)?;
            entry.set_pending_navigation(ChildBrowsingContextBootstrap::Url(url.clone()), true);
            self.queue_child_browsing_context_javascript_url_execution(
                handle,
                url.clone(),
                preserve_delayed_initial_window_event_state,
                dispatch_load_on_no_string_completion,
                navigation_load,
            );
            self.sync_existing_child_browsing_context_window_state(scope, handle);
            return None;
        }
        if is_attribute_about_blank_noop && is_initial_about_blank_commit {
            // The initial about:blank document exists synchronously when the
            // iframe is inserted. Parent script may mutate contentDocument
            // before our deferred host-load turn; committing the same
            // about:blank again would replace that live wrapper and lose
            // dynamically inserted nodes/scripts.
            self.sync_existing_child_browsing_context_runtime_surface_from_seed(scope, handle);
            let _ = self.settle_child_navigation_load(handle, navigation_load, true);
            return None;
        }
        let commit_result = self.commit_child_document_bootstrap_or_start_load(
            scope,
            handle,
            attribute_bootstrap,
            navigation_load,
            ChildDocumentNavigationInitiator::FrameOwnerElement,
        );
        self.sync_existing_child_browsing_context_window_state(scope, handle);
        commit_result
    }

    pub(in crate::native_bridge::context_host) fn commit_pending_child_browsing_context_navigation_if_needed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        navigation_load: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
    ) -> Option<ChildDocumentCommitResult> {
        let entry_snapshot = self.child_browsing_contexts.get(&handle).cloned()?;
        let pending_bootstrap = entry_snapshot.pending_live_navigation()?;
        if self.current_child_navigation_load(handle) != Some(navigation_load) {
            tracing::debug!(
                ?handle,
                ?navigation_load,
                "ignored stale child navigation commit"
            );
            return None;
        }
        if let ChildBrowsingContextBootstrap::Url(url) = &pending_bootstrap
            && url.scheme() == "javascript"
        {
            self.queue_child_browsing_context_javascript_url_execution(
                handle,
                url.clone(),
                false,
                false,
                navigation_load,
            );
            self.sync_existing_child_browsing_context_window_state(scope, handle);
            return None;
        }
        let increment_top_level_history_length = self
            .child_browsing_contexts
            .get_mut(&handle)
            .is_some_and(|entry| entry.take_pending_top_level_history_length_increment());
        self.clear_child_browsing_context_pending_navigation(handle);
        self.clear_pending_form_submission_child_target(handle);
        let commit_result = self.commit_child_document_bootstrap_or_start_load(
            scope,
            handle,
            pending_bootstrap,
            navigation_load,
            ChildDocumentNavigationInitiator::BrowsingContext,
        );
        self.sync_existing_child_browsing_context_window_state(scope, handle);
        if increment_top_level_history_length
            && let Some(window) = self.child_browsing_context_window_wrapper(scope, handle)
        {
            increment_top_level_history_length_for_runtime_owner(scope, window);
        }
        commit_result
    }

    pub(crate) fn queue_child_browsing_context_javascript_url_execution(
        &mut self,
        handle: DomHandle,
        url: Url,
        preserve_window_event_state: bool,
        dispatch_load_on_no_string_completion: bool,
        navigation_load: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
    ) {
        if !self.child_browsing_context_scripting_enabled(handle) {
            let cancelled = self
                .cancel_child_javascript_url_navigation_without_execution(handle, navigation_load);
            tracing::debug!(
                ?handle,
                %url,
                cancelled,
                "blocked child javascript URL because scripting is disabled"
            );
            return;
        }
        let Some(snapshot) = self.frame_owner_store.current_child_owner_snapshot(handle) else {
            let _ = self
                .cancel_child_javascript_url_navigation_without_execution(handle, navigation_load);
            return;
        };
        let task_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        let source = javascript_url_source(&url);
        let work = PendingChildJavascriptUrlDocumentScript {
            child_handle: handle,
            owner: task_owner,
            realm_id: snapshot.realm_id,
            navigation_load,
            url: url.clone(),
            source,
            preserve_window_event_state,
            dispatch_load_on_no_string_completion,
        };
        if self
            .queue_child_document_script_work_with_realm_prerequisite(
                crate::frame_owner_model::FrameDocumentUnboundScriptWork::JavascriptUrl(work),
            )
            .is_none()
        {
            let _ = self
                .cancel_child_javascript_url_navigation_without_execution(handle, navigation_load);
            return;
        }
        tracing::debug!(
            handle = ?handle,
            url = %url,
            preserve_window_event_state,
            "queued child javascript URL execution"
        );
    }

    fn cancel_child_javascript_url_navigation_without_execution(
        &mut self,
        handle: DomHandle,
        navigation_load: crate::frame_owner_model::FrameDocumentNavigationLoadBinding,
    ) -> bool {
        if self.current_child_navigation_load(handle) != Some(navigation_load) {
            return false;
        }
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return false;
        };
        entry.clear_pending_navigation();
        entry.clear_pending_top_level_history_length_increment();
        entry.restore_navigation_entry_seed_from_committed();
        self.clear_pending_form_submission_child_target(handle);
        self.reject_replaced_service_worker_child_client_navigation(
            handle,
            "Cannot navigate to URL.".to_owned(),
        );
        self.finish_child_frame_navigation_without_load_dispatch(handle, navigation_load)
    }

    pub(crate) fn drop_child_javascript_url_document_script(
        &mut self,
        work: &PendingChildJavascriptUrlDocumentScript,
    ) -> bool {
        if !self.frame_document_task_owner_is_current(work.child_handle, work.owner) {
            return false;
        }
        self.cancel_child_javascript_url_navigation_without_execution(
            work.child_handle,
            work.navigation_load,
        )
    }

    pub(crate) fn frame_document_task_owner_is_current(
        &self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store
            .current_child_owner_snapshot(handle)
            .is_some_and(|snapshot| {
                snapshot.scheduler_lane_id == owner.scheduler_lane_id
                    && snapshot.local_window_id == owner.local_window_id
                    && snapshot.document_id == owner.document_id
            })
    }

    pub(crate) fn child_javascript_url_script_execution_action_for_owner(
        &self,
        work: &PendingChildJavascriptUrlDocumentScript,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentJavascriptUrlScriptExecutionAction> {
        if !self.frame_document_task_owner_is_current(work.child_handle, work.owner)
            || self.current_child_navigation_load(work.child_handle) != Some(work.navigation_load)
            || !self.child_browsing_context_scripting_enabled(work.child_handle)
        {
            return None;
        }
        let target = work.execution_target(realm_id);
        let job = self
            .frame_owner_store
            .child_javascript_url_script_job_for_owner(
                work.child_handle,
                work.owner.local_window_id,
                work.owner.document_id,
                work.url.clone(),
                work.source.clone(),
            )?;
        if job.kind != FrameScriptJobKind::JavascriptUrl {
            return None;
        }
        Some(FrameDocumentJavascriptUrlScriptExecutionAction::new(
            target,
            job,
            work.url.clone(),
            work.preserve_window_event_state,
            work.dispatch_load_on_no_string_completion,
        ))
    }

    pub(crate) fn finish_child_javascript_url_without_string_completion(
        &mut self,
        target: FrameDocumentJavascriptUrlScriptExecutionTarget,
        dispatch_load: bool,
    ) -> bool {
        if !self.frame_document_task_owner_is_current(target.child_handle(), target.task_owner()) {
            return false;
        }
        if dispatch_load {
            if !self.settle_child_navigation_load(
                target.child_handle(),
                target.navigation_load(),
                false,
            ) {
                return false;
            }
            self.clear_child_browsing_context_pending_navigation(target.child_handle());
            self.queue_child_document_complete_lifecycle_if_ready_for_document_realm(
                target.owner(),
                target.realm_id(),
            )
        } else {
            if !self.finish_child_frame_navigation_without_load_dispatch(
                target.child_handle(),
                target.navigation_load(),
            ) {
                return false;
            }
            self.clear_child_browsing_context_pending_navigation(target.child_handle());
            false
        }
    }

    pub(crate) fn commit_child_javascript_url_string_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        target: FrameDocumentJavascriptUrlScriptExecutionTarget,
        url: Url,
        markup: String,
        preserve_window_event_state: bool,
    ) -> FrameDocumentJavascriptUrlPostExecutionApplication {
        let attempted_script_job = true;
        if !self.frame_document_task_owner_is_current(target.child_handle(), target.task_owner()) {
            return FrameDocumentJavascriptUrlPostExecutionApplication {
                attempted_script_job,
                failed_script_job: false,
                string_completion_committed: false,
                lifecycle_followup_queued: false,
                initial_classic_ready_work: None,
                owner_transition: None,
            };
        }
        let realm_current = self
            .frame_owner_store
            .current_child_owner_snapshot_for_realm(target.realm_id())
            .is_some_and(|snapshot| {
                snapshot.owner_handle == target.child_handle()
                    && snapshot.local_window_id == target.task_owner().local_window_id
                    && snapshot.document_id == target.task_owner().document_id
            });
        if !realm_current {
            return FrameDocumentJavascriptUrlPostExecutionApplication {
                attempted_script_job,
                failed_script_job: false,
                string_completion_committed: false,
                lifecycle_followup_queued: false,
                initial_classic_ready_work: None,
                owner_transition: None,
            };
        }

        let handle = target.child_handle();
        let window_commit_preflight = self.capture_child_document_window_commit_preflight(handle);
        let replacement_document_url = self
            .child_browsing_context_current_url(handle)
            .or_else(|| {
                self.child_browsing_context_document_handle(handle)
                    .map(|document| self.document_url_for_handle(document))
            })
            .unwrap_or_else(|| url.clone());
        let policy_container = self
            .child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.document_policy_container_snapshot())
            .unwrap_or_default();
        let sandbox = self.child_browsing_context_sandbox_policy_from_owner(handle);
        let owner_credentialless = self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.owner_credentialless());
        let document_credentialless = self
            .child_browsing_context_document_credentialless_for_owner(handle, owner_credentialless);
        let credentialless_storage_nonce =
            self.child_document_credentialless_storage_nonce(document_credentialless);

        self.clear_pending_child_document_loads_for_handle(handle);
        self.dispatch_child_browsing_context_unload_lifecycle_if_needed(scope, handle);
        if !self.child_document_window_commit_preflight_is_current(handle, &window_commit_preflight)
        {
            let _ = self.finish_child_frame_navigation_without_load_dispatch(
                handle,
                target.navigation_load(),
            );
            return FrameDocumentJavascriptUrlPostExecutionApplication {
                attempted_script_job,
                failed_script_job: false,
                string_completion_committed: false,
                lifecycle_followup_queued: false,
                initial_classic_ready_work: None,
                owner_transition: None,
            };
        }
        self.clear_child_browsing_context_pending_navigation(handle);
        let snapshot = ChildBrowsingContextSnapshot::html(replacement_document_url, markup)
            .with_policy_container(policy_container);
        {
            let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
                return FrameDocumentJavascriptUrlPostExecutionApplication {
                    attempted_script_job,
                    failed_script_job: false,
                    string_completion_committed: false,
                    lifecycle_followup_queued: false,
                    initial_classic_ready_work: None,
                    owner_transition: None,
                };
            };
            entry.commit_new_child_document(
                ChildBrowsingContextBootstrap::Url(url),
                Some(&snapshot),
                sandbox,
                document_credentialless,
                credentialless_storage_nonce,
            );
        }
        let window_commit = self.plan_child_document_window_commit(
            handle,
            &snapshot,
            window_commit_preflight,
            DocumentCreationKind::JavascriptUrl,
            None,
        );
        let snapshot = self.cache_child_snapshot_with_current_document_policy(handle, snapshot);
        let Some(install) = snapshot.as_ref().and_then(|snapshot| {
            self.install_child_browsing_context_current_document_from_snapshot(
                scope,
                handle,
                snapshot,
                window_commit,
                preserve_window_event_state,
                None,
            )
        }) else {
            let _ = self.finish_child_frame_navigation_without_load_dispatch(
                handle,
                target.navigation_load(),
            );
            return FrameDocumentJavascriptUrlPostExecutionApplication {
                attempted_script_job,
                failed_script_job: true,
                string_completion_committed: false,
                lifecycle_followup_queued: false,
                initial_classic_ready_work: None,
                owner_transition: None,
            };
        };
        let initial_classic_ready_work = install.initial_classic_ready_work;
        let parser_stop_queued = install
            .parser_stop_action
            .is_some_and(|action| self.queue_child_document_interactive_lifecycle_action(action));
        self.promote_pending_service_worker_child_client(handle);
        self.register_or_update_service_worker_child_client(handle);
        self.complete_pending_service_worker_child_client_navigation(handle);
        self.sync_existing_child_browsing_context_window_state(scope, handle);
        let lifecycle_followup_queued = if parser_stop_queued {
            true
        } else if initial_classic_ready_work.is_none() {
            self.queue_child_document_complete_lifecycle_if_ready(handle)
        } else {
            false
        };
        FrameDocumentJavascriptUrlPostExecutionApplication {
            attempted_script_job,
            failed_script_job: false,
            string_completion_committed: true,
            lifecycle_followup_queued,
            initial_classic_ready_work,
            owner_transition: Some(install.owner_transition),
        }
    }
}

fn javascript_url_source(url: &Url) -> String {
    let source = url
        .as_str()
        .strip_prefix("javascript:")
        .unwrap_or_else(|| url.path());
    percent_decode_str(source).decode_utf8_lossy().into_owned()
}
