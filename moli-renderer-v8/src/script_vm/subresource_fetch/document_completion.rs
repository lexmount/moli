use super::*;

impl ScriptVm {
    #[cfg(test)]
    pub(crate) fn current_document_write_external_script_fetch_target(
        &self,
    ) -> Option<crate::types::DocumentWriteExternalScriptFetchTarget> {
        let target = self
            .document_runtime
            .pending_document_write_external_script_fetch_target()?;
        self.document_write_external_script_fetch_target_is_current(target)
            .then_some(target)
    }
    pub(crate) fn document_write_external_script_fetch_target_is_current(
        &self,
        expected: crate::types::DocumentWriteExternalScriptFetchTarget,
    ) -> bool {
        self.document_runtime
            .has_document_write_external_script_fetch_target(expected)
            && self.current_main_document_task_owner() == Some(expected.task_owner())
    }
    pub(crate) fn apply_current_document_write_external_script_load_completion(
        &mut self,
        completion: crate::runtime::AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion,
    ) -> Result<crate::document_runtime::DocumentWriteExternalScriptLoadApplication> {
        let completion = completion.into_completion();
        let document_runtime: *mut DocumentRuntime = &mut *self.document_runtime;
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *document_runtime }
                .complete_document_write_external_script_load(scope, host_ptr, completion))
        })
    }
    pub(crate) fn apply_current_child_document_load_completion(
        &mut self,
        authorization: AuthorizedCurrentChildDocumentLoadCompletion,
    ) -> Result<CurrentChildDocumentLoadApplication> {
        self.apply_current_child_document_load_completion_inner(authorization.into_completion())
    }
    pub(super) fn apply_current_child_document_load_completion_inner(
        &mut self,
        completion: ChildDocumentLoadCompletion,
    ) -> Result<CurrentChildDocumentLoadApplication> {
        let context_host = self._context_host.clone();
        let application = self.with_default_context_scope(move |scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_current_child_document_load_completion(scope, completion))
        })?;
        let application = match application {
            crate::native_bridge::ChildDocumentLoadApplication::Applied {
                followup,
                body_activity,
            } => (followup.map(|application| *application), body_activity),
            crate::native_bridge::ChildDocumentLoadApplication::SupersededDuringApplication {
                completion,
                body_activity,
            } => {
                let historical_network_recorded =
                    self.record_historical_child_document_load_network(&completion);
                self.apply_pending_child_document_owner_retirements();
                return Ok(
                    CurrentChildDocumentLoadApplication::SupersededDuringApplication {
                        historical_network_recorded,
                        body_activity,
                    },
                );
            }
        };
        self.apply_pending_child_document_owner_retirements();
        let (application, mut body_activity) = application;
        let Some(application) = application else {
            return Ok(CurrentChildDocumentLoadApplication::Applied { body_activity });
        };
        let (work, parser_stop_action, owner_transition) = application.into_followups();
        if let Some(transition) = owner_transition {
            self.apply_child_document_owner_transition(transition);
        }
        if let Some(action) = parser_stop_action
            && super::super::child_document_lifecycle::ChildDocumentLifecycleOwner::new(self)
                .notify_parser_stop_action(action)?
                == crate::frame_owner_model::FrameDocumentLifecycleTaskEffect::EventDispatched
        {
            body_activity =
                crate::native_bridge::ChildDocumentLoadBodyActivity::PageCodeOrEventDispatch;
        }
        if let Some(work) = work {
            super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(
                self,
            )
            .notify_parser_classic_next_owner_action(work);
        }
        Ok(CurrentChildDocumentLoadApplication::Applied { body_activity })
    }
    pub(crate) fn current_child_document_navigation_fetch_target(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::ChildDocumentNavigationFetchTarget> {
        self._context_host
            .borrow()
            .current_child_document_navigation_fetch_target(child_handle)
    }
    pub(crate) fn record_historical_child_document_load_network(
        &mut self,
        completion: &ChildDocumentLoadCompletion,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .record_historical_child_document_load_network(completion)
    }
    pub(crate) fn discard_stale_child_document_load_completion(
        &mut self,
        target: crate::frame_owner_model::ChildDocumentNavigationFetchTarget,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_child_document_load_completion(target);
    }
    pub(crate) fn apply_child_blocking_stylesheet_load_completion_from_page_turn(
        &mut self,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) -> Result<()> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            context_host
                .borrow_mut()
                .apply_child_blocking_stylesheet_load_completion(scope, completion);
            Ok(())
        })
    }
    pub(crate) fn record_historical_child_blocking_stylesheet_network_results(
        &mut self,
        completion: &ChildBlockingStylesheetLoadCompletion,
    ) {
        self._context_host
            .borrow_mut()
            .record_historical_child_blocking_stylesheet_network_results(completion);
    }
    pub(crate) fn current_child_document_task_owner(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
    }
    pub(crate) fn current_child_document_module_fetch_target(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::ChildDocumentModuleFetchTarget> {
        self._context_host
            .borrow()
            .current_child_document_module_fetch_target(child_handle)
    }
    pub(crate) fn apply_child_classic_script_load_completion_from_page_turn(
        &mut self,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Result<()> {
        let context_host = self._context_host.clone();
        let application = self.with_default_context_scope(move |_scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_child_classic_script_load_completion(completion))
        })?;
        if let Some(application) = application {
            if let Some(work) = application.scheduler_work {
                super::super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(
                    self,
                )
                .notify_parser_classic_next_owner_action(work);
                return Ok(());
            }
            let _ = application.queued_document_script_ready;
            let _ = application.queued_document_lifecycle;
        }
        Ok(())
    }
    pub(crate) fn record_historical_child_classic_script_network_result(
        &mut self,
        completion: &ChildClassicScriptLoadCompletion,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .record_historical_child_classic_script_network_result(completion)
    }
    /// Applies a parser-module terminal only after the Page owner has proved
    /// that its complete exact target is current.
    pub(crate) fn apply_current_child_parser_module_root_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildParserModuleRootFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.finish_child_parser_module_root_fetch_completion(authorization.into_completion())
    }
    pub(super) fn finish_child_parser_module_root_fetch_completion(
        &mut self,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let applied = self
            ._context_host
            .borrow_mut()
            .finish_child_parser_module_root_fetch_request(&completion);
        if applied {
            return self.apply_child_parser_module_root_fetch_completion_to_owner(completion);
        }
        FrameDocumentModuleTerminalQueueFollowup::none()
    }
    /// Applies a dependency terminal only after the Page owner has proved
    /// that its complete exact target is current.
    pub(crate) fn apply_current_child_module_dependency_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildModuleDependencyFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.finish_child_module_dependency_fetch_completion(authorization.into_completion())
    }
    /// Applies a child `modulepreload` terminal only after the Page owner has
    /// proved its complete exact target is current.
    pub(crate) fn apply_current_child_modulepreload_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildModulepreloadFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        super::super::child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_current_modulepreload_fetch_completion(authorization.into_completion())
    }
    pub(super) fn finish_child_module_dependency_fetch_completion(
        &mut self,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let applied = self
            ._context_host
            .borrow_mut()
            .finish_child_module_dependency_fetch_request(&completion);
        if applied {
            return self.apply_child_module_dependency_fetch_completion_to_owner(completion);
        }
        FrameDocumentModuleTerminalQueueFollowup::none()
    }
    pub(crate) fn record_current_child_module_fetch_network_result(
        &mut self,
        attribution: &crate::types::ChildModuleFetchNetworkAttribution,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> bool {
        let Some(network_result) = network_result else {
            return false;
        };
        self._context_host
            .borrow_mut()
            .record_current_child_module_fetch_network_result(attribution, network_result.as_ref());
        true
    }
    pub(crate) fn record_historical_child_module_fetch_network_result(
        &mut self,
        attribution: &crate::types::ChildModuleFetchNetworkAttribution,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> bool {
        let Some(network_result) = network_result else {
            return false;
        };
        self._context_host
            .borrow_mut()
            .record_historical_child_module_fetch_network_result(
                attribution,
                network_result.as_ref(),
            );
        true
    }
    #[cfg(test)]
    pub(crate) fn complete_popup_document_load(
        &mut self,
        completion: PopupDocumentLoadCompletion,
    ) -> Result<()> {
        let target = completion.target();
        if self.current_lightweight_popup_document_fetch_target(target.load_id()) != Some(target) {
            return Ok(());
        }
        let _ = self.apply_popup_document_load_completion_inner(completion)?;
        Ok(())
    }
    pub(crate) fn current_lightweight_popup_document_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::native_bridge::LightweightPopupDocumentFetchTarget> {
        self._context_host
            .borrow()
            .current_lightweight_popup_document_fetch_target(load_id)
    }
    pub(crate) fn apply_current_popup_document_load_completion(
        &mut self,
        authorization: AuthorizedCurrentPopupDocumentLoadCompletion,
    ) -> Result<crate::native_bridge::PopupDocumentLoadApplication> {
        self.apply_popup_document_load_completion_inner(authorization.into_completion())
    }
    pub(super) fn apply_popup_document_load_completion_inner(
        &mut self,
        completion: PopupDocumentLoadCompletion,
    ) -> Result<crate::native_bridge::PopupDocumentLoadApplication> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            let application = context_host
                .borrow_mut()
                .apply_lightweight_popup_document_load_completion(scope, completion);
            Ok(application)
        })
    }
    pub(crate) fn current_lightweight_popup_classic_script_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::native_bridge::LightweightPopupClassicScriptFetchTarget> {
        self._context_host
            .borrow()
            .current_lightweight_popup_classic_script_fetch_target(load_id)
    }
    pub(crate) fn discard_stale_lightweight_popup_classic_script_completion(
        &mut self,
        target: crate::native_bridge::LightweightPopupClassicScriptFetchTarget,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_lightweight_popup_classic_script_completion(target);
    }
    pub(crate) fn apply_current_popup_classic_script_load_completion(
        &mut self,
        authorization: AuthorizedCurrentPopupClassicScriptLoadCompletion,
    ) -> Result<crate::native_bridge::PopupClassicScriptLoadApplication> {
        let completion: PopupClassicScriptLoadCompletion = authorization.into_completion();
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_lightweight_popup_classic_script_load_completion(scope, completion))
        })
    }
}
