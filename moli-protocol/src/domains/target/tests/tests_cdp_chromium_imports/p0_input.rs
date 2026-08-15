use super::super::tests_cdp_smoke_fixture::SmokeFixtureServer;
use super::super::*;
use super::support::CdpPageHarness;
use crate::conn::CdpCommandTaskStep;
use serde_json::json;
use std::time::Duration;

async fn expect_page_replacement_cleans_up_pending_input_ack(
    ctx: &mut TestContext,
    page: &CdpPageHarness,
    command_id: u64,
    method: &str,
    params: Value,
    replacement_marker: &str,
) {
    let original_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some(&page.session_id))
        .expect("the input command should have a Page owner");
    let raw = json!({
        "id": command_id,
        "method": method,
        "sessionId": page.session_id,
        "params": params
    })
    .to_string();
    let mut pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("{method} should wait for a renderer ACK")
        }
    };
    assert_eq!(pending.kind_name(), "Input");
    assert!(
        pending.hold_input_renderer_ack_for_test(),
        "{method} should use Chromium's renderer-host ACK cleanup queue"
    );

    let mut completion = Box::pin(pending.wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut completion)
            .await
            .is_err(),
        "{method} must not reply before either its renderer ACK or Page replacement"
    );

    let replacement_url = format!(
        "data:text/html,<body data-marker='{replacement_marker}'>{replacement_marker}</body>"
    );
    ctx.install_navigation_fixture_for_session_owner(&replacement_url, Some(&page.session_id))
        .await;
    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some(&page.session_id))
        .expect("the replacement should install a Page owner");
    assert_ne!(original_owner, replacement_owner);

    let completed = tokio::time::timeout(Duration::from_secs(5), &mut completion)
        .await
        .unwrap_or_else(|_| panic!("{method} did not acknowledge Page replacement"));
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, scheduler_events) = ctx.complete_command_task_step_for_test(step).await;
    assert!(scheduler_events.is_empty());
    assert_eq!(
        messages,
        vec![json!({
            "id": command_id,
            "result": {},
            "sessionId": page.session_id
        })]
    );
    assert_eq!(
        page.evaluate_string(ctx, command_id + 1, "document.body.dataset.marker")
            .await,
        replacement_marker,
        "the original CDP session should remain usable on the replacement Page"
    );
}

// P0 browser contract source:
// Chromium Input.dispatchKeyEvent plus Playwright keyboard typing basics.
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_input_dispatch_key_event_inserts_text() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 138_000).await;

    page.navigate(&mut ctx, 138_005, fixture.url("/plain?input-key"))
        .await;
    page.evaluate_value(
        &mut ctx,
        138_006,
        r#"
            document.body.innerHTML = '<input id="field" value="">';
            window.__keyEvents = [];
            const field = document.getElementById('field');
            field.addEventListener('keydown', event => {
                window.__keyEvents.push({
                    type: event.type,
                    key: event.key,
                    code: event.code,
                    shiftKey: event.shiftKey,
                });
            });
            field.focus();
            'ready'
        "#,
    )
    .await;

    page.expect_empty_command(
        &mut ctx,
        138_007,
        "Input.dispatchKeyEvent",
        json!({
            "type": "keyDown",
            "key": "A",
            "code": "KeyA",
            "modifiers": 8,
            "text": "A"
        }),
    )
    .await;

    assert_eq!(
        page.evaluate_string(&mut ctx, 138_008, "document.getElementById('field').value")
            .await,
        "A"
    );
    assert_eq!(
        page.evaluate_string(&mut ctx, 138_009, "JSON.stringify(window.__keyEvents)")
            .await,
        r#"[{"type":"keydown","key":"A","code":"KeyA","shiftKey":true}]"#
    );
}

// Chromium source:
// content/browser/devtools/protocol/input_handler.cc InputInjector::Cleanup().
#[tokio::test(flavor = "multi_thread")]
async fn rust_cdp_p0_input_mouse_and_key_acknowledge_page_replacement() {
    let fixture = SmokeFixtureServer::start().await;
    let mut ctx = TestContext::new_with_target_discovery(false);
    let page = CdpPageHarness::attach(&mut ctx, 138_100).await;

    page.navigate(&mut ctx, 138_105, fixture.url("/plain?input-mouse-cleanup"))
        .await;
    page.evaluate_value(
        &mut ctx,
        138_106,
        r#"
            document.body.innerHTML = '<button style="position:absolute;left:0;top:0;width:80px;height:80px">go</button>';
            'ready'
        "#,
    )
    .await;
    expect_page_replacement_cleans_up_pending_input_ack(
        &mut ctx,
        &page,
        138_107,
        "Input.dispatchMouseEvent",
        json!({
            "type": "mousePressed",
            "x": 20,
            "y": 20,
            "button": "left",
            "buttons": 1,
            "clickCount": 1
        }),
        "mouse-replacement",
    )
    .await;

    page.navigate(&mut ctx, 138_109, fixture.url("/plain?input-key-cleanup"))
        .await;
    page.evaluate_value(
        &mut ctx,
        138_110,
        r#"
            document.body.innerHTML = '<input id="field">';
            document.getElementById('field').focus();
            'ready'
        "#,
    )
    .await;
    expect_page_replacement_cleans_up_pending_input_ack(
        &mut ctx,
        &page,
        138_111,
        "Input.dispatchKeyEvent",
        json!({
            "type": "keyDown",
            "key": "K",
            "code": "KeyK",
            "text": "K"
        }),
        "key-replacement",
    )
    .await;
}
