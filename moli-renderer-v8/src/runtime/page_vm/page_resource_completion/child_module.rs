use anyhow::Result;

use crate::{
    page_resource_completion::{
        PageResourceCompletionOutputEffect, PageResourceCompletionTurnAction,
        RendererPageResourceCompletionOwner,
    },
    runtime::RendererOwnerResourceActivitySource,
    types::{
        ChildDynamicImportFetchCompletion, ChildModuleDependencyFetchCompletion,
        ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion,
    },
};

use super::super::PageVm;

/// Proof that the Page lane executor compared a child-module terminal against
/// the complete current `{root Document, child, task owner, realm}` target.
pub(crate) struct AuthorizedCurrentChildModuleFetchCompletion<T>(T);

impl<T> AuthorizedCurrentChildModuleFetchCompletion<T> {
    fn new(completion: T) -> Self {
        Self(completion)
    }

    pub(crate) fn into_completion(self) -> T {
        self.0
    }
}

impl PageVm {
    pub(super) fn apply_child_parser_module_root_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return Ok(self.discard_stale_child_module_terminal(
                source,
                owner,
                current_owner,
                completion.network_attribution(),
                completion.network_result(),
            ));
        }

        self.vm_mut()
            .record_current_child_module_fetch_network_result(
                completion.target(),
                completion.network_attribution(),
                completion.network_result(),
                None,
            );
        let _followup = self
            .vm_mut()
            .apply_current_child_parser_module_root_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            );
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::CaptureRequired,
        ))
    }

    pub(super) fn apply_child_module_dependency_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return Ok(self.discard_stale_child_module_terminal(
                source,
                owner,
                current_owner,
                completion.network_attribution(),
                completion.network_result(),
            ));
        }

        self.vm_mut()
            .record_current_child_module_fetch_network_result(
                completion.target(),
                completion.network_attribution(),
                completion.network_result(),
                None,
            );
        let _followup = self
            .vm_mut()
            .apply_current_child_module_dependency_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            );
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::CaptureRequired,
        ))
    }

    pub(super) fn apply_child_dynamic_import_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildDynamicImportFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return Ok(self.discard_stale_child_module_terminal(
                source,
                owner,
                current_owner,
                completion.network_attribution(),
                completion.network_result(),
            ));
        }

        let output_effect = PageResourceCompletionOutputEffect::capture_if(
            self.vm_mut()
                .record_current_child_module_fetch_network_result(
                    completion.target(),
                    completion.network_attribution(),
                    completion.network_result(),
                    None,
                ),
        );
        let _followup = self
            .vm_mut()
            .apply_current_child_dynamic_import_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            )?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            output_effect,
        ))
    }

    pub(super) fn apply_child_modulepreload_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildModulepreloadFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if let Some(context) = completion.fetch_context() {
            self.vm_mut()
                .report_module_fetch_context(context, current_owner == Some(owner));
        }
        if current_owner != Some(owner) {
            return Ok(self.discard_stale_child_module_terminal(
                source,
                owner,
                current_owner,
                completion.network_attribution(),
                completion.network_result(),
            ));
        }

        let entry = completion
            .fetch_context()
            .and_then(|context| context.take_performance_entry())
            .or_else(|| {
                completion.network_result().map(|result| {
                    crate::context_bootstrap::ResourcePerformanceEntry::from_network_result(
                        completion.network_attribution().request_url().as_str(),
                        "script",
                        None,
                        result.as_ref(),
                    )
                })
            });
        let output_effect = PageResourceCompletionOutputEffect::capture_if(
            self.vm_mut()
                .record_current_child_module_fetch_network_result(
                    completion.target(),
                    completion.network_attribution(),
                    completion.network_result(),
                    entry,
                ),
        );
        let _followup = self
            .vm_mut()
            .apply_current_child_modulepreload_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            );
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            output_effect,
        ))
    }

    fn discard_stale_child_module_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        current_owner: Option<RendererPageResourceCompletionOwner>,
        network_attribution: &crate::types::ChildModuleFetchNetworkAttribution,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> PageResourceCompletionTurnAction {
        let output_effect = PageResourceCompletionOutputEffect::capture_if(
            self.vm_mut()
                .record_historical_child_module_fetch_network_result(
                    network_attribution,
                    network_result,
                ),
        );
        PageResourceCompletionTurnAction::discarded_stale(
            source,
            owner,
            current_owner,
            output_effect,
        )
    }
}
