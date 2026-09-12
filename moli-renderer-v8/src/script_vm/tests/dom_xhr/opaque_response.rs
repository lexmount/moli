use super::*;
use crate::util::v8str;

#[test]
fn window_service_worker_response_filter_survives_streaming_clone_and_cache() {
    use crate::types::AsyncSubresourceFetchResponseFilter as Filter;
    for streaming in [false, true] {
        for mode in ["cors", "no-cors"] {
            for cors in [false, true] {
                let mut vm = new_storage_test_vm("https://response-client.test/");
                vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Fetch),
                );
                let response_type = if cors { "cors" } else { "basic" };
                vm.eval(&format!(r#"
                    globalThis.result = 'pending';
                    fetch('https://response-remote.test/body', {{mode: '{mode}'}}).then(async response => {{
                        globalThis.original = response;
                        globalThis.cloned = response.clone();
                        const bucket = await navigator.storageBuckets.open('response-filter');
                        const cache = await bucket.caches.open('responses');
                        await cache.put('/key', cloned.clone());
                        globalThis.cached = await cache.match('/key');
                        await navigator.storageBuckets.delete('response-filter');
                        for (const entry of [original, cloned, cached]) {{
                            if (entry.type !== '{response_type}' || entry.status !== 200 || entry.body === null ||
                                entry.headers.get('x-visible') !== 'visible' || entry.headers.has('set-cookie') ||
                                entry.headers.has('x-hidden') === {cors}) throw new Error('public response');
                            let error;
                            try {{ entry.headers.set('x-author', 'changed'); }} catch (value) {{ error = value; }}
                            if (!(error instanceof TypeError)) throw new Error('immutable headers');
                            if (await entry.clone().text() !== 'hello') throw new Error('body');
                        }}
                        result = 'ok';
                    }}).catch(error => result = String(error.stack || error));
                "#)).unwrap();
                let requests = vm.take_pending_subresource_fetch_infos();
                assert_eq!(requests.len(), 1);
                let request = &requests[0];
                let headers = vec![
                    ("Content-Type".to_owned(), "text/plain".to_owned()),
                    ("Content-Length".to_owned(), "5".to_owned()),
                    ("X-Visible".to_owned(), "visible".to_owned()),
                    ("X-Hidden".to_owned(), "secret".to_owned()),
                    (
                        "Cross-Origin-Resource-Policy".to_owned(),
                        "cross-origin".to_owned(),
                    ),
                    ("Set-Cookie".to_owned(), "hidden=secret".to_owned()),
                    (
                        "Vary".to_owned(),
                        if cors { "*" } else { "Accept" }.to_owned(),
                    ),
                ];
                let head = moli_fetch::ResponseHead {
                    final_url: request.url.clone(),
                    status: 200,
                    status_text: Some("OK".to_owned()),
                    headers: headers.clone(),
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                };
                let response_filter = Some(if cors {
                    Filter::Cors(vec!["content-type".to_owned(), "x-visible".to_owned()])
                } else {
                    Filter::Basic
                });
                if streaming {
                    let body_source_id = crate::network_host::new_network_body_source_id();
                    vm.start_streaming_async_subresource_fetch(
                        crate::types::AsyncSubresourceStreamingStarted {
                            response_filter,
                            skip_fetch_security_validation: true,
                            internal_id: request.internal_id,
                            request_url: request.url.clone(),
                            request_method: "GET".to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            body_source_id,
                            head,
                            network_request_headers: None,
                        },
                    )
                    .unwrap();
                    vm.append_streaming_async_subresource_fetch_chunk(
                        body_source_id,
                        b"hello".to_vec(),
                    );
                    vm.finish_streaming_async_subresource_fetch(
                        request.internal_id,
                        body_source_id,
                        Ok(()),
                    )
                    .unwrap();
                } else {
                    vm.complete_async_subresource_fetch(
                        crate::types::AsyncSubresourceFetchCompletion {
                            response_filter,
                            skip_fetch_security_validation: true,
                            internal_id: request.internal_id,
                            request_url: request.url.clone(),
                            request_method: "GET".to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            response_status_text: None,
                            network_error_text: None,
                            result: Ok(
                                crate::protocol_types::NavigationResponse::from_head_and_body(
                                    head,
                                    "hello".to_owned(),
                                    b"hello".to_vec(),
                                ),
                            ),
                        },
                    )
                    .unwrap();
                }
                vm.exec("0", None).unwrap();
                assert_eq!(
                    vm.eval("result").unwrap(),
                    "ok",
                    "{response_type}/{mode}/streaming={streaming}"
                );
                let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context;
                vm.renderer_document_isolate
                    .with_entered_renderer_document_isolate(move |isolate| {
                        let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                        let scope = &mut scope.init();
                        let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                        let scope = &mut v8::ContextScope::new(scope, context);
                        for name in ["original", "cloned", "cached"] {
                            let value = context
                                .global(scope)
                                .get(scope, v8str(scope, name).into())
                                .unwrap();
                            let (head, _) =
                                crate::network_host::materialize_response_object_internal_head(
                                    scope, value, "test",
                                )
                                .unwrap();
                            assert_eq!(head.response_type, response_type);
                            for (name, value) in &headers {
                                assert!(
                                    head.headers
                                        .iter()
                                        .any(|(key, entry)| key.eq_ignore_ascii_case(name)
                                            && entry == value),
                                    "{name}: {:?}",
                                    head.headers
                                );
                            }
                            if cors {
                                assert_eq!(
                                    head.cors_exposed_header_names,
                                    Some(vec!["content-type".to_owned(), "x-visible".to_owned()])
                                );
                            }
                        }
                        Ok(())
                    })
                    .unwrap();
            }
        }
    }
}

#[test]
fn window_filtered_fetch_preserves_internal_head_through_clone_and_cache() {
    for streaming in [false, true] {
        for (mode, redirect, status, response_type) in [
            ("no-cors", "follow", 200, "opaque"),
            ("cors", "manual", 302, "opaqueredirect"),
        ] {
            let mut vm = new_storage_test_vm("https://opaque-response.test/");
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Fetch),
            );
            vm.eval(&format!(r#"
                globalThis.opaqueResult = 'pending';
                fetch('https://remote-opaque-response.test/response', {{mode: '{mode}', redirect: '{redirect}'}})
                  .then(async response => {{
                    globalThis.original = response;
                    globalThis.cloned = response.clone();
                    const bucket = await navigator.storageBuckets.open('opaque-response');
                    const cache = await bucket.caches.open('responses');
                    await cache.put('/key', cloned.clone());
                    globalThis.cached = await cache.match('/key');
                    await navigator.storageBuckets.delete('opaque-response');
                    for (const entry of [original, cloned, cached]) {{
                      if (!(entry instanceof Response) || entry.type !== '{response_type}' ||
                          entry.status !== 0 || entry.statusText !== '' || entry.body !== null ||
                          entry.bodyUsed || [...entry.headers].length !== 0) throw new Error('public response surface');
                      let error;
                      try {{ entry.headers.set('x-author', 'changed'); }} catch (value) {{ error = value; }}
                      if (!(error instanceof TypeError)) throw new Error('immutable public headers');
                    }}
                    opaqueResult = 'ok';
                  }}).catch(error => opaqueResult = String(error.stack || error));
            "#)).unwrap();
            let requests = vm.take_pending_subresource_fetch_infos();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            let headers = vec![
                (
                    "Content-Type".to_owned(),
                    "application/octet-stream".to_owned(),
                ),
                ("Content-Length".to_owned(), "0".to_owned()),
                ("Access-Control-Allow-Origin".to_owned(), "*".to_owned()),
                (
                    "Cross-Origin-Resource-Policy".to_owned(),
                    "cross-origin".to_owned(),
                ),
                ("Vary".to_owned(), "*".to_owned()),
                ("Set-Cookie".to_owned(), "hidden=secret".to_owned()),
            ];
            let head = moli_fetch::ResponseHead {
                final_url: request.url.clone(),
                status,
                status_text: Some("Internal Status".to_owned()),
                headers: headers.clone(),
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                negotiated_http_version: None,
            };
            if streaming {
                let body_source_id = crate::network_host::new_network_body_source_id();
                vm.start_streaming_async_subresource_fetch(
                    crate::types::AsyncSubresourceStreamingStarted {
                        response_filter: None,
                        skip_fetch_security_validation: false,
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id,
                        head,
                        network_request_headers: None,
                    },
                )
                .unwrap();
                vm.finish_streaming_async_subresource_fetch(
                    request.internal_id,
                    body_source_id,
                    Ok(()),
                )
                .unwrap();
            } else {
                vm.complete_async_subresource_fetch(
                    crate::types::AsyncSubresourceFetchCompletion {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        response_status_text: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        network_error_text: None,
                        result: Ok(
                            crate::protocol_types::NavigationResponse::from_head_and_body(
                                head,
                                String::new(),
                                Vec::new(),
                            ),
                        ),
                    },
                )
                .unwrap();
            }
            vm.exec("0", None).unwrap();
            assert_eq!(
                vm.eval("opaqueResult").unwrap(),
                "ok",
                "{response_type}/streaming={streaming}"
            );
            let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context;
            vm.renderer_document_isolate
                .with_entered_renderer_document_isolate(move |isolate| {
                    let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    for name in ["original", "cloned", "cached"] {
                        let value = context
                            .global(scope)
                            .get(scope, v8str(scope, name).into())
                            .unwrap();
                        let (head, _) =
                            crate::network_host::materialize_response_object_internal_head(
                                scope, value, "test",
                            )
                            .unwrap();
                        assert_eq!(head.response_type, response_type, "{name}");
                        assert_eq!(head.status, status, "{name}");
                        assert_eq!(head.status_text, "Internal Status", "{name}");
                        for (name, value) in &headers {
                            assert!(
                                head.headers
                                    .iter()
                                    .any(|(key, entry)| key.eq_ignore_ascii_case(name)
                                        && entry == value),
                                "missing internal {name}: {:?}",
                                head.headers
                            );
                        }
                    }
                    Ok(())
                })
                .unwrap();
        }
    }
}
