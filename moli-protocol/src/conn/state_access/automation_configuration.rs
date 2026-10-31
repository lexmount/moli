use super::*;
use crate::{
    automation::{AutomationContext, AutomationResult, DevToolsError},
    domains::{browser, emulation, storage},
};

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_network_data_access(
        self,
        prepared: crate::domains::network::PreparedNetworkDataAccess,
        result: AutomationResult,
    ) -> Result<AutomationResult, DevToolsError> {
        prepared.finish(self.connection, result)
    }

    pub(crate) fn finish_automation_permission(
        self,
        completed: browser::CompletedBrowserCommandDispatch,
    ) -> Result<AutomationResult, DevToolsError> {
        browser::finish_devtools_set_permission_command(self.connection, completed)
    }

    pub(crate) fn finish_automation_emulation(
        self,
        completed: emulation::CompletedEmulationAutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        completed.finish(self.connection)
    }

    pub(crate) fn finish_automation_storage(
        self,
        completed: storage::CompletedStorageCommandDispatch,
    ) -> Result<AutomationResult, DevToolsError> {
        storage::complete_pending_set_cookies_result(self.connection, completed)
            .map(AutomationResult::SetCookies)
    }

    pub(crate) fn finish_automation_output(
        self,
        command: &AutomationContext,
        scope: Option<&crate::conn::ProtocolPageHostScope>,
        context: &mut CommandDispatchContext,
    ) {
        if let Some(scope) = scope {
            if !scope.is_current(self.connection) || self.connection.has_pending_javascript_dialog()
            {
                return;
            }
            crate::domains::activity::project_protocol_local_command_outputs(
                self.connection,
                scope.owner(),
                context,
            );
        } else {
            let mut events = Vec::new();
            self.connection
                .project_protocol_local_outputs_for_direct_command(command, context, &mut events);
            context.protocol_events_mut().extend(events);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::*;
    use crate::conn::{
        AutomationCommandTaskStep, BrowserContext, CommandOwnerScope, ProtocolStateOwner,
    };
    use crate::testing::TestContext;
    use serde_json::json;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-configuration".into());
        browser.set_active_target_id("TID-configuration");
        browser.attach_active_session("SID-configuration");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-configuration"))
            .await;
        context
    }

    fn command_context() -> AutomationContext {
        AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some("TID-configuration".into()),
            browser_context_id: None,
        }
    }

    fn permission_command() -> AutomationCommand {
        AutomationCommand::SetPermission(DevToolsSetPermissionCommand {
            context: command_context(),
            permission: json!({"name":"geolocation"}),
            setting: "granted".into(),
            origin: "https://configuration.example".into(),
            embedded_origin: None,
            browser_context_id: None,
        })
    }

    fn cookie_command() -> AutomationCommand {
        AutomationCommand::SetCookies(DevToolsSetCookiesCommand {
            context: command_context(),
            scope: Default::default(),
            cookies: vec![DevToolsCookieParam {
                name: "owned".into(),
                value: "yes".into(),
                url: Some("https://configuration.example/".into()),
                domain: None,
                path: None,
                secure: None,
                http_only: false,
                same_site: None,
                priority: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
                partition_key_opaque: None,
                expires: None,
            }],
        })
    }

    fn window_command() -> AutomationCommand {
        AutomationCommand::SetWindowState(DevToolsSetWindowStateCommand {
            context: command_context(),
            state: DevToolsWindowState::Minimized,
        })
    }

    #[tokio::test]
    async fn configuration_families_keep_typed_results_without_a_mailbox() {
        let context = fixture().await;
        let owner = context.state;
        let handle = owner.handle();
        for command in [permission_command(), cookie_command(), window_command()] {
            let mut step = handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(command)
                })
                .unwrap();
            let result = loop {
                match step {
                    AutomationCommandTaskStep::Pending(pending) => {
                        // Synchronous state access is available while this
                        // command's owned renderer wait is retained elsewhere.
                        assert!(
                            handle
                                .with_state(|state| state
                                    .connection
                                    .has_browser_context_id("BID-configuration"))
                                .unwrap()
                        );
                        let completed = pending.wait().await;
                        step = handle
                            .with_state(move |state| {
                                state
                                    .connection
                                    .complete_automation_command_dispatch(completed)
                            })
                            .unwrap();
                    }
                    AutomationCommandTaskStep::Complete(outcome) => {
                        break outcome.into_result_and_owner_turn().0.unwrap();
                    }
                }
            };
            assert!(matches!(
                result,
                AutomationResult::Empty | AutomationResult::SetCookies(_)
            ));
        }
        handle
            .with_state(|state| {
                let context = state.connection.browser_context.as_ref().unwrap();
                assert!(
                    context
                        .active_page_target()
                        .owner_state
                        .window_document_hidden()
                );
                assert_eq!(context.snapshot_cookies().unwrap().len(), 1);
            })
            .unwrap();
    }

    #[tokio::test]
    async fn configuration_finish_releases_connection_and_preserves_original_result() {
        for command in [permission_command(), cookie_command(), window_command()] {
            let mut context = fixture().await;
            let AutomationCommandTaskStep::Pending(pending) =
                context.with_fixture(|conn| conn.start_automation_command_dispatch(command))
            else {
                panic!("configuration uses its ordinary owned workflow");
            };
            let completed = context.drive_owned_wait(pending.wait()).await;
            let command = AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
                context: command_context(),
                filter: None,
                root: None,
                max_depth: None,
            });
            let (result, _) = context.with_fixture(|conn| {
                crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(conn, command)
            });
            assert!(matches!(result.unwrap(), AutomationResult::GetTargets(_)));
            let AutomationCommandTaskStep::Complete(outcome) =
                context.with_fixture(|conn| conn.complete_automation_command_dispatch(completed))
            else {
                panic!("the final state commit is finite");
            };
            assert!(outcome.into_result_and_owner_turn().0.is_ok());
        }
    }

    #[tokio::test]
    async fn configuration_late_client_window_tail_cannot_mutate_a_recreated_host() {
        let original = fixture().await;
        let replacement = fixture().await;
        let prepared = original
            .with_fixture(|conn| {
                emulation::prepare_emulation_automation_command(
                    conn,
                    AutomationCommand::SetClientWindowState(DevToolsSetClientWindowStateCommand {
                        context: command_context(),
                        client_window: "TID-configuration".into(),
                        state: DevToolsWindowState::Minimized,
                        width: Some(777),
                        height: Some(888),
                        x: Some(9),
                        y: Some(10),
                    }),
                )
            })
            .unwrap();
        let completed = prepared.wait().await;
        let expected_geometry = replacement.with_fixture(|conn| {
            conn.browser_context
                .as_ref()
                .unwrap()
                .active_page_target()
                .owner_state
                .window_surface_geometry
        });
        let browser = replacement.with_fixture(|conn| conn.browser_context.take().unwrap());
        original.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        let error = original
            .with_fixture(|conn| completed.finish(conn))
            .unwrap_err();
        assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
        original.with_fixture(|conn| {
            let state = {
                &conn
                    .browser_context
                    .as_ref()
                    .unwrap()
                    .active_page_target()
                    .owner_state
            };
            assert!(!state.window_document_hidden());
            assert_eq!(state.window_surface_geometry, expected_geometry);
        });
    }
    #[tokio::test]
    async fn configuration_cookie_default_url_stays_with_original_target_after_activation() {
        let mut context = fixture().await;
        context
            .install_buffered_navigation_fixture_for_session_owner(
                url::Url::parse("https://cookie-original.example/page").unwrap(),
                "<!doctype html><p>original</p>".into(),
                Some("SID-configuration"),
            )
            .await;
        let AutomationCommand::SetCookies(mut command) = cookie_command() else {
            unreachable!()
        };
        command.cookies[0].url = None;
        let prepared = context.with_fixture(|conn| {
            storage::prepare_storage_automation_command(
                conn,
                AutomationCommand::SetCookies(command),
            )
        });
        let storage::PreparedStorageAutomationCommand::SetCookies(pending) = prepared else {
            panic!("a live document cookie owner is queried asynchronously");
        };
        let completed = pending.wait().await;
        context.with_fixture(|conn| {
            let browser = conn.browser_context.as_mut().unwrap();
            browser.insert_page_target_host(crate::conn::PageTargetHost::with_url(
                "TID-selected-later".into(),
                None,
                "https://cookie-other.example/".into(),
            ));
            browser.set_active_target_id("TID-selected-later");
        });
        context.with_fixture(|conn| {
            assert!(storage::complete_pending_set_cookies_result(conn, completed).is_ok())
        });
        context.with_fixture(|conn| {
            let browser = conn.browser_context.as_ref().unwrap();
            let cookies = browser.snapshot_cookies().unwrap();
            assert_eq!(cookies.len(), 1);
            assert_eq!(cookies[0].domain, "cookie-original.example");
            assert_eq!(browser.active_target_id(), Some("TID-selected-later"));
        });
    }

    #[tokio::test]
    async fn configuration_cookie_ready_receipt_cannot_write_into_a_recreated_context() {
        let original = fixture().await;
        let replacement = fixture().await;
        let prepared = original.with_fixture(|conn| {
            storage::prepare_storage_automation_command(conn, cookie_command())
        });
        let storage::PreparedStorageAutomationCommand::SetCookies(pending) = prepared else {
            panic!("a live document cookie owner is queried asynchronously");
        };
        let completed = pending.wait().await;
        let browser = replacement.with_fixture(|conn| conn.browser_context.take().unwrap());
        original.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        let error = original
            .with_fixture(|conn| storage::complete_pending_set_cookies_result(conn, completed))
            .unwrap_err();
        assert_eq!(error.kind, DevToolsErrorKind::Internal);
        assert!(error.message.contains("NoDocumentLoaded"));
        original.with_fixture(|conn| {
            assert!({
                conn.browser_context
                    .as_ref()
                    .unwrap()
                    .snapshot_cookies()
                    .unwrap()
                    .is_empty()
            })
        });
    }
    fn input_command() -> AutomationCommand {
        AutomationCommand::DispatchKeyEvent(DevToolsDispatchKeyEventCommand {
            context: command_context(),
            event_type: DevToolsKeyEventType::KeyPress,
            key: "a".into(),
            code: "KeyA".into(),
            text: "a".into(),
            modifiers: 0,
            auto_repeat: false,
            should_insert_text: true,
        })
    }

    fn interception_command() -> AutomationCommand {
        AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
            context: command_context(),
            intercept_id: "INTERCEPT-owned".into(),
            phases: vec![DevToolsNetworkInterceptPhase::BeforeRequestSent],
            url_patterns: Vec::new(),
        })
    }

    fn target_creation_command() -> AutomationCommand {
        AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: Some("BID-configuration".into()),
            },
            url: "about:blank".into(),
            browser_context_id: Some("BID-configuration".into()),
            activate: false,
        })
    }

    async fn finish_owned_automation(
        handle: &crate::conn::ProtocolStateHandle,
        step: AutomationCommandTaskStep,
    ) -> Result<AutomationResult, DevToolsError> {
        let (result, output) = finish_owned_automation_output(handle, step).await;
        let (events, after, _, post, _, _) = output.into_renderer_owner_turn_parts();
        assert!(
            events
                .iter()
                .chain(&after)
                .chain(&post)
                .all(|event| event.protocol_message_id().is_none()),
            "internal terminals cannot escape the original automation owner"
        );
        result
    }

    async fn finish_owned_automation_output(
        handle: &crate::conn::ProtocolStateHandle,
        mut step: AutomationCommandTaskStep,
    ) -> (
        Result<AutomationResult, DevToolsError>,
        crate::conn::CdpRendererOwnerTurnOutcome,
    ) {
        loop {
            match step {
                AutomationCommandTaskStep::Pending(pending) => {
                    let completed = pending.wait().await;
                    step = handle
                        .with_state(move |state| {
                            state
                                .connection
                                .complete_automation_command_dispatch(completed)
                        })
                        .unwrap();
                }
                AutomationCommandTaskStep::Complete(output) => {
                    return output.into_result_and_owner_turn();
                }
            }
        }
    }

    fn preload_command(contexts: &[&str]) -> AutomationCommand {
        AutomationCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
            context: AutomationContext {
                target_id: None,
                ..command_context()
            },
            source: DevToolsPreloadScriptSource::RawScript(
                "globalThis.__ownedPreloadCount = (globalThis.__ownedPreloadCount || 0) + 1".into(),
            ),
            world_name: None,
            target_ids: None,
            browser_context_ids: contexts
                .iter()
                .map(|id| DevToolsBrowserContextId::from(*id))
                .collect(),
            run_immediately: false,
            include_command_line_api: false,
        })
    }

    async fn preload_counter(
        renderer: &mut OwnedAutomationRenderer,
        handle: &crate::conn::ProtocolStateHandle,
        target: &str,
    ) -> serde_json::Value {
        let result = renderer
            .run(
                handle,
                AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                    context: AutomationContext {
                        target_id: Some(target.into()),
                        ..command_context()
                    },
                    realm_id: None,
                    world_name: None,
                    expression: "globalThis.__ownedPreloadCount ?? null".into(),
                    await_promise: false,
                    user_gesture: false,
                    webdriver_bidi_file_prompt_handler: None,
                    result_ownership: DevToolsResultOwnership::None,
                    preserve_remote_metadata: false,
                    serialization_options: None,
                    materialize_bidi_script_result: false,
                }),
            )
            .await
            .unwrap();
        let AutomationResult::Script(result) = result else {
            panic!("Native script value")
        };
        let DevToolsScriptResult::Value(result) = *result else {
            panic!("preload counter cannot throw")
        };
        result.value
    }

    #[tokio::test]
    async fn automation_preload_registry_spans_contexts_and_removes_future_scripts() {
        tokio::task::LocalSet::new()
            .run_until(Box::pin(async {
                let mut context = fixture().await;
                let mut secondary = BrowserContext::new("BID-preload-secondary".into());
                secondary.set_active_target_id("TID-preload-secondary");
                secondary.attach_active_session("SID-preload-secondary");
                context.with_fixture(|conn| {
                    conn.push_inactive_browser_context_fixture_for_test(secondary)
                });
                context
                    .install_navigation_fixture_for_session_owner(
                        "about:blank",
                        Some("SID-preload-secondary"),
                    )
                    .await;
                let mut renderer = OwnedAutomationRenderer {
                    receiver: context.take_renderer_transport_for_owned_state_test(),
                    pending: Default::default(),
                };
                let owner = context.state;
                let handle = owner.handle();
                let step = handle
                    .with_state(|state| {
                        state
                            .connection
                            .start_automation_command_dispatch(preload_command(&[
                                "BID-configuration",
                                "BID-preload-secondary",
                            ]))
                    })
                    .unwrap();
                assert!(matches!(&step, AutomationCommandTaskStep::Pending(_)));
                let query = handle
                    .with_state(|state| {
                        state.connection.start_automation_command_dispatch(
                            AutomationCommand::GetBrowserContexts(
                                DevToolsGetBrowserContextsCommand {
                                    context: AutomationContext {
                                        target_id: None,
                                        ..command_context()
                                    },
                                },
                            ),
                        )
                    })
                    .unwrap();
                assert!(
                    matches!(&query, AutomationCommandTaskStep::Complete(_)),
                    "finite admission does not wait for the retained preload task"
                );
                let AutomationResult::GetBrowserContexts(contexts) =
                    renderer.finish_step(&handle, query).await.unwrap()
                else {
                    panic!("typed context inventory")
                };
                assert_eq!(contexts.browser_context_ids.len(), 2);
                let AutomationResult::AddPreloadScript(added) =
                    renderer.finish_step(&handle, step).await.unwrap()
                else {
                    panic!("one preload id covers both registries")
                };
                let targets = ["TID-configuration", "TID-preload-secondary"];
                for target in targets {
                    renderer
                        .run(
                            &handle,
                            AutomationCommand::Navigate(DevToolsNavigateCommand {
                                context: AutomationContext {
                                    target_id: Some(target.into()),
                                    ..command_context()
                                },
                                url: "data:text/html,<main>with-preload</main>".into(),
                                referrer: None,
                                wait: DevToolsNavigationWait::DocumentInstalled,
                            }),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        preload_counter(&mut renderer, &handle, target).await,
                        json!(1)
                    );
                }
                renderer
                    .run(
                        &handle,
                        AutomationCommand::RemovePreloadScript(
                            DevToolsRemovePreloadScriptCommand {
                                context: AutomationContext {
                                    target_id: None,
                                    ..command_context()
                                },
                                script_id: added.script_id,
                            },
                        ),
                    )
                    .await
                    .unwrap();
                for target in targets {
                    renderer
                        .run(
                            &handle,
                            AutomationCommand::Navigate(DevToolsNavigateCommand {
                                context: AutomationContext {
                                    target_id: Some(target.into()),
                                    ..command_context()
                                },
                                url: "data:text/html,<main>after-removal</main>".into(),
                                referrer: None,
                                wait: DevToolsNavigationWait::DocumentInstalled,
                            }),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        preload_counter(&mut renderer, &handle, target).await,
                        serde_json::Value::Null
                    );
                }
            }))
            .await;
    }

    #[tokio::test]
    async fn automation_preload_late_document_cannot_redirect_mutation() {
        tokio::task::LocalSet::new()
            .run_until(Box::pin(async {
                let mut context = fixture().await;
                let mut renderer = OwnedAutomationRenderer {
                    receiver: context.take_renderer_transport_for_owned_state_test(),
                    pending: Default::default(),
                };
                let owner = context.state;
                let handle = owner.handle();
                let step = handle
                    .with_state(|state| {
                        state
                            .connection
                            .start_automation_command_dispatch(preload_command(&[
                                "BID-configuration",
                            ]))
                    })
                    .unwrap();
                let replacement = renderer
                    .run(
                        &handle,
                        AutomationCommand::Navigate(DevToolsNavigateCommand {
                            context: command_context(),
                            url: "data:text/html,<main>replacement-with-desired-preload</main>"
                                .into(),
                            referrer: None,
                            wait: DevToolsNavigationWait::DocumentInstalled,
                        }),
                    )
                    .await
                    .unwrap();
                assert!(matches!(replacement, AutomationResult::Navigate(_)));
                assert_eq!(
                    preload_counter(&mut renderer, &handle, "TID-configuration").await,
                    json!(1)
                );
                let error = renderer.finish_step(&handle, step).await.unwrap_err();
                assert_eq!(error.kind, DevToolsErrorKind::Internal);
                assert_eq!(error.message, crate::conn::NoDocumentLoaded.to_string());
                assert_eq!(
                    preload_counter(&mut renderer, &handle, "TID-configuration").await,
                    json!(1)
                );
            }))
            .await;
    }

    #[tokio::test]
    async fn automation_preload_accepted_target_script_survives_cancel_and_ready_loss() {
        for discard_ready in [false, true] {
            tokio::task::LocalSet::new()
                .run_until(Box::pin(async {
                    let mut context = fixture().await;
                    let mut renderer = OwnedAutomationRenderer {
                        receiver: context.take_renderer_transport_for_owned_state_test(),
                        pending: Default::default(),
                    };
                    let owner = context.state;
                    let handle = owner.handle();
                    let AutomationCommand::AddPreloadScript(mut command) = preload_command(&[])
                    else {
                        unreachable!()
                    };
                    command.context = command_context();
                    command.run_immediately = true;
                    let AutomationCommandTaskStep::Pending(mut pending) = handle
                        .with_state(|state| {
                            state.connection.start_automation_command_dispatch(
                                AutomationCommand::AddPreloadScript(command),
                            )
                        })
                        .unwrap()
                    else {
                        panic!("the script retains its original Native wait")
                    };
                    if discard_ready {
                        drop(pending.take_wait().wait().await);
                    }
                    let recovery = handle
                        .with_state(|state| {
                            state.connection.fail_automation_command_wait(
                                *pending,
                                crate::OwnedProtocolWaitFailure::Cancelled,
                            )
                        })
                        .unwrap();
                    let AutomationResult::AddPreloadScript(result) =
                        renderer.finish_step(&handle, recovery).await.unwrap()
                    else {
                        panic!("the admitted script retains its original result")
                    };
                    assert!(result.script_id.as_str().starts_with("TID-configuration:"));
                    assert_eq!(
                        preload_counter(&mut renderer, &handle, "TID-configuration").await,
                        json!(1)
                    );
                    handle
                        .with_state(|state| {
                            let owner = state
                                .connection
                                .command_owner_scope_for_devtools_context(&command_context())
                                .unwrap();
                            assert_eq!(
                                state
                                    .connection
                                    .target_owner_state_for_owner(&owner)
                                    .unwrap()
                                    .document_start_scripts
                                    .len(),
                                1
                            );
                        })
                        .unwrap();
                }))
                .await;
        }
    }

    #[tokio::test]
    async fn automation_page_navigation_runs_direct_and_keeps_history_results() {
        tokio::task::LocalSet::new()
            .run_until(Box::pin(async {
                let mut context = fixture().await;
                let receiver = context.take_renderer_transport_for_owned_state_test();
                let mut renderer = OwnedAutomationRenderer {
                    receiver,
                    pending: Default::default(),
                };
                let owner = context.state;
                let handle = owner.handle();
                let mut last_navigation = None;
                for marker in ["alpha-owned", "beta-owned"] {
                    let url = format!("data:text/html,<main>{marker}</main>");
                    let command = AutomationCommand::Navigate(DevToolsNavigateCommand {
                        context: command_context(),
                        url: url.clone(),
                        referrer: None,
                        wait: DevToolsNavigationWait::DocumentInstalled,
                    });
                    let result = renderer.run(&handle, command).await.unwrap();
                    let AutomationResult::Navigate(result) = result else {
                        panic!("the original navigation owner keeps its typed result");
                    };
                    assert_eq!(result.url, url);
                    assert!(result.frame_id.is_none() && result.loader_id.is_none());
                    assert!(result.navigation_id.is_some());
                    assert_ne!(result.navigation_id, last_navigation);
                    last_navigation = result.navigation_id;
                }
                let result = renderer
                    .run(
                        &handle,
                        AutomationCommand::Reload(DevToolsReloadCommand {
                            context: command_context(),
                            ignore_cache: false,
                            script_to_evaluate_on_load: None,
                            wait: DevToolsNavigationWait::DocumentInstalled,
                        }),
                    )
                    .await
                    .unwrap();
                let AutomationResult::Navigate(reloaded) = result else {
                    panic!("reload returns its own typed navigation");
                };
                assert_ne!(reloaded.navigation_id, last_navigation);
                assert_eq!(reloaded.url, "data:text/html,<main>beta-owned</main>");
                let fragment = renderer
                    .run(
                        &handle,
                        AutomationCommand::Navigate(DevToolsNavigateCommand {
                            context: command_context(),
                            url: format!("{}#same-document", reloaded.url),
                            referrer: None,
                            wait: DevToolsNavigationWait::DocumentInstalled,
                        }),
                    )
                    .await
                    .unwrap();
                let AutomationResult::Navigate(fragment) = fragment else {
                    panic!("fragment navigation retains its original result kind");
                };
                assert!(fragment.navigation_id.is_none());
                for same_document in [true, false] {
                    let result = renderer
                        .run(
                            &handle,
                            AutomationCommand::TraverseHistory(DevToolsTraverseHistoryCommand {
                                context: command_context(),
                                destination: DevToolsHistoryTraversalDestination::Delta(-1),
                                wait: DevToolsNavigationWait::DocumentInstalled,
                            }),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        result,
                        AutomationResult::TraverseHistory(DevToolsTraverseHistoryResult {
                            same_document
                        })
                    );
                    // The Native reply accepts a queued traversal. Consume its
                    // actual history publication before resolving another delta.
                    renderer
                        .wait_for_history_url(
                            &handle,
                            if same_document {
                                &reloaded.url
                            } else {
                                "data:text/html,<main>alpha-owned</main>"
                            },
                        )
                        .await;
                }
                let source = renderer
                    .run(
                        &handle,
                        AutomationCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
                            context: command_context(),
                            reference: None,
                            include_shadow_dom: false,
                        }),
                    )
                    .await
                    .unwrap();
                let AutomationResult::GetOuterHtml(source) = source else {
                    panic!("the restored document remains independently queryable");
                };
                assert!(source.outer_html.contains("alpha-owned"));
                assert!(!source.outer_html.contains("beta-owned"));
            }))
            .await;
    }

    #[tokio::test]
    async fn automation_navigation_completes_on_its_synchronous_owner() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut context = fixture().await;
                let url = "data:text/html,<main>owned-navigation</main>";
                let command = AutomationCommand::Navigate(DevToolsNavigateCommand {
                    context: command_context(),
                    url: url.into(),
                    referrer: None,
                    wait: DevToolsNavigationWait::DocumentInstalled,
                });
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    Box::pin(context.execute_owned_automation_command_for_test(command)),
                )
                .await
                .expect("navigation and its renderer fence complete on the synchronous owner");
                let (result, output) = output.into_result_and_owner_turn();
                let (messages, scheduler_events) = context.route_completed_command_outcome_for_test(output).await;
                context.sent.extend(messages);
                context.route_direct_command_output_for_test(Vec::new(), scheduler_events).await;
                let AutomationResult::Navigate(result) = result.unwrap() else {
                    panic!("the shared navigation completion preserves its typed result");
                };
                assert_eq!(result.url, url);
                assert!(result.navigation_id.is_some());
                assert!(
                    context.sent.iter().all(|message| message.get("id").is_none()),
                    "a private adapter reply cannot leak through the owner or an interleaved renderer turn"
                );
            })
            .await;
    }

    struct OwnedAutomationRenderer {
        receiver: moli_core::RendererOutputTransportReceiver,
        pending: futures_util::stream::FuturesUnordered<
            futures_util::future::LocalBoxFuture<
                'static,
                crate::conn::CompletedProtocolSchedulerWork,
            >,
        >,
    }

    impl OwnedAutomationRenderer {
        fn admit_scheduler_events(
            &mut self,
            handle: &crate::conn::ProtocolStateHandle,
            events: Vec<crate::conn::CdpSchedulerEvent>,
        ) {
            use crate::conn::CdpSchedulerEvent;
            use futures_util::FutureExt as _;
            let mut events = std::collections::VecDeque::from(events);
            while let Some(event) = events.pop_front() {
                let pending = match event {
                    CdpSchedulerEvent::RendererOutputProjectionPrepared { pending } => {
                        Some(pending)
                    }
                    CdpSchedulerEvent::ProtocolWorkPublished { work } => handle
                        .with_state(|state| {
                            match state
                                .connection
                                .try_start_owned_protocol_scheduler_work(work)
                            {
                                Ok(pending) => pending,
                                Err(work) => {
                                    let output = state
                                        .connection
                                        .try_complete_protocol_scheduler_work_turn(*work)
                                        .unwrap_or_else(|_| {
                                            panic!("fixture observation is finite")
                                        });
                                    events.extend(output.into_protocol_event_parts().1);
                                    None
                                }
                            }
                        })
                        .unwrap(),
                    CdpSchedulerEvent::PageScreencastStarted { .. } => {
                        panic!("navigation fixture does not request a screencast")
                    }
                };
                if let Some(pending) = pending {
                    self.pending.push(pending.wait().boxed_local());
                }
            }
        }

        async fn advance(&mut self, handle: &crate::conn::ProtocolStateHandle) {
            use futures_util::StreamExt as _;
            let output = tokio::select! {
                Some(completed) = self.pending.next(), if !self.pending.is_empty() => {
                    handle.with_state(|state| {
                        state.connection.complete_owned_protocol_scheduler_work(completed)
                    }).unwrap()
                }
                publication = self.receiver.recv() => {
                    let publication = publication.expect("fixture Native transport stays open");
                    handle.with_state(|state| {
                        state.connection.admit_renderer_output(publication)

                    }).unwrap()
                }
            };
            self.admit_scheduler_events(handle, output.into_protocol_event_parts().1);
        }

        async fn project_fence(
            &mut self,
            handle: &crate::conn::ProtocolStateHandle,
            fence: moli_core::RendererOutputFence,
        ) {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !handle
                    .with_state(|state| {
                        state
                            .connection
                            .renderer_output_cursor_is_projected(fence.cursor())
                    })
                    .unwrap()
                {
                    self.advance(handle).await;
                }
            })
            .await
            .expect("the exact command fence must be projected");
        }

        async fn wait_for_history_url(
            &mut self,
            handle: &crate::conn::ProtocolStateHandle,
            url: &str,
        ) {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !handle
                    .with_state(|state| {
                        let owner = state
                            .connection
                            .command_owner_scope_for_devtools_context(&command_context())
                            .unwrap();
                        let (index, entries) = state
                            .connection
                            .target_session_owner_navigation_history_snapshot_for_owner(&owner)
                            .unwrap();
                        entries[index].url == url
                    })
                    .unwrap()
                {
                    self.advance(handle).await;
                }
            })
            .await
            .expect("the queued traversal must publish its actual history update");
        }

        async fn run(
            &mut self,
            handle: &crate::conn::ProtocolStateHandle,
            command: AutomationCommand,
        ) -> Result<AutomationResult, DevToolsError> {
            let step = handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(command)
                })
                .unwrap();
            self.finish_step(handle, step).await
        }

        async fn finish_step(
            &mut self,
            handle: &crate::conn::ProtocolStateHandle,
            step: AutomationCommandTaskStep,
        ) -> Result<AutomationResult, DevToolsError> {
            let (result, output) = finish_owned_automation_output(handle, step).await;
            let (events, after, boundary, post, scheduler, predecessor) =
                output.into_renderer_owner_turn_parts();
            assert!(
                events
                    .iter()
                    .chain(&after)
                    .chain(&post)
                    .all(|event| event.protocol_message_id().is_none()),
                "internal terminals cannot escape the original automation owner"
            );
            self.admit_scheduler_events(handle, scheduler);
            for fence in predecessor.into_iter().chain(boundary) {
                self.project_fence(handle, fence).await;
            }
            result
        }
    }

    async fn run_owned_automation_for_test(
        handle: &crate::conn::ProtocolStateHandle,
        command: AutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        let step = handle
            .with_state(move |state| state.connection.start_automation_command_dispatch(command))
            .unwrap();
        finish_owned_automation(handle, step).await
    }

    #[tokio::test]
    async fn automation_navigation_ready_loss_keeps_original_loader_after_replacement() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let context = fixture().await;
                let owner = context.state;
                let handle = owner.handle();
                let command = AutomationCommand::Navigate(DevToolsNavigateCommand {
                    context: command_context(),
                    url: "data:text/html,<main>original-ready-navigation</main>".into(),
                    referrer: None,
                    wait: DevToolsNavigationWait::None,
                });
                let AutomationCommandTaskStep::Pending(mut pending) = handle
                    .with_state(move |state| {
                        state.connection.start_automation_command_dispatch(command)
                    })
                    .unwrap()
                else {
                    panic!("the original owner retains its pending receipt");
                };
                drop(pending.take_wait().wait().await);
                let original_loader = handle
                    .with_state(|state| {
                        let owner = state
                            .connection
                            .command_owner_scope_for_devtools_context(&command_context())
                            .unwrap();
                        state
                            .connection
                            .current_document_loader_id_for_owner(&owner)
                            .unwrap()
                    })
                    .unwrap();
                let replacement = run_owned_automation_for_test(
                    &handle,
                    AutomationCommand::Navigate(DevToolsNavigateCommand {
                        context: command_context(),
                        url: "data:text/html,<main>replacement-navigation</main>".into(),
                        referrer: None,
                        wait: DevToolsNavigationWait::DocumentInstalled,
                    }),
                )
                .await
                .unwrap();
                let AutomationResult::Navigate(replacement) = replacement else {
                    panic!("the replacement completes while the old ready result is lost");
                };
                let expected = webdriver_bidi_navigation_id_from_loader_id(&original_loader);
                assert_ne!(replacement.navigation_id.as_ref(), Some(&expected));
                let step = handle
                    .with_state(move |state| {
                        state.connection.fail_automation_command_wait(
                            *pending,
                            crate::OwnedProtocolWaitFailure::Cancelled,
                        )
                    })
                    .unwrap();
                let AutomationResult::Navigate(original) =
                    finish_owned_automation(&handle, step).await.unwrap()
                else {
                    panic!("recovery must preserve the accepted original terminal");
                };
                assert_eq!(original.navigation_id, Some(expected));
                assert_eq!(
                    original.url,
                    "data:text/html,<main>original-ready-navigation</main>"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn automation_page_inventory_handles_distinct_native_streams_without_mailbox() {
        let context = fixture().await;
        let owner = context.state;
        let handle = owner.handle();
        let step = handle
            .with_state(|state| {
                state
                    .connection
                    .start_automation_command_dispatch(target_creation_command())
            })
            .unwrap();
        let AutomationResult::CreateTarget(created) =
            finish_owned_automation(&handle, step).await.unwrap()
        else {
            panic!("the second Page has its own Native stream");
        };
        let step = handle
            .with_state(|state| {
                state.connection.start_automation_command_dispatch(
                    AutomationCommand::GetFrameTrees(DevToolsGetFrameTreesCommand {
                        context: AutomationContext {
                            target_id: None,
                            ..command_context()
                        },
                        max_depth: None,
                    }),
                )
            })
            .unwrap();
        let AutomationResult::GetFrameTrees(trees) =
            finish_owned_automation(&handle, step).await.unwrap()
        else {
            panic!("global frame inventory retains its typed Page results");
        };
        assert_eq!(trees.frame_trees.len(), 2);
        let targets = trees
            .frame_trees
            .iter()
            .map(|tree| tree.frame_tree["frame"]["id"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            targets,
            std::collections::BTreeSet::from(["TID-configuration", created.target_id.as_str()])
        );
        let step = handle
            .with_state(|state| {
                state.connection.start_automation_command_dispatch(
                    AutomationCommand::GetLayoutMetrics(DevToolsGetLayoutMetricsCommand {
                        context: command_context(),
                    }),
                )
            })
            .unwrap();
        assert!(matches!(
            finish_owned_automation(&handle, step).await.unwrap(),
            AutomationResult::LayoutMetrics(_)
        ));
    }

    #[tokio::test]
    async fn automation_dom_child_queries_use_direct_state_and_keep_frame_projection() {
        let mut context = fixture().await;
        context.process_async(json!({
            "id": 8099, "method": "Page.navigate", "sessionId": "SID-configuration",
            "params": { "url": "data:text/html,<iframe srcdoc=\"<main id='inside'>child-owned</main>\"></iframe>" }
        })).await;
        let loader = context
            .sent
            .iter()
            .find(|message| message["id"] == 8099)
            .unwrap()["result"]["loaderId"]
            .as_str()
            .expect("cross-document navigation loader")
            .to_owned();
        crate::testing::wait_until_renderer_document_load(
            &mut context,
            Some("SID-configuration"),
            "TID-configuration",
            &loader,
        )
        .await;
        context.process_async(json!({"id": 8100, "method": "Page.getFrameTree", "sessionId": "SID-configuration"})).await;
        let child = context
            .sent
            .iter()
            .find(|message| message["id"] == 8100)
            .unwrap()["result"]["frameTree"]["childFrames"][0]["frame"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        context.take_all();
        let owner = context.state;
        let handle = owner.handle();
        let target = AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some(child.into()),
            browser_context_id: None,
        };
        let query_context = target.clone();
        let step = handle
            .with_state(move |state| {
                state.connection.start_automation_command_dispatch(
                    AutomationCommand::QuerySelector(DevToolsQuerySelectorCommand {
                        context: query_context,
                        root: None,
                        selector: "#inside".into(),
                        multiple: false,
                    }),
                )
            })
            .unwrap();
        let AutomationResult::QuerySelector(nodes) = finish_owned_automation(&handle, step)
            .await
            .expect("query the loaded child document")
        else {
            panic!("child DOM query retains its typed result");
        };
        assert_eq!(nodes.node_ids.len(), 1);
        let html_context = target.clone();
        let step =
            handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(
                        AutomationCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
                            context: html_context,
                            reference: None,
                            include_shadow_dom: false,
                        }),
                    )
                })
                .unwrap();
        let AutomationResult::GetOuterHtml(html) = finish_owned_automation(&handle, step)
            .await
            .expect("read the original child document source")
        else {
            panic!("child DOM source retains its typed result");
        };
        assert!(html.outer_html.contains("child-owned"));
        let missing_context = target.clone();
        let missing =
            handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(
                        AutomationCommand::ResolveNode(DevToolsResolveNodeCommand {
                            context: missing_context,
                            reference: DevToolsDomNodeReference::FrontendNodeId(999_999),
                            execution_context_id: None,
                            object_group: None,
                        }),
                    )
                })
                .unwrap();
        let missing = finish_owned_automation(&handle, missing)
            .await
            .expect_err("unknown child frontend id must miss in the renderer");
        assert_eq!(missing.kind, DevToolsErrorKind::NoSuchNode);
        assert_eq!(missing.message, "Could not find node with given id");
        let step =
            handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(
                        AutomationCommand::ResolveNode(DevToolsResolveNodeCommand {
                            context: target,
                            reference: DevToolsDomNodeReference::FrontendNodeId(nodes.node_ids[0]),
                            execution_context_id: None,
                            object_group: Some("owned-dom".into()),
                        }),
                    )
                })
                .unwrap();
        let AutomationResult::ResolveNode(node) = finish_owned_automation(&handle, step)
            .await
            .expect("resolve the original child frontend binding")
        else {
            panic!("the original child node is resolved by its own renderer");
        };
        assert_eq!(node.object["subtype"], "node");
        assert!(node.object["objectId"].is_string());
    }

    #[tokio::test]
    async fn automation_dom_admission_rejects_a_recreated_host_with_reused_literal_ids() {
        let context = fixture().await;
        let original = context.with_fixture(|conn| {
            conn.target_page_residence_identity_for_session(Some("SID-configuration"))
                .unwrap()
        });
        let mut replacement = fixture().await;
        replacement
            .install_navigation_fixture_for_session_owner(
                "data:text/html,<body>replacement-dom-owner</body>",
                Some("SID-configuration"),
            )
            .await;
        replacement.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .unwrap()
                .active_page_target_mut()
                .runtime_slot
                .set_page_attachment_id_for_test(original.page_attachment_id().get())
        });
        replacement.with_fixture(|conn| {
            assert_eq!(
                conn.target_page_residence_identity_for_session(Some("SID-configuration"))
                    .unwrap(),
                original,
                "the fixture must reuse the original literal context, target and attachment"
            )
        });
        let owner = context.state;
        let handle = owner.handle();
        let command = || {
            AutomationCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
                context: command_context(),
                reference: None,
                include_shadow_dom: false,
            })
        };
        let old_command = command();
        let step = handle
            .with_state(move |state| {
                state
                    .connection
                    .start_automation_command_dispatch(old_command)
            })
            .unwrap();
        let browser = replacement.with_fixture(|conn| conn.browser_context.take().unwrap());
        handle
            .with_state(move |state| {
                state
                    .connection
                    .install_browser_context_fixture_for_test(browser);
            })
            .unwrap();
        assert!(
            finish_owned_automation(&handle, step).await.is_err(),
            "an admitted query cannot read the replacement Page even when literal ids are reused"
        );
        let next = command();
        let step = handle
            .with_state(move |state| state.connection.start_automation_command_dispatch(next))
            .unwrap();
        let AutomationResult::GetOuterHtml(html) =
            finish_owned_automation(&handle, step).await.unwrap()
        else {
            panic!("the replacement remains independently usable");
        };
        assert!(html.outer_html.contains("replacement-dom-owner"));
    }

    #[tokio::test]
    async fn automation_target_lifecycle_runs_on_direct_state_without_mailbox() {
        let context = fixture().await;
        let owner = context.state;
        let handle = owner.handle();
        let step = handle
            .with_state(|state| {
                state
                    .connection
                    .start_automation_command_dispatch(target_creation_command())
            })
            .unwrap();
        let AutomationResult::CreateTarget(created) =
            finish_owned_automation(&handle, step).await.unwrap()
        else {
            panic!("creation retains its typed target result");
        };
        let target_id = created.target_id.clone();
        let step = handle
            .with_state(move |state| {
                state.connection.start_automation_command_dispatch(
                    AutomationCommand::ActivateTarget(DevToolsActivateTargetCommand {
                        context: command_context(),
                        target_id,
                    }),
                )
            })
            .unwrap();
        assert_eq!(
            finish_owned_automation(&handle, step).await.unwrap(),
            AutomationResult::Empty
        );
        let selected = created.target_id.clone();
        assert!(
            handle
                .with_state(move |state| {
                    state
                        .connection
                        .browser_context_by_id("BID-configuration")
                        .unwrap()
                        .is_active_target(selected.as_str())
                })
                .unwrap()
        );
        let step =
            handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(
                        AutomationCommand::CloseTarget(DevToolsCloseTargetCommand {
                            context: command_context(),
                            target_id: created.target_id,
                        }),
                    )
                })
                .unwrap();
        assert_eq!(
            finish_owned_automation(&handle, step).await.unwrap(),
            AutomationResult::CloseTarget(DevToolsCloseTargetResult { success: true })
        );
        handle
            .with_state(|state| {
                state
                    .connection
                    .insert_browser_context(BrowserContext::new("BID-owned-remove".into()))
            })
            .unwrap();
        let step = handle
            .with_state(|state| {
                state.connection.start_automation_command_dispatch(
                    AutomationCommand::RemoveBrowserContext(DevToolsRemoveBrowserContextCommand {
                        context: command_context(),
                        browser_context_id: "BID-owned-remove".into(),
                    }),
                )
            })
            .unwrap();
        assert_eq!(
            finish_owned_automation(&handle, step).await.unwrap(),
            AutomationResult::Empty
        );
        assert!(
            !handle
                .with_state(|state| state.connection.has_browser_context_id("BID-owned-remove"))
                .unwrap()
        );
    }

    #[tokio::test]
    async fn automation_target_creation_keeps_initial_document_on_cancel_and_ready_loss() {
        for ready_loss in [false, true] {
            let context = fixture().await;
            let owner = context.state;
            let handle = owner.handle();
            let AutomationCommandTaskStep::Pending(mut pending) = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(target_creation_command())
                })
                .unwrap()
            else {
                panic!("creation owns its initial document wait");
            };
            let wait = pending.take_wait().wait();
            if ready_loss {
                drop(wait.await);
            } else {
                drop(wait);
            }
            let step = handle
                .with_state(move |state| {
                    state.connection.fail_automation_command_wait(
                        *pending,
                        crate::OwnedProtocolWaitFailure::Cancelled,
                    )
                })
                .unwrap();
            let AutomationResult::CreateTarget(created) =
                finish_owned_automation(&handle, step).await.unwrap()
            else {
                panic!("the admitted creation finishes through its original owner");
            };
            assert!(
                handle
                    .with_state(move |state| {
                        let route = state
                            .connection
                            .target_session_route_for_target_id(created.target_id.as_str())
                            .unwrap();
                        let owner = CommandOwnerScope::for_route(route);
                        state
                            .connection
                            .start_initial_document_page_ensure_for_owner(&owner)
                            .unwrap()
                            .is_none()
                    })
                    .unwrap(),
                "cancellation must not leave an orphaned initial document reservation"
            );
        }
    }

    #[tokio::test]
    async fn automation_input_and_fetch_complete_through_direct_state_without_mailbox() {
        let context = fixture().await;
        let owner = context.state;
        let handle = owner.handle();
        for command in [input_command(), interception_command()] {
            let is_input = matches!(&command, AutomationCommand::DispatchKeyEvent(_));
            let mut step = handle
                .with_state(move |state| {
                    state.connection.start_automation_command_dispatch(command)
                })
                .unwrap();
            loop {
                match step {
                    AutomationCommandTaskStep::Pending(pending) => {
                        assert!(
                            handle
                                .with_state(|state| state
                                    .connection
                                    .has_browser_context_id("BID-configuration"))
                                .unwrap()
                        );
                        let completed = pending.wait().await;
                        step = handle
                            .with_state(move |state| {
                                state
                                    .connection
                                    .complete_automation_command_dispatch(completed)
                            })
                            .unwrap();
                    }
                    AutomationCommandTaskStep::Complete(output) => {
                        let (result, output) = output.into_result_and_owner_turn();
                        assert_eq!(
                            result.unwrap(),
                            if is_input {
                                AutomationResult::Empty
                            } else {
                                AutomationResult::AddNetworkIntercept(
                                    DevToolsAddNetworkInterceptResult {
                                        intercept_id: "INTERCEPT-owned".into(),
                                    },
                                )
                            }
                        );
                        let (events, _, _, _, _, predecessor) =
                            output.into_renderer_owner_turn_parts();
                        assert!(
                            events
                                .iter()
                                .all(|event| event.protocol_message_id().is_none()),
                            "internal protocol terminals must not escape automation"
                        );
                        if is_input {
                            assert!(
                                predecessor.is_some(),
                                "the accepted Native input turn retains its exact cursor"
                            );
                        }
                        break;
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn automation_fetch_preserves_accepted_terminal_on_unpolled_cancel_and_ready_loss() {
        for ready_loss in [false, true] {
            let mut conn = CdpConnection::new();
            let mut browser = BrowserContext::new("BID-configuration".into());
            browser.set_active_target_id("TID-configuration");
            conn.install_browser_context_fixture_for_test(browser);
            let owner = ProtocolStateOwner::new(conn);
            let handle = owner.handle();
            let AutomationCommandTaskStep::Pending(mut pending) = handle
                .with_state(|state| {
                    state
                        .connection
                        .start_automation_command_dispatch(interception_command())
                })
                .unwrap()
            else {
                panic!("the original automation owner supervises Fetch");
            };
            let wait = pending.take_wait().wait();
            if ready_loss {
                drop(wait.await);
            } else {
                drop(wait);
            }
            let mut step = handle
                .with_state(move |state| {
                    state.connection.fail_automation_command_wait(
                        *pending,
                        crate::OwnedProtocolWaitFailure::Cancelled,
                    )
                })
                .unwrap();
            loop {
                match step {
                    AutomationCommandTaskStep::Pending(pending) => {
                        let completed = pending.wait().await;
                        step = handle
                            .with_state(move |state| {
                                state
                                    .connection
                                    .complete_automation_command_dispatch(completed)
                            })
                            .unwrap();
                    }
                    AutomationCommandTaskStep::Complete(output) => {
                        let (result, output) = output.into_result_and_owner_turn();
                        assert_eq!(
                            result.unwrap(),
                            AutomationResult::AddNetworkIntercept(
                                DevToolsAddNetworkInterceptResult {
                                    intercept_id: "INTERCEPT-owned".into()
                                }
                            )
                        );
                        let (events, _, _, _, _, _) = output.into_renderer_owner_turn_parts();
                        assert!(
                            events
                                .iter()
                                .all(|event| event.protocol_message_id().is_none())
                        );
                        break;
                    }
                }
            }
        }
    }
}
