use super::*;
use crate::automation::DevToolsHandleJavaScriptDialogCommand;
use std::time::Duration;

#[derive(Clone, Copy)]
enum SubscriptionRelease {
    DisablePages,
    DisableThenDetach,
    DetachPages,
    DisableAndDetachAttached,
}

async fn page_command(ctx: &mut TestContext, id: u64, method: &str, session: &str) {
    ctx.process_async(json!({"id": id, "method": method, "sessionId": session}))
        .await;
    assert_eq!(take_response_by_id(ctx, id)["result"], json!({}));
}

async fn detach(ctx: &mut TestContext, id: u64, session: &str) {
    ctx.process_async(json!({
        "id": id, "method": "Target.detachFromTarget", "params": {"sessionId": session},
    }))
    .await;
    assert_eq!(take_response_by_id(ctx, id)["result"], json!({}));
}

fn assert_modal_wait_enabled(ctx: &TestContext, enabled: bool) {
    ctx.with_fixture(move |conn| {
        assert!(conn.has_automation_javascript_dialog_handler());
        assert_eq!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .renderer_runtime()
                .javascript_dialog_handler_enabled(),
            enabled,
        );
    });
}

async fn assert_non_modal_confirm(
    ctx: &mut TestContext,
    target: &'static str,
    probe: &'static str,
    id: u64,
) {
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: None,
        target_id: Some(target.into()),
        browser_context_id: Some("BID-shared-dialogs".into()),
    };
    let evaluated = tokio::time::timeout(
        Duration::from_secs(5),
        ctx.process_and_wait_for_response_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": probe,
            "params": {"expression": "confirm('non-modal shared dialog')", "returnByValue": true},
        })),
    )
    .await;
    if evaluated.is_err() {
        let cleanup_context = context.clone();
        ctx.with_fixture(move |conn| {
            // Settle a stale modal wait before failing, so the regression does
            // not strand the renderer while the test tears down its owner.
            let _ = conn.try_execute_immediate_automation_command(
                AutomationCommand::HandleJavaScriptDialog(DevToolsHandleJavaScriptDialogCommand {
                    context: cleanup_context,
                    accept: false,
                    prompt_text: None,
                }),
            );
        });
    }
    evaluated.expect("a disabled shared handler must not leave a VM-local modal wait");
    let response = take_response_by_id(ctx, id);
    assert!(response.get("error").is_none(), "{response:?}");
    assert_eq!(response["result"]["result"]["value"], json!(false));
    ctx.with_fixture(|conn| assert!(conn.has_pending_javascript_dialog()));
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::HandleJavaScriptDialog(DevToolsHandleJavaScriptDialogCommand {
            context,
            accept: true,
            prompt_text: None,
        }),
    )
    .await
    .expect("the native handler remains able to accept the non-modal prompt");
    ctx.with_fixture(|conn| assert!(!conn.has_pending_javascript_dialog()));
}

async fn assert_shared_dialog_mode_after_release(release: SubscriptionRelease) {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-shared-dialogs",
        "TID-a",
        "SID-a",
        "about:blank",
    );
    ctx.with_fixture(|conn| {
        let context = conn.browser_context.as_mut().unwrap();
        context.stage_background_target(
            "TID-b".to_owned(),
            Some("SID-b".to_owned()),
            "about:blank".to_owned(),
            Some("about:blank".to_owned()),
            None,
        );
        assert!(context.assign_attached_session_to_target("TID-a", "SID-a-peer".to_owned()));
        // Probe sessions stay attached without enabling Page, so evaluating a
        // dialog does not reuse the Primary session being disabled or detached.
        assert!(context.assign_attached_session_to_target("TID-a", "SID-a-probe".to_owned()));
        assert!(context.assign_attached_session_to_target("TID-b", "SID-b-probe".to_owned()));
        conn.commit_declared_session_fixtures_for_test();
        assert!(
            conn.register_automation_javascript_dialog_handler(
                FrontendProtocol::WebDriverBidi,
                false
            )
        );
    });
    page_command(&mut ctx, 300, "Page.enable", "SID-a").await;
    ensure_initial_document_for_session(&mut ctx, Some("SID-a")).await;
    page_command(&mut ctx, 301, "Page.enable", "SID-b").await;
    ensure_initial_document_for_session(&mut ctx, Some("SID-b")).await;
    assert_modal_wait_enabled(&ctx, true);

    match release {
        SubscriptionRelease::DisablePages => {
            page_command(&mut ctx, 302, "Page.disable", "SID-a").await;
            assert_modal_wait_enabled(&ctx, true);
            page_command(&mut ctx, 303, "Page.disable", "SID-b").await;
        }
        SubscriptionRelease::DisableThenDetach => {
            page_command(&mut ctx, 302, "Page.disable", "SID-a").await;
            assert_modal_wait_enabled(&ctx, true);
            detach(&mut ctx, 303, "SID-b").await;
        }
        SubscriptionRelease::DetachPages => {
            detach(&mut ctx, 302, "SID-a").await;
            assert_modal_wait_enabled(&ctx, true);
            detach(&mut ctx, 303, "SID-b").await;
        }
        SubscriptionRelease::DisableAndDetachAttached => {
            page_command(&mut ctx, 302, "Page.enable", "SID-a-peer").await;
            page_command(&mut ctx, 303, "Page.disable", "SID-a").await;
            page_command(&mut ctx, 304, "Page.disable", "SID-b").await;
            assert_modal_wait_enabled(&ctx, true);
            detach(&mut ctx, 305, "SID-a-peer").await;
        }
    }
    assert_modal_wait_enabled(&ctx, false);
    assert_non_modal_confirm(&mut ctx, "TID-a", "SID-a-probe", 310).await;
    assert_non_modal_confirm(&mut ctx, "TID-b", "SID-b-probe", 311).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_two_pages_releases_both_renderer_dialog_waits() {
    assert_shared_dialog_mode_after_release(SubscriptionRelease::DisablePages).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disabling_one_page_and_detaching_the_other_releases_both_dialog_waits() {
    assert_shared_dialog_mode_after_release(SubscriptionRelease::DisableThenDetach).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_both_page_sessions_releases_both_dialog_waits() {
    assert_shared_dialog_mode_after_release(SubscriptionRelease::DetachPages).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_last_attached_observer_releases_all_page_dialog_waits() {
    assert_shared_dialog_mode_after_release(SubscriptionRelease::DisableAndDetachAttached).await;
}
