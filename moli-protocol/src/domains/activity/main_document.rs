use serde_json::{Value, json};
use url::Url;

use moli_fetch::NET_ERR_ABORTED_ERROR_TEXT;

use crate::automation::FrontendProtocol;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, CommittedRendererDocumentBinding,
    DeferredMainDocumentLoadObservationId, DocumentNavigationToken, LoadPublicationState,
    NavigationDispatchState, RendererPageResidenceIdentity,
};
use crate::domains::command_output::{BackgroundProtocolEventBuffer, CommandOutputBuffer};
use crate::domains::network::{
    self, FailedNavigationResponseMode, MainDocumentProgressBackgroundEventBarrier,
    MainDocumentProgressGate,
};
use crate::domains::page;
use moli_core::RendererDocumentLifecycleIdentity;
use moli_core::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererPendingDownloadActivation,
};

pub(crate) struct MainDocumentNavigationActivity {
    state: NavigationDispatchState,
    final_url: Url,
    progress_gate: MainDocumentProgressGate,
    result_mode: LoadedNavigationResultMode,
    document_navigation_token: Option<DocumentNavigationToken>,
    deferred_initial_renderer_document_lifecycle_events: Vec<RendererDocumentLifecycleEvent>,
}

enum LoadedNavigationResultMode {
    Success,
    NetworkErrorPage {
        error_text: String,
        commit: std::sync::Arc<moli_core::page::RendererMainDocumentCommit>,
    },
}

pub(crate) struct MainDocumentFailedNavigationActivity {
    state: NavigationDispatchState,
    progress_gate: MainDocumentProgressGate,
    response_mode: FailedNavigationResponseMode,
}

/// The Document owns its load output until the navigation's commit prefix is
/// installed and a concrete lifecycle fact reaches the load boundary. No
/// waiter owns, releases, or discards these notifications.
pub(crate) struct MainDocumentLoadPublication {
    navigation_activity: MainDocumentNavigationActivity,
    renderer_document_binding: Option<CommittedRendererDocumentBinding>,
    pending_download: Option<RendererPendingDownloadActivation>,
    observation_id: DeferredMainDocumentLoadObservationId,
    renderer_page_residence_identity: Option<RendererPageResidenceIdentity>,
}

/// Ready post-load work retains its scheduler identity even after interruption,
/// when completing an empty action still settles exact output predecessors.
pub(crate) struct MainDocumentPostLoadAction {
    navigation_activity: MainDocumentNavigationActivity,
    renderer_document: Option<RendererDocumentLifecycleIdentity>,
    pending_download: Option<RendererPendingDownloadActivation>,
    observation_id: DeferredMainDocumentLoadObservationId,
    renderer_page_residence_identity: Option<RendererPageResidenceIdentity>,
    run_post_load: bool,
}

impl std::fmt::Debug for MainDocumentLoadPublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainDocumentLoadPublication")
            .field("observation_id", &self.observation_id)
            .finish_non_exhaustive()
    }
}

impl MainDocumentNavigationActivity {
    pub(crate) fn new(
        state: NavigationDispatchState,
        final_url: Url,
        progress_gate: MainDocumentProgressGate,
        document_navigation_token: Option<DocumentNavigationToken>,
    ) -> Self {
        Self {
            state,
            final_url,
            progress_gate,
            result_mode: LoadedNavigationResultMode::Success,
            document_navigation_token,
            deferred_initial_renderer_document_lifecycle_events: Vec::new(),
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.state.owner
    }

    pub(crate) fn with_network_error_page_result(
        mut self,
        error_text: String,
        commit: std::sync::Arc<moli_core::page::RendererMainDocumentCommit>,
    ) -> Self {
        self.result_mode = LoadedNavigationResultMode::NetworkErrorPage { error_text, commit };
        self
    }

    pub(crate) fn defer_initial_renderer_document_lifecycle_events_until_load_boundary(
        &mut self,
        events: Vec<RendererDocumentLifecycleEvent>,
    ) {
        self.deferred_initial_renderer_document_lifecycle_events = events;
    }

    pub(crate) fn state(&self) -> &NavigationDispatchState {
        &self.state
    }

    fn expose_loaded_response_metadata(&mut self, out: &mut Vec<BackgroundProtocolEvent>) {
        MainDocumentProgressBackgroundEventBarrier::drain_until_response_metadata_visible(
            out,
            &mut self.progress_gate,
        );
    }

    pub(crate) fn emit_loaded_navigation_commit(
        mut self,
        conn: &mut CdpConnection,
        out: &mut CommandOutputBuffer,
        pending_download: Option<RendererPendingDownloadActivation>,
        renderer_document_binding: Option<CommittedRendererDocumentBinding>,
        initial_renderer_document_lifecycle_events: Vec<RendererDocumentLifecycleEvent>,
        renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    ) {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let timing_started = std::time::Instant::now();
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "commit_start",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
        let network_error_text = match &self.result_mode {
            LoadedNavigationResultMode::Success => None,
            LoadedNavigationResultMode::NetworkErrorPage { error_text, .. } => {
                Some(error_text.clone())
            }
        };
        if let Some(error_text) = network_error_text.as_deref() {
            let mut failure_events = Vec::new();
            self.expose_loaded_response_metadata(&mut failure_events);
            out.extend_background_events_after_messages(failure_events);
            self.emit_network_error_page_navigation_result_into_buffer(out, error_text);
        } else {
            self.emit_navigation_result_from_state_into_buffer(out);
        }
        let mut target_info_events = Vec::new();
        crate::domains::target::emit_target_info_changed_for_owner_background_event(
            conn,
            &mut target_info_events,
            &self.state.owner,
        );
        out.extend_background_events_after_messages(target_info_events);
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "commit_result_emitted",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }

        if network_error_text.is_none() {
            let mut response_metadata_events = Vec::new();
            self.expose_loaded_response_metadata(&mut response_metadata_events);
            out.extend_background_events_after_messages(response_metadata_events);
        }
        if let LoadedNavigationResultMode::NetworkErrorPage { commit, .. } = &self.result_mode {
            out.extend_background_events_after_messages(
                page::main_document_commit_background_events(conn, &self.state.owner, commit),
            );
        }
        if let Some(renderer_output_boundary) = renderer_output_boundary {
            // Chromium can send Page.navigate/Fetch command responses and the
            // main-resource response metadata before Blink exposes the new
            // LocalFrame commit. The renderer publication transports that
            // exact commit (and its V8 context reset) independently, so place
            // its cursor here rather than treating it as command-causal
            // output. DCL and child-Document observations remain on the far
            // side of the same concrete commit.
            out.insert_renderer_output_boundary_after_messages(renderer_output_boundary);
        }
        if network_error_text.is_some() {
            let mut body_complete_events = Vec::new();
            self.flush_body_complete_activity_background_events(&mut body_complete_events);
            out.extend_background_events_after_messages(body_complete_events);
        }
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "commit_response_metadata_visible",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
        let mut pre_domcontentloaded_events = Vec::new();
        self.emit_pre_domcontentloaded_network_backlog_background_events(
            conn,
            &mut pre_domcontentloaded_events,
            timing_enabled,
            timing_started,
        );
        out.extend_background_events_after_messages(pre_domcontentloaded_events);

        // Adoption can include a terminated epoch followed by document.open().
        // Only the bound epoch can cancel the replacement's load publication.
        let mut current_lifecycle_events = initial_renderer_document_lifecycle_events
            .iter()
            .filter(|event| {
                renderer_document_binding.as_ref().is_none_or(|binding| {
                    event.frame == binding.renderer_frame
                        && event.document == binding.renderer_document
                        && event.epoch == binding.renderer_epoch
                })
            });
        let terminated_before_domcontentloaded = current_lifecycle_events.clone().any(|event| {
            matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Terminated {
                    last_reached: None,
                    ..
                }
            )
        });
        let reached_domcontentloaded = current_lifecycle_events.any(|event| {
            matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded
                )
            )
        });
        if let Some(binding) = renderer_document_binding.as_ref() {
            let mut renderer_lifecycle_events = Vec::new();
            page::emit_bound_renderer_document_lifecycle_background_events(
                conn,
                &mut renderer_lifecycle_events,
                &self.state.owner,
                binding,
                &initial_renderer_document_lifecycle_events,
            );
            out.extend_background_events_after_messages(renderer_lifecycle_events);
        }

        if terminated_before_domcontentloaded {
            conn.cancel_renderer_document_load_visibility_barrier_for_owner(
                &self.state.owner,
                &self.state.loader_id,
            );
            return;
        }
        if !reached_domcontentloaded {
            tracing::debug!(
                session_id = self.state.owner.session_id(),
                "renderer page creation reached commit without DOMContentLoaded or termination"
            );
        }
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "commit_domcontentloaded_phase_emitted",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
        let load_output = conn.enqueue_main_document_load_publication(
            self,
            renderer_document_binding,
            pending_download,
        );
        out.extend_background_events_after_messages(load_output);
    }

    fn prepare_deferred_load_completion(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
        pending_download: Option<RendererPendingDownloadActivation>,
    ) -> Option<crate::conn::PreparedDownloadActivationDelivery> {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let timing_started = std::time::Instant::now();
        if !self.is_still_current(conn) {
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url = %self.final_url,
                    stage = "deferred_load_completion_dropped",
                    reason = "stale_navigation",
                );
            }
            return None;
        }
        let post_load_observation_armed = self.emit_renderer_load_completion(conn, out);
        if post_load_observation_armed {
            conn.settle_root_frame_stopped_loading_observation_for_owner(&self.state.owner)
            .expect(
                "an armed root post-load observation must settle its exact stopped-loading fact",
            );
        }
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "commit_load_completion_emitted",
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }

        let download = pending_download?;
        match conn.prepare_pending_download_activation_delivery(&self.state.owner, download, false)
        {
            Ok(pending) => Some(pending),
            Err(error) => {
                let mut output = CommandOutputBuffer::default();
                self.emit_navigation_error_into_buffer(&mut output, &format!("{error:#}"));
                out.extend_background_events(
                    output.into_plan().into_background_events(
                        self.state.navigate_id,
                        self.state.owner.session_id(),
                    ),
                );
                None
            }
        }
    }

    fn is_still_current(&self, conn: &CdpConnection) -> bool {
        if let Some(token) = self.document_navigation_token.as_ref() {
            // The document token is the document identity. A same-document
            // navigation may update the target URL between DCL and load, but
            // it must not make the current document's load completion stale.
            return conn.accepts_document_body_completion_for_owner(&self.state.owner, token);
        }
        conn.runtime_session_owner_target_url_for_owner(&self.state.owner)
            .is_some_and(|url| url == self.final_url.as_str())
    }

    fn emit_navigation_result_from_state_into_buffer(&mut self, out: &mut CommandOutputBuffer) {
        self.emit_navigation_result_into_buffer(
            out,
            self.state.result_projection.payload().clone(),
        );
    }

    fn emit_network_error_page_navigation_result_into_buffer(
        &mut self,
        out: &mut CommandOutputBuffer,
        error_text: &str,
    ) {
        if self.state.navigate_id.is_none() {
            return;
        }
        match self.state.result_projection.protocol() {
            FrontendProtocol::Cdp => {
                let mut result_payload = self.state.result_projection.payload().clone();
                if let Some(payload) = result_payload.as_object_mut() {
                    payload.insert("errorText".to_owned(), json!(error_text));
                    payload.insert("isDownload".to_owned(), json!(false));
                }
                out.push_result_after_messages(result_payload);
            }
            FrontendProtocol::WebDriverClassic | FrontendProtocol::WebDriverBidi => {
                out.push_error_after_messages(-32000, error_text);
            }
        }
    }

    fn emit_download_navigation_result_into_buffer(&mut self, out: &mut CommandOutputBuffer) {
        let mut result_payload = self.state.result_projection.payload().clone();
        if let Some(payload) = result_payload.as_object_mut() {
            payload.remove("loaderId");
            payload.insert("errorText".to_owned(), json!(NET_ERR_ABORTED_ERROR_TEXT));
            payload.insert("isDownload".to_owned(), json!(true));
        }
        self.emit_navigation_result_into_buffer(out, result_payload);
    }

    fn emit_download_frame_stop_background_events(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
    ) {
        let frame_id = self.state.frame_id.clone();
        let loader_id = self.state.loader_id.clone();
        let session_id = self.state.session_id.clone();
        let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
            out,
            &mut self.progress_gate,
        );
        page::emit_navigation_frame_stop_after_download_background_events(
            output.events_after_progress(),
            session_id.as_deref(),
            &frame_id,
            &loader_id,
        );
    }

    fn emit_navigation_result_into_buffer(
        &mut self,
        out: &mut CommandOutputBuffer,
        result_payload: Value,
    ) {
        let navigate_id = self.state.navigate_id;
        {
            let mut background_events = Vec::new();
            let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
                &mut background_events,
                &mut self.progress_gate,
            );
            output.drain_progress();
            out.extend_background_events_after_messages(background_events);
        }
        if navigate_id.is_some() {
            out.push_result_after_messages(result_payload);
        }
    }

    #[cfg(test)]
    pub(crate) fn navigation_error_messages_for_test(&mut self, message: &str) -> Vec<Value> {
        let state = self.state();
        let navigate_id = state.navigate_id;
        let navigate_session_id = state.owner.session_id().map(str::to_owned);
        let mut background_events = Vec::new();
        let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
            &mut background_events,
            &mut self.progress_gate,
        );
        output.drain_progress();
        let mut out = Vec::new();
        out.extend(
            background_events
                .into_iter()
                .filter_map(BackgroundProtocolEvent::into_wire_message),
        );
        if navigate_id.is_some() {
            crate::domains::command_output::CommandOutputPlan::error(-32000, message).emit_into(
                &mut out,
                navigate_id,
                navigate_session_id.as_deref(),
            );
        }
        out
    }

    fn emit_navigation_error_into_buffer(&mut self, out: &mut CommandOutputBuffer, message: &str) {
        let navigate_id = self.state().navigate_id;
        {
            let mut background_events = Vec::new();
            let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
                &mut background_events,
                &mut self.progress_gate,
            );
            output.drain_progress();
            out.extend_background_events_after_messages(background_events);
        }
        if navigate_id.is_some() {
            out.push_error_after_messages(-32000, message);
        }
    }

    fn emit_renderer_load_completion(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
    ) -> bool {
        let armed =
            conn.arm_root_post_load_observation_for_owner(&self.state.owner, &self.state.loader_id);
        if armed {
            let mut network_idle_events = Vec::new();
            conn.emit_root_network_idle_for_owner(&self.state.owner, &mut network_idle_events);
            out.extend_background_events(network_idle_events);
        }
        armed
    }

    fn emit_renderer_load_boundary_facts(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
    ) {
        let mut body_complete_events = Vec::new();
        self.flush_body_complete_activity_background_events(&mut body_complete_events);
        out.extend_background_events(body_complete_events);

        // The Document's admitted lifecycle facts authorize this output.
        // Drain the network prefix before releasing the frozen load tail;
        // navigation waiters never participate in this publication boundary.
        let renderer_events =
            std::mem::take(&mut self.deferred_initial_renderer_document_lifecycle_events);
        let (binding, mut accepted_events) = conn
            .ingest_renderer_document_lifecycle_events_for_owner(
                &self.state.owner,
                renderer_events,
            );
        accepted_events.extend(
            conn.release_renderer_document_load_visibility_barrier_for_owner(
                &self.state.owner,
                &self.state.loader_id,
            )
            .unwrap_or_default(),
        );
        let mut renderer_lifecycle_events = Vec::new();
        if let Some(binding) = binding.as_ref() {
            page::emit_bound_renderer_document_lifecycle_background_events(
                conn,
                &mut renderer_lifecycle_events,
                &self.state.owner,
                binding,
                &accepted_events,
            );
        }
        {
            let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
                &mut renderer_lifecycle_events,
                &mut self.progress_gate,
            );
            output.drain_progress();
        }
        out.extend_background_events(renderer_lifecycle_events);
    }

    fn flush_body_complete_activity_background_events(
        &mut self,
        out: &mut Vec<BackgroundProtocolEvent>,
    ) {
        super::subresource::flush_main_document_body_complete_activity_background_events(
            out,
            &mut self.progress_gate,
        );
    }

    fn emit_pre_domcontentloaded_network_backlog_background_events(
        &self,
        conn: &mut CdpConnection,
        out: &mut Vec<BackgroundProtocolEvent>,
        timing_enabled: bool,
        timing_started: std::time::Instant,
    ) {
        let before = out.len();
        network::emit_pending_network_backlog_activity_background_events(
            conn,
            out,
            network::NetworkBacklogProjectionContext::for_owner(&self.state.owner),
        );
        if timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                url = %self.final_url,
                stage = "pre_dcl_network_backlog_emitted",
                messages = out.len().saturating_sub(before),
                elapsed_ms = timing_started.elapsed().as_millis(),
            );
        }
    }
}

impl MainDocumentLoadPublication {
    pub(crate) fn new(
        navigation_activity: MainDocumentNavigationActivity,
        renderer_document_binding: Option<CommittedRendererDocumentBinding>,
        pending_download: Option<RendererPendingDownloadActivation>,
        observation_id: DeferredMainDocumentLoadObservationId,
        renderer_page_residence_identity: Option<RendererPageResidenceIdentity>,
    ) -> Self {
        Self {
            navigation_activity,
            renderer_document_binding,
            pending_download,
            observation_id,
            renderer_page_residence_identity,
        }
    }

    pub(crate) fn state(&self, conn: &CdpConnection) -> LoadPublicationState {
        conn.main_document_load_publication_state_for_owner(
            self.navigation_activity.owner_scope(),
            self.renderer_document_binding.as_ref(),
        )
    }

    pub(crate) fn publish(
        mut self,
        conn: &mut CdpConnection,
        run_post_load: bool,
        out: &mut BackgroundProtocolEventBuffer,
    ) -> MainDocumentPostLoadAction {
        self.navigation_activity
            .emit_renderer_load_boundary_facts(conn, out);
        let renderer_document = self
            .renderer_document_binding
            .as_ref()
            .map(CommittedRendererDocumentBinding::renderer_document_identity);
        MainDocumentPostLoadAction {
            navigation_activity: self.navigation_activity,
            renderer_document,
            pending_download: self.pending_download,
            observation_id: self.observation_id,
            renderer_page_residence_identity: self.renderer_page_residence_identity,
            run_post_load,
        }
    }
}

impl MainDocumentPostLoadAction {
    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        self.navigation_activity.owner_scope()
    }

    pub(crate) fn renderer_document_identity(&self) -> Option<RendererDocumentLifecycleIdentity> {
        self.renderer_document
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope().session_id()
    }

    pub(crate) fn target_id(&self) -> &str {
        self.navigation_activity
            .document_navigation_token
            .as_ref()
            .map_or(self.navigation_activity.state.frame_id.as_str(), |token| {
                token.target_id.as_str()
            })
    }

    pub(crate) fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.observation_id
    }

    pub(crate) fn renderer_page_residence_identity(&self) -> Option<RendererPageResidenceIdentity> {
        self.renderer_page_residence_identity
    }

    pub(crate) fn prepare_emission(
        mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
    ) -> Option<crate::conn::PreparedDownloadActivationDelivery> {
        if !self.run_post_load {
            return None;
        }
        let pending_download = self.pending_download.take();
        self.navigation_activity
            .prepare_deferred_load_completion(conn, out, pending_download)
    }
}

impl MainDocumentFailedNavigationActivity {
    pub(crate) fn new(
        state: NavigationDispatchState,
        progress_gate: MainDocumentProgressGate,
        response_mode: FailedNavigationResponseMode,
    ) -> Self {
        Self {
            state,
            progress_gate,
            response_mode,
        }
    }

    pub(crate) fn emit_navigation_error_into_buffer(
        mut self,
        out: &mut CommandOutputBuffer,
        message: &str,
    ) {
        let navigate_id = self.state.navigate_id;
        {
            let mut background_events = Vec::new();
            let mut output = MainDocumentProgressBackgroundEventBarrier::background_events(
                &mut background_events,
                &mut self.progress_gate,
            );
            output.drain_progress();
            out.extend_background_events_after_messages(background_events);
        }
        if navigate_id.is_some() {
            if self.response_mode == FailedNavigationResponseMode::CdpErrorTextResult
                && self.state.result_projection.protocol() == FrontendProtocol::Cdp
            {
                let mut result_payload = self.state.result_projection.into_payload();
                if let Some(payload) = result_payload.as_object_mut() {
                    payload.insert("errorText".to_owned(), json!(message));
                    payload.insert("isDownload".to_owned(), json!(false));
                }
                out.push_result_after_messages(result_payload);
            } else {
                out.push_error_after_messages(-32000, message);
            }
        }
    }
}

pub(crate) struct PreparedMainDocumentDownload {
    pending: Option<crate::conn::PreparedDownloadActivationDelivery>,
}

impl PreparedMainDocumentDownload {
    pub(crate) fn prepare_with_output(
        conn: &mut CdpConnection,
        state: NavigationDispatchState,
        download: crate::domains::network::MaterializedDownloadDocumentProgress,
        context: &mut crate::conn::CommandWorkflowContext,
        output_mode: crate::domains::page::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        let mut activity = MainDocumentNavigationActivity::new(
            state,
            download.final_url,
            download.progress_gate,
            None,
        );
        let mut output = CommandOutputBuffer::default();
        output.extend_background_events_after_messages(prefix);
        activity.emit_download_navigation_result_into_buffer(&mut output);
        let mut events = Vec::new();
        activity.emit_download_frame_stop_background_events(&mut events);
        activity.flush_body_complete_activity_background_events(&mut events);
        output.extend_background_events_after_messages(events);
        let pending = match conn.prepare_navigation_download_delivery(
            activity.state(),
            activity.final_url.clone(),
            download.body_artifact,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                activity.emit_navigation_error_into_buffer(&mut output, &format!("{error:#}"));
                None
            }
        };
        output_mode.retain(context, activity.state(), output.into_plan());
        Self { pending }
    }

    pub(crate) async fn run(self, context: &mut crate::conn::CommandWorkflowContext) {
        if let Some(pending) = self.pending {
            let mut events = Vec::new();
            pending.run(&mut events, context.context_mut()).await;
            context.context_mut().protocol_events_mut().extend(events);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, DownloadNavigation, NavigationLoadOutcome, NavigationResultProjection,
    };
    use crate::domains::activity::{ProtocolSchedulerWork, ProtocolSchedulerWorkKind};
    use crate::domains::network::{
        CompletedDownloadProgressTransfer, CompletedMainDocumentNetworkEvents,
        MaterializedNavigationLoadOutcome, empty_main_document_progress_gate_for_test,
        materialize_navigation_load_result,
    };
    use moli_core::page::{
        RendererDocumentLifecycleSnapshot, RendererDocumentTerminationReason,
        RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch,
        RendererLifecycleEventStamp, RendererLifecycleStartReason,
    };

    fn navigation_state() -> NavigationDispatchState {
        NavigationDispatchState {
            web_mcp_invocation: None,
            navigation_initiator: None,
            initial_document_environment: None,
            auxiliary_document_response: None,
            redirect_chain: Vec::new(),
            redirect_headers: None,
            navigate_id: Some(77),
            owner: CommandOwnerScope::for_session("SID-nav"),
            result_projection: NavigationResultProjection::Cdp(
                json!({ "frameId": "FRAME-1", "loaderId": "LID-1" }),
            ),
            frame_id: "FRAME-1".to_owned(),
            session_id: Some("SID-page".to_owned()),
            request_id: Some("REQ-1".to_owned()),
            loader_id: "LID-1".to_owned(),
            request_announced: false,
            announced_redirects: 0,
            requested_url: Url::parse("https://example.test/start").unwrap(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: vec![("Accept".to_owned(), "text/html".to_owned())].into(),
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 12.5,
            source_document_security: Default::default(),
        }
    }

    fn connection_with_dcl_only_renderer_lifecycle() -> (
        CdpConnection,
        CommittedRendererDocumentBinding,
        RendererDocumentLifecycleEvent,
    ) {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-load-publication".to_owned());
        browser_context.set_active_target_id("TID-load-publication");
        browser_context.attach_active_session("SID-nav");
        browser_context.set_target_url("https://example.test/start".to_owned());
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);

        let page_id = moli_core::PageId::new_for_testing(71);
        let frame = RendererFrameToken { page_id };
        let document = RendererDocumentToken::new_for_testing(page_id, 1);
        let epoch = RendererLifecycleEpoch(1);
        let started = RendererDocumentLifecycleEvent {
            frame,
            document,
            epoch,
            sequence: 1,
            timestamp_micros: 10,
            kind: RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::InitialDocument,
            },
        };
        let dcl = RendererDocumentLifecycleEvent {
            sequence: 2,
            timestamp_micros: 20,
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            ..started
        };
        let (binding, accepted) = conn.bind_renderer_document_lifecycle_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-nav"),
            moli_core::page::RendererPageCreationArtifacts {
                active_document: document,
                active_epoch: epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame,
                    document,
                    epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: 1,
                        timestamp_micros: 10,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: 2,
                        timestamp_micros: 20,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, dcl],
            },
            None,
            "FRAME-1".to_owned(),
            "LID-1".to_owned(),
        );
        assert_eq!(accepted, vec![started, dcl]);
        (
            conn,
            binding.expect("deferred-load test lifecycle binding"),
            started,
        )
    }

    fn renderer_load_event_for_test(
        started: RendererDocumentLifecycleEvent,
    ) -> RendererDocumentLifecycleEvent {
        RendererDocumentLifecycleEvent {
            sequence: 3,
            timestamp_micros: 30,
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::Load,
            ),
            ..started
        }
    }

    fn renderer_termination_event_for_test(
        started: RendererDocumentLifecycleEvent,
    ) -> RendererDocumentLifecycleEvent {
        RendererDocumentLifecycleEvent {
            sequence: 3,
            timestamp_micros: 30,
            kind: RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::DomContentLoaded),
                reason: RendererDocumentTerminationReason::Stopped,
            },
            ..started
        }
    }

    fn load_publication_navigation_for_test() -> MainDocumentNavigationActivity {
        MainDocumentNavigationActivity::new(
            navigation_state(),
            Url::parse("https://example.test/start").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        )
    }

    fn enqueue_load_publication_for_test(
        conn: &mut CdpConnection,
        binding: CommittedRendererDocumentBinding,
    ) -> Vec<BackgroundProtocolEvent> {
        conn.enqueue_main_document_load_publication(
            load_publication_navigation_for_test(),
            Some(binding),
            None,
        )
    }

    fn take_deferred_load_work_for_test(conn: &mut CdpConnection) -> ProtocolSchedulerWork {
        let [event]: [crate::conn::CdpSchedulerEvent; 1] = conn
            .take_scheduler_events()
            .try_into()
            .expect("one exact deferred-load work publication");
        let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
            panic!("deferred load must publish concrete protocol scheduler work");
        };
        assert_eq!(
            work.kind(),
            ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
        );
        work
    }

    fn completed_download_progress() -> CompletedDownloadProgressTransfer {
        CompletedDownloadProgressTransfer::new(
            Vec::new(),
            CompletedMainDocumentNetworkEvents::new(
                "GET".to_owned(),
                vec![("Accept".to_owned(), "text/html".to_owned())],
                None,
                200,
                vec![(
                    "Content-Type".to_owned(),
                    b"application/octet-stream".to_vec(),
                )],
                Vec::new(),
                Vec::new(),
                false,
                false,
            ),
        )
    }

    #[test]
    fn navigation_activity_error_drains_progress_before_error_response() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-1", "TID-page");
        browser_context.attach_active_session("SID-page");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        conn.install_browser_context_fixture_for_test(browser_context);

        let state = navigation_state();
        let final_url = Url::parse("https://example.test/download").unwrap();
        let materialized = materialize_navigation_load_result(
            &mut conn,
            &state,
            Ok(NavigationLoadOutcome::download(DownloadNavigation {
                final_url,
                progress_transfer: completed_download_progress(),
            })),
        );
        let MaterializedNavigationLoadOutcome::Download(download) = materialized else {
            panic!("expected download progress");
        };

        let mut activity = MainDocumentNavigationActivity::new(
            state,
            download.final_url,
            download.progress_gate,
            None,
        );
        let out = activity.navigation_error_messages_for_test("download activation failed");

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(out[1]["id"], json!(77));
        assert_eq!(
            out[1]["error"]["message"],
            json!("download activation failed")
        );
    }

    fn failed_navigation_messages(navigation: anyhow::Result<NavigationLoadOutcome>) -> Vec<Value> {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-page");
        browser_context.attach_active_session("SID-page");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .enable_primary_network_events();
        conn.install_browser_context_fixture_for_test(browser_context);

        let state = navigation_state();
        let MaterializedNavigationLoadOutcome::Failed(failure) =
            materialize_navigation_load_result(&mut conn, &state, navigation)
        else {
            panic!("expected failed navigation progress");
        };
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(
            state,
            failure.progress_gate,
            failure.response_mode,
        )
        .emit_navigation_error_into_buffer(&mut output, &failure.error_text);
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));
        messages
    }

    fn assert_failed_navigation_terminal(messages: &[Value], error_text: &str) {
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["method"], json!("Network.loadingFailed"));
        assert_eq!(messages[0]["sessionId"], json!("SID-page"));
        assert_eq!(messages[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(messages[0]["params"]["errorText"], json!(error_text));
        assert_eq!(messages[1]["id"], json!(77));
        assert_eq!(messages[1]["sessionId"], json!("SID-nav"));
    }

    #[test]
    fn failed_navigation_protocol_error_preserves_context_chain() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ))
        .context("failed to read page body from stream")
        .context("failed to load document")
        .context("failed to continue intercepted navigation");
        let messages = failed_navigation_messages(Err(error));
        let expected = "failed to continue intercepted navigation: failed to load document: failed to read page body from stream: connection reset by peer";

        assert_eq!(messages[1]["error"]["code"], json!(-32000));
        assert_eq!(messages[1]["error"]["message"], json!(expected));
        assert!(messages[1].get("result").is_none());
        assert_failed_navigation_terminal(&messages, expected);
    }

    #[test]
    fn failed_navigation_protocol_error_does_not_classify_network_display_text() {
        for error_text in [
            "net::ERR_INTERNET_DISCONNECTED",
            "net::ERR_BLOCKED_BY_CLIENT",
        ] {
            let error = anyhow::anyhow!(error_text).context("failed to load document");
            let messages = failed_navigation_messages(Err(error));
            let expected = format!("failed to load document: {error_text}");

            assert_eq!(messages[1]["error"]["code"], json!(-32000));
            assert_eq!(messages[1]["error"]["message"], json!(expected));
            assert!(messages[1].get("result").is_none());
            assert_failed_navigation_terminal(&messages, &expected);
        }
    }

    #[test]
    fn failed_cdp_navigation_returns_error_text_result_after_network_terminal() {
        let messages = failed_navigation_messages(Ok(NavigationLoadOutcome::NetworkFailure(
            "net::ERR_CONNECTION_RESET".to_owned(),
        )));

        assert_failed_navigation_terminal(&messages, "net::ERR_CONNECTION_RESET");
        assert_eq!(messages[1]["result"]["frameId"], json!("FRAME-1"));
        assert_eq!(messages[1]["result"]["loaderId"], json!("LID-1"));
        assert_eq!(
            messages[1]["result"]["errorText"],
            json!("net::ERR_CONNECTION_RESET")
        );
        assert_eq!(messages[1]["result"]["isDownload"], json!(false));
        assert!(messages[1].get("error").is_none());
    }

    #[test]
    fn failed_bidi_navigation_remains_a_protocol_error() {
        let mut state = navigation_state();
        state.result_projection = NavigationResultProjection::WebDriverBidi(json!({
            "frameId": "must-not-select-cdp",
            "navigation": "LID-1",
            "url": "https://example.test/start"
        }));
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(
            state,
            empty_main_document_progress_gate_for_test(),
            FailedNavigationResponseMode::CdpErrorTextResult,
        )
        .emit_navigation_error_into_buffer(&mut output, "net::ERR_CONNECTION_RESET");
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["error"]["code"], json!(-32000));
        assert_eq!(
            messages[0]["error"]["message"],
            json!("net::ERR_CONNECTION_RESET")
        );
    }

    #[test]
    fn loaded_network_error_page_cdp_navigation_returns_error_text_result() {
        let mut activity = MainDocumentNavigationActivity::new(
            navigation_state(),
            Url::parse("chrome-error://chromewebdata/").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        );
        let mut output = CommandOutputBuffer::default();
        activity.emit_network_error_page_navigation_result_into_buffer(
            &mut output,
            "net::ERR_NAME_NOT_RESOLVED",
        );
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["result"]["frameId"], json!("FRAME-1"));
        assert_eq!(messages[0]["result"]["loaderId"], json!("LID-1"));
        assert_eq!(
            messages[0]["result"]["errorText"],
            json!("net::ERR_NAME_NOT_RESOLVED")
        );
        assert_eq!(messages[0]["result"]["isDownload"], json!(false));
        assert!(messages[0].get("error").is_none());
    }

    #[test]
    fn loaded_network_error_page_bidi_navigation_remains_a_protocol_error() {
        let mut state = navigation_state();
        state.result_projection = NavigationResultProjection::WebDriverBidi(json!({
            "frameId": "must-not-select-cdp",
            "navigation": "LID-1",
            "url": "http://nonexistent.invalid/"
        }));
        let mut activity = MainDocumentNavigationActivity::new(
            state,
            Url::parse("chrome-error://chromewebdata/").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        );
        let mut output = CommandOutputBuffer::default();
        activity.emit_network_error_page_navigation_result_into_buffer(
            &mut output,
            "net::ERR_NAME_NOT_RESOLVED",
        );
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["error"]["code"], json!(-32000));
        assert_eq!(
            messages[0]["error"]["message"],
            json!("net::ERR_NAME_NOT_RESOLVED")
        );
        assert!(messages[0].get("result").is_none());
    }

    #[test]
    fn loaded_network_error_page_classic_navigation_ignores_cdp_shaped_payload() {
        let mut state = navigation_state();
        state.result_projection = NavigationResultProjection::WebDriverClassic(json!({
            "frameId": "FRAME-1",
            "loaderId": "LID-1"
        }));
        let mut activity = MainDocumentNavigationActivity::new(
            state,
            Url::parse("chrome-error://chromewebdata/").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        );
        let mut output = CommandOutputBuffer::default();
        activity.emit_network_error_page_navigation_result_into_buffer(
            &mut output,
            "net::ERR_NAME_NOT_RESOLVED",
        );
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["error"]["code"], json!(-32000));
        assert_eq!(
            messages[0]["error"]["message"],
            json!("net::ERR_NAME_NOT_RESOLVED")
        );
        assert!(messages[0].get("result").is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loaded_commit_does_not_rediscover_child_frame_activity_from_page_state() {
        let mut browser_context = BrowserContext::new("BID-child".to_owned());
        browser_context.set_active_target_id("FRAME-1");
        browser_context.attach_active_session("SID-nav");
        browser_context.set_target_url("https://example.test/parent".to_owned());
        browser_context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_domain_enabled = true;
        let ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));
        let state = navigation_state();
        let activity = MainDocumentNavigationActivity::new(
            state.clone(),
            Url::parse("https://example.test/parent").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        );
        let mut output = CommandOutputBuffer::default();
        let page_id = moli_core::PageId::new_for_testing(17);
        let renderer_frame = RendererFrameToken { page_id };
        let renderer_document = RendererDocumentToken::new_for_testing(page_id, 1);
        let renderer_epoch = RendererLifecycleEpoch(1);
        let renderer_document_binding = CommittedRendererDocumentBinding {
            renderer_frame,
            renderer_document,
            renderer_epoch,
            navigation: None,
            frame_id: "FRAME-1".to_owned(),
            loader_id: "LID-1".to_owned(),
            page_attachment_id: crate::conn::TargetPageAttachmentId::from_raw_for_test(1),
            document_open_replacement_epoch: None,
        };
        let renderer_lifecycle_events = vec![
            RendererDocumentLifecycleEvent {
                frame: renderer_frame,
                document: renderer_document,
                epoch: renderer_epoch,
                sequence: 1,
                timestamp_micros: 12_000_000,
                kind: RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::InitialDocument,
                },
            },
            RendererDocumentLifecycleEvent {
                frame: renderer_frame,
                document: renderer_document,
                epoch: renderer_epoch,
                sequence: 2,
                timestamp_micros: 12_345_678,
                kind: RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded,
                ),
            },
        ];

        ctx.with_fixture(|conn| {
            activity.emit_loaded_navigation_commit(
                conn,
                &mut output,
                None,
                Some(renderer_document_binding),
                renderer_lifecycle_events,
                None,
            )
        });

        let mut out = Vec::new();
        output
            .into_plan()
            .emit_into(&mut out, state.navigate_id, state.owner.session_id());
        let dcl_index = out
            .iter()
            .position(|message| message["method"] == json!("Page.domContentEventFired"))
            .expect("DCL should still be emitted");
        assert!(
            out.iter()
                .all(|message| message["method"] != json!("Page.frameAttached")),
            "main-document commit must not synthesize child output by reading current Page state"
        );
        assert_eq!(out[dcl_index]["params"]["timestamp"], json!(12.345678));
        ctx.with_fixture(|conn| {
            assert!(
                conn.runtime_session_owner_slot(Some("SID-nav"))
                    .expect("owner slot should exist")
                    .loaded_page()
                    .is_none(),
                "main-document lifecycle emission must not read back from a live page"
            )
        });
    }

    #[test]
    fn deferred_load_completion_rejects_same_url_replacement_navigation() {
        let final_url = Url::parse("https://example.test/reload").unwrap();
        let mut browser_context = BrowserContext::new("BID-reload".to_owned());
        browser_context.set_active_target_id("TID-reload");
        browser_context.attach_active_session("SID-nav");
        browser_context.set_target_url(final_url.as_str().to_owned());
        let old_token = browser_context
            .start_document_navigation_for_active_target("LID-1".to_owned())
            .expect("old navigation token");
        browser_context.commit_document_navigation_if_matches(&old_token);

        let mut conn = CdpConnection::new();
        conn.install_browser_context_fixture_for_test(browser_context);
        let activity = MainDocumentNavigationActivity::new(
            navigation_state(),
            final_url.clone(),
            empty_main_document_progress_gate_for_test(),
            Some(old_token),
        );
        assert!(
            activity.is_still_current(&conn),
            "freshly committed navigation token should be accepted"
        );

        let new_token = conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .start_document_navigation_for_active_target("LID-2".to_owned())
            .expect("new navigation token");
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_target_url(final_url.as_str().to_owned());

        assert!(
            !activity.is_still_current(&conn),
            "URL equality is not enough: a newer pending navigation to the same URL must make the old load completion stale"
        );

        conn.browser_context
            .as_mut()
            .expect("browser context")
            .commit_document_navigation_if_matches(&new_token);

        assert!(
            !activity.is_still_current(&conn),
            "a newer committed navigation to the same URL must not accept an older deferred load completion"
        );
    }

    #[test]
    fn deferred_load_completion_accepts_same_document_url_change_for_same_token() {
        let final_url = Url::parse("https://example.test/page").unwrap();
        let mut browser_context = BrowserContext::new("BID-same-doc".to_owned());
        browser_context.set_active_target_id("TID-same-doc");
        browser_context.attach_active_session("SID-nav");
        browser_context.set_target_url(final_url.as_str().to_owned());
        let token = browser_context
            .start_document_navigation_for_active_target("LID-1".to_owned())
            .expect("navigation token");
        browser_context.commit_document_navigation_if_matches(&token);

        let mut conn = CdpConnection::new();
        conn.install_browser_context_fixture_for_test(browser_context);
        let activity = MainDocumentNavigationActivity::new(
            navigation_state(),
            final_url,
            empty_main_document_progress_gate_for_test(),
            Some(token),
        );
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_target_url("https://example.test/page#after-dcl".to_owned());

        assert!(
            activity.is_still_current(&conn),
            "same-document URL changes after DCL must not cancel the current document's deferred load completion"
        );
    }

    fn enable_page_events(conn: &mut CdpConnection) {
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_domain_enabled = true;
    }

    fn project_lifecycle(
        conn: &mut CdpConnection,
        events: Vec<RendererDocumentLifecycleEvent>,
    ) -> Vec<BackgroundProtocolEvent> {
        page::project_renderer_document_lifecycle_background_events(
            conn,
            &CommandOwnerScope::for_session("SID-nav"),
            events,
        )
    }

    fn load_count(events: Vec<BackgroundProtocolEvent>) -> usize {
        events
            .into_iter()
            .filter(|event| event.has_protocol_wire_message())
            .map(|event| event.into_protocol_message())
            .filter(|event| event["method"] == "Page.loadEventFired")
            .count()
    }

    #[test]
    fn publication_state_distinguishes_stale_owners_from_restarted_epochs() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        let owner = CommandOwnerScope::for_session("SID-nav");
        let state = |conn: &CdpConnection, expected: Option<&CommittedRendererDocumentBinding>| {
            conn.main_document_load_publication_state_for_owner(&owner, expected)
        };
        assert_eq!(state(&conn, Some(&binding)), LoadPublicationState::Pending);
        assert_eq!(state(&conn, None), LoadPublicationState::Stale);
        assert_eq!(
            conn.main_document_load_publication_state_for_owner(
                &CommandOwnerScope::for_session("SID-retired"),
                Some(&binding),
            ),
            LoadPublicationState::Stale
        );

        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            0
        );
        assert_eq!(
            state(&conn, Some(&binding)),
            LoadPublicationState::Reached,
            "the authoritative Load must classify as reached while its output is still hidden"
        );
        assert!(
            conn.renderer_document_lifecycle_visible_state_for_session_owner(Some("SID-nav"))
                .unwrap()
                .1
                .load
                .is_none()
        );

        let terminated = RendererDocumentLifecycleEvent {
            sequence: 4,
            timestamp_micros: 40,
            kind: RendererDocumentLifecycleEventKind::Terminated {
                last_reached: Some(RendererDocumentLifecycleMilestone::Load),
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
            ..started
        };
        let restarted = RendererDocumentLifecycleEvent {
            epoch: RendererLifecycleEpoch(2),
            sequence: 5,
            timestamp_micros: 50,
            kind: RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
            ..started
        };
        assert!(project_lifecycle(&mut conn, vec![terminated, restarted]).is_empty());
        assert_eq!(
            state(&conn, Some(&binding)),
            LoadPublicationState::Interrupted
        );
        let mut foreign = binding.clone();
        foreign.renderer_document =
            RendererDocumentToken::new_for_testing(started.frame.page_id, 2);
        assert_eq!(
            state(&conn, Some(&foreign)),
            LoadPublicationState::Stale,
            "a different Document must be stale even when its epoch also differs"
        );
        foreign = binding.clone();
        foreign.page_attachment_id = crate::conn::TargetPageAttachmentId::from_raw_for_test(2);
        assert_eq!(state(&conn, Some(&foreign)), LoadPublicationState::Stale);
        conn.browser_context
            .as_mut()
            .unwrap()
            .active_page_target_mut()
            .runtime_slot
            .replace_page_attachment_id_for_test();
        assert_eq!(
            state(&conn, Some(&binding)),
            LoadPublicationState::Stale,
            "an obsolete binding must not survive replacement of the current Page attachment"
        );
    }

    #[tokio::test]
    async fn interrupted_publication_retains_its_scheduler_settlement_identity() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert!(enqueue_load_publication_for_test(&mut conn, binding).is_empty());
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_termination_event_for_test(started)]
            )),
            0
        );
        let work = take_deferred_load_work_for_test(&mut conn);
        let observation_id = work.main_document_load_observation_id().unwrap();
        let owner = crate::conn::ProtocolStateOwner::new(conn);
        let handle = owner.handle();
        let pending = handle
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap_or_else(|_| panic!("post-load workflow"))
            .unwrap();
        let completed = pending.wait().await;
        assert_eq!(
            completed.main_document_load_observation_id(),
            Some(observation_id),
            "interruption must still settle the exact scheduler predecessor"
        );
        let (events, scheduler) = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap()
            .into_protocol_event_parts();
        assert!(
            events.is_empty() && scheduler.is_empty(),
            "an interrupted lifecycle must complete without loaded/idle/stopped notifications"
        );
    }

    #[test]
    fn lifecycle_load_is_delivered_without_navigation_observers() {
        let (mut conn, _, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let load = renderer_load_event_for_test(started);
        assert_eq!(load_count(project_lifecycle(&mut conn, vec![load])), 1);
        assert_eq!(
            load_count(project_lifecycle(&mut conn, vec![load])),
            0,
            "duplicate Native records must not replay Load"
        );
    }

    #[test]
    fn document_load_is_published_before_post_load_work_is_started() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert!(enqueue_load_publication_for_test(&mut conn, binding).is_empty());
        assert!(
            conn.take_scheduler_events().is_empty(),
            "DCL must not admit a load-waiting owner task"
        );
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            1
        );
        let work = take_deferred_load_work_for_test(&mut conn);
        drop(work);
        assert!(
            conn.renderer_document_lifecycle_visible_state_for_session_owner(Some("SID-nav"))
                .unwrap()
                .1
                .load
                .is_some(),
            "dropping post-load work cannot revoke a published load"
        );
    }

    #[test]
    fn late_load_publication_releases_recorded_facts_exactly_once() {
        let (mut conn, mut binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            0,
            "the commit prefix still owns its output barrier"
        );
        // Replacement metadata is not part of the owning Document identity.
        binding.document_open_replacement_epoch = Some(RendererLifecycleEpoch(99));
        assert_eq!(
            load_count(enqueue_load_publication_for_test(&mut conn, binding)),
            1
        );
        assert_eq!(
            load_count(conn.publish_ready_main_document_load_output(&owner)),
            0
        );
        drop(take_deferred_load_work_for_test(&mut conn));
    }

    #[test]
    fn cancelling_an_external_lifecycle_wait_does_not_cancel_load_delivery() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        let _ = enqueue_load_publication_for_test(&mut conn, binding);
        let context = crate::automation::AutomationContext {
            protocol: FrontendProtocol::Cdp,
            session_id: Some("SID-nav".into()),
            target_id: None,
            browser_context_id: None,
        };
        let key = conn
            .capture_devtools_document_lifecycle_wait_key(
                &context,
                "LID-1",
                RendererDocumentLifecycleMilestone::Load,
            )
            .expect("external lifecycle wait");
        assert!(conn.release_devtools_document_lifecycle_wait_key(&context, &key));
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            1
        );
    }

    #[test]
    fn pending_successor_navigation_does_not_cancel_committed_documents_load() {
        for late in [false, true] {
            let (mut conn, _, started) = connection_with_dcl_only_renderer_lifecycle();
            let owner = CommandOwnerScope::for_session("SID-nav");
            let token = conn
                .start_document_navigation_for_owner(&owner, "LID-1".into())
                .unwrap();
            conn.commit_document_navigation_for_owner_if_matches(&owner, &token);
            let (_, snapshot) = conn
                .renderer_document_lifecycle_authoritative_state_for_session_owner(Some("SID-nav"))
                .unwrap();
            let (binding, _) = conn.bind_renderer_document_lifecycle_for_owner(
                &owner,
                moli_core::page::RendererPageCreationArtifacts {
                    active_document: started.document,
                    active_epoch: started.epoch,
                    lifecycle_snapshot: snapshot,
                    initial_lifecycle_events: Vec::new(),
                },
                Some(token.clone()),
                "FRAME-1".into(),
                "LID-1".into(),
            );
            enable_page_events(&mut conn);
            assert!(
                conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1")
            );
            let binding = binding.unwrap();
            let mut activity = load_publication_navigation_for_test();
            activity.document_navigation_token = Some(token);
            let mut activity = Some(activity);
            if !late {
                assert!(
                    conn.enqueue_main_document_load_publication(
                        activity.take().unwrap(),
                        Some(binding.clone()),
                        None,
                    )
                    .is_empty()
                );
            }
            conn.start_document_navigation_for_owner(&owner, "LID-2".into())
                .unwrap();
            let mut output =
                project_lifecycle(&mut conn, vec![renderer_load_event_for_test(started)]);
            if let Some(activity) = activity {
                output.extend(conn.enqueue_main_document_load_publication(
                    activity,
                    Some(binding),
                    None,
                ));
            }
            assert_eq!(
                load_count(output),
                1,
                "a pending navigation has not replaced the committed Document; publication late: {late}"
            );
        }
    }

    #[test]
    fn late_external_lifecycle_wait_observes_load_without_replaying_it() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        let _ = enqueue_load_publication_for_test(&mut conn, binding);
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            1
        );
        let context = crate::automation::AutomationContext {
            protocol: FrontendProtocol::Cdp,
            session_id: Some("SID-nav".into()),
            target_id: None,
            browser_context_id: None,
        };
        let key = conn
            .capture_devtools_document_lifecycle_wait_key(
                &context,
                "LID-1",
                RendererDocumentLifecycleMilestone::Load,
            )
            .expect("late external lifecycle wait");
        assert_eq!(
            conn.devtools_document_lifecycle_wait_state(&context, &key),
            crate::conn::DevToolsDocumentLifecycleWaitState::Reached
        );
        assert!(conn.release_devtools_document_lifecycle_wait_key(&context, &key));
        assert!(
            conn.publish_ready_main_document_load_output(&owner)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn post_load_work_rejoins_without_replaying_lifecycle_output() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        let _ = enqueue_load_publication_for_test(&mut conn, binding);
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            1
        );
        let work = take_deferred_load_work_for_test(&mut conn);
        let state = crate::conn::ProtocolStateOwner::new(conn);
        let handle = state.handle();
        let pending = handle
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap_or_else(|_| panic!("post-load workflow"))
            .unwrap();
        drop(pending.clone().wait().await);
        let completed = pending.clone().wait().await;
        let (events, _) = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap()
            .into_protocol_event_parts();
        assert_eq!(
            load_count(events),
            0,
            "post-load work never generates the load event"
        );
        let duplicate = pending.wait().await;
        let (events, scheduler) = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(duplicate))
            .unwrap()
            .into_protocol_event_parts();
        assert!(events.is_empty() && scheduler.is_empty());
    }

    #[test]
    fn navigation_adoption_keeps_load_after_document_open_before_domcontentloaded() {
        let (mut conn, _, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        let terminated = RendererDocumentLifecycleEvent {
            kind: RendererDocumentLifecycleEventKind::Terminated {
                last_reached: None,
                reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            },
            ..renderer_termination_event_for_test(started)
        };
        let restarted = RendererDocumentLifecycleEvent {
            sequence: 4,
            timestamp_micros: 40,
            epoch: RendererLifecycleEpoch(2),
            kind: RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
            },
            ..started
        };
        let dcl = RendererDocumentLifecycleEvent {
            sequence: 5,
            timestamp_micros: 50,
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            ..restarted
        };
        let load = RendererDocumentLifecycleEvent {
            sequence: 6,
            timestamp_micros: 60,
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::Load,
            ),
            ..restarted
        };
        let (binding, events) = conn.bind_renderer_document_lifecycle_for_owner(
            &owner,
            moli_core::page::RendererPageCreationArtifacts {
                active_document: started.document,
                active_epoch: restarted.epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    frame: started.frame,
                    document: started.document,
                    epoch: restarted.epoch,
                    started: RendererLifecycleEventStamp {
                        sequence: restarted.sequence,
                        timestamp_micros: restarted.timestamp_micros,
                    },
                    dom_content_loaded: Some(RendererLifecycleEventStamp {
                        sequence: dcl.sequence,
                        timestamp_micros: dcl.timestamp_micros,
                    }),
                    load: None,
                    terminated: None,
                },
                initial_lifecycle_events: vec![started, terminated, restarted, dcl],
            },
            None,
            "FRAME-1".into(),
            "LID-1".into(),
        );
        assert_eq!(events, vec![started, terminated, restarted, dcl]);
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert_eq!(load_count(project_lifecycle(&mut conn, vec![load])), 0);

        let mut output = CommandOutputBuffer::default();
        MainDocumentNavigationActivity::new(
            navigation_state(),
            Url::parse("https://example.test/start").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        )
        .emit_loaded_navigation_commit(&mut conn, &mut output, None, binding, events, None);
        assert_eq!(
            load_count(output.into_plan().into_background_events(None, None)),
            1,
            "a retired epoch's termination must not discard the replacement's load"
        );
        assert_eq!(
            load_count(conn.publish_ready_main_document_load_output(&owner)),
            0,
            "the replacement's load must be published exactly once"
        );
        drop(take_deferred_load_work_for_test(&mut conn));
    }

    #[test]
    fn document_open_restarts_deliver_load_for_early_and_late_publication() {
        for late in [false, true] {
            let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
            enable_page_events(&mut conn);
            let owner = CommandOwnerScope::for_session("SID-nav");
            assert!(
                conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1")
            );
            if !late {
                let _ = enqueue_load_publication_for_test(&mut conn, binding.clone());
            }
            let terminated = RendererDocumentLifecycleEvent {
                kind: RendererDocumentLifecycleEventKind::Terminated {
                    last_reached: Some(RendererDocumentLifecycleMilestone::DomContentLoaded),
                    reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
                },
                ..renderer_termination_event_for_test(started)
            };
            let restarted = RendererDocumentLifecycleEvent {
                sequence: 4,
                timestamp_micros: 40,
                epoch: RendererLifecycleEpoch(2),
                kind: RendererDocumentLifecycleEventKind::Started {
                    reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
                },
                ..started
            };
            let dcl = RendererDocumentLifecycleEvent {
                sequence: 5,
                timestamp_micros: 50,
                kind: RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::DomContentLoaded,
                ),
                ..restarted
            };
            let load = RendererDocumentLifecycleEvent {
                sequence: 6,
                timestamp_micros: 60,
                kind: RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::Load,
                ),
                ..restarted
            };
            let mut output = Vec::new();
            for event in [terminated, restarted, dcl, load] {
                output.extend(project_lifecycle(&mut conn, vec![event]));
            }
            if late {
                output.extend(enqueue_load_publication_for_test(&mut conn, binding));
            }
            assert_eq!(load_count(output), 1, "publication registered late: {late}");
        }
    }

    #[test]
    fn replacing_the_document_discards_only_its_outgoing_load_publication() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        enable_page_events(&mut conn);
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        assert!(enqueue_load_publication_for_test(&mut conn, binding).is_empty());

        let token = conn
            .start_document_navigation_for_owner(&owner, "LID-2".into())
            .unwrap();
        conn.commit_document_navigation_for_owner_if_matches(&owner, &token);
        let (_, snapshot) = conn
            .renderer_document_lifecycle_authoritative_state_for_session_owner(Some("SID-nav"))
            .unwrap();
        let replacement = RendererDocumentToken::new_for_testing(started.frame.page_id, 2);
        conn.bind_renderer_document_lifecycle_for_owner(
            &owner,
            moli_core::page::RendererPageCreationArtifacts {
                active_document: replacement,
                active_epoch: started.epoch,
                lifecycle_snapshot: RendererDocumentLifecycleSnapshot {
                    document: replacement,
                    ..snapshot
                },
                initial_lifecycle_events: Vec::new(),
            },
            Some(token),
            "FRAME-1".into(),
            "LID-2".into(),
        );
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-2"));
        assert_eq!(
            load_count(project_lifecycle(
                &mut conn,
                vec![renderer_load_event_for_test(started)]
            )),
            0
        );
        assert!(
            conn.take_scheduler_events().is_empty(),
            "a retired Document cannot publish post-load work"
        );
        assert!(
            conn.release_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-2")
                .is_some(),
            "retiring the outgoing publication must preserve the replacement's barrier"
        );
    }

    #[test]
    fn superseded_publication_cannot_release_another_documents_load_barrier() {
        let (mut conn, mut binding, _) = connection_with_dcl_only_renderer_lifecycle();
        let owner = CommandOwnerScope::for_session("SID-nav");
        assert!(conn.begin_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1"));
        binding.page_attachment_id = crate::conn::TargetPageAttachmentId::from_raw_for_test(2);
        assert!(enqueue_load_publication_for_test(&mut conn, binding).is_empty());
        assert!(conn.take_scheduler_events().is_empty());
        assert!(
            conn.release_renderer_document_load_visibility_barrier_for_owner(&owner, "LID-1")
                .is_some(),
            "foreign publication must not clear the current barrier"
        );
    }
}
