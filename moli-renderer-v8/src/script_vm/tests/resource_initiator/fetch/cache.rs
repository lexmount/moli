use super::*;

#[tokio::test]
async fn cached_fetch_keeps_timing_authorization_for_ready_and_pending_bodies() {
    let document = Url::parse("https://cache-timing.test/page").unwrap();
    let url = Url::parse("https://cross.test/asset").unwrap();
    let start = Url::parse("https://cache-timing.test/redirect").unwrap();
    for (mode, streaming) in [("cors", false), ("cors", true), ("no-cors", false)] {
        for timing_allowed in [false, true] {
            let loader = static_http_loader(std::iter::empty::<String>());
            let mut vm =
                new_storage_page_task_executor_test_vm_with_loader(document.as_str(), &loader);
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Fetch),
            );
            vm.exec(
                "caches.open('timing').then(cache=>globalThis.cache=cache)",
                None,
            )
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(!!globalThis.cache)",
                "true",
                "cache-timing-open",
            )
            .await;
            vm.exec(
                r#"
                Object.defineProperty(Array.prototype, '1', {
                    configurable: true,
                    set(value) {
                        if (typeof value === 'string' && value.startsWith('https://cross.test/')) {
                            globalThis.leakedResponseUrl = value;
                        }
                        Object.defineProperty(this, '1', {
                            value, writable: true, configurable: true, enumerable: true
                        });
                    }
                });
            "#,
                None,
            )
            .unwrap();
            vm.exec(
                &format!(
                    r#"
                globalThis.putStarted=false; globalThis.putDone=false;
                fetch('{start}',{{mode:'{mode}'}}).then(response=>{{
                    cache.put('/entry',response.clone()).then(async()=>{{
                        globalThis.cached=await cache.match('/entry');putDone=true;
                    }});
                    putStarted=true;
                }});
            "#
                ),
                None,
            )
            .unwrap();
            let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
            let mut headers = vec![
                (
                    "Content-Type".to_owned(),
                    if mode == "cors" {
                        b"text/plain".to_vec()
                    } else {
                        b"application/javascript".to_vec()
                    },
                ),
                ("Access-Control-Allow-Origin".to_owned(), b"*".to_vec()),
            ];
            if timing_allowed {
                headers.push(("Timing-Allow-Origin".to_owned(), b"*".to_vec()));
            }
            let mut response = crate::protocol_types::NavigationResponse::from_text_body(
                url.clone(),
                200,
                headers,
                "body".to_owned(),
            );
            response.redirected = true;
            response.redirect_chain.push(
                moli_fetch::RedirectInfo {
                    source: moli_fetch::RedirectSource::Network,
                    from_url: start.clone(),
                    to_url: url.clone(),
                    status: 302,
                    headers: Vec::new(),
                    network_extra_info_available: false,
                    request_extra_info: None,
                    dispatched_request: None,
                    response_extra_info: None,
                    redirect_has_extra_info: false,
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                }
                .into(),
            );
            if streaming {
                let body_source_id = crate::network_host::new_network_body_source_id();
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
                vm.exec("0", None).unwrap();
                assert_eq!(vm.eval("String(putStarted)").unwrap(), "true");
                assert_eq!(
                    vm.eval("String(putDone)").unwrap(),
                    "false",
                    "Cache.put must await the unfinished body"
                );
                vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"body".to_vec());
                vm.finish_streaming_async_subresource_fetch(
                    request.internal_id,
                    body_source_id,
                    Ok(()),
                )
                .unwrap();
            } else {
                finish_fetch(&mut vm, &request, response, false);
            }
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(putDone)",
                "true",
                "cache-timing-put",
            )
            .await;
            assert_eq!(
                vm.eval("String(cached.headers.has('timing-allow-origin'))")
                    .unwrap(),
                "false"
            );
            if mode == "no-cors" {
                assert_eq!(
                    vm.eval("String(globalThis.leakedResponseUrl)").unwrap(),
                    "undefined",
                    "private opaque URLs must not reach Array.prototype setters"
                );
                assert_eq!(vm.eval("cached.url").unwrap(), "");
            }
            let context = vm.current_main_world().unwrap().context.clone();
            vm.renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let handles = std::pin::pin!(v8::HandleScope::new(isolate));
                    let scope = &mut handles.init();
                    let context = v8::Local::new(scope, &context);
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let key = v8::String::new(scope, "cached").unwrap();
                    let cached = context.global(scope).get(scope, key.into()).unwrap();
                    let (head, _) = crate::network_host::materialize_response_object_head(
                        scope, cached, "test",
                    )
                    .unwrap();
                    assert_eq!(
                        head.metadata.response_type.as_str(),
                        if mode == "cors" { "cors" } else { "opaque" }
                    );
                    assert_eq!(head.metadata.url_list, [start.clone(), url.clone()]);
                    assert!(head.metadata.redirected);
                    assert_eq!(head.metadata.final_url(), Some(&url));
                    assert_eq!(
                        head.metadata.timing_allow_passed, timing_allowed,
                        "mode={mode}, streaming={streaming}"
                    );
                    Ok(())
                })
                .unwrap();
        }
    }
}
