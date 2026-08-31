use moli_core::{
    RendererOwnerAction, RendererProtocolObservation, RendererRuntimeCommandCausalIdentity,
};

use super::super::contextual_projection::ProtocolOutputProjectionContext;
use super::super::output_payloads::{ProtocolOutputPayload, ProtocolOutputPayloads};
use super::super::output_slot::{
    ProtocolOutputDelivery, ProtocolOutputResponseOrder, ProtocolOutputSink, ProtocolOutputSlot,
};
use crate::conn::{CdpConnection, CommandDispatchContext};
use crate::domains::page::{PagePreparedOutputSlot, SLOT_TOP_LEVEL_LOCATION_NAVIGATION};

/// One move-owned protocol projection batch prepared from concrete output.
///
/// The batch owns typed payload exactly once and preserves the producer's
/// explicit FIFO order. It cannot select renderer work or inspect current renderer state. Consumers
/// must either project it, hold its remaining after-response slots behind an
/// exact command barrier, or consume only its owner actions during stale
/// cleanup.
#[derive(Debug)]
#[must_use = "prepared protocol outputs must be projected, held, or cleaned up exactly once"]
pub(in crate::domains::activity) struct PreparedProtocolOutputs {
    ordered_slots: Vec<ProtocolOutputSlot>,
    payloads: ProtocolOutputPayloads,
    emit_root_network_idle_after_projection: bool,
}

impl PreparedProtocolOutputs {
    pub(in crate::domains::activity) fn empty() -> Self {
        Self {
            ordered_slots: Vec::new(),
            payloads: ProtocolOutputPayloads::default(),
            emit_root_network_idle_after_projection: false,
        }
    }

    /// Applies one concrete renderer Network fact and freezes both live
    /// Network and live Log projection at the same ingress boundary.
    ///
    /// The resulting prepared tokens are move-owned by this publication.
    /// Projection cannot later scan the target backlog, while `Network.enable`
    /// and `Log.enable` retain their independent Chromium-compatible policies.
    pub(in crate::domains::activity) fn from_renderer_network_observation(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        source_renderer_page: Option<crate::conn::RendererPageResidenceIdentity>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Option<Self> {
        let renderer_live = conn
            .ingest_renderer_page_network_output_item_and_prepare_live_delivery_for_session_owner(
                session_id,
                source_renderer_page,
                source_document,
                item,
            )?;

        let mut prepared = Self::empty();
        crate::domains::observable_output::live_log_prepared_outputs_for_renderer_network_fact(
            conn, session_id,
        )
        .append_to_output_sink(&mut prepared);
        crate::domains::network::NetworkPreparedOutputs::from_renderer_live_delivery(renderer_live)
            .append_to_output_sink(&mut prepared);
        prepared.emit_root_network_idle_after_projection = true;
        Some(prepared)
    }

    pub(in crate::domains::activity) async fn from_renderer_observation(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        source_renderer_agent: moli_core::page::RendererDevToolsAgentToken,
        observation: &RendererProtocolObservation,
    ) -> Self {
        let mut prepared = Self::empty();
        match observation {
            RendererProtocolObservation::MainDocumentCommit(commit) => {
                crate::domains::page::append_renderer_main_document_commit_to_output_sink(
                    commit.clone(),
                    &mut prepared,
                );
            }
            RendererProtocolObservation::DocumentTitleChanged(change) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_document_title_change(
                    change.clone(),
                )
                .append_to_document_title_output_sink(&mut prepared);
            }
            RendererProtocolObservation::DocumentLifecycle(event) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_document_lifecycle_event(
                    *event,
                )
                .append_to_document_lifecycle_output_sink(&mut prepared);
            }
            RendererProtocolObservation::Network { .. } => unreachable!(
                "renderer Network facts require the ingress-bound live projection constructor"
            ),
            RendererProtocolObservation::RuntimeBinding(call) => {
                crate::domains::runtime::RuntimePreparedOutputs::
                    from_renderer_runtime_binding_call(conn, session_id, call.clone())
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::DomMutations(batch) => {
                crate::domains::dom::DomPreparedOutputs::
                    from_renderer_dom_mutation_event_batches_for_stream(
                        conn,
                        session_id,
                        source_renderer_agent,
                        std::slice::from_ref(batch),
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::RuntimeInspector(batch) => {
                let source_batches = vec![batch.clone()];
                let mut batches = conn.route_current_renderer_inspector_output_for_session_owner(
                    session_id,
                    source_batches.clone(),
                );
                if batches.is_empty() {
                    crate::domains::runtime::RuntimePreparedOutputs::
                        from_retired_renderer_runtime_inspector_session_responses(
                            conn,
                            session_id,
                            &source_batches,
                        )
                        .append_to_output_sink(&mut prepared);
                    return prepared;
                }
                for batch in &mut batches {
                    let Some(attachment) = conn
                        .target_page_protocol_attachment_identity_for_renderer_inspector_route(
                            session_id,
                            batch.session.wire_session_id(),
                        )
                    else {
                        continue;
                    };
                    let output_session_id = attachment.session_id().map(str::to_owned);
                    let normalize_node_objects = !conn
                        .runtime_inspector_pause_active_for_session_owner(
                            output_session_id.as_deref(),
                        );
                    for message in &mut batch.messages {
                        let moli_core::page::RendererRuntimeInspectorMessage::Protocol(message) =
                            message
                        else {
                            continue;
                        };
                        let mut value = message.value_mut();
                        conn.normalize_runtime_event_context_ids_for_session_owner_async(
                            output_session_id.as_deref(),
                            &mut value,
                        )
                        .await;
                        if normalize_node_objects {
                            conn.normalize_node_remote_objects_for_session_owner_async(
                                output_session_id.as_deref(),
                                &mut value,
                            )
                            .await;
                        }
                    }
                }
                crate::domains::runtime::RuntimePreparedOutputs::
                    from_renderer_runtime_inspector_message_batches(
                        conn,
                        session_id,
                        &batches,
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::RuntimeConsole(message) => {
                crate::domains::observable_output::runtime_console_message_prepared_outputs(
                    conn,
                    message.clone(),
                    session_id,
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::InspectorIssue {
                source_document,
                issue,
            } => {
                crate::domains::observable_output::inspector_issue_prepared_outputs(
                    conn,
                    *source_document,
                    issue.clone(),
                    session_id,
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::WindowOpen(event) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_window_open_event(
                    conn,
                    session_id,
                    event.clone(),
                )
                .append_to_window_open_output_sink(&mut prepared);
            }
            RendererProtocolObservation::RuntimeLifecycleError {
                text,
                execution_context_id,
            } => {
                crate::domains::observable_output::runtime_lifecycle_error_prepared_outputs(
                    conn,
                    text.clone(),
                    *execution_context_id,
                    session_id,
                )
                .append_to_output_sink(&mut prepared);
            }
        }
        prepared
    }

    pub(in crate::domains::activity) async fn from_protocol_local_command_boundary(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
    ) -> Self {
        let mut outputs = Self::empty();
        outputs
            .append_protocol_local_outputs(conn, session_id)
            .await;
        outputs
    }

    pub(in crate::domains::activity) async fn from_renderer_owner_action(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        action: RendererOwnerAction,
    ) -> Self {
        let mut prepared = Self::empty();
        match action {
            RendererOwnerAction::FileChooser(activation) => {
                crate::domains::input::InputPreparedOutputs::from_renderer_file_chooser_activation(
                    conn, session_id, activation,
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::Download(activation) => {
                crate::domains::input::InputPreparedOutputs::from_renderer_download_activation(
                    activation,
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::JavaScriptDialog(dialog) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_javascript_dialog(
                    conn, session_id, dialog,
                )
                .append_to_javascript_dialog_output_sink(&mut prepared);
            }
            RendererOwnerAction::Popup(activation) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_popup_activation(
                    conn, session_id, activation,
                )
                .append_to_popup_output_sink(&mut prepared);
            }
            RendererOwnerAction::ChildFrameTree {
                source_document,
                event,
            } => {
                crate::domains::page::PagePreparedOutputs::from_renderer_child_frame_tree_event(
                    conn,
                    session_id,
                    source_document,
                    event,
                )
                .append_to_child_frame_output_sink(&mut prepared);
            }
            RendererOwnerAction::ChildFrameDocumentOpened {
                source_document,
                event,
            } => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_child_frame_document_opened(
                        conn,
                        session_id,
                        source_document,
                        event,
                    )
                    .append_to_child_frame_output_sink(&mut prepared);
            }
            RendererOwnerAction::ChildFrameDocumentNetwork {
                source_document,
                event,
            } => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_child_frame_document_network(
                        conn,
                        session_id,
                        source_document,
                        event,
                    )
                    .append_to_child_frame_output_sink(&mut prepared);
            }
            RendererOwnerAction::ChildFrameLoad {
                source_document,
                event,
            } => {
                crate::domains::page::PagePreparedOutputs::from_renderer_child_frame_load(
                    conn,
                    session_id,
                    source_document,
                    event,
                )
                .append_to_child_frame_output_sink(&mut prepared);
            }
            RendererOwnerAction::SameDocumentNavigation(navigation) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_same_document_navigation(
                    conn, session_id, navigation,
                )
                .append_to_same_document_navigation_output_sink(&mut prepared);
            }
            RendererOwnerAction::TopLevelLocationNavigation(navigation) => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_top_level_location_navigation(
                        conn,
                        session_id,
                        navigation,
                    )
                    .append_to_top_level_location_navigation_output_sink(&mut prepared);
            }
            RendererOwnerAction::TopLevelHistoryTraversal(traversal) => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_top_level_history_traversal(traversal)
                    .append_to_top_level_history_traversal_output_sink(&mut prepared);
            }
            RendererOwnerAction::SubresourceFetchPause {
                source_document,
                info,
            } => {
                crate::domains::fetch::
                    subresource_fetch_pause_prepared_outputs_for_renderer_record_async(
                        conn,
                        session_id,
                        source_document,
                        *info,
                    )
                    .await
                    .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::SubresourceContinue {
                source_document,
                event,
            } => {
                crate::domains::network::NetworkPreparedOutputs::
                    from_renderer_subresource_continue(
                        conn,
                        session_id,
                        source_document,
                        *event,
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::DetachedParserScriptFetchPause {
                source_document,
                info,
                continuation,
            } => {
                crate::domains::fetch::
                    detached_parser_script_fetch_pause_prepared_outputs_for_renderer_record_async(
                        conn,
                        session_id,
                        source_document,
                        *info,
                        continuation,
                    )
                    .await
                    .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::SharedWorkerTargetLifecycle(event) => {
                if let Some((browser_context_id, _)) =
                    conn.target_owner_identity_for_session(session_id)
                {
                    crate::domains::target::
                        shared_worker_target_lifecycle_prepared_outputs_for_event(
                            conn,
                            browser_context_id,
                            event,
                        )
                        .append_to_shared_worker_target_lifecycle_output_sink(
                            &mut prepared,
                        );
                }
            }
            RendererOwnerAction::ServiceWorkerTargetLifecycle(event) => {
                if let Some((browser_context_id, _)) =
                    conn.target_owner_identity_for_session(session_id)
                {
                    crate::domains::target::
                        service_worker_target_lifecycle_prepared_outputs_for_event(
                            conn,
                            browser_context_id,
                            event,
                        )
                        .append_to_service_worker_target_lifecycle_output_sink(
                            &mut prepared,
                        );
                }
            }
            RendererOwnerAction::DedicatedWorkerTargetLifecycle(event) => {
                crate::domains::target::
                    dedicated_worker_target_lifecycle_prepared_outputs_for_event(
                        conn,
                        session_id,
                        event,
                    )
                    .append_to_dedicated_worker_target_lifecycle_output_sink(&mut prepared);
            }
        }
        prepared
    }

    #[cfg(test)]
    pub(in crate::domains::activity) fn from_top_level_location_navigation_for_test(
        owner: crate::conn::TargetPageResidenceIdentity,
        navigation: moli_core::page::RendererDocumentSourcedTopLevelLocationNavigation,
    ) -> Self {
        Self {
            ordered_slots: vec![SLOT_TOP_LEVEL_LOCATION_NAVIGATION],
            payloads: ProtocolOutputPayloads::from_slot(
                PagePreparedOutputSlot::from_outputs(
                    crate::domains::page::PagePreparedOutputs::from_top_level_location_navigation_for_test(
                        owner,
                        Some(navigation),
                    ),
                ),
            ),
            emit_root_network_idle_after_projection: false,
        }
    }

    pub(in crate::domains::activity) fn top_level_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.payloads
            .page()
            .and_then(PagePreparedOutputSlot::top_level_location_navigation_runtime_command_cause)
    }

    pub(in crate::domains::activity) fn take_top_level_location_navigation_for_runtime_command(
        &mut self,
        cause: &RendererRuntimeCommandCausalIdentity,
    ) -> Option<Self> {
        let navigation_payload = self
            .payloads
            .page_mut()?
            .take_top_level_location_navigation_for_runtime_command(cause)?;
        self.ordered_slots
            .retain(|slot| *slot != SLOT_TOP_LEVEL_LOCATION_NAVIGATION);
        Some(Self {
            ordered_slots: vec![SLOT_TOP_LEVEL_LOCATION_NAVIGATION],
            payloads: ProtocolOutputPayloads::from_slot(navigation_payload),
            emit_root_network_idle_after_projection: false,
        })
    }

    async fn project_slots_async(
        &mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        command_context: &mut CommandDispatchContext,
        slots: Vec<ProtocolOutputSlot>,
    ) {
        for slot in slots {
            let trace_started =
                moli_trace::cdp_runtime_trace_enabled().then(std::time::Instant::now);
            let before_events = command_context.protocol_events_len();
            if trace_started.is_some() {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "protocol_output_projection_start",
                    session_id = ?session_id,
                    slot = ?slot,
                );
            }
            slot.project_async(
                conn,
                &mut ProtocolOutputProjectionContext::new(session_id, command_context),
                Some(&mut self.payloads),
            )
            .await;
            if let Some(started) = trace_started {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "protocol_output_projection_done",
                    session_id = ?session_id,
                    slot = ?slot,
                    events_added = command_context.protocol_events_len().saturating_sub(before_events),
                    elapsed_us = %started.elapsed().as_micros(),
                );
            }
        }
    }

    fn emit_root_network_idle_if_ready(
        &mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        command_context: &mut CommandDispatchContext,
    ) {
        if std::mem::take(&mut self.emit_root_network_idle_after_projection) {
            conn.emit_root_network_idle_for_session_owner(
                session_id,
                command_context.protocol_events_mut(),
            );
        }
    }

    pub(in crate::domains::activity) async fn project_async(
        mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        command_context: &mut CommandDispatchContext,
    ) {
        let slots = std::mem::take(&mut self.ordered_slots);
        self.project_slots_async(conn, session_id, command_context, slots)
            .await;
        self.emit_root_network_idle_if_ready(conn, session_id, command_context);
    }

    /// Projects output explicitly allowed before a Runtime command response
    /// and returns the remaining concrete prepared output as a held batch.
    ///
    /// The payload container is shared across the two phases: each
    /// before-response projection consumes only its own typed fields, leaving
    /// after-response fields in the returned batch. This avoids cloning
    /// one-shot owner actions or recapturing current target state.
    pub(in crate::domains::activity) async fn project_before_command_response_and_hold_after(
        mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        command_context: &mut CommandDispatchContext,
    ) -> Option<Self> {
        let mut before_response = Vec::new();
        let mut after_response = Vec::new();
        for slot in std::mem::take(&mut self.ordered_slots) {
            match slot.command_response_order() {
                ProtocolOutputResponseOrder::BeforeResponse => before_response.push(slot),
                ProtocolOutputResponseOrder::AfterResponse => after_response.push(slot),
            }
        }
        self.project_slots_async(conn, session_id, command_context, before_response)
            .await;
        // A concrete body-finished fact can make the exact committed Document
        // idle. Emit that derived lifecycle event after the corresponding
        // Network output and at the same pre-response boundary.
        self.emit_root_network_idle_if_ready(conn, session_id, command_context);
        self.ordered_slots = after_response;
        (!self.ordered_slots.is_empty()).then_some(self)
    }

    /// Completes browser-owner cleanup for a canceled or superseded barrier
    /// without projecting protocol-only observations from the stale command.
    pub(in crate::domains::activity) async fn project_owner_actions_async(
        mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        command_context: &mut CommandDispatchContext,
    ) {
        let owner_actions = std::mem::take(&mut self.ordered_slots)
            .into_iter()
            .filter(|slot| slot.delivery() == ProtocolOutputDelivery::OwnerAction)
            .collect();
        self.project_slots_async(conn, session_id, command_context, owner_actions)
            .await;
    }

    fn push_output_slot(&mut self, slot: ProtocolOutputSlot) {
        if !self.ordered_slots.contains(&slot) {
            self.ordered_slots.push(slot);
        }
    }

    async fn append_protocol_local_outputs(
        &mut self,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
    ) {
        crate::domains::dom_storage::append_pending_dom_storage_outputs_for_session_owner(
            conn, session_id, self,
        );
    }
}

impl ProtocolOutputSink for PreparedProtocolOutputs {
    fn push_produced_slot(&mut self, slot: ProtocolOutputSlot) {
        self.push_output_slot(slot);
    }

    fn push_prepared_payload(&mut self, payload: ProtocolOutputPayload) {
        self.payloads.extend_payload(payload);
    }
}
