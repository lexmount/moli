use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{ProtocolStateHandle, ProtocolWorkflowDriver};

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
    access: Option<ProtocolStateHandle>,
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
                access: Some(access),
            },
            object_group,
        })
    }

    pub(crate) async fn run(self, driver: &mut ProtocolWorkflowDriver) -> RuntimeHelperOutput {
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
        driver: &mut ProtocolWorkflowDriver,
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<BackgroundCommandResponsePayload> {
        let result = self.run_inner(driver, predecessor).await;
        let identity = self.cancellation.identity.clone();
        // The original weak owner retains cancellation authority through finish.
        let cleanup = driver.with_state(move |state| state.cancel_runtime_helper(identity));
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
        driver: &mut ProtocolWorkflowDriver,
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
        let direct = driver.with_state(move |state| {
            state.finish_runtime_helper_dispatch(identity, command_id, object_group, completed)
        })??;
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
        driver.with_state(move |state| {
            state.finish_runtime_helper_response(identity, command_id, object_group, response)
        })?
    }
}

#[cfg(test)]
pub(crate) async fn dispatch_runtime_helper_using(
    driver: &mut ProtocolWorkflowDriver,
    owner: CommandOwnerScope,
    method: &'static str,
    params: Value,
) -> Result<BackgroundCommandResponsePayload> {
    driver
        .with_state(move |state| state.prepare_runtime_helper(owner, method, params))??
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
    use crate::conn::BrowserContext;
    use crate::testing::TestContext;
    use futures_util::FutureExt as _;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-helper-workflow".into());
        browser.set_active_target_id("TID-helper-workflow");
        browser.attach_active_session("SID-helper-workflow");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
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
            let gate = if boundary == 1 {
                Some(
                    fixture
                        .hold_fixture_renderer(Some("SID-helper-workflow"))
                        .await,
                )
            } else {
                None
            };
            let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
            let step = fixture.with_fixture(|conn| {
                conn.start_automation_command_dispatch(AutomationCommand::EvaluateScript(
                    DevToolsEvaluateScriptCommand {
                        context: AutomationContext {
                            protocol: FrontendProtocol::WebDriverBidi,
                            target_id: Some("TID-helper-workflow".into()),
                            session_id: None,
                            browser_context_id: None,
                        },
                        realm_id: None,
                        world_name: None,
                        expression: if boundary == 2 {
                            "new Promise(resolve => { globalThis.__originalRuntimeScriptRan = true; globalThis.__releaseOriginalRuntime = resolve; })"
                        } else { "globalThis.__originalRuntimeScriptRan = true; 42" }.into(),
                        await_promise: boundary == 2,
                        user_gesture: false,
                        webdriver_bidi_file_prompt_handler: None,
                        result_ownership: DevToolsResultOwnership::None,
                        preserve_remote_metadata: false,
                        materialize_bidi_script_result: false,
                        serialization_options: None,
                    },
                ))
            });
            let AutomationCommandTaskStep::Pending(mut pending) = step else {
                panic!("Runtime starts as an owned task")
            };
            let id = pending.internal_command_id();
            let mut work = Box::pin(pending.take_wait().wait());
            if boundary == 1 {
                assert!(
                    work.as_mut().now_or_never().is_none(),
                    "hold the actual first Native query"
                );
            } else if boundary == 2 {
                let mut peer = ProtocolWorkflowDriver::new(fixture.handle());
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let query = dispatch_runtime_helper_using(&mut peer, command_owner.clone(), "Runtime.evaluate", json!({"expression":"globalThis.__originalRuntimeScriptRan === true","returnByValue":true}));
                        tokio::pin!(query);
                        let response = tokio::select! {
                            response = &mut query => response.unwrap(),
                            _ = &mut work => panic!("the original author Promise stays unresolved"),
                        };
                        let BackgroundCommandResponsePayload::Success { result } = response else { panic!("same-Page peer progresses while the original author script is held") };
                        if result["result"]["value"] == json!(true) { break; }
                    }
                }).await.unwrap();
            }
            let inspector_session = fixture.with_fixture(|conn| {
                conn.target_renderer_runtime_inspector_session_id_for_session(Some(
                    "SID-helper-workflow",
                ))
            });
            let correlation = fixture.with_fixture(|conn| {
                conn.target_devtools_session_state_for_owner(&command_owner)
                    .unwrap()
                    .renderer_call_for_frontend(id)
            });
            assert_eq!(correlation.is_some(), boundary == 2);
            let mut old_host = fixture.with_fixture(|conn| {
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .take_page_target_for_close("TID-helper-workflow")
                    .unwrap()
            });
            fixture.with_fixture(|conn| {
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                        "TID-helper-workflow".into(),
                        Some("SID-helper-workflow".into()),
                        "about:blank".into(),
                    ))
            });
            drop(gate);
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
                let replacement = fixture.with_fixture(|conn| {
                    loop {
                        let call = conn
                            .try_register_renderer_call_for_owner(
                                &command_owner,
                                id,
                                expected.dispatched_attachment_id(),
                                descriptor.clone(),
                            )
                            .unwrap();
                        if call.correlation().renderer_call_id() == expected.renderer_call_id() {
                            break call;
                        }
                        assert!(
                            call.correlation().renderer_call_id().get()
                                < expected.renderer_call_id().get()
                        );
                        assert!(conn.take_renderer_call_if_correlation_matches_for_owner(
                            &command_owner,
                            call.correlation()
                        ));
                    }
                });
                assert_eq!(
                    replacement.correlation(),
                    expected,
                    "keep all numeric call components identical on the successor host"
                );
            }
            if boundary == 2 {
                let release = old_host.runtime_slot.loaded_page_mut().unwrap()
                    .start_runtime_protocol_message_for_inspector_session(inspector_session, json!({"id":9_000_198,"method":"Runtime.evaluate","params":{"expression":"globalThis.__releaseOriginalRuntime(42)"}}).to_string()).unwrap();
                let completed = release.wait().await.unwrap();
                old_host
                    .runtime_slot
                    .loaded_page_mut()
                    .unwrap()
                    .finish_runtime_protocol_message(completed)
                    .unwrap();
            }
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let completed = fixture.drive_owned_wait(&mut work).await;
                    match fixture.with_fixture(|conn| {
                        conn.complete_automation_command_dispatch(
                            pending.with_completion(completed),
                        )
                    }) {
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
            fixture.with_fixture(|conn| { assert_eq!(
                conn
                    .target_devtools_session_state_for_owner(&command_owner)
                    .unwrap()
                    .renderer_call_for_frontend(id),
                correlation,
                "the old task cannot consume the identically numbered call in the successor host"
            ) });
            let state_owner = fixture.state;
            let response = dispatch_runtime_helper_using(
                &mut ProtocolWorkflowDriver::new(state_owner.handle()), command_owner.clone(), "Runtime.evaluate",
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
                state_owner
                    .with_state(|state| {
                        state.fixture(|connection| {
                            connection.take_renderer_call_if_correlation_matches_for_owner(
                                &command_owner,
                                correlation,
                            )
                        })
                    })
                    .unwrap();
            }
            drop(old_host);
        }
    }

    fn inspect_direct<T: 'static>(
        access: &ProtocolStateHandle,
        operation: impl FnOnce(&mut CdpConnection) -> T + 'static,
    ) -> T {
        access.with_state(|state| state.fixture(operation)).unwrap()
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
            let fixture = fixture().await;
            let owner = fixture.state;
            let access = owner.handle();
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
            let mut peer = ProtocolWorkflowDriver::new(access.clone());
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
        let context = fixture().await;
        let owner = context.state;
        let access = owner.handle();
        let mut driver = ProtocolWorkflowDriver::new(access);
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
        let context = fixture().await;
        let access = context.handle();
        let helper = context
            .with_fixture(|conn| {
                PreparedRuntimeHelper::prepare(
                    conn,
                    CommandOwnerScope::for_route(CdpSessionRoute::Browser),
                    "Runtime.evaluate",
                    json!({"expression":"42","returnByValue":true}),
                )
            })
            .unwrap();
        context.with_fixture(|conn| {
            let browser = conn.browser_context.as_mut().unwrap();
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
        });
        let response = helper
            .run(&mut ProtocolWorkflowDriver::new(access))
            .await
            .response
            .unwrap();
        let BackgroundCommandResponsePayload::Success { result } = response else {
            panic!("the original Page still owns its completed helper request");
        };
        assert_eq!(result["result"]["value"], 42);
        context.with_fixture(|conn| {
            assert_eq!(
                conn.browser_context.as_ref().unwrap().active_target_id(),
                Some("TID-helper-neighbour")
            )
        });
        context.with_fixture(|conn| {
            assert!(!conn.browser_context.as_ref().unwrap().has_loaded_page())
        });
    }

    #[tokio::test]
    async fn losing_a_helper_native_result_retires_its_exact_renderer_call() {
        let context = fixture().await;
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let mut helper = context
            .with_fixture(|conn| {
                PreparedRuntimeHelper::prepare(
                    conn,
                    command_owner.clone(),
                    "Runtime.evaluate",
                    json!({"expression":"42","returnByValue":true}),
                )
            })
            .unwrap();
        let id = helper.command_id;
        let completed = helper.pending.take().unwrap().wait().await.unwrap();
        context.with_fixture(|conn| {
            assert!(
                conn.renderer_runtime_command_cause_for_owner(&command_owner, id)
                    .is_some()
            )
        });
        drop(completed);
        drop(helper);
        context.with_fixture(|conn| {
            assert!(
                conn.renderer_runtime_command_cause_for_owner(&command_owner, id)
                    .is_none()
            )
        });
    }

    #[tokio::test]
    async fn helper_cancellation_rejects_a_recreated_host_with_identical_call_components() {
        let context = fixture().await;
        let helper = context
            .with_fixture(|conn| {
                PreparedRuntimeHelper::prepare(
                    conn,
                    CommandOwnerScope::for_route(CdpSessionRoute::Browser),
                    "Runtime.evaluate",
                    json!({"expression":"42"}),
                )
            })
            .unwrap();
        let identity = helper.cancellation.identity.clone();
        let id = helper.command_id;
        let expected = identity.correlation;
        let old = context.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .unwrap()
                .take_page_target_for_close("TID-helper-workflow")
                .unwrap()
        });
        context.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .unwrap()
                .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                    "TID-helper-workflow".into(),
                    Some("SID-helper-workflow".into()),
                    "about:blank".into(),
                ))
        });
        let descriptor = RendererCommandDescriptor::for_document_helper(
            json!({"id":id,"method":"Runtime.evaluate","params":{"expression":"43"}}).to_string(),
        )
        .unwrap();
        let replacement = context.with_fixture(|conn| {
            conn.try_register_renderer_call_for_owner(
                identity.scope.owner(),
                id,
                expected.dispatched_attachment_id(),
                descriptor,
            )
            .unwrap()
        });
        let replacement = replacement.correlation();
        assert_eq!(
            replacement, expected,
            "force every numeric correlation component to collide"
        );
        context.with_fixture(|conn| assert!(!identity.scope.owns_current_host(conn)));
        drop(helper);
        let current = context.with_fixture(|conn| {
            conn.target_devtools_session_state_for_owner(identity.scope.owner())
                .unwrap()
                .renderer_call_for_frontend(id)
        });
        assert_eq!(current, Some(replacement));
        context.with_fixture(|conn| {
            conn.take_renderer_call_if_correlation_matches_for_owner(
                identity.scope.owner(),
                replacement,
            )
        });
        drop(old);
    }

    #[tokio::test]
    async fn a_closed_binding_cannot_cancel_an_identical_rebound_session_route() {
        let context = fixture().await;
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let helper = context
            .with_fixture(|conn| {
                PreparedRuntimeHelper::prepare(
                    conn,
                    command_owner.clone(),
                    "Runtime.evaluate",
                    json!({"expression":"42"}),
                )
            })
            .unwrap();
        let id = helper.command_id;
        let expected = helper.cancellation.identity.correlation;
        let route =
            context.with_fixture(|conn| conn.session_route(Some("SID-helper-workflow")).unwrap());
        context.with_fixture(|conn| {
            conn.register_session_route_for_test("SID-helper-workflow", route)
        });
        assert!(!helper.cancellation.identity.scope.binding_is_current());
        // Keep every literal route and call-id component identical. The
        // original binding lifetime must still prevent access to its successor.
        drop(helper);
        let current = context.with_fixture(|conn| {
            conn.target_devtools_session_state_for_owner(&command_owner)
                .unwrap()
                .renderer_call_for_frontend(id)
        });
        assert_eq!(current, Some(expected));
        context.with_fixture(|conn| {
            assert!({
                conn.take_renderer_call_if_correlation_matches_for_owner(&command_owner, expected)
            })
        });
    }

    #[tokio::test]
    async fn helper_cancellation_cannot_remove_a_reused_frontend_call_id() {
        let context = fixture().await;
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let helper = context
            .with_fixture(|conn| {
                PreparedRuntimeHelper::prepare(
                    conn,
                    command_owner.clone(),
                    "Runtime.evaluate",
                    json!({"expression":"42"}),
                )
            })
            .unwrap();
        let id = helper.command_id;
        let correlation = helper.cancellation.identity.correlation;
        let expected =
            context.with_fixture(|conn| {
                drop(helper);
                assert!(conn.take_renderer_call_if_correlation_matches_for_owner(
                    &command_owner,
                    correlation
                ));
                let descriptor = RendererCommandDescriptor::for_document_helper(
                    json!({"id":id,"method":"Runtime.evaluate","params":{"expression":"43"}})
                        .to_string(),
                )
                .unwrap();
                let replacement = conn
                    .try_register_renderer_call_for_owner(
                        &command_owner,
                        id,
                        correlation.dispatched_attachment_id(),
                        descriptor,
                    )
                    .unwrap();
                let expected = replacement.correlation();
                assert_ne!(expected.renderer_call_id(), correlation.renderer_call_id());
                expected
            });
        let current = context.with_fixture(|conn| {
            conn.target_devtools_session_state_for_owner(&command_owner)
                .unwrap()
                .renderer_call_for_frontend(id)
        });
        assert_eq!(current, Some(expected));
        context.with_fixture(|conn| {
            assert!({
                conn.take_renderer_call_if_correlation_matches_for_owner(&command_owner, expected)
            })
        });
    }
}
