use super::*;

const HTML: &str = r#"<!doctype html><style>html,body{margin:0}
#viewport{width:100vw;height:2000px;background:lime}
.scroller{position:absolute;top:0;width:200px;height:100px;overflow:scroll}
.content{width:400px;height:300px}
#stable{left:220px;scrollbar-gutter:stable both-edges}</style>
<div id=viewport></div><div id=scroller class=scroller><div class=content></div></div>
<div id=stable class=scroller><div class=content></div></div>"#;

async fn set_hidden(ctx: &mut TestContext, session: &str, hidden: bool) {
    expect_session_command_result(
        ctx,
        88500,
        session,
        "Emulation.setScrollbarsHidden",
        json!({"hidden":hidden}),
    )
    .await;
}

async fn metrics(ctx: &mut TestContext) -> serde_json::Value {
    evaluate(
        ctx,
        "[innerWidth,document.documentElement.clientWidth,document.documentElement.scrollWidth,scroller.clientWidth,scroller.clientHeight,stable.clientWidth,stable.clientHeight]",
    )
    .await
}

async fn load_fixture(ctx: &mut TestContext) {
    load_session_page_for_pending_emulation_test(ctx).await;
    expect_session_command_result(
        ctx,
        88501,
        "SID-1",
        "Emulation.setDeviceMetricsOverride",
        json!({"width":1280,"height":720,"deviceScaleFactor":1,"mobile":false}),
    )
    .await;
    ctx.install_buffered_navigation_fixture_for_session_owner(
        url::Url::parse("https://scrollbars.example/page").unwrap(),
        HTML.to_owned(),
        Some("SID-1"),
    )
    .await;
    ctx.capture_fixture_layout(Some("SID-1")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scrollbars_hidden_updates_live_geometry_and_keeps_scrolling_and_author_styles() {
    let mut ctx = TestContext::new();
    load_fixture(&mut ctx).await;
    assert_eq!(
        metrics(&mut ctx).await,
        json!([1280, 1265, 1280, 185, 85, 170, 85])
    );
    evaluate(
        &mut ctx,
        "scroller.scrollLeft=20;scroller.scrollTop=40;scrollTo(0,100);undefined",
    )
    .await;
    for hidden in [true, false, true] {
        set_hidden(&mut ctx, "SID-1", hidden).await;
        // No screenshot or test-only refresh between the command and this
        // read: acknowledgement must publish the new live geometry.
        assert_eq!(
            metrics(&mut ctx).await,
            if hidden {
                json!([1280, 1280, 1280, 200, 100, 170, 100])
            } else {
                json!([1280, 1265, 1280, 185, 85, 170, 85])
            },
        );
        assert_eq!(
            evaluate(&mut ctx, "[scroller.scrollLeft,scroller.scrollTop,scrollY,getComputedStyle(scroller).overflow,getComputedStyle(scroller).scrollbarWidth,getComputedStyle(stable).scrollbarGutter]").await,
            json!([20,40,100,"scroll","auto","stable both-edges"]),
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scrollbars_hidden_validates_params_and_requires_layout_without_changing_policy() {
    let mut ctx = TestContext::new();
    load_fixture(&mut ctx).await;
    for params in [
        json!({}),
        json!({"hidden":null}),
        json!({"hidden":0}),
        json!({"hidden":"true"}),
    ] {
        ctx.process_async(json!({"id":88502,"sessionId":"SID-1","method":"Emulation.setScrollbarsHidden","params":params})).await;
        ctx.expect_error(88502, -32602, "InvalidParams");
    }
    assert_eq!(
        metrics(&mut ctx).await,
        json!([1280, 1265, 1280, 185, 85, 170, 85])
    );

    let mut mock = TestContext::new_with_layout_policy(moli_core::LayoutPolicy::Mock);
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    mock.conn.install_browser_context_fixture_for_test(bc);
    expect_session_command_error(
        &mut mock,
        88503,
        "SID-1",
        "Emulation.setScrollbarsHidden",
        json!({"hidden":true}),
        "Emulation.setScrollbarsHidden requires --layout",
    )
    .await;
    assert!(
        !mock
            .conn
            .emulation_session_state_for_session_owner(Some("SID-1"))
            .unwrap()
            .scrollbars_hidden
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scrollbars_hidden_is_replayed_before_navigation_scripts_and_initial_page_creation() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.set_target_url("about:blank".to_owned());
    bc.attach_active_session("SID-1");
    ctx.conn.install_browser_context_fixture_for_test(bc);
    expect_session_command_result(
        &mut ctx,
        88502,
        "SID-1",
        "Emulation.setEmulatedMedia",
        json!({"features":[{"name":"prefers-color-scheme","value":"dark"}]}),
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        88503,
        "SID-1",
        "Emulation.setDeviceMetricsOverride",
        json!({"width":900,"height":500,"deviceScaleFactor":1,"mobile":false}),
    )
    .await;
    set_hidden(&mut ctx, "SID-1", true).await;
    // Materialize the staged about:blank Page first, then navigate to a new
    // Document. Both direct HTML creation and prepared commit must inherit.
    let owner = crate::conn::CommandOwnerScope::for_session("SID-1");
    let pending = ctx
        .conn
        .start_initial_document_page_ensure_for_owner(&owner)
        .expect("initial Page ensure")
        .expect("staged initial Page");
    let completed = pending.wait().await.expect("initial Page creation");
    ctx.conn
        .complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("initial Page installation");
    assert_eq!(
        evaluate(
            &mut ctx,
            "document.documentElement.clientWidth === innerWidth"
        )
        .await,
        json!(true)
    );
    for hidden in [true, false, true] {
        set_hidden(&mut ctx, "SID-1", hidden).await;
        let html = format!(
            "{HTML}<script>globalThis.__initial = [innerWidth,document.documentElement.clientWidth,scroller.clientWidth,matchMedia('(prefers-color-scheme: dark)').matches];</script>"
        );
        ctx.install_buffered_navigation_fixture_for_session_owner(
            url::Url::parse("https://scrollbars.example/next").unwrap(),
            html,
            Some("SID-1"),
        )
        .await;
        assert_eq!(
            evaluate(&mut ctx, "__initial[1] === __initial[0]").await,
            json!(hidden)
        );
        assert_eq!(
            evaluate(&mut ctx, "__initial[2]").await,
            json!(if hidden { 200 } else { 185 })
        );
        assert_eq!(evaluate(&mut ctx, "__initial[0]").await, json!(900));
        assert_eq!(evaluate(&mut ctx, "__initial[3]").await, json!(true));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scrollbars_hidden_respects_session_noops_and_detach_cleanup() {
    let mut ctx = TestContext::new();
    load_fixture(&mut ctx).await;
    ctx.process_async(json!({"id":88504,"method":"Target.attachToTarget","params":{"targetId":"TID-1","flatten":true}})).await;
    let b = ctx.take_response_by_id(88504)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    for (session, hidden, expected) in [
        ("SID-1", true, 200),
        (b.as_str(), false, 200),
        (b.as_str(), true, 200),
        ("SID-1", false, 185),
        (b.as_str(), true, 185),
        (b.as_str(), false, 185),
        ("SID-1", true, 200),
    ] {
        set_hidden(&mut ctx, session, hidden).await;
        assert_eq!(
            evaluate(&mut ctx, "scroller.clientWidth").await,
            json!(expected)
        );
    }
    ctx.process_async(
        json!({"id":88505,"method":"Target.detachFromTarget","params":{"sessionId":b}}),
    )
    .await;
    ctx.expect_result(88505, json!({}), None);
    assert_eq!(evaluate(&mut ctx, "scroller.clientWidth").await, json!(200));
    ctx.process_async(json!({"id":88506,"method":"Target.attachToTarget","params":{"targetId":"TID-1","flatten":true}})).await;
    let c = ctx.take_response_by_id(88506)["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    set_hidden(&mut ctx, &c, true).await;
    ctx.process_async(
        json!({"id":88507,"method":"Target.detachFromTarget","params":{"sessionId":c}}),
    )
    .await;
    ctx.expect_result(88507, json!({}), None);
    assert_eq!(evaluate(&mut ctx, "scroller.clientWidth").await, json!(185));
    set_hidden(&mut ctx, "SID-1", true).await;
    assert_eq!(
        evaluate(&mut ctx, "scroller.clientWidth").await,
        json!(185),
        "repeating the remaining handler's raw true is a no-op"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scrollbars_hidden_false_and_detach_preserve_the_browser_startup_flag() {
    let conn = crate::CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        moli_core::runtime::NavigationRuntimeConfig::new(
            moli_fetch::FetchConfig::default(),
            moli_core::OptionalResourceFetchMask::NONE,
            true,
            moli_core::LayoutPolicy::OnDemand,
        )
        .with_scrollbars_hidden(true),
    );
    let mut ctx = TestContext::from_conn(conn);
    load_fixture(&mut ctx).await;
    for hidden in [false, true, false, true] {
        set_hidden(&mut ctx, "SID-1", hidden).await;
        assert_eq!(
            metrics(&mut ctx).await,
            json!([1280, 1280, 1280, 200, 100, 170, 100])
        );
    }
    super::super::dispose_page_session_async(&mut ctx.conn, "SID-1")
        .await
        .unwrap();
    // Disposing this handler also clears its device metrics override. Capture
    // the resulting default viewport before checking scrollbar geometry.
    ctx.capture_fixture_layout(Some("SID-1")).await;
    assert_eq!(
        evaluate(&mut ctx,"[innerWidth===document.documentElement.clientWidth,scroller.clientWidth,scroller.clientHeight,stable.clientWidth,stable.clientHeight]").await,
        json!([true, 200, 100, 170, 100])
    );
}
