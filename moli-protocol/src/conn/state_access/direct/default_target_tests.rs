use super::*;
use crate::conn::{
    BackgroundProtocolEvent, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
};
use futures_util::FutureExt as _;
use serde_json::json;

#[tokio::test]
async fn placeholder_close_keeps_native_cleanup_owned_across_waiter_and_ready_loss() {
    for phase in ["normal", "before", "waiting", "ready"] {
        moli_v8_init::ensure_v8_initialized(moli_v8_platform::create_platform);
        let (task_tx, mut tasks) = tokio::sync::mpsc::unbounded_channel();
        let mut isolate = v8::Isolate::new(Default::default());
        let registration = moli_v8_platform::V8PlatformIsolateRegistration::register(
            &mut isolate,
            moli_v8_platform::V8ForegroundTaskWake::queued(move |task| {
                let _ = task_tx.send(task);
            }),
            moli_v8_platform::ProcessEnvironmentNotifications::default().notifier(|| {}),
        );
        let mut conn = CdpConnection::new();
        conn.publish_default_browser_target();
        assert!(conn.assign_session_to_tab_target(
            crate::DEFAULT_CDP_TAB_TARGET_ID,
            "SID-placeholder".into(),
            false
        ));
        conn.register_session_route_for_test(
            "SID-placeholder",
            crate::conn::CdpSessionRoute::TabTarget {
                browser_context_id: crate::conn::target::DEFAULT_BROWSER_CONTEXT_ID.into(),
                tab_target_id: crate::DEFAULT_CDP_TAB_TARGET_ID.into(),
            },
        );
        let discover = conn.start_command_dispatch(
            &json!({"id":69,"method":"Target.setDiscoverTargets",
                "params":{"discover":true,"filter":[{}]},
            })
            .to_string(),
        );
        assert!(matches!(discover, CdpCommandTaskStep::Complete(_)));
        assert!(conn.default_target_lifecycle.is_placeholder());
        assert!(conn.browser_context.is_none());
        let owner = ProtocolStateOwner::new(conn);
        let handle = owner.handle();
        let start = ParsedCdpCommand::parse_value(json!({"id":70,"method":"Tracing.start","sessionId":"SID-placeholder",
            "params":{"traceConfig":{"includedCategories":["disabled-by-default-v8.cpu_profiler"]}},
        })).unwrap();
        let mut trace_context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&start, &mut trace_context)
            })
            .unwrap()
        else {
            panic!("CPU tracing owns a native start acknowledgement");
        };
        let completed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let wait = pending.wait();
            tokio::pin!(wait);
            loop {
                tokio::select! {
                    completed = &mut wait => break completed,
                    task = tasks.recv() => assert!(task.unwrap().run()),
                }
            }
        })
        .await
        .unwrap();
        let CdpCommandTaskStep::Complete(started) = handle
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut trace_context)
            })
            .unwrap()
        else {
            panic!("the native start has settled");
        };
        assert!(
            started
                .into_protocol_event_parts()
                .0
                .iter()
                .any(|event| event.clone().into_protocol_message()["id"] == 70)
        );

        let close = ParsedCdpCommand::parse_value(json!({"id":71,"method":"Target.closeTarget","params":{"targetId":crate::DEFAULT_CDP_PAGE_TARGET_ID}})).unwrap();
        let mut context = CommandDispatchContext::default();
        let CdpCommandTaskStep::Pending(pending) = handle
            .with_state(|state| {
                state.start_parsed_command_dispatch_with_context(&close, &mut context)
            })
            .unwrap()
        else {
            panic!("a placeholder's session cleanup belongs to an owned workflow");
        };
        assert_eq!(pending.kind_name(), "Workflow");
        let stop = tokio::time::timeout(std::time::Duration::from_secs(5), tasks.recv())
            .await
            .unwrap()
            .unwrap();
        let mut wait = Box::pin(pending.wait());
        if phase != "before" {
            assert!(wait.as_mut().now_or_never().is_none());
        }
        let version =
            ParsedCdpCommand::parse_value(json!({"id":72,"method":"Browser.getVersion"})).unwrap();
        handle
            .with_state(|state| {
                let conn = state.connection;
                assert!(conn.has_attached_devtools_session("SID-placeholder"));
                assert!(conn.session_is_closing("SID-placeholder"));
                assert!(conn.default_target_lifecycle.is_closed());
                conn.ensure_default_target_live(crate::DEFAULT_CDP_PAGE_TARGET_ID);
                conn.publish_default_browser_target();
                assert!(
                    conn.browser_context.is_none(),
                    "close admission forbids creating a late Page"
                );
                assert!(matches!(
                    conn.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default()
                    ),
                    CdpCommandTaskStep::Complete(_)
                ));
            })
            .unwrap();
        if phase == "waiting" {
            assert!(wait.as_mut().now_or_never().is_none());
        }
        let completed = if matches!(phase, "normal" | "ready") {
            assert!(stop.run());
            let completed = wait.await;
            if phase == "ready" {
                drop(completed);
                None
            } else {
                Some(completed)
            }
        } else {
            drop(wait);
            let CdpCommandTaskStep::Pending(recovery) = handle
                .with_state(|state| {
                    state.complete_failed_command_wait_with_context(
                        Some(71),
                        None,
                        crate::OwnedProtocolWaitFailure::Cancelled.message(),
                        &mut context,
                    )
                })
                .unwrap()
            else {
                panic!("the original command continues its admitted cleanup");
            };
            let mut waiting = Box::pin(recovery.wait());
            assert!(
                waiting.as_mut().now_or_never().is_none(),
                "cancellation cannot finish before the real native stop"
            );
            assert!(stop.run());
            Some(waiting.await)
        };
        let completed = match completed {
            Some(completed) => completed,
            None => {
                let CdpCommandTaskStep::Pending(recovery) = handle
                    .with_state(|state| {
                        state.complete_failed_command_wait_with_context(
                            Some(71),
                            None,
                            crate::OwnedProtocolWaitFailure::Cancelled.message(),
                            &mut context,
                        )
                    })
                    .unwrap()
                else {
                    panic!("ready loss returns the same cleanup and output context");
                };
                recovery.wait().await
            }
        };
        let CdpCommandTaskStep::Complete(outcome) = handle
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut context)
            })
            .unwrap()
        else {
            panic!("all native cleanup has finished");
        };
        let messages: Vec<_> = outcome
            .into_protocol_event_parts()
            .0
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect();
        let replies: Vec<_> = messages.iter().filter(|m| m["id"] == 71).collect();
        assert_eq!(replies.len(), 1, "{phase}: {messages:?}");
        assert_eq!(replies[0]["result"], json!({"success":true}));
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["method"] == "Target.detachedFromTarget"
                    && m["params"]["sessionId"] == "SID-placeholder")
                .count(),
            1,
            "{phase}: {messages:?}"
        );
        for target in [
            crate::DEFAULT_CDP_PAGE_TARGET_ID,
            crate::DEFAULT_CDP_TAB_TARGET_ID,
        ] {
            assert_eq!(
                messages
                    .iter()
                    .filter(|m| m["method"] == "Target.targetDestroyed"
                        && m["params"]["targetId"] == target)
                    .count(),
                1,
                "{phase}: {messages:?}"
            );
        }
        handle
            .with_state(|state| {
                assert!(
                    !state
                        .connection
                        .has_attached_devtools_session("SID-placeholder")
                );
                assert!(state.connection.browser_context.is_none());
                assert!(state.connection.default_target_lifecycle.is_closed());
            })
            .unwrap();
        assert!(context.take_command_failure_cleanup().is_none());
        registration.unregister();
        drop(isolate);
    }
}
