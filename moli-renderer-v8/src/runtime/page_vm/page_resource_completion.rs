use anyhow::Result;

use crate::page_resource_completion::{
    PageResourceCompletionDocumentEffect, PageResourceCompletionTurnAction,
    PageResourceCompletionTurnOutcome, RendererPageResourceCompletion,
    RendererPageResourceTerminal,
};

use super::PageVm;

mod async_subresource;
mod child_document;
mod child_module;
mod document_load;
mod exact_owner;
mod main_document;
mod main_module;

pub(crate) use child_module::AuthorizedCurrentChildModuleFetchCompletion;
pub(crate) use document_load::{
    AuthorizedCurrentChildDocumentLoadCompletion, CurrentChildDocumentLoadApplication,
};
pub(crate) use main_document::AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion;
pub(crate) use main_module::{
    AuthorizedCurrentMainDynamicImportGraphFetchCompletion,
    AuthorizedCurrentMainParserModuleGraphFetchCompletion,
    AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion,
    AuthorizedLiveMainModulepreloadFetchCompletion,
};

impl PageVm {
    /// Thin typed dispatcher for one task already selected by the stable Page
    /// scheduler. Exact-owner authorization, Network policy, Document
    /// application and follow-up construction belong to the lane modules.
    fn apply_page_resource_completion_owner_action(
        &mut self,
        completion: RendererPageResourceCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let owner = completion.owner();
        let source = completion.activity_source();
        let action = match completion.into_terminal() {
            RendererPageResourceTerminal::DocumentWriteExternalScript { completion } => {
                self.apply_document_write_external_script_terminal(source, owner, completion)?
            }
            RendererPageResourceTerminal::MainParserDeferredClassicSource {
                completion,
                network_attribution,
            } => self.apply_main_parser_deferred_classic_source_terminal(
                source,
                owner,
                completion,
                network_attribution,
            )?,
            RendererPageResourceTerminal::MainParserModuleGraphFetch { completion } => {
                self.apply_main_parser_module_graph_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::MainRuntimeModuleGraphFetch { completion } => {
                self.apply_main_runtime_module_graph_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::MainDynamicImportGraphFetch { completion } => {
                self.apply_main_dynamic_import_graph_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::MainModulepreloadFetch { completion } => {
                self.apply_main_modulepreload_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::AsyncSubresource { event } => {
                self.apply_async_subresource_terminal(source, owner, *event)?
            }
            RendererPageResourceTerminal::ChildClassicScript { completion } => {
                self.apply_child_classic_script_terminal(source, owner, completion)?
            }
            RendererPageResourceTerminal::ChildBlockingStylesheet { completion } => {
                self.apply_child_blocking_stylesheet_terminal(source, owner, completion)?
            }
            RendererPageResourceTerminal::ChildParserModuleRootFetch { completion } => {
                self.apply_child_parser_module_root_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::ChildModuleDependencyFetch { completion } => {
                self.apply_child_module_dependency_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::ChildDynamicImportFetch { completion } => {
                self.apply_child_dynamic_import_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::ChildModulepreloadFetch { completion } => {
                self.apply_child_modulepreload_fetch_terminal(source, owner, *completion)?
            }
            RendererPageResourceTerminal::ChildDocumentLoad { completion } => {
                self.apply_child_document_load_terminal(source, owner, *completion)?
            }
        };

        if matches!(
            action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
        ) {
            tracing::debug!(
                ?source,
                ?owner,
                document_effect = ?action.document_effect,
                output_effect = ?action.output_effect,
                "discarded stale exact-owner Document effect from page resource completion"
            );
        }
        Ok(action)
    }

    fn settle_page_resource_completion_body(
        &self,
        action: PageResourceCompletionTurnAction,
    ) -> PageResourceCompletionTurnOutcome {
        PageResourceCompletionTurnOutcome::new(action)
    }

    pub(in crate::runtime) fn apply_selected_page_resource_completion_turn(
        &mut self,
        completion: RendererPageResourceCompletion,
    ) -> Result<PageResourceCompletionTurnOutcome> {
        let action = self.apply_page_resource_completion_owner_action(completion)?;
        Ok(self.settle_page_resource_completion_body(action))
    }

    /// Apply one low-level resource terminal for a two-stage admission test.
    ///
    /// This deliberately stops after the resource owner transition and any
    /// typed successor publication. It does not complete an HTML task,
    /// checkpoint V8, or reconcile child/runtime follow-up. Complete Page
    /// workflows must select `ResourceCompletion` through the production Page
    /// dispatcher instead.
    #[cfg(test)]
    pub(in crate::runtime) fn apply_one_page_resource_terminal_owner_admission_for_test(
        &mut self,
        queue: &mut impl crate::page_resource_completion::RendererPageResourceCompletionTestSource,
    ) -> Result<Option<PageResourceCompletionTurnOutcome>> {
        let Some(candidate) = queue.next_ready_metadata() else {
            return Ok(None);
        };
        let (actual, completion) = queue
            .pop_front()
            .expect("selected test resource completion must remain queued");
        debug_assert_eq!(actual, candidate);
        self.apply_selected_page_resource_completion_turn(completion)
            .map(Some)
    }
}
