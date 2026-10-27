use std::sync::Arc;

use super::ProtocolWorkflowState;
use crate::conn::{
    BackgroundPreparedNavigationCommit, CommandDispatchContext, CommandOwnerScope,
    CommandWorkflowContext, DocumentNavigationToken, PreparedRendererNavigationFinish,
    SessionDisposalWait, TargetPageResidenceIdentity,
};
use crate::domains::{
    command_output::CommandOutputPlan,
    page::{
        CompletedNavigationProtocolCompletion, NavigationProtocolCleanup,
        PreparedNavigationProtocolCompletion,
    },
    runtime::PreparedBidiPreloadChannelListeners,
    target::PreparedDedicatedWorkerRetirement,
};
use moli_core::page::{
    RendererAgentAttachmentId, RendererMainDocumentCommit,
    RendererPageCommandPostResponseContinuation, RendererRuntimeRealmInfo,
};
use url::Url;

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_navigation_protocol_completion(
        self,
        completion: BackgroundPreparedNavigationCommit,
        context: &mut CommandWorkflowContext,
    ) -> PreparedNavigationProtocolCompletion {
        PreparedNavigationProtocolCompletion::prepare(self.connection, completion, context)
    }

    pub(crate) fn prepare_replaced_navigation_workers(
        self,
        page: Option<TargetPageResidenceIdentity>,
        cleanup: NavigationProtocolCleanup,
    ) -> Vec<PreparedDedicatedWorkerRetirement> {
        let workers = page
            .map(|page| {
                PreparedDedicatedWorkerRetirement::prepare_for_replaced_page(self.connection, &page)
            })
            .unwrap_or_default();
        cleanup.register(&workers);
        workers
    }

    pub(crate) fn prepare_navigation_commit_preloads(
        self,
        owner: CommandOwnerScope,
        token: DocumentNavigationToken,
        binding: Option<SessionDisposalWait>,
        attachment: RendererAgentAttachmentId,
        main_commit: Arc<RendererMainDocumentCommit>,
        url: Url,
        realms: Vec<RendererRuntimeRealmInfo>,
        continuation: Option<RendererPageCommandPostResponseContinuation>,
        context: &mut CommandDispatchContext,
    ) -> Vec<PreparedBidiPreloadChannelListeners> {
        let conn = self.connection;
        if let Some(continuation) = continuation {
            context
                .response_flush()
                .defer_until_response_flush(move || continuation.release());
        }
        if binding.as_ref().is_some_and(SessionDisposalWait::is_closed)
            || !conn.accepts_pending_document_navigation_for_owner(&owner, &token)
            || conn.current_renderer_agent_attachment_id_for_owner(&owner) != Some(attachment)
        {
            return Vec::new();
        }
        let _ = conn.commit_loaded_navigation_target_identity_for_owner(&owner, &main_commit, &url);
        if conn
            .target_runtime_configuration_for_owner(&owner)
            .is_some_and(|state| state.runtime_frontend_enabled)
        {
            let _ = conn
                .set_renderer_runtime_agent_owns_page_console_api_events_for_owner(&owner, true);
        }
        conn.commit_document_navigation_for_owner_if_matches(&owner, &token);
        if !conn.target_owner_has_bidi_channel_preload_script_for_owner(&owner) {
            return Vec::new();
        }
        let mut ids = Vec::new();
        for realm in realms {
            if realm.realm_id.as_deref().is_some_and(|id| !id.is_empty())
                && !ids.contains(&realm.context_id)
            {
                ids.push(realm.context_id);
            }
        }
        ids.into_iter()
            .filter_map(|id| PreparedBidiPreloadChannelListeners::prepare(conn, &owner, id))
            .collect()
    }

    pub(crate) fn prepare_navigation_protocol_finish(
        self,
        owner: CommandOwnerScope,
        token: DocumentNavigationToken,
        binding: Option<SessionDisposalWait>,
    ) -> Option<PreparedRendererNavigationFinish> {
        binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
            .then(|| {
                self.connection
                    .prepare_renderer_navigation_finish(&owner, &token)
            })
    }

    pub(crate) fn publish_prepared_navigation_protocol_completion(
        self,
        completion: CompletedNavigationProtocolCompletion,
        context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        completion.publish(self.connection, context)
    }
}
