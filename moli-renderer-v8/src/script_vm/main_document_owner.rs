use super::{ScriptVm, input_helpers::clear_input_dispatch_state};
use crate::frame_owner_model::{
    FrameDocumentTaskOwner, MainDocumentInteractiveLifecycleAction, MainDocumentLoadCompletionState,
};

impl ScriptVm {
    pub(crate) fn queue_initial_connected_style_loads_for_current_owner(&mut self) {
        let prepared = self
            .document_runtime
            .prepare_initial_connected_style_loads();
        self.commit_and_apply_connected_style_loads(prepared);
    }

    fn commit_and_apply_connected_style_loads(
        &mut self,
        prepared_loads: Vec<crate::document_runtime::PreparedConnectedStyleLoad>,
    ) {
        // This is a synchronous owner-thread transaction. Preparing reads the
        // DOM, committing touches only ContextHost's FrameOwnerStore, and
        // applying starts/queues the resource operation. No V8 callback, task
        // checkpoint, or event-loop turn runs between those steps.
        for prepared in prepared_loads {
            let inline_source = self
                ._context_host
                .borrow()
                .owner_style_sheet_processing_source(prepared.owner());
            let event_admission = self
                ._context_host
                .borrow_mut()
                .commit_connected_style_load_event_plan(prepared.event_plan());
            let Some(event_admission) = event_admission else {
                tracing::debug!(
                    owner = ?prepared.owner(),
                    "discarded connected-style plan rejected by the current main Document"
                );
                continue;
            };
            let prime_result = self.document_runtime.apply_prepared_connected_style_load(
                prepared,
                inline_source,
                event_admission,
                self._context_host.as_ref().as_ptr(),
            );
            self.apply_connected_style_load_prime_result(prime_result);
        }
    }

    pub(crate) fn accept_main_document_blocking_stylesheet_inputs(
        &mut self,
        expected_owner: FrameDocumentTaskOwner,
        inputs: &[crate::stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput],
    ) -> usize {
        if self.current_main_document_task_owner() != Some(expected_owner) {
            tracing::debug!(
                ?expected_owner,
                current_owner = ?self.current_main_document_task_owner(),
                input_count = inputs.len(),
                "dropping main stylesheet discovery for stale parser owner"
            );
            return 0;
        }
        let completed_stylesheet_clients = self
            .document_runtime
            .note_discovered_document_owned_blocking_stylesheet_inputs(inputs.iter());
        self.settle_stylesheet_link_clients(completed_stylesheet_clients);
        inputs.len()
    }

    pub(crate) fn finish_current_main_document_parsing(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentInteractiveLifecycleAction> {
        let action = self
            ._context_host
            .borrow_mut()
            .finish_current_main_document_parsing(owner);
        if action.is_some() {
            self.document_runtime
                .clear_main_document_script_preload_store();
        }
        action
    }

    pub(super) fn apply_pending_main_document_owner_transitions(&mut self) {
        let transitions = self
            ._context_host
            .borrow_mut()
            .take_pending_main_document_owner_transitions();
        let Some(last_transition) = transitions.last().copied() else {
            return;
        };

        let mut previous_current = None;
        let mut rebound_isolated_world_count = 0;
        for transition in transitions.iter().copied() {
            if let Some(expected_retired) = previous_current
                && transition.retired_owner() != expected_retired
            {
                tracing::warn!(
                    ?expected_retired,
                    actual_retired = ?transition.retired_owner(),
                    current_owner = ?transition.current_owner(),
                    "main document owner transition journal contains a discontinuous replacement"
                );
            }
            tracing::debug!(
                retired_owner = ?transition.retired_owner(),
                current_owner = ?transition.current_owner(),
                "applying main document owner replacement at runtime turn boundary"
            );
            rebound_isolated_world_count += self
                .rebind_isolated_worlds_for_document_owner_transition(
                    transition.retired_owner(),
                    transition.current_owner(),
                );
            previous_current = Some(transition.current_owner());
        }

        let actual_current = self.current_main_document_task_owner();
        if actual_current != Some(last_transition.current_owner()) {
            tracing::warn!(
                journal_current = ?last_transition.current_owner(),
                ?actual_current,
                transition_count = transitions.len(),
                "main document owner transition journal ended at a non-current owner"
            );
        } else {
            tracing::debug!(
                retired_owner = ?transitions.first().map(|transition| transition.retired_owner()),
                current_owner = ?last_transition.current_owner(),
                transition_count = transitions.len(),
                rebound_isolated_world_count,
                "retired ScriptVm-local state for main document owner transition"
            );
        }

        // DocumentRuntime clears script/module owners synchronously inside the
        // replacement transaction. Layout services and downloadable fonts live
        // on the context host, while input dispatch lives on ScriptVm, so retire them here from the exact
        // owner-transition journal. Async font completion is separately bound
        // to the originating Window/document owner and exact request identity.
        self._context_host.borrow().reset_document_layout_state();
        clear_input_dispatch_state(self);
    }

    pub(crate) fn finish_main_document_load_after_descendant_completion(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        self._context_host
            .borrow_mut()
            .finish_current_main_document_load_after_descendant_completion(owner)
    }

    pub(crate) fn current_main_document_load_completion_state(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<MainDocumentLoadCompletionState> {
        self._context_host
            .borrow()
            .current_main_document_load_completion_state(owner)
    }
}
