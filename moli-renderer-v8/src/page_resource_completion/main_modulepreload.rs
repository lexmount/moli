use crate::{
    frame_owner_model::FrameDocumentTaskOwner, module_runtime::ModuleGraphFetchedSource,
    types::SharedNavigationResponseResult,
};

use super::MainModuleFetchNetworkAttribution;

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
    network_attribution: MainModuleFetchNetworkAttribution,
    fetch_context: Option<std::sync::Arc<crate::module_runtime::NativeModuleFetchContext>>,
}

impl MainModulepreloadFetchCompletion {
    pub(crate) fn new(
        target: MainModulepreloadFetchTarget,
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

    pub(crate) fn target(&self) -> MainModulepreloadFetchTarget {
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
