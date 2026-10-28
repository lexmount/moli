use std::{cell::RefCell, future, rc::Rc, time::Duration};

use moli_protocol::{
    CompletedCdpCommandDispatch, DevToolsRuntimeCommandTaskStep, OwnedProtocolWaitFailure,
    OwnedProtocolWaitTask, PendingDevToolsRuntimeCommandDispatch,
    automation::{AutomationCommand, DevToolsError, DevToolsErrorKind},
    conn::BackgroundProtocolEvent,
};
use tokio::{
    sync::mpsc,
    time::{Instant as TokioInstant, sleep_until},
};

use super::{
    AutomationExecution, CdpScheduler, CdpSchedulerEventReceivers, ProtocolOutputSequence,
};

/// A notification only. The move-only receipt stays in its original command
/// owner even if this notification is late, duplicated or discarded.
#[derive(Debug)]
pub(crate) struct DevToolsRuntimeWorkflowReady {
    command_id: u64,
    wait_turn: u64,
}

impl DevToolsRuntimeWorkflowReady {
    pub(crate) fn command_id(&self) -> u64 {
        self.command_id
    }
}

type RuntimeReadyReceipt =
    Rc<RefCell<Option<Result<CompletedCdpCommandDispatch, OwnedProtocolWaitFailure>>>>;

pub(crate) struct PendingDevToolsRuntimeExecution {
    pending: PendingDevToolsRuntimeCommandDispatch,
    interleaved_command_events: Vec<BackgroundProtocolEvent>,
    receipt: RuntimeReadyReceipt,
    wait_turn: u64,
    task: Option<OwnedProtocolWaitTask>,
}

pub(crate) enum DevToolsRuntimeCommandProgress {
    Complete(Box<AutomationExecution>),
    Pending {
        pending: Box<PendingDevToolsRuntimeExecution>,
        protocol_output: ProtocolOutputSequence,
    },
}

pub(super) fn automation_command_uses_interleaved_runtime_dispatch(
    command: &AutomationCommand,
) -> bool {
    matches!(
        command,
        AutomationCommand::GetRealms(_)
            | AutomationCommand::EvaluateScript(_)
            | AutomationCommand::CallFunction(_)
            | AutomationCommand::TerminateExecution(_)
            | AutomationCommand::LocateNodes(_)
            | AutomationCommand::ReleaseObjects(_)
    )
}

impl CdpScheduler {
    pub(super) async fn execute_devtools_runtime_command_with_interleaved_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: AutomationCommand,
    ) -> AutomationExecution {
        self.execute_devtools_runtime_command_with_interleaved_progress_until(
            receivers, command, None,
        )
        .await
    }

    pub(crate) async fn execute_devtools_runtime_command_with_interleaved_progress_timeout(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: AutomationCommand,
        timeout: Duration,
    ) -> AutomationExecution {
        self.execute_devtools_runtime_command_with_interleaved_progress_until(
            receivers,
            command,
            Some(TokioInstant::now() + timeout),
        )
        .await
    }

    async fn execute_devtools_runtime_command_with_interleaved_progress_until(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: AutomationCommand,
        deadline: Option<TokioInstant>,
    ) -> AutomationExecution {
        let step = self.conn.start_devtools_runtime_command_dispatch(command);
        self.drive_runtime_command(receivers, step, ProtocolOutputSequence::empty(), deadline)
            .await
    }

    async fn drive_runtime_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        step: DevToolsRuntimeCommandTaskStep,
        output: ProtocolOutputSequence,
        mut deadline: Option<TokioInstant>,
    ) -> AutomationExecution {
        let (sender, mut ready) = mpsc::unbounded_channel();
        let mut progress = self
            .advance_runtime_command_step(receivers, &sender, step, output, 0)
            .await;
        let mut terminal_error = None;
        let mut input_closed = false;
        loop {
            let (mut pending, mut output) = match progress {
                DevToolsRuntimeCommandProgress::Complete(mut execution) => {
                    if let Some(error) = terminal_error {
                        execution.result = Err(error);
                    }
                    return *execution;
                }
                DevToolsRuntimeCommandProgress::Pending {
                    pending,
                    protocol_output,
                } => (pending, protocol_output),
            };
            let completed = loop {
                if !input_closed {
                    let events = self
                        .complete_ready_protocol_residences_after_command()
                        .await;
                    pending.route_protocol_output(events, &mut output);
                }
                tokio::select! {
                    biased;
                    Some(notification) = ready.recv() => {
                        if notification.command_id == pending.command_id() && notification.wait_turn == pending.wait_turn { break Some(notification); }
                    }
                    _ = wait_until_runtime_deadline(deadline), if deadline.is_some() => {
                        terminal_error = Some(DevToolsError::new(DevToolsErrorKind::Timeout, "script timed out"));
                        deadline = None;
                        break None;
                    }
                    input = async {
                        if input_closed {
                            match receivers.protocol_state_request_rx.recv().await {
                                Some(request) => Some(super::CdpSchedulerInterleavedInput::ProtocolStateRequest(request)),
                                None => future::pending().await,
                            }
                        } else { receivers.recv_interleaved_input().await }
                    } => {
                        let Some(input) = input else {
                            input_closed = true;
                            terminal_error.get_or_insert_with(|| DevToolsError::new(DevToolsErrorKind::Internal, SchedulerInputClosed));
                            break None;
                        };
                        if input_closed {
                            if let super::CdpSchedulerInterleavedInput::ProtocolStateRequest(request) = input { self.apply_protocol_state_request(request); }
                            continue;
                        }
                        match self.complete_interleaved_scheduler_input(receivers, input).await {
                            Ok(events) => pending.route_protocol_output(events, &mut output),
                            Err(failure) => {
                                let (events, error) = failure.into_parts();
                                pending.route_protocol_output(events, &mut output);
                                terminal_error.get_or_insert(error);
                                deadline = None;
                                break None;
                            }
                        }
                    }
                }
            };
            progress = if let Some(notification) = completed {
                self.complete_devtools_runtime_workflow_ready(
                    receivers,
                    &sender,
                    pending,
                    notification,
                    output,
                )
                .await
            } else {
                if let Some(task) = pending.task.take() {
                    task.cancel_and_join().await;
                }
                drop(pending.receipt.borrow_mut().take());
                pending
                    .pending
                    .append_interleaved_protocol_events(pending.interleaved_command_events);
                let step = self.conn.fail_devtools_runtime_command_wait(
                    pending.pending,
                    OwnedProtocolWaitFailure::Cancelled,
                );
                self.advance_runtime_command_step(
                    receivers,
                    &sender,
                    step,
                    output,
                    pending.wait_turn,
                )
                .await
            };
        }
    }

    pub(crate) async fn start_devtools_runtime_command_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<DevToolsRuntimeWorkflowReady>,
        command: AutomationCommand,
    ) -> DevToolsRuntimeCommandProgress {
        let step = self.conn.start_devtools_runtime_command_dispatch(command);
        self.advance_runtime_command_step(
            receivers,
            sender,
            step,
            ProtocolOutputSequence::empty(),
            0,
        )
        .await
    }

    pub(crate) fn route_devtools_runtime_protocol_output(
        &mut self,
        mut pending: Box<PendingDevToolsRuntimeExecution>,
        output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        let mut protocol_output = ProtocolOutputSequence::empty();
        pending.route_protocol_output(output, &mut protocol_output);
        DevToolsRuntimeCommandProgress::Pending {
            pending,
            protocol_output,
        }
    }

    pub(crate) async fn complete_devtools_runtime_workflow_ready(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<DevToolsRuntimeWorkflowReady>,
        mut pending: Box<PendingDevToolsRuntimeExecution>,
        ready: DevToolsRuntimeWorkflowReady,
        output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        if ready.command_id != pending.command_id() || ready.wait_turn != pending.wait_turn {
            return DevToolsRuntimeCommandProgress::Pending {
                pending,
                protocol_output: output,
            };
        }
        let completed = pending.receipt.borrow_mut().take();
        let Some(completed) = completed else {
            return DevToolsRuntimeCommandProgress::Pending {
                pending,
                protocol_output: output,
            };
        };
        pending.task.take();
        let step = match completed {
            Ok(completed) => {
                let mut completed = pending.pending.with_completion(completed);
                completed.append_interleaved_protocol_events(pending.interleaved_command_events);
                self.conn
                    .complete_devtools_runtime_command_dispatch(completed)
            }
            Err(failure) => {
                pending
                    .pending
                    .append_interleaved_protocol_events(pending.interleaved_command_events);
                self.conn
                    .fail_devtools_runtime_command_wait(pending.pending, failure)
            }
        };
        self.advance_runtime_command_step(receivers, sender, step, output, pending.wait_turn)
            .await
    }

    pub(crate) async fn cancel_devtools_runtime_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending: Box<PendingDevToolsRuntimeExecution>,
    ) -> ProtocolOutputSequence {
        if let Some(task) = pending.task.take() {
            task.cancel_and_join().await;
        }
        // Return a ready context before asking its original recovery to join it.
        drop(pending.receipt.borrow_mut().take());
        pending
            .pending
            .append_interleaved_protocol_events(pending.interleaved_command_events);
        let step = self.conn.fail_devtools_runtime_command_wait(
            pending.pending,
            OwnedProtocolWaitFailure::Cancelled,
        );
        self.drive_runtime_command(receivers, step, ProtocolOutputSequence::empty(), None)
            .await
            .protocol_output
    }

    async fn advance_runtime_command_step(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<DevToolsRuntimeWorkflowReady>,
        step: DevToolsRuntimeCommandTaskStep,
        mut output: ProtocolOutputSequence,
        previous_turn: u64,
    ) -> DevToolsRuntimeCommandProgress {
        match step {
            DevToolsRuntimeCommandTaskStep::Complete(outcome) => {
                let (result, turn) = outcome.into_result_and_owner_turn();
                let result = match self
                    .apply_renderer_owner_turn_outcome(receivers, turn)
                    .await
                {
                    Ok(events) => {
                        output.append(events);
                        result
                    }
                    Err(failure) => {
                        let (events, error) = failure.into_parts();
                        output.append(events);
                        Err(error)
                    }
                };
                DevToolsRuntimeCommandProgress::Complete(Box::new(AutomationExecution {
                    result,
                    protocol_output: output,
                }))
            }
            DevToolsRuntimeCommandTaskStep::Pending(mut pending) => {
                self.apply_scheduler_events(pending.take_scheduler_events());
                let id = pending.internal_command_id();
                let wait_turn = previous_turn
                    .checked_add(1)
                    .expect("Runtime wait turn exhausted");
                let wait = pending.take_wait();
                let receipt: RuntimeReadyReceipt = Default::default();
                let ready_receipt = receipt.clone();
                let sender = sender.clone();
                let task = super::command_task::spawn_owned_wait(wait.wait(), move |result| {
                    assert!(
                        ready_receipt.borrow_mut().replace(result).is_none(),
                        "one Runtime wait receipt per turn"
                    );
                    let _ = sender.send(DevToolsRuntimeWorkflowReady {
                        command_id: id,
                        wait_turn,
                    });
                });
                DevToolsRuntimeCommandProgress::Pending {
                    pending: Box::new(PendingDevToolsRuntimeExecution {
                        pending: *pending,
                        interleaved_command_events: Vec::new(),
                        receipt,
                        wait_turn,
                        task: Some(task),
                    }),
                    protocol_output: output,
                }
            }
        }
    }
}

impl PendingDevToolsRuntimeExecution {
    pub(crate) fn command_id(&self) -> u64 {
        self.pending.internal_command_id()
    }

    fn route_protocol_output(
        &mut self,
        mut output: ProtocolOutputSequence,
        published: &mut ProtocolOutputSequence,
    ) {
        self.interleaved_command_events
            .extend(output.take_protocol_events_with_id(self.command_id()));
        published.append(output);
    }
}

async fn wait_until_runtime_deadline(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => future::pending().await,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("SchedulerInputClosed")]
struct SchedulerInputClosed;

#[cfg(test)]
mod tests {
    use moli_protocol::{
        BackgroundProtocolEvent,
        automation::{AutomationEvent, DevToolsFrameId, PageFileChooserOpenedEvent},
    };
    use serde_json::json;

    #[test]
    fn runtime_deferred_initial_output_preserves_typed_event_sidecars() {
        let output = super::ProtocolOutputSequence::from_background_events(vec![
            BackgroundProtocolEvent::immediate_automation_event(
                json!({
                    "method": "Page.fileChooserOpened",
                    "params": {
                        "frameId": "FRAME-runtime",
                        "backendNodeId": 91,
                        "mode": "selectSingle"
                    },
                    "sessionId": "SID-runtime"
                }),
                AutomationEvent::PageFileChooserOpened(PageFileChooserOpenedEvent {
                    frame_id: DevToolsFrameId::from("FRAME-runtime"),
                    backend_node_id: 91,
                    mode: "selectSingle".to_owned(),
                    element_shared_id: None,
                }),
            ),
        ]);

        let mut events = output.into_background_events();
        let (message, automation_event) = events
            .pop()
            .expect("runtime deferred output should contain the raw event")
            .into_parts();

        assert_eq!(message["method"], json!("Page.fileChooserOpened"));
        assert_eq!(message["sessionId"], json!("SID-runtime"));
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::PageFileChooserOpened(event))
                if event.frame_id.as_str() == "FRAME-runtime"
                    && event.backend_node_id == 91
                    && event.mode == "selectSingle"
        ));
    }

    #[test]
    fn runtime_deferred_initial_output_keeps_command_responses_sidecar_free() {
        let mut output = super::ProtocolOutputSequence::from_background_events(vec![
            BackgroundProtocolEvent::immediate(json!({
                "id": 71,
                "result": {},
                "sessionId": "SID-runtime"
            })),
        ]);

        let responses = output
            .take_protocol_events_with_id(71)
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert_eq!(
            responses,
            vec![json!({
                "id": 71,
                "result": {},
                "sessionId": "SID-runtime"
            })]
        );
        assert!(output.is_empty());
    }
}
