use std::io;

use crate::conn::BrowserContext;

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn browser_version_ignores_runtime_identity_changes() {
    use crate::testing::TestContext;

    let mut fetch = moli_fetch::FetchConfig::default();
    fetch.set_user_agent("BrowserStartup/1.0");
    let mut conn = CdpConnection::new_with_fetch_config(fetch);
    // This API can change even the base identity before a Context exists.
    // Browser version reporting must retain the original launch identity.
    conn.set_user_agent_override_async("RuntimeDefault/2.0")
        .await;
    assert_eq!(conn.user_agent(), "RuntimeDefault/2.0");
    let mut ctx = TestContext::from_conn(conn);
    ctx.process_async(json!({"id": 1, "method": "Browser.getVersion"}))
        .await;
    let browser = ctx.take_response_by_id(1)["result"].clone();
    assert_eq!(browser["userAgent"], "BrowserStartup/1.0");

    ctx.with_fixture(|conn| {
        conn.set_global_browser_identity_override_from_user_agent(Some(
            "RuntimeGlobal/3.0".to_owned(),
        ));
        assert_eq!(conn.user_agent(), "RuntimeGlobal/3.0");
    });
    ctx.process_async(json!({"id": 2, "method": "Browser.getVersion"}))
        .await;
    assert_eq!(ctx.take_response_by_id(2)["result"], browser);

    ctx.with_fixture(|conn| {
        let fallback = conn.base_browser_identity().clone();
        let mut first = BrowserContext::new_with_page_for_test("Context-A", "Page-A");
        first.set_default_user_agent_override(Some("ContextA/4.0".to_owned()), &fallback);
        conn.install_browser_context_fixture_for_test(first);
        let mut second = BrowserContext::new_with_page_for_test("Context-B", "Page-B");
        second.set_default_user_agent_override(Some("ContextB/5.0".to_owned()), &fallback);
        conn.push_inactive_browser_context_fixture_for_test(second);
    });
    for (index, (context, user_agent)) in [
        ("Context-A", "ContextA/4.0"),
        ("Context-B", "ContextB/5.0"),
        ("Context-A", "ContextA/4.0"),
    ]
    .into_iter()
    .enumerate()
    {
        ctx.with_fixture(|conn| {
            assert!(conn.activate_browser_context_by_id(context));
            assert_eq!(conn.user_agent(), user_agent);
        });
        let id = 3 + index as u64;
        ctx.process_async(json!({"id": id, "method": "Browser.getVersion"}))
            .await;
        assert_eq!(ctx.take_response_by_id(id)["result"], browser);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_projection_retains_live_owner_error_source() {
    let mut conn = CdpConnection::new();
    let mut context = BrowserContext::new("permission-error-context".to_owned());
    context.set_active_target_id("permission-error-target");
    conn.install_browser_context_fixture_for_test(context);
    let page = conn
        .load_page_via_runtime_async("about:blank")
        .await
        .unwrap();
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);
    let page = conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .loaded_page()
        .unwrap();
    let host = conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .host_identity();
    let target = PendingBrowserPageTarget {
        host,
        browser_context_id: "permission-error-context".to_owned(),
        target_id: "permission-error-target".to_owned(),
        page_id: page.page_id(),
        attachment_id: page.renderer_agent_attachment_id(),
    };

    let error = finish_pending_permission_override_command(
        &mut conn,
        CompletedBrowserPageCommand {
            target,
            completed: Err(io::Error::from(io::ErrorKind::BrokenPipe).into()),
        },
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[tokio::test]
async fn download_task_retains_join_error() {
    let pending = PendingBrowserCommandDispatch {
        command_id: Some(1),
        response_session_id: None,
        kind: PendingBrowserCommandKind::OpenDownloadAsStream {
            pending: tokio::spawn(async { panic!("download task failed") }),
        },
    };
    let CompletedBrowserCommandKind::OpenDownloadAsStream { completed } = pending.wait().await.kind
    else {
        panic!("expected download completion");
    };
    assert!(
        completed
            .unwrap_err()
            .downcast_ref::<tokio::task::JoinError>()
            .unwrap()
            .is_panic()
    );
}
