use super::*;

#[tokio::test(flavor = "current_thread")]
async fn mouse_input_completion_publishes_handler_navigation_once() {
    let runtime = initialize_layout_test_runtime();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    for (listener, event_name, microtask, navigates, cancel) in [
        ("mousedown", "mousedown", false, true, false),
        ("pointerdown", "mousedown", false, true, false),
        ("mousemove", "mousemove", false, true, false),
        ("pointermove", "mousemove", false, true, false),
        ("mousedown", "mousedown", true, true, false),
        ("pointermove", "mousemove", true, true, false),
        ("mousedown", "mousedown", false, true, true),
        ("pointerdown", "mousedown", true, true, true),
        ("mousedown", "mousedown", false, false, false),
        ("mousemove", "mousemove", true, false, true),
    ] {
        let body = if navigates {
            "location.href = 'https://example.test/next.html';"
        } else {
            "window.handlerRan = true;"
        };
        let handler = if microtask {
            format!("queueMicrotask(() => {{ {body} }});")
        } else {
            body.to_owned()
        };
        let html = format!(
            "<!doctype html><button id='target' style='width:200px;height:80px'>go</button><script>window.handlerRuns=0;target.addEventListener('{listener}', (event) => {{ window.handlerRuns++; if ({cancel}) event.preventDefault(); {handler} }}, {{once:true}});window.handlerInstalled=true;</script>"
        );
        let mut page = create_test_html_page_with_navigation_dispatch(
            &runtime,
            &loader,
            url::Url::parse("https://example.test/source.html").unwrap(),
            &html,
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
        )
        .await;
        capture_screenshot_for_renderer_page(&page).await;
        let (snapshot, _) = page
            .run_async_command(RendererPageCommand::PageDiagnosticsSnapshot)
            .await
            .expect("source snapshot");
        let RendererPageReply::PageDiagnosticsSnapshot(snapshot) = snapshot else {
            panic!("expected diagnostics snapshot");
        };
        let source_document = snapshot
            .document_lifecycle_identity()
            .expect("source identity");
        let (preflight, _) = page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify([document.elementFromPoint(30,30).id,window.handlerInstalled,window.handlerRuns])".to_owned(),
            await_promise: false,
        }).await.expect("input fixture preflight");
        assert_eq!(
            mouse_input_json_witness(&preflight),
            serde_json::json!(["target", true, 0])
        );
        output_rx.drain();
        let (reply, _) = page
            .run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
                x: 30.0,
                y: 30.0,
                event_name: event_name.to_owned(),
                button: if event_name == "mousedown" { 0 } else { -1 },
                buttons: Some(if event_name == "mousedown" { 1 } else { 0 }),
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
                pointer: RendererPointerEventProperties::default(),
                modifiers: 0,
            })
            .await
            .expect("input command");
        let RendererPageReply::InputDispatchOutcome(outcome) = reply else {
            panic!("expected input outcome");
        };
        assert_eq!(
            outcome.triggered_top_level_navigation, navigates,
            "{listener}, microtask={microtask}"
        );
        let navigations = output_rx
            .drain()
            .into_iter()
            .flat_map(RendererOutputPublication::into_records)
            .filter_map(|record| match record.into_parts().1 {
                RendererOutputItem::OwnerAction(
                    RendererOwnerAction::TopLevelLocationNavigation(navigation),
                ) => Some(navigation),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            navigations.len(),
            usize::from(navigates),
            "{listener}, microtask={microtask}"
        );
        if let Some(navigation) = navigations.first() {
            assert_eq!(navigation.source_document(), source_document);
            assert_eq!(navigation.url(), "https://example.test/next.html");
        }
        assert_eq!(
            has_pending_location_navigation_for_test(&page).await,
            Some(false),
            "completed input must not leave navigation for a later Runtime command"
        );
        // Read only after the command's navigation-output assertions: a
        // Runtime command must not make a broken input publication look green.
        let (handler_runs, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: "window.handlerRuns".to_owned(),
                await_promise: false,
            })
            .await
            .expect("handler execution witness");
        assert_eq!(
            renderer_json_value(handler_runs),
            Some(serde_json::json!(1))
        );
        assert!(
            output_rx
                .drain()
                .into_iter()
                .flat_map(RendererOutputPublication::into_records)
                .all(|record| !matches!(
                    record.item(),
                    RendererOutputItem::OwnerAction(
                        RendererOwnerAction::TopLevelLocationNavigation(_)
                    )
                )),
            "the next command must not republish the input navigation"
        );
        page.close_async().await.expect("close input fixture");
    }
}
#[tokio::test(flavor = "current_thread")]
async fn mouse_input_completion_preserves_standalone_and_child_navigation_scope() {
    let runtime = initialize_layout_test_runtime();
    let (output_tx, mut output_rx) = renderer_external_activity_test_channel();
    runtime.set_renderer_output_transport_sender(output_tx);
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    for (case, mode) in [
        (
            "standalone",
            RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
        ),
        (
            "child self",
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
        ),
        (
            "child top",
            RendererTopLevelNavigationDispatch::DelegateToBrowser,
        ),
    ] {
        let html = if case == "standalone" {
            "<!doctype html><button id='target' style='width:200px;height:80px' onmousedown=\"location.href='data:text/html,next'\">go</button>".to_owned()
        } else {
            let navigation = if case == "child top" {
                "top.location.href='https://example.test/next.html'"
            } else {
                "location.href='data:text/html,child-next'"
            };
            let child = format!(
                "<!doctype html><button id='target' style='width:200px;height:80px' onmousedown=\"parent.handlerRuns++;{navigation}\">go</button>"
            );
            let child_literal = serde_json::to_string(&child).expect("child HTML literal");
            format!(
                "<!doctype html><script>window.handlerRuns=0;</script><iframe id='frame' style='position:absolute;left:0;top:0;border:0;width:400px;height:200px'></iframe><script>frame.srcdoc={child_literal};</script>"
            )
        };
        let mut page = create_test_html_page_with_navigation_dispatch(
            &runtime,
            &loader,
            url::Url::parse("https://example.test/source.html").unwrap(),
            &html,
            mode,
        )
        .await;
        capture_screenshot_for_renderer_page(&page).await;
        let hit_expression = if case == "standalone" {
            "document.elementFromPoint(30,30).id"
        } else {
            "document.getElementById('frame').contentDocument.elementFromPoint(30,30).id"
        };
        let (hit, _) = page
            .run_async_command(RendererPageCommand::EvaluateExpression {
                expression: hit_expression.to_owned(),
                await_promise: false,
            })
            .await
            .expect("scope fixture hit preflight");
        assert_eq!(
            renderer_json_value(hit),
            Some(serde_json::json!("target")),
            "{case}"
        );
        page.run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "document.elementFromPoint(0, 0); true".to_owned(),
            await_promise: false,
        })
        .await
        .expect("publish root geometry before mouse input");

        output_rx.drain();
        let (reply, _) = page
            .run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
                x: 30.0,
                y: 30.0,
                event_name: "mousedown".to_owned(),
                button: 0,
                buttons: Some(1),
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
                pointer: RendererPointerEventProperties::default(),
                modifiers: 0,
            })
            .await
            .expect("scope input command");
        let RendererPageReply::InputDispatchOutcome(outcome) = reply else {
            panic!("expected input outcome");
        };
        assert_eq!(
            outcome.triggered_top_level_navigation,
            case != "child self",
            "{case}"
        );
        let navigations = output_rx
            .drain()
            .into_iter()
            .flat_map(RendererOutputPublication::into_records)
            .filter_map(|record| match record.into_parts().1 {
                RendererOutputItem::OwnerAction(
                    RendererOwnerAction::TopLevelLocationNavigation(navigation),
                ) => Some(navigation),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            navigations.len(),
            usize::from(case == "child top"),
            "{case}"
        );
        if let Some(navigation) = navigations.first() {
            assert_eq!(navigation.url(), "https://example.test/next.html");
        }
        if case != "standalone" {
            let (witness, _) = page
                .run_async_command(RendererPageCommand::EvaluateExpression {
                    expression:
                        "JSON.stringify([window.handlerRuns,!!document.getElementById('frame')])"
                            .to_owned(),
                    await_promise: false,
                })
                .await
                .expect("child input witness after publication assertions");
            assert_eq!(
                mouse_input_json_witness(&witness),
                serde_json::json!([1, true]),
                "{case}"
            );
        }
        page.close_async().await.expect("close scope fixture");
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn owner_scheduler_applies_a_wheel_batch_at_the_fixed_action_window_deadline() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let (base_url, effect_request_seen, release_effect_response, effect_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/action-window-scroll-applied",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        r#"<!doctype html>
<style>
html, body { margin: 0; }
body { height: 1200px; }
#target { position: absolute; top: 250px; width: 20px; height: 20px; }
</style>
<div id="target"></div>
<script>
globalThis.__lmActionWindowWheelLog = [];
globalThis.__lmActionWindowIoLog = [];
addEventListener("wheel", event => {
  __lmActionWindowWheelLog.push("event:" + event.deltaY);
  Promise.resolve().then(() => {
    __lmActionWindowWheelLog.push("microtask:" + event.deltaY);
  });
}, { capture: true });
</script>"#,
    )
    .await;
    page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
        crate::protocol_types::ViewportSurface {
            inner_width: 200,
            inner_height: 200,
            outer_width: 200,
            outer_height: 200,
            device_pixel_ratio: 1.0,
            screen_width: 200,
            screen_height: 200,
            screen_avail_width: 200,
            screen_avail_height: 200,

            ..Default::default()
        },
    )))
    .await
    .expect("action-window viewport should update");
    capture_screenshot_for_renderer_page(&page).await;
    let (observer_installed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"
globalThis.__lmActionWindowObserver = new IntersectionObserver(entries => {
  const entry = entries.find(candidate => candidate.target.id === "target");
  if (!entry) return;
  __lmActionWindowIoLog.push(entry.isIntersecting);
});
__lmActionWindowObserver.observe(document.getElementById("target"));
addEventListener("scroll", () => fetch("/action-window-scroll-applied"), { once: true });
"installed"
"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("IntersectionObserver should install");
    assert_eq!(
        renderer_json_value(observer_installed),
        Some(serde_json::json!("installed"))
    );
    let (initial_intersection, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify(__lmActionWindowIoLog)".to_owned(),
            await_promise: false,
        })
        .await
        .expect("initial intersection state should be observable");
    assert_eq!(
        renderer_json_value(initial_intersection),
        Some(serde_json::json!("[false]"))
    );

    let opened_at = std::time::Instant::now();
    for delta_y in [100.0, -100.0, 100.0] {
        let outcome = dispatch_wheel_for_action_window_test(&page, delta_y).await;
        assert!(outcome.handled, "wheel admission should be acknowledged");
    }

    tokio::time::timeout(Duration::from_secs(3), effect_request_seen)
        .await
        .expect("the owner scheduler should apply the wheel batch at its deadline")
        .expect("scroll effect signal should remain open");
    assert!(
        opened_at.elapsed() >= Duration::from_millis(900),
        "the fixed one-second action window must not apply immediately"
    );
    release_effect_response
        .send(())
        .expect("scroll effect response should release once");
    effect_server
        .await
        .expect("scroll effect server should finish");

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  scrollY,
  wheelLog: __lmActionWindowWheelLog,
  ioLog: __lmActionWindowIoLog
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("applied action-window state should remain observable");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(
            r#"{"scrollY":100,"wheelLog":["event:100","event:-100","event:100","microtask:100","microtask:-100","microtask:100"],"ioLog":[false]}"#
        ))
    );

    page.close_async()
        .await
        .expect("action-window deadline page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn wheel_batch_stops_on_document_replacement() {
    let runtime = initialize_layout_test_runtime();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
        .expect("default resource request client");
    let url = url::Url::parse("https://example.test/action-window-document-open").unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url,
        r#"<!doctype html>
<style>html, body { margin: 0; } body { height: 1200px; }</style>
<script>
globalThis.__lmRetiredDeltas = [];
globalThis.__lmReplacementDeltas = [];
document.addEventListener("wheel", event => {
  __lmRetiredDeltas.push(event.deltaY);
  document.open();
  document.write("<!doctype html><body style='height:1200px'>replacement</body>");
  document.close();
  document.addEventListener("wheel", replacementEvent => {
    __lmReplacementDeltas.push(replacementEvent.deltaY);
  }, { capture: true });
}, { capture: true, once: true });
</script>"#,
    )
    .await;

    capture_screenshot_for_renderer_page(&page).await;

    for delta_y in [10.0, 20.0, 30.0] {
        assert!(
            dispatch_wheel_for_action_window_test(&page, delta_y)
                .await
                .handled
        );
    }

    let (state, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"JSON.stringify({
  retired: __lmRetiredDeltas,
  replacement: __lmReplacementDeltas
})"#
            .to_owned(),
            await_promise: false,
        })
        .await
        .expect("the explicit read barrier should apply the pending wheel batch");
    assert_eq!(
        renderer_json_value(state),
        Some(serde_json::json!(r#"{"retired":[10],"replacement":[]}"#)),
        "actions admitted for the retired lifecycle must not continue in its document.open replacement"
    );

    page.close_async()
        .await
        .expect("document replacement action-window page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn page_producer_wake_is_admitted_during_sustained_command_input() {
    const COMMAND_COUNT: usize = 32;

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, delivery_request_seen, release_delivery_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-admission-command-fairness",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Page admission command fairness</body>",
    )
    .await;

    // Queue the producer setup first, then populate the command channel
    // without yielding. The BroadcastChannel wake must pass the owner
    // admission boundary before that already-ready command batch drains.
    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmPageAdmissionCommandCount = 0;
  globalThis.__lmPageAdmissionCountAtDelivery = null;
  const receiver = new BroadcastChannel('page-admission-command-fairness');
  const sender = new BroadcastChannel('page-admission-command-fairness');
  globalThis.__lmPageAdmissionChannels = { receiver, sender };
  receiver.onmessage = () => {
    globalThis.__lmPageAdmissionCountAtDelivery =
      globalThis.__lmPageAdmissionCommandCount;
    fetch('/page-admission-command-fairness');
  };
  sender.postMessage('go');
  return 'scheduled';
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("BroadcastChannel setup command should enqueue");
    let mut command_batch = Vec::with_capacity(COMMAND_COUNT);
    for _ in 0..COMMAND_COUNT {
        command_batch.push(
            page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
                expression: "++globalThis.__lmPageAdmissionCommandCount".to_owned(),
                await_promise: false,
            })
            .expect("command-fairness probe should enqueue"),
        );
    }
    let setup_completion = setup
        .wait()
        .await
        .expect("BroadcastChannel setup command should run");
    let (setup_completion, _renderer_output_predecessor) =
        setup_completion.into_completion_and_predecessor();
    let (setup_reply, _, _) = setup_completion.into_parts();
    assert_eq!(
        renderer_json_value(setup_reply),
        Some(serde_json::json!("scheduled"))
    );

    tokio::time::timeout(Duration::from_secs(2), delivery_request_seen)
        .await
        .expect("Page admissions must not remain hidden behind the command queue")
        .expect("delivery effect request signal should remain open");
    release_delivery_response
        .send(())
        .expect("delivery effect response should release once");
    server
        .await
        .expect("Page admission fairness server should finish");
    for command in command_batch {
        command
            .wait()
            .await
            .expect("queued command-fairness probe should complete");
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmPageAdmissionCountAtDelivery".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Page admission fairness result should remain observable");
    let count_at_delivery = renderer_json_value(observed)
        .and_then(|value| value.as_u64())
        .expect("delivery handler should capture the command count");
    assert!(
        count_at_delivery < COMMAND_COUNT as u64,
        "a ready Page producer wake must not wait for the entire command queue: {count_at_delivery}"
    );
    assert!(
        count_at_delivery <= 1,
        "one admitted producer Page turn may allow at most one ready command to overtake: {count_at_delivery}"
    );

    page.close_async()
        .await
        .expect("Page admission fairness page should close");
}
#[tokio::test(flavor = "multi_thread")]
async fn due_page_deadline_is_admitted_during_sustained_command_input() {
    const COMMAND_COUNT: usize = 1_024;

    let runtime = JsRuntime::initialize();
    let loader =
        ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("default loader");
    let (base_url, timer_request_seen, release_timer_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/page-deadline-command-fairness",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page URL");
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        page_url,
        "<!doctype html><body>Page deadline command fairness</body>",
    )
    .await;

    let setup = page
        .enqueue_async_command(RendererPageCommand::EvaluateExpression {
            expression: r#"(() => {
  globalThis.__lmPageDeadlineCommandCount = 0;
  globalThis.__lmPageDeadlineCountAtCallback = null;
  setTimeout(() => {
    globalThis.__lmPageDeadlineCountAtCallback =
      globalThis.__lmPageDeadlineCommandCount;
    fetch('/page-deadline-command-fairness');
  }, 0);
  return 'scheduled';
})()"#
                .to_owned(),
            await_promise: false,
        })
        .expect("zero-delay timer setup command should enqueue");
    let mut command_batch = Vec::with_capacity(COMMAND_COUNT);
    for _ in 0..COMMAND_COUNT {
        command_batch.push(
            page.enqueue_async_command(RendererPageCommand::EvaluateExpression {
                expression: "++globalThis.__lmPageDeadlineCommandCount".to_owned(),
                await_promise: false,
            })
            .expect("deadline command-fairness probe should enqueue"),
        );
    }
    setup
        .wait()
        .await
        .expect("zero-delay timer setup command should run");

    tokio::time::timeout(Duration::from_secs(2), timer_request_seen)
        .await
        .expect("a due Page deadline must not remain hidden behind the command queue")
        .expect("timer effect request signal should remain open");
    release_timer_response
        .send(())
        .expect("timer effect response should release once");
    server
        .await
        .expect("Page deadline fairness server should finish");
    for command in command_batch {
        command
            .wait()
            .await
            .expect("queued deadline command-fairness probe should complete");
    }

    let (observed, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "globalThis.__lmPageDeadlineCountAtCallback".to_owned(),
            await_promise: false,
        })
        .await
        .expect("Page deadline fairness result should remain observable");
    let count_at_callback = renderer_json_value(observed)
        .and_then(|value| value.as_u64())
        .expect("timer callback should capture the command count");
    assert!(
        count_at_callback < COMMAND_COUNT as u64,
        "a Page deadline that becomes due must interrupt sustained command admission: {count_at_callback}"
    );

    page.close_async()
        .await
        .expect("Page deadline fairness page should close");
}
