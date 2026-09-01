use std::{future::Future, pin::pin, task::Poll};

use serde_json::{Value, json};

use crate::conn::{BrowserContext, CdpCommandTaskStep, CdpSessionRoute};
use crate::testing::TestContext;

fn context_with_page_sessions() -> TestContext {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-tracing".to_owned());
    browser_context.set_active_target_id("TID-tracing".to_owned());
    browser_context.set_target_url("https://example.test/page#fragment".to_owned());
    browser_context.attach_active_session("SID-owner".to_owned());
    assert!(
        browser_context.assign_auxiliary_session_to_target("TID-tracing", "SID-peer".to_owned(),)
    );
    ctx.conn.browser_context = Some(browser_context);
    ctx
}

async fn drive_owner_tasks_until<F>(
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<moli_v8_platform::V8ForegroundTask>,
    future: F,
) -> (F::Output, usize)
where
    F: Future,
{
    let mut future = pin!(future);
    let mut tasks_run = 0;
    loop {
        let next = std::future::poll_fn(|cx| {
            if let Poll::Ready(output) = future.as_mut().poll(cx) {
                return Poll::Ready(Ok(output));
            }
            receiver
                .poll_recv(cx)
                .map(|task| Err(task.expect("isolate owner task route should remain open")))
        })
        .await;
        match next {
            Ok(output) => return (output, tasks_run),
            Err(task) => {
                assert!(task.run(), "live isolate owner task should execute");
                tasks_run += 1;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn report_events_are_flushed_after_end_response_with_real_metadata() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 1,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
        "params": {
            "categories": "__metadata,disabled-by-default-devtools.timeline",
            "transferMode": "ReportEvents",
        },
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-owner"));

    ctx.process_async(json!({
        "id": 2,
        "method": "Tracing.recordClockSyncMarker",
        "sessionId": "SID-peer",
        "params": { "syncId": "peer-marker" },
    }))
    .await;
    ctx.expect_result(2, json!({}), Some("SID-peer"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;

    assert_eq!(
        ctx.take_one(),
        json!({ "id": 3, "result": {}, "sessionId": "SID-owner" })
    );
    let data = ctx.take_one();
    assert_eq!(data["method"], "Tracing.dataCollected");
    assert_eq!(data["sessionId"], "SID-owner");
    let events = data["params"]["value"]
        .as_array()
        .expect("Tracing.dataCollected value array");
    assert!(events.iter().any(|event| event["name"] == "process_name"));
    assert!(events.iter().any(|event| event["name"] == "thread_name"));
    let started = events
        .iter()
        .find(|event| event["name"] == "TracingStartedInBrowser")
        .expect("real frame-tree trace event");
    assert_eq!(started["args"]["data"]["frames"][0]["frame"], "TID-tracing");
    assert_eq!(
        started["args"]["data"]["frames"][0]["url"],
        "https://example.test/page"
    );
    assert!(events.iter().any(|event| {
        event["name"] == "clock_sync" && event["args"]["sync_id"] == "peer-marker"
    }));
    assert_eq!(
        ctx.take_one(),
        json!({
            "method": "Tracing.tracingComplete",
            "params": { "dataLossOccurred": false },
            "sessionId": "SID-owner",
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn synchronous_start_response_is_claimed_before_tracing_end() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 4,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
        "params": {
            "categories": "__metadata",
            "transferMode": "ReportEvents",
        },
    }))
    .await;
    ctx.expect_result(4, json!({}), Some("SID-owner"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    let messages = ctx.take_all();

    assert_eq!(
        messages
            .iter()
            .filter(|message| message["id"] == json!(5))
            .count(),
        1,
        "Tracing.end must receive exactly one response: {messages:?}"
    );
    assert!(
        messages.iter().all(|message| message["id"] != json!(4)),
        "an already-acknowledged synchronous Tracing.start must not receive a second response: \
         {messages:?}"
    );
    assert_eq!(
        messages.last().and_then(|message| message.get("method")),
        Some(&json!("Tracing.tracingComplete"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tracing_is_browser_global_but_only_the_starting_session_can_end_it() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 10,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
    }))
    .await;
    ctx.expect_result(10, json!({}), Some("SID-owner"));

    for (id, session_id) in [(11, "SID-owner"), (12, "SID-peer")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Tracing.start",
            "sessionId": session_id,
        }))
        .await;
        ctx.expect_error(
            id,
            -32000,
            "Tracing has already been started (possibly in another tab).",
        );
    }
    ctx.process_async(json!({
        "id": 13,
        "method": "Tracing.end",
        "sessionId": "SID-peer",
    }))
    .await;
    ctx.expect_error(13, -32000, "Tracing is not started");
    assert!(ctx.conn.tracing_state.is_active());

    ctx.process_async(json!({
        "id": 14,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    ctx.expect_result(14, json!({}), Some("SID-owner"));
    assert!(!ctx.conn.tracing_state.is_active());
}

#[tokio::test(flavor = "multi_thread")]
async fn return_as_stream_is_valid_json_readable_through_io() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 20,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
        "params": {
            "transferMode": "ReturnAsStream",
            "streamFormat": "json",
            "streamCompression": "none",
            "traceConfig": {
                "recordMode": "recordContinuously",
                "includedCategories": ["moli.devtools"],
                "excludedCategories": ["*"],
            },
        },
    }))
    .await;
    ctx.expect_result(20, json!({}), Some("SID-owner"));
    ctx.process_async(json!({
        "id": 21,
        "method": "Tracing.getCategories",
        "sessionId": "SID-owner",
    }))
    .await;
    let categories = ctx.take_response_by_id(21);
    assert!(categories["result"]["categories"].is_array());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    assert_eq!(
        ctx.take_one(),
        json!({ "id": 22, "result": {}, "sessionId": "SID-owner" })
    );
    let complete = ctx.take_one();
    assert_eq!(complete["method"], "Tracing.tracingComplete");
    assert_eq!(complete["params"]["traceFormat"], "json");
    assert_eq!(complete["params"]["streamCompression"], "none");
    let handle = complete["params"]["stream"]
        .as_str()
        .expect("trace stream handle")
        .to_owned();

    ctx.process_async(json!({
        "id": 23,
        "method": "IO.read",
        "sessionId": "SID-owner",
        "params": { "handle": handle },
    }))
    .await;
    let read = ctx.take_response_by_id(23);
    assert_eq!(read["result"]["eof"], true);
    let trace: Value = serde_json::from_str(
        read["result"]["data"]
            .as_str()
            .expect("UTF-8 trace stream data"),
    )
    .expect("valid trace JSON");
    let events = trace["traceEvents"].as_array().expect("traceEvents array");
    assert!(events.iter().any(|event| {
        event["name"] == "MoliDevToolsCommand" && event["args"]["method"] == "Tracing.getCategories"
    }));
    assert_eq!(trace["metadata"]["product"], "moli");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_cleanup_cancels_trace_without_emitting_completion() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 30,
        "method": "Tracing.start",
        "sessionId": "SID-peer",
    }))
    .await;
    ctx.expect_result(30, json!({}), Some("SID-peer"));
    ctx.sent.clear();

    assert!(
        ctx.conn
            .execute_target_binding_cleanup_for_session_without_event_async("SID-peer")
            .await
            .unwrap()
    );
    assert!(!ctx.conn.tracing_state.is_active());
    assert!(
        ctx.sent.is_empty(),
        "owner cleanup must not flush trace output"
    );

    ctx.process_async(json!({
        "id": 31,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
    }))
    .await;
    ctx.expect_result(31, json!({}), Some("SID-owner"));
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_cleanup_waits_for_cpu_trace_release_before_replacement_start() {
    moli_v8_init::ensure_v8_initialized(moli_v8_platform::create_platform);
    let (owner_task_tx, mut owner_task_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut isolate = v8::Isolate::new(Default::default());
    let registration = moli_v8_platform::V8PlatformIsolateRegistration::register(
        &mut isolate,
        moli_v8_platform::V8ForegroundTaskWake::queued(move |task| {
            let _ = owner_task_tx.send(task);
        }),
    );
    let mut ctx = context_with_page_sessions();

    let start = ctx.conn.start_command_dispatch(
        &json!({
            "id": 32,
            "method": "Tracing.start",
            "sessionId": "SID-peer",
            "params": {
                "traceConfig": {
                    "includedCategories": ["disabled-by-default-v8.cpu_profiler"]
                }
            },
        })
        .to_string(),
    );
    let pending_start = match start {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => panic!("CPU Tracing.start must be pending"),
    };
    let (completed_start, start_tasks) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        drive_owner_tasks_until(&mut owner_task_rx, pending_start.wait()),
    )
    .await
    .expect("initial CPU trace should start");
    assert!(start_tasks > 0);
    let start_step = ctx
        .conn
        .complete_pending_command_dispatch(completed_start)
        .await;
    assert_eq!(
        start_step.into_parts().0,
        vec![json!({
            "id": 32,
            "result": {},
            "sessionId": "SID-peer",
        })]
    );

    let (cleaned, stop_tasks) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        drive_owner_tasks_until(
            &mut owner_task_rx,
            ctx.conn
                .execute_target_binding_cleanup_for_session_without_event_async("SID-peer"),
        ),
    )
    .await
    .expect("owner detach should wait for CPU trace cancellation");
    assert!(cleaned.unwrap());
    assert!(stop_tasks > 0);
    assert!(!ctx.conn.tracing_state.is_active());

    let replacement_start = ctx.conn.start_command_dispatch(
        &json!({
            "id": 33,
            "method": "Tracing.start",
            "sessionId": "SID-owner",
            "params": {
                "traceConfig": {
                    "includedCategories": ["disabled-by-default-v8.cpu_profiler"]
                }
            },
        })
        .to_string(),
    );
    let pending_replacement = match replacement_start {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(plan) => {
            panic!("replacement CPU trace should start after detach: {plan:?}")
        }
    };
    let (completed_replacement, replacement_start_tasks) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        drive_owner_tasks_until(&mut owner_task_rx, pending_replacement.wait()),
    )
    .await
    .expect("replacement CPU trace should start");
    assert!(replacement_start_tasks > 0);
    let replacement_step = ctx
        .conn
        .complete_pending_command_dispatch(completed_replacement)
        .await;
    assert_eq!(
        replacement_step.into_parts().0,
        vec![json!({
            "id": 33,
            "result": {},
            "sessionId": "SID-owner",
        })]
    );

    let (cancelled, replacement_stop_tasks) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        drive_owner_tasks_until(
            &mut owner_task_rx,
            ctx.conn
                .cancel_tracing_for_session_owner_async(Some("SID-owner")),
        ),
    )
    .await
    .expect("replacement CPU trace should cancel cleanly");
    assert!(cancelled);
    assert!(replacement_stop_tasks > 0);

    registration.unregister();
    drop(isolate);
}

#[tokio::test(flavor = "multi_thread")]
async fn bounded_trace_reports_data_loss_instead_of_growing_without_limit() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 40,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
        "params": {
            "transferMode": "ReportEvents",
            "traceConfig": {
                "traceBufferSizeInKb": 1,
                "includedCategories": ["__metadata"],
            },
        },
    }))
    .await;
    ctx.expect_result(40, json!({}), Some("SID-owner"));
    ctx.process_async(json!({
        "id": 41,
        "method": "Tracing.recordClockSyncMarker",
        "sessionId": "SID-owner",
        "params": { "syncId": "x".repeat(4096) },
    }))
    .await;
    ctx.expect_result(41, json!({}), Some("SID-owner"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 42,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    let complete = ctx.take_first_matching("Tracing completion", |message| {
        message["method"] == "Tracing.tracingComplete"
    });
    assert_eq!(complete["params"]["dataLossOccurred"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_trace_encodings_fail_explicitly() {
    let mut ctx = context_with_page_sessions();
    for (id, params, message) in [
        (
            50,
            json!({ "transferMode": "ReportEvents", "streamFormat": "proto" }),
            "Proto format is only supported when using stream transfer mode.",
        ),
        (
            51,
            json!({ "transferMode": "ReturnAsStream", "streamFormat": "proto" }),
            "Proto trace streams are not supported.",
        ),
        (
            52,
            json!({ "transferMode": "ReturnAsStream", "streamCompression": "gzip" }),
            "Gzip trace streams are not supported.",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Tracing.start",
            "sessionId": "SID-owner",
            "params": params,
        }))
        .await;
        ctx.expect_error(id, -32602, message);
    }
    assert!(!ctx.conn.tracing_state.is_active());
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_enable_sampling_is_accepted_like_chromium() {
    let mut ctx = context_with_page_sessions();
    ctx.process_async(json!({
        "id": 60,
        "method": "Tracing.start",
        "sessionId": "SID-owner",
        "params": {
            "transferMode": "ReportEvents",
            "traceConfig": {
                "includedCategories": [
                    "devtools.timeline",
                    "disabled-by-default-v8.cpu_profiler"
                ],
                "enableSampling": true,
            },
        },
    }))
    .await;
    ctx.expect_result(60, json!({}), Some("SID-owner"));

    ctx.process_async(json!({
        "id": 61,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    ctx.expect_result(61, json!({}), Some("SID-owner"));
    let complete = ctx.take_first_matching("Tracing completion", |message| {
        message["method"] == "Tracing.tracingComplete"
    });
    assert_eq!(complete["params"]["dataLossOccurred"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn cpu_trace_start_response_waits_for_existing_isolate_ack() {
    moli_v8_init::ensure_v8_initialized(moli_v8_platform::create_platform);
    let (owner_task_tx, mut owner_task_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut isolate = v8::Isolate::new(Default::default());
    let registration = moli_v8_platform::V8PlatformIsolateRegistration::register(
        &mut isolate,
        moli_v8_platform::V8ForegroundTaskWake::queued(move |task| {
            let _ = owner_task_tx.send(task);
        }),
    );
    let mut ctx = context_with_page_sessions();

    let start = ctx.conn.start_command_dispatch(
        &json!({
            "id": 70,
            "method": "Tracing.start",
            "sessionId": "SID-owner",
            "params": {
                "traceConfig": {
                    "includedCategories": ["disabled-by-default-v8.cpu_profiler"]
                }
            },
        })
        .to_string(),
    );
    let pending = match start {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("CPU Tracing.start must wait for the existing isolate start ack")
        }
    };
    assert_eq!(pending.kind_name(), "Tracing");

    let owner_task = tokio::time::timeout(std::time::Duration::from_secs(2), owner_task_rx.recv())
        .await
        .expect("CPU profiler start callback should reach the isolate owner")
        .expect("isolate owner task route should remain open");
    assert!(owner_task.run());

    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), pending.wait())
        .await
        .expect("Tracing.start should complete after the data-source ack");
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, scheduler_events) = step.into_parts();
    assert!(scheduler_events.is_empty());
    assert_eq!(
        messages,
        vec![json!({
            "id": 70,
            "result": {},
            "sessionId": "SID-owner",
        })]
    );

    registration.unregister();
    ctx.process_async(json!({
        "id": 71,
        "method": "Tracing.end",
        "sessionId": "SID-owner",
    }))
    .await;
    ctx.expect_result(71, json!({}), Some("SID-owner"));
    drop(isolate);
}

#[tokio::test(flavor = "multi_thread")]
async fn cpu_trace_start_reports_chromium_error_when_stopped_before_ack() {
    moli_v8_init::ensure_v8_initialized(moli_v8_platform::create_platform);
    let (owner_task_tx, mut owner_task_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut isolate = v8::Isolate::new(Default::default());
    let registration = moli_v8_platform::V8PlatformIsolateRegistration::register(
        &mut isolate,
        moli_v8_platform::V8ForegroundTaskWake::queued(move |task| {
            let _ = owner_task_tx.send(task);
        }),
    );
    let mut ctx = context_with_page_sessions();

    let start = ctx.conn.start_command_dispatch(
        &json!({
            "id": 72,
            "method": "Tracing.start",
            "sessionId": "SID-owner",
            "params": {
                "traceConfig": {
                    "includedCategories": ["disabled-by-default-v8.cpu_profiler"]
                }
            },
        })
        .to_string(),
    );
    let pending_start = match start {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => panic!("CPU Tracing.start must be pending"),
    };
    let end = ctx.conn.start_command_dispatch(
        &json!({
            "id": 73,
            "method": "Tracing.end",
            "sessionId": "SID-owner",
        })
        .to_string(),
    );
    let pending_end = match end {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => panic!("CPU Tracing.end must wait for owner cleanup"),
    };

    let owner_task = tokio::time::timeout(std::time::Duration::from_secs(2), owner_task_rx.recv())
        .await
        .expect("stopped CPU profiler start callback should reach the isolate owner")
        .expect("isolate owner task route should remain open");
    assert!(owner_task.run());
    let completed_end = tokio::time::timeout(std::time::Duration::from_secs(2), pending_end.wait())
        .await
        .expect("Tracing.end should complete after the owner callback cleans up");
    let end_step = ctx
        .conn
        .complete_pending_command_dispatch(completed_end)
        .await;
    let (end_messages, scheduler_events) = end_step.into_parts();
    assert!(scheduler_events.is_empty());
    assert_eq!(end_messages[0]["id"], 73);
    assert_eq!(end_messages[0]["result"], json!({}));
    assert_eq!(
        end_messages[1],
        json!({
            "id": 72,
            "error": {
                "code": -32000,
                "message": "Tracing was stopped before start has been completed."
            },
            "sessionId": "SID-owner",
        })
    );
    assert!(
        end_messages[2..]
            .iter()
            .any(|message| { message.get("method") == Some(&json!("Tracing.dataCollected")) }),
        "trace data must follow the end response and interrupted start error"
    );
    assert_eq!(
        end_messages
            .last()
            .and_then(|message| message.get("method")),
        Some(&json!("Tracing.tracingComplete"))
    );

    let completed_start = pending_start.wait().await;
    let start_step = ctx
        .conn
        .complete_pending_command_dispatch(completed_start)
        .await;
    let (start_messages, scheduler_events) = start_step.into_parts();
    assert!(scheduler_events.is_empty());
    assert!(
        start_messages.is_empty(),
        "Tracing.end already emitted the interrupted start response"
    );
    assert!(!ctx.conn.tracing_state.is_active());

    registration.unregister();
    drop(isolate);
}

#[test]
fn tracing_owner_scope_distinguishes_flattened_sessions() {
    let ctx = context_with_page_sessions();
    assert_eq!(
        ctx.conn.session_route(Some("SID-peer")),
        Some(CdpSessionRoute::PageTarget {
            browser_context_id: "BID-tracing".to_owned(),
            target_id: "TID-tracing".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Attached("SID-peer".to_owned()),
        })
    );
    assert_ne!(
        ctx.conn.tracing_owner_scope(Some("SID-owner")),
        ctx.conn.tracing_owner_scope(Some("SID-peer")),
    );
}
