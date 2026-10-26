use anyhow::Result;

use crate::conn::{
    CdpConnection, DocumentNavigationToken, NavigationDispatchState,
    PreparedRendererNavigationFinish, ProtocolWorkflowDriver, StablePageNavigationCommitTarget,
};
use crate::domains::{
    activity,
    command_output::CommandOutputBuffer,
    network::{self, MaterializedFailedDocumentProgress},
};

/// Retirement is applied once, while the original navigation is current. The
/// remaining native work owns its Page and original renderer response leases.
pub(crate) struct PreparedNavigationFailureCleanup {
    page: Option<moli_core::page::Page>,
    renderer: PreparedRendererNavigationFinish,
}

impl PreparedNavigationFailureCleanup {
    pub(crate) async fn wait_using(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> Result<Vec<crate::conn::BackgroundProtocolEvent>> {
        if let Some(page) = self.page {
            let _ = page.close_async().await;
        }
        self.renderer.wait_using(driver).await
    }

    pub(crate) async fn wait_inline(
        self,
        conn: &mut CdpConnection,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        self.wait_using(&mut ProtocolWorkflowDriver::inline(conn))
            .await
            .expect("the inline protocol owner is live")
    }
}

pub(crate) fn prepare_failed_navigation_cleanup(
    conn: &mut CdpConnection,
    output: &mut CommandOutputBuffer,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    failure: MaterializedFailedDocumentProgress,
) -> PreparedNavigationFailureCleanup {
    let owner = state.owner.clone();
    let loader = state.loader_id.clone();
    let mut page = None;
    let current = conn.accepts_pending_document_navigation_for_owner(&owner, &token);
    if !current {
        if let Some(invocation) = state.web_mcp_invocation.as_ref() {
            output.extend_background_events_after_messages(
                super::super::web_mcp::navigation_failed(conn, &owner, invocation.invocation_id),
            );
        }
        super::push_superseded_navigation_result(output, &state);
    } else {
        if let Some(invocation) = state.web_mcp_invocation.as_ref() {
            output.extend_background_events_after_messages(
                super::super::web_mcp::navigation_failed(conn, &owner, invocation.invocation_id),
            );
        }
        let _ = conn.clear_pending_navigation_history_update_for_owner(&owner);
        let network::MaterializedFailedDocumentProgress {
            error_text,
            document_policy,
            response_mode,
            progress_gate,
        } = failure;
        if document_policy.invalidates_committed_document() {
            let expected = conn
                .target_page_residence_identity_for_owner(&owner)
                .zip(conn.renderer_page_residence_identity_for_owner(&owner))
                .map(
                    |(target_page, renderer_page)| StablePageNavigationCommitTarget {
                        target_page,
                        renderer_page,
                    },
                );
            page = conn.take_loaded_page_after_failed_navigation_for_owner(
                &owner,
                &token,
                expected.as_ref(),
                &state.requested_url,
            );
        }
        activity::MainDocumentFailedNavigationActivity::new(state, progress_gate, response_mode)
            .emit_navigation_error_into_buffer(output, &error_text);
    }
    let renderer = conn.prepare_renderer_navigation_finish(&owner, &token);
    if current {
        conn.clear_pending_document_navigation_for_owner_if_loader_matches(&owner, &loader);
    }
    PreparedNavigationFailureCleanup { page, renderer }
}
