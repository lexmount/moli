use std::{cell::RefCell, rc::Rc};

use anyhow::Result;

use super::*;
use crate::conn::{
    AutoAttachPolicyIdentity, CommandWorkflowContext, PreparedCommandWorkflow, ProtocolStateHandle,
    ProtocolWorkflowDriver,
};

/// The original command's uncommitted declarations. A waiter never becomes
/// their owner; its recovery keeps this same transaction until commit/rollback.
#[derive(Clone, Default)]
pub(crate) struct AutoAttachSweepRollback(Rc<RefCell<Vec<TargetAttachSessionCommit>>>);

impl AutoAttachSweepRollback {
    pub(super) fn record(&self, session: TargetAttachSessionCommit) {
        self.0.borrow_mut().push(session);
    }

    fn committed(&self, session_id: &str) {
        self.0
            .borrow_mut()
            .retain(|session| session.session_id() != session_id);
    }

    async fn run(self, driver: &mut ProtocolWorkflowDriver) {
        let sessions = std::mem::take(&mut *self.0.borrow_mut());
        for session in sessions {
            if let Err(error) = super::super::opening::rollback_attachment(driver, session).await {
                tracing::warn!(%error, "failed to roll back an uncommitted auto-attach session");
            }
        }
    }

    fn into_recovery(
        self,
        access: ProtocolStateHandle,
        response: CommandOutputPlan,
    ) -> PreparedCommandWorkflow {
        PreparedCommandWorkflow::new(move |context| async move {
            self.run(&mut ProtocolWorkflowDriver::new(access)).await;
            context.complete(response)
        })
    }
}

pub(crate) struct PreparedAutoAttachEnable {
    owner_session_id: Option<String>,
    policy: AutoAttachPolicyIdentity,
    rollback: AutoAttachSweepRollback,
}

pub(crate) struct AutoAttachSweepContext {
    pub(super) pages: Vec<TargetAttachSessionCommit>,
    pub(super) tabs: Vec<TargetAttachSessionCommit>,
    pub(super) workers: Vec<PreparedTargetAttach>,
    pub(super) activation: Option<super::super::activation::PreparedTargetActivation>,
}

pub(crate) struct PreparedAutoAttachSweep {
    policy: AutoAttachPolicyIdentity,
    contexts: Vec<AutoAttachSweepContext>,
    rollback: AutoAttachSweepRollback,
}

impl PreparedAutoAttachEnable {
    pub(in crate::domains::target) fn prepare(
        conn: &mut CdpConnection,
        owner_session_id: Option<&str>,
    ) -> Self {
        Self {
            policy: AutoAttachPolicyIdentity::capture(conn, owner_session_id)
                .expect("an enabling command retains its installed policy"),
            owner_session_id: owner_session_id.map(str::to_owned),
            rollback: AutoAttachSweepRollback::default(),
        }
    }

    pub(in crate::domains::target) fn into_workflow(
        self,
        access: ProtocolStateHandle,
    ) -> PreparedCommandWorkflow {
        let rollback = self.rollback.clone();
        let recovery_access = access.clone();
        PreparedCommandWorkflow::new(move |mut context| async move {
            match self.run(&mut ProtocolWorkflowDriver::new(access), &mut context) {
                Ok(()) => context.complete(CommandOutputPlan::default()),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .with_failure_cleanup(move |response| rollback.into_recovery(recovery_access, response))
    }

    fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        let policy = self.policy;
        let rollback = self.rollback;
        let prepared_rollback = rollback.clone();
        let mut sweep = driver.with_state(move |state| {
            state.prepare_auto_attach_sweep(policy, self.owner_session_id, prepared_rollback)
        })?;
        for batch in &mut sweep.contexts {
            if let Some(activation) = batch.activation.take() {
                driver.with_state(move |state| state.defer_auto_attach_activation(activation))?;
            }
        }
        {
            let access = driver.state_access();

            context.defer_owner_turn_completion(move |state, context| {
                let plan = state.commit_auto_attach_sweep(sweep);
                if !rollback.0.borrow().is_empty() {
                    context.register_command_failure_cleanup(
                        crate::conn::CommandFailureCleanup::Continuation(
                            crate::conn::CommandWorkflowFailureCleanup::new(move |response| {
                                rollback.into_recovery(access, response)
                            }),
                        ),
                    );
                }
                plan
            });
        }
        Ok(())
    }
}

impl PreparedAutoAttachSweep {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        policy: AutoAttachPolicyIdentity,
        owner: Option<String>,
        rollback: AutoAttachSweepRollback,
    ) -> Self {
        let contexts = if policy.is_current(conn) {
            let ids = conn
                .browser_contexts()
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            let tab = owner
                .as_deref()
                .and_then(|owner| conn.tab_target_id_for_session_id(owner))
                .map(str::to_owned);
            if let Some(tab) = tab {
                super::prepare_tab_child_sweep(conn, owner.as_deref().unwrap(), &tab, &rollback)
                    .into_iter()
                    .collect()
            } else {
                ids.into_iter()
                    .filter_map(|id| {
                        super::prepare_context_sweep(conn, &id, owner.as_deref(), &rollback)
                    })
                    .collect()
            }
        } else {
            Vec::new()
        };
        Self {
            contexts,
            policy,
            rollback,
        }
    }

    pub(crate) fn finish(self, conn: &mut CdpConnection) -> CommandOutputPlan {
        let mut plan = CommandOutputPlan::default();
        for context in self.contexts {
            for session in context.tabs.into_iter().chain(context.pages) {
                if !self.policy.is_current(conn)
                    || !super::prepared_session_can_commit(conn, &session)
                {
                    continue;
                }
                let Some(target_id) = session.route().target_id().map(str::to_owned) else {
                    continue;
                };
                let info = match session.route() {
                    CdpSessionRoute::TabTarget { .. } => conn.tab_target_info(&target_id),
                    _ => session
                        .route()
                        .browser_context_id()
                        .and_then(|id| conn.browser_context_by_id(id))
                        .and_then(|context| context.devtools_target_info(&target_id)),
                };
                let Some(info) = info else {
                    continue;
                };
                let committed = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    target_id,
                    info,
                    [session],
                ));
                for session in committed.committed_sessions() {
                    self.rollback.committed(session.session_id());
                }
                plan.extend_background_events(committed);
            }
            for prepared in context.workers {
                if !self.policy.is_current(conn)
                    || prepared
                        .sessions()
                        .iter()
                        .any(|session| !super::prepared_session_can_commit(conn, session))
                {
                    continue;
                }
                let committed = conn.commit_prepared_attach_event_plan(prepared);
                for session in committed.committed_sessions() {
                    self.rollback.committed(session.session_id());
                }
                plan.extend_background_events(committed);
            }
        }
        plan.push_success();
        plan
    }
}
