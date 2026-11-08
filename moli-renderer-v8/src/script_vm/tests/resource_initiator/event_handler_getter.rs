use super::*;

#[tokio::test]
async fn resource_initiator_body_messageerror_getter_does_not_capture_reader_source() {
    let server = StaticHttpServer::spawn_with_responder(
        3,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    let reader = base.join("inspect.js").unwrap();
    for (index, access) in [
        "",
        "void document.body.onmessageerror;",
        "document.body.onmessageerror = document.body.onmessageerror;",
    ]
    .into_iter()
    .enumerate()
    {
        vm.eval(&format!(
            r#"
            {{
            const root = document.documentElement || document.appendChild(document.createElement('html'));
            const body = document.body || root.appendChild(document.createElement('body'));
            body.setAttribute('onmessageerror', "const x = new XMLHttpRequest(); x.open('GET', '/handler-{index}'); x.send();");
            }}
            "#,
        ))
        .unwrap();
        execute_with_source(&mut vm, access, &reader, &base);
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let target = crate::document_runtime::EventTargetHandle::Window;
            let event = host.prepare_native_event_dispatch(target, target, "messageerror");
            host.dispatch_native_event_in_scope(scope, host_ptr, event)
                .map_err(anyhow::Error::msg)
        })
        .unwrap();
    }
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "3",
        "native body messageerror requests",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => [new URL(entry.name).pathname, entry.initiatorUrl]).sort())").unwrap(),
        serde_json::json!([["/handler-0", ""], ["/handler-1", ""], ["/handler-2", reader.as_str()]]).to_string()
    );
    assert_eq!(server.finish().await.len(), 3);
}
