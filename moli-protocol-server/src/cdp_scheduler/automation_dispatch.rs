use std::{cell::RefCell, future, rc::Rc, time::Duration};

use moli_protocol::{
    AutomationCommandTaskStep, CompletedCdpCommandDispatch, OwnedProtocolWaitFailure,
    OwnedProtocolWaitTask, PendingAutomationCommandDispatch,
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
pub(crate) struct AutomationWorkflowReady {
    command_id: u64,
    wait_turn: u64,
}

impl AutomationWorkflowReady {
    pub(crate) fn command_id(&self) -> u64 {
        self.command_id
    }
}

type AutomationReadyReceipt =
    Rc<RefCell<Option<Result<CompletedCdpCommandDispatch, OwnedProtocolWaitFailure>>>>;

pub(crate) struct PendingAutomationExecution {
    pending: PendingAutomationCommandDispatch,
    interleaved_command_events: Vec<BackgroundProtocolEvent>,
    receipt: AutomationReadyReceipt,
    wait_turn: u64,
    task: Option<OwnedProtocolWaitTask>,
    wait_lifetime: Option<moli_protocol::CommandWaitLifetime>,
}

pub(crate) enum AutomationCommandProgress {
    Complete(Box<AutomationExecution>),
    Pending {
        pending: Box<PendingAutomationExecution>,
        protocol_output: ProtocolOutputSequence,
    },
}

impl CdpScheduler {
    pub(super) async fn execute_owned_automation_command_with_interleaved_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: AutomationCommand,
        timeout: Option<Duration>,
        background_id: Option<u64>,
    ) -> AutomationExecution {
        let deadline = timeout.map(|timeout| TokioInstant::now() + timeout);
        let step = self
            .conn
            .start_automation_command_dispatch_with_background_command_id(command, background_id);
        self.drive_automation_command(receivers, step, ProtocolOutputSequence::empty(), deadline)
            .await
    }

    async fn drive_automation_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        step: AutomationCommandTaskStep,
        output: ProtocolOutputSequence,
        mut deadline: Option<TokioInstant>,
    ) -> AutomationExecution {
        let (sender, mut ready) = mpsc::unbounded_channel();
        let mut progress = self
            .advance_automation_command_step(receivers, &sender, step, output, 0)
            .await;
        let mut terminal_error = None;
        let mut input_closed = false;
        loop {
            let (mut pending, mut output) = match progress {
                AutomationCommandProgress::Complete(mut execution) => {
                    if let Some(error) = terminal_error {
                        execution.result = Err(error);
                    }
                    return *execution;
                }
                AutomationCommandProgress::Pending {
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
                    _ = wait_until_automation_revocation(pending.wait_lifetime.as_ref()), if pending.wait_lifetime.is_some() => {
                        break None;
                    }
                    _ = wait_until_automation_deadline(deadline), if deadline.is_some() => {
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
                self.complete_automation_workflow_ready(
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
                let step = self.conn.fail_automation_command_wait(
                    pending.pending,
                    OwnedProtocolWaitFailure::Cancelled,
                );
                self.advance_automation_command_step(
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

    pub(crate) async fn start_owned_automation_command_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<AutomationWorkflowReady>,
        command: AutomationCommand,
    ) -> AutomationCommandProgress {
        let step = self.conn.start_automation_command_dispatch(command);
        self.advance_automation_command_step(
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
        mut pending: Box<PendingAutomationExecution>,
        output: ProtocolOutputSequence,
    ) -> AutomationCommandProgress {
        let mut protocol_output = ProtocolOutputSequence::empty();
        pending.route_protocol_output(output, &mut protocol_output);
        AutomationCommandProgress::Pending {
            pending,
            protocol_output,
        }
    }

    pub(crate) async fn complete_automation_workflow_ready(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<AutomationWorkflowReady>,
        mut pending: Box<PendingAutomationExecution>,
        ready: AutomationWorkflowReady,
        output: ProtocolOutputSequence,
    ) -> AutomationCommandProgress {
        if ready.command_id != pending.command_id() || ready.wait_turn != pending.wait_turn {
            return AutomationCommandProgress::Pending {
                pending,
                protocol_output: output,
            };
        }
        let completed = take_automation_wait_result(
            &pending.receipt,
            &mut pending.task,
            pending.wait_lifetime.as_ref(),
        )
        .await;
        let Some(completed) = completed else {
            return AutomationCommandProgress::Pending {
                pending,
                protocol_output: output,
            };
        };
        let step = match completed {
            Ok(completed) => {
                let mut completed = pending.pending.with_completion(completed);
                completed.append_interleaved_protocol_events(pending.interleaved_command_events);
                self.conn.complete_automation_command_dispatch(completed)
            }
            Err(failure) => {
                pending
                    .pending
                    .append_interleaved_protocol_events(pending.interleaved_command_events);
                self.conn
                    .fail_automation_command_wait(pending.pending, failure)
            }
        };
        self.advance_automation_command_step(receivers, sender, step, output, pending.wait_turn)
            .await
    }

    pub(crate) async fn cancel_devtools_runtime_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending: Box<PendingAutomationExecution>,
    ) -> ProtocolOutputSequence {
        if let Some(task) = pending.task.take() {
            task.cancel_and_join().await;
        }
        // Return a ready context before asking its original recovery to join it.
        drop(pending.receipt.borrow_mut().take());
        pending
            .pending
            .append_interleaved_protocol_events(pending.interleaved_command_events);
        let step = self
            .conn
            .fail_automation_command_wait(pending.pending, OwnedProtocolWaitFailure::Cancelled);
        self.drive_automation_command(receivers, step, ProtocolOutputSequence::empty(), None)
            .await
            .protocol_output
    }

    async fn advance_automation_command_step(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        sender: &mpsc::UnboundedSender<AutomationWorkflowReady>,
        step: AutomationCommandTaskStep,
        mut output: ProtocolOutputSequence,
        previous_turn: u64,
    ) -> AutomationCommandProgress {
        match step {
            AutomationCommandTaskStep::Complete(outcome) => {
                let (result, turn) = outcome.into_result_and_owner_turn();
                let result = match self
                    .apply_automation_command_turn_outcome(receivers, turn)
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
                AutomationCommandProgress::Complete(Box::new(AutomationExecution {
                    result,
                    protocol_output: output,
                }))
            }
            AutomationCommandTaskStep::Pending(mut pending) => {
                self.apply_scheduler_events(pending.take_scheduler_events());
                let id = pending.internal_command_id();
                let wait_turn = previous_turn
                    .checked_add(1)
                    .expect("Runtime wait turn exhausted");
                let wait_lifetime = pending.wait_lifetime();
                let wait = pending.take_wait();
                let receipt: AutomationReadyReceipt = Default::default();
                let ready_receipt = receipt.clone();
                let sender = sender.clone();
                let task = super::command_task::spawn_owned_wait(wait.wait(), move |result| {
                    assert!(
                        ready_receipt.borrow_mut().replace(result).is_none(),
                        "one Runtime wait receipt per turn"
                    );
                    let _ = sender.send(AutomationWorkflowReady {
                        command_id: id,
                        wait_turn,
                    });
                });
                AutomationCommandProgress::Pending {
                    pending: Box::new(PendingAutomationExecution {
                        pending: *pending,
                        interleaved_command_events: Vec::new(),
                        receipt,
                        wait_turn,
                        task: Some(task),
                        wait_lifetime,
                    }),
                    protocol_output: output,
                }
            }
        }
    }
}

impl PendingAutomationExecution {
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

async fn take_automation_wait_result<T>(
    receipt: &Rc<RefCell<Option<Result<T, OwnedProtocolWaitFailure>>>>,
    task: &mut Option<OwnedProtocolWaitTask>,
    lifetime: Option<&moli_protocol::CommandWaitLifetime>,
) -> Option<Result<T, OwnedProtocolWaitFailure>> {
    let completed = receipt.borrow_mut().take()?;
    if lifetime.is_some_and(|lifetime| !lifetime.is_active()) {
        if let Some(task) = task.take() {
            task.cancel_and_join().await;
        }
        drop(completed);
        Some(Err(OwnedProtocolWaitFailure::Cancelled))
    } else {
        task.take();
        Some(completed)
    }
}

async fn wait_until_automation_revocation(lifetime: Option<&moli_protocol::CommandWaitLifetime>) {
    match lifetime {
        Some(lifetime) => lifetime.wait_revoked().await,
        None => future::pending().await,
    }
}

async fn wait_until_automation_deadline(deadline: Option<TokioInstant>) {
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
    #[tokio::test]
    async fn automation_ready_receipt_rejects_revoked_handler_after_native_success() {
        tokio::task::LocalSet::new()
            .run_until(async {
                use std::{cell::Cell, rc::Rc};
                struct AcceptedNativeReply(Rc<Cell<bool>>);
                impl Drop for AcceptedNativeReply {
                    fn drop(&mut self) {
                        self.0.set(true);
                    }
                }
                let (lifetime, revoke) =
                    moli_protocol::test_support::revocable_command_wait_lifetime();
                let dropped = Rc::new(Cell::new(false));
                let completed = AcceptedNativeReply(dropped.clone());
                let receipt = Rc::new(std::cell::RefCell::new(None));
                let ready_receipt = receipt.clone();
                let (ready, wait) = tokio::sync::oneshot::channel();
                let mut task = Some(super::super::command_task::spawn_owned_wait(
                    async move { completed },
                    move |result| {
                        ready_receipt.borrow_mut().replace(result);
                        let _ = ready.send(());
                    },
                ));
                wait.await.unwrap();
                assert!(!dropped.get());
                revoke();
                let result =
                    super::take_automation_wait_result(&receipt, &mut task, Some(&lifetime))
                        .await
                        .unwrap();
                assert!(matches!(
                    result,
                    Err(moli_protocol::OwnedProtocolWaitFailure::Cancelled)
                ));
                assert!(dropped.get());
                assert!(task.is_none() && receipt.borrow().is_none());
            })
            .await;
    }
}
