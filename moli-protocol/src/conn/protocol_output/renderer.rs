use std::{cell::RefCell, rc::Rc};

use anyhow::Result;

use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};
use moli_core::{
    RendererOutputCursor, RendererOutputResidenceIdentity, RendererOutputTransportMessage,
};

use crate::conn::{
    CdpConnection, CdpSchedulerEvent, CdpTurnOutcome, CommandDispatchContext,
    CompletedCommandWorkflow, PreparedCommandWorkflow, ProtocolStateAccess, ProtocolWorkflowDriver,
};
use crate::domains::{
    activity::{
        AdmittedRendererOutputPublication, RendererOutputIngressAdmission, WorkerRendererProjection,
    },
    command_output::CommandOutputPlan,
};

pub(super) type SharedRendererProjectionResult = Rc<RefCell<Option<CompletedCommandWorkflow>>>;

#[derive(Clone)]
pub(super) struct PendingWorkerRendererProjection {
    pending: Shared<LocalBoxFuture<'static, SharedRendererProjectionResult>>,
    context: Rc<RefCell<CommandDispatchContext>>,
    cursor: RendererOutputCursor,
}

impl PendingWorkerRendererProjection {
    fn prepare(admitted: AdmittedRendererOutputPublication, access: ProtocolStateAccess) -> Self {
        let (publication, owner) = admitted.into_parts();
        let cursor = publication.cursor();
        let projection = WorkerRendererProjection::new(publication, owner);
        let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
            if let Err(error) = projection
                .run(&mut ProtocolWorkflowDriver::owned(access), &mut context)
                .await
            {
                tracing::warn!(%error, ?cursor, "Worker output projection completed with cleanup failure");
            }
            context.complete(CommandOutputPlan::default())
        });
        let mut context = CommandDispatchContext::default();
        let pending = workflow.start(&mut context, None, None);
        Self {
            pending: async move { Rc::new(RefCell::new(Some(pending.wait().await))) }
                .boxed_local()
                .shared(),
            context: Rc::new(RefCell::new(context)),
            cursor,
        }
    }

    pub(super) async fn wait(&self) -> SharedRendererProjectionResult {
        self.pending.clone().await
    }

    pub(super) fn finish(
        self,
        conn: &mut CdpConnection,
        result: SharedRendererProjectionResult,
    ) -> CdpTurnOutcome {
        let Some(completed) = result.borrow_mut().take() else {
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                conn.take_scheduler_events(),
            );
        };
        let mut context = std::mem::take(&mut *self.context.borrow_mut());
        let plan = completed.finish(conn, &mut context);
        super::super::dispatch::append_command_output_plan(
            &mut Vec::new(),
            &mut context,
            plan,
            None,
            None,
        );
        let (mut events, boundary, after) = context.take_renderer_fenced_protocol_events();
        assert!(
            boundary.is_none(),
            "Worker projection has no nested Page output boundary"
        );
        assert!(
            context.take_renderer_output_predecessor().is_none(),
            "Worker projection has no Page commit predecessor"
        );
        events.extend(after);
        // Output is materialized before releasing the same-stream cursor.
        // A successor can be polled only after this owner turn is published.
        let ready = conn.complete_renderer_output_projection(self.cursor);
        conn.publish_admitted_worker_renderer_projections(ready);
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            events,
            context.take_post_response_events(),
            conn.take_scheduler_events(),
        )
    }
}

impl CdpConnection {
    /// Finite production ingress for the two independent Worker stream kinds.
    /// Page publications keep their existing driver during the remaining
    /// Page/Fetch migration.
    pub fn try_start_owned_worker_renderer_output(
        &mut self,
        message: RendererOutputTransportMessage,
    ) -> Result<CdpTurnOutcome, Box<RendererOutputTransportMessage>> {
        if !matches!(
            &message,
            RendererOutputTransportMessage::Publication(publication)
                if matches!(publication.cursor().stream().residence(),
                    RendererOutputResidenceIdentity::SharedWorker { .. }
                    | RendererOutputResidenceIdentity::ServiceWorker { .. })
        ) || self.protocol_state_access().is_none()
        {
            return Err(Box::new(message));
        }
        let RendererOutputTransportMessage::Publication(publication) = message else {
            unreachable!("the checked transport carries a Worker publication");
        };
        let ready = self.admit_renderer_output_publication(publication);
        self.publish_admitted_worker_renderer_projections(ready);
        Ok(CdpTurnOutcome::new_with_protocol_events(
            Vec::new(),
            self.take_scheduler_events(),
        ))
    }

    fn publish_admitted_worker_renderer_projections(
        &mut self,
        ready: RendererOutputIngressAdmission,
    ) {
        if let RendererOutputIngressAdmission::Ready(ready) = ready {
            let access = self
                .protocol_state_access()
                .expect("owned Worker ingress retains its state owner");
            for admitted in ready {
                let pending = super::PendingProtocolSchedulerWork::for_worker_renderer_projection(
                    PendingWorkerRendererProjection::prepare(admitted, access.clone()),
                    access.clone(),
                );
                self.scheduler_state.push_scheduler_event(
                    CdpSchedulerEvent::RendererOutputProjectionPrepared { pending },
                );
            }
        }
    }
}
