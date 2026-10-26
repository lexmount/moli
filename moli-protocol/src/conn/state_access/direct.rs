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
/// The existing actor retains its ownership until its remaining asynchronous
/// connection borrows have migrated; it does not share this owner's storage.
pub struct ProtocolStateOwner {
    connection: Rc<RefCell<CdpConnection>>,
}

impl ProtocolStateOwner {
    pub fn new(connection: CdpConnection) -> Self {
        Self {
            connection: Rc::new(RefCell::new(connection)),
        }
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
