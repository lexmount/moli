use anyhow::Result;

use moli_core::{
    RendererOwnerAction, RendererProtocolObservation, RendererRuntimeCommandCausalIdentity,
};

use super::super::contextual_projection::ProtocolOutputProjectionContext;
use super::super::output_payloads::{ProtocolOutputPayload, ProtocolOutputPayloads};
use super::super::output_slot::{
    ProtocolOutputDelivery, ProtocolOutputResponseOrder, ProtocolOutputSink, ProtocolOutputSlot,
};
use crate::conn::{CdpConnection, CommandDispatchContext, CommandOwnerScope};
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
pub(crate) struct PreparedProtocolOutputs {
    ordered_slots: Vec<ProtocolOutputSlot>,
    payloads: ProtocolOutputPayloads,
    emit_root_network_idle_after_projection: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum ProtocolOutputProjectionMode {
    All,
    BeforeResponse,
    OwnerActions,
}

impl ProtocolOutputProjectionMode {
    fn admits(self, slot: ProtocolOutputSlot) -> bool {
        match self {
            Self::All => true,
            Self::BeforeResponse => {
                slot.command_response_order() == ProtocolOutputResponseOrder::BeforeResponse
            }
            Self::OwnerActions => slot.delivery() == ProtocolOutputDelivery::OwnerAction,
        }
    }
}

impl PreparedProtocolOutputs {
    #[cfg(test)]
    pub(in crate::domains) fn from_slots_for_test(
        slots: impl IntoIterator<Item = ProtocolOutputSlot>,
        payloads: ProtocolOutputPayloads,
    ) -> Self {
        Self {
            ordered_slots: slots.into_iter().collect(),
            payloads,
            emit_root_network_idle_after_projection: false,
        }
    }

    pub(in crate::domains) fn from_worker_target_removal(
        outputs: crate::domains::target::TargetPreparedOutputSlot,
    ) -> Self {
        let mut prepared = Self::empty();
        if outputs.is_empty() {
            return prepared;
        }
        prepared.push_produced_slot(ProtocolOutputSlot::SharedWorkerTargetLifecycle);
        prepared.push_produced_slot(ProtocolOutputSlot::ServiceWorkerTargetLifecycle);
        prepared.push_prepared_payload(outputs.into());
        prepared
    }

    pub(crate) async fn project_using(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        owner: CommandOwnerScope,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let (_, error) = self
            .project_selected_using(driver, owner, context, ProtocolOutputProjectionMode::All)
            .await?;
        error.map_or(Ok(()), Err)
    }

    pub(crate) async fn project_owner_actions_using(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        owner: CommandOwnerScope,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let (_, error) = self
            .project_selected_using(
                driver,
                owner,
                context,
                ProtocolOutputProjectionMode::OwnerActions,
            )
            .await?;
        error.map_or(Ok(()), Err)
    }

    async fn project_selected_using(
        mut self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        owner: CommandOwnerScope,
        context: &mut crate::conn::CommandWorkflowContext,
        mode: ProtocolOutputProjectionMode,
    ) -> Result<(Self, Option<anyhow::Error>)> {
        let mut first_error = None;
        context.materialize_projection_output();
        while self
            .ordered_slots
            .iter()
            .copied()
            .any(|slot| mode.admits(slot))
        {
            let step_owner = owner.clone();
            let (remaining, pending) = context.with_state(driver, move |state, context| {
                state.prepare_protocol_output_projection_step(self, step_owner, context, mode)
            })?;
            self = remaining;
            if let Some(pending) = pending
                && let Err(error) = pending.run(driver, context).await
            {
                first_error.get_or_insert(error);
            }
            context.materialize_projection_output();
        }
        self = context.with_state(driver, move |state, context| {
            state.finish_protocol_output_projection(self, owner, context, mode)
        })?;
        Ok((self, first_error))
    }

    pub(crate) fn prepare_next_projection_step(
        mut self,
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        context: &mut crate::conn::CommandWorkflowContext,
        mode: ProtocolOutputProjectionMode,
    ) -> (
        Self,
        Option<super::super::output_slot::PreparedProtocolOutputWait>,
    ) {
        while let Some(index) = self
            .ordered_slots
            .iter()
            .position(|slot| mode.admits(*slot))
        {
            let slot = self.ordered_slots.remove(index);
            let pending = if slot
                .project_finite(
                    conn,
                    &mut ProtocolOutputProjectionContext::new(&owner, context.context_mut()),
                    &mut self.payloads,
                )
                .is_ok()
            {
                None
            } else {
                slot.prepare_wait(conn, &owner, &mut self.payloads, context)
            };
            context.materialize_projection_output();
            if pending.is_some() {
                return (self, pending);
            }
        }
        (self, None)
    }

    pub(crate) fn finish_projection(
        mut self,
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        context: &mut crate::conn::CommandWorkflowContext,
        mode: ProtocolOutputProjectionMode,
    ) -> Self {
        if !matches!(mode, ProtocolOutputProjectionMode::OwnerActions) {
            self.emit_root_network_idle_if_ready(conn, &owner, context.context_mut());
        }
        self
    }

    pub(in crate::domains::activity) fn has_after_response_slots(&self) -> bool {
        self.ordered_slots
            .iter()
            .any(|slot| slot.command_response_order() == ProtocolOutputResponseOrder::AfterResponse)
    }

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
        owner: &CommandOwnerScope,
        source_renderer_page: Option<crate::conn::RendererPageResidenceIdentity>,
        source_document: moli_core::RendererDocumentLifecycleIdentity,
        item: &moli_core::page::ScriptNetworkOutputItem,
    ) -> Option<Self> {
        let renderer_live = conn
            .ingest_renderer_page_network_output_item_and_prepare_live_delivery_for_owner(
                owner,
                source_renderer_page,
                source_document,
                item,
            )?;

        let mut prepared = Self::empty();
        crate::domains::observable_output::live_log_prepared_outputs_for_renderer_network_fact(
            conn, owner,
        )
        .append_to_output_sink(&mut prepared);
        crate::domains::network::NetworkPreparedOutputs::from_renderer_live_delivery(renderer_live)
            .append_to_output_sink(&mut prepared);
        prepared.emit_root_network_idle_after_projection = true;
        Some(prepared)
    }

    pub(in crate::domains::activity) fn from_renderer_observation(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        source_residence: moli_core::RendererOutputResidenceIdentity,
        observation: &RendererProtocolObservation,
    ) -> Self {
        let mut prepared = Self::empty();
        match observation {
            RendererProtocolObservation::WebMcp(observation) => {
                crate::domains::web_mcp::append_observation(
                    conn,
                    owner,
                    observation,
                    &mut prepared,
                );
            }
            RendererProtocolObservation::MainDocumentCommit {
                source_document,
                commit,
            } => {
                // Native destruction may already have been retained for a
                // debugger-paused failed Worker. A new owner Document ends
                // that deferral even though no second Destroyed can arrive.
                let mut retired = crate::domains::target::TargetPreparedOutputs::default();
                for outputs in
                    crate::domains::target::prepare_failed_dedicated_worker_retirements_for_owner(
                        conn, owner,
                    )
                {
                    retired.extend(outputs);
                }
                retired.append_to_dedicated_worker_target_lifecycle_output_sink(&mut prepared);
                crate::domains::page::append_renderer_main_document_commit_to_output_sink(
                    *source_document,
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
                let Some(renderer_page) =
                    crate::conn::RendererPageResidenceIdentity::from_residence(source_residence)
                else {
                    return prepared;
                };
                crate::domains::runtime::RuntimePreparedOutputs::
                    from_renderer_runtime_binding_call(conn, owner, renderer_page, call.clone())
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::DomMutations(batch) => {
                crate::domains::dom::DomPreparedOutputs::from_renderer_dom_mutation_event_batches(
                    conn,
                    owner,
                    std::slice::from_ref(batch),
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::RuntimeInspector(batch) => {
                let worker_output = match source_residence {
                    moli_core::RendererOutputResidenceIdentity::SharedWorker { .. }
                    | moli_core::RendererOutputResidenceIdentity::ServiceWorker { .. } => true,
                    moli_core::RendererOutputResidenceIdentity::Page { .. } => batch
                        .session
                        .wire_session_id()
                        .and_then(|session_id| conn.session_route(Some(session_id)))
                        .is_some_and(|route| {
                            matches!(
                                route,
                                crate::conn::CdpSessionRoute::DedicatedWorkerTarget { .. }
                            )
                        }),
                };
                if worker_output {
                    crate::domains::runtime::RuntimePreparedOutputs::
                        from_worker_renderer_runtime_inspector_message_batch(
                            conn,
                            source_residence,
                            batch,
                        )
                        .append_to_output_sink(&mut prepared);
                    return prepared;
                }
                let Some(renderer_page) =
                    crate::conn::RendererPageResidenceIdentity::from_residence(source_residence)
                else {
                    return prepared;
                };
                crate::domains::runtime::RuntimePreparedOutputs::
                    from_page_renderer_runtime_inspector_message_batches(
                        conn,
                        owner,
                        renderer_page,
                        vec![batch.clone()],
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::RuntimeConsole(message) => {
                crate::domains::observable_output::runtime_console_message_prepared_outputs(
                    conn,
                    message.clone(),
                    owner,
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
                    owner,
                )
                .append_to_output_sink(&mut prepared);
            }
            RendererProtocolObservation::WindowOpen(event) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_window_open_event(
                    conn,
                    owner,
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
                    owner,
                )
                .append_to_output_sink(&mut prepared);
            }
        }
        prepared
    }

    pub(in crate::domains::activity) fn from_renderer_owner_action(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        action: RendererOwnerAction,
        page_source: Option<(
            crate::conn::TargetPageResidenceIdentity,
            crate::conn::RendererPageResidenceIdentity,
        )>,
    ) -> Self {
        let mut prepared = Self::empty();
        match action {
            RendererOwnerAction::FileChooser(activation) => {
                crate::domains::input::InputPreparedOutputs::from_renderer_file_chooser_activation(
                    conn,
                    owner,
                    page_source,
                    activation,
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
                    conn,
                    owner,
                    page_source.map(|(page, _)| page),
                    dialog,
                )
                .append_to_javascript_dialog_output_sink(&mut prepared);
            }
            RendererOwnerAction::CloseAuxiliaryWindow(window) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_auxiliary_window_close(
                    page_source.map(|(page, _)| page),
                    window,
                )
                .append_to_auxiliary_window_close_output_sink(&mut prepared);
            }
            RendererOwnerAction::NavigateAuxiliaryWindow {
                window,
                url,
                kind,
                document_response,
            } => {
                crate::domains::page::PagePreparedOutputs::from_renderer_auxiliary_window_navigation(
                    page_source.map(|(page, _)| page), window, url, kind, document_response,
                )
                .append_to_auxiliary_window_navigation_output_sink(&mut prepared);
            }
            RendererOwnerAction::Popup(activation) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_popup_activation(
                    page_source.map(|(page, _)| page),
                    activation,
                )
                .append_to_popup_output_sink(&mut prepared);
            }
            RendererOwnerAction::ChildFrameTree {
                source_document,
                event,
            } => {
                crate::domains::page::PagePreparedOutputs::from_renderer_child_frame_tree_event(
                    conn,
                    owner,
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
                        owner,
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
                        owner,
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
                    owner,
                    source_document,
                    event,
                )
                .append_to_child_frame_output_sink(&mut prepared);
            }
            RendererOwnerAction::SessionHistoryUpdate {
                source_document,
                update,
            } => {
                crate::domains::page::PagePreparedOutputs::from_renderer_session_history_update(
                    page_source.map(|(page, _)| page),
                    source_document,
                    update,
                )
                .append_to_session_history_output_sink(&mut prepared);
            }
            RendererOwnerAction::SameDocumentNavigation(navigation) => {
                crate::domains::page::PagePreparedOutputs::from_renderer_same_document_navigation(
                    page_source.map(|(page, _)| page),
                    navigation,
                )
                .append_to_same_document_navigation_output_sink(&mut prepared);
            }
            RendererOwnerAction::TopLevelLocationNavigation(navigation) => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_top_level_location_navigation(
                        page_source.map(|(page, _)| page), navigation,
                    )
                    .append_to_top_level_location_navigation_output_sink(&mut prepared);
            }
            RendererOwnerAction::TopLevelHistoryTraversal(traversal) => {
                crate::domains::page::PagePreparedOutputs::
                    from_renderer_top_level_history_traversal(page_source.map(|(page, _)| page), owner, traversal)
                    .append_to_top_level_history_traversal_output_sink(&mut prepared);
            }
            RendererOwnerAction::SubresourceFetchPause {
                source_document,
                info,
                document_loader_id,
            } => {
                crate::domains::fetch::
                    subresource_fetch_pause_prepared_outputs_for_renderer_record(
                        conn,
                        owner,
                        source_document,
                        *info,
                        document_loader_id,
                        page_source,
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::SubresourceContinue {
                source_document,
                event,
            } => {
                crate::domains::network::NetworkPreparedOutputs::
                    from_renderer_subresource_continue(
                        conn,
                        owner,
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
                    detached_parser_script_fetch_pause_prepared_outputs_for_renderer_record(
                        conn,
                        owner,
                        source_document,
                        *info,
                        continuation,
                    )
                    .append_to_output_sink(&mut prepared);
            }
            RendererOwnerAction::SharedWorkerTargetLifecycle(event) => {
                if let Some((browser_context_id, _)) = conn.target_owner_identity_for_owner(owner) {
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
                if let Some((browser_context_id, _)) = conn.target_owner_identity_for_owner(owner) {
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
                        owner,
                        page_source,
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

    fn emit_root_network_idle_if_ready(
        &mut self,
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        command_context: &mut CommandDispatchContext,
    ) {
        if std::mem::take(&mut self.emit_root_network_idle_after_projection) {
            conn.emit_root_network_idle_for_owner(owner, command_context.protocol_events_mut());
        }
    }

    fn push_output_slot(&mut self, slot: ProtocolOutputSlot) {
        if !self.ordered_slots.contains(&slot) {
            self.ordered_slots.push(slot);
        }
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
