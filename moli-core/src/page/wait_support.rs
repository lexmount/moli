use std::time::Duration;

use anyhow::Result;

use super::RendererPageDiagnosticsSnapshot;
use super::protocol_support::SubresourceResponseWaitCriteria;
use super::{CompletedPageCommand, Page, PendingPageCommand, RendererCommandTurnOutput};
use crate::{
    RendererOutputFence,
    network::ResourceRequestClient,
    renderer::{RendererDocumentQuerySelectorNode, RendererPageCommand, RendererPageReply},
};

impl Page {
    /// Runs an already-queued target-Document `javascript:` URL task at the
    /// protocol navigation owner boundary.
    ///
    /// The command completion is the causal barrier that prevents a browser
    /// navigation commit from replacing the target Document first.  Its
    /// concrete renderer output remains on the Page stream and the returned
    /// fence is available to owners that also need to order protocol output.
    pub async fn run_pending_javascript_url_tasks_before_browser_navigation(
        &mut self,
    ) -> Result<Option<RendererOutputFence>> {
        let pending = self.start_page_command(
            RendererPageCommand::RunPendingJavascriptUrlTasksBeforeBrowserNavigation,
        )?;
        let completion = pending.wait().await?;
        let renderer_output_predecessor = completion.renderer_output_predecessor();
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "pending javascript URL task command",
            "a unit reply",
            RendererPageReply::Unit => Ok(renderer_output_predecessor),
        )
    }

    pub async fn ms_to_next_timeout(&mut self) -> Result<Option<u64>> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::MsToNextTimeout)
            .await?;
        expect_page_reply!(
            reply,
            "page next timeout command",
            "an optional u64 reply",
            RendererPageReply::OptionalU64(ms) => Ok(ms),
        )
    }

    pub(crate) fn start_complete_child_frame_lifecycle_work_best_effort(
        &self,
        loader: &ResourceRequestClient,
        timeout: Duration,
    ) -> Result<PendingPageCommand> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        self.start_page_command(
            RendererPageCommand::CompleteChildFrameLifecycleWorkBestEffort { timeout_ms, loader },
        )
    }

    pub(crate) fn finish_complete_child_frame_lifecycle_work_best_effort_command_turn(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<(bool, RendererCommandTurnOutput)> {
        let output = self.finish_page_command_turn(completion);
        let RendererPageReply::Bool(completed) = output.completion().reply() else {
            anyhow::bail!(
                "complete child frame lifecycle work page command expected a bool reply, got {}",
                Self::page_reply_kind(output.completion().reply())
            );
        };
        Ok((*completed, output))
    }

    pub async fn has_pending_location_navigation(&mut self) -> Result<bool> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::HasPendingLocationNavigation)
            .await?;
        expect_page_reply!(
            reply,
            "has pending location navigation page command",
            "a bool reply",
            RendererPageReply::Bool(has_work) => Ok(has_work),
        )
    }

    pub async fn page_diagnostics_snapshot_async(
        &mut self,
    ) -> Result<RendererPageDiagnosticsSnapshot> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::PageDiagnosticsSnapshot)
            .await?;
        expect_page_reply!(
            reply,
            "page diagnostics snapshot page command",
            "page diagnostics snapshot",
            RendererPageReply::PageDiagnosticsSnapshot(snapshot) => Ok(snapshot),
        )
    }

    pub fn start_page_diagnostics_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::PageDiagnosticsSnapshot)
    }

    pub fn finish_page_diagnostics_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<RendererPageDiagnosticsSnapshot> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "page diagnostics snapshot page command",
            "page diagnostics snapshot",
            RendererPageReply::PageDiagnosticsSnapshot(snapshot) => Ok(snapshot),
        )
    }

    pub(crate) async fn wait_for_selector(
        &mut self,
        loader: &ResourceRequestClient,
        selector: &str,
        timeout: Duration,
    ) -> Result<RendererDocumentQuerySelectorNode> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let command = RendererPageCommand::WaitForSelector {
            selector: selector.to_owned(),
            timeout_ms,
            loader,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "wait for selector page command",
            "a matched document query selector node reply",
            RendererPageReply::DocumentQuerySelectorNode(node) => Ok(node),
        )
    }

    pub(crate) async fn wait_for_script_truthy(
        &mut self,
        loader: &ResourceRequestClient,
        expression: &str,
        timeout: Duration,
    ) -> Result<()> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let command = RendererPageCommand::WaitForScriptTruthy {
            expression: expression.to_owned(),
            timeout_ms,
            loader,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "wait for script truthy page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(crate) async fn wait_for_subresource_response(
        &mut self,
        loader: &ResourceRequestClient,
        criteria: SubresourceResponseWaitCriteria,
        timeout: Duration,
    ) -> Result<()> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let command = RendererPageCommand::WaitForSubresourceResponse {
            criteria,
            timeout_ms,
            loader,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "wait for subresource response page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(crate) async fn wait_for_network_idle(
        &mut self,
        loader: &ResourceRequestClient,
        timeout: Duration,
    ) -> Result<()> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let (reply, page_state) = self
            .handle
            .wait_for_network_idle(timeout_ms, loader)
            .await?;
        self.replace_page_state(page_state);
        expect_page_reply!(
            reply,
            "wait for network idle page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(crate) async fn wait_for_dom_stable(
        &mut self,
        loader: &ResourceRequestClient,
        timeout: Duration,
    ) -> Result<()> {
        let loader = loader.clone();
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let (reply, page_state) = self.handle.wait_for_dom_stable(timeout_ms, loader).await?;
        self.replace_page_state(page_state);
        expect_page_reply!(
            reply,
            "wait for dom stable page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }
}
