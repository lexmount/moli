use crate::conn::{BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};
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
        fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-opening"))
            .await;
        fixture.with_fixture(|conn| {
            conn.set_target_discovery_for_owner(
                None,
                crate::conn::CdpTargetFilter::default_target_discovery(),
            )
        });
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":1810,"method":"Target.createTarget","params":{"url":"about:blank"}}"#,
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = fixture.with_fixture(|conn| {
            conn.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
        }) else {
            panic!("creation prepares its initial Document before installation");
        };
        let completed = pending.wait().await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (gate, guard) = fixture.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(Some("SID-opening"))
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
                .unwrap()
        });
        let release = ReleaseOnDrop(Some(release));
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        let owner = fixture.state;
        let handle = owner.handle();
        let CdpCommandTaskStep::Pending(workflow) = handle
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
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
                state.complete_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("the initial Document and all surface updates have settled");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages = events
            .into_iter()
            .map(|event| event.into_protocol_message())
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
                assert!(state.connection.state_access.is_some());
            })
            .unwrap();
    }
}

#[tokio::test]
async fn attachment_commits_while_native_is_busy_and_rejects_a_closing_target() {
    for closing in [false, true] {
        let mut fixture = TestContext::new();
        let mut browser = BrowserContext::new("BID-opening".into());
        browser.set_active_target_id("TID-opening");
        browser.attach_active_session("SID-opening");
        fixture.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        fixture
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-opening"))
            .await;
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        let (gate, guard) = fixture.with_fixture(|conn| {
            conn.loaded_page_mut_for_protocol_access(Some("SID-opening"))
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
                .unwrap()
        });
        let release = ReleaseOnDrop(Some(release));
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        if closing {
            fixture.with_fixture(|conn| {
                conn.browser_context
                    .as_mut()
                    .unwrap()
                    .active_page_target_mut()
                    .closing = true;
            });
        }
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":1800,"method":"Target.attachToTarget","params":{"targetId":"TID-opening","flatten":true}}"#,
        ).unwrap();
        let CdpCommandTaskStep::Complete(outcome) = fixture.with_fixture(|conn| {
            conn.start_parsed_command_dispatch_with_context(
                &command,
                &mut CommandDispatchContext::default(),
            )
        }) else {
            panic!("Browser attachment never waits on native readiness");
        };
        let (events, _, _) = outcome.into_protocol_event_parts();
        let messages = events
            .into_iter()
            .map(|event| event.into_protocol_message())
            .collect::<Vec<_>>();
        let replies = messages
            .iter()
            .filter(|message| message["id"] == 1800)
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 1, "{messages:?}");
        let attached = messages
            .iter()
            .position(|message| message["method"] == "Target.attachedToTarget");
        if closing {
            assert!(replies[0]["error"].is_object(), "{messages:?}");
            assert!(attached.is_none(), "{messages:?}");
            fixture.with_fixture(|conn| {
                assert!(
                    conn.browser_context
                        .as_ref()
                        .unwrap()
                        .active_page_target()
                        .devtools_sessions
                        .attached_session_ids()
                        .next()
                        .is_none()
                );
            });
        } else {
            let session_id = replies[0]["result"]["sessionId"].as_str().unwrap();
            assert!(
                attached.unwrap()
                    < messages
                        .iter()
                        .position(|message| message["id"] == 1800)
                        .unwrap()
            );
            fixture.with_fixture(|conn| assert!(conn.has_attached_devtools_session(session_id)));
        }
        let peer =
            ParsedCdpCommand::parse_str(r#"{"id":1801,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            fixture.with_fixture(|conn| {
                conn.start_parsed_command_dispatch_with_context(&peer, &mut Default::default())
            }),
            CdpCommandTaskStep::Complete(_)
        ));
        drop(release);
        gate.wait().await.unwrap();
        drop(guard);
    }
}
