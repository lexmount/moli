use std::io;

use crate::conn::BrowserContext;

use super::*;

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
