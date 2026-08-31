use super::{CdpConnection, CdpTurnOutcome};
use crate::domains::activity::{
    ReadyProtocolSchedulerWork, RuntimeCommandOutputBarrierCompletion,
    RuntimeCommandOutputBarrierPermit, RuntimeCommandOutputBarriers,
};
use moli_core::RendererOutputTransportMessage;

impl CdpConnection {
    pub(crate) fn apply_renderer_output_stream_control(
        &mut self,
        control: moli_core::RendererOutputStreamControl,
    ) {
        match control {
            moli_core::RendererOutputStreamControl::Opened { stream } => {
                let residence = stream.residence();
                let owners = crate::domains::activity::renderer_publication_owners(self, residence);
                let owner = match owners.as_slice() {
                    [] => None,
                    [owner] => Some(owner.clone()),
                    _ => {
                        panic!(
                            "renderer output residence {residence:?} must resolve to at most one \
                             protocol owner when its stream opens, got {}",
                            owners.len()
                        )
                    }
                };
                self.scheduler_state
                    .renderer_output_ingress
                    .open(stream, owner);
            }
            moli_core::RendererOutputStreamControl::Closed {
                stream,
                last_published_sequence,
                ..
            } => {
                self.scheduler_state
                    .renderer_output_ingress
                    .close(stream, last_published_sequence);
                if let Some(renderer_page) =
                    super::RendererPageResidenceIdentity::from_residence(stream.residence())
                {
                    self.finish_renderer_page_output_retirement(renderer_page);
                }
            }
        }
    }

    fn finish_renderer_page_output_retirement(
        &mut self,
        renderer_page: super::RendererPageResidenceIdentity,
    ) {
        for browser_context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            for target in browser_context.page_targets.iter_mut() {
                target
                    .runtime_slot
                    .finish_renderer_page_output_retirement(renderer_page);
            }
        }
    }

    pub(crate) fn bind_renderer_page_output_owner(
        &mut self,
        renderer_page: super::RendererPageResidenceIdentity,
        page_owner: super::TargetPageResidenceIdentity,
    ) {
        let owner = crate::domains::activity::RendererPublicationOwner::PageTarget {
            browser_context_id: page_owner.browser_context_id().to_owned(),
            target_id: page_owner.target_id().map(str::to_owned),
            renderer_page,
            page_owner,
        };
        self.scheduler_state.renderer_output_ingress.bind_owner(
            moli_core::RendererOutputResidenceIdentity::Page {
                owner_local_host_id: renderer_page.owner_local_host_id(),
                page_id: renderer_page.page_id(),
            },
            owner,
        );
    }

    pub(crate) fn release_renderer_page_output_owner_reservation(
        &mut self,
        owner_local_host_id: moli_core::RendererOwnerLocalHostId,
        page_id: moli_core::PageId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .release_page_owner_reservation(owner_local_host_id, page_id);
    }

    pub(crate) fn declare_renderer_output_cursor_lease(
        &mut self,
        cursor: moli_core::RendererOutputCursor,
        lease_id: moli_core::RendererOutputFenceLeaseId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .declare_cursor_lease(cursor, lease_id);
    }

    pub(crate) fn release_renderer_output_cursor_lease(
        &mut self,
        stream: moli_core::RendererOutputStreamIdentity,
        lease_id: moli_core::RendererOutputFenceLeaseId,
    ) {
        self.scheduler_state
            .renderer_output_ingress
            .release_cursor_lease(stream, lease_id);
    }

    pub(crate) fn admit_renderer_output_publication(
        &mut self,
        publication: moli_core::RendererOutputPublication,
    ) -> crate::domains::activity::RendererOutputIngressAdmission {
        self.scheduler_state
            .renderer_output_ingress
            .admit(publication)
    }

    pub(crate) fn complete_renderer_output_projection(
        &mut self,
        cursor: moli_core::RendererOutputCursor,
    ) -> crate::domains::activity::RendererOutputIngressAdmission {
        let ready = self
            .scheduler_state
            .renderer_output_ingress
            .complete_projection(cursor);
        if ready.is_empty() {
            crate::domains::activity::RendererOutputIngressAdmission::Buffered
        } else {
            crate::domains::activity::RendererOutputIngressAdmission::Ready(ready)
        }
    }

    pub fn renderer_output_cursor_is_projected(
        &self,
        cursor: moli_core::RendererOutputCursor,
    ) -> bool {
        self.scheduler_state
            .renderer_output_ingress
            .is_projection_complete(cursor)
    }

    /// Materializes one already-frozen protocol output work item.
    ///
    /// This path never scans a source for payload or reconstructs a route from
    /// current target state. The work owns both. Completion performs only the
    /// exact attachment authorization required to prevent a detached session
    /// or replacement Page from receiving historical output.
    pub async fn complete_ready_protocol_scheduler_work_turn(
        &mut self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        match work.into_ready() {
            ReadyProtocolSchedulerWork::ProtocolObservation(work) => {
                CdpTurnOutcome::new_with_protocol_events(
                    work.into_background_events(self),
                    Vec::new(),
                )
            }
            ReadyProtocolSchedulerWork::MainDocumentLoadOwnerAction(completion) => {
                self.complete_deferred_main_document_load_completion_for_scheduler(
                    super::CompletedDeferredMainDocumentLoadCompletion::new(*completion),
                )
                .await
            }
            ReadyProtocolSchedulerWork::BidiChannelOwnerAction(action) => {
                let mut protocol_events = Vec::new();
                self.complete_bidi_channel_owner_action_with_background_events_async(
                    action,
                    &mut protocol_events,
                )
                .await;
                CdpTurnOutcome::new_with_protocol_events(
                    protocol_events,
                    self.take_scheduler_events(),
                )
            }
            ReadyProtocolSchedulerWork::TopLevelLocationNavigationOwnerAction(action) => {
                let (owner_scope, page_owner, navigation) = action.into_parts();
                let session_id = owner_scope.session_id().map(str::to_owned);
                let mut route_scope = owner_scope.enter(self);
                let mut protocol_events = Vec::new();
                crate::domains::page::navigate_page_owned_top_level_location_background_events_async(
                    route_scope.conn_mut(),
                    &mut protocol_events,
                    session_id.as_deref(),
                    &page_owner,
                    navigation,
                )
                .await;
                CdpTurnOutcome::new_with_protocol_events(
                    protocol_events,
                    route_scope.conn_mut().take_scheduler_events(),
                )
            }
            ReadyProtocolSchedulerWork::PopupTargetNavigationOwnerAction(action) => {
                crate::domains::target::complete_popup_target_navigation_owner_action_async(
                    self, action,
                )
                .await
            }
            ReadyProtocolSchedulerWork::PopupTargetActivationAction(action) => {
                crate::domains::target::complete_popup_target_activation_action_async(self, action)
                    .await
            }
            ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action) => {
                crate::domains::page::complete_page_target_termination_owner_action_async(
                    self, action,
                )
                .await
            }
        }
    }

    /// Ingests one concrete renderer publication and returns the protocol
    /// output/scheduler work produced from its exact owner route.
    ///
    /// The publication is consumed once. This facade does not execute Page
    /// work or rescan a source after ingress.
    pub async fn ingest_renderer_output_turn_async(
        &mut self,
        publication: RendererOutputTransportMessage,
        barriers: &mut RuntimeCommandOutputBarriers,
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        let protocol_events = crate::domains::activity::ingest_renderer_output_transport_async(
            self,
            publication,
            barriers,
            &mut command_context,
        )
        .await;
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            protocol_events,
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub async fn project_protocol_local_command_outputs_turn_async(
        &mut self,
        session_id: Option<&str>,
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            session_id,
            &mut command_context,
        )
        .await;
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            command_context.take_protocol_events(),
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub async fn release_runtime_command_output_barrier_turn_async(
        &mut self,
        barriers: &mut RuntimeCommandOutputBarriers,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        let mut command_context = super::CommandDispatchContext::default();
        let terminal = barriers.release(self, permit, &mut command_context).await;
        RuntimeCommandOutputBarrierCompletion::new(
            terminal,
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                command_context.take_protocol_events(),
                command_context.take_post_response_events(),
                self.take_scheduler_events(),
            ),
        )
    }

    pub async fn cancel_runtime_command_output_barrier_turn_async(
        &mut self,
        barriers: &mut RuntimeCommandOutputBarriers,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        let mut command_context = super::CommandDispatchContext::default();
        let terminal = barriers.cancel(self, permit, &mut command_context).await;
        RuntimeCommandOutputBarrierCompletion::new(
            terminal,
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                command_context.take_protocol_events(),
                command_context.take_post_response_events(),
                self.take_scheduler_events(),
            ),
        )
    }
}
