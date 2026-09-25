use super::*;

#[tokio::test]
async fn nested_frame_windows_use_accessing_realm_without_granting_top_access() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        include_str!("../../../../tests/fixtures/nested-frame-window-access.html").to_owned(),
    ])
    .await;
    let origin = server.base_url().origin().ascii_serialization();
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &format!("{}/top", origin.replace("127.0.0.1", "localhost")),
        "<!doctype html><body>top",
        &loader,
    );
    vm.eval(&format!(
        r#"
        globalThis.nestedAccessResult = null;
        const frame = document.createElement('iframe');
        frame.src = '{origin}/child';
        onmessage = event => {{
            let topCannotReadGrandchild = false;
            try {{ frame.contentWindow[0].document; }}
            catch (error) {{ topCannotReadGrandchild = error.name === 'SecurityError'; }}
            nestedAccessResult = {{...event.data, topCannotReadGrandchild,
                sourceIsChild: event.source === frame.contentWindow}};
        }};
        document.body.append(frame);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(nestedAccessResult !== null)",
        "true",
        "nested frame access matrix",
    )
    .await;
    assert_eq!(
        vm.eval(
            "JSON.stringify({...nestedAccessResult, \
             rows: nestedAccessResult.rows.filter(row => row.error || !row.checks.every(Boolean)), \
             count: nestedAccessResult.rows.length})"
        )
        .unwrap(),
        r#"{"rows":[],"opaqueDocumentNull":true,"denials":[true,true],"topDenied":true,"topCannotReadGrandchild":true,"sourceIsChild":true,"count":20}"#,
    );
    assert_eq!(server.finish_targets().await, vec!["/child"]);
}

#[tokio::test]
async fn frame_content_document_uses_getter_realm_after_document_domain_changes() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><body><iframe id=nested src='/nested'></iframe>".to_owned(),
        "<!doctype html><body>nested loaded".to_owned(),
    ])
    .await;
    let origin = server.base_url().origin().ascii_serialization();
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &format!("{origin}/top"),
        "<!doctype html><body>top",
        &loader,
    );
    vm.eval(&format!(
        r#"
        const frame = document.createElement('iframe'); frame.id = 'outer';
        frame.src = '{origin}/outer'; document.body.append(frame);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(document.getElementById('outer').contentDocument?.getElementById('nested')?.contentDocument?.body?.textContent === 'nested loaded')",
        "true",
        "loaded independent-origin nested documents",
    )
    .await;
    assert_eq!(
        vm.eval(&format!(
            "JSON.stringify({})",
            include_str!("../../../../tests/fixtures/frame-content-document-domain.js")
        ))
        .unwrap(),
        "[true,true,true,true,true,true,true,true,true,true]",
    );
    assert_eq!(server.finish_targets().await, vec!["/outer", "/nested"]);
}
