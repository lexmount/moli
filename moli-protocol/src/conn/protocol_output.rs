use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::{CdpConnection, CdpTurnOutcome};
use crate::domains::activity::{
    ReadyProtocolSchedulerWork, RuntimeCommandOutputBarrierCompletion,
    RuntimeCommandOutputBarrierPermit, RuntimeCommandOutputBarriers,
};
use moli_core::RendererOutputTransportMessage;

mod navigation;
pub use navigation::BackgroundNavigationProtocolReady;
pub(crate) use navigation::PendingBackgroundNavigationProtocol;
mod renderer;
mod task;
mod workflow;
pub use task::{CompletedProtocolSchedulerWork, PendingProtocolSchedulerWork};

impl CdpConnection {
    /// Barrier admission and renderer-call registration belong to this same
    /// connection. The adapter owns only the move-only response permit.
    pub fn admit_runtime_command_output_barrier(
        &mut self,
        command_id: u64,
        session_id: Option<&str>,
    ) -> Option<RuntimeCommandOutputBarrierPermit> {
        self.with_runtime_command_output_barriers(|conn, barriers| {
            barriers.admit(conn, command_id, session_id)
        })
    }

    /// Returns only owned values; no connection or barrier borrow can escape
    /// this finite operation. Restore the original registry while unwinding.
    pub(crate) fn with_runtime_command_output_barriers<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(
            &'state mut Self,
            &'state mut RuntimeCommandOutputBarriers,
        ) -> T,
    ) -> T {
        let mut barriers =
            std::mem::take(&mut self.scheduler_state.runtime_command_output_barriers);
        let result = catch_unwind(AssertUnwindSafe(|| operation(self, &mut barriers)));
        self.scheduler_state.runtime_command_output_barriers = barriers;
        result.unwrap_or_else(|payload| resume_unwind(payload))
    }

    pub(crate) fn publish_renderer_fenced_protocol_output(
        &mut self,
        events: Vec<super::BackgroundProtocolEvent>,
        predecessor: moli_core::RendererOutputFence,
        target_id: Option<String>,
    ) {
        let sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::renderer_fenced_output(
            sequence,
            events,
            vec![predecessor],
            target_id,
        );
        self.scheduler_state
            .push_scheduler_event(super::CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(crate) fn publish_renderer_fenced_protocol_output_after(
        &mut self,
        events: Vec<super::BackgroundProtocolEvent>,
        predecessors: Vec<moli_core::RendererOutputFence>,
        target_id: Option<String>,
    ) {
        let sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::renderer_fenced_output(
            sequence,
            events,
            predecessors,
            target_id,
        );
        self.scheduler_state
            .push_scheduler_event(super::CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

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
                let retired = self
                    .scheduler_state
                    .renderer_output_ingress
                    .close(stream, last_published_sequence);
                if retired
                    && let Some(renderer_page) =
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
        let (ready, retired) = self
            .scheduler_state
            .renderer_output_ingress
            .complete_projection(cursor);
        if retired
            && let Some(renderer_page) =
                super::RendererPageResidenceIdentity::from_residence(cursor.stream().residence())
        {
            self.finish_renderer_page_output_retirement(renderer_page);
        }
        if ready.is_empty() {
            crate::domains::activity::RendererOutputIngressAdmission::Buffered
        } else {
            crate::domains::activity::RendererOutputIngressAdmission::Ready(ready)
        }
    }

    pub fn renderer_output_cursor_is_outstanding(
        &self,
        cursor: moli_core::RendererOutputCursor,
    ) -> bool {
        self.scheduler_state
            .renderer_output_ingress
            .is_projection_outstanding(cursor)
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
    pub fn try_complete_protocol_scheduler_work_turn(
        &mut self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> Result<CdpTurnOutcome, Box<crate::domains::activity::ProtocolSchedulerWork>> {
        if work.kind() != crate::domains::activity::ProtocolSchedulerWorkKind::ProtocolObservation {
            return Err(Box::new(work));
        }
        let events = match work.into_ready() {
            ReadyProtocolSchedulerWork::RendererFencedOutput(output) => {
                assert!(
                    output
                        .predecessors
                        .iter()
                        .all(|fence| self.renderer_output_cursor_is_projected(fence.cursor())),
                    "materialized protocol output cannot cross its concrete renderer predecessor"
                );
                output.events
            }
            ReadyProtocolSchedulerWork::ProtocolObservation(work) => {
                work.into_background_events(self)
            }
            _ => unreachable!("only finite protocol observations were admitted"),
        };
        Ok(CdpTurnOutcome::new_with_protocol_events(
            events,
            self.take_scheduler_events(),
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn complete_ready_protocol_scheduler_work_turn(
        &mut self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        match work.into_ready() {
            ReadyProtocolSchedulerWork::NavigationProtocolCompletion(_) => {
                unreachable!("navigation protocol waits retain the existing owner-work supervisor")
            }
            ReadyProtocolSchedulerWork::RendererFencedOutput(output) => {
                assert!(
                    output
                        .predecessors
                        .iter()
                        .all(|fence| self.renderer_output_cursor_is_projected(fence.cursor())),
                    "materialized protocol output cannot cross its concrete renderer predecessor"
                );
                CdpTurnOutcome::new_with_protocol_events(
                    output.events,
                    self.take_scheduler_events(),
                )
            }

            ReadyProtocolSchedulerWork::ProtocolObservation(work) => {
                CdpTurnOutcome::new_with_protocol_events(
                    work.into_background_events(self),
                    Vec::new(),
                )
            }
            ReadyProtocolSchedulerWork::MainDocumentLoadOwnerAction(completion) => {
                self.complete_deferred_main_document_load_completion_for_scheduler(
                    super::CompletedDeferredMainDocumentLoadCompletion::new(
                        completion.try_complete().unwrap_or_else(|_| {
                            panic!("inline load completion requires a terminal lifecycle observer")
                        }),
                    ),
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
            ReadyProtocolSchedulerWork::TopLevelNavigationOwnerAction(action) => {
                let (owner_scope, page_owner, navigation) = action.into_parts();
                let mut protocol_events = Vec::new();
                if self.target_page_residence_identity_is_current(&page_owner) {
                    match navigation {
                        super::TopLevelNavigationRequest::Location(navigation) => {
                            crate::domains::page::navigate_page_owned_top_level_location_background_events_async(
                    self,
                    &mut protocol_events,
                    &owner_scope,
                    &page_owner,
                    navigation,
                )
                .await;
                        }
                        super::TopLevelNavigationRequest::History(traversal) => {
                            crate::domains::page::traverse_session_owner_history_from_renderer_background_events_async(
                                self, &mut protocol_events, &owner_scope, traversal.delta,
                            ).await;
                        }
                    }
                }
                CdpTurnOutcome::new_with_protocol_events(
                    protocol_events,
                    self.take_scheduler_events(),
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
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        let protocol_events = crate::domains::activity::ingest_renderer_output_transport_async(
            self,
            publication,
            &mut command_context,
        )
        .await;
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            protocol_events,
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub fn project_protocol_local_command_outputs_turn(
        &mut self,
        session_id: Option<&str>,
    ) -> CdpTurnOutcome {
        let mut command_context = super::CommandDispatchContext::default();
        let owner = super::CommandOwnerScope::capture(self, session_id);
        crate::domains::activity::project_protocol_local_command_outputs(
            self,
            &owner,
            &mut command_context,
        );
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            command_context.take_protocol_events(),
            command_context.take_post_response_events(),
            self.take_scheduler_events(),
        )
    }

    pub(crate) fn finish_runtime_output_projection_permits(
        &mut self,
        permits: Vec<crate::domains::activity::RuntimeCommandOutputProjectionPermit>,
    ) {
        let ready = self.with_runtime_command_output_barriers(move |_, barriers| {
            permits
                .into_iter()
                .flat_map(|permit| barriers.complete_output_projection(permit))
                .collect::<Vec<_>>()
        });
        if !ready.is_empty() {
            let access = self
                .protocol_state_access()
                .expect("owned output handoff retains its protocol owner");
            self.schedule_runtime_output_projections(ready, access);
        }
    }

    pub(crate) fn schedule_runtime_output_projections(
        &mut self,
        ready: Vec<crate::domains::activity::ReadyRuntimeCommandOutputProjection>,
        access: super::ProtocolStateAccess,
    ) {
        for projection in ready {
            let target_id = projection.target_id(self);
            let pending = PendingProtocolSchedulerWork::for_output_projection(
                projection,
                access.clone(),
                target_id,
            );
            self.scheduler_state.push_scheduler_event(
                super::CdpSchedulerEvent::RendererOutputProjectionPrepared { pending },
            );
        }
    }

    pub async fn release_runtime_command_output_barrier_turn_async(
        &mut self,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        self.complete_runtime_command_output_barrier_turn(permit, false)
            .await
    }

    pub async fn cancel_runtime_command_output_barrier_turn_async(
        &mut self,
        permit: RuntimeCommandOutputBarrierPermit,
    ) -> RuntimeCommandOutputBarrierCompletion {
        self.complete_runtime_command_output_barrier_turn(permit, true)
            .await
    }

    async fn complete_runtime_command_output_barrier_turn(
        &mut self,
        permit: RuntimeCommandOutputBarrierPermit,
        canceled: bool,
    ) -> RuntimeCommandOutputBarrierCompletion {
        let (terminal, ready) = self.with_runtime_command_output_barriers(move |conn, barriers| {
            barriers.prepare_completion(conn, permit, canceled)
        });
        let mut command_context = super::CommandDispatchContext::default();
        if !ready.is_empty() {
            if let Some(access) = self.protocol_state_access() {
                self.schedule_runtime_output_projections(ready, access);
            } else {
                let mut local = super::CommandDispatchContext::default();
                let (mut context, _scope) =
                    super::CommandWorkflowContext::take_from(&mut local, None, None);
                let result =
                    crate::domains::activity::ReadyRuntimeCommandOutputProjection::run_ready(
                        ready,
                        &mut super::ProtocolWorkflowDriver::inline(self),
                        &mut context,
                    )
                    .await;
                context.finish_projection_inline(self, &mut command_context);
                result.expect("inline barrier completion retains its protocol owner");
            }
        }
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
