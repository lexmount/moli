use crate::dom::native::NativeNodeId;
use crate::frame_owner_model::{
    ChildDocumentModuleFetchTarget, ChildDocumentNavigationFetchTarget, FrameDocumentTaskOwner,
};
use crate::runtime::RendererDocumentToken;
use crate::types::AsyncSubresourceFetchEventTarget;
use crate::types::DocumentWriteExternalScriptFetchTarget;

use super::{
    MainDynamicImportGraphFetchTarget, MainModulepreloadFetchTarget,
    MainParserModuleGraphFetchTarget, MainRuntimeModuleGraphFetchTarget,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageResourceCompletionLocalOwner {
    MainDocument(FrameDocumentTaskOwner),
    MainParserModuleGraphFetch(MainParserModuleGraphFetchTarget),
    MainRuntimeModuleGraphFetch(MainRuntimeModuleGraphFetchTarget),
    MainDynamicImportGraphFetch(MainDynamicImportGraphFetchTarget),
    MainModulepreloadFetch(MainModulepreloadFetchTarget),
    AsyncSubresource(AsyncSubresourceFetchEventTarget),
    DocumentWriteExternalScript(DocumentWriteExternalScriptFetchTarget),
    ChildDocument {
        child_handle: NativeNodeId,
        owner: FrameDocumentTaskOwner,
    },
    ChildModuleFetch(ChildDocumentModuleFetchTarget),
    ChildDocumentNavigation(ChildDocumentNavigationFetchTarget),
}

/// Exact owner of a completion stored in the stable Page queue.
///
/// `FrameDocumentTaskOwner` is only unique inside one `PageVm`: cross-Document
/// replacement constructs a new frame-owner store whose counters restart at
/// zero. The root renderer Document token namespaces those local identities
/// across PageVm replacement. Within one PageVm, `document.open()` keeps the
/// root token but advances the local document owner, so the pair remains exact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererPageResourceCompletionOwner {
    root_document: RendererDocumentToken,
    local_owner: RendererPageResourceCompletionLocalOwner,
}

impl RendererPageResourceCompletionOwner {
    pub(crate) fn main_document(
        root_document: RendererDocumentToken,
        owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::MainDocument(owner),
        }
    }

    pub(crate) fn child_document(
        root_document: RendererDocumentToken,
        child_handle: NativeNodeId,
        owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::ChildDocument {
                child_handle,
                owner,
            },
        }
    }

    pub(crate) fn main_parser_module_graph_fetch(
        root_document: RendererDocumentToken,
        target: MainParserModuleGraphFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::MainParserModuleGraphFetch(
                target,
            ),
        }
    }

    pub(crate) fn main_runtime_module_graph_fetch(
        root_document: RendererDocumentToken,
        target: MainRuntimeModuleGraphFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::MainRuntimeModuleGraphFetch(
                target,
            ),
        }
    }

    pub(crate) fn main_modulepreload_fetch(
        root_document: RendererDocumentToken,
        target: MainModulepreloadFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::MainModulepreloadFetch(target),
        }
    }

    pub(crate) fn main_dynamic_import_graph_fetch(
        root_document: RendererDocumentToken,
        target: MainDynamicImportGraphFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::MainDynamicImportGraphFetch(
                target,
            ),
        }
    }

    pub(crate) fn async_subresource(
        root_document: RendererDocumentToken,
        target: AsyncSubresourceFetchEventTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::AsyncSubresource(target),
        }
    }

    pub(crate) fn document_write_external_script(
        root_document: RendererDocumentToken,
        target: DocumentWriteExternalScriptFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::DocumentWriteExternalScript(
                target,
            ),
        }
    }

    pub(crate) fn child_module_fetch(
        root_document: RendererDocumentToken,
        target: ChildDocumentModuleFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::ChildModuleFetch(target),
        }
    }

    pub(crate) fn child_document_navigation(
        root_document: RendererDocumentToken,
        target: ChildDocumentNavigationFetchTarget,
    ) -> Self {
        Self {
            root_document,
            local_owner: RendererPageResourceCompletionLocalOwner::ChildDocumentNavigation(target),
        }
    }

    pub(crate) fn root_document(self) -> RendererDocumentToken {
        self.root_document
    }

    pub(crate) fn local_owner(self) -> RendererPageResourceCompletionLocalOwner {
        self.local_owner
    }
}
