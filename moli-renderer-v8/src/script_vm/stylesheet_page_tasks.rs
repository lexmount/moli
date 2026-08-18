use super::ScriptVm;
use crate::{
    page_task_queue::{
        PageConnectedStyleEventTargetEffect, PageConnectedStyleLoadDelayEffect,
        PageStylesheetNetworkingTargetEffect, RendererPageConnectedStyleEventTask,
        RendererPageStylesheetCompletion, RendererPageStylesheetNetworkingTask,
        RendererPageStylesheetTaskOwner,
    },
    runtime::RendererDocumentToken,
};

impl ScriptVm {
    /// Project one stylesheet task owner onto the currently installed main
    /// Document.
    ///
    /// PageVm and low-level typed-source fixtures share this authorization
    /// boundary. Tests therefore cannot accidentally apply a stale task by
    /// unpacking its transport envelope themselves.
    pub(crate) fn stylesheet_task_owner_is_current(
        &self,
        root_document: RendererDocumentToken,
        owner: RendererPageStylesheetTaskOwner,
    ) -> bool {
        owner.root_document() == root_document
            && self.current_main_document_task_owner() == Some(owner.document_owner())
    }

    pub(crate) fn apply_page_stylesheet_networking_task(
        &mut self,
        root_document: RendererDocumentToken,
        task: RendererPageStylesheetNetworkingTask,
    ) -> PageStylesheetNetworkingTargetEffect {
        let owner = task.owner();
        let current = self.stylesheet_task_owner_is_current(root_document, owner);
        match task.into_completion() {
            RendererPageStylesheetCompletion::Blocking(completion) => self
                .document_runtime
                .apply_blocking_stylesheet_completion(completion),
            RendererPageStylesheetCompletion::Connected(completion) => self
                .document_runtime
                .apply_connected_style_load_completion(completion),
            RendererPageStylesheetCompletion::LiveImport(completion) => self
                .document_runtime
                .apply_live_stylesheet_import_load_completion(completion, current),
        }
        self.record_ready_stylesheet_network_results();
        if current {
            PageStylesheetNetworkingTargetEffect::AppliedToCurrentOwner
        } else {
            PageStylesheetNetworkingTargetEffect::RecordedForStaleOwner
        }
    }

    /// Apply the synchronous body of one selected connected style/link event.
    ///
    /// Dispatch and exact load-delay settlement belong to the DOM event task
    /// itself. The selected Page-task dispatcher owns the later agent
    /// checkpoint, child synchronization, and runtime follow-up.
    pub(crate) fn apply_page_connected_style_event_task_body(
        &mut self,
        root_document: RendererDocumentToken,
        task: RendererPageConnectedStyleEventTask,
    ) -> PageConnectedStyleEventTargetEffect {
        let owner = task.owner();
        if !self.stylesheet_task_owner_is_current(root_document, owner) {
            tracing::debug!(
                ?owner,
                "discarded stale exact-Document connected style event task"
            );
            return PageConnectedStyleEventTargetEffect::DiscardedStaleOwner;
        }
        let ready = task.into_ready();
        let binding = ready.load_event_binding();
        let dispatched = self.dispatch_connected_style_load(ready);
        let load_delay_effect = self.settle_connected_style_load(binding);
        if dispatched {
            PageConnectedStyleEventTargetEffect::DispatchedToCurrentOwner { load_delay_effect }
        } else {
            PageConnectedStyleEventTargetEffect::CurrentOwnerHadNoEvent { load_delay_effect }
        }
    }

    pub(crate) fn settle_connected_style_load(
        &mut self,
        binding: Option<crate::frame_owner_model::MainDocumentStyleLoadEventBinding>,
    ) -> PageConnectedStyleLoadDelayEffect {
        let Some(binding) = binding else {
            return PageConnectedStyleLoadDelayEffect::NoBindingRequired;
        };
        if !self
            ._context_host
            .borrow()
            .main_style_load_event_is_current(binding)
        {
            tracing::debug!(
                owner = ?binding.owner(),
                element = ?binding.element(),
                load_delay_token = ?binding.load_delay_token(),
                "left a connected style load binding with its retired Document"
            );
            return PageConnectedStyleLoadDelayEffect::ExactBindingRetiredWithDocument;
        }
        let had_load_delay_token = binding.load_delay_token().is_some();
        let settled = self
            ._context_host
            .borrow_mut()
            .settle_main_style_load_event(binding);
        assert!(
            settled,
            "a current connected style event must release its exact load-delay binding"
        );
        tracing::debug!(
            owner = ?binding.owner(),
            element = ?binding.element(),
            load_delay_token = ?binding.load_delay_token(),
            "settled main connected style load inside selected event body"
        );
        if had_load_delay_token {
            PageConnectedStyleLoadDelayEffect::ReleasedExactBinding
        } else {
            PageConnectedStyleLoadDelayEffect::NoBindingRequired
        }
    }
}
