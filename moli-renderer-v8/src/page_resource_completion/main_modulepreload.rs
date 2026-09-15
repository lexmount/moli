use crate::{
    frame_owner_model::FrameDocumentTaskOwner, module_runtime::ModuleGraphFetchedSource,
    types::SharedNavigationResponseResult,
};

use url::Url;

/// Exact PageVm-local owner of one main-Document modulepreload network fetch.
///
/// The load id is allocated by the originating Document's module-map owner and
/// identifies its one retained in-flight single-module request. A terminal is
/// not owned by one link client: several link/parser/module clients may join
/// the same module-map entry and are notified by later owner-event turns. The
/// stable Page envelope adds the producing root `RendererDocumentToken`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainModulepreloadFetchTarget {
    document_owner: FrameDocumentTaskOwner,
    load_id: u64,
}

impl MainModulepreloadFetchTarget {
    pub(crate) fn new(document_owner: FrameDocumentTaskOwner, load_id: u64) -> Self {
        Self {
            document_owner,
            load_id,
        }
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }
}

/// One network terminal for an exact main-Document modulepreload fetch.
#[derive(Debug)]
pub(crate) struct MainModulepreloadFetchCompletion {
    target: MainModulepreloadFetchTarget,
    result: std::result::Result<ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    request_url: Url,
}

impl MainModulepreloadFetchCompletion {
    pub(crate) fn new(
        target: MainModulepreloadFetchTarget,
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

    pub(crate) fn target(&self) -> MainModulepreloadFetchTarget {
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
