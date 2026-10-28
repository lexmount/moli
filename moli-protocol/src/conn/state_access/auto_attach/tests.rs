use crate::conn::{
    BrowserContext, CdpCommandTaskStep, CommandDispatchContext, CompletedCdpCommandDispatch,
    ParsedCdpCommand, PendingCdpCommandDispatch, ProtocolStateHandle,
};
use crate::testing::TestContext;
use futures_util::FutureExt as _;
use serde_json::{Value, json};

struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

async fn fixture() -> (TestContext, String) {
    let mut fixture = TestContext::new();
    let mut browser = BrowserContext::new("BID-auto-owned".into());
    browser.set_active_target_id("TID-auto-owned");
    browser.attach_active_session("SID-auto-owned");
    fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
    fixture
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-auto-owned"))
        .await;
    fixture
        .process_async(json!({"id":1900,"method":"Target.attachToBrowserTarget"}))
        .await;
    let session = fixture.take_response_by_id(1900)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.take_all();
    (fixture, session)
}

async fn hold_renderer(
    fixture: &mut TestContext,
) -> (
    moli_core::page::PendingPageCommand,
    moli_core::RendererNativeCommandResponseGuard,
    ReleaseOnDrop,
) {
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let (pending, guard) = fixture.with_fixture(|conn| {
        conn.loaded_page_mut_for_protocol_access(Some("SID-auto-owned"))
            .unwrap()
            .start_cdp_call(
                1999,
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
            .unwrap()
    });
    let release = ReleaseOnDrop(Some(release));
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    (pending, guard, release)
}

fn start(
    handle: &ProtocolStateHandle,
    id: u64,
    owner: &str,
    enabled: bool,
    context: &mut CommandDispatchContext,
) -> PendingCdpCommandDispatch {
    let command = ParsedCdpCommand::parse_str(json!({
        "id":id,"sessionId":owner,"method":"Target.setAutoAttach",
        "params":{"autoAttach":enabled,"waitForDebuggerOnStart":false,"flatten":true,
            "filter": if enabled { json!([{"type":"page"},{"exclude":true}]) } else { json!([]) }},
    }).to_string()).unwrap();
    let CdpCommandTaskStep::Pending(pending) = handle
        .with_state(|state| state.start_parsed_command_dispatch_with_context(&command, context))
        .unwrap()
    else {
        panic!("auto-attach uses its supervised ordinary async workflow");
    };
    assert_eq!(pending.kind_name(), "Workflow");
    *pending
}

fn recover(
    handle: &ProtocolStateHandle,
    id: u64,
    owner: &str,
    context: &mut CommandDispatchContext,
) -> PendingCdpCommandDispatch {
    let CdpCommandTaskStep::Pending(recovery) = handle
        .with_state(|state| {
            state.complete_failed_command_wait_with_context(
                Some(id),
                Some(owner),
                crate::OwnedProtocolWaitFailure::Cancelled.message(),
                context,
            )
        })
        .unwrap()
    else {
        panic!("the original command retains its resource transaction");
    };
    *recovery
}

async fn finish(
    handle: &ProtocolStateHandle,
    mut completed: CompletedCdpCommandDispatch,
    context: &mut CommandDispatchContext,
) -> Vec<Value> {
    for _ in 0..4 {
        match handle
            .with_state(|state| state.complete_command_dispatch_with_context(completed, context))
            .unwrap()
        {
            CdpCommandTaskStep::Complete(outcome) => {
                let (events, _, _) = outcome.into_protocol_event_parts();
                return events
                    .into_iter()
                    .map(|event| event.into_protocol_message())
                    .collect();
            }
            CdpCommandTaskStep::Pending(pending) => {
                assert_eq!(pending.kind_name(), "Workflow");
                completed = tokio::time::timeout(std::time::Duration::from_secs(5), pending.wait())
                    .await
                    .unwrap();
            }
        }
    }
    panic!("one final handoff and its original rollback must settle");
}

fn peer(handle: &ProtocolStateHandle) {
    let command =
        ParsedCdpCommand::parse_str(r#"{"id":1980,"method":"Browser.getVersion"}"#).unwrap();
    assert!(matches!(
        handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut Default::default())
            })
            .unwrap(),
        CdpCommandTaskStep::Complete(_)
    ));
}

fn attached(messages: &[Value]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["targetId"] == "TID-auto-owned"
        })
        .map(|message| message["params"]["sessionId"].as_str().unwrap().to_owned())
        .collect()
}

fn success(messages: &[Value], id: u64) {
    let replies = messages
        .iter()
        .filter(|message| message["id"] == id)
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 1, "{messages:?}");
    assert_eq!(replies[0]["result"], json!({}), "{messages:?}");
    let reply = messages
        .iter()
        .position(|message| message["id"] == id)
        .unwrap();
    assert!(
        messages
            .iter()
            .skip(reply + 1)
            .all(|message| message["method"] != "Target.attachedToTarget"),
        "{messages:?}"
    );
}

#[tokio::test]
async fn auto_attach_enable_owns_wait_and_rejects_retired_policy_after_ready_loss() {
    for phase in 0..6 {
        let (mut fixture, owner_session) = fixture().await;
        let (gate, guard, release) = hold_renderer(&mut fixture).await;
        let owner = fixture.state;
        let handle = owner.handle();
        let mut context = CommandDispatchContext::default();
        let pending = start(&handle, 1901, &owner_session, true, &mut context);
        let mut waiting = Box::pin(pending.wait());
        assert!(waiting.as_mut().now_or_never().is_none());
        peer(&handle);
        if phase == 1 {
            drop(waiting);
            waiting = Box::pin(recover(&handle, 1901, &owner_session, &mut context).wait());
            assert!(waiting.as_mut().now_or_never().is_none());
        }
        let mut replacement = None;
        let mut replacement_context = CommandDispatchContext::default();
        if phase == 3 || phase == 4 {
            let mut disabled_context = CommandDispatchContext::default();
            let disabled = start(&handle, 1902, &owner_session, false, &mut disabled_context)
                .wait()
                .await;
            success(
                &finish(&handle, disabled, &mut disabled_context).await,
                1902,
            );
            if phase == 4 {
                let mut reenabled = Box::pin(
                    start(
                        &handle,
                        1903,
                        &owner_session,
                        true,
                        &mut replacement_context,
                    )
                    .wait(),
                );
                assert!(reenabled.as_mut().now_or_never().is_none());
                replacement = Some(reenabled);
            }
        }
        if phase == 5 {
            handle
                .with_state(|state| {
                    let route = state
                        .connection
                        .session_route(Some(&owner_session))
                        .unwrap();
                    let plan =
                        crate::conn::SessionDisposalPlan::for_session_route(&owner_session, &route)
                            .unwrap();
                    state.connection.mark_session_closing(&plan);
                })
                .unwrap();
        }
        drop(release);
        gate.wait().await.unwrap();
        drop(guard);
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap();
        let completed = if phase == 2 {
            drop(completed);
            recover(&handle, 1901, &owner_session, &mut context)
                .wait()
                .await
        } else {
            completed
        };
        let messages = finish(&handle, completed, &mut context).await;
        success(&messages, 1901);
        assert_eq!(
            attached(&messages).len(),
            usize::from(phase < 3),
            "phase={phase} {messages:?}"
        );
        let mut expected = attached(&messages);
        if let Some(replacement) = replacement {
            let ready = tokio::time::timeout(std::time::Duration::from_secs(5), replacement)
                .await
                .unwrap();
            let messages = finish(&handle, ready, &mut replacement_context).await;
            success(&messages, 1903);
            expected = attached(&messages);
            assert_eq!(expected.len(), 1, "{messages:?}");
        }
        handle
            .with_state(|state| {
                assert_eq!(
                    state
                        .connection
                        .auto_attached_sessions_for_owner(Some(&owner_session)),
                    expected
                );
                let declarations = state
                    .connection
                    .browser_context_by_id("BID-auto-owned")
                    .unwrap()
                    .page_target("TID-auto-owned")
                    .unwrap()
                    .devtools_sessions
                    .attached_session_ids()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                assert_eq!(
                    declarations, expected,
                    "retired provisional sessions cannot survive rollback"
                );
                assert!(state.connection.state_access.is_some());
            })
            .unwrap();
    }
}

#[tokio::test]
async fn auto_attach_disable_owns_cleanup_and_preserves_a_reenabled_successor() {
    for phase in 0..4 {
        let (fixture, owner_session) = fixture().await;
        let owner = fixture.state;
        let handle = owner.handle();
        let mut enabled_context = CommandDispatchContext::default();
        let enabled = start(&handle, 1910, &owner_session, true, &mut enabled_context)
            .wait()
            .await;
        let enabled = finish(&handle, enabled, &mut enabled_context).await;
        let old_session = attached(&enabled).pop().unwrap();
        // Install a real session-owned Native script. Empty session disposal
        // can finish synchronously and would not exercise the cleanup wait.
        let preload = ParsedCdpCommand::parse_str(
            json!({
                "id":1915,"sessionId":old_session,"method":"Page.addScriptToEvaluateOnNewDocument",
                "params":{"source":"globalThis.autoAttachCleanup = true"},
            })
            .to_string(),
        )
        .unwrap();
        let mut preload_context = CommandDispatchContext::default();
        let preload = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&preload, &mut preload_context)
            })
            .unwrap();
        if let CdpCommandTaskStep::Pending(preload) = preload {
            finish(&handle, preload.wait().await, &mut preload_context).await;
        }
        let (gate, guard, entered, release) = handle
            .with_state(|state| {
                // The test-only setup owns no state borrow while the Native gate waits.
                let page = state
                    .connection
                    .loaded_page_mut_for_protocol_access(Some("SID-auto-owned"))
                    .unwrap();
                let (entered, waiting) = tokio::sync::oneshot::channel();
                let (release, held) = std::sync::mpsc::channel();
                let (gate, guard) = page
                    .start_cdp_call(
                        1999,
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
                (gate, guard, waiting, ReleaseOnDrop(Some(release)))
            })
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), entered)
            .await
            .unwrap()
            .unwrap();
        let mut context = CommandDispatchContext::default();
        let mut waiting =
            Box::pin(start(&handle, 1911, &owner_session, false, &mut context).wait());
        assert!(waiting.as_mut().now_or_never().is_none());
        peer(&handle);
        if phase == 1 {
            drop(waiting);
            waiting = Box::pin(recover(&handle, 1911, &owner_session, &mut context).wait());
        }
        let mut replacement_context = CommandDispatchContext::default();
        let replacement = if phase == 3 {
            let mut pending = Box::pin(
                start(
                    &handle,
                    1912,
                    &owner_session,
                    true,
                    &mut replacement_context,
                )
                .wait(),
            );
            assert!(pending.as_mut().now_or_never().is_none());
            Some(pending)
        } else {
            None
        };
        drop(release);
        gate.wait().await.unwrap();
        drop(guard);
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap();
        let completed = if phase == 2 {
            drop(completed);
            recover(&handle, 1911, &owner_session, &mut context)
                .wait()
                .await
        } else {
            completed
        };
        let messages = finish(&handle, completed, &mut context).await;
        success(&messages, 1911);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget"
                    && message["params"]["sessionId"] == old_session)
                .count(),
            1,
            "{messages:?}"
        );
        let expected = if let Some(replacement) = replacement {
            let ready = tokio::time::timeout(std::time::Duration::from_secs(5), replacement)
                .await
                .unwrap();
            let messages = finish(&handle, ready, &mut replacement_context).await;
            success(&messages, 1912);
            let sessions = attached(&messages);
            assert_eq!(sessions.len(), 1);
            assert_ne!(sessions[0], old_session);
            sessions
        } else {
            Vec::new()
        };
        handle
            .with_state(|state| {
                assert!(!state.connection.has_attached_devtools_session(&old_session));
                assert_eq!(
                    state
                        .connection
                        .auto_attached_sessions_for_owner(Some(&owner_session)),
                    expected
                );
            })
            .unwrap();
    }
}

#[tokio::test]
async fn auto_attach_multi_context_sweep_keeps_page_streams_independent() {
    let (mut fixture, owner_session) = fixture().await;
    let mut second = BrowserContext::new("BID-auto-second".into());
    second.set_active_target_id("TID-auto-second");
    second.attach_active_session("SID-auto-second");
    fixture.with_fixture(|conn| conn.insert_browser_context(second));
    fixture.with_fixture(|conn| conn.commit_declared_session_fixtures_for_test());
    fixture
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-auto-second"))
        .await;
    let owner = fixture.state;
    let handle = owner.handle();
    let mut context = CommandDispatchContext::default();
    let ready = start(&handle, 1920, &owner_session, true, &mut context)
        .wait()
        .await;
    let CdpCommandTaskStep::Complete(outcome) = handle
        .with_state(|state| state.complete_command_dispatch_with_context(ready, &mut context))
        .unwrap()
    else {
        panic!("all per-Page binding work has settled");
    };
    let (events, _, predecessor) = outcome.into_protocol_event_parts();
    assert!(
        predecessor.is_none(),
        "a Browser policy ACK cannot join independent Page tails"
    );
    let messages = events
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    success(&messages, 1920);
    let mut targets = messages
        .iter()
        .filter(|message| message["method"] == "Target.attachedToTarget")
        .map(|message| {
            message["params"]["targetInfo"]["targetId"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>();
    targets.sort_unstable();
    assert_eq!(targets, ["TID-auto-owned", "TID-auto-second"]);
}
