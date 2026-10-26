use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use futures_util::{FutureExt as _, future::LocalBoxFuture};

use super::*;
use crate::conn::{CompletedPageTargetClose, PreparedTargetHostClosure, ProtocolWorkflowDriver};
use crate::domains::session::{CompletedClosedTargetSessions, PreparedClosedTargetSessions};
use crate::domains::target::activation::{PreparedTargetActivation, TargetActivationOutput};

/// Retains the original native receipts when its supervised wait is cancelled.
/// Clones share one final commit authority, not another task or target lookup.
#[derive(Clone)]
pub(crate) struct TargetTermination {
    native: futures_util::future::Shared<LocalBoxFuture<'static, NativeTargetTermination>>,
    selection: TargetTerminationSelection,
    activity: Rc<RefCell<Option<TargetTerminationActivity>>>,
    commit: Rc<RefCell<Option<TargetTerminationCommit>>>,
}

type TargetTerminationActivity =
    futures_util::future::Shared<LocalBoxFuture<'static, Option<TargetActivationOutput>>>;

#[derive(Clone)]
struct NativeTargetTermination {
    page: CompletedPageTargetClose,
    page_sessions: CompletedClosedTargetSessions,
    tab_sessions: Option<CompletedClosedTargetSessions>,
}

#[derive(Clone)]
pub(crate) struct TargetTerminationSelection {
    browser_context_id: String,
    next_target_id: Option<String>,
}

struct TargetTerminationCommit {
    target_id: String,
    owner: CommandOwnerScope,
    host: crate::conn::PageTargetHostIdentity,
    tab_target_id: Option<String>,
    closure: PreparedTargetHostClosure,
    events: Vec<BackgroundProtocolEvent>,
}

pub(crate) struct CompletedTargetTermination {
    termination: TargetTermination,
    native: NativeTargetTermination,
    activation: Option<TargetActivationOutput>,
}

impl TargetTermination {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        action: PageTargetTerminationOwnerAction,
    ) -> Option<Self> {
        let (owner, target_id, host) = action.into_parts();
        let host = host?;
        let (browser_context_id, current_target_id) =
            conn.target_owner_identity_for_owner(&owner)?;
        if current_target_id.as_deref() != Some(&target_id)
            || conn
                .page_target_host_identity_for_owner(&owner)
                .is_none_or(|current| !host.matches(&current))
        {
            return None;
        }
        let context = conn.browser_context_by_id(&browser_context_id)?;
        let next_target_id = context
            .is_active_target(&target_id)
            .then(|| context.page_targets.adjacent_target_id(&target_id))
            .flatten()
            .map(str::to_owned);
        let closure = conn.prepare_target_host_closure(&target_id);
        let tab_plan = conn.prepare_closed_top_level_target_sessions_cleanup_plan(
            &target_id,
            Some("Render process gone."),
        );
        let tab_target_id = tab_plan.as_ref().map(|plan| plan.target_id().to_owned());
        let mut events = Vec::new();
        let page = conn.start_page_target_close(&target_id, &mut events, "Target closed")?;
        let page_sessions = PreparedClosedTargetSessions::prepare(
            conn,
            page.session_cleanup_plan(Some("Render process gone.")),
            None,
        );
        let tab_sessions =
            tab_plan.map(|plan| PreparedClosedTargetSessions::prepare(conn, plan, None));
        let native = async move {
            NativeTargetTermination {
                page: page.wait().await,
                page_sessions: page_sessions.wait().await,
                tab_sessions: match tab_sessions {
                    Some(pending) => Some(pending.wait().await),
                    None => None,
                },
            }
        }
        .boxed_local()
        .shared();
        Some(Self {
            native,
            selection: TargetTerminationSelection {
                browser_context_id,
                next_target_id,
            },
            activity: Rc::default(),
            commit: Rc::new(RefCell::new(Some(TargetTerminationCommit {
                target_id,
                owner,
                host,
                tab_target_id,
                closure,
                events,
            }))),
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> CompletedTargetTermination {
        let native = self.native.clone().await;
        let activation = if let Some(access) = driver.owned_access() {
            let activity = {
                let mut activity = self.activity.borrow_mut();
                activity.get_or_insert_with(|| {
                    let selection = self.selection.clone();
                    async move {
                        let mut driver = ProtocolWorkflowDriver::owned(access);
                        match Self::restore_activity(selection, &mut driver).await {
                            Ok(output) => output,
                            Err(error) => {
                                tracing::warn!(%error, "failed to restore document activity after target close");
                                None
                            }
                        }
                    }.boxed_local().shared()
                }).clone()
            };
            activity.await
        } else {
            Self::restore_activity(self.selection.clone(), driver)
                .await
                .expect("inline activity restoration retains its protocol state")
        };
        CompletedTargetTermination {
            termination: self,
            native,
            activation,
        }
    }

    async fn restore_activity(
        selection: TargetTerminationSelection,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> Result<Option<TargetActivationOutput>> {
        let activation = driver
            .with_state(move |state| state.prepare_target_termination_activation(selection))
            .await?;
        let activation = match activation {
            Some(activation) => match activation.run(driver).await {
                Ok(completed) => Some(completed),
                Err(error) => {
                    tracing::warn!(%error, "failed to activate a neighboring target after close");
                    None
                }
            },
            None => None,
        };
        let activity = driver
            .with_state(|state| state.start_restore_browser_focus_after_removal())
            .await?;
        let activity = match activity {
            Ok(updates) => {
                crate::domains::target::activation::wait_for_surface_updates(driver, updates).await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = activity {
            tracing::warn!(%error, "failed to update document activity after target removal");
        }
        Ok(activation)
    }
}

impl TargetTerminationSelection {
    pub(crate) fn prepare_activation(
        &self,
        conn: &mut CdpConnection,
    ) -> Option<PreparedTargetActivation> {
        let context = conn.browser_context_by_id(&self.browser_context_id)?;
        // A peer may already have selected another Page during native close.
        // Only fill the empty selection left by this target's removal.
        let next = context
            .active_target_id()
            .is_none()
            .then(|| {
                self.next_target_id
                    .as_deref()
                    .filter(|next| context.page_target(next).is_some())
                    .or_else(|| {
                        context
                            .page_targets
                            .iter()
                            .next_back()
                            .map(|host| host.target_id())
                    })
                    .map(str::to_owned)
            })
            .flatten();
        let next = next?;
        match crate::domains::target::activation::prepare_target_activation_after_removal(
            conn, &next,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(?error, "failed to select a neighboring target after close");
                None
            }
        }
    }
}

impl CompletedTargetTermination {
    pub(crate) fn finish(self, conn: &mut CdpConnection) -> crate::conn::CdpTurnOutcome {
        let Some(commit) = self.termination.commit.borrow_mut().take() else {
            return crate::conn::CdpTurnOutcome::new_with_protocol_events(Vec::new(), Vec::new());
        };
        self.native.page.finish(conn);
        let successor_exists = conn
            .page_target_host_identity_for_owner(&commit.owner)
            .is_some_and(|current| !commit.host.matches(&current));
        let mut events = commit.events;
        if let Some(activation) = self.activation {
            events.extend(activation.finish(conn));
        }
        for sid in self.native.page_sessions.open_session_ids(conn) {
            events.push(BackgroundProtocolEvent::inspector_detached(
                Some(sid),
                "Render process gone.",
            ));
        }
        let (detached, destroyed) = commit.closure.into_parts();
        if !successor_exists {
            events.extend(conn.prepared_target_host_deltas_event_plan(detached));
        }
        events.extend(self.native.page_sessions.finish(conn));
        if !successor_exists
            && conn.tab_target_id_for_page_target_id(&commit.target_id)
                == commit.tab_target_id.as_deref()
        {
            conn.remove_tab_for_page_target(&commit.target_id);
        }
        if let Some(tab_sessions) = self.native.tab_sessions {
            events.extend(tab_sessions.finish(conn));
        }
        if !successor_exists {
            events.extend(conn.prepared_target_host_deltas_event_plan(destroyed));
        }
        conn.release_idle_navigation_engine_memory_after_target_close();
        crate::conn::CdpTurnOutcome::new_with_protocol_events(events, conn.take_scheduler_events())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, CompletedProtocolSchedulerWork,
        ParsedCdpCommand, ProtocolStateAccess, ProtocolStateOwner,
    };
    use crate::testing::TestContext;
    use serde_json::json;
    use std::cell::Cell;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn cancellation_rejoins_the_original_close_and_activity_receipts() {
        for hold_activity in [false, true] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-cancel-close".into());
            browser.set_active_target_id("TID-cancel-close");
            browser.attach_active_session("SID-cancel-close");
            browser.stage_background_target(
                "TID-cancel-next".into(),
                Some("SID-cancel-next".into()),
                "about:blank".into(),
                None,
                None,
            );
            context
                .conn
                .install_browser_context_fixture_for_test(browser);
            for session in ["SID-cancel-close", "SID-cancel-next"] {
                context
                    .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                    .await;
            }
            context.process_async(json!({"id":1,"sessionId":"SID-cancel-next","method":"Page.startScreencast","params":{}})).await;
            assert!(context.take_response_by_id(1).get("result").is_some());
            let action = PageTargetTerminationOwnerAction::new(
                &context.conn,
                CommandOwnerScope::for_session("SID-cancel-close"),
                "TID-cancel-close".into(),
            );
            let mut termination = TargetTermination::prepare(&mut context.conn, action).unwrap();
            let owner = ProtocolStateOwner::new(context.conn);
            let handle = owner.handle();
            let access = ProtocolStateAccess::Direct(handle.clone());
            let (started, entered) = oneshot::channel();
            let (release, held) = oneshot::channel();
            let runs = Rc::new(Cell::new(0));
            let actual_runs = runs.clone();
            if hold_activity {
                let selection = termination.selection.clone();
                let access = access.clone();
                *termination.activity.borrow_mut() = Some(
                    async move {
                        actual_runs.set(actual_runs.get() + 1);
                        let output = TargetTermination::restore_activity(
                            selection,
                            &mut ProtocolWorkflowDriver::owned(access),
                        )
                        .await
                        .unwrap();
                        started.send(()).unwrap();
                        held.await.unwrap();
                        output
                    }
                    .boxed_local()
                    .shared(),
                );
            } else {
                let native = termination.native.clone();
                termination.native = async move {
                    actual_runs.set(actual_runs.get() + 1);
                    let output = native.await;
                    started.send(()).unwrap();
                    held.await.unwrap();
                    output
                }
                .boxed_local()
                .shared();
            }
            let original = termination.clone();
            let original_access = access.clone();
            let mut wait = Box::pin(async move {
                original
                    .run(&mut ProtocolWorkflowDriver::owned(original_access))
                    .await
            });
            tokio::select! {
                _ = &mut wait => panic!("the retained receipt is held"),
                result = entered => result.unwrap(),
            }
            let version =
                ParsedCdpCommand::parse_str(r#"{"id":2,"method":"Browser.getVersion"}"#).unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default(),
                    ))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            drop(wait);
            release.send(()).unwrap();
            let completed = termination
                .clone()
                .run(&mut ProtocolWorkflowDriver::owned(access.clone()))
                .await;
            let messages = handle
                .with_state(|state| {
                    state.complete_owned_protocol_scheduler_work(CompletedProtocolSchedulerWork {
                        termination: Box::new(completed),
                    })
                })
                .unwrap()
                .into_parts()
                .0;
            assert_eq!(
                runs.get(),
                1,
                "cancellation must reuse its admitted native work"
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Target.detachedFromTarget")
                    .count(),
                1
            );
            assert!(
                messages.iter().any(|message| message["method"]
                    == "Page.screencastVisibilityChanged"
                    && message["sessionId"] == "SID-cancel-next"
                    && message["params"]["visible"] == true),
                "{messages:?}"
            );
            let repeated = termination
                .run(&mut ProtocolWorkflowDriver::owned(access))
                .await;
            assert!(
                handle
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(
                        CompletedProtocolSchedulerWork {
                            termination: Box::new(repeated)
                        },
                    ))
                    .unwrap()
                    .into_parts()
                    .0
                    .is_empty()
            );
        }
    }
}
