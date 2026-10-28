use super::*;
use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpCommandTaskStep, CommandDispatchContext,
    ParsedCdpCommand,
};
use crate::testing::TestContext;
use futures_util::FutureExt as _;
use serde_json::{Value, json};

struct ReleaseRenderer(Option<std::sync::mpsc::Sender<()>>);

impl Drop for ReleaseRenderer {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[tokio::test]
async fn target_message_owns_real_attached_command_and_recovers_its_original_output() {
    attached_command_recovers_its_original_output(false).await;
}

#[tokio::test]
async fn internal_command_owns_real_attached_command_and_recovers_its_original_output() {
    attached_command_recovers_its_original_output(true).await;
}

async fn attached_command_recovers_its_original_output(internal: bool) {
    for phase in ["normal", "before", "waiting", "ready", "deferred"] {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-nested-direct".into());
        browser.set_active_target_id("TID-nested-direct");
        browser.attach_active_session("SID-primary");
        fixture
            .conn
            .install_browser_context_fixture_for_test(browser);
        fixture
            .install_navigation_fixture_for_session_owner(
                if phase == "deferred" { "data:text/html,<script>globalThis.__entered=false;globalThis.__hold=new Promise(r=>globalThis.__release=r)</script>" } else { "about:blank" },
                Some("SID-primary"),
            )
            .await;
        fixture.process_async(json!({"id":1,"method":"Target.attachToTarget","params":{"targetId":"TID-nested-direct","flatten":false}})).await;
        let sid = fixture.take_response_by_id(1)["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_owned();
        fixture.take_all();
        assert_ne!(sid, "SID-primary");
        let gate = if phase == "waiting" {
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, held) = std::sync::mpsc::channel();
            let native = fixture
                .conn
                .loaded_page_mut_for_protocol_access(Some(&sid))
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
            let release = ReleaseRenderer(Some(release));
            tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap();
            Some((release, native))
        } else {
            None
        };
        drop(fixture.conn.state_mailbox.receiver.take());
        let owner = ProtocolStateOwner::new(fixture.conn);
        let handle = owner.handle();
        let expression = if phase == "deferred" {
            "(__entered=true,__hold)"
        } else {
            "6*7"
        };
        let command = ParsedCdpCommand::parse_str(json!({"id":71,"method":"Target.sendMessageToTarget","params":{
            "sessionId":sid,"message":json!({"id":42,"method":"Runtime.evaluate","params":{"expression":expression,"awaitPromise":phase=="deferred","returnByValue":true}}).to_string(),
        }}).to_string()).unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let prefix = json!({"method":"Page.frameStartedLoading","params":{"frameId":"TID-nested-direct"},"sessionId":sid});
        dispatch.push_protocol_event(BackgroundProtocolEvent::immediate(prefix.clone()));
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                if internal {
                    state.start_internal_protocol_command_with_context(
                        &json!({"id":42,"sessionId":sid,"method":"Runtime.evaluate","params":{"expression":expression,"awaitPromise":phase=="deferred","returnByValue":true}}).to_string(),
                        &mut dispatch,
                    )
                } else {
                    state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
                }
            })
            .unwrap()
        else {
            panic!("the adapter uses the ordinary owned workflow");
        };
        assert_eq!(pending.kind_name(), "Workflow");
        let completed = if phase == "deferred" {
            let mut wait = Box::pin(pending.wait());
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let messages = tokio::select! {
                        _ = &mut wait => panic!("the original command must stay pending until explicit promise release"),
                        messages = direct_internal_command(&handle, json!({"id":90,"sessionId":sid,"method":"Runtime.evaluate","params":{"expression":"__entered===true","returnByValue":true}})) => messages,
                    };
                    if messages.iter().any(|message| message["id"] == 90 && message["result"]["result"]["value"] == true) { break; }
                }
            }).await.unwrap();
            assert!(wait.as_mut().now_or_never().is_none());
            direct_internal_command(&handle, json!({"id":91,"sessionId":sid,"method":"Runtime.evaluate","params":{"expression":"__release(42)","returnByValue":true}})).await;
            Some(
                tokio::time::timeout(std::time::Duration::from_secs(5), wait)
                    .await
                    .unwrap(),
            )
        } else if phase == "normal" {
            Some(pending.wait().await)
        } else if phase == "ready" {
            drop(pending.wait().await);
            None
        } else {
            let mut wait = Box::pin(pending.wait());
            if phase == "waiting" {
                assert!(wait.as_mut().now_or_never().is_none());
            }
            let metadata = ParsedCdpCommand::parse_str(
                json!({"id":72,"method":"Browser.getVersion"}).to_string(),
            )
            .unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| state.start_parsed_command_dispatch_with_context(
                        &metadata,
                        &mut CommandDispatchContext::default()
                    ))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            drop(wait);
            None
        };
        drop(gate);
        let completed = match completed {
            Some(completed) => completed,
            None => {
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            if internal { None } else { Some(71) },
                            None,
                            crate::OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut dispatch,
                        )
                    })
                    .unwrap()
                else {
                    panic!("the original adapter owns its cleanup");
                };
                recovery.wait().await
            }
        };
        let mut step = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("owned completion must be finite"));
        let outcome = loop {
            match step {
                CdpCommandTaskStep::Complete(outcome) => break outcome,
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
                        .unwrap_or_else(|_| panic!("recovery must use finite state"));
                }
            }
        };
        let (events, _, predecessor) = outcome.into_protocol_event_parts();
        let messages: Vec<_> = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect();
        assert_eq!(
            messages.iter().filter(|m| **m == prefix).count(),
            1,
            "{phase}: {messages:?}"
        );
        if internal {
            assert!(!messages.iter().any(|message| message["id"] == 71));
            assert!(
                !messages
                    .iter()
                    .any(|message| message["method"] == "Target.receivedMessageFromTarget")
            );
            let terminals: Vec<_> = messages
                .iter()
                .filter(|message| message["id"] == 42)
                .collect();
            assert_eq!(terminals.len(), 1, "{phase}: {messages:?}");
            assert_eq!(terminals[0]["sessionId"], sid);
            if matches!(phase, "normal" | "ready" | "deferred") {
                assert_eq!(
                    terminals[0]["result"]["result"]["value"], 42,
                    "{phase}: {messages:?}"
                );
                assert!(predecessor.is_some());
            } else {
                assert_eq!(
                    terminals[0]["error"]["code"], -32000,
                    "{phase}: {messages:?}"
                );
            }
        } else {
            let terminals: Vec<_> = messages.iter().filter(|m| m["id"] == 71).collect();
            assert_eq!(terminals.len(), 1, "{phase}: {messages:?}");
            assert_eq!(
                terminals[0]["result"],
                json!({}),
                "accepted transport keeps its acknowledgement"
            );
            assert!(!messages.iter().any(|m| m["id"] == 42));
            let nested: Vec<Value> = messages
                .iter()
                .filter(|m| m["method"] == "Target.receivedMessageFromTarget")
                .map(|m| {
                    assert_eq!(m["params"]["sessionId"], sid);
                    serde_json::from_str(m["params"]["message"].as_str().unwrap()).unwrap()
                })
                .filter(|m: &Value| m["id"] == 42)
                .collect();
            assert_eq!(nested.len(), 1, "{phase}: {messages:?}");
            assert!(nested[0].get("sessionId").is_none());
            if matches!(phase, "normal" | "ready" | "deferred") {
                assert_eq!(
                    nested[0]["result"]["result"]["value"], 42,
                    "{phase}: {messages:?}"
                );
                assert!(predecessor.is_some());
            } else {
                assert_eq!(nested[0]["error"]["code"], -32000, "{phase}: {messages:?}");
            }
        }
        assert!(dispatch.take_command_failure_cleanup().is_none());
    }
}

async fn direct_internal_command(handle: &ProtocolStateHandle, command: Value) -> Vec<Value> {
    let mut context = CommandDispatchContext::default();
    let mut step = handle
        .with_state(|state| {
            state.start_internal_protocol_command_with_context(&command.to_string(), &mut context)
        })
        .unwrap();
    loop {
        match step {
            CdpCommandTaskStep::Complete(outcome) => {
                return outcome
                    .into_protocol_event_parts()
                    .0
                    .into_iter()
                    .map(BackgroundProtocolEvent::into_protocol_message)
                    .collect();
            }
            CdpCommandTaskStep::Pending(pending) => {
                let completed = pending.wait().await;
                step = handle
                    .with_state(|state| {
                        state.try_complete_ready_command_dispatch_with_context(
                            completed,
                            &mut context,
                        )
                    })
                    .unwrap()
                    .unwrap_or_else(|_| panic!("internal peer completion is finite"));
            }
        }
    }
}
