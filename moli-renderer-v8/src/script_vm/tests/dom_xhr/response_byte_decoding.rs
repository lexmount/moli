use super::*;
use crate::util::v8str;

#[test]
fn xhr_response_decoding_uses_headers_received_overrides_for_buffered_and_streamed_bytes() {
    for streaming in [false, true] {
        for (chunks, partial, expected) in [
            (
                vec![vec![0x83], vec![0x65, 0x83], vec![0x58, 0x83, 0x67]],
                vec!["", "テ", "テスト"],
                "テスト",
            ),
            (
                vec![vec![0xff], vec![0xfe, b'A'], vec![0, 0xe9], vec![0]],
                vec!["", "", "A", "Aé"],
                "Aé",
            ),
        ] {
            let mut vm = new_storage_test_vm("https://xhr-response-decoding.test/");
            vm.set_fetch_subresource_interception(
                crate::types::SubresourceInterceptionClasses::for_resource_type(
                    crate::types::SubresourceResourceType::Xhr,
                ),
            );
            vm.eval(
                r#"
                globalThis.__decodingXhr = new XMLHttpRequest();
                globalThis.__decodingHeadersText = null;
                __decodingXhr.open('GET', '/response');
                __decodingXhr.onreadystatechange = () => {
                    if (__decodingXhr.readyState === 2) {
                        __decodingHeadersText = __decodingXhr.responseText;
                        __decodingXhr.overrideMimeType('text/plain;charset=Shift_JIS');
                    }
                };
                __decodingXhr.send();
            "#,
            )
            .unwrap();
            let requests = vm.take_pending_subresource_fetch_infos();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            let bytes = chunks.concat();
            let head = moli_fetch::ResponseHead {
                final_url: request.url.clone(),
                status: 200,
                status_text: None,
                headers: vec![
                    (
                        "Content-Type".to_owned(),
                        b"text/plain;charset=UTF-8".to_vec(),
                    ),
                    (
                        "Content-Length".to_owned(),
                        bytes.len().to_string().into_bytes(),
                    ),
                ],
                request_cookie_report: None,

                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: false,
                cache_state: Default::default(),

                negotiated_http_version: None,
            };
            if streaming {
                let body_source_id = crate::network_host::new_network_body_source_id();
                vm.start_streaming_async_subresource_fetch(
                    crate::types::AsyncSubresourceStreamingStarted {
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Default::default(),
                        request_body: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        body_source_id,
                        head,
                        network_request_headers: None,
                    },
                )
                .unwrap();
                for (chunk, text) in chunks.into_iter().zip(partial) {
                    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, chunk);
                    assert_eq!(vm.eval("__decodingXhr.responseText").unwrap(), text);
                }
                vm.finish_streaming_async_subresource_fetch(
                    request.internal_id,
                    body_source_id,
                    Ok(()),
                )
                .unwrap();
            } else {
                let context_ptr = &vm.current_main_world().unwrap().context as *const _;
                vm.with_context_scope_by_ptr_and_checkpoint_for_test(
                    context_ptr,
                    move |scope, _| {
                        let context = scope.get_current_context();
                        let xhr = context
                            .global(scope)
                            .get(scope, v8str(scope, "__decodingXhr").into())
                            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                            .unwrap();
                        crate::network_host::apply_xhr_response_body_source(
                            scope,
                            xhr,
                            head,
                            moli_fetch::ResponseBody::materialized_bytes(bytes),
                        );
                        Ok(())
                    },
                )
                .unwrap();
            }
            assert_eq!(vm.eval("__decodingHeadersText").unwrap(), "");
            assert_eq!(vm.eval("__decodingXhr.responseText").unwrap(), expected);
            assert_eq!(vm.eval("__decodingXhr.response === __decodingXhr.responseText && __decodingXhr.readyState === 4").unwrap(), "true");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn xhr_queued_response_retains_legacy_bytes_and_decodes_json_as_utf8() {
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://xhr-queued-decoding.test/",
        &loader,
    );
    vm.eval(r#"
        globalThis.__queuedDecoding = [];
        for (const [url, type, override] of [
            ['data:text/plain;charset=windows-1252,%FF', '', null],
            ['data:text/plain;charset=utf-8,%83%65%83%58%83%67', '', 'text/plain;charset=Shift_JIS'],
            ['data:application/json;charset=windows-1252,%EF%BB%BF%7B%22x%22%3A1%7D', 'json', null],
            ['data:text/html,%3Cmeta%20charset=windows-1252%3E%3Cx%3E%FF%3C%2Fx%3E', 'document', null],
        ]) {
            const xhr = new XMLHttpRequest();
            xhr.open('GET', url);
            xhr.responseType = type;
            xhr.onreadystatechange = () => {
                if (xhr.readyState === 2 && override) xhr.overrideMimeType(override);
            };
            xhr.onload = () => __queuedDecoding.push(type === 'json' ? xhr.response.x :
                type === 'document' ? [xhr.response.querySelector('x').textContent, xhr.response.characterSet] : xhr.responseText);
            xhr.send();
        }
    "#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__queuedDecoding.length)",
        "4",
        "queued XHR byte decoding",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__queuedDecoding)").unwrap(),
        r#"["ÿ","テスト",1,["ÿ","windows-1252"]]"#
    );
}
