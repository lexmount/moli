use std::{cell::RefCell, rc::Rc};

use anyhow::Result;

use super::*;
use crate::conn::{
    AutoAttachPolicyIdentity, CommandWorkflowContext, InitialDocumentPageBuildWaiter,
    PreparedCommandWorkflow, ProtocolStateAccess, ProtocolWorkflowDriver,
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

    pub(in crate::domains::target) async fn run(self, driver: &mut ProtocolWorkflowDriver<'_>) {
        let sessions = std::mem::take(&mut *self.0.borrow_mut());
        for session in sessions {
            if let Err(error) = super::super::opening::rollback_attachment(driver, session).await {
                tracing::warn!(%error, "failed to roll back an uncommitted auto-attach session");
            }
        }
    }

    fn into_recovery(
        self,
        access: ProtocolStateAccess,
        response: CommandOutputPlan,
    ) -> PreparedCommandWorkflow {
        PreparedCommandWorkflow::new(move |context| async move {
            self.run(&mut ProtocolWorkflowDriver::owned(access)).await;
            context.complete(response)
        })
    }
}

pub(crate) struct PreparedAutoAttachCommand {
    owner_session_id: Option<String>,
    policy: Option<AutoAttachPolicyIdentity>,
    initial_document: Option<InitialDocumentPageBuildWaiter>,
    detach: Option<super::super::attachment::TargetDetachment>,
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

impl PreparedAutoAttachCommand {
    pub(in crate::domains::target) fn prepare(
        conn: &mut CdpConnection,
        enabled: bool,
        owner_session_id: Option<&str>,
        initial_document: Option<InitialDocumentPageBuildWaiter>,
    ) -> Self {
        Self {
            policy: enabled.then(|| {
                AutoAttachPolicyIdentity::capture(conn, owner_session_id)
                    .expect("an enabled auto-attach command retains its installed policy")
            }),
            detach: (!enabled).then(|| {
                super::super::attachment::TargetDetachment::prepare_auto_attached_sessions(
                    conn,
                    owner_session_id,
                )
            }),
            owner_session_id: owner_session_id.map(str::to_owned),
            initial_document,
            rollback: AutoAttachSweepRollback::default(),
        }
    }

    pub(in crate::domains::target) async fn wait_initial_document(&mut self) {
        if let Some(wait) = self.initial_document.take()
            && let Err(error) = wait.wait().await
        {
            tracing::debug!(%error, "auto-attach initial Page dependency retired");
        }
    }

    pub(in crate::domains::target) fn into_workflow(
        self,
        access: ProtocolStateAccess,
    ) -> PreparedCommandWorkflow {
        let detached_cleanup = self.detach.as_ref().map(|detach| detach.failure_cleanup());
        let rollback = self.rollback.clone();
        let recovery_access = access.clone();
        PreparedCommandWorkflow::new(move |mut context| async move {
            match Box::pin(self.run(&mut ProtocolWorkflowDriver::owned(access), &mut context)).await
            {
                Ok(()) => context.complete(CommandOutputPlan::default()),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .with_failure_cleanup(move |response| {
            let Some(cleanup) = detached_cleanup else {
                return rollback.into_recovery(recovery_access, response);
            };
            PreparedCommandWorkflow::new(move |mut context| async move {
                let mut driver = ProtocolWorkflowDriver::owned(recovery_access);
                let recovery = context
                    .with_state(&mut driver, move |state, context| {
                        state.prepare_auto_attach_detachment_recovery(
                            cleanup,
                            response,
                            context.context_mut(),
                        )
                    })
                    .await;
                match recovery {
                    Ok(recovery) => {
                        let recovery = recovery.wait().await;
                        context.complete_in_owner_turn(
                            CommandOutputPlan::default(),
                            move |state, _| state.finish_auto_attach_detachment_recovery(recovery),
                        )
                    }
                    Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
                }
            })
        })
    }

    pub(in crate::domains::target) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        self.wait_initial_document().await;
        if let Some(detach) = self.detach {
            let completed = detach.run(driver).await;
            context.defer_owner_turn_completion(move |state, context| {
                state.finish_auto_attach_detachment(completed, context)
            });
            return Ok(());
        }
        let policy = self
            .policy
            .expect("an enabling command retains its policy identity");
        let rollback = self.rollback;
        let prepared_rollback = rollback.clone();
        let mut sweep = driver
            .with_state(move |state| {
                state.prepare_auto_attach_sweep(policy, self.owner_session_id, prepared_rollback)
            })
            .await?;
        for batch in &mut sweep.contexts {
            if let Some(activation) = batch.activation.take() {
                match activation.run(driver).await {
                    Ok(output) => {
                        context
                            .with_state(driver, move |state, context| {
                                let mut plan = CommandOutputPlan::default();
                                plan.extend_background_events(
                                    state.finish_auto_attach_activation(output),
                                );
                                context.retain_output(plan);
                            })
                            .await?;
                    }
                    Err(error) => tracing::warn!(%error, "auto-attach foreground target retired"),
                }
            }
            for session in &batch.pages {
                let policy = sweep.policy.clone();
                let ensure = session.clone();
                match driver
                    .with_state(move |state| {
                        state.start_auto_attach_initial_document(policy, ensure)
                    })
                    .await?
                {
                    Ok(Some(pending)) => match pending.wait().await {
                        Ok(completed) => match completed.install_using(driver).await {
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(%error, "failed to install auto-attach initial Document")
                            }
                        },
                        Err(failed) => {
                            driver
                                .with_state(move |state| {
                                    state.fail_auto_attach_initial_document(failed)
                                })
                                .await?;
                        }
                    },
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "failed to ensure auto-attach initial Document")
                    }
                }
            }
            for session in &batch.pages {
                let policy = sweep.policy.clone();
                let bind = session.clone();
                let binding = driver
                    .with_state(move |state| state.start_auto_attach_bindings(policy, bind))
                    .await?;
                let result = match binding {
                    Ok(Some(pending)) => match pending.wait().await {
                        Ok(completed) => {
                            driver
                                .with_state(move |state| {
                                    state.finish_auto_attach_bindings(completed)
                                })
                                .await?
                        }
                        Err(error) => Err(error),
                    },
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                };
                if let Err(error) = result
                    && !error.is::<crate::conn::NoDocumentLoaded>()
                {
                    tracing::warn!(%error, session_id = session.session_id(), "failed to apply auto-attach binding state");
                }
            }
        }
        if let Some(access) = driver.owned_access() {
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
        } else {
            context
                .with_state(driver, move |state, context| {
                    context.retain_output(state.commit_auto_attach_sweep(sweep));
                })
                .await?;
            rollback.run(driver).await;
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
                if super::super::transient_no_page_devtools_target_info_error(conn, &info).is_some()
                {
                    continue;
                }
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
