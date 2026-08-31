use super::JsContextHost;
use crate::frame_owner_model::{
    DocumentLinkEventOwner, DocumentLoadDelayTokenId, FrameDocumentTaskOwner,
    MainDocumentCompleteLifecycleAction, MainDocumentDomContentLoadedLifecycleAction,
    MainDocumentImageLoadDelayBinding, MainDocumentInteractiveLifecycleAction,
    MainDocumentLoadCompletionState, MainDocumentMediaLoadDelayBinding,
    MainDocumentScriptLoadDelayKind, MainDocumentScriptLoadDelayLease,
    MainDocumentScriptLoadDelayRelease, MainDocumentStyleLoadEventBinding,
};
use crate::{
    document_runtime::{
        ConnectedStyleLoadEventAdmission, ConnectedStyleLoadEventPlan, DocumentRuntime,
    },
    host::{
        CommittedInlineClassicScript, PreparedRuntimeScriptStartCommit, RuntimeScriptAdmission,
        RuntimeScriptStartPlan,
    },
};

impl JsContextHost {
    pub(crate) fn plan_and_commit_current_main_runtime_script_start(
        &mut self,
        node: crate::document_runtime::DomHandle,
        host_script_handle: &str,
    ) -> std::result::Result<Option<CommittedInlineClassicScript>, String> {
        let runtime = unsafe { &mut *self.runtime };
        let Some(plan) = runtime.host_plan_script_start(node, host_script_handle) else {
            return Ok(None);
        };
        self.commit_current_main_runtime_script_start(runtime, plan)
    }

    pub(crate) fn commit_current_main_runtime_script_start(
        &mut self,
        runtime: &mut DocumentRuntime,
        plan: RuntimeScriptStartPlan,
    ) -> std::result::Result<Option<CommittedInlineClassicScript>, String> {
        let load_delay_binding = if plan.requires_runtime_admission() {
            let owner = self.current_main_document_task_owner().ok_or_else(|| {
                "runtime script start requires a current main Document owner".to_owned()
            })?;
            let kind = plan
                .load_delay_kind()
                .expect("runtime-script admission plan must classify its load delay");
            Some(
                self.frame_owner_store
                    .acquire_current_main_document_script_load_delay(owner, kind)
                    .ok_or_else(|| {
                        "current main Document rejected runtime script load-delay lease".to_owned()
                    })?,
            )
        } else {
            None
        };

        let prepared = match runtime.prepare_runtime_script_start_commit(plan) {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Some(binding) = load_delay_binding {
                    let _ = self.release_main_document_script_load_delay(binding);
                }
                return Err(error);
            }
        };

        match prepared {
            PreparedRuntimeScriptStartCommit::Noop => {
                if let Some(binding) = load_delay_binding {
                    let _ = self.release_main_document_script_load_delay(binding);
                }
                Ok(None)
            }
            PreparedRuntimeScriptStartCommit::InlineClassic {
                node,
                host_script_handle,
                source,
            } => {
                debug_assert!(load_delay_binding.is_none());
                Ok(Some(CommittedInlineClassicScript::new(
                    node,
                    host_script_handle,
                    source,
                )))
            }
            PreparedRuntimeScriptStartCommit::Admission {
                reservation,
                payload,
            } => {
                let binding = load_delay_binding
                    .expect("runtime-script admission must own its load-delay lease");
                let admission = RuntimeScriptAdmission::from_boxed_payload(payload, binding);
                if let Err(rejected) = runtime.publish_runtime_script_admission(admission) {
                    let (_, binding) = rejected.into_parts();
                    let _ = self.release_main_document_script_load_delay(binding);
                    runtime.cancel_runtime_script_start_admission(reservation);
                    return Err(
                        "main-Document runtime route closed before script admission".to_owned()
                    );
                }
                // Stable-source publication stores the task but cannot execute it
                // reentrantly on this owner thread, so the exact reservation is
                // committed before any consumer can observe the admission.
                runtime.finish_runtime_script_start_admission(reservation);
                Ok(None)
            }
        }
    }

    pub(crate) fn main_document_task_owner_is_current(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store
            .main_document_task_owner_is_current(owner)
    }

    pub(crate) fn finish_current_main_document_parsing(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentInteractiveLifecycleAction> {
        self.frame_owner_store
            .finish_current_main_document_parsing(owner)
    }

    pub(crate) fn acquire_current_main_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<DocumentLoadDelayTokenId> {
        self.frame_owner_store
            .acquire_current_main_parser_deferred_script_load_delay(owner)
    }

    pub(crate) fn release_main_parser_deferred_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        token: DocumentLoadDelayTokenId,
    ) -> bool {
        self.frame_owner_store
            .release_parser_deferred_script_load_delay(owner, token)
    }

    pub(crate) fn acquire_current_main_document_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        kind: MainDocumentScriptLoadDelayKind,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        self.frame_owner_store
            .acquire_current_main_document_script_load_delay(owner, kind)
    }

    pub(crate) fn release_main_document_script_load_delay(
        &mut self,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> MainDocumentScriptLoadDelayRelease {
        self.frame_owner_store
            .release_main_document_script_load_delay(binding)
    }

    pub(crate) fn accept_current_main_style_load_event(
        &mut self,
        element: crate::document_runtime::DomHandle,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        let owner = self.current_main_document_task_owner()?;
        self.frame_owner_store
            .accept_current_main_style_load_event(owner, element)
    }

    /// Commit the lifecycle authority for a plan prepared by
    /// `DocumentRuntime`.
    ///
    /// This method deliberately does not dereference `self.runtime`. The
    /// caller owns the mutable runtime phase, temporarily borrows the context
    /// host only for this exact FrameOwnerStore transaction, and then applies
    /// the committed value back to the runtime synchronously.
    pub(crate) fn commit_connected_style_load_event_plan(
        &mut self,
        plan: ConnectedStyleLoadEventPlan,
    ) -> Option<ConnectedStyleLoadEventAdmission> {
        match plan {
            ConnectedStyleLoadEventPlan::LoadDelaying { element } => {
                let binding = self.accept_current_main_style_load_event(element)?;
                Some(ConnectedStyleLoadEventAdmission::LoadDelaying(binding))
            }
            ConnectedStyleLoadEventPlan::NonBlockingModulepreload { element } => {
                let owner = self.accept_current_main_modulepreload_event_owner(element)?;
                Some(ConnectedStyleLoadEventAdmission::NonBlockingModulepreload(
                    owner,
                ))
            }
        }
    }

    pub(crate) fn accept_current_main_modulepreload_event_owner(
        &self,
        element: crate::document_runtime::DomHandle,
    ) -> Option<DocumentLinkEventOwner> {
        let owner = self.current_main_document_task_owner()?;
        self.frame_owner_store
            .accept_current_main_modulepreload_event_owner(owner, element)
    }

    pub(crate) fn main_style_load_event_is_current(
        &self,
        binding: MainDocumentStyleLoadEventBinding,
    ) -> bool {
        self.frame_owner_store
            .main_style_load_event_is_current(binding)
    }

    pub(crate) fn settle_main_style_load_event(
        &mut self,
        binding: MainDocumentStyleLoadEventBinding,
    ) -> bool {
        self.frame_owner_store.settle_main_style_load_event(binding)
    }

    pub(crate) fn accept_current_main_image_load_delay(
        &mut self,
        element: crate::document_runtime::DomHandle,
    ) -> Option<MainDocumentImageLoadDelayBinding> {
        let owner = self.current_main_document_task_owner()?;
        self.frame_owner_store
            .accept_current_main_image_load_delay(owner, element)
    }

    pub(crate) fn main_image_load_delay_is_current(
        &self,
        binding: MainDocumentImageLoadDelayBinding,
    ) -> bool {
        self.frame_owner_store
            .main_image_load_delay_is_current(binding)
    }

    pub(crate) fn settle_main_image_load_delay(
        &mut self,
        binding: MainDocumentImageLoadDelayBinding,
    ) -> bool {
        self.frame_owner_store.settle_main_image_load_delay(binding)
    }

    pub(crate) fn accept_current_main_media_load_delay(
        &mut self,
        element: crate::document_runtime::DomHandle,
    ) -> Option<MainDocumentMediaLoadDelayBinding> {
        let owner = self.current_main_document_task_owner()?;
        self.frame_owner_store
            .accept_current_main_media_load_delay(owner, element)
    }

    pub(crate) fn main_media_load_delay_is_current(
        &self,
        binding: MainDocumentMediaLoadDelayBinding,
    ) -> bool {
        self.frame_owner_store
            .main_media_load_delay_is_current(binding)
    }

    pub(crate) fn settle_main_media_load_delay(
        &mut self,
        binding: MainDocumentMediaLoadDelayBinding,
    ) -> bool {
        self.frame_owner_store.settle_main_media_load_delay(binding)
    }

    pub(crate) fn current_main_document_has_async_script_load_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .current_main_document_has_async_script_load_delay(owner)
    }

    #[cfg(test)]
    pub(crate) fn current_main_document_has_style_load_event_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .current_main_document_has_style_load_event_delay(owner)
    }

    #[cfg(test)]
    pub(crate) fn current_main_document_has_parser_deferred_script_load_delay(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .current_main_document_has_parser_deferred_script_load_delay(owner)
    }

    pub(crate) fn apply_current_main_document_interactive_transition(
        &mut self,
        action: MainDocumentInteractiveLifecycleAction,
    ) -> bool {
        self.frame_owner_store
            .apply_current_main_document_interactive_transition(action)
    }

    pub(crate) fn prepare_current_main_document_domcontentloaded_transition(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentDomContentLoadedLifecycleAction> {
        self.frame_owner_store
            .prepare_current_main_document_domcontentloaded_transition(owner)
    }

    pub(crate) fn current_main_document_domcontentloaded_transition_is_ready(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .current_main_document_domcontentloaded_transition_is_ready(owner)
    }

    pub(crate) fn apply_current_main_document_domcontentloaded_transition(
        &mut self,
        action: MainDocumentDomContentLoadedLifecycleAction,
    ) -> bool {
        self.frame_owner_store
            .apply_current_main_document_domcontentloaded_transition(action)
    }

    pub(crate) fn prepare_current_main_document_complete_transition(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentCompleteLifecycleAction> {
        self.frame_owner_store
            .prepare_current_main_document_complete_transition(owner)
    }

    pub(crate) fn current_main_document_complete_transition_is_ready(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .current_main_document_complete_transition_is_ready(owner)
    }

    pub(crate) fn stop_current_main_document_loading(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self.frame_owner_store
            .stop_current_main_document_loading(owner)
    }

    pub(crate) fn apply_current_main_document_complete_transition(
        &mut self,
        action: MainDocumentCompleteLifecycleAction,
    ) -> bool {
        self.frame_owner_store
            .apply_current_main_document_complete_transition(action)
    }

    pub(crate) fn begin_current_main_document_load_dispatch(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store
            .begin_current_main_document_load_dispatch(owner)
    }

    pub(crate) fn finish_current_main_document_load_dispatch(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        self.frame_owner_store
            .finish_current_main_document_load_dispatch(owner)
    }

    pub(crate) fn finish_current_main_document_load_after_descendant_completion(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        self.frame_owner_store
            .finish_current_main_document_load_after_descendant_completion(owner)
    }

    pub(crate) fn current_main_document_load_has_dispatched(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store
            .current_main_document_load_has_dispatched(owner)
    }

    pub(crate) fn current_main_document_load_completion_state(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        self.frame_owner_store
            .current_main_document_load_completion_state(owner)
    }
}
