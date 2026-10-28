use std::{cell::RefCell, rc::Rc};

use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use crate::conn::{
    CdpConnection, CdpTurnOutcome, CommandDispatchContext, CompletedCommandWorkflow,
    PreparedCommandWorkflow,
};

pub(super) type SharedProtocolWorkflowResult = Rc<RefCell<Option<CompletedCommandWorkflow>>>;

/// One supervised owner workflow, including its move-only output context.
/// Cancelling a waiter or losing a ready receipt rejoins this same future.
#[derive(Clone)]
pub(super) struct PendingProtocolWorkflow {
    pending: Shared<LocalBoxFuture<'static, SharedProtocolWorkflowResult>>,
    context: Rc<RefCell<CommandDispatchContext>>,
    target_id: Option<String>,
    source_cursor: Option<moli_core::RendererOutputCursor>,
    load_observation: Option<crate::conn::DeferredMainDocumentLoadObservationId>,
}

impl PendingProtocolWorkflow {
    pub(super) fn prepare(workflow: PreparedCommandWorkflow, target_id: Option<String>) -> Self {
        let mut context = CommandDispatchContext::default();
        let pending = workflow.start(&mut context, None, None);
        Self {
            pending: async move { Rc::new(RefCell::new(Some(pending.wait().await))) }
                .boxed_local()
                .shared(),
            context: Rc::new(RefCell::new(context)),
            target_id,
            source_cursor: None,
            load_observation: None,
        }
    }

    pub(super) fn with_renderer_source(
        mut self,
        cursor: Option<moli_core::RendererOutputCursor>,
    ) -> Self {
        self.source_cursor = cursor;
        self
    }

    pub(super) fn renderer_source(&self) -> Option<moli_core::RendererOutputCursor> {
        self.source_cursor
    }

    pub(super) fn with_load_observation(
        mut self,
        observation: crate::conn::DeferredMainDocumentLoadObservationId,
    ) -> Self {
        self.load_observation = Some(observation);
        self
    }

    pub(super) fn load_observation(
        &self,
    ) -> Option<crate::conn::DeferredMainDocumentLoadObservationId> {
        self.load_observation
    }

    pub(super) async fn wait(&self) -> SharedProtocolWorkflowResult {
        self.pending.clone().await
    }

    pub(super) fn finish(
        self,
        conn: &mut CdpConnection,
        result: SharedProtocolWorkflowResult,
    ) -> CdpTurnOutcome {
        let Some(completed) = result.borrow_mut().take() else {
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                conn.take_scheduler_events(),
            );
        };
        let scheduler_start = conn.scheduler_state.scheduler_event_count();
        let mut context = std::mem::take(&mut *self.context.borrow_mut());
        let mut plan = completed.finish(conn, &mut context);
        if let Some(Err(error)) = plan.take_command_status() {
            tracing::warn!(?error, "renderer protocol workflow failed");
        }
        super::super::dispatch::append_command_output_plan(
            &mut Vec::new(),
            &mut context,
            plan,
            None,
            None,
        );
        let events = conn.finish_protocol_output_projection(context, self.target_id);
        if let Some(source) = self.source_cursor {
            conn.scheduler_state
                .bind_renderer_output_source_since(scheduler_start, source);
        }
        CdpTurnOutcome::new_with_protocol_events(events, conn.take_scheduler_events())
    }
}

impl CdpConnection {
    pub(crate) fn finish_protocol_output_projection(
        &mut self,
        mut context: CommandDispatchContext,
        target_id: Option<String>,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        let (mut events, boundary, mut after) = context.take_renderer_fenced_protocol_events();
        after.extend(context.take_post_response_events());
        let predecessor = context.take_renderer_output_predecessor();
        if let Some(boundary) = boundary {
            if let Some(predecessor) = predecessor.as_ref()
                && !events.is_empty()
            {
                self.publish_renderer_fenced_protocol_output(
                    std::mem::take(&mut events),
                    predecessor.clone(),
                    target_id.clone(),
                );
            }
            // The insertion also releases independently transported Native
            // observations. Its receipt must survive an empty wire suffix.
            let mut predecessors = Vec::new();
            predecessors.extend(predecessor);
            predecessors.push(boundary);
            self.publish_renderer_fenced_protocol_output_after(after, predecessors, target_id);
        } else {
            events.extend(after);
            if let Some(predecessor) = predecessor
                && !events.is_empty()
            {
                self.publish_renderer_fenced_protocol_output(
                    std::mem::take(&mut events),
                    predecessor,
                    target_id,
                );
            }
        }
        self.finish_runtime_output_projection_permits(
            context.take_runtime_output_projection_permits(),
        );
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BackgroundProtocolEvent, CdpSchedulerEvent};
    use crate::domains::{activity::RendererPublicationOwner, command_output::CommandOutputPlan};
    use moli_core::{
        RendererOutputCursor, RendererOutputFence, RendererOutputPublication,
        RendererOutputStreamIdentity,
    };
    use serde_json::json;

    #[tokio::test]
    async fn after_boundary_output_keeps_both_independent_native_fences() {
        for emit_events in [false, true] {
            for first in [0, 1] {
                let mut conn = CdpConnection::new();
                let cursors = [11, 12].map(|instance| {
                    RendererOutputCursor::new_for_test(
                        RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(instance),
                        1,
                    )
                });
                for cursor in cursors {
                    conn.scheduler_state.renderer_output_ingress.open(
                        cursor.stream(),
                        Some(RendererPublicationOwner::BrowserContext {
                            browser_context_id: "BID-fences".into(),
                        }),
                    );
                    assert!(matches!(
                        conn.admit_renderer_output_publication(
                            RendererOutputPublication::new_for_test(
                                cursor,
                                vec![moli_core::RendererOutputRecord::new_for_test(
                                moli_core::RendererOutputItem::Observation(
                                    moli_core::RendererProtocolObservation::RuntimeLifecycleError {
                                        text: "fence fixture".into(),
                                        execution_context_id: None,
                                    },
                                )
                            ),]
                            )
                        ),
                        crate::domains::activity::RendererOutputIngressAdmission::Ready(_)
                    ));
                }
                let workflow = PreparedCommandWorkflow::new(move |context| async move {
                    let mut plan = CommandOutputPlan::default();
                    plan.set_renderer_output_predecessor(RendererOutputFence::new_for_test(
                        cursors[0],
                    ));
                    if emit_events {
                        plan.push_background_event(BackgroundProtocolEvent::immediate(
                            json!({"method": "Moli.beforeBoundary"}),
                        ));
                    }
                    plan.insert_renderer_output_boundary(RendererOutputFence::new_for_test(
                        cursors[1],
                    ));
                    if emit_events {
                        plan.push_background_event(BackgroundProtocolEvent::immediate(
                            json!({"method": "Moli.afterBoundary"}),
                        ));
                    }
                    context.complete(plan)
                });
                let pending = PendingProtocolWorkflow::prepare(workflow, None);
                let completed = pending.wait().await;
                let (output, events) = pending
                    .finish(&mut conn, completed)
                    .into_protocol_event_parts();
                assert!(output.is_empty());
                let work = events
                    .into_iter()
                    .map(|event| match event {
                        CdpSchedulerEvent::ProtocolWorkPublished { work } => work,
                        _ => panic!("only the original fenced output was published"),
                    })
                    .collect::<Vec<_>>();
                assert_eq!(work.len(), if emit_events { 2 } else { 1 });
                assert!(
                    work.iter()
                        .all(|work| !work.renderer_output_predecessor_is_projected(&conn))
                );
                conn.complete_renderer_output_projection(cursors[first]);
                if emit_events {
                    assert_eq!(
                        work[0].renderer_output_predecessor_is_projected(&conn),
                        first == 0
                    );
                }
                assert!(
                    !work
                        .last()
                        .unwrap()
                        .renderer_output_predecessor_is_projected(&conn),
                    "one independent fence cannot release the after-boundary segment"
                );
                conn.complete_renderer_output_projection(cursors[1 - first]);
                assert!(
                    work.iter()
                        .all(|work| work.renderer_output_predecessor_is_projected(&conn))
                );
            }
        }
    }
}
