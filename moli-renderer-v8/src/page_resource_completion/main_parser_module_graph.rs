use crate::{
    document_script_scheduler::ParserPendingScriptId, module_runtime::ModuleGraphFetchedSource,
    module_script_continuation::MainParserDocumentOwner, types::SharedNavigationResponseResult,
};

use url::Url;

/// Exact PageVm-local owner of one main-Document parser module fetch.
///
/// A parser graph may have several concurrent dependency fetches. The
/// `PendingScript` identifies the parser continuation that started the fetch,
/// while `load_id` identifies the suspended module-map fetch. A shared fetch
/// may outlive that initiating script after another graph joins it, so current
/// terminal authority is the exact Document plus the still-inflight `load_id`,
/// not continued residence of the initiating `PendingScript`. The stable Page
/// queue adds the producing root `RendererDocumentToken` at its envelope
/// boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainParserModuleGraphFetchTarget {
    pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
    load_id: u64,
}

impl MainParserModuleGraphFetchTarget {
    pub(crate) fn new(
        pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
        load_id: u64,
    ) -> Self {
        Self {
            pending_script_id,
            load_id,
        }
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }

    pub(crate) fn document_owner(self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.pending_script_id.owner().task_owner()
    }
}

/// One network terminal for an exact parser-owned main-Document module fetch.
#[derive(Debug)]
pub(crate) struct MainParserModuleGraphFetchCompletion {
    target: MainParserModuleGraphFetchTarget,
    result: std::result::Result<ModuleGraphFetchedSource, String>,
    network_result: Option<SharedNavigationResponseResult>,
    request_url: Url,
}

impl MainParserModuleGraphFetchCompletion {
    pub(crate) fn new(
        target: MainParserModuleGraphFetchTarget,
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

    pub(crate) fn target(&self) -> MainParserModuleGraphFetchTarget {
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
