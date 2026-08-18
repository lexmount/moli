use anyhow::Result;

use crate::{
    module_script_continuation::MainParserDeferredClassicSourceLoadCompletion,
    page_resource_completion::{
        MainParserDeferredClassicSourceNetworkAttribution, PageResourceCompletionOutputEffect,
        PageResourceCompletionTurnAction, RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
    types::DocumentWriteExternalScriptLoadCompletion,
};

use super::super::PageVm;

/// Proof that the Page lane executor matched one `document.write()` fetch
/// terminal against its complete current root/main-Document/load target.
pub(crate) struct AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion(
    DocumentWriteExternalScriptLoadCompletion,
);

impl AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion {
    fn new(completion: DocumentWriteExternalScriptLoadCompletion) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> DocumentWriteExternalScriptLoadCompletion {
        self.0
    }
}

impl PageVm {
    pub(super) fn apply_document_write_external_script_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: DocumentWriteExternalScriptLoadCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = if let Some(network_result) = completion.network_result() {
                self.vm_mut()
                    .record_historical_script_subresource_network_result(
                        completion.network_attribution().document_url().clone(),
                        completion.network_attribution().request_url().clone(),
                        network_result.as_ref(),
                    );
                PageResourceCompletionOutputEffect::CaptureRequired
            } else {
                PageResourceCompletionOutputEffect::None
            };
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        if let Some(network_result) = completion.network_result() {
            self.vm_mut().record_script_subresource_network_result(
                completion.network_attribution().document_url().clone(),
                completion.network_attribution().request_url().clone(),
                network_result.as_ref(),
            );
        }
        let application = self
            .vm_mut()
            .apply_current_document_write_external_script_load_completion(
                AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion::new(completion),
            )?;
        let action = match application {
            crate::document_runtime::DocumentWriteExternalScriptLoadApplication::Applied => {
                PageResourceCompletionTurnAction::applied_after_page_code(
                    source,
                    owner,
                    PageResourceCompletionOutputEffect::CaptureRequired,
                )
            }
            crate::document_runtime::DocumentWriteExternalScriptLoadApplication::SupersededDuringApplication => {
                PageResourceCompletionTurnAction::superseded_after_page_code(
                    source,
                    owner,
                    self.current_page_resource_completion_owner(owner),
                    PageResourceCompletionOutputEffect::CaptureRequired,
                )
            }
            crate::document_runtime::DocumentWriteExternalScriptLoadApplication::RejectedStaleTarget => {
                PageResourceCompletionTurnAction::current_owner_without_payload(
                    source,
                    owner,
                    PageResourceCompletionOutputEffect::CaptureRequired,
                )
            }
        };
        Ok(action)
    }

    pub(super) fn apply_main_parser_deferred_classic_source_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: MainParserDeferredClassicSourceLoadCompletion,
        network_attribution: MainParserDeferredClassicSourceNetworkAttribution,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            let output_effect = if let Some(network_result) = completion.network_result() {
                self.vm_mut()
                    .record_historical_script_subresource_network_result(
                        network_attribution.document_url().clone(),
                        network_attribution.request_url().clone(),
                        network_result.as_ref(),
                    );
                PageResourceCompletionOutputEffect::CaptureRequired
            } else {
                PageResourceCompletionOutputEffect::None
            };
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                output_effect,
            ));
        }

        self.vm_mut()
            .complete_main_parser_deferred_classic_source_load(completion);
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::CaptureRequired,
        ))
    }
}
