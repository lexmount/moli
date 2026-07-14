use std::{future::Future, pin::Pin};

use anyhow::Result;
use url::Url;

use crate::{
    document_script_scheduler::{
        PageOwnedDocumentScriptBodyExecution, PageOwnedDocumentScriptHooks,
        PageOwnedDocumentScriptSourceFailure,
    },
    dom::native::DocumentReadyState,
    dynamic_script_owner::DynamicScriptPageTaskClaim,
    frame_owner_model::{FrameDocumentTaskOwner, MainDocumentScriptLoadDelayLease},
    module_script_continuation::ModuleScriptCompletionOwner,
    network::ResourceRequestClient,
    planning::PreparedScript,
    protocol_types::NavigationResponse,
};

use super::{
    PageOwnedScriptFailureClassification, PageVm,
    complete_page_owned_prepared_script_execution_failure_body,
    complete_prepared_script_execution_failure_report_with_activity,
    execute_prepared_script_on_script_execution_lane,
};

pub(super) struct MainPageOwnedDocumentScriptHooks<'page, 'loader> {
    page_vm: &'page mut PageVm,
    loader: &'loader ResourceRequestClient,
}

impl<'page, 'loader> MainPageOwnedDocumentScriptHooks<'page, 'loader> {
    pub(super) fn new(page_vm: &'page mut PageVm, loader: &'loader ResourceRequestClient) -> Self {
        Self { page_vm, loader }
    }
}

impl PageOwnedDocumentScriptHooks for MainPageOwnedDocumentScriptHooks<'_, '_> {
    type DocumentOwnerToken = FrameDocumentTaskOwner;

    fn current_document_owner_token(&self) -> Option<Self::DocumentOwnerToken> {
        self.page_vm.vm().current_main_document_task_owner()
    }

    fn set_loading_ready_state(&mut self) -> Result<()> {
        self.page_vm
            .vm_mut()
            .set_document_ready_state(DocumentReadyState::Loading)
    }

    fn record_script_source_network_result(
        &mut self,
        initiator_url: Url,
        script_url: Url,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
        network_result: &std::result::Result<NavigationResponse, String>,
    ) {
        self.page_vm
            .vm_mut()
            .record_script_subresource_network_result_with_initiator(
                initiator_url,
                script_url,
                request_initiator_type,
                network_result,
            );
    }

    fn perform_pre_script_checkpoint(&mut self, script_url: &Url) -> Result<()> {
        self.page_vm
            .vm_mut()
            .perform_script_task_checkpoint(Some(script_url))
    }

    fn begin_classic_defer_timer_schedule_range(&mut self) {
        self.page_vm
            .vm_mut()
            .document_runtime
            .begin_classic_defer_timer_schedule_range();
    }

    fn finish_classic_defer_timer_schedule_range(&mut self) {
        self.page_vm
            .vm_mut()
            .document_runtime
            .finish_classic_defer_timer_schedule_range();
    }

    fn execute_prepared_script<'a>(
        &'a mut self,
        script: PreparedScript,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    ) -> Pin<Box<dyn Future<Output = PageOwnedDocumentScriptBodyExecution> + 'a>> {
        Box::pin(async move {
            let local_executor = self.page_vm.local_executor.clone();
            let script_execution_disabled = !self.page_vm.main_document_scripting_enabled();
            let outcome = execute_prepared_script_on_script_execution_lane(
                &local_executor,
                self.loader,
                self.page_vm.vm_mut(),
                script,
                runtime_script_claim,
                script_execution_disabled,
            )
            .await;
            outcome.into_body_execution()
        })
    }

    fn complete_async_source_failure(
        &mut self,
        script: PreparedScript,
        failure: PageOwnedDocumentScriptSourceFailure,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    ) -> PageOwnedDocumentScriptBodyExecution {
        let (error, module_failure_policy, error_constructor) = failure.into_parts();
        if let Some(claim) = runtime_script_claim {
            let terminal_activity = self
                .page_vm
                .vm_mut()
                .finish_claimed_runtime_owned_script_failure_body(
                    claim,
                    &script,
                    &error,
                    module_failure_policy,
                    error_constructor,
                );
            return complete_prepared_script_execution_failure_report_with_activity(
                script,
                error,
                crate::script_vm::PreparedScriptBodyActivity::NotEntered,
            )
            .note_terminal_activity(terminal_activity)
            .into_body_execution();
        }
        let outcome = complete_page_owned_prepared_script_execution_failure_body(
            self.page_vm.vm_mut(),
            script,
            ModuleScriptCompletionOwner::Parser,
            None,
            error,
            PageOwnedScriptFailureClassification::LegacyMessageText,
            crate::script_vm::PreparedScriptBodyActivity::NotEntered,
        );
        outcome.into_body_execution()
    }

    fn queue_script_load_delay_settlement(
        &mut self,
        script: &PreparedScript,
        binding: MainDocumentScriptLoadDelayLease,
    ) {
        let owner = binding.owner();
        let kind = binding.kind();
        let load_delay_token = binding.load_delay_token();
        let disposition = self
            .page_vm
            .vm_mut()
            .enqueue_main_document_script_load_delay_settlement_best_effort(script, binding);
        tracing::debug!(
            ?owner,
            ?kind,
            ?load_delay_token,
            script_node_id = ?script.node_id,
            script_url = %script.url,
            ?disposition,
            "published main async script lifecycle settlement follow-up"
        );
    }
}
