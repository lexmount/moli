use crate::{
    dynamic_script_owner::DynamicScriptOwnerId, frame_owner_model::FrameDocumentTaskOwner,
    module_runtime::ModuleGraphFetchedSource, types::SharedNavigationResponseResult,
};

use super::MainModuleFetchNetworkAttribution;

/// Exact PageVm-local owner of one runtime-created main-Document module fetch.
///
/// `document_owner` rejects same-Page `document.open()` replacement,
/// `dynamic_script_owner_id` identifies the script element that started the
/// graph, and `load_id` identifies one suspended module-map fetch. A shared
/// fetch may outlive the initiating element after another graph joins it, so
/// terminal authority does not require that element to remain resident. The
/// stable Page queue adds the producing root `RendererDocumentToken` to reject
/// PageVm replacement even when every local counter naturally collides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainRuntimeModuleGraphFetchTarget {
    document_owner: FrameDocumentTaskOwner,
    dynamic_script_owner_id: DynamicScriptOwnerId,
    load_id: u64,
}

impl MainRuntimeModuleGraphFetchTarget {
    pub(crate) fn new(
        document_owner: FrameDocumentTaskOwner,
        dynamic_script_owner_id: DynamicScriptOwnerId,
        load_id: u64,
    ) -> Self {
        Self {
            document_owner,
            dynamic_script_owner_id,
            load_id,
        }
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }

    pub(crate) fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }
}

/// One network terminal for an exact runtime-created main module fetch.
#[derive(Debug)]
pub(crate) struct MainRuntimeModuleGraphFetchCompletion {
    target: MainRuntimeModuleGraphFetchTarget,
    result: std::result::Result<ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    network_attribution: MainModuleFetchNetworkAttribution,
    fetch_context: Option<std::sync::Arc<crate::module_runtime::NativeModuleFetchContext>>,
}

impl MainRuntimeModuleGraphFetchCompletion {
    pub(crate) fn new(
        target: MainRuntimeModuleGraphFetchTarget,
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

    pub(crate) fn target(&self) -> MainRuntimeModuleGraphFetchTarget {
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
