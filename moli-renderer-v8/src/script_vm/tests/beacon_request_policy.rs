use super::*;
use crate::runtime::PageVmTaskExecutorTestHarness;

fn beacon_url(server: &StaticHttpServer, path: &str) -> String {
    server.base_url().join(path).unwrap().to_string()
}

fn beacon_referrer_policy(vm: &mut PageVmTaskExecutorTestHarness, policy: &str) {
    vm.eval(&format!(
        "document.head.appendChild(Object.assign(document.createElement('meta'), {{name:'referrer',content:{policy:?}}}));true"
    )).unwrap();
}

async fn finish_beacon_timing(
    vm: &mut PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    url: &str,
) -> serde_json::Value {
    advance_page_task_executor_until_eval_equals(
        vm,
        loader,
        &format!("String(performance.getEntriesByName({url:?}).length)"),
        "1",
        "Beacon native resource timing at its transport terminal",
    )
    .await;
    serde_json::from_str(&vm.eval(&format!(
        "JSON.stringify(performance.getEntriesByName({url:?}).map(entry=>[entry.initiatorType,entry.responseStatus]))"
    )).unwrap()).unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_cors_preserves_content_type_credentials_referrer_and_native_lifetime() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/plain\r\nAccess-Control-Allow-Origin: http://beacon-source.test\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\n",
        |_, _| "ok".to_owned(),
    ).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://beacon-source.test/path?query=1",
        &loader,
    );
    beacon_referrer_policy(&mut vm, "origin");
    let url = beacon_url(&server, "/beacon");
    assert_eq!(vm.eval(&format!(
        "String(navigator.sendBeacon({url:?}, new Blob(['payload'],{{type:'application/octet-stream'}})))"
    )).unwrap(), "true");
    {
        let host = vm._context_host.borrow();
        let pending = host.pending_window_beacon_execution_contexts_for_test();
        assert_eq!(pending.len(), 1);
        assert!(
            !pending[0].2,
            "accepted Beacon must not retain its V8 context"
        );
        assert!(
            !host.has_pending_load_event_delaying_subresource_requests(),
            "an accepted Beacon must not delay the Document's load event"
        );
    }
    assert_eq!(
        finish_beacon_timing(&mut vm, &loader, &url).await,
        serde_json::json!([["beacon", 200]])
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "OPTIONS");
    assert_eq!(
        requests[0].header_value("access-control-request-headers"),
        Some("content-type")
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].body, b"payload");
    assert_eq!(
        requests[1].header_value("content-type"),
        Some("application/octet-stream")
    );
    for request in requests {
        assert_eq!(
            request.header_value("origin"),
            Some("http://beacon-source.test")
        );
        assert_eq!(
            request.header_value("referer"),
            Some("http://beacon-source.test/")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_preflight_requires_credential_permission_before_sending_body() {
    let server = StaticHttpServer::spawn_with_responder(
        1,
        "Content-Type: text/plain\r\nAccess-Control-Allow-Origin: http://beacon-source.test\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\n",
        |_, _| "ok".to_owned(),
    ).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("http://beacon-source.test/", &loader);
    let url = beacon_url(&server, "/no-credentials");
    assert_eq!(vm.eval(&format!(
        "String(navigator.sendBeacon({url:?},new Blob(['private'],{{type:'application/json'}})))"
    )).unwrap(), "true");
    assert_eq!(
        finish_beacon_timing(&mut vm, &loader, &url).await,
        serde_json::json!([["beacon", 0]])
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "OPTIONS");
    assert!(requests[0].body.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_interception_keeps_request_policy_across_document_open() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/plain\r\nAccess-Control-Allow-Origin: http://beacon-source.test\r\nAccess-Control-Allow-Credentials: true\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\n",
        |_, _| "ok".to_owned(),
    ).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://beacon-source.test/page",
        &loader,
    );
    beacon_referrer_policy(&mut vm, "no-referrer");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    let url = beacon_url(&server, "/paused");
    assert_eq!(vm.eval(&format!(
        "String(navigator.sendBeacon({url:?}, new Blob(['payload'],{{type:'application/json'}})))"
    )).unwrap(), "true");
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2);
    vm.eval("document.open();document.write(\"<!doctype html><meta name=referrer content=unsafe-url><meta http-equiv=Content-Security-Policy content=\\\"connect-src 'none'\\\">\");document.close();true").unwrap();
    vm.set_fetch_subresource_interception(false, None);
    vm.continue_pending_subresource_fetch(accepted[0].0, None, None, None, None, false, false)
        .unwrap();
    assert_eq!(
        finish_beacon_timing(&mut vm, &loader, &url).await,
        serde_json::json!([["beacon", 200]])
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "OPTIONS");
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].body, b"payload");
    assert_eq!(
        requests[1].header_value("content-type"),
        Some("application/json")
    );
    assert!(
        requests
            .iter()
            .all(|request| request.header_value("referer").is_none())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_referrer_policy_uses_the_initiating_document() {
    for policy in [
        "no-referrer",
        "origin",
        "strict-origin",
        "same-origin",
        "unsafe-url",
    ] {
        let server =
            StaticHttpServer::spawn_with_responder(1, "Content-Type: text/plain\r\n", |_, _| {
                "ok".to_owned()
            })
            .await;
        let loader = static_http_loader(std::iter::empty::<String>());
        let document = beacon_url(&server, "/path/page?query=1#fragment");
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document, &loader);
        beacon_referrer_policy(&mut vm, policy);
        let url = beacon_url(&server, "/beacon");
        assert_eq!(
            vm.eval(&format!("String(navigator.sendBeacon({url:?},'payload'))"))
                .unwrap(),
            "true"
        );
        assert_eq!(
            finish_beacon_timing(&mut vm, &loader, &url).await,
            serde_json::json!([["beacon", 200]])
        );
        let expected = match policy {
            "no-referrer" => None,
            "origin" | "strict-origin" => Some(server.base_url().to_string()),
            _ => Some(document.trim_end_matches("#fragment").to_owned()),
        };
        let requests = server.finish().await;
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].header_value("content-type"),
            Some("text/plain;charset=UTF-8")
        );
        assert_eq!(
            requests[0].header_value("referer"),
            expected.as_deref(),
            "{policy}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn link_audit_ping_omits_referer_without_losing_ping_headers() {
    let server =
        StaticHttpServer::spawn_with_responder(1, "Content-Type: text/plain\r\n", |_, _| {
            "ok".to_owned()
        })
        .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let document = beacon_url(&server, "/page?query=1");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document, &loader);
    let url = beacon_url(&server, "/ping");
    vm.eval(&format!(
        "const a=document.body.appendChild(document.createElement('a'));a.href='#destination';a.ping={url:?};a.click();true"
    )).unwrap();
    assert_eq!(
        finish_beacon_timing(&mut vm, &loader, &url).await,
        serde_json::json!([["ping", 200]])
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.body, b"PING");
    assert_eq!(request.header_value("content-type"), Some("text/ping"));
    assert_eq!(request.header_value("referer"), None);
    assert_eq!(request.header_value("ping-from"), Some(document.as_str()));
    assert_eq!(
        request.header_value("ping-to"),
        Some(format!("{document}#destination").as_str())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn link_audit_ping_uses_its_adopted_document_base_policy_and_timeline() {
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
    vm.eval("globalThis.childPerformance=frame.contentWindow.performance;document.head.appendChild(Object.assign(document.createElement('meta'),{httpEquiv:'Content-Security-Policy',content:\"connect-src 'none'\"}));globalThis.link=document.createElement('a');link.href=frame.contentWindow.location.href+'#destination';link.ping='audit';frame.contentDocument.body.appendChild(link);HTMLElement.prototype.click.call(link);true").unwrap();
    let url = beacon_url(&server, "/child-assets/audit");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        &format!("String(childPerformance.getEntriesByName({url:?}).length)"),
        "1",
        "Ping uses the link Document despite its caller's stricter CSP",
    )
    .await;
    assert_eq!(vm.eval(&format!("JSON.stringify(childPerformance.getEntriesByName({url:?}).map(entry=>[entry.initiatorType,entry.responseStatus]))")).unwrap(), r#"[["ping",200]]"#);
    assert_eq!(
        vm.eval(&format!(
            "String(performance.getEntriesByName({url:?}).length)"
        ))
        .unwrap(),
        "0"
    );
    let child_url = beacon_url(&server, "/child/page");
    let destination_url = beacon_url(&server, "/child/page#destination");
    let requests = server.finish().await;
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

fn complete_intercepted_beacon(vm: &mut StandaloneScriptVmHarness, internal_id: u64, url: &str) {
    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id,
        request_url: Url::parse(url).unwrap(),
        request_method: "POST".to_owned(),
        request_headers: Vec::new().into(),
        request_body: Some("payload".to_owned()),
        response_status_text: Some("No Content".to_owned()),
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: None,
        result: Ok(crate::types::NavigationResponse::from_text_body(
            Url::parse(url).unwrap(),
            204,
            Vec::new(),
            String::new(),
        ))
        .into(),
    })
    .unwrap();
}

#[test]
fn beacon_csp_rejection_precedes_quota_and_does_not_create_network_timing() {
    let mut vm = new_storage_html_test_vm("https://beacon-policy.test/page");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval("document.head.appendChild(Object.assign(document.createElement('meta'),{httpEquiv:'Content-Security-Policy',content:\"connect-src 'none'\"}));true").unwrap();
    assert_eq!(vm.eval("JSON.stringify([navigator.sendBeacon('/blocked','payload'),navigator.sendBeacon('/blocked','x'.repeat(65537))])").unwrap(), "[true,true]");
    assert!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test()
            .is_empty()
    );
    assert_eq!(
        vm.eval(
            "String(performance.getEntriesByName('https://beacon-policy.test/blocked').length)"
        )
        .unwrap(),
        "0"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_argument_conversion_uses_the_current_document_in_the_same_window() {
    let server =
        StaticHttpServer::spawn_with_responder(2, "Content-Type: text/plain\r\n", |_, _| {
            "ok".to_owned()
        })
        .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let document = beacon_url(&server, "/main");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document, &loader);
    for kind in ["url", "body"] {
        assert_eq!(vm.eval(&format!(r#"(() => {{
            const kind={kind:?};
            const rewrite=()=>{{
                document.open();
                document.write('<!doctype html><head><base href="/changed/'+kind+'/"></head><body>replacement</body>');
                document.close();
                return kind==='url'?'capture?case=url':'payload-body';
            }};
            return String(navigator.sendBeacon(
                kind==='url'?{{toString:rewrite}}:'capture?case=body',
                kind==='body'?{{toString:rewrite}}:'payload-url'));
        }})()"#)).unwrap(), "true", "{kind}");
        let url = beacon_url(&server, &format!("/changed/{kind}/capture?case={kind}"));
        assert_eq!(
            finish_beacon_timing(&mut vm, &loader, &url).await,
            serde_json::json!([["beacon", 200]])
        );
    }
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    for (request, kind) in requests.iter().zip(["url", "body"]) {
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.target,
            format!("/changed/{kind}/capture?case={kind}")
        );
        assert_eq!(request.body, format!("payload-{kind}").as_bytes());
        assert_eq!(request.header_value("referer"), Some(document.as_str()));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn csp_blocked_link_ping_records_failure_without_admitting_full_keepalive_quota() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://ping-policy.test/page",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    assert_eq!(
        vm.eval("String(navigator.sendBeacon('/filled','x'.repeat(65536)))")
            .unwrap(),
        "true"
    );
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    vm.eval("document.head.appendChild(Object.assign(document.createElement('meta'),{httpEquiv:'Content-Security-Policy',content:\"connect-src 'none'\"}));const a=document.body.appendChild(document.createElement('a'));a.href='#destination';a.ping='/blocked';a.click();true").unwrap();
    assert_eq!(
        finish_beacon_timing(&mut vm, &loader, "https://ping-policy.test/blocked").await,
        serde_json::json!([["ping", 0]])
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted
    );
    assert_eq!(
        vm.eval("String(navigator.sendBeacon('/blocked-beacon','x'.repeat(65537)))")
            .unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval(
            "String(performance.getEntriesByName('https://ping-policy.test/blocked-beacon').length)"
        )
        .unwrap(),
        "0"
    );
}

#[test]
fn beacon_document_open_keeps_the_window_timeline_without_retaining_v8() {
    let mut vm = new_storage_test_vm("https://beacon-timeline.test/page");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    assert_eq!(
        vm.eval("String(navigator.sendBeacon('/beacon','payload'))")
            .unwrap(),
        "true"
    );
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2);
    vm.eval("globalThis.originalPerformance=performance;document.open();document.write('<title>replacement</title>');document.close();true").unwrap();
    complete_intercepted_beacon(
        &mut vm,
        accepted[0].0,
        "https://beacon-timeline.test/beacon",
    );
    assert_eq!(vm.eval("String(performance===originalPerformance && performance.getEntriesByName('https://beacon-timeline.test/beacon').length===1 && performance.getEntriesByName('https://beacon-timeline.test/beacon')[0].responseStatus===204)").unwrap(), "true");
}

#[test]
fn beacon_registered_native_proxy_uses_its_navigator_window_without_author_traps() {
    let mut vm = new_storage_html_test_vm("https://beacon-native-proxy.test/page");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval("globalThis.frame=document.body.appendChild(document.createElement('iframe'));globalThis.childNavigator=frame.contentWindow.navigator;globalThis.childPerformance=frame.contentWindow.performance;globalThis.aliasTraps=0;globalThis.aliasTrap=()=>{aliasTraps++;throw Error('author trap')};true").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let target_key = crate::util::v8str(scope, "childNavigator");
        let target = global.get(scope, target_key.into()).unwrap();
        let target = v8::Local::<v8::Object>::try_from(target).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let trap_key = crate::util::v8str(scope, "aliasTrap");
        let trap = global.get(scope, trap_key.into()).unwrap();
        let get_key = crate::util::v8str(scope, "get");
        assert_eq!(
            handler.create_data_property(scope, get_key.into(), trap),
            Some(true)
        );
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let alias_key = crate::util::v8str(scope, "navigatorAlias");
        assert_eq!(
            global.create_data_property(scope, alias_key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("String(Navigator.prototype.sendBeacon.call(navigatorAlias,'/alias','payload'))")
            .unwrap(),
        "true"
    );
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2);
    complete_intercepted_beacon(
        &mut vm,
        accepted[0].0,
        "https://beacon-native-proxy.test/alias",
    );
    assert_eq!(vm.eval("String(childPerformance.getEntriesByName('https://beacon-native-proxy.test/alias').length===1 && performance.getEntriesByName('https://beacon-native-proxy.test/alias').length===0 && aliasTraps===0)").unwrap(), "true");
    assert_eq!(vm.eval("(() => {let conversions=0,error;try{Navigator.prototype.sendBeacon.call(new Proxy(navigatorAlias,{get:aliasTrap}),{toString(){conversions++;return '/author'}},'payload')}catch(caught){error=caught}return String(error instanceof TypeError && conversions===0 && aliasTraps===0)})()").unwrap(), "true");
}

#[test]
fn beacon_window_retirement_records_one_failure_and_keeps_accepted_transport() {
    let mut vm = new_storage_html_test_vm("https://beacon-timeline.test/page");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval("globalThis.frame=document.body.appendChild(document.createElement('iframe'));globalThis.oldPerformance=frame.contentWindow.performance;frame.contentWindow.navigator.sendBeacon('/retired','payload');true").unwrap();
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2);
    vm.eval("frame.remove();true").unwrap();
    assert_eq!(vm.eval("JSON.stringify(oldPerformance.getEntriesByName('https://beacon-timeline.test/retired').map(entry=>[entry.initiatorType,entry.responseStatus]))").unwrap(), r#"[["beacon",0]]"#);
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted
    );
    complete_intercepted_beacon(
        &mut vm,
        accepted[0].0,
        "https://beacon-timeline.test/retired",
    );
    assert_eq!(vm.eval("JSON.stringify(oldPerformance.getEntriesByName('https://beacon-timeline.test/retired').map(entry=>[entry.initiatorType,entry.responseStatus]))").unwrap(), r#"[["beacon",0]]"#);
    assert_eq!(
        vm.eval(
            "String(performance.getEntriesByName('https://beacon-timeline.test/retired').length)"
        )
        .unwrap(),
        "0"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn beacon_child_navigation_finishes_the_retired_performance_timeline() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://beacon-navigation.test/page",
        &loader,
    );
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Ping));
    vm.eval("globalThis.frame=document.body.appendChild(document.createElement('iframe'));globalThis.oldPerformance=frame.contentWindow.performance;frame.contentWindow.navigator.sendBeacon('/retired','payload');true").unwrap();
    let accepted = vm
        ._context_host
        .borrow()
        .pending_window_beacon_execution_contexts_for_test();
    assert_eq!(accepted.len(), 1);
    assert!(!accepted[0].2);
    vm.eval("frame.srcdoc='<p>successor</p>';true").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(frame.contentDocument.body?.textContent==='successor')",
        "true",
        "child navigation commits its successor Document",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(oldPerformance.getEntriesByName('https://beacon-navigation.test/retired').map(entry=>[entry.initiatorType,entry.responseStatus]))").unwrap(), r#"[["beacon",0]]"#);
    assert_eq!(vm.eval("String(frame.contentWindow.performance!==oldPerformance && frame.contentWindow.performance.getEntriesByName('https://beacon-navigation.test/retired').length===0)").unwrap(), "true");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_window_beacon_execution_contexts_for_test(),
        accepted,
        "retiring the old timeline must preserve the native transport without a V8 root"
    );
}
