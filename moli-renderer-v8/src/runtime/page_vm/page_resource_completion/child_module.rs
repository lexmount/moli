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
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            ));
        }

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
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            ));
        }

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
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            ));
        }

        let _followup = self
            .vm_mut()
            .apply_current_child_dynamic_import_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            )?;
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::None,
        ))
    }

    pub(super) fn apply_child_modulepreload_fetch_terminal(
        &mut self,
        source: RendererOwnerResourceActivitySource,
        owner: RendererPageResourceCompletionOwner,
        completion: ChildModulepreloadFetchCompletion,
    ) -> Result<PageResourceCompletionTurnAction> {
        let current_owner = self.current_page_resource_completion_owner(owner);
        if current_owner != Some(owner) {
            return Ok(PageResourceCompletionTurnAction::discarded_stale(
                source,
                owner,
                current_owner,
                PageResourceCompletionOutputEffect::None,
            ));
        }

        let _followup = self
            .vm_mut()
            .apply_current_child_modulepreload_fetch_completion(
                AuthorizedCurrentChildModuleFetchCompletion::new(completion),
            );
        Ok(PageResourceCompletionTurnAction::applied(
            source,
            owner,
            PageResourceCompletionOutputEffect::None,
        ))
    }
}
