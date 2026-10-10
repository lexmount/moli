use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_certificate_values_normalization_brands_and_clone_use_native_state() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for url in [
        "https://rtc-certificate.test/",
        "http://rtc-certificate.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(url, &loader);
        vm.eval("document.body.innerHTML='<iframe></iframe>'")
            .unwrap();
        vm.eval(include_str!("../../../tests/fixtures/rtc-certificate.js"))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(globalThis.__uiEventResults?.complete)",
            "true",
            "certificate offers must finish through selected networking tasks",
        )
        .await;
        assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{url}"
        );
        assert_eq!(
            vm.eval(
                "__uiEventResults.total===new Set(__uiEventResults.checks.map(row=>row.name)).size"
            )
            .unwrap(),
            "true"
        );
    }
}

#[test]
fn rtc_certificate_registered_native_proxy_keeps_key_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-certificate.test/");
    vm.eval("RTCPeerConnection.generateCertificate({name:'ECDSA',namedCurve:'P-256'}).then(value=>globalThis.cert=value)").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let target = v8::Local::<v8::Object>::try_from(
            global
                .get(scope, crate::util::v8str(scope, "cert").into())
                .unwrap(),
        )
        .unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativeCert").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      if(nativeCert.expires!==cert.expires||JSON.stringify(nativeCert.getFingerprints())!==JSON.stringify(cert.getFingerprints()))return false;
      const pc=new RTCPeerConnection({certificates:[nativeCert]});pc.setConfiguration({certificates:[nativeCert]});pc.close();
      let reads=0;
      try{new RTCPeerConnection({certificates:[new Proxy(nativeCert,{})],get iceServers(){reads++;return [];}});return false;}
      catch(error){return error instanceof TypeError&&reads===0;}
    })()"#).unwrap(), "true");
}

#[test]
fn rtc_certificate_wire_clones_keep_key_leases_and_immutable_tuple_or_opaque_origins() {
    for (source_url, targets) in [
        (
            "https://a.example.test/",
            vec![
                ("https://a.example.test/path", true),
                ("https://b.example.test/", false),
                ("http://a.example.test/", false),
                ("https://a.example.test:8443/", false),
            ],
        ),
        (
            "data:text/html,<body>",
            vec![
                ("data:text/html,<body>", false),
                ("https://a.example.test/", false),
            ],
        ),
    ] {
        let (wire, native) = {
            let mut source = new_storage_page_task_executor_test_vm(source_url);
            source.eval("RTCPeerConnection.generateCertificate({name:'ECDSA',namedCurve:'P-256'}).then(cert=>globalThis.cert=cert)").unwrap();
            if source_url.starts_with("https:") {
                source.eval("document.domain='example.test'").unwrap();
            }
            let context_ptr = &source.page_default_runtime.context as *const _;
            source
                .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
                    let global = scope.get_current_context().global(scope);
                    let object = v8::Local::<v8::Object>::try_from(
                        global
                            .get(scope, crate::util::v8str(scope, "cert").into())
                            .unwrap(),
                    )
                    .unwrap();
                    let native = crate::context_bootstrap::rtc_certificate::payload_from_object(
                        scope, object,
                    )
                    .unwrap();
                    let wire = crate::structured_clone::serialize_for_wire_for_runtime(
                        scope,
                        object.into(),
                    )
                    .unwrap();
                    Ok((wire, native))
                })
                .unwrap()
        }; // The source isolate is gone; only native key leases survive.
        for (target_url, allowed) in targets {
            let mut target = new_storage_page_task_executor_test_vm(target_url);
            if target_url.starts_with("https:") && target_url.ends_with(".test/") {
                target.eval("document.domain='example.test'").unwrap();
            }
            let context_ptr = &target.page_default_runtime.context as *const _;
            target
                .with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
                    let object = v8::Local::<v8::Object>::try_from(
                        crate::structured_clone::deserialize_from_wire(scope, &wire).unwrap(),
                    )
                    .unwrap();
                    let copy = crate::context_bootstrap::rtc_certificate::payload_from_object(
                        scope, object,
                    )
                    .unwrap();
                    assert!(copy.certificate.same_certificate(&native.certificate));
                    let global = scope.get_current_context().global(scope);
                    assert_eq!(
                        global.create_data_property(
                            scope,
                            crate::util::v8str(scope, "remoteCert").into(),
                            object.into()
                        ),
                        Some(true)
                    );
                    Ok(())
                })
                .unwrap();
            let result=target.eval(r#"(() => {try{const pc=new RTCPeerConnection({certificates:[remoteCert]});pc.close();return 'accepted';}catch(error){return error instanceof DOMException?error.name:String(error);}})()"#).unwrap();
            assert_eq!(
                result,
                if allowed {
                    "accepted"
                } else {
                    "InvalidAccessError"
                },
                "{source_url} -> {target_url}"
            );
        }
        for realm in [
            crate::context_bootstrap::exposed_interfaces::RealmKind::DedicatedWorker,
            crate::context_bootstrap::exposed_interfaces::RealmKind::SharedWorker,
            crate::context_bootstrap::exposed_interfaces::RealmKind::ServiceWorker,
        ] {
            let mut isolate = v8::Isolate::new(Default::default());
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            crate::context_bootstrap::install_worker_lazy_exposed_interfaces(
                scope, global, realm, true,
            )
            .unwrap();
            let catch = std::pin::pin!(v8::TryCatch::new(scope));
            let scope = &mut catch.init();
            assert!(
                crate::structured_clone::deserialize_from_wire(scope, &wire).is_none(),
                "RTCCertificate is Window-only"
            );
            assert!(scope.has_caught());
        }
    }
}

#[test]
fn rtc_certificate_history_keeps_native_attachments_after_original_wrapper_gc() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rtc-certificate.test/");
    vm.eval(r#"RTCPeerConnection.generateCertificate({name:'ECDSA',namedCurve:'P-256'}).then(cert=>{globalThis.expected=cert.getFingerprints()[0].value;history.replaceState({cert},'');});"#).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        scope.low_memory_notification();
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {const cert=history.state.cert;if(!(cert instanceof RTCCertificate)||cert.getFingerprints()[0].value!==expected)return false;const pc=new RTCPeerConnection({certificates:[cert]});pc.close();return true;})()"#).unwrap(),"true");
}
