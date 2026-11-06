use super::*;

async fn set_media_viewport(
    page: &mut TestCdpSocket,
    id: u64,
    width: u32,
) -> Vec<serde_json::Value> {
    let messages = send_cdp_command(
        page,
        id,
        "Emulation.setDeviceMetricsOverride",
        None,
        json!({"width":width,"height":500,"deviceScaleFactor":1,"mobile":false}),
    )
    .await;
    assert!(response_by_id(&messages, id)["error"].is_null());
    messages
}

async fn resize_and_wait_for_media_control(page: &mut TestCdpSocket, id: u64) {
    set_media_viewport(page, id, 800).await;
    let messages = send_cdp_command(
        page,
        id + 1000,
        "Runtime.evaluate",
        None,
        json!({
            "expression":"controlDone","awaitPromise":true,"returnByValue":true
        }),
    )
    .await;
    let response = response_by_id(&messages, id + 1000);
    assert!(response["error"].is_null(), "{response}");
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "{response}"
    );
    assert_eq!(response["result"]["result"]["value"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn borrowed_media_query_listener_methods_update_the_popup_host() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async {
            axum::response::Html("<!doctype html><body>MQL popup")
        })),
        "popup-media-queries",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server_with_runtime_config(
        NavigationRuntimeConfig::new(
            protocol_server_test_fetch_config(FetchConfig::default()),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::Realtime,
        )
        .with_debugger_enabled(true),
    )
    .await;
    let (mut browser, _) = connect_async(discover_browser_websocket_url(addr).await.unwrap())
        .await
        .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 3, &format!("{base}/opener")).await;
    send_cdp_command(&mut opener, 2, "Runtime.enable", None, json!({})).await;
    set_media_viewport(&mut opener, 4, 400).await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            5,
            r#"(() => {
        window.calls=[];window.controlCalls=[];
        window.q=matchMedia('(min-width:600px)');
        q.addEventListener('change',event=>calls.push(event.matches));
        window.control=matchMedia('(min-width:600px)');
        control.addEventListener('change',event=>controlCalls.push(event.matches));
        return [q.matches,control.matches];
    })()"#
        )
        .await,
        json!([false, false])
    );

    let (_, mut popup) = popup_navigation_referrer::open_http_popup(
        addr,
        &mut opener,
        "media-query-popup",
        &format!("{base}/popup"),
        "",
    )
    .await;
    send_cdp_command(&mut popup, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut popup, 3, &format!("{base}/popup-ready")).await;
    send_cdp_command(&mut popup, 2, "Runtime.enable", None, json!({})).await;
    set_media_viewport(&mut popup, 4, 400).await;
    assert_eq!(evaluate_window_name_probe(&mut popup,5,r#"(() => {
        window.calls=[];
        window.q=matchMedia('(min-width:600px)');
        window.callback=function(event){
            calls.push([event.matches,this===q,event.target===q,event instanceof MediaQueryListEvent]);
        };
        window.control=matchMedia('(min-width:600px)');
        window.controlDone=new Promise(resolve=>control.addEventListener('change',()=>resolve(true)));
        return [q.matches,control.matches];
    })()"#).await,json!([false,false]));
    assert_eq!(evaluate_window_name_probe(&mut opener,6,
        "EventTarget.prototype.addEventListener.call(lastPopup.q,'change',lastPopup.callback);true"
    ).await,true);
    // The popup control is a positive same-phase barrier for the preceding q event.
    resize_and_wait_for_media_control(&mut popup, 6).await;
    let popup_calls = evaluate_window_name_probe(&mut popup, 7, "[q.matches,calls]").await;

    assert_eq!(evaluate_window_name_probe(&mut opener,7,
        "EventTarget.prototype.removeEventListener.call(lastPopup.q,'change',lastPopup.callback);true"
    ).await,true);
    // Register this completion signal after the borrowed removal, so even a
    // corrupt caller-side listening flag cannot disable the test's barrier.
    assert_eq!(evaluate_window_name_probe(&mut opener,10,r#"(() => {
        window.completion=matchMedia('(min-width:600px)');
        window.controlDone=new Promise(resolve=>completion.addEventListener('change',()=>resolve(true)));
        return completion.matches;
    })()"#).await,false);
    // Opening a popup backgrounds its opener. Resume the opener's rendering
    // opportunities before asserting its media-query notifications.
    send_cdp_command(
        &mut browser,
        20,
        "Target.activateTarget",
        None,
        json!({"targetId":opener_id}),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 21, "document.visibilityState").await,
        "visible"
    );
    resize_and_wait_for_media_control(&mut opener, 8).await;
    let opener_calls = evaluate_window_name_probe(
        &mut opener,
        9,
        "[q.matches,control.matches,calls,controlCalls]",
    )
    .await;
    abort_test_cdp_server(server).await;
    assert_eq!(
        json!([popup_calls, opener_calls]),
        json!([
            [true, [[true, true, true, true]]],
            [true, true, [true], [true]]
        ]),
        "borrowed listener methods must update the popup without disabling caller queries"
    );
}
