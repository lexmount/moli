use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};
use crate::domains::activity::{ProtocolSchedulerWork, ProtocolSchedulerWorkKind};
use crate::domains::page::{CompletedTargetTermination, TargetTermination};
use crate::domains::target::activation::TargetActivationOutput;

/// Owned protocol work submitted to the adapter's existing task supervisor.
/// Clones retain the same native waits and final publication authority.
#[derive(Clone)]
pub struct PendingProtocolSchedulerWork {
    work: PendingProtocolWork,
    access: ProtocolStateAccess,
}

#[derive(Clone)]
enum PendingProtocolWork {
    Termination(TargetTermination),
    PopupActivation(PendingPopupActivation),
}

#[derive(Clone)]
struct PendingPopupActivation {
    pending: Shared<LocalBoxFuture<'static, Option<TargetActivationOutput>>>,
    action: Rc<crate::conn::PopupTargetActivationAction>,
    committed: Rc<Cell<bool>>,
}

impl PendingPopupActivation {
    fn prepare(
        conn: &mut CdpConnection,
        action: crate::conn::PopupTargetActivationAction,
        access: ProtocolStateAccess,
    ) -> Option<Self> {
        let prepared = match crate::domains::target::prepare_popup_target_activation(conn, &action)
        {
            Ok(prepared) => prepared?,
            Err(error) => {
                tracing::debug!(?error, "popup target could not be activated");
                return None;
            }
        };
        let pending = async move {
            let mut driver = ProtocolWorkflowDriver::owned(access);
            match prepared.run(&mut driver).await {
                Ok(output) => Some(output),
                Err(error) => {
                    tracing::debug!(%error, "popup target could not be activated");
                    None
                }
            }
        }
        .boxed_local()
        .shared();
        Some(Self {
            pending,
            action: Rc::new(action),
            committed: Rc::new(Cell::new(false)),
        })
    }
}

pub struct CompletedProtocolSchedulerWork {
    work: CompletedProtocolWork,
}

enum CompletedProtocolWork {
    Termination(Box<CompletedTargetTermination>),
    PopupActivation(PendingPopupActivation, Option<TargetActivationOutput>),
}

impl CompletedProtocolSchedulerWork {
    pub(crate) fn for_termination(completed: CompletedTargetTermination) -> Self {
        Self {
            work: CompletedProtocolWork::Termination(Box::new(completed)),
        }
    }
}

impl PendingProtocolSchedulerWork {
    /// The adapter and finite admission share this migration boundary.
    pub fn handles(kind: ProtocolSchedulerWorkKind) -> bool {
        matches!(
            kind,
            ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction
                | ProtocolSchedulerWorkKind::PopupTargetActivationAction
        )
    }

    pub async fn wait(self) -> CompletedProtocolSchedulerWork {
        match self.work {
            PendingProtocolWork::Termination(termination) => {
                let completed = termination
                    .run(&mut ProtocolWorkflowDriver::owned(self.access))
                    .await;
                CompletedProtocolSchedulerWork::for_termination(completed)
            }
            PendingProtocolWork::PopupActivation(activation) => {
                let output = activation.pending.clone().await;
                CompletedProtocolSchedulerWork {
                    work: CompletedProtocolWork::PopupActivation(activation, output),
                }
            }
        }
    }
}

impl CdpConnection {
    /// Unsupported work stays with its existing driver during migration.
    pub fn try_start_owned_protocol_scheduler_work(
        &mut self,
        work: ProtocolSchedulerWork,
    ) -> Result<Option<PendingProtocolSchedulerWork>, Box<ProtocolSchedulerWork>> {
        let Some(access) = self.protocol_state_access() else {
            return Err(Box::new(work));
        };
        if !PendingProtocolSchedulerWork::handles(work.kind()) {
            return Err(Box::new(work));
        }
        let prepared = match work.into_ready() {
            ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action) => {
                TargetTermination::prepare(self, action).map(PendingProtocolWork::Termination)
            }
            ReadyProtocolSchedulerWork::PopupTargetActivationAction(action) => {
                PendingPopupActivation::prepare(self, action, access.clone())
                    .map(PendingProtocolWork::PopupActivation)
            }
            _ => unreachable!("the work kind identifies its owned payload"),
        };
        Ok(prepared.map(|work| PendingProtocolSchedulerWork { work, access }))
    }

    pub fn complete_owned_protocol_scheduler_work(
        &mut self,
        completed: CompletedProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        match completed.work {
            CompletedProtocolWork::Termination(termination) => termination.finish(self),
            CompletedProtocolWork::PopupActivation(activation, output) => {
                let events =
                    if !activation.committed.replace(true) && activation.action.is_current(self) {
                        output.map(|output| output.finish(self)).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                CdpTurnOutcome::new_with_protocol_events(events, self.take_scheduler_events())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        PopupTargetActivationAction, ProtocolStateOwner,
    };
    use crate::domains::activity::ProtocolWorkPublishSequence;
    use crate::testing::TestContext;
    use serde_json::json;
    use tokio::sync::oneshot;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-popup-owned".into());
        browser.set_active_target_id("TID-popup-first");
        browser.attach_active_session("SID-popup-first");
        browser.stage_background_target(
            "TID-popup-second".into(),
            Some("SID-popup-second".into()),
            "about:blank".into(),
            None,
            None,
        );
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        for session in ["SID-popup-first", "SID-popup-second"] {
            context
                .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                .await;
        }
        context
    }

    #[tokio::test]
    async fn popup_activation_rejoins_admitted_work_after_cancellation_and_ready_loss() {
        for hold_after_ready in [false, true] {
            let mut context = fixture().await;
            for (id, session) in [(1, "SID-popup-first"), (2, "SID-popup-second")] {
                context.process_async(json!({"id":id,"sessionId":session,"method":"Page.startScreencast","params":{}})).await;
                assert!(context.take_response_by_id(id).get("result").is_some());
            }
            let action = PopupTargetActivationAction::capture(
                &context.conn,
                "BID-popup-owned",
                "TID-popup-second",
            )
            .unwrap();
            drop(context.conn.take_protocol_state_requests());
            let owner = ProtocolStateOwner::new(context.conn);
            let handle = owner.handle();
            let work = ProtocolSchedulerWork::popup_target_activation_action(
                ProtocolWorkPublishSequence::new(1),
                action,
            );
            let mut pending = handle
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap_or_else(|_| {
                    panic!("popup activation must use the existing owned-work supervisor")
                })
                .expect("the live popup owns its surface update");
            let PendingProtocolWork::PopupActivation(activation) = &mut pending.work else {
                panic!("popup work keeps its activation kind");
            };
            let actual = activation.pending.clone();
            let (entered, ready) = oneshot::channel();
            let (release, held) = oneshot::channel();
            let runs = Rc::new(Cell::new(0));
            let actual_runs = runs.clone();
            activation.pending = async move {
                actual_runs.set(actual_runs.get() + 1);
                if hold_after_ready {
                    let output = actual.await;
                    entered.send(()).unwrap();
                    held.await.unwrap();
                    output
                } else {
                    entered.send(()).unwrap();
                    held.await.unwrap();
                    actual.await
                }
            }
            .boxed_local()
            .shared();
            let mut wait = Box::pin(pending.clone().wait());
            tokio::select! {
                _ = &mut wait => panic!("the controlled native handoff is still held"),
                result = ready => result.unwrap(),
            }
            let version =
                ParsedCdpCommand::parse_str(r#"{"id":50,"method":"Browser.getVersion"}"#).unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default()
                    ))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            drop(wait);
            release.send(()).unwrap();
            // A ready result can be discarded before the actor consumes it.
            // The supervised work still owns the same native result and events.
            drop(pending.clone().wait().await);
            let completed = pending.clone().wait().await;
            let messages = handle
                .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                .unwrap()
                .into_parts()
                .0;
            let visible: Vec<_> = messages
                .iter()
                .filter(|event| event["method"] == "Page.screencastVisibilityChanged")
                .map(|event| json!([event["sessionId"], event["params"]["visible"]]))
                .collect();
            assert_eq!(
                visible,
                vec![
                    json!(["SID-popup-first", false]),
                    json!(["SID-popup-second", true])
                ]
            );
            assert_eq!(runs.get(), 1, "recovery cannot repeat native admission");
            let repeated = pending.wait().await;
            assert!(
                handle
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(repeated))
                    .unwrap()
                    .into_parts()
                    .0
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn popup_activation_cannot_select_a_recreated_target_with_the_same_id() {
        let mut context = fixture().await;
        let action = PopupTargetActivationAction::capture(
            &context.conn,
            "BID-popup-owned",
            "TID-popup-second",
        )
        .unwrap();
        let old = context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .take_page_target_for_close("TID-popup-second")
            .unwrap();
        context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                "TID-popup-second".into(),
                Some("SID-popup-second".into()),
                "about:blank".into(),
            ));
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-popup-second"))
            .await;
        assert!(!action.is_current(&context.conn));
        let work = ProtocolSchedulerWork::popup_target_activation_action(
            ProtocolWorkPublishSequence::new(1),
            action,
        );
        drop(context.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(context.conn);
        assert!(
            owner
                .handle()
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap_or_else(|_| panic!("a stale popup action is consumed without legacy work"))
                .is_none()
        );
        drop(old);
    }
}
