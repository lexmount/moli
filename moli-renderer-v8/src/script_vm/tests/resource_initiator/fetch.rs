use super::*;

mod cancellation;
mod orb;

fn finish_fetch(
    vm: &mut ScriptVm,
    request: &crate::types::PendingSubresourceFetchInfo,
    response: crate::protocol_types::NavigationResponse,
    streaming: bool,
) {
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
        assert_eq!(
            vm.eval("String(performance.getEntriesByType('resource').length)")
                .unwrap(),
            "0",
            "headers must not publish an unfinished Fetch"
        );
        vm.append_streaming_async_subresource_fetch_chunk(
            body_source_id,
            response.body_bytes().to_vec(),
        );
        assert_eq!(
            vm.eval("String(performance.getEntriesByType('resource').length)")
                .unwrap(),
            "0",
            "body chunks must not publish an unfinished Fetch"
        );
        vm.finish_streaming_async_subresource_fetch(request.internal_id, body_source_id, Ok(()))
            .unwrap();
    } else {
        vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
            internal_id: request.internal_id,
            request_url: request.url.clone(),
            request_method: request.method.clone(),
            request_headers: request.request_headers.clone(),
            request_body: None,
            response_status_text: None,
            skip_fetch_security_validation: false,
            response_filter: None,
            network_error_text: None,
            result: Ok(response).into(),
        })
        .unwrap();
    }
}

#[test]
fn fetch_resource_timing_publishes_once_after_buffered_and_streaming_body_completion() {
    let document_url = Url::parse("https://fetch-timing.test/page").unwrap();
    let caller = document_url.join("caller.js").unwrap();
    for streaming in [false, true] {
        let mut vm = new_storage_test_vm(document_url.as_str());
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Fetch),
        );
        execute_with_source(
            &mut vm,
            r#"
          globalThis.observed = [];
          globalThis.observer = new PerformanceObserver(list => observed.push(...list.getEntries()));
          observer.observe({type:'resource'});
          globalThis.fetchResponse = null;
          globalThis.started = performance.now();
          fetch('/asset').then(response => fetchResponse = response);
        "#,
            &caller,
            &document_url,
        );
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        finish_fetch(
            &mut vm,
            request,
            crate::protocol_types::NavigationResponse::from_text_body(
                request.url.clone(),
                200,
                vec![("Content-Type".to_owned(), b"text/plain".to_vec())],
                "body".to_owned(),
            ),
            streaming,
        );
        assert_eq!(vm.eval("String(fetchResponse.status)").unwrap(), "200");
        assert_eq!(vm.eval(r#"JSON.stringify({
          entries:performance.getEntriesByType('resource').map(entry => ({
            name:entry.name, type:entry.initiatorType, source:entry.initiatorUrl,
            jsonSource:entry.toJSON().initiatorUrl, status:entry.responseStatus,
            encoded:entry.encodedBodySize, decoded:entry.decodedBodySize,
            transfer:entry.transferSize, native:entry instanceof PerformanceResourceTiming,
            start:entry.startTime >= started && entry.startTime <= performance.now()
          })),
          observer:[...observed, ...observer.takeRecords()].map(entry => entry.name)
        })"#).unwrap(), serde_json::json!({
            "entries":[{"name":request.url.as_str(),"type":"fetch","source":caller.as_str(),
                "jsonSource":caller.as_str(),"status":200,"encoded":4,"decoded":4,"transfer":304,"native":true,"start":true}],
            "observer":[request.url.as_str()]
        }).to_string(), "streaming={streaming}");
    }
}

#[tokio::test]
async fn fetch_resource_timing_retains_initiators_through_real_network_and_lazy_performance() {
    let server = StaticHttpServer::spawn_with_responder(
        3,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
      globalThis.fetchDone = 0;
      globalThis.get = path => fetch(path).then(response => response.text()).then(() => fetchDone++);
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let caller = base.join("caller.js?version=2").unwrap();
    execute_with_source(
        &mut vm,
        r#"
      get('/direct');
      Promise.resolve().then(() => get('/promise'));
      setTimeout(() => get('/timer'), 0);
    "#,
        &caller,
        &base,
    );
    execute_with_source(
        &mut vm,
        "globalThis.laterScript = true;",
        &base.join("later.js").unwrap(),
        &base,
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(fetchDone)",
        "3",
        "real Fetch body completion",
    )
    .await;
    assert_eq!(vm.eval(r#"JSON.stringify(performance.getEntriesByType('resource')
      .map(entry => [new URL(entry.name).pathname, entry.initiatorType, entry.initiatorUrl, entry.toJSON().initiatorUrl]).sort())"#).unwrap(),
      serde_json::json!([
          ["/direct","fetch",caller.as_str(),caller.as_str()],
          ["/promise","fetch",caller.as_str(),caller.as_str()],
          ["/timer","fetch",caller.as_str(),caller.as_str()]
      ]).to_string());
    assert_eq!(server.finish().await.len(), 3);
}

#[test]
fn fetch_resource_timing_reports_failed_network_requests() {
    let base = Url::parse("https://failed-fetch-timing.test/page").unwrap();
    let mut vm = new_storage_test_vm(base.as_str());
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    execute_with_source(
        &mut vm,
        "fetch('/failed').catch(error => globalThis.fetchError = error.name);",
        &base,
        &base,
    );
    let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
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
        result: Err("network failed".to_owned()).into(),
    })
    .unwrap();
    assert_eq!(vm.eval("fetchError").unwrap(), "TypeError");
    assert_eq!(
        vm.eval(
            r#"JSON.stringify(performance.getEntriesByType('resource')
      .map(entry => [entry.name,entry.initiatorType,entry.responseStatus,entry.transferSize]))"#
        )
        .unwrap(),
        serde_json::json!([[request.url.as_str(), "fetch", 0, 0]]).to_string()
    );
}

#[test]
fn fetch_resource_timing_separates_cors_attribution_from_timing_allow_origin() {
    let document = Url::parse("https://fetch-timing.test/page").unwrap();
    let caller = document.join("caller.js").unwrap();
    let streaming_modes: &[bool] = &[false, true];
    // The production transport buffers no-cors responses for ORB body sniffing.
    let buffered: &[bool] = &[false];
    for (modes, mode, tao, expected_source, expected_size, expected_status) in [
        (streaming_modes, "cors", None, caller.as_str(), 0, 200),
        (streaming_modes, "cors", Some("*"), caller.as_str(), 4, 200),
        (
            streaming_modes,
            "cors",
            Some("https://fetch-timing.test"),
            caller.as_str(),
            4,
            200,
        ),
        (
            streaming_modes,
            "cors",
            Some("https://other.test"),
            caller.as_str(),
            0,
            200,
        ),
        (buffered, "no-cors", None, "", 0, 0),
        (buffered, "no-cors", Some("*"), "", 4, 0),
    ] {
        for &streaming in modes {
            let mut vm = new_storage_test_vm(document.as_str());
            vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Fetch),
            );
            execute_with_source(
                &mut vm,
                &format!(
                    "fetch('https://cross-origin.test/asset', {{mode:'{mode}'}}).then(response => globalThis.fetchResponse = response);"
                ),
                &caller,
                &document,
            );
            let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
            let mut headers = vec![
                (
                    "Content-Type".to_owned(),
                    // Keep the opaque control ORB-safelisted: blocked responses
                    // have separate coverage and must suppress even TAO sizes.
                    if mode == "no-cors" {
                        b"application/javascript".to_vec()
                    } else {
                        b"text/plain".to_vec()
                    },
                ),
                ("Access-Control-Allow-Origin".to_owned(), b"*".to_vec()),
            ];
            if let Some(tao) = tao {
                headers.push(("Timing-Allow-Origin".to_owned(), tao.as_bytes().to_vec()));
            }
            finish_fetch(
                &mut vm,
                &request,
                crate::protocol_types::NavigationResponse::from_text_body(
                    request.url.clone(),
                    200,
                    headers,
                    "body".to_owned(),
                ),
                streaming,
            );
            assert_eq!(
                vm.eval(
                    r#"JSON.stringify(performance.getEntriesByType('resource').map(entry => [
                entry.name, entry.initiatorType, entry.initiatorUrl, entry.responseStatus,
                entry.encodedBodySize, entry.decodedBodySize, entry.transferSize, entry.contentType
            ]))"#
                )
                .unwrap(),
                serde_json::json!([[
                    request.url.as_str(),
                    "fetch",
                    expected_source,
                    expected_status,
                    expected_size,
                    expected_size,
                    if expected_size == 0 { 0 } else { 304 },
                    if mode == "cors" { "text/plain" } else { "" }
                ]])
                .to_string(),
                "streaming={streaming}, mode={mode}, tao={tao:?}"
            );
        }
    }
}

#[test]
fn fetch_resource_timing_keeps_cache_hits_and_unknown_script_sources_unattributed() {
    let document = Url::parse("https://fetch-cache-timing.test/page").unwrap();
    let caller = document.join("caller.js").unwrap();
    for streaming in [false, true] {
        for (cache, transfer) in [
            (moli_fetch::ResponseCacheState::None, 304),
            (moli_fetch::ResponseCacheState::Local, 0),
            (moli_fetch::ResponseCacheState::Validated, 300),
        ] {
            for known_source in [false, true] {
                let mut vm = new_storage_test_vm(document.as_str());
                vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Fetch),
                );
                let script =
                    "fetch('/asset').then(response => globalThis.fetchResponse = response);";
                if known_source {
                    execute_with_source(&mut vm, script, &caller, &document);
                } else {
                    vm.exec(script, None).unwrap();
                }
                let request = vm.take_pending_subresource_fetch_infos().pop().unwrap();
                let mut response = crate::protocol_types::NavigationResponse::from_text_body(
                    request.url.clone(),
                    200,
                    Vec::new(),
                    "body".to_owned(),
                );
                response.cache_state = cache;
                response.from_cache = cache != moli_fetch::ResponseCacheState::None;
                finish_fetch(&mut vm, &request, response, streaming);
                let source = if known_source && cache == moli_fetch::ResponseCacheState::None {
                    caller.as_str()
                } else {
                    ""
                };
                assert_eq!(
                    vm.eval(
                        r#"JSON.stringify(performance.getEntriesByType('resource').map(entry => [
                    entry.initiatorUrl, entry.encodedBodySize, entry.transferSize
                ]))"#
                    )
                    .unwrap(),
                    serde_json::json!([[source, 4, transfer]]).to_string(),
                    "streaming={streaming}, cache={cache:?}, known_source={known_source}"
                );
            }
        }
    }
}
