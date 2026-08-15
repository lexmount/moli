use std::{future, time::Duration};

use moli_protocol::{
    CompletedDevToolsRuntimeCommandDispatch, DevToolsRuntimeCommandTaskStep,
    PendingDevToolsRuntimeCommandDispatch,
    conn::{
        BackgroundProtocolEvent, RuntimeInspectorAsyncCompletionReceiver,
        RuntimeInspectorResponseReady,
    },
    devtools_runtime::{DevToolsCommand, DevToolsError, DevToolsErrorKind},
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{Instant as TokioInstant, sleep_until},
};

use super::{
    CdpScheduler, CdpSchedulerEventReceivers, CdpSchedulerInterleavedInput,
    DevToolsCommandExecution, ProtocolOutputSequence, RendererOutputTransportFailure,
};

pub(crate) struct PendingDevToolsRuntimeDeferredReplyExecution {
    pending: PendingDevToolsRuntimeCommandDispatch,
    interleaved_command_events: Vec<BackgroundProtocolEvent>,
    response_wait_handle: RuntimeResponseReadyWaitHandle,
}

pub(crate) enum DevToolsRuntimeCommandProgress {
    Complete(Box<DevToolsCommandExecution>),
    PendingDeferredReply {
        pending: Box<PendingDevToolsRuntimeDeferredReplyExecution>,
        protocol_output: ProtocolOutputSequence,
    },
}

pub(super) fn devtools_command_uses_interleaved_runtime_dispatch(
    command: &DevToolsCommand,
) -> bool {
    matches!(
        command,
        DevToolsCommand::GetRealms(_)
            | DevToolsCommand::EvaluateScript(_)
            | DevToolsCommand::CallFunction(_)
            | DevToolsCommand::TerminateExecution(_)
            | DevToolsCommand::LocateNodes(_)
            | DevToolsCommand::ReleaseObjects(_)
    )
}

impl CdpScheduler {
    async fn route_renderer_response_to_devtools_pending(
        &mut self,
        pending: &mut PendingDevToolsRuntimeCommandDispatch,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        pending
            .route_scheduler_deferred_inspector_response(&mut self.conn, response)
            .await
    }

    pub(super) async fn execute_devtools_runtime_command_with_interleaved_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
    ) -> DevToolsCommandExecution {
        self.execute_devtools_runtime_command_with_interleaved_progress_until(
            receivers, command, None,
        )
        .await
    }

    pub(crate) async fn execute_devtools_runtime_command_with_interleaved_progress_timeout(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
        timeout: Duration,
    ) -> DevToolsCommandExecution {
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
        command: DevToolsCommand,
        deadline: Option<TokioInstant>,
    ) -> DevToolsCommandExecution {
        let mut protocol_output = ProtocolOutputSequence::empty();
        let mut step = self
            .conn
            .start_devtools_runtime_command_dispatch(command)
            .await;
        loop {
            match step {
                DevToolsRuntimeCommandTaskStep::Complete(outcome) => {
                    let (result, scheduler_events, protocol_events, renderer_output_predecessor) =
                        outcome.into_complete_parts();
                    if let Some(predecessor) = renderer_output_predecessor {
                        match self
                            .project_renderer_output_predecessor_before_devtools_result(
                                receivers,
                                &predecessor,
                            )
                            .await
                        {
                            Ok(output) => protocol_output.append(output),
                            Err(failure) => {
                                let (output, error) = failure.into_parts();
                                protocol_output.append(output);
                                return DevToolsCommandExecution {
                                    result: Err(error),
                                    protocol_output,
                                };
                            }
                        }
                    }
                    protocol_output.append(ProtocolOutputSequence::from_background_events(
                        protocol_events,
                    ));
                    self.apply_scheduler_events(scheduler_events);
                    return DevToolsCommandExecution {
                        result,
                        protocol_output,
                    };
                }
                DevToolsRuntimeCommandTaskStep::Pending(mut pending) => {
                    let scheduler_events = pending.take_scheduler_events();
                    self.apply_scheduler_events(scheduler_events);
                    let completed = match self
                        .wait_for_devtools_runtime_command_progress(
                            receivers,
                            *pending,
                            &mut protocol_output,
                            deadline,
                        )
                        .await
                    {
                        Ok(Some(completed)) => completed,
                        Ok(None) => {
                            return DevToolsCommandExecution {
                                result: Err(DevToolsError::new(
                                    DevToolsErrorKind::Internal,
                                    "SchedulerInputClosed",
                                )),
                                protocol_output,
                            };
                        }
                        Err(error) => {
                            return DevToolsCommandExecution {
                                result: Err(error),
                                protocol_output,
                            };
                        }
                    };
                    step = self
                        .conn
                        .complete_devtools_runtime_command_dispatch(completed)
                        .await;
                }
            }
        }
    }

    pub(crate) async fn start_devtools_runtime_command_with_deferred_reply_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        runtime_response_ready_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
        command: DevToolsCommand,
    ) -> DevToolsRuntimeCommandProgress {
        let protocol_output = ProtocolOutputSequence::empty();
        let step = self
            .conn
            .start_devtools_runtime_command_dispatch(command)
            .await;
        self.continue_devtools_runtime_command_until_deferred_reply_or_complete(
            receivers,
            runtime_response_ready_tx,
            step,
            protocol_output,
        )
        .await
    }

    pub(crate) async fn advance_devtools_runtime_deferred_reply_after_protocol_output(
        &mut self,
        pending: Box<PendingDevToolsRuntimeDeferredReplyExecution>,
        output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        self.advance_devtools_runtime_deferred_reply_once(*pending, output)
            .await
    }

    pub(crate) async fn advance_devtools_runtime_deferred_reply_after_renderer_response(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        runtime_response_ready_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
        pending: Box<PendingDevToolsRuntimeDeferredReplyExecution>,
        response: RuntimeInspectorResponseReady,
    ) -> DevToolsRuntimeCommandProgress {
        let mut pending = *pending;
        let protocol_output = ProtocolOutputSequence::empty();
        if self
            .route_renderer_response_to_devtools_pending(&mut pending.pending, response)
            .await
        {
            return self
                .complete_devtools_runtime_deferred_reply(
                    receivers,
                    runtime_response_ready_tx,
                    pending,
                    protocol_output,
                )
                .await;
        }
        DevToolsRuntimeCommandProgress::PendingDeferredReply {
            pending: Box::new(pending),
            protocol_output,
        }
    }

    pub(crate) fn cancel_devtools_runtime_deferred_reply(
        &mut self,
        pending: Box<PendingDevToolsRuntimeDeferredReplyExecution>,
    ) {
        let pending = *pending;
        pending
            .pending
            .forget_scheduler_deferred_inspector_reply(&mut self.conn);
    }

    async fn continue_devtools_runtime_command_until_deferred_reply_or_complete(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        runtime_response_ready_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
        mut step: DevToolsRuntimeCommandTaskStep,
        mut protocol_output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        loop {
            match step {
                DevToolsRuntimeCommandTaskStep::Complete(outcome) => {
                    let (result, scheduler_events, protocol_events, renderer_output_predecessor) =
                        outcome.into_complete_parts();
                    if let Some(predecessor) = renderer_output_predecessor {
                        match self
                            .project_renderer_output_predecessor_before_devtools_result(
                                receivers,
                                &predecessor,
                            )
                            .await
                        {
                            Ok(output) => protocol_output.append(output),
                            Err(failure) => {
                                let (output, error) = failure.into_parts();
                                protocol_output.append(output);
                                return DevToolsRuntimeCommandProgress::Complete(Box::new(
                                    DevToolsCommandExecution {
                                        result: Err(error),
                                        protocol_output,
                                    },
                                ));
                            }
                        }
                    }
                    protocol_output.append(ProtocolOutputSequence::from_background_events(
                        protocol_events,
                    ));
                    self.apply_scheduler_events(scheduler_events);
                    return DevToolsRuntimeCommandProgress::Complete(Box::new(
                        DevToolsCommandExecution {
                            result,
                            protocol_output,
                        },
                    ));
                }
                DevToolsRuntimeCommandTaskStep::Pending(mut pending) => {
                    let scheduler_events = pending.take_scheduler_events();
                    self.apply_scheduler_events(scheduler_events);
                    if pending.waits_for_scheduler_deferred_inspector_reply() {
                        let mut pending = *pending;
                        protocol_output.append(ProtocolOutputSequence::from_background_events(
                            pending.take_scheduler_deferred_inspector_reply_events(),
                        ));
                        let command_id = pending
                            .command_id()
                            .unwrap_or_else(|| pending.internal_command_id());
                        let session_id = pending.session_id().map(str::to_owned);
                        let response_wait_handle = match pending
                            .take_scheduler_deferred_inspector_reply_receiver()
                        {
                            Some(response_rx) => RuntimeResponseReadyWaitHandle::new(
                                start_devtools_runtime_response_ready_wait(
                                    command_id,
                                    session_id,
                                    response_rx,
                                    runtime_response_ready_tx,
                                ),
                            ),
                            None => {
                                let _ = runtime_response_ready_tx.send(
                                    RuntimeInspectorResponseReady::new(
                                        command_id,
                                        session_id.as_deref(),
                                        Err("RuntimeDeferredInspectorResponseMissing".to_owned()),
                                    ),
                                );
                                RuntimeResponseReadyWaitHandle::none()
                            }
                        };
                        let pending = PendingDevToolsRuntimeDeferredReplyExecution {
                            pending,
                            interleaved_command_events: Vec::new(),
                            response_wait_handle,
                        };
                        return DevToolsRuntimeCommandProgress::PendingDeferredReply {
                            pending: Box::new(pending),
                            protocol_output,
                        };
                    }
                    let completed = match self
                        .wait_for_devtools_runtime_command_progress(
                            receivers,
                            *pending,
                            &mut protocol_output,
                            None,
                        )
                        .await
                    {
                        Ok(Some(completed)) => completed,
                        Ok(None) => {
                            return DevToolsRuntimeCommandProgress::Complete(Box::new(
                                DevToolsCommandExecution {
                                    result: Err(DevToolsError::new(
                                        DevToolsErrorKind::Internal,
                                        "SchedulerInputClosed",
                                    )),
                                    protocol_output,
                                },
                            ));
                        }
                        Err(error) => {
                            return DevToolsRuntimeCommandProgress::Complete(Box::new(
                                DevToolsCommandExecution {
                                    result: Err(error),
                                    protocol_output,
                                },
                            ));
                        }
                    };
                    step = self
                        .conn
                        .complete_devtools_runtime_command_dispatch(completed)
                        .await;
                }
            }
        }
    }

    async fn complete_devtools_runtime_interleaved_input(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        input: CdpSchedulerInterleavedInput,
    ) -> Result<ProtocolOutputSequence, RendererOutputTransportFailure> {
        match input {
            // A deferred Runtime reply owns command-correlated background
            // events directly. Preserve that routing while sharing the same
            // cancellation-safe receive boundary as other command waits.
            CdpSchedulerInterleavedInput::BackgroundEvent(event) => {
                Ok(ProtocolOutputSequence::from_background_event(event))
            }
            input => {
                self.complete_interleaved_scheduler_input(receivers, input)
                    .await
            }
        }
    }

    async fn wait_for_devtools_runtime_command_progress(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        pending: PendingDevToolsRuntimeCommandDispatch,
        protocol_output: &mut ProtocolOutputSequence,
        deadline: Option<TokioInstant>,
    ) -> Result<Option<CompletedDevToolsRuntimeCommandDispatch>, DevToolsError> {
        if pending.waits_for_scheduler_deferred_inspector_reply() {
            return self
                .wait_for_devtools_runtime_deferred_inspector_reply(
                    receivers,
                    pending,
                    protocol_output,
                    deadline,
                )
                .await;
        }
        let mut interleaved_command_events = Vec::new();
        let command_id = pending.internal_command_id();
        let pending_completion = pending.wait();
        tokio::pin!(pending_completion);
        loop {
            let mut ready_output = self
                .complete_ready_protocol_residences_after_command()
                .await;
            interleaved_command_events
                .extend(ready_output.take_protocol_events_with_id(command_id));
            if !ready_output.is_empty() {
                protocol_output.append(ready_output);
                continue;
            }
            tokio::select! {
                biased;
                mut completed = &mut pending_completion => {
                    completed.append_interleaved_protocol_events(interleaved_command_events);
                    return Ok(Some(completed));
                }
                _ = wait_until_runtime_deadline(deadline) => {
                    return Err(runtime_command_timeout_error());
                }
                maybe_input = receivers.recv_interleaved_input() => {
                    let Some(input) = maybe_input else {
                        return Ok(None);
                    };
                    // The selected input is now move-owned by this branch.
                    // Complete it before racing the command reply again, so
                    // projection cannot be canceled after channel dequeue.
                    let mut output = match self
                        .complete_interleaved_scheduler_input(receivers, input)
                        .await
                    {
                        Ok(output) => output,
                        Err(failure) => {
                            let (output, error) = failure.into_parts();
                            protocol_output.append(output);
                            return Err(error);
                        }
                    };
                    interleaved_command_events.extend(output.take_protocol_events_with_id(command_id));
                    protocol_output.append(output);
                }
            }
        }
    }

    async fn wait_for_devtools_runtime_deferred_inspector_reply(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        mut pending: PendingDevToolsRuntimeCommandDispatch,
        protocol_output: &mut ProtocolOutputSequence,
        deadline: Option<TokioInstant>,
    ) -> Result<Option<CompletedDevToolsRuntimeCommandDispatch>, DevToolsError> {
        let command_id = pending
            .command_id()
            .unwrap_or_else(|| pending.internal_command_id());
        let session_id = pending.session_id().map(str::to_owned);
        protocol_output.append(ProtocolOutputSequence::from_background_events(
            pending.take_scheduler_deferred_inspector_reply_events(),
        ));
        let mut renderer_response_rx =
            match pending.take_scheduler_deferred_inspector_reply_receiver() {
                Some(response_rx) => response_rx,
                None => {
                    pending.forget_scheduler_deferred_inspector_reply(&mut self.conn);
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::Internal,
                        "RuntimeDeferredInspectorResponseMissing",
                    ));
                }
            };
        let mut renderer_response_done = false;
        let mut interleaved_command_events = Vec::new();
        loop {
            let mut ready_output = self
                .complete_ready_protocol_residences_after_command()
                .await;
            let mut command_events = ready_output.take_protocol_events_with_id(command_id);
            if !command_events.is_empty() {
                pending.forget_scheduler_deferred_inspector_reply(&mut self.conn);
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "RuntimeDeferredReplyLooseProtocolResponse",
                ));
            }
            interleaved_command_events.append(&mut command_events);
            if !ready_output.is_empty() {
                protocol_output.append(ready_output);
            }
            tokio::select! {
                biased;
                renderer_response = &mut renderer_response_rx, if !renderer_response_done => {
                    renderer_response_done = true;
                    let response = RuntimeInspectorResponseReady::new(
                        command_id,
                        session_id.as_deref(),
                        renderer_response
                            .map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned()),
                    );
                    if self
                        .route_renderer_response_to_devtools_pending(&mut pending, response)
                        .await
                    {
                        let mut completed =
                            pending.complete_scheduler_deferred_inspector_reply(&mut self.conn);
                        completed
                            .append_interleaved_protocol_events(interleaved_command_events);
                        return Ok(Some(completed));
                    }
                    continue;
                }
                _ = wait_until_runtime_deadline(deadline) => {
                    pending.forget_scheduler_deferred_inspector_reply(&mut self.conn);
                    return Err(runtime_command_timeout_error());
                }
                maybe_input = receivers.recv_interleaved_input() => {
                    let Some(input) = maybe_input else {
                        return Ok(None);
                    };
                    let mut output = match self
                        .complete_devtools_runtime_interleaved_input(receivers, input)
                        .await
                    {
                        Ok(output) => output,
                        Err(failure) => {
                            let (output, error) = failure.into_parts();
                            protocol_output.append(output);
                            return Err(error);
                        }
                    };
                    let mut command_events = output.take_protocol_events_with_id(command_id);
                    if !command_events.is_empty() {
                        pending.forget_scheduler_deferred_inspector_reply(&mut self.conn);
                        return Err(DevToolsError::new(
                            DevToolsErrorKind::Internal,
                            "RuntimeDeferredReplyLooseProtocolResponse",
                        ));
                    }
                    interleaved_command_events.append(&mut command_events);
                    protocol_output.append(output);
                }
            }
        }
    }

    async fn advance_devtools_runtime_deferred_reply_once(
        &mut self,
        mut pending: PendingDevToolsRuntimeDeferredReplyExecution,
        initial_output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        let mut protocol_output = ProtocolOutputSequence::empty();
        if pending.route_protocol_output(initial_output, &mut protocol_output) {
            return complete_devtools_runtime_deferred_reply_with_loose_response_error(
                pending,
                protocol_output,
            );
        }
        let ready_output = self
            .complete_ready_protocol_residences_after_command()
            .await;
        if pending.route_protocol_output(ready_output, &mut protocol_output) {
            return complete_devtools_runtime_deferred_reply_with_loose_response_error(
                pending,
                protocol_output,
            );
        }
        DevToolsRuntimeCommandProgress::PendingDeferredReply {
            pending: Box::new(pending),
            protocol_output,
        }
    }

    async fn complete_devtools_runtime_deferred_reply(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        runtime_response_ready_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
        pending: PendingDevToolsRuntimeDeferredReplyExecution,
        protocol_output: ProtocolOutputSequence,
    ) -> DevToolsRuntimeCommandProgress {
        let PendingDevToolsRuntimeDeferredReplyExecution {
            pending,
            interleaved_command_events,
            response_wait_handle,
        } = pending;
        drop(response_wait_handle);
        let mut completed = pending.complete_scheduler_deferred_inspector_reply(&mut self.conn);
        completed.append_interleaved_protocol_events(interleaved_command_events);
        let step = self
            .conn
            .complete_devtools_runtime_command_dispatch(completed)
            .await;
        self.continue_devtools_runtime_command_until_deferred_reply_or_complete(
            receivers,
            runtime_response_ready_tx,
            step,
            protocol_output,
        )
        .await
    }
}

impl PendingDevToolsRuntimeDeferredReplyExecution {
    pub(crate) fn command_id(&self) -> u64 {
        self.pending
            .command_id()
            .unwrap_or_else(|| self.pending.internal_command_id())
    }

    fn route_protocol_output(
        &mut self,
        mut output: ProtocolOutputSequence,
        protocol_output: &mut ProtocolOutputSequence,
    ) -> bool {
        let command_id = self
            .pending
            .command_id()
            .unwrap_or_else(|| self.pending.internal_command_id());
        let mut command_events = output.take_protocol_events_with_id(command_id);
        let saw_command_response = !command_events.is_empty();
        self.interleaved_command_events.append(&mut command_events);
        if !output.is_empty() {
            protocol_output.append(output);
        }
        saw_command_response
    }
}

fn complete_devtools_runtime_deferred_reply_with_loose_response_error(
    pending: PendingDevToolsRuntimeDeferredReplyExecution,
    protocol_output: ProtocolOutputSequence,
) -> DevToolsRuntimeCommandProgress {
    drop(pending);
    DevToolsRuntimeCommandProgress::Complete(Box::new(DevToolsCommandExecution {
        result: Err(DevToolsError::new(
            DevToolsErrorKind::Internal,
            "RuntimeDeferredReplyLooseProtocolResponse",
        )),
        protocol_output,
    }))
}

struct RuntimeResponseReadyWaitHandle(Option<JoinHandle<()>>);

impl RuntimeResponseReadyWaitHandle {
    fn none() -> Self {
        Self(None)
    }

    fn new(handle: JoinHandle<()>) -> Self {
        Self(Some(handle))
    }
}

impl Drop for RuntimeResponseReadyWaitHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

fn start_devtools_runtime_response_ready_wait(
    command_id: u64,
    session_id: Option<String>,
    response_rx: RuntimeInspectorAsyncCompletionReceiver,
    response_tx: &mpsc::UnboundedSender<RuntimeInspectorResponseReady>,
) -> JoinHandle<()> {
    let response_tx = response_tx.clone();
    tokio::task::spawn_local(async move {
        let response = response_rx
            .await
            .map_err(|_| "RuntimeDeferredInspectorResponseCanceled".to_owned());
        let _ = response_tx.send(RuntimeInspectorResponseReady::new(
            command_id,
            session_id.as_deref(),
            response,
        ));
    })
}

async fn wait_until_runtime_deadline(deadline: Option<TokioInstant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => future::pending().await,
    }
}

fn runtime_command_timeout_error() -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Timeout, "script timed out")
}

#[cfg(test)]
mod tests {
    use moli_protocol::{
        BackgroundProtocolEvent,
        devtools_runtime::{AutomationEvent, DevToolsFrameId, PageFileChooserOpenedEvent},
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
