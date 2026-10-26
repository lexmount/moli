use super::*;
use crate::conn::{
    BrowserContext, CdpCommandTaskStep, CdpSchedulerEvent, CommandDispatchContext,
    CommandOwnerScope, ParsedCdpCommand,
};
use crate::domains::page::PageTargetTerminationOwnerAction;
use crate::testing::TestContext;
use serde_json::json;

#[tokio::test]
async fn ordinary_target_close_uses_direct_state_and_recovers_its_original_scope() {
    use crate::conn::{CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    for failure_phase in [None, Some(false), Some(true)] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-command-close".into());
        browser.set_active_target_id("TID-command-close");
        browser.attach_active_session("SID-command-close");
        browser.stage_background_target(
            "TID-command-next".into(),
            Some("SID-command-next".into()),
            "about:blank".into(),
            None,
            None,
        );
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        let _ = context
            .conn
            .register_top_level_page_target("TID-command-close");
        for session in ["SID-command-close", "SID-command-next"] {
            context
                .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                .await;
        }
        let destroyed = Arc::new(AtomicUsize::new(0));
        let observed = destroyed.clone();
        context
            .conn
            .set_target_host_lifecycle_observer(CdpTargetHostLifecycleObserver::new(
                move |delta| {
                    if matches!(delta, CdpTargetHostLifecycleDelta::Destroyed { .. }) {
                        observed.fetch_add(1, Ordering::Relaxed);
                    }
                },
            ));
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let command = ParsedCdpCommand::parse_str(
            r#"{"id":80,"method":"Target.closeTarget","params":{"targetId":"TID-command-close"}}"#,
        )
        .unwrap();
        let mut dispatch = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
            })
            .unwrap()
        else {
            panic!("ordinary target close owns its native waits");
        };
        assert!(pending.owns_session_disposal());
        handle
            .with_state(|state| {
                assert!(state.connection.session_is_closing("SID-command-close"));
                assert!(
                    state
                        .connection
                        .has_attached_devtools_session("SID-command-close")
                );
                assert!(state.connection.state_mailbox.receiver.is_some());
            })
            .unwrap();
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":81,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| state.start_parsed_command_dispatch_with_context(
                    &version,
                    &mut CommandDispatchContext::default(),
                ))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        let completed = if let Some(after_ready) = failure_phase {
            if after_ready {
                let ready = pending.wait().await;
                assert_eq!(destroyed.load(Ordering::Relaxed), 0);
                drop(ready);
            } else {
                drop(pending);
            }
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(80),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut dispatch,
                    )
                })
                .unwrap()
            else {
                panic!("failed waits retain their admitted native close");
            };
            assert!(recovery.owns_session_disposal());
            recovery.wait().await
        } else {
            pending.wait().await
        };
        handle
            .with_state(|state| {
                assert!(
                    state
                        .connection
                        .has_attached_devtools_session("SID-command-close")
                )
            })
            .unwrap();
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
            })
            .unwrap()
            .unwrap_or_else(|_| panic!("native close completion is finite"))
        else {
            panic!("native waits have completed");
        };
        let messages = outcome.into_parts().0;
        let reply = messages.iter().find(|message| message["id"] == 80).unwrap();
        if failure_phase.is_some() {
            assert_eq!(
                reply["error"]["message"],
                crate::OwnedProtocolWaitFailure::Cancelled.message()
            );
        } else {
            assert_eq!(reply["result"]["success"], true);
        }
        let response = messages
            .iter()
            .position(|message| message["id"] == 80)
            .unwrap();
        let detached = messages
            .iter()
            .position(|message| message["method"] == "Target.detachedFromTarget")
            .unwrap();
        assert!(response < detached, "{messages:?}");
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count(),
            1
        );
        assert_eq!(destroyed.load(Ordering::Relaxed), 2);
        handle
            .with_state(|state| {
                let conn = state.connection;
                assert!(!conn.has_attached_devtools_session("SID-command-close"));
                assert!(conn.has_attached_devtools_session("SID-command-next"));
                assert_eq!(
                    conn.browser_context.as_ref().unwrap().active_target_id(),
                    Some("TID-command-next")
                );
            })
            .unwrap();
    }
}

#[tokio::test]
async fn target_termination_uses_direct_state_and_preserves_a_peers_selection() {
    for peer_selects in [false, true] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-termination".into());
        browser.set_active_target_id("TID-closing");
        browser.attach_active_session("SID-closing");
        assert!(browser.assign_attached_session_to_target("TID-closing", "SID-attached".into()));
        for name in ["next", "peer"] {
            browser.stage_background_target(
                format!("TID-{name}"),
                Some(format!("SID-{name}")),
                "about:blank".into(),
                None,
                None,
            );
        }
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        let tab = context.conn.register_top_level_page_target("TID-closing");
        context
            .conn
            .attach_tab_target_session_event_plan("SID-tab".into(), None, &tab, false)
            .unwrap();
        for session in ["SID-closing", "SID-next", "SID-peer"] {
            context
                .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                .await;
        }
        for command in [
            json!({"id":1,"method":"Target.setDiscoverTargets","params":{"discover":true,"filter":[{}]}}),
            json!({"id":2,"sessionId":"SID-next","method":"Page.startScreencast","params":{}}),
        ] {
            let id = command["id"].as_u64().unwrap();
            context.process_async(command).await;
            assert!(context.take_response_by_id(id).get("result").is_some());
        }
        let mut routing = BrowserContext::new("BID-routing".into());
        routing.set_active_target_id("TID-routing");
        context
            .conn
            .push_inactive_browser_context_fixture_for_test(routing);
        assert!(context.conn.activate_browser_context_by_id("BID-routing"));
        let action = PageTargetTerminationOwnerAction::new(
            &context.conn,
            CommandOwnerScope::for_session("SID-closing"),
            "TID-closing".into(),
        );
        context
            .conn
            .publish_page_target_termination_owner_action(action);
        let work = context
            .conn
            .take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work } => Some(work),
                _ => None,
            })
            .unwrap();
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let pending = handle
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap()
            .unwrap();
        handle
            .with_state(|state| {
                assert_eq!(
                    state.connection.browser_context.as_ref().unwrap().id,
                    "BID-routing"
                );
                assert!(state.connection.state_mailbox.receiver.is_some());
                for session in ["SID-closing", "SID-attached", "SID-tab"] {
                    assert!(state.connection.has_attached_devtools_session(session));
                    assert!(state.connection.session_is_closing(session));
                }
            })
            .unwrap();
        let version =
            ParsedCdpCommand::parse_str(r#"{"id":3,"method":"Browser.getVersion"}"#).unwrap();
        assert!(matches!(
            handle
                .with_state(|state| state.start_parsed_command_dispatch_with_context(
                    &version,
                    &mut CommandDispatchContext::default(),
                ))
                .unwrap(),
            CdpCommandTaskStep::Complete(_)
        ));
        if peer_selects {
            let command = ParsedCdpCommand::parse_str(
                json!({"id":4,"method":"Target.activateTarget","params":{"targetId":"TID-peer"}})
                    .to_string(),
            )
            .unwrap();
            let mut dispatch = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(activation) = handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
                })
                .unwrap()
            else {
                panic!("activation owns its waits");
            };
            let activation = activation.wait().await;
            assert!(matches!(
                handle
                    .with_state(
                        |state| state.try_complete_ready_command_dispatch_with_context(
                            activation,
                            &mut dispatch
                        )
                    )
                    .unwrap()
                    .unwrap_or_else(|_| panic!("activation completion is finite")),
                CdpCommandTaskStep::Complete(_)
            ));
        }
        let recovery = pending.clone();
        let completed = pending.wait().await;
        handle
            .with_state(|state| {
                for session in ["SID-closing", "SID-attached", "SID-tab"] {
                    assert!(
                        state.connection.has_attached_devtools_session(session),
                        "a queued terminal cannot unbind early"
                    );
                }
                assert_eq!(
                    state
                        .connection
                        .tab_target_id_for_page_target_id("TID-closing"),
                    Some(tab.as_str())
                );
            })
            .unwrap();
        let output = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap()
            .into_parts()
            .0;
        let detached: Vec<_> = output
            .iter()
            .filter(|message| message["method"] == "Target.detachedFromTarget")
            .collect();
        assert_eq!(detached.len(), 3, "{output:?}");
        let destroyed: Vec<_> = output
            .iter()
            .filter(|message| message["method"] == "Target.targetDestroyed")
            .map(|message| message["params"]["targetId"].as_str().unwrap())
            .collect();
        assert_eq!(destroyed, ["TID-closing", tab.as_str()]);
        assert!(
            output
                .iter()
                .position(|message| message["method"] == "Target.targetDestroyed")
                .unwrap()
                > output
                    .iter()
                    .rposition(|message| message["method"] == "Target.detachedFromTarget")
                    .unwrap()
        );
        handle
            .with_state(|state| {
                let conn = state.connection;
                for session in ["SID-closing", "SID-attached", "SID-tab"] {
                    assert!(!conn.has_attached_devtools_session(session));
                }
                assert_eq!(conn.browser_context.as_ref().unwrap().id, "BID-routing");
                let expected = if peer_selects { "TID-peer" } else { "TID-next" };
                assert_eq!(
                    conn.browser_context_by_id("BID-termination")
                        .unwrap()
                        .active_target_id(),
                    Some(expected)
                );
                assert!(
                    conn.tab_target_id_for_page_target_id("TID-closing")
                        .is_none()
                );
            })
            .unwrap();
        // A retained cancellation authority shares the original native ACK,
        // and cannot commit a second detach or targetDestroyed batch.
        let duplicate = recovery.wait().await;
        assert!(
            handle
                .with_state(|state| state.complete_owned_protocol_scheduler_work(duplicate))
                .unwrap()
                .into_parts()
                .0
                .is_empty()
        );
    }
}

#[tokio::test]
async fn queued_termination_cannot_destroy_a_recreated_page_or_tab_with_the_same_ids() {
    for reuse_sessions in [false, true] {
        let successor_page = if reuse_sessions {
            "SID-retiring-page"
        } else {
            "SID-successor-page"
        };
        let successor_tab = if reuse_sessions {
            "SID-retiring-tab"
        } else {
            "SID-successor-tab"
        };
        let mut conn = CdpConnection::new();
        let mut browser = BrowserContext::new("BID-recreated-close".into());
        browser.set_active_target_id("TID-recreated-close");
        browser.attach_active_session("SID-retiring-page");
        conn.install_browser_context_fixture_for_test(browser);
        let tab = conn.register_top_level_page_target("TID-recreated-close");
        conn.attach_tab_target_session_event_plan("SID-retiring-tab".into(), None, &tab, false)
            .unwrap();
        let _ = conn.process_message_with_turn_outcome_async(
        r#"{"id":1,"method":"Target.setDiscoverTargets","params":{"discover":true,"filter":[{}]}}"#,
    )
    .await.into_protocol_event_parts();
        let action = PageTargetTerminationOwnerAction::new(
            &conn,
            CommandOwnerScope::for_session("SID-retiring-page"),
            "TID-recreated-close".into(),
        );
        conn.publish_page_target_termination_owner_action(action);
        let work = conn
            .take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.kind()
                        == crate::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction =>
                {
                    Some(work)
                }
                _ => None,
            })
            .unwrap();
        let owner = ProtocolStateOwner::new(conn);
        let handle = owner.handle();
        let pending = handle
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap()
            .unwrap();
        let completed = pending.wait().await;
        let successor = handle
            .with_state(|state| {
                let conn = state.connection;
                if reuse_sessions {
                    for (target, session) in [
                        ("TID-recreated-close", "SID-retiring-page"),
                        (tab.as_str(), "SID-retiring-tab"),
                    ] {
                        conn.commit_target_closure_session_detachment_events(
                            crate::conn::TargetClosureCleanupPlan::new(
                                target,
                                None,
                                [session.to_owned()],
                            ),
                            None,
                        );
                    }
                }
                let mut browser = BrowserContext::new("BID-recreated-close".into());
                browser.set_active_target_id("TID-recreated-close");
                browser.attach_active_session(successor_page);
                conn.install_browser_context_fixture_for_test(browser);
                conn.target_control
                    .register_tab(tab.clone(), "TID-recreated-close".into());
                conn.attach_tab_target_session_event_plan(successor_tab.into(), None, &tab, false)
                    .unwrap();
                conn.page_target_host_identity_for_owner(&CommandOwnerScope::for_session(
                    successor_page,
                ))
                .unwrap()
            })
            .unwrap();
        let messages = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap()
            .into_parts()
            .0;
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != "Target.targetDestroyed"
                    && message["method"] != "Target.targetInfoChanged"),
            "{messages:?}"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count(),
            if reuse_sessions { 0 } else { 2 }
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != "Inspector.detached"
                    || message["sessionId"] != successor_page),
            "{messages:?}"
        );
        handle
            .with_state(|state| {
                let conn = state.connection;
                if !reuse_sessions {
                    for retired in ["SID-retiring-page", "SID-retiring-tab"] {
                        assert!(!conn.has_attached_devtools_session(retired));
                    }
                }
                for current in [successor_page, successor_tab] {
                    assert!(conn.has_attached_devtools_session(current));
                    assert!(!conn.session_is_closing(current));
                }
                assert_eq!(
                    conn.tab_target_id_for_page_target_id("TID-recreated-close"),
                    Some(tab.as_str())
                );
                assert_eq!(
                    conn.primary_session_id_for_tab_target_id(&tab),
                    Some(successor_tab)
                );
                assert!(
                    successor.matches(
                        &conn
                            .page_target_host_identity_for_owner(&CommandOwnerScope::for_session(
                                successor_page
                            ),)
                            .unwrap()
                    )
                );
            })
            .unwrap();
    }
}

#[tokio::test]
async fn intercepted_target_close_retains_cleanup_across_cancelled_and_discarded_waits() {
    use axum::{Router, routing::get};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/",
                    get(|| async { axum::response::Html("<!doctype html><p>ready</p>") }),
                )
                .route("/api", get(|| async { "ok" })),
        )
        .await
        .unwrap();
    });
    for stage in ["Request", "Response"] {
        for failure_phase in [None, Some(false), Some(true)] {
            let mut context = TestContext::new();
            let mut browser = BrowserContext::new("BID-fetch-close".into());
            browser.set_active_target_id("TID-fetch-close");
            browser.attach_active_session("SID-fetch-close");
            assert!(
                browser.assign_attached_session_to_target(
                    "TID-fetch-close",
                    "SID-fetch-attached".into()
                )
            );
            context
                .conn
                .install_browser_context_fixture_for_test(browser);
            let tab = context
                .conn
                .register_top_level_page_target("TID-fetch-close");
            context
                .conn
                .attach_tab_target_session_event_plan("SID-fetch-tab".into(), None, &tab, false)
                .unwrap();
            context
                .install_navigation_fixture_for_session_owner(
                    &format!("http://{address}/"),
                    Some("SID-fetch-close"),
                )
                .await;
            for command in [
                json!({"id":1,"method":"Runtime.enable","sessionId":"SID-fetch-close"}),
                json!({"id":2,"method":"Network.enable","sessionId":"SID-fetch-close"}),
                json!({"id":3,"method":"Fetch.enable","sessionId":"SID-fetch-close","params":{"patterns":[{"urlPattern":"*api","requestStage":stage}]}}),
                json!({"id":4,"method":"Fetch.enable","sessionId":"SID-fetch-attached","params":{"patterns":[{"urlPattern":"*other"}]}}),
            ] {
                let id = command["id"].as_u64().unwrap();
                context.process_async(command).await;
                assert!(context.take_response_by_id(id).get("result").is_some());
            }
            context.sent.clear();
            context
                .process_async(json!({
                    "id":5,"method":"Runtime.evaluate","sessionId":"SID-fetch-close",
                    "params":{"expression":"fetch('/api').catch(() => {}); 'scheduled'"}
                }))
                .await;
            let paused = context
                .wait_for_scheduler_message("intercepted request before target close", |message| {
                    message["method"] == "Fetch.requestPaused"
                        && message["sessionId"] == "SID-fetch-close"
                })
                .await;
            if stage == "Response" {
                assert_eq!(paused["params"]["responseStatusCode"], 200);
            } else {
                assert!(paused["params"]["responseStatusCode"].is_null());
            }
            let scope = CommandOwnerScope::for_session("SID-fetch-close");
            let host = context
                .conn
                .page_target_host_identity_for_owner(&scope)
                .unwrap();
            let owner = ProtocolStateOwner::new(context.conn);
            let handle = owner.handle();
            let command = ParsedCdpCommand::parse_str(r#"{"id":80,"method":"Target.closeTarget","params":{"targetId":"TID-fetch-close"}}"#).unwrap();
            let mut dispatch = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(pending) = handle
                .with_state(|state| {
                    state.start_parsed_command_dispatch_with_context(&command, &mut dispatch)
                })
                .unwrap()
            else {
                panic!("Fetch cleanup must execute outside the state borrow");
            };
            assert!(pending.owns_session_disposal());
            handle
                .with_state(|state| {
                    let conn = state.connection;
                    assert!(
                        host.matches(&conn.page_target_host_identity_for_owner(&scope).unwrap())
                    );
                    assert_eq!(
                        conn.target_fetch_pending_state_is_empty_for_owner(&scope),
                        Some(true)
                    );
                    assert!(
                        !conn
                            .target_session_owner_aggregate_fetch_config_for_owner(&scope)
                            .unwrap()
                            .is_enabled()
                    );
                    for session in ["SID-fetch-close", "SID-fetch-attached", "SID-fetch-tab"] {
                        assert!(conn.session_is_closing(session));
                    }
                    let browser = conn.browser_context.as_mut().unwrap();
                    assert!(!browser.assign_attached_session_to_target(
                        "TID-fetch-close",
                        "SID-too-late".into()
                    ));
                    assert!(!conn.assign_session_to_tab_target(
                        &tab,
                        "SID-tab-too-late".into(),
                        true
                    ));
                })
                .unwrap();
            for command in [
                json!({"id":81,"method":"Page.navigate","params":{"url":"about:blank"}}),
                json!({"id":82,"method":"Runtime.evaluate","sessionId":"SID-fetch-attached","params":{"expression":"42"}}),
                json!({"id":83,"method":"Target.activateTarget","params":{"targetId":"TID-fetch-close"}}),
                json!({"id":84,"method":"Target.closeTarget","params":{"targetId":"TID-fetch-close"}}),
            ] {
                let command = ParsedCdpCommand::parse_str(command.to_string()).unwrap();
                let CdpCommandTaskStep::Complete(outcome) = handle
                    .with_state(|state| {
                        state.start_parsed_command_dispatch_with_context(
                            &command,
                            &mut CommandDispatchContext::default(),
                        )
                    })
                    .unwrap()
                else {
                    panic!("a closing target cannot admit new work");
                };
                assert!(
                    outcome
                        .into_parts()
                        .0
                        .iter()
                        .any(|message| message.get("error").is_some())
                );
            }
            let version =
                ParsedCdpCommand::parse_str(r#"{"id":85,"method":"Browser.getVersion"}"#).unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default()
                    ))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            let completed = if let Some(after_ready) = failure_phase {
                if after_ready {
                    drop(pending.wait().await);
                } else {
                    drop(pending);
                }
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            Some(80),
                            None,
                            crate::OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut dispatch,
                        )
                    })
                    .unwrap()
                else {
                    panic!("recovery retains the original Fetch cleanup");
                };
                recovery.wait().await
            } else {
                pending.wait().await
            };
            handle
                .with_state(|state| {
                    assert!(
                        host.matches(
                            &state
                                .connection
                                .page_target_host_identity_for_owner(&scope)
                                .unwrap()
                        )
                    )
                })
                .unwrap();
            let CdpCommandTaskStep::Complete(outcome) = handle
                .with_state(|state| {
                    state.try_complete_ready_command_dispatch_with_context(completed, &mut dispatch)
                })
                .unwrap()
                .unwrap_or_else(|_| panic!("Fetch close completion is finite"))
            else {
                panic!("all Fetch waits have completed");
            };
            let (events, scheduler, predecessor) = outcome.into_protocol_event_parts();
            assert!(
                predecessor.is_some(),
                "the real failed request retains its renderer predecessor"
            );
            let messages: Vec<_> = events
                .into_iter()
                .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
                .collect();
            let response = messages.iter().find(|message| message["id"] == 80).unwrap();
            if failure_phase.is_some() {
                assert!(response.get("error").is_some());
            } else {
                assert_eq!(response["result"]["success"], true);
            }
            assert!(
                messages
                    .iter()
                    .all(|message| message["method"] != "Target.targetDestroyed"
                        && message["method"] != "Target.detachedFromTarget")
            );
            let work = scheduler.into_iter().find_map(|event| match event {
                CdpSchedulerEvent::ProtocolWorkPublished { work } if work.kind() == crate::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction => Some(work),
                _ => None,
            }).expect("destruction is handed to ordered-ingress owner work");
            handle
                .with_state(|state| {
                    assert!(
                        host.matches(
                            &state
                                .connection
                                .page_target_host_identity_for_owner(&scope)
                                .unwrap()
                        )
                    )
                })
                .unwrap();
            // This test verifies the handoff and retained cleanup authority.
            // Transport smoke checks projection before invoking the owner work.
            drop(predecessor);
            let pending = handle
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap()
                .unwrap();
            let completed = pending.wait().await;
            let messages = handle
                .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                .unwrap()
                .into_parts()
                .0;
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message["method"] == "Target.detachedFromTarget")
                    .count(),
                3
            );
            handle
                .with_state(|state| {
                    for session in ["SID-fetch-close", "SID-fetch-attached", "SID-fetch-tab"] {
                        assert!(!state.connection.has_attached_devtools_session(session));
                    }
                    assert!(
                        !state
                            .connection
                            .browser_context
                            .as_ref()
                            .unwrap()
                            .has_active_target()
                    );
                })
                .unwrap();
        }
    }
    server.abort();
}

#[tokio::test]
async fn late_closed_session_completion_cannot_detach_an_already_retired_binding() {
    use crate::conn::TargetClosureCleanupPlan;
    for rebind in [false, true] {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-close-binding".into());
        browser.set_active_target_id("TID-old-binding");
        browser.attach_active_session("SID-original-binding");
        browser.stage_background_target(
            "TID-new-binding".into(),
            None,
            "about:blank".into(),
            None,
            None,
        );
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        let tab = context
            .conn
            .register_top_level_page_target("TID-new-binding");
        let plan = TargetClosureCleanupPlan::new(
            "TID-old-binding",
            Some("Render process gone."),
            ["SID-original-binding".into()],
        );
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let pending = handle
            .with_state(|state| state.prepare_closed_target_sessions(plan.clone(), None))
            .unwrap();
        let completed = pending.wait().await;
        handle
            .with_state(|state| {
                let conn = state.connection;
                let first = conn
                    .commit_target_closure_session_detachment_events(plan, None)
                    .into_background_events();
                assert_eq!(first.len(), 1);
                if rebind {
                    conn.attach_tab_target_session_event_plan(
                        "SID-original-binding".into(),
                        None,
                        &tab,
                        false,
                    )
                    .unwrap();
                    assert!(!conn.session_is_closing("SID-original-binding"));
                }
            })
            .unwrap();
        for completed in [completed.clone(), completed] {
            let events = handle
                .with_state(|state| state.finish_closed_target_sessions(completed))
                .unwrap()
                .into_background_events();
            assert!(
                events.is_empty(),
                "a peer already detached this exact binding"
            );
            handle
                .with_state(|state| {
                    let conn = state.connection;
                    if rebind {
                        assert_eq!(
                            conn.session_route(Some("SID-original-binding"))
                                .unwrap()
                                .target_id(),
                            Some(tab.as_str())
                        );
                        assert!(!conn.session_is_closing("SID-original-binding"));
                    } else {
                        assert!(!conn.has_attached_devtools_session("SID-original-binding"));
                    }
                })
                .unwrap();
        }
    }
}
