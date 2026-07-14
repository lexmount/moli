use std::{future::Future, pin::Pin};

use anyhow::Result;
use url::Url;

use crate::{
    document_script_scheduler::{
        PageOwnedDocumentScriptBodyExecution, PageOwnedDocumentScriptSourceFailure,
    },
    dynamic_script_owner::DynamicScriptPageTaskClaim,
    frame_owner_model::MainDocumentScriptLoadDelayLease,
    planning::PreparedScript,
    protocol_types::NavigationResponse,
};

/// Host hooks required by main page-owned post-parse script execution.
///
/// `PageOwnedDocumentScriptRunner` adapts main `PostParsePageOwnedWork` /
/// `PageTask` bridge payloads to the shared `DocumentScriptExecutionRunner`
/// phase contract through these hooks. Child frame document-script work uses
/// frame owner hooks instead of implementing this trait.
pub(crate) trait PageOwnedDocumentScriptHooks {
    type DocumentOwnerToken: Copy + Eq;

    fn current_document_owner_token(&self) -> Option<Self::DocumentOwnerToken>;

    fn set_loading_ready_state(&mut self) -> Result<()>;

    fn record_script_source_network_result(
        &mut self,
        initiator_url: Url,
        script_url: Url,
        request_initiator_type: crate::types::SubresourceRequestInitiatorType,
        network_result: &std::result::Result<NavigationResponse, String>,
    );

    fn perform_pre_script_checkpoint(&mut self, script_url: &Url) -> Result<()>;

    fn begin_classic_defer_timer_schedule_range(&mut self) {}

    fn finish_classic_defer_timer_schedule_range(&mut self) {}

    fn execute_prepared_script<'a>(
        &'a mut self,
        script: PreparedScript,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    ) -> Pin<Box<dyn Future<Output = PageOwnedDocumentScriptBodyExecution> + 'a>>;

    fn complete_async_source_failure(
        &mut self,
        script: PreparedScript,
        failure: PageOwnedDocumentScriptSourceFailure,
        runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    ) -> PageOwnedDocumentScriptBodyExecution;

    /// Publish the exact parser-async load-delay settlement produced by this
    /// task.
    ///
    /// Publication does not execute the follow-up. The current carrier still
    /// owns task completion, and the Page scheduler cannot consume the
    /// settlement before that carrier returns.
    fn queue_script_load_delay_settlement(
        &mut self,
        script: &PreparedScript,
        binding: MainDocumentScriptLoadDelayLease,
    );
}
