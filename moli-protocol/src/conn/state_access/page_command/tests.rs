use crate::conn::{
    BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
    PendingCdpCommandDispatch, ProtocolStateHandle, ProtocolStateOwner,
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

fn start(
    handle: &ProtocolStateHandle,
    command: Value,
    context: &mut CommandDispatchContext,
) -> CdpCommandTaskStep {
    let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
    handle
        .with_state(|state| state.start_parsed_command_dispatch_with_context(&command, context))
        .unwrap()
}

fn recover(
    handle: &ProtocolStateHandle,
    context: &mut CommandDispatchContext,
) -> PendingCdpCommandDispatch {
    let CdpCommandTaskStep::Pending(pending) = handle
        .with_state(|state| {
            state.complete_failed_command_wait_with_context(
                Some(2101),
                Some("SID-page-owned"),
                crate::OwnedProtocolWaitFailure::Cancelled.message(),
                context,
            )
        })
        .unwrap()
    else {
        panic!("the original Page command retains its workflow");
    };
    *pending
}

async fn assert_owned_page_wait(method: &str) {
    for phase in 0..3 {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-page-owned".into());
        browser.set_active_target_id("TID-page-owned");
        browser.attach_active_session("SID-page-owned");
        fixture
            .conn
            .install_browser_context_fixture_for_test(browser);
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-page-owned"))
            .await;
        if method == "Page.createIsolatedWorld" {
            fixture
                .conn
                .with_target_owner_state_for_session_mut(Some("SID-page-owned"), |state| {
                    state.document_start_scripts.push((
                        "owned-bidi".to_owned(),
                        crate::conn::DocumentStartScript {
                            registry_key: None,
                            devtools_session: None,
                            source: "() => {}".to_owned(),
                            world_name: Some("owned-world".to_owned()),
                            has_bidi_channel_argument: true,
                            bidi_channel_handoffs: Vec::new(),
                        },
                    ));
                })
                .unwrap();
        }
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (native, guard) = fixture
            .conn
            .loaded_page_mut_for_protocol_access(Some("SID-page-owned"))
            .unwrap()
            .start_cdp_call(
                2199,
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
        let release = ReleaseOnDrop(Some(release));
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        drop(fixture.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(fixture.conn);
        let handle = owner.handle();
        // Release the native gate before dropping its owner if an assertion fails.
        let release = release;
        let mut context = CommandDispatchContext::default();
        let mut command = json!({"id":2101,"method":method,"sessionId":"SID-page-owned"});
        if method == "Page.createIsolatedWorld" {
            command["params"] = json!({"frameId":"TID-page-owned","worldName":"owned-world"});
        }
        let CdpCommandTaskStep::Pending(pending) = start(&handle, command, &mut context) else {
            panic!("{method} starts a supervised workflow at admission");
        };
        assert_eq!(pending.kind_name(), "Workflow");
        let mut wait = Box::pin(pending.wait());
        let mut ready = None;
        if method == "Page.close" {
            if phase != 1 {
                ready = Some(
                    tokio::time::timeout(std::time::Duration::from_secs(5), wait.as_mut())
                        .await
                        .unwrap(),
                );
            }
        } else {
            assert!(
                wait.as_mut().now_or_never().is_none(),
                "{method} is held by the real renderer"
            );
        }
        let CdpCommandTaskStep::Complete(peer) = start(
            &handle,
            json!({"id":2102,"method":"Browser.getVersion"}),
            &mut Default::default(),
        ) else {
            panic!("peer completes while Native is held");
        };
        let (_, mut published_work, _) = peer.into_protocol_event_parts();
        handle
            .with_state(|state| {
                assert!(state.connection.state_mailbox.receiver.is_none());
                let owner = crate::conn::CommandOwnerScope::capture(
                    state.connection,
                    Some("SID-page-owned"),
                );
                if method == "Page.crash" {
                    assert!(
                        state
                            .connection
                            .runtime_session_owner_slot_for_owner(&owner)
                            .is_some_and(|slot| !slot.has_loaded_page()),
                        "crash owns the old Page before awaiting its close"
                    );
                }
            })
            .unwrap();
        if phase == 1 {
            drop(wait);
            wait = Box::pin(recover(&handle, &mut context).wait());
            if method == "Page.close" {
                ready = Some(
                    tokio::time::timeout(std::time::Duration::from_secs(5), wait.as_mut())
                        .await
                        .unwrap(),
                );
            } else {
                assert!(wait.as_mut().now_or_never().is_none());
            }
        }
        drop(release);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), native.wait())
            .await
            .unwrap();
        drop(guard);
        let completed = match ready {
            Some(completed) => completed,
            None => tokio::time::timeout(std::time::Duration::from_secs(5), wait)
                .await
                .unwrap(),
        };
        let completed = if phase == 2 {
            drop(completed);
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                recover(&handle, &mut context).wait(),
            )
            .await
            .unwrap()
        } else {
            completed
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut context)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("Page completion has no connection wait"))
        else {
            panic!("one finite handoff");
        };
        let (events, scheduler_events, _) = outcome.into_protocol_event_parts();
        published_work.extend(scheduler_events);
        if method == "Page.close" {
            assert_eq!(published_work.iter().filter(|event| matches!(event, crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.kind() == crate::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction)).count(), 1, "one original target retirement survives command recovery");
        }
        let messages = events
            .into_iter()
            .map(|event| event.into_protocol_message())
            .collect::<Vec<_>>();
        let replies = messages
            .iter()
            .filter(|message| message["id"] == 2101)
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 1, "{method} phase={phase}: {messages:?}");
        if method == "Page.createIsolatedWorld" {
            assert!(
                replies[0]["result"]["executionContextId"].is_i64(),
                "{method} phase={phase}: {messages:?}"
            );
        } else {
            assert_eq!(
                replies[0]["result"],
                json!({}),
                "{method} phase={phase}: {messages:?}"
            );
        }
    }
}

#[tokio::test]
async fn stop_loading_owns_native_wait_through_cancellation_and_ready_loss() {
    assert_owned_page_wait("Page.stopLoading").await;
}

#[tokio::test]
async fn page_crash_owns_retired_page_through_cancellation_and_ready_loss() {
    assert_owned_page_wait("Page.crash").await;
}

#[tokio::test]
async fn page_close_preserves_ack_and_retirement_through_cancellation_and_ready_loss() {
    assert_owned_page_wait("Page.close").await;
}

#[tokio::test]
async fn isolated_world_composite_owns_native_wait_through_cancellation_and_ready_loss() {
    assert_owned_page_wait("Page.createIsolatedWorld").await;
}

#[tokio::test]
async fn stop_loading_captures_original_navigation_at_admission_before_a_successor() {
    let mut fixture = TestContext::new();
    let mut browser = BrowserContext::new("BID-page-owned".into());
    browser.set_active_target_id("TID-page-owned");
    browser.attach_active_session("SID-page-owned");
    fixture
        .conn
        .install_browser_context_fixture_for_test(browser);
    fixture
        .install_navigation_fixture_for_session_owner("about:blank", Some("SID-page-owned"))
        .await;
    fixture
        .process_async(json!({"id":2103,"method":"Fetch.enable","sessionId":"SID-page-owned"}))
        .await;
    fixture.process_async(json!({"id":2104,"method":"Page.navigate","sessionId":"SID-page-owned","params":{"url":"http://example.test/stop-original"}})).await;
    let original = fixture
        .take_all()
        .into_iter()
        .find(|message| message["method"] == "Fetch.requestPaused")
        .expect("original request pauses")["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(fixture.conn.take_protocol_state_requests());
    let owner = ProtocolStateOwner::new(fixture.conn);
    let handle = owner.handle();
    let mut context = CommandDispatchContext::default();
    let CdpCommandTaskStep::Pending(pending) = start(
        &handle,
        json!({"id":2101,"method":"Page.stopLoading","sessionId":"SID-page-owned"}),
        &mut context,
    ) else {
        panic!("stop owns its native continuation");
    };
    assert_eq!(pending.kind_name(), "Workflow");
    handle
        .with_state(|state| {
            assert!(
                !state
                    .connection
                    .browser_context_by_id("BID-page-owned")
                    .unwrap()
                    .page_target("TID-page-owned")
                    .unwrap()
                    .fetch_owner
                    .has_pending_fetch_navigation_for_test(&original),
                "admission already owns the interrupted request"
            );
        })
        .unwrap();
    // Deliberately leave the original task unpolled until the successor has
    // been accepted. Its later cleanup must keep that newer request intact.
    let CdpCommandTaskStep::Complete(paused) = start(
        &handle,
        json!({"id":2105,"method":"Page.navigate","sessionId":"SID-page-owned","params":{"url":"http://example.test/stop-newer"}}),
        &mut Default::default(),
    ) else {
        panic!("successor request can pause independently");
    };
    let (events, _, _) = paused.into_protocol_event_parts();
    let newer = events
        .into_iter()
        .map(|event| event.into_protocol_message())
        .find(|message| message["method"] == "Fetch.requestPaused")
        .expect("successor request pauses")["params"]["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let completed = tokio::time::timeout(std::time::Duration::from_secs(5), pending.wait())
        .await
        .unwrap();
    let CdpCommandTaskStep::Complete(outcome) = handle
        .with_state(|state| {
            state.try_complete_ready_command_dispatch_with_context(completed, &mut context)
        })
        .unwrap()
        .unwrap_or_else(|_| panic!("finite completion"))
    else {
        panic!("one completion");
    };
    let (events, _, _) = outcome.into_protocol_event_parts();
    let messages = events
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["id"] == 2101 && message["result"] == json!({}))
            .count(),
        1,
        "{messages:?}"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["id"] == 2104)
            .count(),
        1,
        "{messages:?}"
    );
    assert!(
        messages.iter().all(|message| message["id"] != 2105),
        "{messages:?}"
    );
    handle
        .with_state(|state| {
            assert!(
                state
                    .connection
                    .browser_context_by_id("BID-page-owned")
                    .unwrap()
                    .page_target("TID-page-owned")
                    .unwrap()
                    .fetch_owner
                    .has_pending_fetch_navigation_for_test(&newer),
                "late cleanup preserves the successor"
            );
        })
        .unwrap();
}
