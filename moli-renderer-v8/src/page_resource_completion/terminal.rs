use crate::module_script_continuation::MainParserDeferredClassicSourceLoadCompletion;
use crate::runtime::{RendererDocumentToken, RendererOwnerResourceActivitySource};
use crate::types::{
    AsyncSubresourceFetchEvent, ChildBlockingStylesheetLoadCompletion,
    ChildClassicScriptLoadCompletion, ChildDocumentLoadCompletion,
    ChildDynamicImportFetchCompletion, ChildModuleDependencyFetchCompletion,
    ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion,
    DocumentWriteExternalScriptLoadCompletion,
};

use super::{
    MainDynamicImportGraphFetchCompletion, MainModulepreloadFetchCompletion,
    MainParserModuleGraphFetchCompletion, MainRuntimeModuleGraphFetchCompletion,
    RendererPageResourceCompletionOwner,
};

#[derive(Clone, Debug)]
pub(crate) struct MainParserDeferredClassicSourceNetworkAttribution {
    document_url: url::Url,
    request_url: url::Url,
}

impl MainParserDeferredClassicSourceNetworkAttribution {
    pub(crate) fn new(document_url: url::Url, request_url: url::Url) -> Self {
        Self {
            document_url,
            request_url,
        }
    }

    pub(crate) fn document_url(&self) -> &url::Url {
        &self.document_url
    }

    pub(crate) fn request_url(&self) -> &url::Url {
        &self.request_url
    }
}

/// A typed native/network terminal whose executable payload is owned by the
/// Document that created it.
#[derive(Debug)]
pub(crate) enum RendererPageResourceTerminal {
    DocumentWriteExternalScript {
        completion: DocumentWriteExternalScriptLoadCompletion,
    },
    MainParserDeferredClassicSource {
        completion: MainParserDeferredClassicSourceLoadCompletion,
        network_attribution: MainParserDeferredClassicSourceNetworkAttribution,
    },
    MainParserModuleGraphFetch {
        completion: Box<MainParserModuleGraphFetchCompletion>,
    },
    MainRuntimeModuleGraphFetch {
        completion: Box<MainRuntimeModuleGraphFetchCompletion>,
    },
    MainDynamicImportGraphFetch {
        completion: Box<MainDynamicImportGraphFetchCompletion>,
    },
    MainModulepreloadFetch {
        completion: Box<MainModulepreloadFetchCompletion>,
    },
    AsyncSubresource {
        event: Box<AsyncSubresourceFetchEvent>,
    },
    ChildClassicScript {
        completion: ChildClassicScriptLoadCompletion,
    },
    ChildBlockingStylesheet {
        completion: ChildBlockingStylesheetLoadCompletion,
    },
    ChildParserModuleRootFetch {
        completion: Box<ChildParserModuleRootFetchCompletion>,
    },
    ChildModuleDependencyFetch {
        completion: Box<ChildModuleDependencyFetchCompletion>,
    },
    ChildDynamicImportFetch {
        completion: Box<ChildDynamicImportFetchCompletion>,
    },
    ChildModulepreloadFetch {
        completion: Box<ChildModulepreloadFetchCompletion>,
    },
    ChildDocumentLoad {
        completion: Box<ChildDocumentLoadCompletion>,
    },
}

/// Stable-queue envelope for one typed resource terminal.
///
/// The root Document namespace is mandatory for every terminal, while the
/// terminal payload remains the single source of its PageVm-local owner. This
/// prevents a newly migrated lane from omitting the cross-PageVm namespace or
/// storing a second, independently drifting local owner.
#[derive(Debug)]
pub(crate) struct RendererPageResourceCompletion {
    root_document: RendererDocumentToken,
    terminal: RendererPageResourceTerminal,
}

impl RendererPageResourceCompletion {
    pub(crate) fn document_write_external_script(
        root_document: RendererDocumentToken,
        completion: DocumentWriteExternalScriptLoadCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::DocumentWriteExternalScript { completion },
        }
    }

    pub(crate) fn main_parser_deferred_classic_source(
        root_document: RendererDocumentToken,
        completion: MainParserDeferredClassicSourceLoadCompletion,
        network_attribution: MainParserDeferredClassicSourceNetworkAttribution,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::MainParserDeferredClassicSource {
                completion,
                network_attribution,
            },
        }
    }

    pub(crate) fn main_parser_module_graph_fetch(
        root_document: RendererDocumentToken,
        completion: MainParserModuleGraphFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::MainParserModuleGraphFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn main_runtime_module_graph_fetch(
        root_document: RendererDocumentToken,
        completion: MainRuntimeModuleGraphFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::MainRuntimeModuleGraphFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn main_dynamic_import_graph_fetch(
        root_document: RendererDocumentToken,
        completion: MainDynamicImportGraphFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::MainDynamicImportGraphFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn main_modulepreload_fetch(
        root_document: RendererDocumentToken,
        completion: MainModulepreloadFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::MainModulepreloadFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn async_subresource(
        root_document: RendererDocumentToken,
        event: AsyncSubresourceFetchEvent,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::AsyncSubresource {
                event: Box::new(event),
            },
        }
    }

    pub(crate) fn child_blocking_stylesheet(
        root_document: RendererDocumentToken,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildBlockingStylesheet { completion },
        }
    }

    pub(crate) fn child_classic_script(
        root_document: RendererDocumentToken,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildClassicScript { completion },
        }
    }

    pub(crate) fn child_parser_module_root_fetch(
        root_document: RendererDocumentToken,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildParserModuleRootFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn child_module_dependency_fetch(
        root_document: RendererDocumentToken,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildModuleDependencyFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn child_modulepreload_fetch(
        root_document: RendererDocumentToken,
        completion: ChildModulepreloadFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildModulepreloadFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn child_dynamic_import_fetch(
        root_document: RendererDocumentToken,
        completion: ChildDynamicImportFetchCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildDynamicImportFetch {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn child_document_load(
        root_document: RendererDocumentToken,
        completion: ChildDocumentLoadCompletion,
    ) -> Self {
        Self {
            root_document,
            terminal: RendererPageResourceTerminal::ChildDocumentLoad {
                completion: Box::new(completion),
            },
        }
    }

    pub(crate) fn owner(&self) -> RendererPageResourceCompletionOwner {
        match &self.terminal {
            RendererPageResourceTerminal::DocumentWriteExternalScript { completion } => {
                RendererPageResourceCompletionOwner::document_write_external_script(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::MainParserDeferredClassicSource {
                completion, ..
            } => RendererPageResourceCompletionOwner::main_document(
                self.root_document,
                completion.pending_script_id().owner().task_owner(),
            ),
            RendererPageResourceTerminal::MainParserModuleGraphFetch { completion } => {
                RendererPageResourceCompletionOwner::main_parser_module_graph_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::MainRuntimeModuleGraphFetch { completion } => {
                RendererPageResourceCompletionOwner::main_runtime_module_graph_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::MainDynamicImportGraphFetch { completion } => {
                RendererPageResourceCompletionOwner::main_dynamic_import_graph_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::MainModulepreloadFetch { completion } => {
                RendererPageResourceCompletionOwner::main_modulepreload_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::AsyncSubresource { event } => {
                RendererPageResourceCompletionOwner::async_subresource(
                    self.root_document,
                    event.target(),
                )
            }
            RendererPageResourceTerminal::ChildClassicScript { completion } => {
                RendererPageResourceCompletionOwner::child_document(
                    self.root_document,
                    completion.handle,
                    completion.owner,
                )
            }
            RendererPageResourceTerminal::ChildBlockingStylesheet { completion } => {
                RendererPageResourceCompletionOwner::child_document(
                    self.root_document,
                    completion.child_handle,
                    completion.owner,
                )
            }
            RendererPageResourceTerminal::ChildParserModuleRootFetch { completion } => {
                RendererPageResourceCompletionOwner::child_module_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::ChildModuleDependencyFetch { completion } => {
                RendererPageResourceCompletionOwner::child_module_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::ChildDynamicImportFetch { completion } => {
                RendererPageResourceCompletionOwner::child_module_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::ChildModulepreloadFetch { completion } => {
                RendererPageResourceCompletionOwner::child_module_fetch(
                    self.root_document,
                    completion.target(),
                )
            }
            RendererPageResourceTerminal::ChildDocumentLoad { completion } => {
                RendererPageResourceCompletionOwner::child_document_navigation(
                    self.root_document,
                    completion.target(),
                )
            }
        }
    }

    pub(crate) fn activity_source(&self) -> RendererOwnerResourceActivitySource {
        match &self.terminal {
            RendererPageResourceTerminal::DocumentWriteExternalScript { .. } => {
                RendererOwnerResourceActivitySource::DocumentWriteExternalScript
            }
            RendererPageResourceTerminal::MainParserDeferredClassicSource { .. } => {
                RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
            }
            RendererPageResourceTerminal::MainParserModuleGraphFetch { .. } => {
                RendererOwnerResourceActivitySource::ModuleGraphFetch
            }
            RendererPageResourceTerminal::MainRuntimeModuleGraphFetch { .. } => {
                RendererOwnerResourceActivitySource::ModuleGraphFetch
            }
            RendererPageResourceTerminal::MainDynamicImportGraphFetch { .. } => {
                RendererOwnerResourceActivitySource::ModuleGraphFetch
            }
            RendererPageResourceTerminal::MainModulepreloadFetch { .. } => {
                RendererOwnerResourceActivitySource::ModuleGraphFetch
            }
            RendererPageResourceTerminal::AsyncSubresource { .. } => {
                RendererOwnerResourceActivitySource::AsyncSubresource
            }
            RendererPageResourceTerminal::ChildClassicScript { .. } => {
                RendererOwnerResourceActivitySource::ChildClassicScript
            }
            RendererPageResourceTerminal::ChildBlockingStylesheet { .. } => {
                RendererOwnerResourceActivitySource::ChildBlockingStylesheet
            }
            RendererPageResourceTerminal::ChildParserModuleRootFetch { .. }
            | RendererPageResourceTerminal::ChildModuleDependencyFetch { .. }
            | RendererPageResourceTerminal::ChildDynamicImportFetch { .. }
            | RendererPageResourceTerminal::ChildModulepreloadFetch { .. } => {
                RendererOwnerResourceActivitySource::ModuleGraphFetch
            }
            RendererPageResourceTerminal::ChildDocumentLoad { .. } => {
                RendererOwnerResourceActivitySource::ChildDocument
            }
        }
    }

    pub(crate) fn into_terminal(self) -> RendererPageResourceTerminal {
        self.terminal
    }

    #[cfg(test)]
    pub(crate) fn terminal(&self) -> &RendererPageResourceTerminal {
        &self.terminal
    }
}
