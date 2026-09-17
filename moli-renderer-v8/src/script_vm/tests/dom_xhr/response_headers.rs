use super::*;

#[test]
fn xhr_response_headers_are_filtered_combined_and_sorted_before_events() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/xhr-response-headers.json"
    ))
    .unwrap();
    let headers: Vec<(String, String)> =
        serde_json::from_value(fixture["headers"].clone()).unwrap();
    for streaming in [false, true] {
        let mut vm = new_storage_test_vm("https://xhr-response-headers.test/");
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Xhr),
        );
        vm.eval(&format!(
            "{}\nglobalThis.headerResult = null; xhrResponseHeadersProbe('/response', {}).then(result => {{ headerResult = result; }}, error => {{ headerResult = {{error:String(error.stack || error)}}; }});",
            include_str!("../../../../tests/fixtures/xhr-response-headers.js"),
            fixture["expected"],
        )).unwrap();
        // Both credential modes must expose the same filtered response.
        for _ in 0..2 {
            let pending = vm.take_pending_subresource_fetch_infos();
            assert_eq!(pending.len(), 1);
            let request = &pending[0];
            let head = moli_fetch::ResponseHead {
                status_text: None,
                final_url: request.url.clone(),
                status: 200,
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
                        internal_id: request.internal_id,
                        request_url: request.url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        skip_fetch_security_validation: false,
                        response_filter: None,
                        body_source_id,
                        head,
                        network_request_headers: None,
                    },
                )
                .unwrap();
                vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"body".to_vec());
                vm.finish_streaming_async_subresource_fetch(
                    request.internal_id,
                    body_source_id,
                    Ok(()),
                )
                .unwrap();
            } else {
                vm.complete_async_subresource_fetch(
                    crate::types::AsyncSubresourceFetchCompletion {
                        response_filter: None,
                        skip_fetch_security_validation: false,
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
                                "body".to_owned(),
                                b"body".to_vec(),
                            ),
                        ),
                    },
                )
                .unwrap();
            }
            vm.exec("0", None).unwrap();
        }
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(headerResult)").unwrap()).unwrap();
        assert_eq!(result["state"], "pass", "streaming={streaming}: {result}");
        assert_eq!(result["checks"].as_array().unwrap().len(), 270);
    }
}
