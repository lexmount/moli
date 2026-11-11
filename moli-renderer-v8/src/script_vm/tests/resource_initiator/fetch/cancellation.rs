use super::*;

#[test]
fn fetch_resource_timing_records_abort_once_and_ignores_late_completion() {
    let document = Url::parse("https://fetch-abort-timing.test/page").unwrap();
    for streaming in [false, true] {
        let mut vm = new_storage_test_vm(document.as_str());
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Fetch),
        );
        execute_with_source(
            &mut vm,
            r#"
            globalThis.controller = new AbortController();
            globalThis.observed = [];
            new PerformanceObserver(list => observed.push(...list.getEntries())).observe({type:'resource'});
            globalThis.outcome = '';
            fetch('/asset', {signal:controller.signal}).then(() => outcome='headers', error => outcome=error.name);
        "#,
            &document.join("caller.js").unwrap(),
            &document,
        );
        let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
        if streaming {
            let body_source_id = crate::network_host::new_network_body_source_id();
            let response = crate::protocol_types::NavigationResponse::from_text_body(
                request.url.clone(),
                200,
                vec![("Content-Type".into(), b"text/plain".to_vec())],
                String::new(),
            );
            vm.start_streaming_async_subresource_fetch(
                crate::types::AsyncSubresourceStreamingStarted {
                    internal_id: request.internal_id,
                    request_url: request.url.clone(),
                    request_method: request.method.clone(),
                    request_headers: request.request_headers.clone(),
                    request_body: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    body_source_id,
                    head: response.head(),
                    network_request_headers: None,
                },
            )
            .unwrap();
            vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"partial".to_vec());
        }
        vm.exec("controller.abort();", None).unwrap();
        vm.exec("0", None).unwrap();
        assert_eq!(
            vm.eval("outcome").unwrap(),
            if streaming { "headers" } else { "AbortError" }
        );
        assert_eq!(
            vm.eval(
                "String(performance.getEntriesByName(new URL('/asset',location.href).href).length)"
            )
            .unwrap(),
            "1",
            "streaming={streaming}"
        );
        assert_eq!(
            vm.eval("String(observed.filter(e=>e.initiatorType==='fetch').length)")
                .unwrap(),
            "1"
        );
        vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
            internal_id: request.internal_id,
            request_url: request.url.clone(),
            request_method: request.method,
            request_headers: request.request_headers,
            request_body: None,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            result: Err("late cancellation completion".to_owned()).into(),
        })
        .unwrap();
        vm.exec("0", None).unwrap();
        assert_eq!(
            vm.eval("String(performance.getEntriesByType('resource').length)")
                .unwrap(),
            "1"
        );
        assert_eq!(vm.eval("String(observed.length)").unwrap(), "1");
    }
}

#[test]
fn fetch_resource_timing_pre_aborted_signal_has_no_request_or_entry() {
    let mut vm = new_storage_test_vm("https://fetch-abort-timing.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.exec("const controller = new AbortController();controller.abort();fetch('/asset',{signal:controller.signal}).catch(()=>{});",None).unwrap();
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    assert_eq!(
        vm.eval("String(performance.getEntriesByType('resource').length)")
            .unwrap(),
        "0"
    );
}

#[test]
fn fetch_resource_timing_abort_uses_the_fetch_realm_instead_of_the_signal_realm() {
    let mut vm = new_storage_html_test_vm("https://fetch-abort-timing.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.exec(
        r#"
        globalThis.frame = document.body.appendChild(document.createElement('iframe'));
        globalThis.controller = new AbortController();
        frame.contentWindow.fetch('/asset',{signal:controller.signal}).catch(()=>{});
        controller.abort();
    "#,
        None,
    )
    .unwrap();
    assert_eq!(
        vm.eval("String(performance.getEntriesByType('resource').length)")
            .unwrap(),
        "0"
    );
    assert_eq!(
        vm.eval("String(frame.contentWindow.performance.getEntriesByType('resource').length)")
            .unwrap(),
        "1"
    );
}
