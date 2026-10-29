use super::*;

fn beacon_url(server: &StaticHttpServer, path: &str) -> String {
    server.base_url().join(path).unwrap().to_string()
}

#[tokio::test(flavor = "current_thread")]
async fn link_audit_ping_uses_its_adopted_document_base_policy_and_owner() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            "<!doctype html><head><base href='/child-assets/'></head><body>child-ready</body>"
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &beacon_url(&server, "/top/page"),
        &loader,
    );
    vm.eval("globalThis.frame=document.body.appendChild(document.createElement('iframe'));frame.src='/child/page';true").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frame.contentDocument.body?.textContent==='child-ready')",
        "true",
        "real child Document commits before parent script activates an adopted link",
    )
    .await;
    vm.eval("document.head.appendChild(Object.assign(document.createElement('meta'),{httpEquiv:'Content-Security-Policy',content:\"connect-src 'none'\"}));globalThis.link=document.createElement('a');link.href=frame.contentWindow.location.href+'#destination';link.ping='audit';frame.contentDocument.body.appendChild(link);HTMLElement.prototype.click.call(link);true").unwrap();
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(matches!(
        accepted[0].1.dispatch_scope(),
        crate::native_bridge::OwnerDispatchScope::Child(_)
    ));
    assert!(
        !accepted[0].2,
        "network-only ping must not root its source realm"
    );
    let child_url = beacon_url(&server, "/child/page");
    let destination_url = beacon_url(&server, "/child/page#destination");
    let requests = tokio::time::timeout(std::time::Duration::from_secs(5), server.finish())
        .await
        .expect("child-owned ping must reach the real HTTP fixture");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].target, "/child/page");
    let request = &requests[1];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/child-assets/audit");
    assert_eq!(request.body, b"PING");
    assert_eq!(request.header_value("content-type"), Some("text/ping"));
    assert_eq!(request.header_value("referer"), None);
    assert_eq!(request.header_value("ping-from"), Some(child_url.as_str()));
    assert_eq!(
        request.header_value("ping-to"),
        Some(destination_url.as_str())
    );
}
