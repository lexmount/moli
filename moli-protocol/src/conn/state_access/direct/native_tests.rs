use super::*;
use crate::conn::{BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};
use crate::testing::TestContext;

struct ReleaseInputRenderer(Option<std::sync::mpsc::Sender<()>>);

impl Drop for ReleaseInputRenderer {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[tokio::test]
async fn native_completion_and_discarded_ready_recovery_use_direct_state() {
    for discard_ready in [false, true] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-direct-native".into());
        browser.set_active_target_id("TID-direct-native");
        browser.attach_active_session("SID-direct-native");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-direct-native"))
            .await;
        // The final owner must not need the legacy actor's mailbox receiver.
        drop(context.conn.state_mailbox.receiver.take());
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":51,"sessionId":"SID-direct-native","method":"DOM.getDocument"}"#,
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("DOM query must retain its native wait");
        };
        assert_eq!(pending.kind_name(), "Native");
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":52,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| state.start_parsed_command_dispatch_with_context(
                    &version,
                    &mut CommandDispatchContext::default()
                ))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        let completed = pending.wait().await;
        let completed = if discard_ready {
            drop(completed);
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(51),
                        Some("SID-direct-native"),
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("ready-result loss must settle the original native response authority");
            };
            assert!(recovery.continues_after_session_detachment());
            recovery.wait().await
        } else {
            completed
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("Native completion must be finite"))
        else {
            panic!("a settled native command cannot introduce another wait");
        };
        let (events, _, predecessor) = outcome.into_protocol_event_parts();
        assert!(
            predecessor.is_some(),
            "the renderer terminal retains its exact predecessor"
        );
        assert!(
            events.is_empty(),
            "adapter settlement cannot emit a second native response"
        );
        assert!(dispatch.take_command_failure_cleanup().is_none());
    }
}

#[tokio::test]
async fn network_and_adapter_emulation_complete_through_direct_state() {
    use serde_json::json;
    let mut context = TestContext::new();
    let mut browser = BrowserContext::new("BID-finite-completion".into());
    browser.set_active_target_id("TID-finite-completion");
    browser.attach_active_session("SID-finite-completion");
    context
        .conn
        .install_browser_context_fixture_for_test(browser);
    context
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-finite-completion"))
        .await;
    drop(context.conn.state_mailbox.receiver.take());
    let owner = ProtocolStateOwner::new(context.conn);
    let handle = owner.handle();
    for (kind, command) in [
        (
            "Network",
            json!({"id":61,"sessionId":"SID-finite-completion","method":"Network.enable"}),
        ),
        (
            "Emulation",
            json!({"id":62,"sessionId":"SID-finite-completion","method":"Emulation.setScriptExecutionDisabled","params":{"value":true}}),
        ),
    ] {
        let id = command["id"].as_u64().unwrap();
        let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
        let mut dispatch = CommandDispatchContext::default();
        dispatch.set_terminal_response_delivery_override(
            moli_page_types::RendererInspectorResponseDelivery::AdapterReply,
        );
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("{kind} must own its native policy wait");
        };
        assert_eq!(pending.kind_name(), kind);
        let completed = pending.wait().await;
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("{kind} completion is synchronous"))
        else {
            panic!("{kind} policy receipt already settled");
        };
        let (events, _, _predecessor) = outcome.into_protocol_event_parts();
        let messages: Vec<_> = events
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1
        );
        assert_eq!(
            messages.iter().find(|message| message["id"] == id).unwrap(),
            &json!({"id":id,"sessionId":"SID-finite-completion","result":{}})
        );
    }
}

#[tokio::test]
async fn performance_accessibility_and_dom_use_synchronous_state_completion() {
    use serde_json::json;
    let mut context = TestContext::new();
    let mut browser = BrowserContext::new("BID-query-completion".into());
    browser.set_active_target_id("TID-query-completion");
    browser.attach_active_session("SID-query-completion");
    context
        .conn
        .install_browser_context_fixture_for_test(browser);
    context
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-query-completion"))
        .await;
    for command in [
        json!({"id":1,"sessionId":"SID-query-completion","method":"Runtime.enable"}),
        json!({"id":2,"sessionId":"SID-query-completion","method":"Performance.enable"}),
    ] {
        context.process_async(command).await;
    }
    drop(context.conn.state_mailbox.receiver.take());
    let owner = ProtocolStateOwner::new(context.conn);
    let handle = owner.handle();
    let mut root_node_id = None;
    for (index, (kind, method)) in [
        ("Performance", "Performance.getMetrics"),
        ("Accessibility", "Accessibility.getFullAXTree"),
        ("DOM", "DOM.getDocument"),
        ("DOM", "DOM.resolveNode"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = index as u64 + 71;
        let params = if method == "DOM.resolveNode" {
            json!({"nodeId":root_node_id.unwrap()})
        } else {
            json!({})
        };
        let command = ParsedCdpCommand::parse_str(
            json!({"id":id,"sessionId":"SID-query-completion","method":method,"params":params})
                .to_string(),
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        dispatch.set_terminal_response_delivery_override(
            moli_page_types::RendererInspectorResponseDelivery::AdapterReply,
        );
        let mut step = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap();
        let mut waits = 0;
        let outcome = loop {
            match step {
                CdpCommandTaskStep::Complete(outcome) => break outcome,
                CdpCommandTaskStep::Pending(pending) => {
                    assert_eq!(pending.kind_name(), kind);
                    waits += 1;
                    let completed = pending.wait().await;
                    step = handle
                        .with_state(|state| {
                            state.try_complete_ready_command_dispatch_with_context(
                                completed,
                                &mut dispatch,
                            )
                        })
                        .unwrap()
                        .unwrap_or_else(|_| {
                            panic!("{method} must finish using synchronous state access")
                        });
                }
            }
        };
        assert!(waits > 0, "{method} must exercise an owned renderer wait");
        let (events, _, _predecessor) = outcome.into_protocol_event_parts();
        let messages: Vec<_> = events
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect();
        let response = messages.iter().find(|message| message["id"] == id).unwrap();
        assert!(response.get("error").is_none(), "{method}: {response:?}");
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1
        );
        match method {
            "Performance.getMetrics" => {
                assert!(!response["result"]["metrics"].as_array().unwrap().is_empty())
            }
            "Accessibility.getFullAXTree" => {
                assert!(!response["result"]["nodes"].as_array().unwrap().is_empty())
            }
            "DOM.getDocument" => {
                root_node_id = response["result"]["root"]["nodeId"].as_u64();
                assert!(root_node_id.is_some());
            }
            "DOM.resolveNode" => assert!(response["result"]["object"]["objectId"].is_string()),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn input_owned_completion_retains_context_through_cancel_and_ready_loss() {
    use crate::conn::BackgroundProtocolEvent;
    use futures_util::FutureExt as _;
    use serde_json::json;

    for phase in ["normal", "before-poll", "waiting", "ready"] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-owned-input".into());
        browser.set_active_target_id("TID-owned-input");
        browser.attach_active_session("SID-owned-input");
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-owned-input"))
            .await;
        let native_gate = if phase == "waiting" {
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, held) = std::sync::mpsc::channel();
            let (native, guard) = context
                .conn
                .loaded_page_mut_for_protocol_access(Some("SID-owned-input"))
                .unwrap()
                .start_cdp_call(
                    7199,
                    None,
                    moli_core::RendererNativeOperation::then(
                        moli_core::RendererPageCommand::ChildFrameTreeSnapshot,
                        move |_| {
                            let _ = entered.send(());
                            let _ = held.recv();
                            moli_core::RendererNativeOperationStep::Complete(
                                moli_core::RendererNativeProtocolResponse::success(json!({})),
                            )
                        },
                    ),
                )
                .unwrap();
            let release = ReleaseInputRenderer(Some(release));
            tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap();
            Some((release, native, guard))
        } else {
            None
        };
        drop(context.conn.state_mailbox.receiver.take());
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let _native_gate = native_gate;
        let command = ParsedCdpCommand::parse_str(
            json!({"id":71,"sessionId":"SID-owned-input",
            "method":"Input.dispatchKeyEvent","params":{"type":"keyUp","key":"A","code":"KeyA"}})
            .to_string(),
        )
        .unwrap();
        let prefix = json!({"method":"Page.frameStartedLoading","params":{"frameId":"TID-owned-input"},"sessionId":"SID-owned-input"});
        let mut dispatch = CommandDispatchContext::default();
        dispatch.push_protocol_event(BackgroundProtocolEvent::immediate(prefix.clone()));
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("input must retain its owned renderer wait");
        };
        assert_eq!(pending.kind_name(), "Workflow");
        assert!(!pending.continues_after_session_detachment());
        let completed = match phase {
            "normal" => Some(pending.wait().await),
            "ready" => {
                drop(pending.wait().await);
                None
            }
            "waiting" => {
                let mut wait = Box::pin(pending.wait());
                assert!(wait.as_mut().now_or_never().is_none());
                let version =
                    ParsedCdpCommand::parse_str(r#"{"id":72,"method":"Browser.getVersion"}"#)
                        .unwrap();
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
                drop(wait);
                None
            }
            _ => {
                drop(pending);
                None
            }
        };
        let completed = match completed {
            Some(completed) => completed,
            None => {
                // Cancellation of the recovery's ready result must return the
                // same context and any already-completed terminal again.
                for _ in 0..2 {
                    let CdpCommandTaskStep::Pending(recovery) = handle
                        .with_state(|state| {
                            state.complete_failed_command_wait_with_context(
                                Some(71),
                                Some("SID-owned-input"),
                                "cancelled input wait",
                                &mut dispatch,
                            )
                        })
                        .unwrap()
                    else {
                        panic!("the original command must recover its context");
                    };
                    assert!(recovery.continues_after_session_detachment());
                    drop(recovery.wait().await);
                }
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            Some(71),
                            Some("SID-owned-input"),
                            "cancelled input wait",
                            &mut dispatch,
                        )
                    })
                    .unwrap()
                else {
                    panic!("recovery retains the original output authority");
                };
                recovery.wait().await
            }
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("owned input completion must be finite"))
        else {
            panic!("all input work is complete");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages: Vec<_> = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect();
        assert_eq!(
            messages
                .iter()
                .filter(|message| **message == prefix)
                .count(),
            1,
            "{phase}"
        );
        let responses: Vec<_> = messages
            .iter()
            .filter(|message| message["id"] == 71)
            .collect();
        assert_eq!(responses.len(), 1, "{phase}: {messages:?}");
        let response = responses[0];
        if matches!(phase, "normal" | "ready") {
            assert_eq!(response["result"], json!({}), "{phase}: {response:?}");
            assert!(response.get("error").is_none());
        } else {
            assert_eq!(response["error"]["code"], -32000, "{phase}: {response:?}");
        }
        assert!(dispatch.take_command_failure_cleanup().is_none());
        assert!(
            messages
                .iter()
                .position(|message| *message == prefix)
                .unwrap()
                < messages
                    .iter()
                    .position(|message| message["id"] == 71)
                    .unwrap()
        );
    }
}

#[tokio::test]
async fn cancelled_input_state_reply_keeps_the_original_native_output() {
    use crate::conn::BackgroundProtocolEvent;
    use serde_json::json;
    let mut fixture = TestContext::new();
    let mut browser = BrowserContext::new("BID-input-reply-drop".into());
    browser.set_active_target_id("TID-input-reply-drop");
    browser.attach_active_session("SID-input-reply-drop");
    fixture
        .conn
        .install_browser_context_fixture_for_test(browser);
    fixture.install_navigation_fixture_for_session_owner(
        "data:text/html,<script>document.addEventListener('keyup',()=>console.log('input-native-output'))</script>",
        Some("SID-input-reply-drop"),
    ).await;
    fixture
        .process_async(
            json!({"id":70,"method":"Runtime.enable","sessionId":"SID-input-reply-drop"}),
        )
        .await;
    let mut requests = fixture.conn.take_protocol_state_requests().unwrap();
    let command = ParsedCdpCommand::parse_str(
        json!({
            "id":71,"method":"Input.dispatchKeyEvent","sessionId":"SID-input-reply-drop",
            "params":{"type":"keyUp","key":"A","code":"KeyA"},
        })
        .to_string(),
    )
    .unwrap();
    let mut context = CommandDispatchContext::default();
    let prefix = json!({"method":"Page.frameStartedLoading","params":{"frameId":"TID-input-reply-drop"},"sessionId":"SID-input-reply-drop"});
    context.push_protocol_event(BackgroundProtocolEvent::immediate(prefix.clone()));
    let CdpCommandTaskStep::Pending(pending) = fixture
        .conn
        .start_parsed_command_dispatch_with_context(&command, &mut context)
    else {
        panic!("the input owns its ordinary command workflow");
    };
    let mut wait = Box::pin(pending.wait());
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            request = requests.recv() => request.unwrap(),
            _ = &mut wait => panic!("Native input output must enter one finite state operation"),
        }
    })
    .await
    .unwrap();
    request.apply(&mut fixture.conn);
    // The Native result has been accepted; its state reply is still unread.
    drop(wait);
    let CdpCommandTaskStep::Pending(recovery) =
        fixture.conn.complete_failed_command_wait_with_context(
            Some(71),
            Some("SID-input-reply-drop"),
            "cancelled input waiter",
            &mut context,
        )
    else {
        panic!("the same context owns cancellation recovery")
    };
    let completed = recovery.wait().await;
    let CdpCommandTaskStep::Complete(outcome) = fixture
        .conn
        .try_complete_ready_command_dispatch_with_context(completed, &mut context)
        .unwrap_or_else(|_| panic!("recovery is finite"))
    else {
        panic!("recovery has no external wait")
    };
    let (events, _, predecessor) = outcome.into_protocol_event_parts();
    assert!(
        predecessor.is_some(),
        "accepted Native output survives the discarded state reply"
    );
    let messages: Vec<_> = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect();
    assert_eq!(
        messages
            .iter()
            .filter(|message| **message == prefix)
            .count(),
        1
    );
    let terminals: Vec<_> = messages
        .iter()
        .filter(|message| message["id"] == 71)
        .collect();
    assert_eq!(terminals.len(), 1);
    assert_eq!(terminals[0]["error"]["code"], -32000);
    assert!(context.take_command_failure_cleanup().is_none());
}

#[tokio::test]
async fn direct_input_mutates_the_original_document_without_a_mailbox() {
    use serde_json::json;
    let mut context = TestContext::new();
    let mut browser = BrowserContext::new("BID-input-document".into());
    browser.set_active_target_id("TID-input-document");
    browser.attach_active_session("SID-input-document");
    context
        .conn
        .install_browser_context_fixture_for_test(browser);
    context.install_navigation_fixture_for_session_owner("data:text/html,<body><div id=field contenteditable=true></div><script>document.getElementById('field').focus()</script></body>", Some("SID-input-document")).await;
    drop(context.conn.state_mailbox.receiver.take());
    let owner = ProtocolStateOwner::new(context.conn);
    let handle = owner.handle();
    for (id, method, params) in [
        (73, "Input.insertText", json!({"text":"owned input text"})),
        (74, "DOM.getDocument", json!({"depth":-1})),
    ] {
        let command = ParsedCdpCommand::parse_str(
            json!({"id":id,"sessionId":"SID-input-document","method":method,"params":params})
                .to_string(),
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let mut step = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap();
        let outcome = loop {
            match step {
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = pending.wait().await;
                    step = handle
                        .with_state(|state| {
                            state.try_complete_ready_command_dispatch_with_context(
                                completed,
                                &mut dispatch,
                            )
                        })
                        .unwrap()
                        .unwrap_or_else(|_| panic!("{method} must use finite state completion"));
                }
                CdpCommandTaskStep::Complete(outcome) => break outcome,
            }
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages: Vec<_> = events
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect();
        if id == 73 {
            assert_eq!(
                messages.iter().find(|message| message["id"] == id).unwrap()["result"],
                json!({})
            );
        } else {
            // DOM's native terminal is published to ingress; its immutable
            // cache is nevertheless refreshed against the same Page owner.
            assert!(messages.is_empty());
            assert!(dispatch.take_command_failure_cleanup().is_none());
        }
    }
    let connection = std::rc::Rc::try_unwrap(owner.connection)
        .ok()
        .expect("tasks retain only weak handles")
        .into_inner();
    let mut context = TestContext::new();
    context.conn = connection;
    context.conn.state_mailbox = Default::default();
    assert_eq!(
        context
            .conn
            .evaluate_runtime_expression_with_await_async(
                "document.getElementById('field').textContent",
                false
            )
            .await
            .unwrap()["value"],
        "owned input text"
    );
}

async fn run_direct_fetch_command(
    handle: &ProtocolStateHandle,
    command: serde_json::Value,
) -> (Vec<serde_json::Value>, usize) {
    let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
    let mut context = CommandDispatchContext::default();
    context.set_terminal_response_delivery_override(
        moli_page_types::RendererInspectorResponseDelivery::AdapterReply,
    );
    let mut step = handle
        .with_state(|state| {
            state.start_parsed_command_dispatch_with_context(&command, &mut context)
        })
        .unwrap();
    let mut workflows = 0;
    loop {
        match step {
            CdpCommandTaskStep::Complete(outcome) => {
                let (events, after, _, response, _, _) = outcome.into_renderer_owner_turn_parts();
                return (
                    events
                        .into_iter()
                        .chain(after)
                        .chain(response)
                        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
                        .collect(),
                    workflows,
                );
            }
            CdpCommandTaskStep::Pending(pending) => {
                workflows += usize::from(pending.kind_name() == "Workflow");
                let completed =
                    tokio::time::timeout(std::time::Duration::from_secs(5), pending.wait())
                        .await
                        .unwrap();
                step = handle
                    .with_state(|state| {
                        state.try_complete_ready_command_dispatch_with_context(
                            completed,
                            &mut context,
                        )
                    })
                    .unwrap()
                    .unwrap_or_else(|completed| {
                        panic!(
                            "{} completion {} still requires an exclusive connection",
                            command.request().method(),
                            completed.kind_name()
                        )
                    });
            }
        }
    }
}

#[tokio::test]
async fn fetch_document_continuations_complete_with_direct_state_and_no_mailbox() {
    use serde_json::json;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/",
                axum::routing::get(|| async { axum::response::Html("<title>Fetch</title>") }),
            ),
        )
        .await
        .unwrap();
    });
    for action in [
        "continueRequest",
        "fulfillRequest",
        "failRequest",
        "continueResponse",
    ] {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-direct-fetch".into());
        browser.set_active_target_id("TID-direct-fetch");
        browser.attach_active_session("SID-direct-fetch");
        fixture
            .conn
            .install_browser_context_fixture_for_test(browser);
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-direct-fetch"))
            .await;
        drop(fixture.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(fixture.conn);
        let handle = owner.handle();
        let mut patterns = vec![json!({"urlPattern":"*","requestStage":"Request"})];
        if action == "continueResponse" {
            let (enabled, _) = run_direct_fetch_command(
                &handle,
                json!({"id":0,"sessionId":"SID-direct-fetch","method":"Network.enable"}),
            )
            .await;
            assert_eq!(
                enabled.iter().find(|message| message["id"] == 0).unwrap()["result"],
                json!({})
            );
            patterns.push(json!({"urlPattern":"*","requestStage":"Response"}));
        }
        let (enabled, _) = run_direct_fetch_command(&handle,
            json!({"id":1,"sessionId":"SID-direct-fetch","method":"Fetch.enable","params":{"patterns":patterns}})
        ).await;
        assert_eq!(
            enabled.iter().find(|message| message["id"] == 1).unwrap()["result"],
            json!({})
        );
        let (paused, _) = run_direct_fetch_command(&handle,
            json!({"id":2,"sessionId":"SID-direct-fetch","method":"Page.navigate","params":{"url":url}})
        ).await;
        let request_id = paused
            .iter()
            .find(|message| message["method"] == "Fetch.requestPaused")
            .unwrap()["params"]["requestId"]
            .clone();
        assert!(
            !paused.iter().any(|message| message["id"] == 2),
            "navigation remains paused"
        );
        let mut params = json!({"requestId":request_id});
        if action == "fulfillRequest" {
            params["responseCode"] = json!(200);
            params["responseHeaders"] = json!([{"name":"Content-Type","value":"text/html"}]);
            params["body"] = json!("PHRpdGxlPkZldGNoPC90aXRsZT4=");
        } else if action == "failRequest" {
            params["errorReason"] = json!("Aborted");
        }
        let method = if action == "continueResponse" {
            "continueRequest"
        } else {
            action
        };
        let (mut completed, workflows) = run_direct_fetch_command(&handle,
            json!({"id":3,"sessionId":"SID-direct-fetch","method":format!("Fetch.{method}"),"params":params})
        ).await;
        assert!(
            workflows > 0,
            "{action} must use its owned protocol completion"
        );
        assert_eq!(
            completed.iter().find(|message| message["id"] == 3).unwrap()["result"],
            json!({})
        );
        if action == "continueResponse" {
            let response_id = completed
                .iter()
                .find(|message| message["method"] == "Fetch.requestPaused")
                .unwrap()["params"]["requestId"]
                .clone();
            assert!(
                completed
                    .iter()
                    .position(|message| message["id"] == 3)
                    .unwrap()
                    < completed
                        .iter()
                        .position(|message| message["method"] == "Fetch.requestPaused")
                        .unwrap()
            );
            assert!(!completed.iter().any(|message| message["id"] == 2));
            let (response, workflows) = run_direct_fetch_command(&handle,
                json!({"id":4,"sessionId":"SID-direct-fetch","method":"Fetch.continueResponse","params":{"requestId":response_id,"responseCode":201,"responseHeaders":[{"name":"Content-Type","value":"text/html"}]}})
            ).await;
            assert!(workflows > 0);
            assert_eq!(
                response.iter().find(|message| message["id"] == 4).unwrap()["result"],
                json!({})
            );
            assert_eq!(
                response
                    .iter()
                    .find(|message| message["method"] == "Network.responseReceived")
                    .unwrap()["params"]["response"]["status"],
                201
            );
            completed.extend(response);
        }
        assert_eq!(
            completed
                .iter()
                .filter(|message| message["id"] == 2)
                .count(),
            1,
            "{action}: {completed:?}"
        );
        let navigation = completed.iter().find(|message| message["id"] == 2).unwrap();
        let ack_id = if action == "continueResponse" { 4 } else { 3 };
        assert!(
            completed
                .iter()
                .position(|message| message["id"] == ack_id)
                .unwrap()
                < completed
                    .iter()
                    .position(|message| message["id"] == 2)
                    .unwrap()
        );
        if action == "failRequest" {
            assert!(
                navigation.get("error").is_some()
                    || navigation["result"].get("errorText").is_some(),
                "{navigation:?}"
            );
            assert!(
                !completed
                    .iter()
                    .any(|message| message["method"] == "Page.frameNavigated"),
                "failure must not load an error document"
            );
        } else {
            assert!(navigation.get("error").is_none(), "{navigation:?}");
            let (evaluated, _) = run_direct_fetch_command(&handle,
                json!({"id":5,"sessionId":"SID-direct-fetch","method":"Runtime.evaluate","params":{"expression":"document.title","returnByValue":true}})
            ).await;
            assert_eq!(
                evaluated.iter().find(|message| message["id"] == 5).unwrap()["result"]["result"]["value"],
                "Fetch"
            );
        }
    }
    server.abort();
    let _ = server.await;
}
