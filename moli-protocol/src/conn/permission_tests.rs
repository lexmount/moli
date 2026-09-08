use super::*;
use crate::testing::TestContext;
use serde_json::Value;

fn permission_command(conn: &mut CdpConnection, method: &str, params: Value) {
    let raw = json!({"id": 1, "method": method, "params": params}).to_string();
    let CdpCommandTaskStep::Complete(outcome) = conn.start_command_dispatch(&raw) else {
        panic!("permission policy without Documents must not start renderer work");
    };
    assert_eq!(outcome.into_parts().0, vec![json!({"id": 1, "result": {}})]);
}

fn set_permission(conn: &mut CdpConnection, context: Option<&str>, setting: &str) {
    let mut params = json!({"permission": {"name": "geolocation"}, "setting": setting});
    if let Some(context) = context {
        params["browserContextId"] = json!(context);
    }
    permission_command(conn, "Browser.setPermission", params);
}

fn permission_settings(conn: &CdpConnection, context: &str) -> Vec<String> {
    conn.effective_permission_overrides_for_browser_context_id(context)
        .into_iter()
        .map(|entry| entry.setting)
        .collect()
}

#[test]
fn permission_scope_order_tracks_writes_instead_of_fixed_scope_precedence() {
    let mut conn = crate::test_support::connection();
    let first = conn.new_browser_context_fixture_for_test("BID-first");
    let second = conn.new_browser_context_fixture_for_test("BID-second");
    conn.install_browser_context_fixture_for_test(first);
    conn.push_inactive_browser_context_fixture_for_test(second);

    set_permission(&mut conn, None, "denied");
    set_permission(&mut conn, Some("BID-first"), "granted");
    assert_eq!(
        permission_settings(&conn, "BID-first"),
        ["denied", "granted"]
    );
    assert_eq!(permission_settings(&conn, "BID-second"), ["denied"]);

    set_permission(&mut conn, None, "prompt");
    assert_eq!(
        permission_settings(&conn, "BID-first"),
        ["granted", "prompt"]
    );
    assert_eq!(permission_settings(&conn, "BID-second"), ["prompt"]);

    set_permission(&mut conn, Some("BID-first"), "denied");
    assert_eq!(
        permission_settings(&conn, "BID-first"),
        ["prompt", "denied"]
    );
    let later = conn.new_browser_context_fixture_for_test("BID-later");
    conn.push_inactive_browser_context_fixture_for_test(later);
    assert_eq!(permission_settings(&conn, "BID-later"), ["prompt"]);
}

#[test]
fn permission_resets_preserve_other_scopes_and_do_not_materialize_pages() {
    let mut conn = crate::test_support::connection_with_config(
        CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::default(),
    );
    set_permission(&mut conn, None, "denied");
    assert!(conn.browser_context.is_none());
    assert_eq!(
        conn.moli_memory_diagnostics()["isolateScope"]["estimatedRendererOwnerCount"],
        json!(0)
    );
    let first = conn.new_browser_context_fixture_for_test("BID-first");
    let second = conn.new_browser_context_fixture_for_test("BID-second");
    conn.install_browser_context_fixture_for_test(first);
    conn.push_inactive_browser_context_fixture_for_test(second);
    set_permission(&mut conn, Some("BID-first"), "granted");
    set_permission(&mut conn, Some("BID-second"), "prompt");

    permission_command(
        &mut conn,
        "Browser.resetPermissions",
        json!({"browserContextId": "BID-first"}),
    );
    assert_eq!(permission_settings(&conn, "BID-first"), ["denied"]);
    assert_eq!(
        permission_settings(&conn, "BID-second"),
        ["denied", "prompt"]
    );
    permission_command(&mut conn, "Browser.resetPermissions", json!({}));
    assert!(permission_settings(&conn, "BID-first").is_empty());
    assert!(permission_settings(&conn, "BID-second").is_empty());
    assert!(
        conn.browser_contexts()
            .all(|context| context.page_targets.is_empty())
    );
    assert_eq!(
        conn.moli_memory_diagnostics()["isolateScope"]["estimatedRendererOwnerCount"],
        json!(0)
    );
}

#[test]
fn permission_rules_leave_the_connection_with_their_context() {
    let mut conn = crate::test_support::connection();
    let original = conn.new_browser_context_fixture_for_test("same-context");
    conn.install_browser_context_fixture_for_test(original);
    set_permission(&mut conn, None, "denied");
    set_permission(&mut conn, Some("same-context"), "granted");
    assert_eq!(conn.permission_override_count(), 2);

    // No disposal handler or ID-keyed cleanup is allowed to be necessary.
    let removed = conn.browser_context.take().unwrap();
    let replacement = conn.new_browser_context_fixture_for_test("same-context");
    conn.install_browser_context_fixture_for_test(replacement);
    assert_ne!(
        removed.browser_context_id(),
        conn.browser_context.as_ref().unwrap().browser_context_id()
    );
    assert_eq!(permission_settings(&conn, "same-context"), ["denied"]);
    assert_eq!(conn.permission_override_count(), 1);
    assert_eq!(
        removed
            .permission_snapshot()
            .iter()
            .map(|entry| entry.setting.as_str())
            .collect::<Vec<_>>(),
        ["denied", "granted"]
    );
    drop(removed);
    assert_eq!(permission_settings(&conn, "same-context"), ["denied"]);
}

async fn current_permission(ctx: &mut TestContext) -> String {
    ctx.process_async(json!({"id": 9, "method": "Runtime.evaluate", "params": {
        "expression": "navigator.permissions.query({name:'geolocation'}).then(status => status.state)",
        "awaitPromise": true,
        "returnByValue": true,
    }})).await;
    let response = ctx.take_response_by_id(9);
    assert!(response.get("error").is_none(), "{response}");
    response["result"]["result"]["value"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_update_order_and_context_reset_reach_live_and_replacement_documents() {
    let mut ctx = TestContext::new();
    let mut context = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-permission-live");
    context.set_active_target_id("TID-permission-live");
    ctx.conn.install_browser_context_fixture_for_test(context);
    ctx.install_navigation_fixture_for_session_owner("data:text/html,<title>first</title>", None)
        .await;
    for (scope, setting) in [
        (None, "denied"),
        (Some("BID-permission-live"), "granted"),
        (None, "prompt"),
    ] {
        let mut params = json!({"permission": {"name": "geolocation"}, "setting": setting});
        if let Some(scope) = scope {
            params["browserContextId"] = json!(scope);
        }
        ctx.process_async(json!({"id": 8, "method": "Browser.setPermission", "params": params}))
            .await;
        ctx.expect_result(8, json!({}), None);
        assert_eq!(current_permission(&mut ctx).await, setting);
    }
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement</title>",
        None,
    )
    .await;
    assert_eq!(current_permission(&mut ctx).await, "prompt");
    ctx.process_async(json!({"id": 8, "method": "Browser.resetPermissions", "params": {"browserContextId": "BID-permission-live"}})).await;
    ctx.expect_result(8, json!({}), None);
    assert_eq!(current_permission(&mut ctx).await, "prompt");
}

async fn loaded_permission_context(ctx: &mut TestContext, title: &str) {
    let mut context = ctx
        .conn
        .new_browser_context_fixture_for_test("BID-permission-live");
    context.set_active_target_id("TID-permission-live");
    ctx.conn.install_browser_context_fixture_for_test(context);
    ctx.install_navigation_fixture_for_session_owner(
        &format!("data:text/html,<title>{title}</title>"),
        None,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_completion_preserves_its_ack_without_observing_a_replacement_document() {
    let mut ctx = TestContext::new();
    loaded_permission_context(&mut ctx, "first").await;
    ctx.process_async(json!({"id": 8, "method": "Browser.setPermission", "params": {
        "browserContextId": "BID-permission-live", "permission": {"name": "geolocation"}, "setting": "granted",
    }})).await;
    ctx.expect_result(8, json!({}), None);
    let old_document = ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .target_document_id("TID-permission-live")
        .unwrap();
    let mut updates = ctx.conn.start_permission_updates().unwrap();
    assert_eq!(updates.len(), 1);
    let completed = updates.pop().unwrap().wait().await;

    ctx.process_async(
        json!({"id": 8, "method": "Browser.setPermission", "params": {
            "permission": {"name": "geolocation"}, "setting": "denied",
        }}),
    )
    .await;
    ctx.expect_result(8, json!({}), None);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement</title>",
        None,
    )
    .await;
    assert_ne!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .target_document_id("TID-permission-live"),
        Some(old_document)
    );
    ctx.conn.finish_permission_update(completed).unwrap();
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .loaded_document_title_for_test()
            .unwrap(),
        "replacement"
    );
    assert_eq!(current_permission(&mut ctx).await, "denied");
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_completion_resolves_browser_identity_not_a_reused_context_id() {
    let mut ctx = TestContext::new();
    loaded_permission_context(&mut ctx, "first").await;
    ctx.process_async(json!({"id": 8, "method": "Browser.setPermission", "params": {
        "browserContextId": "BID-permission-live", "permission": {"name": "geolocation"}, "setting": "granted",
    }})).await;
    ctx.expect_result(8, json!({}), None);
    let mut updates = ctx.conn.start_permission_updates().unwrap();
    assert_eq!(updates.len(), 1);
    let completed = updates.pop().unwrap().wait().await;
    let old_context = ctx.conn.browser_context.take().unwrap();
    loaded_permission_context(&mut ctx, "new-context").await;
    assert_ne!(
        old_context.browser_context_id(),
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .browser_context_id()
    );
    assert_eq!(
        ctx.conn.finish_permission_update(completed),
        Err("NoDocumentLoaded".into())
    );
    assert!(permission_settings(&ctx.conn, "BID-permission-live").is_empty());
    assert_eq!(current_permission(&mut ctx).await, "prompt");
}
