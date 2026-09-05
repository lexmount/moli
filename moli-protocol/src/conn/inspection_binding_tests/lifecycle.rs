use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn session_detach_reaches_renderer_without_protocol_document() {
    let mut ctx = dom_context().await;
    for session in ["SID-detached-inspection", "SID-surviving-inspection"] {
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .unwrap()
                .assign_attached_session_to_target("TID-dom-inspection", session.to_owned())
        );
    }
    ctx.conn.commit_declared_session_fixtures_for_test();
    for (id, session) in (1..).zip(["SID-detached-inspection", "SID-surviving-inspection"]) {
        ctx.process_async(json!({"id": id, "sessionId": session, "method": "Runtime.enable"}))
            .await;
        ctx.expect_result(id, json!({}), Some(session));
    }
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    // Both child sessions have already used the binding. Browser-owned work
    // must not inherit either frontend or resurrect its V8 session on detach.
    let mut document = take_inspection_document(&mut ctx.conn, &owner);
    let before = document
        .page
        .runtime_heap_usage_async()
        .await
        .unwrap()
        .moli
        .runtime
        .inspector_session_count;
    assert!(before >= 3, "both attached V8 sessions must exist");

    ctx.process_async(
        json!({"id": 3, "method": "Target.detachFromTarget", "params": {
            "targetId": "TID-dom-inspection", "sessionId": "SID-detached-inspection",
        }}),
    )
    .await;
    ctx.expect_result(3, json!({}), None);
    assert!(
        ctx.conn
            .session_route(Some("SID-detached-inspection"))
            .is_none()
    );
    // This real owner command is sequenced after detach finalization. The
    // frontend acknowledgement alone cannot prove that V8 cleanup happened.
    let after = document
        .page
        .runtime_heap_usage_async()
        .await
        .unwrap()
        .moli
        .runtime
        .inspector_session_count;
    assert_eq!(
        after,
        before - 1,
        "detach must destroy only the exact V8 session"
    );

    ctx.process_async(json!({"id": 4, "sessionId": "SID-surviving-inspection", "method": "Runtime.evaluate", "params": {
        "expression": "document.getElementById('inspected').textContent",
    }}))
    .await;
    let surviving = ctx.take_response_by_id(4);
    assert_eq!(surviving["result"]["result"]["value"], json!("value"));
    assert!(!ctx.conn.has_loaded_page_for_owner(&owner));
    drop(document);
}
