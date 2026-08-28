use crate::{
    page_resource_completion::{
        RendererPageResourceCompletionLocalOwner, RendererPageResourceCompletionOwner,
    },
    runtime::RendererDocumentToken,
};

use super::ScriptVm;

impl ScriptVm {
    /// Projects the complete owner currently installed for the same local
    /// target shape as `expected`.
    ///
    /// `ScriptVm` owns the local target identities, while the caller owns the
    /// root-Document namespace. Keeping this projection here lets production
    /// Page execution and low-level test drivers share the same stale-owner
    /// decision instead of reimplementing authorization around terminal
    /// application.
    pub(crate) fn current_page_resource_completion_owner_for_root(
        &self,
        root_document: RendererDocumentToken,
        expected: RendererPageResourceCompletionOwner,
    ) -> Option<RendererPageResourceCompletionOwner> {
        match expected.local_owner() {
            RendererPageResourceCompletionLocalOwner::MainDocument(_) => {
                self.current_main_document_task_owner().map(|owner| {
                    RendererPageResourceCompletionOwner::main_document(root_document, owner)
                })
            }
            RendererPageResourceCompletionLocalOwner::MainParserModuleGraphFetch(target) => self
                .main_parser_module_graph_fetch_target_is_current(target)
                .then(|| {
                    RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                        root_document,
                        target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(target) => self
                .main_runtime_module_graph_fetch_target_is_current(target)
                .then(|| {
                    RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
                        root_document,
                        target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::MainDynamicImportGraphFetch(target) => self
                .current_main_dynamic_import_graph_fetch_target(target.load_id())
                .map(|current_target| {
                    RendererPageResourceCompletionOwner::main_dynamic_import_graph_fetch(
                        root_document,
                        current_target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(target) => self
                .current_main_modulepreload_fetch_target(target.load_id())
                .map(|current_target| {
                    RendererPageResourceCompletionOwner::main_modulepreload_fetch(
                        root_document,
                        current_target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::AsyncSubresource(target) => self
                .async_subresource_fetch_event_target_is_current(target)
                .then(|| {
                    RendererPageResourceCompletionOwner::async_subresource(root_document, target)
                }),
            RendererPageResourceCompletionLocalOwner::DocumentWriteExternalScript(target) => self
                .document_write_external_script_fetch_target_is_current(target)
                .then(|| {
                    RendererPageResourceCompletionOwner::document_write_external_script(
                        root_document,
                        target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::ChildDocument { child_handle, .. } => self
                .current_child_document_task_owner(child_handle)
                .map(|owner| {
                    RendererPageResourceCompletionOwner::child_document(
                        root_document,
                        child_handle,
                        owner,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::ChildModuleFetch(target) => self
                .current_child_document_module_fetch_target(target.child_handle())
                .map(|current_target| {
                    RendererPageResourceCompletionOwner::child_module_fetch(
                        root_document,
                        current_target,
                    )
                }),
            RendererPageResourceCompletionLocalOwner::ChildDocumentNavigation(target) => self
                .current_child_document_navigation_fetch_target(target.child_handle())
                .map(|current_target| {
                    RendererPageResourceCompletionOwner::child_document_navigation(
                        root_document,
                        current_target,
                    )
                }),
        }
    }
}
