use anyhow::Result;

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use anyhow::Context as _;

use super::{CdpConnection, ProtocolWorkflowState};

/// The single strong owner of protocol state accessed synchronously.
///
/// Construction consumes the connection. Legacy entry points cannot continue
/// borrowing that connection, and tasks receive only revocable weak handles.
/// Owned workflows admitted through this owner use the same synchronous
/// access. The legacy actor retains separate ownership until its remaining
/// asynchronous connection borrows have migrated.
pub struct ProtocolStateOwner {
    connection: Rc<RefCell<CdpConnection>>,
}

impl ProtocolStateOwner {
    pub fn new(connection: CdpConnection) -> Self {
        let owner = Self {
            connection: Rc::new(RefCell::new(connection)),
        };
        owner.connection.borrow_mut().state_mailbox.direct = Some(owner.handle());
        owner
    }

    pub fn handle(&self) -> ProtocolStateHandle {
        ProtocolStateHandle {
            connection: Rc::downgrade(&self.connection),
        }
    }
}

/// Revocable, thread-local access to finite synchronous state operations.
/// Keeping a handle across an await does not keep its owner or connection alive.
#[derive(Clone)]
pub struct ProtocolStateHandle {
    connection: Weak<RefCell<CdpConnection>>,
}

impl ProtocolStateHandle {
    /// Runs a finite operation immediately and releases the connection borrow
    /// before returning owned work. Reentrant access returns an error.
    ///
    /// ```
    /// use moli_protocol::{CdpConnection, ProtocolStateOwner};
    ///
    /// let owner = ProtocolStateOwner::new(CdpConnection::new());
    /// let handle = owner.handle();
    /// handle.with_state(|_| ()).unwrap();
    /// drop(owner);
    /// assert!(handle.with_state(|_| ()).is_err());
    /// ```
    ///
    /// The state capability cannot escape as the result:
    ///
    /// ```compile_fail
    /// use moli_protocol::{CdpConnection, ProtocolStateOwner};
    /// let owner = ProtocolStateOwner::new(CdpConnection::new());
    /// let escaped = owner.handle().with_state(|state| state).unwrap();
    /// ```
    ///
    /// A future retaining state cannot escape either:
    ///
    /// ```compile_fail
    /// use moli_protocol::{CdpConnection, ProtocolStateOwner};
    /// let owner = ProtocolStateOwner::new(CdpConnection::new());
    /// let escaped = owner.handle().with_state(|state| async move {
    ///     std::hint::black_box(state);
    /// }).unwrap();
    /// ```
    ///
    /// Capturing an outer slot cannot extend the state borrow:
    ///
    /// ```compile_fail
    /// use moli_protocol::{CdpConnection, ProtocolStateOwner};
    /// let owner = ProtocolStateOwner::new(CdpConnection::new());
    /// let mut escaped = None;
    /// owner.handle().with_state(|state| escaped = Some(state)).unwrap();
    /// ```
    ///
    /// The underlying connection and its guard are private:
    ///
    /// ```compile_fail
    /// use moli_protocol::{CdpConnection, ProtocolStateOwner};
    /// let owner = ProtocolStateOwner::new(CdpConnection::new());
    /// let connection = owner.handle().with_state(|state| state.connection).unwrap();
    /// ```
    pub fn with_state<T: 'static>(
        &self,
        operation: impl for<'state> FnOnce(ProtocolWorkflowState<'state>) -> T,
    ) -> Result<T> {
        let connection = self.connection.upgrade().context("protocol owner closed")?;
        let mut connection = connection
            .try_borrow_mut()
            .context("protocol state access is already active")?;
        Ok(operation(ProtocolWorkflowState {
            connection: &mut connection,
        }))
    }
}

#[cfg(test)]
mod termination_tests;

#[cfg(test)]
mod native_tests;

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::*;

    #[test]
    fn handles_cannot_extend_the_connection_owners_lifetime() {
        let owner = ProtocolStateOwner::new(CdpConnection::new());
        let first = owner.handle();
        let second = first.clone();
        assert_eq!(first.connection.strong_count(), 1);
        drop(owner);
        for handle in [first, second] {
            let mut ran = false;
            assert!(handle.with_state(|_| ran = true).is_err());
            assert!(!ran);
        }
    }

    #[test]
    fn reentrant_access_fails_without_revoking_later_operations() {
        let owner = ProtocolStateOwner::new(CdpConnection::new());
        let handle = owner.handle();
        let error = handle
            .with_state(|_| handle.with_state(|_| ()))
            .unwrap()
            .unwrap_err();
        assert!(error.is::<std::cell::BorrowMutError>());
        handle.with_state(|_| ()).unwrap();
    }

    #[tokio::test]
    async fn closing_session_waiters_end_when_the_owner_exits_even_with_retained_snapshots() {
        let mut connection = CdpConnection::new();
        connection.register_browser_session("SID-owner-exit".into());
        connection
            .target_control
            .mark_session_closing("SID-owner-exit");
        let waiter = connection.session_disposal_wait("SID-owner-exit").unwrap();
        let snapshot = connection.target_control.clone();
        let owner = ProtocolStateOwner::new(connection);
        drop(owner);
        tokio::time::timeout(std::time::Duration::from_secs(30), waiter.wait())
            .await
            .expect("snapshots cannot keep an exited protocol owner's close wait alive");
        drop(snapshot);
    }

    #[test]
    fn unwinding_releases_the_synchronous_state_borrow() {
        let owner = ProtocolStateOwner::new(CdpConnection::new());
        let handle = owner.handle();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                handle.with_state::<()>(|_| panic!("controlled state operation panic"))
            }))
            .is_err()
        );
        handle.with_state(|_| ()).unwrap();
    }

    #[tokio::test]
    async fn native_target_close_and_session_disposal_use_direct_state() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        };
        use crate::testing::TestContext;

        let mut context = TestContext::new();
        let mut retiring = BrowserContext::new("BID-native-close".into());
        retiring.set_active_target_id("TID-native-close");
        retiring.attach_active_session("SID-native-close");
        assert!(
            retiring.assign_attached_session_to_target(
                "TID-native-close",
                "SID-native-close-peer".into(),
            )
        );
        context
            .conn
            .install_browser_context_fixture_for_test(retiring);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-native-close"))
            .await;
        let mut routing = BrowserContext::new("BID-native-routing".into());
        routing.set_active_target_id("TID-native-routing");
        routing.attach_active_session("SID-native-routing");
        context
            .conn
            .push_inactive_browser_context_fixture_for_test(routing);
        assert!(
            context
                .conn
                .activate_browser_context_by_id("BID-native-routing")
        );

        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let mut events = Vec::new();
        let pending = handle
            .with_state(|state| {
                state.start_page_target_close("TID-native-close", &mut events, "Target closed")
            })
            .unwrap()
            .unwrap();
        handle
            .with_state(|state| {
                let conn = state.connection;
                assert_eq!(
                    conn.browser_context.as_ref().unwrap().id,
                    "BID-native-routing"
                );
                assert!(
                    conn.browser_context_by_id("BID-native-close")
                        .unwrap()
                        .page_target("TID-native-close")
                        .is_none()
                );
                for session in ["SID-native-close", "SID-native-close-peer"] {
                    assert!(conn.has_attached_devtools_session(session));
                    assert!(conn.session_is_closing(session));
                }
                assert!(conn.state_mailbox.receiver.is_some());
            })
            .unwrap();
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":10,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default(),
                    )
                })
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));

        let completed = pending.wait().await;
        let plan = handle
            .with_state(|state| {
                let closed = state.finish_page_target_close(completed);
                assert_eq!(closed.target_id, "TID-native-close");
                closed.into_detach_cleanup_plan(Some("Render process gone."))
            })
            .unwrap();
        handle
            .with_state(|state| {
                assert!(
                    state
                        .connection
                        .has_attached_devtools_session("SID-native-close")
                );
            })
            .unwrap();
        let pending = handle
            .with_state(|state| state.prepare_closed_target_sessions(plan, None))
            .unwrap();
        let completed = pending.wait().await;
        handle
            .with_state(|state| {
                for session in ["SID-native-close", "SID-native-close-peer"] {
                    assert!(
                        state.connection.has_attached_devtools_session(session),
                        "a ready handler result does not commit a binding on its own"
                    );
                }
            })
            .unwrap();
        let detached = handle
            .with_state(|state| state.finish_closed_target_sessions(completed))
            .unwrap();
        assert_eq!(detached.into_background_events().len(), 2);
        handle
            .with_state(|state| {
                for session in ["SID-native-close", "SID-native-close-peer"] {
                    assert!(!state.connection.has_attached_devtools_session(session));
                }
                assert_eq!(
                    state.connection.browser_context.as_ref().unwrap().id,
                    "BID-native-routing"
                );
            })
            .unwrap();
    }

    #[tokio::test]
    async fn target_activation_uses_direct_state_and_rejects_superseded_visibility() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CdpTargetHostLifecycleDelta,
            CdpTargetHostLifecycleObserver, CommandDispatchContext, ParsedCdpCommand,
        };
        use crate::testing::TestContext;
        use serde_json::json;

        let mut context = TestContext::new();
        let mut first = BrowserContext::new("BID-activation".into());
        first.set_active_target_id("TID-activation-first");
        first.attach_active_session("SID-activation-first");
        first.stage_background_target(
            "TID-activation-second".into(),
            Some("SID-activation-second".into()),
            "about:blank".into(),
            None,
            None,
        );
        context.conn.install_browser_context_fixture_for_test(first);
        let mut routing = BrowserContext::new("BID-routing".into());
        routing.set_active_target_id("TID-routing");
        routing.attach_active_session("SID-routing");
        context
            .conn
            .push_inactive_browser_context_fixture_for_test(routing);
        for session in [
            "SID-activation-first",
            "SID-activation-second",
            "SID-routing",
        ] {
            context
                .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                .await;
        }
        for (id, session) in [(1, "SID-activation-first"), (2, "SID-activation-second")] {
            context
                .process_async(json!({
                    "id": id, "sessionId": session, "method": "Page.startScreencast", "params": {}
                }))
                .await;
            assert!(context.take_response_by_id(id).get("result").is_some());
        }
        assert!(context.conn.activate_browser_context_by_id("BID-routing"));
        let (activated_tx, activated_rx) = std::sync::mpsc::channel();
        context
            .conn
            .set_target_host_lifecycle_observer(CdpTargetHostLifecycleObserver::new(
                move |delta| {
                    if let CdpTargetHostLifecycleDelta::Activated { target_id } = delta {
                        activated_tx.send(target_id).unwrap();
                    }
                },
            ));
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let mut commands = Vec::new();
        for (id, target) in [(10, "TID-activation-second"), (11, "TID-activation-first")] {
            let command = ParsedCdpCommand::parse_str(
                json!({
                    "id": id, "method": "Target.activateTarget", "params": {"targetId": target}
                })
                .to_string(),
            )
            .unwrap();
            let mut dispatch = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
                })
                .unwrap()
            else {
                panic!("surface work stays outside synchronous state access");
            };
            handle
                .with_state(|state| {
                    assert_eq!(
                        state.connection.browser_context.as_ref().unwrap().id,
                        "BID-routing"
                    );
                    assert!(state.connection.state_mailbox.receiver.is_some());
                })
                .unwrap();
            commands.push((id, pending, dispatch));
        }
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":12,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default(),
                    )
                })
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        assert!(activated_rx.try_recv().is_err());
        for (id, pending, mut dispatch) in commands {
            let completed = pending.wait().await;
            let CdpCommandTaskStep::Complete(outcome) = handle
                .with_state(|state| {
                    state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("activation has no legacy completion wait"))
            else {
                panic!("all native waits finished in the command task");
            };
            let messages = outcome.into_parts().0;
            let visibility: Vec<_> = messages
                .iter()
                .filter(|message| message["method"] == "Page.screencastVisibilityChanged")
                .map(|message| json!([message["sessionId"], message["params"]["visible"]]))
                .collect();
            assert_eq!(
                visibility,
                if id == 10 {
                    Vec::new()
                } else {
                    vec![
                        json!(["SID-activation-second", false]),
                        json!(["SID-activation-first", true]),
                    ]
                }
            );
            assert!(
                messages
                    .iter()
                    .any(|message| message["id"] == id && message.get("result").is_some()),
                "{messages:?}"
            );
            if id == 10 {
                assert!(activated_rx.try_recv().is_err());
            }
        }
        assert_eq!(activated_rx.try_recv().unwrap(), "TID-activation-first");
        assert!(activated_rx.try_recv().is_err());

        // A terminal can already be queued when the actor admits another
        // selection. Validate visibility when consuming that terminal too.
        let start_activation = |id, target| {
            let command = ParsedCdpCommand::parse_str(
                json!({"id": id, "method": "Target.activateTarget", "params": {"targetId": target}})
                    .to_string(),
            )
            .unwrap();
            let mut dispatch = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
                })
                .unwrap()
            else {
                panic!("activation owns its renderer waits");
            };
            (pending, dispatch)
        };
        let (old, mut old_context) = start_activation(20, "TID-activation-second");
        let queued = old.wait().await;
        let (current, mut current_context) = start_activation(21, "TID-activation-first");
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(queued, &mut old_context)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("activation completion is finite"))
        else {
            panic!("activation has completed");
        };
        assert!(
            outcome
                .into_parts()
                .0
                .iter()
                .all(|message| message["method"] != "Page.screencastVisibilityChanged"),
            "a queued activation cannot publish superseded visibility"
        );
        let current = current.wait().await;
        assert!(matches!(
            handle
                .with_state(
                    |state| state.try_complete_ready_command_dispatch_with_context(
                        current,
                        &mut current_context
                    )
                )
                .unwrap()
                .unwrap_or_else(|_| panic!("activation completion is finite")),
            CdpCommandTaskStep::Complete(_)
        ));
        handle
            .with_state(|state| {
                let conn = state.connection;
                assert_eq!(conn.browser_context.as_ref().unwrap().id, "BID-routing");
                assert_eq!(
                    conn.browser_context_by_id("BID-activation")
                        .unwrap()
                        .active_target_id(),
                    Some("TID-activation-first")
                );
                for (target, visible) in [
                    ("TID-activation-first", true),
                    ("TID-activation-second", false),
                ] {
                    let page = conn
                        .browser_context_by_id("BID-activation")
                        .unwrap()
                        .page_target(target)
                        .unwrap()
                        .loaded_page()
                        .unwrap();
                    assert_eq!(page.document_activity().visible, visible);
                }
            })
            .unwrap();
    }

    #[tokio::test]
    async fn complete_session_detachment_uses_direct_access_without_a_mailbox() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CommandDispatchContext, CommandOwnerScope,
            ParsedCdpCommand,
        };
        use crate::testing::TestContext;
        use serde_json::json;

        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-direct-detachment".into());
        browser.set_active_target_id("TID-direct-detachment");
        browser.attach_active_session("SID-direct-detachment");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank",
                Some("SID-direct-detachment"),
            )
            .await;
        for (id, method, params) in [
            (
                1,
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": "globalThis.directPreload = true;"}),
            ),
            (2, "Runtime.addBinding", json!({"name": "directBinding"})),
            (3, "Network.enable", json!({})),
            (
                4,
                "Emulation.setScriptExecutionDisabled",
                json!({"value": true}),
            ),
        ] {
            context.process_async(json!({"id": id, "sessionId": "SID-direct-detachment", "method": method, "params": params})).await;
            assert!(context.take_response_by_id(id).get("result").is_some());
        }
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let command = ParsedCdpCommand::parse_str(json!({
            "id": 10, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-direct-detachment"}
        }).to_string()).unwrap();
        let mut dispatch_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch_context)
            })
            .unwrap()
        else {
            panic!("native cleanup must be owned work");
        };

        handle
            .with_state(|state| {
                let connection = state.connection;
                assert!(
                    connection.state_mailbox.receiver.is_some(),
                    "no actor is pumping a mailbox"
                );
                assert!(connection.session_is_closing("SID-direct-detachment"));
                let owner = CommandOwnerScope::for_session("SID-direct-detachment");
                let inputs = connection.navigation_load_inputs_for_owner(&owner);
                assert!(inputs.document_start_scripts.is_empty());
                assert!(inputs.runtime_bindings.is_empty());
                assert!(
                    inputs
                        .runtime_inspector_session_restore_snapshots
                        .is_empty()
                );
                assert!(!inputs.script_execution_disabled);
                assert!(
                    !connection
                        .target_owner_state_for_session(Some("SID-direct-detachment"))
                        .unwrap()
                        .document_start_scripts
                        .is_empty(),
                    "script cleanup authority remains until native results are consumed"
                );
            })
            .unwrap();

        for (id, method, session, params) in [
            (11, "Browser.getVersion", None, json!({})),
            (
                12,
                "Page.addScriptToEvaluateOnNewDocument",
                Some("SID-direct-detachment"),
                json!({"source": "throw 1"}),
            ),
            (
                13,
                "Fetch.continueRequest",
                Some("SID-direct-detachment"),
                json!({"requestId": "missing"}),
            ),
        ] {
            let mut value = json!({"id": id, "method": method, "params": params});
            if let Some(session) = session {
                value["sessionId"] = json!(session);
            }
            let command = ParsedCdpCommand::parse_str(value.to_string()).unwrap();
            let CdpCommandTaskStep::Complete(outcome) = handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(
                        &command,
                        &mut CommandDispatchContext::default(),
                    )
                })
                .unwrap()
            else {
                panic!("the independent command must finish before cleanup is polled");
            };
            let response = outcome
                .into_parts()
                .0
                .into_iter()
                .find(|message| message["id"] == id)
                .unwrap();
            match id {
                11 => assert!(response.get("result").is_some()),
                12 => assert_eq!(response["error"]["message"], "Session is closing"),
                13 => assert_ne!(
                    response["error"]["message"], "Session is closing",
                    "continuations bypass Closing"
                ),
                _ => unreachable!(),
            }
        }
        let completed = tokio::time::timeout(std::time::Duration::from_secs(30), pending.wait())
            .await
            .expect("all handlers must finish using synchronous access");
        assert!(
            handle
                .with_state(|state| state
                    .connection
                    .has_attached_devtools_session("SID-direct-detachment"))
                .unwrap(),
            "a ready result cannot unbind before the command owner consumes it"
        );
        let step = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(
                    completed,
                    &mut dispatch_context,
                )
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("owned detachment needs no further connection wait"));
        let CdpCommandTaskStep::Complete(outcome) = step else {
            panic!("detachment must finish synchronously");
        };
        let messages = outcome.into_parts().0;
        let response = messages
            .iter()
            .position(|message| message["id"] == 10)
            .unwrap();
        assert!(messages[response].get("result").is_some());
        let detached = messages
            .iter()
            .position(|message| message["method"] == "Target.detachedFromTarget")
            .unwrap();
        assert!(
            response < detached,
            "explicit target detachment preserves its response/event boundary"
        );
        assert!(
            !handle
                .with_state(|state| state
                    .connection
                    .has_attached_devtools_session("SID-direct-detachment"))
                .unwrap()
        );
    }

    #[tokio::test]
    async fn cancelled_detachment_observes_page_close_before_releasing_its_binding() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        };
        use crate::testing::TestContext;
        use serde_json::json;

        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-cancel-detachment".into());
        browser.set_active_target_id("TID-cancel-detachment");
        browser.attach_active_session("SID-cancel-detachment");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank",
                Some("SID-cancel-detachment"),
            )
            .await;
        context.process_async(json!({"id": 1, "sessionId": "SID-cancel-detachment", "method": "Page.addScriptToEvaluateOnNewDocument", "params": {"source": "globalThis.cancelPreload = true;"}})).await;
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let command = ParsedCdpCommand::parse_str(json!({"id": 10, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-cancel-detachment"}}).to_string()).unwrap();
        let mut dispatch_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch_context)
            })
            .unwrap()
        else {
            panic!("cleanup must have an owned wait");
        };
        assert!(pending.owns_session_disposal());
        let mut retained_context = dispatch_context.clone();
        drop(pending);
        let recovery = handle
            .with_state(|state| {
                state.complete_failed_command_wait_with_context(
                    Some(10),
                    None,
                    "controlled cancellation",
                    &mut dispatch_context,
                )
            })
            .unwrap();
        let CdpCommandTaskStep::Pending(recovery) = recovery else {
            panic!("failure retains the native close acknowledgement");
        };
        assert!(recovery.owns_session_disposal());
        assert!(
            retained_context.take_command_failure_cleanup().is_none(),
            "cloned contexts share one recovery authority"
        );
        assert!(
            handle
                .with_state(|state| state
                    .connection
                    .has_attached_devtools_session("SID-cancel-detachment"))
                .unwrap()
        );
        assert!(
            handle
                .with_state(|state| state
                    .connection
                    .renderer_page_residence_identity_for_owner(
                        &crate::conn::CommandOwnerScope::for_session("SID-cancel-detachment"),
                    )
                    .is_none())
                .unwrap()
        );
        let completed = tokio::time::timeout(std::time::Duration::from_secs(30), recovery.wait())
            .await
            .unwrap();
        assert!(
            handle
                .with_state(|state| state
                    .connection
                    .has_attached_devtools_session("SID-cancel-detachment"))
                .unwrap()
        );
        let step = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(
                    completed,
                    &mut dispatch_context,
                )
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("recovery must finish with finite state access"));
        let CdpCommandTaskStep::Complete(outcome) = step else {
            panic!("recovery must terminate the command");
        };
        let messages = outcome.into_parts().0;
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == 10)
                .count(),
            1
        );
        assert!(
            messages
                .iter()
                .find(|message| message["id"] == 10)
                .unwrap()
                .get("error")
                .is_some()
        );
        assert!(
            !handle
                .with_state(|state| state
                    .connection
                    .has_attached_devtools_session("SID-cancel-detachment"))
                .unwrap()
        );
    }

    #[tokio::test]
    async fn parent_detachment_joins_a_child_already_being_disposed() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        };
        use crate::testing::TestContext;
        use serde_json::json;

        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-joined-disposal".into());
        browser.set_active_target_id("TID-joined-disposal");
        browser.attach_active_session("SID-child");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-child"))
            .await;
        context.conn.register_browser_session("SID-parent".into());
        let route = context.conn.session_route(Some("SID-child")).unwrap();
        context.conn.register_auto_attached_session_route_for_test(
            "SID-child".into(),
            Some("SID-parent"),
            route,
        );
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let mut child_context = CommandDispatchContext::default();
        let mut parent_context = CommandDispatchContext::default();
        let child = ParsedCdpCommand::parse_str(json!({"id": 10, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-child"}}).to_string()).unwrap();
        let parent = ParsedCdpCommand::parse_str(json!({"id": 11, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-parent"}}).to_string()).unwrap();
        let CdpCommandTaskStep::Pending(child) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&child, &mut child_context)
            })
            .unwrap()
        else {
            panic!("child cleanup must be pending");
        };
        let CdpCommandTaskStep::Pending(parent) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&parent, &mut parent_context)
            })
            .unwrap()
        else {
            panic!("parent must join its closing child");
        };
        let parent = parent.wait();
        tokio::pin!(parent);
        tokio::select! { biased;
            _ = &mut parent => panic!("the child binding has not been released"),
            _ = std::future::ready(()) => {},
        }
        let completed = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
            .await
            .unwrap();
        let child = handle
            .with_state(|state| {
                state
                    .try_complete_ready_command_dispatch_with_context(completed, &mut child_context)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("child finalization is synchronous"));
        let CdpCommandTaskStep::Complete(child) = child else {
            panic!("child must finish");
        };
        assert_eq!(
            child
                .into_parts()
                .0
                .iter()
                .filter(|event| event["method"] == "Target.detachedFromTarget"
                    && event["params"]["sessionId"] == "SID-child")
                .count(),
            1
        );
        let completed = tokio::time::timeout(std::time::Duration::from_secs(30), parent)
            .await
            .unwrap();
        let parent = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(
                    completed,
                    &mut parent_context,
                )
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("parent finalization is synchronous"));
        let CdpCommandTaskStep::Complete(parent) = parent else {
            panic!("parent must finish");
        };
        let events = parent.into_parts().0;
        assert!(
            events
                .iter()
                .any(|event| event["id"] == 11 && event.get("result").is_some())
        );
        assert!(
            !events
                .iter()
                .any(|event| event["method"] == "Target.detachedFromTarget"
                    && event["params"]["sessionId"] == "SID-child")
        );
        handle
            .with_state(|state| {
                assert!(!state.connection.has_attached_devtools_session("SID-parent"));
                assert!(!state.connection.has_attached_devtools_session("SID-child"));
            })
            .unwrap();
    }

    #[tokio::test]
    async fn native_script_cleanup_allows_peer_progress_before_result_consumption() {
        use crate::conn::{
            BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        };
        use crate::testing::TestContext;
        use serde_json::json;
        use tokio::sync::oneshot;

        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-direct-state".into());
        browser.set_active_target_id("TID-direct-state");
        browser.attach_active_session("SID-direct-state");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-direct-state"))
            .await;
        context
            .process_async(json!({
                "id": 1, "sessionId": "SID-direct-state",
                "method": "Page.addScriptToEvaluateOnNewDocument",
                "params": {"source": "globalThis.directStatePreload = true;"},
            }))
            .await;
        context.expect_result(1, json!({"identifier": "1"}), Some("SID-direct-state"));
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let pending = handle
            .with_state(|state| {
                state
                    .connection
                    .prepare_session_script_cleanup("SID-direct-state")
            })
            .unwrap();
        let (ready, ready_rx) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let task_handle = handle.clone();
        let workflow = async move {
            let completed = pending.wait().await;
            ready.send(()).unwrap();
            released.await.unwrap();
            task_handle.with_state(|state| completed.finish(state.connection))
        };
        tokio::pin!(workflow);
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::select! {
                ready = ready_rx => ready.unwrap(),
                _ = &mut workflow => panic!("cleanup must wait for result consumption"),
            }
        })
        .await
        .expect("the native cleanup must acknowledge its admitted operation");

        let command = ParsedCdpCommand::parse_str(
            json!({"id": 42, "method": "Browser.getVersion"}).to_string(),
        )
        .unwrap();
        let mut dispatch_context = CommandDispatchContext::default();
        let step = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch_context)
            })
            .unwrap();
        let CdpCommandTaskStep::Complete(outcome) = step else {
            panic!("the independent command must complete while cleanup is held");
        };
        let messages = outcome.into_parts().0;
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == 42 && message.get("result").is_some())
        );
        assert_eq!(
            handle
                .with_state(|state| {
                    state
                        .connection
                        .target_owner_state_for_session(Some("SID-direct-state"))
                        .unwrap()
                        .document_start_scripts
                        .len()
                })
                .unwrap(),
            1,
            "metadata is retained until the exact native result is consumed"
        );

        release.send(()).unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), workflow)
            .await
            .expect("the released cleanup result must be consumed")
            .unwrap();
        assert!(result.predecessor.is_some());
        result.into_result().unwrap();
        assert_eq!(
            handle
                .with_state(|state| {
                    state
                        .connection
                        .target_owner_state_for_session(Some("SID-direct-state"))
                        .unwrap()
                        .document_start_scripts
                        .len()
                })
                .unwrap(),
            0
        );
    }
}
