use super::*;
use moli_core::browser::{DownloadBehavior, DownloadPolicy};
use serde_json::Value;

fn command(conn: &mut CdpConnection, method: &str, params: Value) {
    let raw = json!({"id": 1, "method": method, "params": params}).to_string();
    let CdpCommandTaskStep::Complete(outcome) = conn.start_command_dispatch(&raw) else {
        panic!("download configuration must not start renderer work");
    };
    assert_eq!(outcome.into_parts().0, vec![json!({"id": 1, "result": {}})]);
}

#[test]
fn download_policy_scopes_preserve_observer_shadowing_and_reset_fallback() {
    let mut conn = crate::test_support::connection();
    conn.browser_context = Some(conn.new_browser_context_fixture_for_test("first"));
    conn.inactive_browser_contexts
        .push(conn.new_browser_context_fixture_for_test("second"));
    conn.configure_download_policy(
        None,
        DownloadPolicy {
            behavior: DownloadBehavior::Allow,
            download_path: Some("/global".into()),
        },
        Some(true),
    )
    .unwrap();
    command(
        &mut conn,
        "Browser.setDownloadBehavior",
        json!({
            "browserContextId": "first", "behavior": "deny", "eventsEnabled": true,
        }),
    );
    let first = conn.download_policy_for_browser_context(Some("first"));
    assert_eq!(first.behavior, DownloadBehavior::Deny);
    assert!(!conn.automation_download_events_enabled_for_context(Some("first")));
    let second = conn
        .download_policy_for_browser_context(Some("second"))
        .clone();
    assert_eq!(second.behavior, DownloadBehavior::Allow);
    assert_eq!(second.download_path.as_deref(), Some("/global"));
    assert!(conn.automation_download_events_enabled_for_context(Some("second")));

    conn.configure_download_policy(
        Some("first"),
        DownloadPolicy {
            behavior: DownloadBehavior::Allow,
            download_path: Some("/first".into()),
        },
        Some(true),
    )
    .unwrap();
    command(
        &mut conn,
        "Browser.setDownloadBehavior",
        json!({
            "browserContextId": "first", "behavior": "allowAndName", "downloadPath": "/updated",
        }),
    );
    assert!(conn.automation_download_events_enabled_for_context(Some("first")));
    assert!(conn.browser_download_event_session_ids().is_empty());
    conn.reset_download_policy(Some("first")).unwrap();
    assert_eq!(
        conn.download_policy_for_browser_context(Some("first")),
        second
    );
    assert!(conn.automation_download_events_enabled_for_context(Some("first")));

    conn.configure_download_policy(
        Some("second"),
        DownloadPolicy {
            behavior: DownloadBehavior::Deny,
            download_path: None,
        },
        Some(false),
    )
    .unwrap();
    conn.reset_download_policy(None).unwrap();
    assert_eq!(
        conn.download_policy_for_browser_context(Some("first")),
        DownloadPolicy::default()
    );
    assert!(!conn.automation_download_events_enabled_for_context(Some("first")));
    assert_eq!(
        conn.download_policy_for_browser_context(Some("second"))
            .behavior,
        DownloadBehavior::Deny
    );
}

#[test]
fn download_configuration_is_lazy_and_missing_allow_path_remains_explicit() {
    let mut conn = crate::test_support::connection_with_config(
        CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::default(),
    );
    command(
        &mut conn,
        "Browser.setDownloadBehavior",
        json!({"behavior": "allow"}),
    );
    assert!(conn.browser_context.is_none());
    assert_eq!(
        conn.moli_memory_diagnostics()["isolateScope"]["estimatedRendererOwnerCount"],
        json!(0)
    );
    assert_eq!(
        conn.download_policy_for_browser_context(None).behavior,
        DownloadBehavior::Allow
    );
    assert!(
        conn.download_policy_for_browser_context(None)
            .download_path
            .is_none()
    );
    conn.browser_context = Some(conn.new_browser_context_fixture_for_test("later"));
    assert_eq!(
        conn.download_policy_for_browser_context(Some("later"))
            .behavior,
        DownloadBehavior::Allow
    );
}

#[test]
fn download_policy_leaves_with_its_context_without_disposal_cleanup() {
    let mut conn = crate::test_support::connection();
    conn.browser_context = Some(conn.new_browser_context_fixture_for_test("same-context"));
    let policy = DownloadPolicy {
        behavior: DownloadBehavior::AllowAndName,
        download_path: Some("/owned".into()),
    };
    conn.configure_download_policy(Some("same-context"), policy.clone(), Some(true))
        .unwrap();
    conn.set_browser_download_events_enabled_for_session(Some("observer"), true);

    let removed = conn
        .browser_context
        .replace(conn.new_browser_context_fixture_for_test("same-context"))
        .unwrap();
    assert_eq!(removed.download_policy(), Some(policy));
    assert_eq!(removed.automation_download_events_enabled, Some(true));
    assert_eq!(
        conn.download_policy_for_browser_context(Some("same-context")),
        DownloadPolicy::default()
    );
    assert!(!conn.automation_download_events_enabled_for_context(Some("same-context")));
    assert_eq!(
        conn.browser_download_event_session_ids(),
        vec![Some("observer".into())]
    );
    drop(removed);
    assert!(
        conn.browser_context
            .as_ref()
            .unwrap()
            .download_policy()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_a_download_observer_preserves_browser_context_policy() {
    let mut ctx = crate::testing::TestContext::new();
    let mut context = BrowserContext::new("CTX-download".into());
    context.set_active_target_id("TID-download");
    context.attach_active_session("SID-download");
    ctx.conn.install_browser_context_fixture_for_test(context);
    ctx.process_async(json!({
        "id": 3, "sessionId": "SID-download", "method": "Browser.setDownloadBehavior",
        "params": {"browserContextId": "CTX-download", "behavior": "allowAndName", "downloadPath": "/owned", "eventsEnabled": true}
    })).await;
    ctx.expect_result(3, json!({}), Some("SID-download"));
    let policy = ctx
        .conn
        .download_policy_for_browser_context(Some("CTX-download"))
        .clone();
    assert_eq!(
        ctx.conn.browser_download_event_session_ids(),
        vec![Some("SID-download".into())]
    );
    ctx.process_async(json!({"id": 4, "method": "Target.detachFromTarget", "params": {"sessionId": "SID-download"}})).await;
    ctx.expect_result(4, json!({}), None);
    assert!(ctx.conn.browser_download_event_session_ids().is_empty());
    assert_eq!(
        ctx.conn
            .download_policy_for_browser_context(Some("CTX-download")),
        policy
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_is_crashed("TID-download")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_target_id_owned()
            .as_deref(),
        Some("TID-download")
    );
}
