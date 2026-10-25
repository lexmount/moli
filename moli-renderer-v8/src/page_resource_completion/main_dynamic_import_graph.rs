use crate::{
    module_runtime::{DynamicModuleImportOwner, ModuleGraphFetchedSource},
    types::SharedNavigationResponseResult,
};

use super::MainModuleFetchNetworkAttribution;

/// Exact PageVm-local owner of one main-Document dynamic-import graph fetch.
///
/// The dynamic-import resolver is the authority for both fields: `load_id`
/// identifies the suspended fetch, while `import_owner` captures the Document
/// snapshot and Window execution context that own the import promise. The
/// stable Page queue adds the producing root `RendererDocumentToken` so a
/// replacement PageVm cannot accept a naturally colliding local identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainDynamicImportGraphFetchTarget {
    import_owner: DynamicModuleImportOwner,
    load_id: u64,
}

impl MainDynamicImportGraphFetchTarget {
    pub(crate) fn new(import_owner: DynamicModuleImportOwner, load_id: u64) -> Self {
        Self {
            import_owner,
            load_id,
        }
    }

    pub(crate) fn import_owner(self) -> DynamicModuleImportOwner {
        self.import_owner
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }
}

/// One network terminal for an exact main-Document dynamic-import fetch.
#[derive(Debug)]
pub(crate) struct MainDynamicImportGraphFetchCompletion {
    target: MainDynamicImportGraphFetchTarget,
    result: std::result::Result<ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: MainModuleFetchNetworkAttribution,
    fetch_context: Option<std::sync::Arc<crate::module_runtime::NativeModuleFetchContext>>,
}

impl MainDynamicImportGraphFetchCompletion {
    pub(crate) fn new(
        target: MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: MainModuleFetchNetworkAttribution,
    ) -> Self {
        Self {
            target,
            result,
            network_result,
            network_attribution,
            fetch_context: None,
        }
    }

    pub(crate) fn with_fetch_context(
        mut self,
        context: Option<std::sync::Arc<crate::module_runtime::NativeModuleFetchContext>>,
    ) -> Self {
        self.fetch_context = context;
        self
    }

    pub(crate) fn fetch_context(
        &self,
    ) -> Option<&std::sync::Arc<crate::module_runtime::NativeModuleFetchContext>> {
        self.fetch_context.as_ref()
    }

    pub(crate) fn target(&self) -> MainDynamicImportGraphFetchTarget {
        self.target
    }

    pub(crate) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(crate) fn network_attribution(&self) -> &MainModuleFetchNetworkAttribution {
        &self.network_attribution
    }

    pub(crate) fn into_result(self) -> std::result::Result<ModuleGraphFetchedSource, String> {
        self.result
    }
}
