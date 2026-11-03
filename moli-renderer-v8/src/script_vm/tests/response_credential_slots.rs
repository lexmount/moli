use super::*;

#[test]
fn native_response_credentials_survive_filters_clones_and_public_property_spoofing() {
    let mut vm = new_storage_test_vm("https://response-credential.test/");
    let context = &vm.page_default_runtime.context as *const _;
    let mut expected = Vec::new();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let origin = Url::parse("https://response-credential.test/").unwrap();
        let mut values = Vec::new();
        for (url, status, mode) in [
            (
                "https://response-credential.test/",
                200,
                moli_fetch::RequestMode::Cors,
            ),
            ("https://foreign.test/", 200, moli_fetch::RequestMode::Cors),
            (
                "https://foreign.test/",
                200,
                moli_fetch::RequestMode::NoCors,
            ),
            ("https://foreign.test/", 302, moli_fetch::RequestMode::Cors),
        ] {
            for flag in [false, true] {
                let head = moli_fetch::ResponseHead {
                    final_url: Url::parse(url).unwrap(),
                    status,
                    status_text: None,
                    headers: Vec::new(),
                    request_cookie_report: None,
                    request_include_credentials: flag,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    cache_state: Default::default(),
                    negotiated_http_version: None,
                };
                let response = crate::network_host::build_fetch_response_object_for_request_mode(
                    scope,
                    moli_url::WebOrigin::from_url(&origin),
                    crate::network_host::FetchResponseRequest {
                        method: "GET",
                        mode,
                    },
                    moli_fetch::Response::from_head_and_body(
                        head,
                        "body".to_owned(),
                        b"body".to_vec(),
                    ),
                );
                values.push(response.into());
                expected.extend([flag, flag]);
            }
        }
        let samples = v8::Array::new_with_elements(scope, &values);
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "samples");
        assert_eq!(
            global.create_data_property(scope, key.into(), samples.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(r#"JSON.stringify(samples.map(x=>x.type))"#)
            .unwrap(),
        r#"["basic","basic","cors","cors","opaque","opaque","opaqueredirect","opaqueredirect"]"#
    );
    vm.eval(r#"samples = samples.flatMap(response => {
        if (Object.hasOwn(response,'__lmResponseRequestIncludeCredentials')) throw Error('native flag exposed');
        response.__lmResponseRequestIncludeCredentials = 'author';
        const clone = response.clone();
        clone.__lmResponseRequestIncludeCredentials = 'author clone';
        return [response,clone];
    })"#).unwrap();
    let mut actual = Vec::new();
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "samples");
        let samples =
            v8::Local::<v8::Array>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        for i in 0..samples.length() {
            let value = samples.get_index(scope, i).unwrap();
            let response = v8::Local::<v8::Object>::try_from(value).unwrap();
            actual.push(crate::network_host::response_slot_bool(
                scope,
                response,
                crate::network_host::RESPONSE_REQUEST_INCLUDE_CREDENTIALS_SLOT,
            ));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
}
