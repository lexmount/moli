use super::*;
use crate::conn::{BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};
use crate::testing::TestContext;

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
        drop(context.conn.state_mailbox.receiver.take());
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let command = ParsedCdpCommand::parse_str(
            json!({"id":71,"sessionId":"SID-owned-input",
            "method":"Input.dispatchKeyEvent","params":{"type":"keyUp","key":"A","code":"KeyA"}})
            .to_string(),
        )
        .unwrap();
        let prefix = json!({"method":"Page.frameStartedLoading","params":{"frameId":"TID-owned-input"},"sessionId":"SID-owned-input"});
        let mut dispatch = CommandDispatchContext::default();
        dispatch.push_protocol_event(BackgroundProtocolEvent::immediate(prefix.clone()));
        let CdpCommandTaskStep::Pending(mut pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("input must retain its owned renderer wait");
        };
        assert_eq!(pending.kind_name(), "Input");
        let completed = match phase {
            "normal" => Some(pending.wait().await),
            "ready" => {
                drop(pending.wait().await);
                None
            }
            "waiting" => {
                assert!(pending.hold_input_renderer_ack_for_test());
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
