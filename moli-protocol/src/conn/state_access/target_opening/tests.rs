use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpCommandTaskStep, CommandDispatchContext,
    ParsedCdpCommand, ProtocolStateOwner,
};
use crate::testing::TestContext;
use futures_util::FutureExt as _;
use serde_json::json;

struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[tokio::test]
async fn creation_owns_surface_wait_and_preserves_events_after_waiter_or_ready_loss() {
    for phase in 0..3 {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-opening".into());
        browser.set_active_target_id("TID-opening");
        browser.attach_active_session("SID-opening");
        fixture
            .conn
            .install_browser_context_fixture_for_test(browser);
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-opening"))
            .await;
        fixture.conn.set_target_discovery_for_owner(
            None,
            crate::conn::CdpTargetFilter::default_target_discovery(),
        );
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":1810,"method":"Target.createTarget","params":{"url":"about:blank"}}"#,
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = fixture
            .conn
            .start_parsed_command_dispatch_with_context(&command, &mut dispatch)
        else {
            panic!("creation prepares its initial Document before installation");
        };
        let completed = pending.wait().await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (gate, guard) = fixture
            .conn
            .loaded_page_mut_for_protocol_access(Some("SID-opening"))
            .unwrap()
            .start_cdp_call(
                1899,
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
        let CdpCommandTaskStep::Pending(workflow) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("production creation completion must be owned"))
        else {
            panic!("surface updates wait behind the old Page's Native gate");
        };
        assert_eq!(workflow.kind_name(), "Workflow");
        let mut waiting = Box::pin(workflow.wait());
        assert!(waiting.as_mut().now_or_never().is_none());
        let peer =
            ParsedCdpCommand::parse_str(r#"{"id":1811,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&peer, &mut Default::default())
                })
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        if phase == 1 {
            drop(waiting);
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(1810),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("creation retains its original supervised task");
            };
            waiting = Box::pin(recovery.wait());
        }
        drop(release);
        gate.wait().await.unwrap();
        drop(guard);
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap();
        let completed = if phase == 2 {
            drop(completed);
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(1810),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("ready loss preserves the original creation terminal");
            };
            recovery.wait().await
        } else {
            completed
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("creation handoff is finite"))
        else {
            panic!("the initial Document and all surface updates have settled");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        let response = messages
            .iter()
            .position(|message| message["id"] == 1810)
            .unwrap();
        let target_id = messages[response]["result"]["targetId"].as_str().unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["id"] == 1810)
                .count(),
            1,
            "{messages:?}"
        );
        let created = messages
            .iter()
            .position(|message| {
                message["method"] == "Target.targetCreated"
                    && message["params"]["targetInfo"]["targetId"] == target_id
            })
            .unwrap();
        assert!(created < response, "{messages:?}");
        handle
            .with_state(|state| {
                assert!(
                    state
                        .connection
                        .browser_context_by_id("BID-opening")
                        .unwrap()
                        .page_target(target_id)
                        .unwrap()
                        .runtime_slot
                        .has_loaded_page()
                );
                assert!(state.connection.state_mailbox.receiver.is_none());
            })
            .unwrap();
    }
}

#[tokio::test]
async fn attachment_owns_native_wait_and_recovers_waiter_ready_loss_and_closing() {
    for phase in 0..5 {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-opening".into());
        browser.set_active_target_id("TID-opening");
        browser.attach_active_session("SID-opening");
        fixture
            .conn
            .install_browser_context_fixture_for_test(browser);
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-opening"))
            .await;
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":1800,"method":"Target.attachToTarget","params":{"targetId":"TID-opening","flatten":true}}"#,
        ).unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = fixture
            .conn
            .start_parsed_command_dispatch_with_context(&command, &mut dispatch)
        else {
            panic!("Page attachment has a renderer configuration tail");
        };
        let session_id = fixture
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .devtools_sessions
            .attached_session_ids()
            .next()
            .unwrap()
            .to_owned();
        let completed = pending.wait().await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (gate, guard) = fixture
            .conn
            .loaded_page_mut_for_protocol_access(Some("SID-opening"))
            .unwrap()
            .start_cdp_call(
                1899,
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
        let CdpCommandTaskStep::Pending(workflow) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("production attach completion must be owned"))
        else {
            panic!("attachment still waits behind the Native gate");
        };
        assert_eq!(workflow.kind_name(), "Workflow");
        let mut waiting = Box::pin(workflow.wait());
        assert!(waiting.as_mut().now_or_never().is_none());
        let peer =
            ParsedCdpCommand::parse_str(r#"{"id":1801,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&peer, &mut Default::default())
                })
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        if phase == 1 {
            drop(waiting);
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(1800),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("the original task retains the admitted attachment");
            };
            waiting = Box::pin(recovery.wait());
            assert!(waiting.as_mut().now_or_never().is_none());
        }
        if phase == 3 {
            handle
                .with_state(|state| {
                    state
                        .connection
                        .browser_context
                        .as_mut()
                        .unwrap()
                        .active_page_target_mut()
                        .closing = true;
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
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(1800),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("ready loss rejoins the original terminal");
            };
            recovery.wait().await
        } else {
            completed
        };
        if phase == 4 {
            handle
                .with_state(|state| {
                    state
                        .connection
                        .browser_context
                        .as_mut()
                        .unwrap()
                        .active_page_target_mut()
                        .closing = true;
                })
                .unwrap();
        }
        let mut completed = completed;
        let outcome = loop {
            match handle
                .with_state(|state| {
                    state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("attachment final handoff is finite"))
            {
                CdpCommandTaskStep::Complete(outcome) => break outcome,
                CdpCommandTaskStep::Pending(rollback) => {
                    assert!(phase >= 3, "only a retired attachment needs rollback");
                    assert_eq!(rollback.kind_name(), "Workflow");
                    completed =
                        tokio::time::timeout(std::time::Duration::from_secs(5), rollback.wait())
                            .await
                            .unwrap();
                }
            }
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();
        let replies = messages
            .iter()
            .filter(|message| message["id"] == 1800)
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 1, "{messages:?}");
        let attached = messages
            .iter()
            .position(|message| message["method"] == "Target.attachedToTarget");
        if phase >= 3 {
            assert!(replies[0]["error"].is_object(), "{messages:?}");
            assert!(attached.is_none(), "{messages:?}");
        } else {
            assert_eq!(replies[0]["result"]["sessionId"], json!(session_id));
            assert!(
                attached.unwrap()
                    < messages
                        .iter()
                        .position(|message| message["id"] == 1800)
                        .unwrap()
            );
        }
        handle
            .with_state(|state| {
                assert_eq!(
                    state.connection.has_attached_devtools_session(&session_id),
                    phase < 3
                );
                assert_eq!(
                    state
                        .connection
                        .browser_context
                        .as_ref()
                        .unwrap()
                        .active_page_target()
                        .devtools_sessions
                        .key_for_wire_session_id(&session_id)
                        .is_some(),
                    phase < 3
                );
            })
            .unwrap();
    }
}
