use anyhow::Result;

use super::*;
use anyhow::Context as _;

pub(crate) struct PreparedRendererNavigationFinish {
    events: Vec<BackgroundProtocolEvent>,
    replays: Option<(RendererAgentAttachmentId, Vec<SessionRendererCallReplay>)>,
}

impl PreparedRendererNavigationFinish {
    pub(crate) async fn wait_using(
        self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> Result<Vec<BackgroundProtocolEvent>> {
        let mut events = self.events;
        if let Some((attachment, replays)) = self.replays {
            for replay in replays {
                let pending = driver.with_state(move |state| {
                    state.start_navigation_renderer_replay(replay, attachment)
                })?;
                let completed = pending.wait().await;
                events.extend(
                    driver.with_state(move |state| {
                        state.finish_navigation_renderer_replay(completed)
                    })?,
                );
            }
        }
        Ok(events)
    }
}

/// One already registered renderer call and its original response lease.
/// Native admission is synchronous; waiting owns no protocol state.
pub(crate) struct PendingRendererCallReplay {
    context: RendererCallReplayContext,
    pending: Result<RendererCallReplayOperation>,
}

pub(crate) struct CompletedRendererCallReplay {
    context: RendererCallReplayContext,
    completed: Result<Option<Box<CompletedRuntimeProtocolMessageDispatch>>>,
}

struct RendererCallReplayContext {
    frontend_session_id: Option<String>,
    correlation: RendererCommandCorrelation,
    delivery: RendererInspectorResponseDelivery,
    response_sender: RendererRuntimeInspectorResponseSender,
    attachment: RendererAgentAttachmentId,
}

enum RendererCallReplayOperation {
    Inspector(Box<PendingRuntimeProtocolMessageDispatch>),
    Io {
        pending: moli_core::page::PendingDevToolsIoCommandDispatch,
        operation: &'static str,
    },
}

impl PendingRendererCallReplay {
    pub(crate) async fn wait(self) -> CompletedRendererCallReplay {
        let completed = match self.pending {
            Ok(RendererCallReplayOperation::Inspector(pending)) => pending
                .wait()
                .await
                .map(|completed| Some(Box::new(completed))),
            Ok(RendererCallReplayOperation::Io { pending, operation }) => pending
                .wait()
                .await
                .map(|_| None)
                .with_context(|| format!("{operation} replay dispatch failed")),
            Err(error) => Err(error),
        };
        CompletedRendererCallReplay {
            context: self.context,
            completed,
        }
    }
}

impl CdpConnection {
    pub(crate) fn prepare_renderer_navigation_finish(
        &mut self,
        owner: &CommandOwnerScope,
        token: &DocumentNavigationToken,
    ) -> PreparedRendererNavigationFinish {
        let replacements = self
            .finish_renderer_document_navigation_for_owner(owner, token)
            .and_then(|finish| finish.renderer_call_replacements);
        let mut events = Vec::new();
        let replays = replacements.map(|replacements| {
            let (attachment, terminations, replays) = replacements.into_parts();
            events.extend(self.terminate_prepared_renderer_calls_after_navigation(
                terminations,
                "Execution context was destroyed.",
            ));
            (attachment, replays)
        });
        PreparedRendererNavigationFinish { events, replays }
    }

    pub(crate) fn start_renderer_call_replay(
        &mut self,
        replay: SessionRendererCallReplay,
        attachment: RendererAgentAttachmentId,
    ) -> PendingRendererCallReplay {
        let frontend_session_id = replay.frontend_session_id().map(str::to_owned);
        let inspector_session_id = replay.renderer_inspector_session_id().map(str::to_owned);
        let (correlation, replay, delivery, payload, response_sender) =
            replay.into_replay().into_parts();
        let context = RendererCallReplayContext {
            frontend_session_id,
            correlation,
            delivery,
            response_sender,
            attachment,
        };
        let pending =
            self.admit_renderer_call_replay(&context, inspector_session_id, replay, payload);
        PendingRendererCallReplay { context, pending }
    }

    fn admit_renderer_call_replay(
        &mut self,
        context: &RendererCallReplayContext,
        inspector_session_id: Option<String>,
        replay: RendererCommandReplay,
        payload: String,
    ) -> Result<RendererCallReplayOperation> {
        let session_id = context.frontend_session_id.as_deref();
        let owner = CommandOwnerScope::capture(self, session_id);
        let route = self
            .runtime_protocol_message_page_route_for_owner(&owner)
            .context(NoDocumentLoaded)?;
        anyhow::ensure!(
            route.renderer_agent_attachment_id == context.attachment,
            "renderer replay attachment is no longer current"
        );
        let dispatch = match replay {
            RendererCommandReplay::Inspector(dispatch) => dispatch,
            RendererCommandReplay::PerformanceGetMetrics => {
                debug_assert_eq!(
                    context.delivery,
                    RendererInspectorResponseDelivery::SessionSink
                );
                let page = self.loaded_page_mut_for_protocol_access(session_id)?;
                let result = crate::domains::performance::performance_metrics_result(
                    &page.cached_performance_metric_snapshot(),
                );
                let pending = page
                    .start_performance_get_metrics_from_io_with_response(
                        inspector_session_id,
                        result,
                        context.response_sender.clone(),
                    )
                    .context("Performance replay dispatch failed")?;
                return Ok(RendererCallReplayOperation::Io {
                    pending,
                    operation: "Performance",
                });
            }
            RendererCommandReplay::SetScriptExecutionDisabled { disabled } => {
                debug_assert_eq!(
                    context.delivery,
                    RendererInspectorResponseDelivery::SessionSink
                );
                let page = self.loaded_page_mut_for_protocol_access(session_id)?;
                let pending = page
                    .start_set_script_execution_disabled_from_io_with_response(
                        inspector_session_id,
                        disabled,
                        context.response_sender.clone(),
                    )
                    .context("Emulation replay dispatch failed")?;
                return Ok(RendererCallReplayOperation::Io {
                    pending,
                    operation: "Emulation",
                });
            }
        };
        let raw_json = self.rewrite_runtime_inspector_command_for_session_owner(
            session_id,
            &payload,
            Some((
                context.correlation.frontend_command_id(),
                context.correlation.renderer_call_id(),
            )),
        )?;
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        let sender = context.response_sender.clone();
        let pending = match context.delivery {
            RendererInspectorResponseDelivery::AdapterReply => match dispatch {
                CdpRendererCommandReplayDispatch::ResolveRuntimeContext => page
                    .start_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
                        inspector_session_id, "addBinding".to_owned(), raw_json, sender,
                    ).map(PendingRuntimeProtocolMessageDispatchKind::Page),
                CdpRendererCommandReplayDispatch::Direct => page
                    .start_runtime_protocol_message_for_inspector_session_with_deferred_response(
                        inspector_session_id, raw_json, sender,
                    ).map(PendingRuntimeProtocolMessageDispatchKind::Page),
            },
            RendererInspectorResponseDelivery::SessionSink => {
                debug_assert_eq!(dispatch, CdpRendererCommandReplayDispatch::Direct,
                    "the migrated synchronous IO family must replay directly");
                page.start_routable_runtime_protocol_message_for_inspector_session(
                    inspector_session_id, RendererInspectorCommandRoute::Io, None, raw_json, sender,
                ).map(PendingRuntimeProtocolMessageDispatchKind::Routable)
            }
        }.context("runtime inspector replay dispatch failed")?;
        Ok(RendererCallReplayOperation::Inspector(Box::new(
            PendingRuntimeProtocolMessageDispatch {
                owner,
                route,
                pending,
                response_route: RuntimeProtocolResponseRoute::without_local_receiver_for_delivery(
                    context.delivery,
                ),
            },
        )))
    }

    pub(crate) fn finish_renderer_call_replay(
        &mut self,
        completed: CompletedRendererCallReplay,
    ) -> Vec<BackgroundProtocolEvent> {
        let CompletedRendererCallReplay { context, completed } = completed;
        let mut events = Vec::new();
        let completed = match completed {
            Ok(Some(completed)) => *completed,
            Ok(None) => return events,
            Err(error) => {
                self.settle_renderer_replacement_error(
                    &mut events,
                    context.frontend_session_id.as_deref(),
                    context.delivery,
                    &context.response_sender,
                    context.correlation,
                    &error.to_string(),
                );
                return events;
            }
        };
        let owner = completed.owner().clone();
        let completion = match completed.completion {
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Owner(completion)
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionResponse { completion, .. } => *completion,
            moli_core::page::CompletedRuntimeInspectorCommandDispatch::Inspector
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::InspectorSessionResponse { .. }
            | moli_core::page::CompletedRuntimeInspectorCommandDispatch::OwnerSessionErrorSettled(_) => return events,
        };
        let mut output =
            match self.consume_runtime_protocol_message_completion(&completed.route, completion) {
                Ok(output) => output,
                Err(error) => {
                    send_renderer_replacement_error(
                        &context.response_sender,
                        context.correlation,
                        &format!("runtime inspector replay dispatch failed: {error:#}"),
                    );
                    return events;
                }
            };
        output.bind_renderer_agent_attachment(context.attachment);
        self.ingest_runtime_protocol_message_started_route_output_updates(&completed.route);
        let mut command = CommandDispatchContext::default();
        let completion = command.consume_renderer_command_turn_output(output);
        events.extend(command.take_protocol_events());
        events.extend(command.take_post_response_events());
        let Some(output) = completion.into_runtime_inspector_output() else {
            send_renderer_replacement_error(
                &context.response_sender,
                context.correlation,
                "runtime inspector replay completed with a non-Runtime renderer reply",
            );
            return events;
        };
        if output
            .protocol_response(context.correlation.renderer_call_id().get())
            .is_some()
        {
            let _ = context.response_sender.send_output(output);
        } else {
            let _ = self.route_renderer_runtime_command_output_for_owner_into(
                output,
                None,
                &owner,
                &mut events,
            );
        }
        events
    }
}
