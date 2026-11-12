use super::*;
use moli_core::ace_page_task::{AcePageTask, AcePageTaskKind};
use moli_protocol::ace_page_task::{
    AcePageTaskError, AcePageTaskResult, PendingAcePageTaskDispatch,
};

#[cfg(test)]
mod tests;

pub(super) fn blocks_page_javascript(in_flight: &InFlightCommands) -> bool {
    in_flight
        .values()
        .any(|work| matches!(work, InFlightWork::AcePageTask(state) if state.kind == AcePageTaskKind::Input))
}

fn owner_output_metadata() -> InFlightCommandMetadata {
    // Flush only the retained owner output. No protocol command, response
    // receiver, or CDP session is associated with an embedder result.
    InFlightCommandMetadata {
        method: None,
        id: None,
        session_id: None,
        started: None,
        command_output_session_id: None,
        internal_response: None,
    }
}

pub(super) struct InFlightAcePageTask {
    pub kind: AcePageTaskKind,
    pub turn: u64,
    pub wait_task: CommandWaitTask,
    pending: PendingAcePageTaskDispatch,
    context: CommandDispatchContext,
    permit: CommandOutputReleasePermit,
    reply: oneshot::Sender<Result<AcePageTaskResult, AcePageTaskError>>,
}

pub(super) fn start(
    scheduler: &mut CdpScheduler,
    target_id: String,
    task: Box<dyn AcePageTask>,
    reply: oneshot::Sender<Result<AcePageTaskResult, AcePageTaskError>>,
    completions: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight: &mut InFlightCommands,
    next_token: &mut u64,
) {
    let kind = task.kind();
    let (pending, permit, context) = scheduler.start_ace_page_task(&target_id, task);
    let token = take_next_in_flight_command_token(next_token);
    let wait_task = spawn_wait(token, 0, pending.clone(), completions);
    in_flight.insert(
        token,
        InFlightWork::AcePageTask(Box::new(InFlightAcePageTask {
            kind,
            turn: 0,
            wait_task,
            pending,
            context,
            permit,
            reply,
        })),
    );
}

fn spawn_wait(
    token: u64,
    turn: u64,
    pending: PendingAcePageTaskDispatch,
    completions: &mpsc::UnboundedSender<PendingCommandCompletion>,
) -> CommandWaitTask {
    spawn_command_wait(
        CommandWaitIdentity { token, turn },
        async move { CompletedActorWork::AcePageTask(pending.wait().await) },
        completions,
    )
}

pub(super) async fn complete(
    router: &CdpFrontendRouter,
    scheduler: &mut CdpScheduler,
    inputs: &mut SchedulerInputReceivers,
    deferred: &mut VecDeque<PendingRuntimeDeferredReplyState>,
    adapters: &mut ProtocolAdapterScheduler,
    completions: &mpsc::UnboundedSender<PendingCommandCompletion>,
    in_flight: &mut InFlightCommands,
    mut state: Box<InFlightAcePageTask>,
    completion: PendingCommandCompletion,
) -> bool {
    let completed = match completion.result {
        Ok(CompletedActorWork::AcePageTask(completed)) => completed,
        Ok(_) => unreachable!("typed page task completion"),
        Err(error) if state.turn == 0 => {
            // Keep the original future, context and final publication authority.
            // Cancellation of this waiter must not replay renderer work.
            tracing::warn!(%error, "rejoining admitted page task");
            state.turn = 1;
            state.wait_task = spawn_wait(
                completion.identity.token,
                state.turn,
                state.pending.clone(),
                completions,
            );
            in_flight.insert(completion.identity.token, InFlightWork::AcePageTask(state));
            return true;
        }
        Err(error) => {
            let _ = state.reply.send(Err(AcePageTaskError::new(500, error)));
            return false;
        }
    };
    let (result, output) = scheduler
        .state
        .with_state(|owner| owner.complete_ace_page_task(completed, &mut state.context))
        .expect("live protocol actor state");
    let CommandTaskStep::Complete(output) = scheduler
        .materialize_command_dispatch_step(moli_protocol::CdpCommandTaskStep::Complete(output))
    else {
        unreachable!("completed page task only publishes retained output");
    };
    let output = output
        .with_renderer_output_predecessor(state.context.take_renderer_output_predecessor())
        .with_output_release_permit(state.permit);
    if !flush_completed_command_output(
        router,
        scheduler,
        inputs,
        deferred,
        adapters,
        &owner_output_metadata(),
        output,
    )
    .await
    {
        return false;
    }
    let _ = state.reply.send(result);
    true
}

impl CdpScheduler {
    fn start_ace_page_task(
        &mut self,
        target_id: &str,
        task: Box<dyn moli_core::ace_page_task::AcePageTask>,
    ) -> (
        PendingAcePageTaskDispatch,
        CommandOutputReleasePermit,
        CommandDispatchContext,
    ) {
        let (permit, flush) = self
            .state
            .with_state(|state| state.begin_command_response_flush_permit())
            .expect("live protocol actor state");
        let mut context = CommandDispatchContext::new(flush);
        let (pending, events) = self
            .state
            .with_state(|state| state.start_ace_page_task(target_id, task, &mut context))
            .expect("live protocol actor state");
        self.apply_scheduler_events(events);
        (
            pending,
            CommandOutputReleasePermit::new(permit, None),
            context,
        )
    }
}
