use serde_json::{Value, json};
use url::Url;

use moli_fetch::NET_ERR_ABORTED_ERROR_TEXT;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandDispatchContext, CommandOwnerScope,
    CommittedRendererDocumentBinding, CompletedDownloadBodyArtifact,
    DeferredMainDocumentLoadObservationId, DocumentNavigationToken, NavigationDispatchState,
    RendererDocumentLifecycleObservation, RendererDocumentLifecycleObserver,
    RendererPageResidenceIdentity,
};
use crate::devtools_runtime::DevToolsProtocol;
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
    NetworkErrorPage { error_text: String },
}

pub(crate) struct MainDocumentFailedNavigationActivity {
    state: NavigationDispatchState,
    progress_gate: MainDocumentProgressGate,
    response_mode: FailedNavigationResponseMode,
}

pub(crate) struct MainDocumentDownloadNavigationActivity {
    navigation_activity: MainDocumentNavigationActivity,
    body_artifact: CompletedDownloadBodyArtifact,
}

struct DeferredMainDocumentLoadCompletionState {
    navigation_activity: MainDocumentNavigationActivity,
    owner_scope: CommandOwnerScope,
    renderer_document_binding: Option<CommittedRendererDocumentBinding>,
    pending_download: Option<RendererPendingDownloadActivation>,
}

pub(crate) struct DeferredMainDocumentLoadCompletionAdmission {
    state: DeferredMainDocumentLoadCompletionState,
}

pub(crate) struct DeferredMainDocumentLoadCompletionActivity {
    state: DeferredMainDocumentLoadCompletionState,
    observation_id: DeferredMainDocumentLoadObservationId,
    renderer_page_residence_identity: Option<RendererPageResidenceIdentity>,
    lifecycle_observer: RendererDocumentLifecycleObserver,
}

pub(crate) struct PendingDeferredMainDocumentLoadCompletionActivity {
    completion: DeferredMainDocumentLoadCompletionActivity,
}

pub(crate) struct CompletedDeferredMainDocumentLoadCompletionActivity {
    state: DeferredMainDocumentLoadCompletionState,
    observation_id: DeferredMainDocumentLoadObservationId,
    observation: RendererDocumentLifecycleObservation,
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

    pub(crate) fn with_network_error_page_result(mut self, error_text: String) -> Self {
        self.result_mode = LoadedNavigationResultMode::NetworkErrorPage { error_text };
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

    pub(crate) async fn emit_loaded_navigation_commit_async(
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
            LoadedNavigationResultMode::NetworkErrorPage { error_text } => Some(error_text.clone()),
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
        crate::domains::target::emit_target_info_changed_for_session_owner_background_event(
            conn,
            &mut target_info_events,
            self.state.navigate_session_id.as_deref(),
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

        let terminated_before_domcontentloaded = initial_renderer_document_lifecycle_events
            .iter()
            .any(|event| {
                matches!(
                    event.kind,
                    RendererDocumentLifecycleEventKind::Terminated {
                        last_reached: None,
                        ..
                    }
                )
            });
        let reached_domcontentloaded =
            initial_renderer_document_lifecycle_events
                .iter()
                .any(|event| {
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
                self.state.navigate_session_id.as_deref(),
                binding,
                &initial_renderer_document_lifecycle_events,
            );
            out.extend_background_events_after_messages(renderer_lifecycle_events);
        }

        if terminated_before_domcontentloaded {
            conn.cancel_renderer_document_load_visibility_barrier_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                &self.state.loader_id,
            );
            return;
        }
        if !reached_domcontentloaded {
            tracing::debug!(
                session_id = self.state.navigate_session_id.as_deref(),
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
        let owner_scope =
            CommandOwnerScope::capture(conn, self.state.navigate_session_id.as_deref());
        conn.enqueue_deferred_main_document_load_completion(
            DeferredMainDocumentLoadCompletionAdmission::new(self, pending_download)
                .with_renderer_document_binding(renderer_document_binding)
                .with_owner_scope(owner_scope),
        );
    }

    async fn emit_deferred_load_completion_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
        pending_download: Option<RendererPendingDownloadActivation>,
        renderer_document: Option<RendererDocumentLifecycleIdentity>,
    ) {
        let timing_enabled = moli_trace::cdp_nav_timing_enabled();
        let timing_started = std::time::Instant::now();
        if !self.is_still_current(conn) {
            conn.cancel_renderer_document_load_visibility_barrier_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                &self.state.loader_id,
            );
            if timing_enabled {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    url = %self.final_url,
                    stage = "deferred_load_completion_dropped",
                    reason = "stale_navigation",
                );
            }
            return;
        }
        let post_load_observation_armed = self
            .emit_renderer_load_completion_async(conn, out, renderer_document)
            .await;
        if post_load_observation_armed {
            conn.settle_root_frame_stopped_loading_observation(
                self.state.navigate_session_id.as_deref(),
            )
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

        if pending_download.is_some() {
            self.activate_pending_download_async(conn, out, pending_download)
                .await;
        }
    }

    fn is_still_current(&self, conn: &CdpConnection) -> bool {
        if let Some(token) = self.document_navigation_token.as_ref() {
            // The document token is the document identity. A same-document
            // navigation may update the target URL between DCL and load, but
            // it must not make the current document's load completion stale.
            return conn.accepts_document_body_completion_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                token,
            );
        }
        conn.runtime_session_owner_target_url(self.state.navigate_session_id.as_deref())
            .is_some_and(|url| url == self.final_url.as_str())
    }

    async fn emit_download_navigation_commit_into_buffer_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut CommandOutputBuffer,
        body_artifact: CompletedDownloadBodyArtifact,
        command_context: &mut CommandDispatchContext,
    ) {
        self.emit_download_navigation_result_into_buffer(out);
        let mut background_events = Vec::new();
        self.emit_download_frame_stop_background_events(&mut background_events);
        out.extend_background_events_after_messages(background_events);
        self.emit_navigation_download_response_into_buffer_async(
            conn,
            out,
            body_artifact,
            command_context,
        )
        .await;
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
            DevToolsProtocol::Cdp => {
                let mut result_payload = self.state.result_projection.payload().clone();
                if let Some(payload) = result_payload.as_object_mut() {
                    payload.insert("errorText".to_owned(), json!(error_text));
                    payload.insert("isDownload".to_owned(), json!(false));
                }
                out.push_result_after_messages(result_payload);
            }
            DevToolsProtocol::WebDriverClassic | DevToolsProtocol::WebDriverBidi => {
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
        let navigate_session_id = state.navigate_session_id.clone();
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
                .map(BackgroundProtocolEvent::into_protocol_message),
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

    async fn emit_renderer_load_completion_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
        _renderer_document: Option<RendererDocumentLifecycleIdentity>,
    ) -> bool {
        self.emit_renderer_load_boundary_facts(conn, out);
        let armed = conn.arm_root_post_load_observation_for_session_owner(
            self.state.navigate_session_id.as_deref(),
            &self.state.loader_id,
        );
        if armed {
            let mut network_idle_events = Vec::new();
            conn.emit_root_network_idle_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                &mut network_idle_events,
            );
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

        // Reaching this boundary means the protocol-side exact lifecycle
        // observer has already consumed the live concrete load record. Only
        // commit-time events and the visibility-barrier tail remain to be
        // projected; rescanning renderer state here would rediscover output
        // owned by an earlier turn.
        let renderer_events =
            std::mem::take(&mut self.deferred_initial_renderer_document_lifecycle_events);
        let (binding, mut accepted_events) = conn
            .ingest_renderer_document_lifecycle_events_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                renderer_events,
            );
        accepted_events.extend(
            conn.release_renderer_document_load_visibility_barrier_for_session_owner(
                self.state.navigate_session_id.as_deref(),
                &self.state.loader_id,
            )
            .unwrap_or_default(),
        );
        let mut renderer_lifecycle_events = Vec::new();
        if let Some(binding) = binding.as_ref() {
            page::emit_bound_renderer_document_lifecycle_background_events(
                conn,
                &mut renderer_lifecycle_events,
                self.state.navigate_session_id.as_deref(),
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

    async fn emit_navigation_download_response_into_buffer_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut CommandOutputBuffer,
        body_artifact: CompletedDownloadBodyArtifact,
        command_context: &mut CommandDispatchContext,
    ) {
        let mut background_events = Vec::new();
        self.flush_body_complete_activity_background_events(&mut background_events);
        let final_url = self.final_url.clone();
        let error = conn
            .handle_navigation_download_response_async(
                &mut background_events,
                self.state(),
                final_url,
                body_artifact,
                command_context,
            )
            .await
            .err();
        out.extend_background_events_after_messages(background_events);
        if let Some(message) = error {
            self.emit_navigation_error_into_buffer(out, &message);
        }
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

    async fn activate_pending_download_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
        pending_download: Option<RendererPendingDownloadActivation>,
    ) {
        let mut command_context = CommandDispatchContext::default();
        let error = if let Some(download) = pending_download {
            let mut download_events = Vec::new();
            let error = conn
                .handle_pending_download_activation_inline_async(
                    &mut download_events,
                    self.state.navigate_session_id.as_deref(),
                    download,
                    &mut command_context,
                )
                .await
                .err();
            out.extend_background_events(download_events);
            error
        } else {
            None
        };
        if let Some(message) = error {
            let navigate_id = self.state.navigate_id;
            let navigate_session_id = self.state.navigate_session_id.clone();
            let mut command_output = CommandOutputBuffer::default();
            self.emit_navigation_error_into_buffer(&mut command_output, &message);
            out.extend_background_events(
                command_output
                    .into_plan()
                    .into_background_events(navigate_id, navigate_session_id.as_deref()),
            );
        }
        out.extend_background_events(command_context.take_protocol_events());
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
            network::NetworkBacklogProjectionContext::new(
                self.state.navigate_session_id.as_deref(),
            ),
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

impl DeferredMainDocumentLoadCompletionAdmission {
    fn new(
        navigation_activity: MainDocumentNavigationActivity,
        pending_download: Option<RendererPendingDownloadActivation>,
    ) -> Self {
        let owner_scope = CommandOwnerScope::from_session_and_owner_route(
            navigation_activity.state.navigate_session_id.as_deref(),
            None,
        );
        Self {
            state: DeferredMainDocumentLoadCompletionState {
                navigation_activity,
                owner_scope,
                renderer_document_binding: None,
                pending_download,
            },
        }
    }

    fn with_renderer_document_binding(
        mut self,
        binding: Option<CommittedRendererDocumentBinding>,
    ) -> Self {
        self.state.renderer_document_binding = binding;
        self
    }

    fn with_owner_scope(mut self, owner_scope: CommandOwnerScope) -> Self {
        self.state.owner_scope = owner_scope;
        self
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.state.owner_scope
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.state.owner_scope.session_id()
    }

    pub(crate) fn is_still_current_for_scheduler(&self, conn: &CdpConnection) -> bool {
        self.state.navigation_activity.is_still_current(conn)
    }

    pub(crate) fn bind_lifecycle_observer(
        self,
        conn: &mut CdpConnection,
        observation_id: DeferredMainDocumentLoadObservationId,
    ) -> DeferredMainDocumentLoadCompletionActivity {
        let is_current = self.is_still_current_for_scheduler(conn);
        let renderer_page_residence_identity = is_current
            .then(|| conn.renderer_page_residence_identity_for_session_owner(self.session_id()))
            .flatten();
        let lifecycle_observer = if is_current {
            conn.register_exact_renderer_document_lifecycle_observer_for_session_owner(
                self.session_id(),
                self.state.renderer_document_binding.as_ref(),
                RendererDocumentLifecycleMilestone::Load,
            )
        } else {
            RendererDocumentLifecycleObserver::resolved(
                RendererDocumentLifecycleObservation::Superseded,
            )
        };
        DeferredMainDocumentLoadCompletionActivity {
            state: self.state,
            observation_id,
            renderer_page_residence_identity,
            lifecycle_observer,
        }
    }
}

impl DeferredMainDocumentLoadCompletionActivity {
    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.state.owner_scope
    }

    pub(crate) fn renderer_document_identity(&self) -> Option<RendererDocumentLifecycleIdentity> {
        self.state.renderer_document_identity()
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.state.owner_scope.session_id()
    }

    pub(crate) fn target_id(&self) -> &str {
        self.state
            .navigation_activity
            .document_navigation_token
            .as_ref()
            .map_or(
                self.state.navigation_activity.state.frame_id.as_str(),
                |token| token.target_id.as_str(),
            )
    }

    pub(crate) fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.observation_id
    }

    pub(crate) fn renderer_page_residence_identity(&self) -> Option<RendererPageResidenceIdentity> {
        self.renderer_page_residence_identity
    }

    pub(crate) fn has_terminal_lifecycle_observation(&self) -> bool {
        self.lifecycle_observer.observation().is_terminal()
    }

    pub(crate) fn try_complete(
        self: Box<Self>,
    ) -> Result<CompletedDeferredMainDocumentLoadCompletionActivity, Box<Self>> {
        let observation = self.lifecycle_observer.observation();
        if !observation.is_terminal() {
            return Err(self);
        }
        let completion = *self;
        Ok(CompletedDeferredMainDocumentLoadCompletionActivity {
            state: completion.state,
            observation_id: completion.observation_id,
            observation,
        })
    }

    pub(crate) fn start_scheduler_step(self) -> PendingDeferredMainDocumentLoadCompletionActivity {
        PendingDeferredMainDocumentLoadCompletionActivity { completion: self }
    }
}

impl DeferredMainDocumentLoadCompletionState {
    fn renderer_document_identity(&self) -> Option<RendererDocumentLifecycleIdentity> {
        self.renderer_document_binding
            .as_ref()
            .map(CommittedRendererDocumentBinding::renderer_document_identity)
    }

    async fn emit_load_completion_after_lifecycle_ready_async(
        &mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
    ) {
        let pending_download = self.pending_download.take();
        let renderer_document = self.renderer_document_identity();
        self.navigation_activity
            .emit_deferred_load_completion_async(conn, out, pending_download, renderer_document)
            .await;
    }
}

impl PendingDeferredMainDocumentLoadCompletionActivity {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.completion.session_id()
    }

    pub(crate) fn renderer_page_residence_identity(&self) -> Option<RendererPageResidenceIdentity> {
        self.completion.renderer_page_residence_identity()
    }

    pub(crate) fn renderer_document_identity(&self) -> Option<RendererDocumentLifecycleIdentity> {
        self.completion.renderer_document_identity()
    }

    pub(crate) fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.completion.observation_id()
    }

    pub(crate) async fn wait(self) -> CompletedDeferredMainDocumentLoadCompletionActivity {
        let DeferredMainDocumentLoadCompletionActivity {
            state,
            observation_id,
            renderer_page_residence_identity: _,
            lifecycle_observer,
        } = self.completion;
        let observation = lifecycle_observer.wait().await;
        CompletedDeferredMainDocumentLoadCompletionActivity {
            state,
            observation_id,
            observation,
        }
    }
}

impl CompletedDeferredMainDocumentLoadCompletionActivity {
    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.state.owner_scope
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.state.owner_scope.session_id()
    }

    pub(crate) fn observation_id(&self) -> DeferredMainDocumentLoadObservationId {
        self.observation_id
    }

    pub(crate) async fn emit_async(
        mut self,
        conn: &mut CdpConnection,
        out: &mut BackgroundProtocolEventBuffer,
    ) {
        match self.observation {
            RendererDocumentLifecycleObservation::Reached => {}
            RendererDocumentLifecycleObservation::Pending => {
                unreachable!("a completed lifecycle observer cannot remain pending")
            }
            RendererDocumentLifecycleObservation::Interrupted => {
                self.state
                    .navigation_activity
                    .emit_renderer_load_boundary_facts(conn, out);
                return;
            }
            RendererDocumentLifecycleObservation::Superseded
            | RendererDocumentLifecycleObservation::Unavailable => {
                conn.cancel_renderer_document_load_visibility_barrier_for_session_owner(
                    self.session_id(),
                    &self.state.navigation_activity.state.loader_id,
                );
                return;
            }
        }
        self.state
            .emit_load_completion_after_lifecycle_ready_async(conn, out)
            .await;
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
                && self.state.result_projection.protocol() == DevToolsProtocol::Cdp
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

impl MainDocumentDownloadNavigationActivity {
    pub(crate) fn new(
        navigation_activity: MainDocumentNavigationActivity,
        body_artifact: CompletedDownloadBodyArtifact,
    ) -> Self {
        Self {
            navigation_activity,
            body_artifact,
        }
    }

    pub(crate) async fn emit_commit_into_buffer_async(
        mut self,
        conn: &mut CdpConnection,
        out: &mut CommandOutputBuffer,
        command_context: &mut CommandDispatchContext,
    ) {
        self.navigation_activity
            .emit_download_navigation_commit_into_buffer_async(
                conn,
                out,
                self.body_artifact,
                command_context,
            )
            .await;
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
            navigate_id: Some(77),
            navigate_session_id: Some("SID-nav".to_owned()),
            result_projection: NavigationResultProjection::Cdp(
                json!({ "frameId": "FRAME-1", "loaderId": "LID-1" }),
            ),
            frame_id: "FRAME-1".to_owned(),
            session_id: Some("SID-page".to_owned()),
            request_id: Some("REQ-1".to_owned()),
            loader_id: "LID-1".to_owned(),
            request_announced: false,
            requested_url: Url::parse("https://example.test/start").unwrap(),
            request_method: "GET".to_owned(),
            request_body: None,
            request_body_bytes: None,
            request_headers: vec![("Accept".to_owned(), "text/html".to_owned())],
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
        let mut browser_context = BrowserContext::new("BID-deferred-load-observer".to_owned());
        browser_context.set_active_target_id("TID-deferred-load-observer");
        browser_context.attach_active_session("SID-nav");
        browser_context.set_target_url("https://example.test/start".to_owned());
        browser_context
            .active_target
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.browser_context = Some(browser_context);

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
        let (binding, accepted) = conn.bind_renderer_document_lifecycle_for_session_owner(
            Some("SID-nav"),
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

    fn deferred_load_admission_for_test(
        conn: &CdpConnection,
        binding: CommittedRendererDocumentBinding,
    ) -> DeferredMainDocumentLoadCompletionAdmission {
        let navigation_activity = MainDocumentNavigationActivity::new(
            navigation_state(),
            Url::parse("https://example.test/start").unwrap(),
            empty_main_document_progress_gate_for_test(),
            None,
        );
        DeferredMainDocumentLoadCompletionAdmission::new(navigation_activity, None)
            .with_renderer_document_binding(Some(binding))
            .with_owner_scope(CommandOwnerScope::capture(conn, Some("SID-nav")))
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
                    "application/octet-stream".to_owned(),
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
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.attach_active_session("SID-page");
        browser_context
            .active_target
            .runtime_slot
            .enable_primary_network_events();
        conn.browser_context = Some(browser_context);

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

    #[test]
    fn failed_cdp_navigation_returns_error_text_result_after_network_terminal() {
        let mut output = CommandOutputBuffer::default();
        MainDocumentFailedNavigationActivity::new(
            navigation_state(),
            empty_main_document_progress_gate_for_test(),
            FailedNavigationResponseMode::CdpErrorTextResult,
        )
        .emit_navigation_error_into_buffer(&mut output, "net::ERR_CONNECTION_RESET");
        let mut messages = Vec::new();
        output
            .into_plan()
            .emit_into(&mut messages, Some(77), Some("SID-nav"));

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], json!(77));
        assert_eq!(messages[0]["result"]["frameId"], json!("FRAME-1"));
        assert_eq!(messages[0]["result"]["loaderId"], json!("LID-1"));
        assert_eq!(
            messages[0]["result"]["errorText"],
            json!("net::ERR_CONNECTION_RESET")
        );
        assert_eq!(messages[0]["result"]["isDownload"], json!(false));
        assert!(messages[0].get("error").is_none());
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
        browser_context
            .devtools_session_state
            .page_session_state
            .page_domain_enabled = true;
        let mut conn = CdpConnection::new();
        conn.browser_context = Some(browser_context);
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

        activity
            .emit_loaded_navigation_commit_async(
                &mut conn,
                &mut output,
                None,
                Some(renderer_document_binding),
                renderer_lifecycle_events,
                None,
            )
            .await;

        let mut out = Vec::new();
        output.into_plan().emit_into(
            &mut out,
            state.navigate_id,
            state.navigate_session_id.as_deref(),
        );
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
        assert!(
            conn.runtime_session_owner_slot(Some("SID-nav"))
                .expect("owner slot should exist")
                .loaded_page()
                .is_none(),
            "main-document lifecycle emission must not read back from a live page"
        );
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
        conn.browser_context = Some(browser_context);
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
        conn.browser_context = Some(browser_context);
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

    #[tokio::test]
    async fn deferred_load_observer_waits_for_load_not_domcontentloaded() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        let observer = conn.register_exact_renderer_document_lifecycle_observer_for_session_owner(
            Some("SID-nav"),
            Some(&binding),
            RendererDocumentLifecycleMilestone::Load,
        );
        assert_eq!(
            observer.observation(),
            RendererDocumentLifecycleObservation::Pending,
            "DOMContentLoaded must not satisfy an exact Load observer"
        );

        let load = renderer_load_event_for_test(started);
        let (_, accepted) = conn.ingest_renderer_document_lifecycle_events_for_session_owner(
            Some("SID-nav"),
            vec![load],
        );
        assert_eq!(accepted, vec![load]);
        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Reached
        );
    }

    #[tokio::test]
    async fn deferred_load_terminal_before_adapter_wait_is_not_lost() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        let admission = deferred_load_admission_for_test(&conn, binding);
        conn.enqueue_deferred_main_document_load_completion(admission);
        let work = take_deferred_load_work_for_test(&mut conn);

        let load = renderer_load_event_for_test(started);
        let _ = conn.ingest_renderer_document_lifecycle_events_for_session_owner(
            Some("SID-nav"),
            vec![load],
        );
        let pending = work.start_main_document_load_wait();
        let observation_id = pending.observation_id();
        let completed = tokio::time::timeout(std::time::Duration::from_secs(1), pending.wait())
            .await
            .expect("an already-published Load terminal must complete without another wake");

        assert_eq!(completed.observation_id(), observation_id);
    }

    #[test]
    fn same_owner_load_admissions_publish_distinct_ordered_work() {
        let (mut conn, binding, _) = connection_with_dcl_only_renderer_lifecycle();
        let first = deferred_load_admission_for_test(&conn, binding.clone());
        let second = deferred_load_admission_for_test(&conn, binding);
        conn.enqueue_deferred_main_document_load_completion(first);
        conn.enqueue_deferred_main_document_load_completion(second);

        let events = conn.take_scheduler_events();
        assert_eq!(events.len(), 2);
        let sequences = events
            .into_iter()
            .map(|event| {
                let crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } = event else {
                    panic!("each load admission must publish concrete scheduler work");
                };
                assert_eq!(
                    work.kind(),
                    ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                );
                assert_eq!(work.main_document_load_session_id(), Some("SID-nav"));
                work.publish_sequence().get()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            sequences,
            [1, 2],
            "same-session load actions must not collapse into one source-shaped capture"
        );
    }

    #[tokio::test]
    async fn deferred_load_work_remains_resident_without_republication_until_terminal() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        let admission = deferred_load_admission_for_test(&conn, binding);
        conn.enqueue_deferred_main_document_load_completion(admission);
        let work = take_deferred_load_work_for_test(&mut conn);
        assert!(
            !work.is_ready(),
            "DOMContentLoaded alone must leave the exact load owner action pending"
        );

        let load = renderer_load_event_for_test(started);
        let _ = conn.ingest_renderer_document_lifecycle_events_for_session_owner(
            Some("SID-nav"),
            vec![load],
        );
        assert!(
            conn.take_scheduler_events().is_empty(),
            "a typed lifecycle terminal wakes the existing observer and must not rebuild a source-shaped scheduler ticket"
        );
        assert!(
            work.is_ready(),
            "the same durable owner-action residence must observe the terminal"
        );
        let outcome = conn.complete_ready_protocol_scheduler_work_turn(work).await;
        let (_, scheduler_events) = outcome.into_protocol_event_parts();
        assert!(
            scheduler_events.iter().all(|event| !matches!(
                event,
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.kind()
                        == ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
            )),
            "consuming the exact owner action must not recreate another load residence"
        );
    }

    #[tokio::test]
    async fn deferred_load_observer_reports_exact_document_interruption() {
        let (mut conn, binding, started) = connection_with_dcl_only_renderer_lifecycle();
        let observer = conn.register_exact_renderer_document_lifecycle_observer_for_session_owner(
            Some("SID-nav"),
            Some(&binding),
            RendererDocumentLifecycleMilestone::Load,
        );
        let terminated = renderer_termination_event_for_test(started);
        let _ = conn.ingest_renderer_document_lifecycle_events_for_session_owner(
            Some("SID-nav"),
            vec![terminated],
        );

        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Interrupted
        );
    }

    #[tokio::test]
    async fn newer_document_navigation_supersedes_deferred_load_observer() {
        let (mut conn, binding, _) = connection_with_dcl_only_renderer_lifecycle();
        let observer = conn.register_exact_renderer_document_lifecycle_observer_for_session_owner(
            Some("SID-nav"),
            Some(&binding),
            RendererDocumentLifecycleMilestone::Load,
        );
        conn.start_document_navigation_for_session_owner(Some("SID-nav"), "LID-2".to_owned())
            .expect("replacement navigation token");

        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Superseded
        );
    }

    #[tokio::test]
    async fn losing_page_slot_terminates_deferred_load_observer() {
        let (mut conn, binding, _) = connection_with_dcl_only_renderer_lifecycle();
        let observer = conn.register_exact_renderer_document_lifecycle_observer_for_session_owner(
            Some("SID-nav"),
            Some(&binding),
            RendererDocumentLifecycleMilestone::Load,
        );
        conn.browser_context = None;

        assert_eq!(
            observer.wait().await,
            RendererDocumentLifecycleObservation::Unavailable
        );
    }

    #[tokio::test]
    async fn superseded_deferred_load_completion_is_consumed_through_its_observer() {
        let (mut conn, binding, _) = connection_with_dcl_only_renderer_lifecycle();
        let old_completion = deferred_load_admission_for_test(&conn, binding);
        conn.enqueue_deferred_main_document_load_completion(old_completion);
        let work = take_deferred_load_work_for_test(&mut conn);

        conn.start_document_navigation_for_session_owner(Some("SID-nav"), "LID-2".to_owned())
            .expect("replacement navigation token");

        assert!(
            conn.take_scheduler_events().is_empty(),
            "replacement must resolve the existing observer without manufacturing another scheduler admission"
        );
        assert!(work.is_ready(), "replacement must publish Superseded");
        let pending = work.start_main_document_load_wait();
        let completed = pending.wait().await;
        let _ = conn
            .complete_deferred_main_document_load_completion_for_scheduler(completed)
            .await;
    }
}
