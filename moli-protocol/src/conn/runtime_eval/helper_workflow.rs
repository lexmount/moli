use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};

#[derive(Clone)]
pub(crate) struct RuntimeHelperIdentity {
    scope: RuntimePageScope,
    correlation: RendererCommandCorrelation,
}

impl RuntimeHelperIdentity {
    pub(crate) fn cancel(&self, conn: &mut CdpConnection) {
        if self.scope.owns_current_host(conn) {
            conn.take_renderer_call_if_correlation_matches_for_owner(
                self.scope.owner(),
                self.correlation,
            );
        }
    }
}

struct RuntimeHelperCancellation {
    identity: RuntimeHelperIdentity,
    access: Option<ProtocolStateAccess>,
}

impl Drop for RuntimeHelperCancellation {
    fn drop(&mut self) {
        if let Some(access) = self.access.take() {
            let identity = self.identity.clone();
            access.defer_cleanup(Box::new(move |conn| identity.cancel(conn)));
        }
    }
}

/// One internal Inspector request, retaining its existing session-local call
/// registration while its native dispatch and response wait outside state.
/// The caller retains ownership of any object group referenced by the request.
pub(crate) struct PreparedRuntimeHelper {
    command_id: u64,
    pending: Option<PendingRuntimeProtocolMessageDispatch>,
    cancellation: RuntimeHelperCancellation,
    object_group: Option<String>,
}

impl PreparedRuntimeHelper {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        method: &str,
        params: Value,
    ) -> Result<Self> {
        let access = conn.protocol_state_access();
        let scope = RuntimePageScope::capture(conn, &owner)?;
        let command_id = conn.next_internal_runtime_command_id();
        let object_group = params
            .get("objectGroup")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let descriptor = RendererCommandDescriptor::for_document_helper(
            json!({"id":command_id,"method":method,"params":params}).to_string(),
        )?;
        let owner = scope.owner();
        let action = method.split_once('.').map_or(method, |(_, action)| action);
        let pending = conn.start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
            owner, action, descriptor, command_id,
        )?;
        let correlation = conn
            .target_devtools_session_state_for_owner(owner)
            .and_then(|session| session.renderer_call_for_frontend(command_id))
            .expect("synchronous helper admission registered its exact renderer call");
        Ok(Self {
            command_id,
            pending: Some(pending),
            cancellation: RuntimeHelperCancellation {
                identity: RuntimeHelperIdentity { scope, correlation },
                access,
            },
            object_group,
        })
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> Result<BackgroundCommandResponsePayload> {
        let result = self.run_inner(driver).await;
        let identity = self.cancellation.identity.clone();
        // Inline compatibility shares the same finish boundary; direct and
        // mailbox workflows also keep this authority on cancellation/drop.
        let cleanup = driver
            .with_state(move |state| state.cancel_runtime_helper(identity))
            .await;
        if cleanup.is_ok() {
            self.cancellation.access.take();
        }
        match (result, cleanup) {
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            (Ok(response), Ok(())) => Ok(response),
        }
    }

    async fn run_inner(
        &mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> Result<BackgroundCommandResponsePayload> {
        let pending = self
            .pending
            .take()
            .expect("one helper owns one native dispatch");
        let mut completed = pending.wait().await?;
        let receiver = completed.take_deferred_response_receiver();
        let identity = self.cancellation.identity.clone();
        let command_id = self.command_id;
        let object_group = self.object_group.clone();
        let direct = driver
            .with_state(move |state| {
                state.finish_runtime_helper_dispatch(identity, command_id, object_group, completed)
            })
            .await??;
        if let Some(response) = direct {
            return Ok(response);
        }
        let response = receiver
            .context("internal Runtime helper has no response receiver")?
            .await
            .context("internal Runtime helper response cancelled")?;
        let identity = self.cancellation.identity.clone();
        let object_group = self.object_group.clone();
        driver
            .with_state(move |state| {
                state.finish_runtime_helper_response(identity, command_id, object_group, response)
            })
            .await?
    }
}

pub(crate) async fn dispatch_runtime_helper_using(
    driver: &mut ProtocolWorkflowDriver<'_>,
    owner: CommandOwnerScope,
    method: &'static str,
    params: Value,
) -> Result<BackgroundCommandResponsePayload> {
    driver
        .with_state(move |state| state.prepare_runtime_helper(owner, method, params))
        .await??
        .run(driver)
        .await
}

fn register_helper_result(
    conn: &mut CdpConnection,
    identity: &RuntimeHelperIdentity,
    object_group: Option<&str>,
    response: &BackgroundCommandResponsePayload,
) {
    if let (Some(group), BackgroundCommandResponsePayload::Success { result }) =
        (object_group, response)
    {
        conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
            identity.scope.owner(),
            result,
            group,
        );
    }
}

impl RuntimeHelperIdentity {
    pub(crate) fn finish_dispatch(
        &self,
        conn: &mut CdpConnection,
        command_id: u64,
        object_group: Option<String>,
        completed: CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<Option<BackgroundCommandResponsePayload>> {
        anyhow::ensure!(self.scope.is_current(conn), NoDocumentLoaded);
        let output = conn.complete_runtime_protocol_message(completed)?;
        let response = output
            .as_ref()
            .and_then(|output| renderer_command_turn_frontend_protocol_response(output, command_id))
            .map(BackgroundCommandResponsePayload::from_runtime_inspector_message);
        if let Some(response) = &response {
            register_helper_result(conn, self, object_group.as_deref(), response);
        }
        Ok(response)
    }

    pub(crate) fn finish_response(
        &self,
        conn: &mut CdpConnection,
        command_id: u64,
        object_group: Option<String>,
        response: moli_core::RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<BackgroundCommandResponsePayload> {
        anyhow::ensure!(self.scope.is_current(conn), NoDocumentLoaded);
        let ready =
            RuntimeInspectorResponseReady::for_owner(command_id, self.scope.owner(), Ok(response));
        let response = conn
            .resolve_runtime_inspector_response_ready(ready)
            .context("internal Runtime helper response lost its exact correlation")?
            .into_command_response_payload();
        register_helper_result(conn, self, object_group.as_deref(), &response);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BrowserContext, ProtocolStateOwner};
    use crate::testing::TestContext;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-helper-workflow".into());
        browser.set_active_target_id("TID-helper-workflow");
        browser.attach_active_session("SID-helper-workflow");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank",
                Some("SID-helper-workflow"),
            )
            .await;
        context
    }

    #[tokio::test]
    async fn runtime_helper_and_realm_inventory_complete_without_a_mailbox() {
        let mut context = fixture().await;
        drop(context.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(context.conn);
        let access = ProtocolStateAccess::Direct(owner.handle());
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let response = dispatch_runtime_helper_using(
            &mut driver,
            command_owner.clone(),
            "Runtime.evaluate",
            json!({"expression":"40 + 2","returnByValue":true}),
        )
        .await
        .unwrap();
        let BackgroundCommandResponsePayload::Success { result } = response else {
            panic!("the internal Runtime helper must return its own response");
        };
        assert_eq!(result["result"]["value"], 42);
        let realms = crate::conn::runtime_realm_inventory_using(&mut driver, command_owner)
            .await
            .unwrap();
        assert!(!realms.is_empty());
        assert!(realms.iter().all(|realm| {
            realm
                .realm_id
                .as_ref()
                .is_some_and(|id| id.as_str().starts_with("TID-helper-workflow:"))
        }));
    }

    #[tokio::test]
    async fn browser_helper_keeps_its_page_when_the_active_target_changes() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let access = context.conn.protocol_state_access().unwrap();
        let helper = PreparedRuntimeHelper::prepare(
            &mut context.conn,
            CommandOwnerScope::for_route(CdpSessionRoute::Browser),
            "Runtime.evaluate",
            json!({"expression":"42","returnByValue":true}),
        )
        .unwrap();
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let mut work = Box::pin(helper.run(&mut driver));
        let ready = tokio::select! {
            request = requests.recv() => request.unwrap(),
            _ = &mut work => panic!("hold the original native completion before state settlement"),
        };
        let browser = context.conn.browser_context.as_mut().unwrap();
        browser.stage_background_target(
            "TID-helper-neighbour".into(),
            None,
            "about:blank".into(),
            None,
            None,
        );
        let _updates = browser
            .start_select_page_target("TID-helper-neighbour")
            .expect("switch logical selection without replacing the original Page");
        ready.apply(&mut context.conn);
        let response = loop {
            tokio::select! {
                request = requests.recv() => request.unwrap().apply(&mut context.conn),
                result = &mut work => break result.unwrap(),
            }
        };
        let BackgroundCommandResponsePayload::Success { result } = response else {
            panic!("the original Page still owns its completed helper request");
        };
        assert_eq!(result["result"]["value"], 42);
        assert_eq!(
            context
                .conn
                .browser_context
                .as_ref()
                .unwrap()
                .active_target_id(),
            Some("TID-helper-neighbour")
        );
        assert!(
            !context
                .conn
                .browser_context
                .as_ref()
                .unwrap()
                .has_loaded_page()
        );
    }

    #[tokio::test]
    async fn realm_inventory_rejects_a_completed_query_after_its_binding_is_reused() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let access = context.conn.protocol_state_access().unwrap();
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let inventory =
            PreparedRuntimeRealmInventory::prepare(&mut context.conn, &command_owner).unwrap();
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let mut work = Box::pin(inventory.run(&mut driver));
        let finish = tokio::select! {
            request = requests.recv() => request.unwrap(),
            _ = &mut work => panic!("hold inventory after native completion and before state settlement"),
        };
        let route = context
            .conn
            .session_route(Some("SID-helper-workflow"))
            .unwrap();
        context
            .conn
            .register_session_route_for_test("SID-helper-workflow", route);
        finish.apply(&mut context.conn);
        let error = work.await.unwrap_err();
        assert!(error.downcast_ref::<NoDocumentLoaded>().is_some());
        // A fresh query on the successor binding remains valid.
        let realms = context
            .conn
            .runtime_realm_inventory_for_owner_async(&command_owner)
            .await
            .unwrap();
        assert!(!realms.is_empty());
    }

    #[tokio::test]
    async fn losing_a_helper_native_result_retires_its_exact_renderer_call() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let access = context.conn.protocol_state_access().unwrap();
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let helper = PreparedRuntimeHelper::prepare(
            &mut context.conn,
            command_owner.clone(),
            "Runtime.evaluate",
            json!({"expression":"42","returnByValue":true}),
        )
        .unwrap();
        let id = helper.command_id;
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let mut work = Box::pin(helper.run(&mut driver));
        let finish = tokio::select! {
            request = requests.recv() => request.unwrap(),
            _ = &mut work => panic!("completion must first acquire finite state"),
        };
        assert!(
            context
                .conn
                .renderer_runtime_command_cause_for_owner(&command_owner, id)
                .is_some()
        );
        drop(work);
        // The held finish request observes its cancelled receiver and skips
        // mutation. The scope's finite cancellation is the next operation.
        finish.apply(&mut context.conn);
        requests.recv().await.unwrap().apply(&mut context.conn);
        assert!(
            context
                .conn
                .renderer_runtime_command_cause_for_owner(&command_owner, id)
                .is_none()
        );
    }

    #[tokio::test]
    async fn helper_cancellation_rejects_a_recreated_host_with_identical_call_components() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let helper = PreparedRuntimeHelper::prepare(
            &mut context.conn,
            CommandOwnerScope::for_route(CdpSessionRoute::Browser),
            "Runtime.evaluate",
            json!({"expression":"42"}),
        )
        .unwrap();
        let identity = helper.cancellation.identity.clone();
        let id = helper.command_id;
        let expected = identity.correlation;
        let old = context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .take_page_target_for_close("TID-helper-workflow")
            .unwrap();
        context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                "TID-helper-workflow".into(),
                Some("SID-helper-workflow".into()),
                "about:blank".into(),
            ));
        let descriptor = RendererCommandDescriptor::for_document_helper(
            json!({"id":id,"method":"Runtime.evaluate","params":{"expression":"43"}}).to_string(),
        )
        .unwrap();
        let replacement = context
            .conn
            .try_register_renderer_call_for_owner(
                identity.scope.owner(),
                id,
                expected.dispatched_attachment_id(),
                descriptor,
            )
            .unwrap();
        let replacement = replacement.correlation();
        assert_eq!(
            replacement, expected,
            "force every numeric correlation component to collide"
        );
        assert!(!identity.scope.owns_current_host(&context.conn));
        drop(helper);
        requests.recv().await.unwrap().apply(&mut context.conn);
        let current = context
            .conn
            .target_devtools_session_state_for_owner(identity.scope.owner())
            .unwrap()
            .renderer_call_for_frontend(id);
        assert_eq!(current, Some(replacement));
        context
            .conn
            .take_renderer_call_if_correlation_matches_for_owner(
                identity.scope.owner(),
                replacement,
            );
        drop(old);
    }

    #[tokio::test]
    async fn a_closed_binding_cannot_cancel_an_identical_rebound_session_route() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let helper = PreparedRuntimeHelper::prepare(
            &mut context.conn,
            command_owner.clone(),
            "Runtime.evaluate",
            json!({"expression":"42"}),
        )
        .unwrap();
        let id = helper.command_id;
        let expected = helper.cancellation.identity.correlation;
        let route = context
            .conn
            .session_route(Some("SID-helper-workflow"))
            .unwrap();
        context
            .conn
            .register_session_route_for_test("SID-helper-workflow", route);
        assert!(!helper.cancellation.identity.scope.binding_is_current());
        // Keep every literal route and call-id component identical. The
        // original binding lifetime must still prevent access to its successor.
        drop(helper);
        requests.recv().await.unwrap().apply(&mut context.conn);
        let current = context
            .conn
            .target_devtools_session_state_for_owner(&command_owner)
            .unwrap()
            .renderer_call_for_frontend(id);
        assert_eq!(current, Some(expected));
        assert!(
            context
                .conn
                .take_renderer_call_if_correlation_matches_for_owner(&command_owner, expected,)
        );
    }

    #[tokio::test]
    async fn helper_cancellation_cannot_remove_a_reused_frontend_call_id() {
        let mut context = fixture().await;
        let mut requests = context.conn.take_protocol_state_requests().unwrap();
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let helper = PreparedRuntimeHelper::prepare(
            &mut context.conn,
            command_owner.clone(),
            "Runtime.evaluate",
            json!({"expression":"42"}),
        )
        .unwrap();
        let id = helper.command_id;
        let correlation = helper.cancellation.identity.correlation;
        drop(helper);
        assert!(
            context
                .conn
                .take_renderer_call_if_correlation_matches_for_owner(&command_owner, correlation,)
        );
        let descriptor = RendererCommandDescriptor::for_document_helper(
            json!({"id":id,"method":"Runtime.evaluate","params":{"expression":"43"}}).to_string(),
        )
        .unwrap();
        let replacement = context
            .conn
            .try_register_renderer_call_for_owner(
                &command_owner,
                id,
                correlation.dispatched_attachment_id(),
                descriptor,
            )
            .unwrap();
        let expected = replacement.correlation();
        assert_ne!(expected.renderer_call_id(), correlation.renderer_call_id());
        requests.recv().await.unwrap().apply(&mut context.conn);
        let current = context
            .conn
            .target_devtools_session_state_for_owner(&command_owner)
            .unwrap()
            .renderer_call_for_frontend(id);
        assert_eq!(current, Some(expected));
        assert!(
            context
                .conn
                .take_renderer_call_if_correlation_matches_for_owner(&command_owner, expected,)
        );
    }
}
