use super::workflow::{PendingProtocolWorkflow, SharedProtocolWorkflowResult};
use crate::conn::{
    CdpConnection, CdpSchedulerEvent, CdpTurnOutcome, PreparedCommandWorkflow, ProtocolStateHandle,
    ProtocolWorkflowDriver,
};
use crate::domains::{
    activity::{
        AdmittedRendererOutputPublication, RendererOutputIngressAdmission, RendererOutputProjection,
    },
    command_output::CommandOutputPlan,
};
use moli_core::{RendererOutputCursor, RendererOutputTransportMessage};

pub(super) type SharedRendererProjectionResult = SharedProtocolWorkflowResult;

#[derive(Clone)]
pub(super) struct PendingRendererOutputProjection {
    workflow: PendingProtocolWorkflow,
    cursor: RendererOutputCursor,
}

impl PendingRendererOutputProjection {
    fn prepare(
        conn: &mut CdpConnection,
        admitted: AdmittedRendererOutputPublication,
        access: ProtocolStateHandle,
    ) -> Self {
        let (publication, owner) = admitted.into_parts();
        let cursor = publication.cursor();
        let target_id = match &owner {
            crate::domains::activity::RendererPublicationOwner::PageTarget {
                target_id, ..
            } => target_id.clone(),
            crate::domains::activity::RendererPublicationOwner::BrowserContext { .. } => None,
        };
        let projection = RendererOutputProjection::new(conn, publication, owner);
        let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
            let mut driver = ProtocolWorkflowDriver::new(access);
            driver.replace_renderer_output_source(Some(cursor));
            if let Err(error) = projection.run(&mut driver, &mut context).await {
                tracing::warn!(error = ?error, ?cursor, "renderer output projection completed with cleanup failure");
            }
            context.complete(CommandOutputPlan::default())
        });
        Self {
            workflow: PendingProtocolWorkflow::prepare(workflow, target_id)
                .with_renderer_source(Some(cursor)),
            cursor,
        }
    }

    pub(super) fn cursor(&self) -> RendererOutputCursor {
        self.cursor
    }

    pub(super) async fn wait(&self) -> SharedRendererProjectionResult {
        self.workflow.wait().await
    }

    pub(super) fn finish(
        self,
        conn: &mut CdpConnection,
        result: SharedRendererProjectionResult,
    ) -> CdpTurnOutcome {
        if result.borrow().is_none() {
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                conn.take_scheduler_events(),
            );
        }
        let outcome = self.workflow.finish(conn, result);
        let (events, after_renderer, boundary, after_response, mut scheduler) =
            outcome.into_command_turn_parts();
        assert!(
            boundary.is_none(),
            "nested output is handed to the original renderer fence before cursor release"
        );
        let ready = conn.complete_renderer_output_projection(self.cursor);
        conn.publish_admitted_renderer_projections(ready);
        scheduler.extend(conn.take_scheduler_events());
        let mut events = events;
        events.extend(after_renderer);
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            events,
            after_response,
            scheduler,
        )
    }
}

impl CdpConnection {
    /// All concrete renderer publications leave the actor before Native waits.
    /// Stream controls and leases remain finite owner operations.
    pub fn admit_renderer_output(
        &mut self,
        message: RendererOutputTransportMessage,
    ) -> CdpTurnOutcome {
        let publication = match message {
            RendererOutputTransportMessage::Publication(publication) => Some(publication),
            RendererOutputTransportMessage::StreamControl(control) => {
                self.apply_renderer_output_stream_control(control);
                None
            }
            RendererOutputTransportMessage::PageReservationReleased {
                owner_local_host_id,
                page_id,
            } => {
                self.release_renderer_page_output_owner_reservation(owner_local_host_id, page_id);
                None
            }
            RendererOutputTransportMessage::CursorLeaseDeclared { cursor, lease_id } => {
                self.declare_renderer_output_cursor_lease(cursor, lease_id);
                None
            }
            RendererOutputTransportMessage::CursorLeaseReleased { stream, lease_id } => {
                self.release_renderer_output_cursor_lease(stream, lease_id);
                None
            }
        };
        let Some(publication) = publication else {
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                self.take_scheduler_events(),
            );
        };
        let ready = self.admit_renderer_output_publication(publication);
        self.publish_admitted_renderer_projections(ready);
        CdpTurnOutcome::new_with_protocol_events(Vec::new(), self.take_scheduler_events())
    }

    fn publish_admitted_renderer_projections(&mut self, ready: RendererOutputIngressAdmission) {
        if let RendererOutputIngressAdmission::Ready(ready) = ready {
            let access = self.protocol_state_access();
            for admitted in ready {
                let pending = super::PendingProtocolSchedulerWork::for_renderer_output_projection(
                    PendingRendererOutputProjection::prepare(self, admitted, access.clone()),
                    access.clone(),
                );
                self.scheduler_state.push_scheduler_event(
                    CdpSchedulerEvent::RendererOutputProjectionPrepared { pending },
                );
            }
        }
    }
}
