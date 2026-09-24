use crate::{
    module_runtime::{DynamicModuleImportOwner, ModuleGraphFetchedSource},
    types::SharedNavigationResponseResult,
};

use url::Url;

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
    request_url: Url,
}

impl MainDynamicImportGraphFetchCompletion {
    pub(crate) fn new(
        target: MainDynamicImportGraphFetchTarget,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        request_url: Url,
    ) -> Self {
        Self {
            target,
            result,
            network_result,
            request_url,
        }
    }

    pub(crate) fn target(&self) -> MainDynamicImportGraphFetchTarget {
        self.target
    }

    pub(crate) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.network_result.as_ref()
    }

    pub(crate) fn request_url(&self) -> &Url {
        &self.request_url
    }

    pub(crate) fn into_result(self) -> std::result::Result<ModuleGraphFetchedSource, String> {
        self.result
    }
}
