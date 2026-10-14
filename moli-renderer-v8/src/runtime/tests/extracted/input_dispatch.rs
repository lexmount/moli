use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn realtime_popup_show_measure_position_and_input_complete_without_capture() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (base_url, ready, release_ready, ready_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/popup-position-ready",
            "ready",
            "text/plain; charset=utf-8",
        )
        .await;
    let (completion_base_url, completed, release_completed, completion_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/popup-position-completed",
            "completed",
            "text/plain; charset=utf-8",
        )
        .await;
    let html = include_str!("../../../../tests/fixtures/realtime-popup-position-input.html")
        .replace(
            "__READY_URL__",
            &serde_json::to_string(&format!("{base_url}/popup-position-ready")).unwrap(),
        )
        .replace(
            "__COMPLETED_URL__",
            &serde_json::to_string(&format!("{completion_base_url}/popup-position-completed"))
                .unwrap(),
        );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page")).unwrap(),
        &html,
    )
    .await;

    // No evaluate, capture, layout-publication helper or manual frame pump is
    // sent before ready. The page's two rAF callbacks reveal and place the
    // previously hidden panel using its own synchronous geometry reads.
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("the page must show, measure and position its panel without a follow-up command")
        .unwrap();
    release_ready.send(()).unwrap();
    ready_server.await.unwrap();

    // The fixture places a 160x48 panel at (100, 4). Pick a point inside
    // that result without asking a selector or geometry getter for a point.
    // An old hidden-panel width of zero would place it at x=260, so a later
    // frame could not rescue the wrong computed position before this click.
    for (event_name, buttons) in [("mousedown", 1), ("mouseup", 0)] {
        let pending = page
            .enqueue_protocol_command(RendererPageCommand::DispatchMouseEventAtPoint {
                x: 120.0,
                y: 16.0,
                event_name: event_name.to_owned(),
                button: 0,
                buttons: Some(buttons),
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
                pointer: RendererPointerEventProperties::default(),
                modifiers: 0,
            })
            .expect("popup pointer command must enqueue");
        let outcome = tokio::time::timeout(Duration::from_secs(2), pending.wait())
            .await
            .expect("popup pointer command must reach its completion ACK")
            .expect("popup pointer command must complete");
        let (completion, _) = outcome.into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        assert!(
            matches!(reply, RendererPageReply::InputDispatchOutcome(outcome) if outcome.handled),
            "{event_name} must dispatch through the real Page input route"
        );
    }
    tokio::time::timeout(Duration::from_secs(2), completed)
        .await
        .expect("pointer click and its microtask must finish the business without a repair command")
        .unwrap();
    release_completed.send(()).unwrap();
    completion_server.await.unwrap();

    // This is a stored string created before the completion fetch, not a DOM
    // or computed-style/geometry read that could make the business succeed.
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "window.popupResult".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    let result = mouse_input_json_witness(&reply);
    page.close_async().await.unwrap();
    assert_eq!(
        result,
        serde_json::json!({
            "phase": "closed",
            "frames": 2,
            "events": [
                "hidden-frame",
                "shown-measured-positioned",
                ["mousedown", "choice", 120, 16],
                ["mouseup", "choice", 120, 16],
                ["click", "choice", 120, 16],
                "click-microtask"
            ],
            "anchor": [180, 60, 80, 24],
            "panelSize": [160, 48],
            "placement": [100, 4],
            "selected": "alpha",
            "closed": true
        }),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn realtime_timer_network_and_frame_advance_during_sustained_wheel_input() {
    use futures_util::{StreamExt, stream::FuturesUnordered};

    const IN_FLIGHT: usize = 128;
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (base_url, mut payload_requested, release_payload, payload_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/input-fairness-payload",
            "network-ready",
            "text/plain; charset=utf-8",
        )
        .await;
    let (completion_url, mut completed, release_completion, completion_server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/input-fairness-complete",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let html = include_str!("../../../../tests/fixtures/realtime-input-fairness.html").replace(
        "__COMPLETION_URL__",
        &serde_json::to_string(&format!("{completion_url}/input-fairness-complete")).unwrap(),
    );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page")).unwrap(),
        &html,
    )
    .await;
    let enqueue_wheel = || {
        page.enqueue_protocol_command(RendererPageCommand::DispatchMouseEventAtPoint {
            x: 10.0,
            y: 10.0,
            event_name: "wheel".to_owned(),
            button: -1,
            buttons: Some(0),
            click_count: 0,
            delta_x: 0.0,
            delta_y: 1.0,
            pointer: RendererPointerEventProperties::default(),
            modifiers: 0,
        })
        .expect("wheel input should enqueue")
        .wait()
    };
    // Refill a bounded set of input commands until the page itself signals
    // all three callbacks. They are registered by the first wheel handler,
    // so setup cannot satisfy this test before input starts. No evaluate or
    // capture barrier helps them run, and no fixed input count assumes how
    // many commands a machine needs to reach the first frame deadline.
    let mut pending = FuturesUnordered::new();
    for _ in 0..IN_FLIGHT {
        pending.push(enqueue_wheel());
    }
    let mut sent = IN_FLIGHT;
    let mut release_payload = Some(release_payload);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::select! {
                signal = &mut completed => {
                    signal.expect("page completion signal must remain open");
                    break;
                }
                request = &mut payload_requested, if release_payload.is_some() => {
                    request.expect("network request signal must remain open");
                    release_payload.take().unwrap().send(()).unwrap();
                }
                Some(result) = pending.next() => {
                    let (completion, _) = result.unwrap().into_completion_and_predecessor();
                    let (reply, _, _) = completion.into_parts();
                    assert!(matches!(reply, RendererPageReply::InputDispatchOutcome(outcome) if outcome.handled));
                    pending.push(enqueue_wheel());
                    sent += 1;
                }
            }
        }
    })
    .await
    .expect("timer, network and frame callbacks must run during sustained input");
    assert!(
        release_payload.is_none(),
        "network response must be consumed"
    );
    release_completion.send(()).unwrap();
    payload_server.await.unwrap();
    completion_server.await.unwrap();
    while let Some(result) = pending.next().await {
        let (completion, _) = result.unwrap().into_completion_and_predecessor();
        let (reply, _, _) = completion.into_parts();
        assert!(
            matches!(reply, RendererPageReply::InputDispatchOutcome(outcome) if outcome.handled)
        );
    }
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify({...fairness,scrollTop:scroller.scrollTop})".to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    let state = mouse_input_json_witness(&reply);
    page.close_async().await.unwrap();
    assert_eq!(state["inputs"], serde_json::json!(sent), "{state}");
    assert_eq!(state["microtasks"], serde_json::json!(sent), "{state}");
    assert_eq!(state["deltaSum"], serde_json::json!(sent), "{state}");
    assert_eq!(state["scrollTop"], serde_json::json!(sent), "{state}");
    assert_eq!(state["errors"], serde_json::json!([]), "{state}");
    assert_eq!(state["networkBody"], "network-ready", "{state}");
    assert_eq!(state["samples"].as_object().unwrap().len(), 3, "{state}");
    for name in ["timer", "network", "frame"] {
        let sample = &state["samples"][name];
        let count = sample["inputs"].as_u64().unwrap();
        assert!(
            count > 0 && count < sent as u64,
            "{name} must execute before input drains: {state}"
        );
        assert_eq!(sample["microtasks"], sample["inputs"], "{name}: {state}");
        assert_eq!(sample["width"], sample["expectedWidth"], "{name}: {state}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn realtime_input_scroll_io_and_dynamic_checkbox_complete_without_capture() {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let (base_url, ready, release_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            "/realtime-input-ready",
            "ok",
            "text/plain; charset=utf-8",
        )
        .await;
    let mut page = create_test_html_page(&runtime, &loader,
        url::Url::parse(&format!("{base_url}/page")).unwrap(), r#"
        <!doctype html><div id=scroller style="position:absolute;left:0;top:0;width:200px;height:100px;overflow:auto">
          <div style="height:300px"></div><div id=sentinel style="height:20px"></div>
        </div><script>
        window.events=[];window.clicks=0;
        scroller.addEventListener('wheel',event=>{
          events.push('wheel:'+event.deltaY);Promise.resolve().then(()=>events.push('wheel-microtask'));
        });
        window.io=new IntersectionObserver(entries=>{
          if(!entries.some(entry=>entry.isIntersecting))return;
          io.disconnect();events.push('io');
          const input=document.createElement('input');input.id='toggle';input.type='checkbox';
          input.style.cssText='position:fixed;left:230px;top:20px;width:24px;height:24px;margin:0;opacity:0';
          input.addEventListener('click',()=>{
            clicks++;events.push('click');Promise.resolve().then(()=>events.push('click-microtask'));
          });
          document.body.append(input);fetch('/realtime-input-ready');
        },{root:scroller});io.observe(sentinel);
        </script>
    "#).await;
    let outcome = dispatch_wheel_for_action_window_test(&page, 300.0).await;
    assert!(outcome.handled);
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("wheel and IO must advance without a screenshot or barrier command")
        .unwrap();
    release_response.send(()).unwrap();
    server.await.unwrap();
    for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
        page.run_async_command(RendererPageCommand::DispatchMouseEventAtPoint {
            x: 240.0,
            y: 30.0,
            event_name: event.to_owned(),
            button: 0,
            buttons: Some(buttons),
            click_count: 1,
            delta_x: 0.0,
            delta_y: 0.0,
            pointer: RendererPointerEventProperties::default(),
            modifiers: 0,
        })
        .await
        .unwrap();
    }
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify([scroller.scrollTop,toggle.checked,clicks,events])"
                .to_owned(),
            await_promise: false,
        })
        .await
        .unwrap();
    let value = renderer_json_value(reply);
    page.close_async().await.unwrap();
    assert_eq!(
        value,
        Some(serde_json::json!(
            "[220,true,1,[\"wheel:300\",\"wheel-microtask\",\"io\",\"click\",\"click-microtask\"]]"
        ))
    );
}

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

#[tokio::test(flavor = "current_thread")]
async fn realtime_captured_child_pointer_keeps_its_current_frame_coordinates_until_release() {
    assert_realtime_captured_child_pointer_coordinates(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_captured_child_pointer_keeps_current_packet_coordinates_across_got_publication() {
    assert_realtime_captured_child_pointer_coordinates(true).await;
}

async fn assert_realtime_captured_child_pointer_coordinates(publish_in_got: bool) {
    async fn command(
        page: &RendererPageHandle,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageReply> {
        let (reply, _) =
            tokio::time::timeout(Duration::from_secs(5), page.run_async_command(command)).await??;
        Ok(reply)
    }

    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/realtime-child-pointer-capture").unwrap(),
        "<!doctype html><style>html,body{margin:0;padding:0}</style><body></body>",
    )
    .await;

    // Keep close outside the fallible case so an input/setup error is reported
    // together with the result of the normal owner teardown.
    let result = async {
        command(
            &page,
            RendererPageCommand::SetViewportSurface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 640,
                inner_height: 480,
                outer_width: 640,
                outer_height: 480,
                device_pixel_ratio: 1.0,
                ..Default::default()
            })),
        )
        .await?;
        let setup = command(
            &page,
            RendererPageCommand::EvaluateExpression {
                expression: r#"
(() => {
  window.captureWitness = {
    events: [], transitions: [], errors: [], childMoves: 0, frameMoved: false
  };
  const witness = window.captureWitness;
  const publishInGot = __PUBLISH_IN_GOT__;
  const recordError = event => {
    witness.errors.push(String(event.message || event.reason || 'unknown error'));
  };
  window.addEventListener('error', recordError);
  window.addEventListener('unhandledrejection', recordError);
  const outside = document.createElement('div');
  outside.id = 'outside';
  outside.style.cssText = 'position:fixed;left:0;top:0;width:50px;height:50px';
  document.body.appendChild(outside);
  for (const type of ['pointerdown', 'pointermove', 'pointerup']) {
    outside.addEventListener(type, event => {
      witness.events.push([type, 'root', event.target.id,
        event.clientX, event.clientY, event.buttons, false]);
    });
  }
  const frame = document.createElement('iframe');
  frame.style.cssText =
    'position:absolute;left:100px;top:60px;width:200px;height:120px;border:0;padding:0;margin:0';
  const installed = new Promise((resolve, reject) => {
    frame.addEventListener('load', () => {
      try {
        const child = frame.contentDocument;
        const captured = child.getElementById('captured');
        if (!captured) throw new Error('loaded child is missing the capture target');
        frame.contentWindow.addEventListener('error', recordError);
        frame.contentWindow.addEventListener('unhandledrejection', recordError);
        let pointerId = null;
        captured.addEventListener('pointerdown', event => {
          pointerId = event.pointerId;
          captured.setPointerCapture(pointerId);
        });
        for (const type of ['gotpointercapture', 'lostpointercapture']) {
          captured.addEventListener(type, event => {
            witness.transitions.push([type, event.target.id,
              event.target.ownerDocument === child,
              captured.hasPointerCapture(pointerId), event.buttons]);
            if (publishInGot && type === 'gotpointercapture') {
              // This separate control deliberately replaces the screen sample
              // during pending-capture delivery. The current input's point was
              // already projected; only later native input uses the new sample.
              frame.style.left = '200px';
              frame.style.top = '100px';
              const rect = frame.getBoundingClientRect();
              witness.publishedFrame = {
                rect: [rect.x, rect.y, rect.width, rect.height],
                childMovesBeforePublication: witness.childMoves
              };
              witness.frameMoved = true;
            }
          });
        }
        for (const type of ['pointerdown', 'pointermove', 'pointerup']) {
          child.addEventListener(type, event => {
            witness.events.push([type, 'child', event.target.id,
              event.clientX, event.clientY, event.buttons,
              captured.hasPointerCapture(pointerId)]);
            if (type === 'pointermove') {
              witness.childMoves++;
              if (!publishInGot && witness.childMoves === 1) {
                // The original business case still performs only this write.
                // Its branch never reads geometry or explicitly publishes it.
                frame.style.left = '200px';
                frame.style.top = '100px';
                witness.frameMoved = true;
              }
            }
          });
        }
        resolve('installed');
      } catch (error) {
        reject(error);
      }
    }, {once: true});
  });
  frame.srcdoc = `<!doctype html>
    <style>html,body{margin:0;padding:0}</style>
    <div id="captured" style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>`;
  document.body.appendChild(frame);
  return installed;
})()
"#
                .replace(
                    "__PUBLISH_IN_GOT__",
                    if publish_in_got { "true" } else { "false" },
                ),
                await_promise: true,
            },
        )
        .await?;
        anyhow::ensure!(
            renderer_json_value(setup) == Some(serde_json::json!("installed")),
            "the ordinary child load must install its listeners before input"
        );

        // Root (130,90) initially gives child (30,30). The second input begins
        // at child (31,31), then delivers pending gotpointercapture. The control
        // publishes the frame's new location inside that callback; the original
        // case only writes it in the later pointermove listener. In both cases
        // the next root (20,20) maps to captured child (-180,-80).
        for (event_name, x, y, button, buttons) in [
            ("mousedown", 130.0, 90.0, 0, 1),
            ("mousemove", 131.0, 91.0, -1, 1),
            ("mousemove", 20.0, 20.0, -1, 1),
            ("mouseup", 20.0, 20.0, 0, 0),
            ("mousemove", 20.0, 20.0, -1, 0),
        ] {
            let reply = command(
                &page,
                RendererPageCommand::DispatchMouseEventAtPoint {
                    x,
                    y,
                    event_name: event_name.to_owned(),
                    button,
                    buttons: Some(buttons),
                    click_count: i32::from(event_name != "mousemove"),
                    delta_x: 0.0,
                    delta_y: 0.0,
                    pointer: RendererPointerEventProperties::default(),
                    modifiers: 0,
                },
            )
            .await?;
            anyhow::ensure!(
                matches!(reply, RendererPageReply::InputDispatchOutcome(outcome)
                    if outcome.handled && !outcome.triggered_top_level_navigation),
                "{event_name} must finish through the real owner input route"
            );
        }
        // The business and coordinates are already recorded by input handlers.
        // This final read neither selects a target nor computes any geometry.
        let witness = command(
            &page,
            RendererPageCommand::EvaluateExpression {
                expression: "JSON.stringify(window.captureWitness)".to_owned(),
                await_promise: false,
            },
        )
        .await?;
        let observation = tokio::time::timeout(
            Duration::from_secs(5),
            RendererPageTestingHandle::new_for_testing(&page)
                .layout_working_set_observation_async(),
        )
        .await??;
        Ok::<_, anyhow::Error>((witness, observation))
    }
    .await;
    let close = tokio::time::timeout(Duration::from_secs(5), page.close_async()).await;
    let (reply, observation) = match result {
        Ok(value) => value,
        Err(error) => panic!("child capture case failed: {error:#}; close result: {close:?}"),
    };
    close
        .expect("child capture close deadline")
        .expect("child capture page must close");
    let witness = mouse_input_json_witness(&reply);
    assert_eq!(
        witness["events"][0],
        serde_json::json!(["pointerdown", "child", "captured", 30, 30, 1, true]),
        "the first native down establishes the initial child coordinate control"
    );
    assert_eq!(
        witness["events"][1],
        serde_json::json!(["pointermove", "child", "captured", 31, 31, 1, true]),
        "the current move keeps its pre-callback point even when gotpointercapture publishes a new screen sample"
    );
    let mut expected = serde_json::json!({
        "events": [
            ["pointerdown", "child", "captured", 30, 30, 1, true],
            ["pointermove", "child", "captured", 31, 31, 1, true],
            ["pointermove", "child", "captured", -180, -80, 1, true],
            ["pointerup", "child", "captured", -180, -80, 0, true],
            ["pointermove", "root", "outside", 20, 20, 0, false]
        ],
        "transitions": [
            ["gotpointercapture", "captured", true, true, 1],
            ["lostpointercapture", "captured", true, false, 0]
        ],
        "errors": [],
        "childMoves": 2,
        "frameMoved": true
    });
    if publish_in_got {
        let published_frame = serde_json::json!({
            "rect": [200, 100, 200, 120],
            "childMovesBeforePublication": 0
        });
        assert_eq!(
            witness["publishedFrame"], published_frame,
            "the control must observe the new frame geometry during gotpointercapture before the first child move"
        );
        expected["publishedFrame"] = published_frame;
    }
    assert_eq!(
        witness, expected,
        "capture target identity and its current Document coordinate system are separate from the raw hit"
    );
    assert_eq!(
        observation.last_pass_paint_operations,
        Some(0),
        "the last observed layout pass did not paint; this is not a cumulative paint counter"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn realtime_hidden_iframe_capture_keeps_input_commands_deliverable_without_layout_box() {
    async fn command(
        page: &RendererPageHandle,
        command: RendererPageCommand,
    ) -> anyhow::Result<RendererPageReply> {
        let (reply, _) =
            tokio::time::timeout(Duration::from_secs(5), page.run_async_command(command)).await??;
        Ok(reply)
    }

    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(moli_page_types::LayoutPolicy::Realtime)
        .unwrap();
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse("https://example.test/realtime-hidden-frame-capture").unwrap(),
        "<!doctype html><style>html,body{margin:0;padding:0}</style><body></body>",
    )
    .await;

    let result = async {
        command(
            &page,
            RendererPageCommand::SetViewportSurface(Some(crate::protocol_types::ViewportSurface {
                inner_width: 640,
                inner_height: 480,
                outer_width: 640,
                outer_height: 480,
                device_pixel_ratio: 1.0,
                ..Default::default()
            })),
        )
        .await?;
        let setup = command(
            &page,
            RendererPageCommand::EvaluateExpression {
                expression: r#"
(() => {
  const witness = window.hiddenCaptureWitness = {
    events: [], transitions: [], errors: [], childMoves: 0
  };
  const recordError = event => witness.errors.push(
    String(event.message || event.reason || 'unknown error'));
  window.addEventListener('error', recordError);
  window.addEventListener('unhandledrejection', recordError);
  const outside = document.createElement('div');
  outside.id = 'outside';
  outside.style.cssText = 'position:fixed;left:0;top:0;width:50px;height:50px';
  document.body.appendChild(outside);
  outside.addEventListener('pointermove', event => {
    witness.events.push(['pointermove', 'root', event.target.id,
      event.clientX, event.clientY, event.buttons, false]);
  });
  const frame = document.createElement('iframe');
  frame.style.cssText =
    'position:absolute;left:100px;top:60px;width:200px;height:120px;border:0;padding:0;margin:0';
  const installed = new Promise((resolve, reject) => {
    frame.addEventListener('load', () => {
      try {
        const child = frame.contentDocument;
        const captured = child.getElementById('captured');
        if (!captured) throw new Error('loaded child is missing its target');
        frame.contentWindow.addEventListener('error', recordError);
        frame.contentWindow.addEventListener('unhandledrejection', recordError);
        let pointerId = null;
        captured.addEventListener('pointerdown', event => {
          pointerId = event.pointerId;
          captured.setPointerCapture(pointerId);
        });
        for (const type of ['gotpointercapture', 'lostpointercapture']) {
          captured.addEventListener(type, event => {
            witness.transitions.push([type, event.target.id,
              captured.hasPointerCapture(pointerId), event.buttons]);
          });
        }
        for (const type of ['pointerdown', 'pointermove', 'pointerup']) {
          child.addEventListener(type, event => {
            witness.events.push([type, 'child', event.target.id,
              event.clientX, event.clientY, event.buttons,
              captured.hasPointerCapture(pointerId)]);
            if (type === 'pointermove' && ++witness.childMoves === 1) {
              // Capture is already active. This source write removes its
              // containing frame's layout box, not its Document or DOM nodes.
              // No getter, capture, rAF or manual frame pump follows it.
              frame.style.display = 'none';
              witness.hiddenState = [frame.style.display, frame.isConnected,
                captured.isConnected, captured.ownerDocument === child,
                captured.hasPointerCapture(pointerId)];
            }
          });
        }
        resolve('installed');
      } catch (error) {
        reject(error);
      }
    }, {once: true});
  });
  frame.srcdoc = `<!doctype html>
    <style>html,body{margin:0;padding:0}</style>
    <div id="captured" style="position:absolute;left:20px;top:20px;width:40px;height:40px"></div>`;
  document.body.appendChild(frame);
  return installed;
})()
"#
                .to_owned(),
                await_promise: true,
            },
        )
        .await?;
        anyhow::ensure!(
            renderer_json_value(setup) == Some(serde_json::json!("installed")),
            "normal child load must install listeners before native input"
        );

        for (event_name, x, y, button, buttons) in [
            ("mousedown", 130.0, 90.0, 0, 1),
            ("mousemove", 131.0, 91.0, -1, 1),
            ("mousemove", 20.0, 20.0, -1, 1),
            ("mouseup", 20.0, 20.0, 0, 0),
            ("mousemove", 20.0, 20.0, -1, 0),
        ] {
            let reply = command(
                &page,
                RendererPageCommand::DispatchMouseEventAtPoint {
                    x,
                    y,
                    event_name: event_name.to_owned(),
                    button,
                    buttons: Some(buttons),
                    click_count: i32::from(event_name != "mousemove"),
                    delta_x: 0.0,
                    delta_y: 0.0,
                    pointer: RendererPointerEventProperties::default(),
                    modifiers: 0,
                },
            )
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "{event_name} at ({x},{y}) must remain deliverable after hiding: {error:#}"
                )
            })?;
            anyhow::ensure!(
                matches!(reply, RendererPageReply::InputDispatchOutcome(outcome)
                    if outcome.handled && !outcome.triggered_top_level_navigation),
                "{event_name} must complete through the normal owner input route"
            );
        }
        command(
            &page,
            RendererPageCommand::EvaluateExpression {
                expression: "JSON.stringify(window.hiddenCaptureWitness)".to_owned(),
                await_promise: false,
            },
        )
        .await
    }
    .await;
    let close = tokio::time::timeout(Duration::from_secs(5), page.close_async()).await;
    let reply = match result {
        Ok(value) => value,
        Err(error) => {
            panic!("hidden frame capture case failed: {error:#}; close result: {close:?}")
        }
    };
    close
        .expect("hidden frame capture close deadline")
        .expect("hidden frame capture page must close");
    let witness = mouse_input_json_witness(&reply);
    // Preserve the raw hidden-frame coordinates in the witness, but do not
    // turn the former identity fallback into a new coordinate contract.
    // The visible controls below remain exact; hidden input must be finite
    // and keep its existing capture target, delivery, and release behavior.
    for event_index in [2, 3] {
        for coordinate_index in [3, 4] {
            assert!(
                witness["events"][event_index][coordinate_index]
                    .as_f64()
                    .is_some_and(f64::is_finite),
                "hidden captured input must carry finite coordinates: {witness}"
            );
        }
    }
    assert_eq!(
        witness,
        serde_json::json!({
            "events": [
                ["pointerdown", "child", "captured", 30, 30, 1, true],
                ["pointermove", "child", "captured", 31, 31, 1, true],
                ["pointermove", "child", "captured",
                    witness["events"][2][3].clone(), witness["events"][2][4].clone(), 1, true],
                ["pointerup", "child", "captured",
                    witness["events"][3][3].clone(), witness["events"][3][4].clone(), 0, true],
                ["pointermove", "root", "outside", 20, 20, 0, false]
            ],
            "transitions": [
                ["gotpointercapture", "captured", true, 1],
                ["lostpointercapture", "captured", false, 0]
            ],
            "errors": [],
            "childMoves": 2,
            "hiddenState": ["none", true, true, true, true]
        }),
        "a legal missing iframe box must not turn existing capture delivery into a native geometry error"
    );
}
