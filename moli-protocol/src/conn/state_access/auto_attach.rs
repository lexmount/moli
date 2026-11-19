use super::ProtocolWorkflowState;
use crate::conn::AutoAttachPolicyIdentity;
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::target::{AutoAttachSweepRollback, PreparedAutoAttachSweep};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn prepare_auto_attach_sweep(
        self,
        policy: AutoAttachPolicyIdentity,
        owner: Option<String>,
        rollback: AutoAttachSweepRollback,
    ) -> PreparedAutoAttachSweep {
        PreparedAutoAttachSweep::prepare(self.connection, policy, owner, rollback)
    }

    pub(crate) fn defer_auto_attach_activation(
        self,
        activation: crate::domains::target::activation::PreparedTargetActivation,
    ) {
        let access = self.connection.protocol_state_access();
        let workflow = crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            match activation
                .run(&mut crate::conn::ProtocolWorkflowDriver::new(access))
                .await
            {
                Ok(output) => context.defer_owner_turn_completion(move |state, _| {
                    let mut plan = CommandOutputPlan::default();
                    plan.extend_background_events(state.finish_auto_attach_activation(output));
                    plan
                }),
                Err(error) => {
                    tracing::warn!(%error, "auto-attach foreground synchronization retired")
                }
            }
            context.complete(CommandOutputPlan::default())
        });
        crate::conn::PendingProtocolSchedulerWork::publish_workflow(
            self.connection,
            workflow,
            None,
        );
    }

    pub(crate) fn finish_auto_attach_activation(
        self,
        output: crate::domains::target::activation::TargetActivationOutput,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        output.finish(self.connection)
    }

    pub(crate) fn commit_auto_attach_sweep(
        self,
        sweep: PreparedAutoAttachSweep,
    ) -> CommandOutputPlan {
        sweep.finish(self.connection)
    }
}

#[cfg(test)]
mod tests;
