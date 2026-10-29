use anyhow::Result;
use url::Url;

use crate::conn::{
    CdpConnection, CommandDispatchContext, CommittedNavigationPage,
    CommittedRendererAgentAttachment, DocumentNavigationToken, LoadedNavigationPageCommit,
    LoadedNavigationRendererAttachmentCommit, NavigationDispatchState,
};
use crate::domains::activity::MainDocumentNavigationActivity;
use crate::domains::command_output::CommandOutputBuffer;
use crate::domains::network::MaterializedLoadedDocumentProgress;
use moli_core::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererPageCreationArtifacts, RendererRuntimeRealmInfo,
};

mod owned;
pub(crate) use owned::{
    CompletedNavigationProtocolCompletion, NavigationCommandOutput, NavigationProtocolCleanup,
    PreparedNavigationProtocolCompletion,
};

#[derive(Debug, thiserror::Error)]
#[error("loaded navigation is missing its frozen main Document commit identity")]
struct MissingMainDocumentCommit;

struct LoadedNavigationPublication {
    state: NavigationDispatchState,
    token: Option<DocumentNavigationToken>,
    pending_download: Option<moli_core::page::RendererPendingDownloadActivation>,
    page_creation_artifacts: RendererPageCreationArtifacts,
    final_url: Url,
    response_headers: Vec<(String, Vec<u8>)>,
    response_from_cache: bool,
    main_document_body: Option<crate::conn::CapturedBody>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
    progress_gate: crate::domains::network::MainDocumentProgressGate,
    network_error_page: Option<crate::conn::NetworkErrorPageNavigation>,
    main_document_commit: std::sync::Arc<moli_core::page::RendererMainDocumentCommit>,
}

impl LoadedNavigationPublication {
    fn split(
        state: NavigationDispatchState,
        token: Option<DocumentNavigationToken>,
        navigation: MaterializedLoadedDocumentProgress,
    ) -> Result<(
        Self,
        CommittedNavigationPage,
        Vec<RendererRuntimeRealmInfo>,
        std::sync::Arc<moli_core::page::RendererMainDocumentCommit>,
    )> {
        let MaterializedLoadedDocumentProgress {
            page,
            pending_download,
            page_creation_artifacts,
            final_url,
            response_headers,
            response_from_cache,
            main_document_body,
            initial_runtime_realms,
            renderer_output_predecessor,
            main_document_commit,
            progress_gate,
            network_error_page,
        } = navigation;
        let commit = main_document_commit.ok_or(MissingMainDocumentCommit)?;
        Ok((
            Self {
                state,
                token,
                pending_download,
                page_creation_artifacts,
                final_url,
                response_headers,
                response_from_cache,
                main_document_body,
                renderer_output_predecessor,
                progress_gate,
                network_error_page,
                main_document_commit: commit.clone(),
            },
            page,
            initial_runtime_realms,
            commit,
        ))
    }

    fn target_url(&self) -> Url {
        self.network_error_page
            .as_ref()
            .map(|error| error.unreachable_url().clone())
            .unwrap_or_else(|| self.final_url.clone())
    }

    fn publish_superseded(mut self, out: &mut CommandOutputBuffer) {
        // A replacement revokes new state effects, not the original response
        // metadata and native commit already accepted by this navigation.
        super::navigation::push_superseded_navigation_result(out, &self.state);
        let mut metadata = Vec::new();
        crate::domains::network::MainDocumentProgressBackgroundEventBarrier::drain_until_response_metadata_visible(
            &mut metadata, &mut self.progress_gate,
        );
        out.extend_background_events_after_messages(metadata);
        if let Some(boundary) = self.renderer_output_predecessor {
            out.insert_renderer_output_boundary_after_messages(boundary);
        }
    }

    fn publish(mut self, conn: &mut CdpConnection, out: &mut CommandOutputBuffer) {
        let early_title = if let Some(token) = self.token.as_ref()
            && let Some(slot) = conn.runtime_session_owner_slot_mut_for_owner(&self.state.owner)
        {
            slot.merge_prepared_document_lifecycle_events(token, &mut self.page_creation_artifacts);
            slot.take_prepared_document_title(token, &self.page_creation_artifacts)
        } else {
            None
        };
        let Self {
            state,
            token,
            pending_download,
            page_creation_artifacts,
            final_url,
            response_headers,
            response_from_cache,
            main_document_body,
            renderer_output_predecessor,
            progress_gate,
            network_error_page,
            main_document_commit,
        } = self;
        let is_network_error_page = network_error_page.is_some();
        let (page_creation_artifacts, mut deferred_initial_renderer_document_lifecycle_events) =
            split_renderer_page_creation_lifecycle_at_load_boundary(page_creation_artifacts);
        let mut navigation_activity = MainDocumentNavigationActivity::new(
            state,
            final_url.clone(),
            progress_gate,
            token.clone(),
        );
        if let Some(error) = network_error_page {
            navigation_activity = navigation_activity.with_network_error_page_result(
                error.error_text().to_owned(),
                main_document_commit,
            );
        }
        if !is_network_error_page {
            let _ = conn.commit_main_document_resource_for_owner(
                &navigation_activity.state().owner,
                navigation_activity.state().frame_id.clone(),
                navigation_activity.state().loader_id.clone(),
                final_url.clone(),
                response_headers,
                response_from_cache,
                main_document_body,
            );
        }

        let (renderer_document_binding, mut initial_renderer_document_lifecycle_events) = conn
            .bind_renderer_document_lifecycle_for_owner(
                &navigation_activity.state().owner,
                page_creation_artifacts,
                token.clone(),
                navigation_activity.state().frame_id.clone(),
                navigation_activity.state().loader_id.clone(),
            );
        if let Some(change) = early_title {
            conn.apply_renderer_document_title_for_owner(
                &navigation_activity.state().owner,
                &change,
            );
        }
        let load_visibility_barrier_armed = renderer_document_binding.is_some()
            && conn.begin_renderer_document_load_visibility_barrier_for_owner(
                &navigation_activity.state().owner,
                &navigation_activity.state().loader_id,
            );
        if load_visibility_barrier_armed {
            // The creation artifact owns only the initial handoff prefix. Once
            // that prefix is taken, every later lifecycle fact is frozen directly
            // into the Page output FIFO, even if it is produced before protocol
            // finishes installing this binding. Never read the Page back here:
            // ordered ingress and the commit cursor preserve that handoff.
            let (_, visible_events) = conn.ingest_renderer_document_lifecycle_events_for_owner(
                &navigation_activity.state().owner,
                std::mem::take(&mut deferred_initial_renderer_document_lifecycle_events),
            );
            initial_renderer_document_lifecycle_events.extend(visible_events);
        }
        navigation_activity.defer_initial_renderer_document_lifecycle_events_until_load_boundary(
            deferred_initial_renderer_document_lifecycle_events,
        );
        navigation_activity.emit_loaded_navigation_commit(
            conn,
            out,
            pending_download,
            renderer_document_binding,
            initial_renderer_document_lifecycle_events,
            renderer_output_predecessor,
        );
    }
}

fn split_renderer_page_creation_lifecycle_at_load_boundary(
    mut artifacts: RendererPageCreationArtifacts,
) -> (
    RendererPageCreationArtifacts,
    Vec<RendererDocumentLifecycleEvent>,
) {
    let Some(load_sequence) = artifacts
        .lifecycle_snapshot
        .load
        .as_ref()
        .map(|stamp| stamp.sequence)
        .or_else(|| {
            artifacts.initial_lifecycle_events.iter().find_map(|event| {
                matches!(
                    event.kind,
                    RendererDocumentLifecycleEventKind::Milestone(
                        RendererDocumentLifecycleMilestone::Load
                    )
                )
                .then_some(event.sequence)
            })
        })
    else {
        return (artifacts, Vec::new());
    };

    let mut deferred = Vec::new();
    let mut before_load = Vec::new();
    for event in std::mem::take(&mut artifacts.initial_lifecycle_events) {
        if event.sequence >= load_sequence {
            deferred.push(event);
        } else {
            before_load.push(event);
        }
    }
    artifacts.initial_lifecycle_events = before_load;
    artifacts.lifecycle_snapshot.load = None;
    if artifacts
        .lifecycle_snapshot
        .terminated
        .as_ref()
        .is_some_and(|stamp| stamp.sequence >= load_sequence)
    {
        artifacts.lifecycle_snapshot.terminated = None;
    }

    if !deferred.iter().any(|event| {
        matches!(
            event.kind,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
        )
    }) {
        tracing::warn!(
            load_sequence,
            "renderer page creation snapshot contained load without its journal event"
        );
    }

    (artifacts, deferred)
}
