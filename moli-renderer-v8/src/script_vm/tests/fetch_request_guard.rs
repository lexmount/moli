use super::*;

#[test]
fn fetch_request_initializers_use_request_header_guards() {
    let base = "https://fetch-guard.test";
    let mut vm = new_storage_test_vm(&format!("{base}/page.html"));
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Fetch));
    vm.eval(&format!(
        "{}\nglobalThis.guardResult = null; fetchRequestGuardProbe('{base}', false).then(value => {{ guardResult = value; }}, error => {{ guardResult = {{ error: String(error) }}; }});",
        include_str!("../../../tests/fixtures/fetch-request-guard.js"),
    )).unwrap();
    let mut requests = 0;
    for _ in 0..40 {
        vm.exec("0", None).unwrap();
        if vm.eval("guardResult !== null").unwrap() == "true" {
            break;
        }
        let pending = vm.take_pending_subresource_fetch_infos();
        assert_eq!(pending.len(), 1, "request {requests}");
        let request = &pending[0];
        assert_eq!(request.url.path(), "/echo");
        let body =
            serde_json::json!({"headers": request.request_headers.to_byte_strings()}).to_string();
        let head = moli_fetch::ResponseHead {
            status_text: None,
            final_url: request.url.clone(),
            status: 200,
            headers: vec![("content-type".to_owned(), b"application/json".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            cache_state: Default::default(),
            preload_state: Default::default(),
            negotiated_http_version: None,
        };
        vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
            response_filter: None,
            skip_fetch_security_validation: false,
            internal_id: request.internal_id,
            request_url: request.url.clone(),
            request_method: request.method.clone(),
            request_headers: request.request_headers.clone(),
            request_body: None,
            response_status_text: None,
            network_error_text: None,
            result: Ok(
                crate::protocol_types::NavigationResponse::from_head_and_body(
                    head,
                    body.clone(),
                    body.into_bytes(),
                ),
            )
            .into(),
        })
        .unwrap();
        requests += 1;
    }
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("JSON.stringify(guardResult)").unwrap()).unwrap();
    assert_eq!(requests, 33);
    assert_eq!(result["state"], "pass", "{result}");
    assert_eq!(result["checks"].as_array().unwrap().len(), 1046);
}
