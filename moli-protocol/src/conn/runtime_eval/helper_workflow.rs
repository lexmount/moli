use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};

#[derive(Clone)]
pub(crate) struct RuntimeHelperIdentity {
    scope: RuntimePageScope,
    correlation: RendererCommandCorrelation,
}

impl RuntimeHelperIdentity {
    pub(crate) fn host_scope(&self) -> ProtocolPageHostScope {
        self.scope.host_scope()
    }
    pub(crate) fn capture_script(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
        id: u64,
    ) -> Option<Self> {
        let scope = RuntimePageScope::capture(conn, owner).ok()?;
        let correlation = conn
            .target_devtools_session_state_for_owner(owner)?
            .renderer_call_for_frontend(id)?;
        Some(Self { scope, correlation })
    }

    pub(crate) fn cancel_script(&self, conn: &mut CdpConnection) {
        if !self.scope.owns_current_host(conn) {
            return;
        }
        let owner = self.scope.owner();
        let id = self.correlation.frontend_command_id().get();
        if conn
            .target_devtools_session_state_for_owner(owner)
            .and_then(|state| state.renderer_call_for_frontend(id))
            != Some(self.correlation)
        {
            return;
        }
        conn.forget_pending_inspector_await_for_owner(id, owner);
        conn.take_renderer_call_if_correlation_matches_for_owner(owner, self.correlation);
    }

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

pub(crate) struct RuntimeHelperOutput {
    pub(crate) response: Result<BackgroundCommandResponsePayload>,
    pub(crate) predecessor: Option<moli_core::RendererOutputFence>,
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

    pub(crate) async fn run(self, driver: &mut ProtocolWorkflowDriver<'_>) -> RuntimeHelperOutput {
        let mut predecessor = None;
        let response = self
            .run_collecting_renderer_predecessor(driver, &mut predecessor)
            .await;
        RuntimeHelperOutput {
            response,
            predecessor,
        }
    }

    pub(crate) async fn run_collecting_renderer_predecessor(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<BackgroundCommandResponsePayload> {
        let result = self.run_inner(driver, predecessor).await;
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
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<BackgroundCommandResponsePayload> {
        let pending = self
            .pending
            .take()
            .expect("one helper owns one native dispatch");
        let mut completed = pending.wait().await?;
        if let Some(fence) = completed.renderer_output_predecessor() {
            fence.merge_into_same_stream_tail(predecessor);
        }
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
        if let Some(fence) = response.renderer_output_predecessor() {
            fence.merge_into_same_stream_tail(predecessor);
        }
        let identity = self.cancellation.identity.clone();
        let object_group = self.object_group.clone();
        driver
            .with_state(move |state| {
                state.finish_runtime_helper_response(identity, command_id, object_group, response)
            })
            .await?
    }
}

#[cfg(test)]
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
        .response
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
    async fn neutral_runtime_keeps_original_host_before_poll_between_queries_and_at_native_finish()
    {
        use crate::{
            AutomationCommandTaskStep,
            automation::{
                AutomationCommand, AutomationContext, DevToolsEvaluateScriptCommand,
                DevToolsResultOwnership, FrontendProtocol,
            },
        };
        for boundary in 0..3 {
            let mut fixture = fixture().await;
            let mut requests = fixture.conn.take_protocol_state_requests().unwrap();
            let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
            let step =
                fixture
                    .conn
                    .start_automation_command_dispatch(AutomationCommand::EvaluateScript(
                        DevToolsEvaluateScriptCommand {
                            context: AutomationContext {
                                protocol: FrontendProtocol::WebDriverBidi,
                                target_id: Some("TID-helper-workflow".into()),
                                session_id: None,
                                browser_context_id: None,
                            },
                            realm_id: None,
                            world_name: None,
                            expression: "globalThis.__originalRuntimeScriptRan = true; 42".into(),
                            await_promise: false,
                            user_gesture: false,
                            webdriver_bidi_file_prompt_handler: None,
                            result_ownership: DevToolsResultOwnership::None,
                            preserve_remote_metadata: false,
                            materialize_bidi_script_result: false,
                            serialization_options: None,
                        },
                    ));
            let AutomationCommandTaskStep::Pending(mut pending) = step else {
                panic!("Runtime starts as an owned task")
            };
            let id = pending.internal_command_id();
            let mut work = Box::pin(pending.take_wait().wait());
            let mut ready = None;
            if boundary == 1 {
                let request = tokio::select! {
                    request = requests.recv() => request.unwrap(),
                    _ = &mut work => panic!("first state turn is held"),
                };
                request.apply(&mut fixture.conn);
            } else if boundary == 2 {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        tokio::select! {
                            request = requests.recv() => request.unwrap().apply(&mut fixture.conn),
                            _ = &mut work => panic!("hold the native finish before settling"),
                        }
                        if fixture.conn.target_devtools_session_state_for_owner(&command_owner).unwrap().renderer_call_for_frontend(id).is_some() { break; }
                    }
                    ready = Some(tokio::select! {
                        request = requests.recv() => request.unwrap(),
                        _ = &mut work => panic!("Native completion must enter its finite owner turn"),
                    });
                }).await.unwrap();
            }
            let correlation = fixture
                .conn
                .target_devtools_session_state_for_owner(&command_owner)
                .unwrap()
                .renderer_call_for_frontend(id);
            assert_eq!(correlation.is_some(), boundary == 2);
            let old_host = fixture
                .conn
                .browser_context
                .as_mut()
                .unwrap()
                .take_page_target_for_close("TID-helper-workflow")
                .unwrap();
            fixture
                .conn
                .browser_context
                .as_mut()
                .unwrap()
                .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                    "TID-helper-workflow".into(),
                    Some("SID-helper-workflow".into()),
                    "about:blank".into(),
                ));
            fixture
                .install_navigation_fixture_for_session_owner(
                    "about:blank",
                    Some("SID-helper-workflow"),
                )
                .await;
            if let Some(expected) = correlation {
                let descriptor = RendererCommandDescriptor::for_document_helper(
                    json!({"id":id,"method":"Runtime.evaluate","params":{"expression":"43"}})
                        .to_string(),
                )
                .unwrap();
                let replacement = fixture
                    .conn
                    .try_register_renderer_call_for_owner(
                        &command_owner,
                        id,
                        expected.dispatched_attachment_id(),
                        descriptor,
                    )
                    .unwrap();
                assert_eq!(
                    replacement.correlation(),
                    expected,
                    "keep all numeric call components identical on the successor host"
                );
            }
            if let Some(ready) = ready {
                ready.apply(&mut fixture.conn);
            }
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let completed = loop {
                        tokio::select! {
                            request = requests.recv() => request.unwrap().apply(&mut fixture.conn),
                            completed = &mut work => break completed,
                        }
                    };
                    match fixture
                        .conn
                        .complete_automation_command_dispatch(pending.with_completion(completed))
                    {
                        AutomationCommandTaskStep::Complete(outcome) => break outcome,
                        AutomationCommandTaskStep::Pending(next) => {
                            pending = next;
                            work = Box::pin(pending.take_wait().wait());
                        }
                    }
                }
            })
            .await
            .unwrap();
            let (result, output) = outcome.into_result_and_owner_turn();
            assert!(
                result.is_err(),
                "boundary {boundary} cannot admit or finish against a recreated host"
            );
            let (_, _, _, _, _, fence) = output.into_renderer_owner_turn_parts();
            if boundary == 2 {
                assert!(
                    fence.is_some(),
                    "accepted Native output keeps its original position even when state settlement is rejected"
                );
            }
            assert_eq!(
                fixture
                    .conn
                    .target_devtools_session_state_for_owner(&command_owner)
                    .unwrap()
                    .renderer_call_for_frontend(id),
                correlation,
                "the old task cannot consume the identically numbered call in the successor host"
            );
            let response = dispatch_runtime_helper_using(
                &mut ProtocolWorkflowDriver::inline(&mut fixture.conn), command_owner.clone(), "Runtime.evaluate",
                json!({"expression":"globalThis.__originalRuntimeScriptRan === true","returnByValue":true}),
            ).await.unwrap();
            let BackgroundCommandResponsePayload::Success { result } = response else {
                panic!("successor evaluation must succeed")
            };
            assert_eq!(
                result["result"]["value"],
                json!(false),
                "the old workflow must never execute its script on the new host: {result:?}"
            );
            // Remove the test's deliberately retained successor journal entry.
            if let Some(correlation) = correlation {
                fixture
                    .conn
                    .take_renderer_call_if_correlation_matches_for_owner(
                        &command_owner,
                        correlation,
                    );
            }
            drop(old_host);
        }
    }

    fn inspect_direct<T: 'static>(
        access: &ProtocolStateAccess,
        operation: impl FnOnce(&mut CdpConnection) -> T + 'static,
    ) -> T {
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let reply = result.clone();
        access.defer_cleanup(Box::new(move |conn| {
            *reply.borrow_mut() = Some(operation(conn))
        }));
        result
            .borrow_mut()
            .take()
            .expect("direct owner is available outside its finite state turn")
    }

    #[tokio::test]
    async fn neutral_runtime_direct_owner_cancel_and_ready_loss_preserve_the_original_result() {
        use crate::{
            AutomationCommandTaskStep,
            automation::{
                AutomationCommand, AutomationContext, DevToolsEvaluateScriptCommand,
                DevToolsResultOwnership, FrontendProtocol,
            },
        };
        for boundary in 0..3 {
            let before_poll = boundary == 0;
            let ready_loss = boundary == 2;
            let mut fixture = fixture().await;
            drop(fixture.conn.take_protocol_state_requests());
            let owner = ProtocolStateOwner::new(fixture.conn);
            let access = ProtocolStateAccess::Direct(owner.handle());
            let command = AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    target_id: Some("TID-helper-workflow".into()),
                    session_id: None,
                    browser_context_id: None,
                },
                realm_id: None,
                world_name: None,
                expression: if ready_loss {
                    "40 + 2"
                } else {
                    "new Promise(() => { globalThis.__ownedRuntimeEntered = true; })"
                }
                .into(),
                await_promise: !ready_loss,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            });
            let step = inspect_direct(&access, move |conn| {
                conn.start_automation_command_dispatch(command)
            });
            let AutomationCommandTaskStep::Pending(mut pending) = step else {
                panic!("ordinary Runtime starts as a task")
            };
            let id = pending.internal_command_id();
            let mut work = Box::pin(pending.take_wait().wait());
            let command_owner = CommandOwnerScope::for_route(inspect_direct(&access, |conn| {
                conn.target_session_route_for_target_id("TID-helper-workflow")
                    .unwrap()
            }));
            let mut peer = ProtocolWorkflowDriver::owned(access.clone());
            if !before_poll && !ready_loss {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let query = dispatch_runtime_helper_using(&mut peer, command_owner.clone(), "Runtime.evaluate", json!({"expression":"globalThis.__ownedRuntimeEntered === true","returnByValue":true}));
                        tokio::pin!(query);
                        let response = tokio::select! {
                            response = &mut query => response.unwrap(),
                            _ = &mut work => panic!("the author Promise stays unresolved while a same-Page peer progresses"),
                        };
                        let BackgroundCommandResponsePayload::Success { result } = response else { panic!("the same-Page peer succeeds during the held author Promise") };
                        if result["result"]["value"] == json!(true) { break; }
                    }
                }).await.unwrap();
                let check_owner = command_owner.clone();
                inspect_direct(&access, move |conn| {
                    assert!(
                        conn.target_devtools_session_state_for_owner(&check_owner)
                            .unwrap()
                            .has_pending_inspector_awaits()
                    );
                    assert!(
                        conn.target_devtools_session_state_for_owner(&check_owner)
                            .unwrap()
                            .renderer_call_for_frontend(id)
                            .is_some()
                    );
                    assert_eq!(conn.pending_runtime_await_jobs.len(), 1);
                });
            }
            if ready_loss {
                let completed = tokio::time::timeout(std::time::Duration::from_secs(5), &mut work)
                    .await
                    .unwrap();
                // Lose the fully ready result before the actor adopts it.
                drop(completed);
            }
            drop(work);
            let mut step = inspect_direct(&access, move |conn| {
                conn.fail_automation_command_wait(
                    *pending,
                    crate::OwnedProtocolWaitFailure::Cancelled,
                )
            });
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    match step {
                        AutomationCommandTaskStep::Complete(outcome) => break outcome,
                        AutomationCommandTaskStep::Pending(mut pending) => {
                            let completed = pending.take_wait().wait().await;
                            step = inspect_direct(&access, move |conn| {
                                conn.complete_automation_command_dispatch(
                                    pending.with_completion(completed),
                                )
                            });
                        }
                    }
                }
            })
            .await
            .unwrap();
            let (result, output) = outcome.into_result_and_owner_turn();
            let (events, _, _, _, _, fence) = output.into_renderer_owner_turn_parts();
            assert!(
                events
                    .iter()
                    .all(|event| event.protocol_message_id() != Some(id)),
                "a neutral command consumes its internal terminal exactly once"
            );
            if ready_loss {
                let crate::automation::AutomationResult::Script(script) =
                    result.expect("a lost accepted ready result retains success")
                else {
                    panic!("the original result is a script value")
                };
                let crate::automation::DevToolsScriptResult::Value(value) = *script else {
                    panic!("the accepted script has a value")
                };
                assert_eq!(value.value, json!(42));
                assert!(
                    fence.is_some(),
                    "ready loss preserves the accepted Native cursor"
                );
            } else {
                assert!(
                    result.is_err(),
                    "cancelled author work has one failure result"
                );
            }
            let check_owner = command_owner.clone();
            inspect_direct(&access, move |conn| {
                assert!(
                    !conn
                        .target_devtools_session_state_for_owner(&check_owner)
                        .unwrap()
                        .has_pending_inspector_awaits()
                );
                assert!(
                    conn.target_devtools_session_state_for_owner(&check_owner)
                        .unwrap()
                        .renderer_call_for_frontend(id)
                        .is_none()
                );
                assert!(conn.pending_runtime_await_jobs.is_empty());
            });
            let response = dispatch_runtime_helper_using(&mut peer, command_owner, "Runtime.evaluate", json!({"expression":"[globalThis.__ownedRuntimeEntered === true, 40 + 2]","returnByValue":true})).await.unwrap();
            let BackgroundCommandResponsePayload::Success { result } = response else {
                panic!("the original Page remains usable after cancellation")
            };
            assert_eq!(
                result["result"]["value"],
                json!([!before_poll && !ready_loss, 42])
            );
        }
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
                result = &mut work => break result.response.unwrap(),
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
