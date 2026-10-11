use super::*;

fn check_native_handler_receivers(kind: &str, expected_checks: &str) {
    for url in [
        "http://handler-receivers.test/",
        "https://handler-receivers.test/",
        "data:text/html,<body>",
    ] {
        let mut vm = new_parsed_test_vm(url, "<!doctype html><body><iframe></iframe>");
        vm.eval(&format!(
            "globalThis.__deferHandlerChecks=true;globalThis.__handlerKind='{kind}'"
        ))
        .unwrap();
        vm.eval(include_str!(
            "../../../tests/fixtures/document-shadow-handler-receivers.js"
        ))
        .unwrap();
        vm.eval("globalThis.nativeProxyTraps=0;globalThis.nativeHandlerTrap=function(){nativeProxyTraps++;throw Error('native Proxy trap')}")
            .unwrap();
        let context_ptr = &vm.page_default_runtime.context as *const _;
        vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let rows = v8::Local::<v8::Array>::try_from(
                global
                    .get(
                        scope,
                        crate::util::v8str(scope, "__handlerTargetRows").into(),
                    )
                    .unwrap(),
            )
            .unwrap();
            let receivers = v8::Array::new(scope, rows.length() as i32);
            let trap = global
                .get(scope, crate::util::v8str(scope, "nativeHandlerTrap").into())
                .unwrap();
            for index in 0..rows.length() {
                let row = rows
                    .get_index(scope, index)
                    .unwrap()
                    .to_object(scope)
                    .unwrap();
                let target = row
                    .get(scope, crate::util::v8str(scope, "target").into())
                    .unwrap()
                    .to_object(scope)
                    .unwrap();
                let target = moli_webapi_declare::web_api_object_target(scope, target).unwrap();
                let handler = crate::util::new_null_prototype_object(scope);
                for name in ["get", "getPrototypeOf", "set"] {
                    assert_eq!(
                        handler.create_data_property(
                            scope,
                            crate::util::v8str(scope, name).into(),
                            trap
                        ),
                        Some(true)
                    );
                }
                let proxy = v8::Proxy::new(scope, target, handler).unwrap();
                moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
                assert!(crate::web_api_interfaces::Node::is_instance(
                    scope,
                    proxy.into()
                ));
                assert_eq!(receivers.set_index(scope, index, proxy.into()), Some(true));
            }
            assert_eq!(
                global.create_data_property(
                    scope,
                    crate::util::v8str(scope, "nativeHandlerReceivers").into(),
                    receivers.into()
                ),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
        vm.eval("runHandlerReceiverChecks(nativeHandlerReceivers)")
            .unwrap();
        assert_eq!(
            vm.eval("__uiEventResults.total").unwrap(),
            expected_checks,
            "{kind}/{url}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
                .unwrap(),
            "[]",
            "{kind}/{url}"
        );
        assert_eq!(vm.eval("nativeProxyTraps").unwrap(), "0", "{kind}/{url}");
    }
}

#[test]
fn document_handlers_resolve_native_proxy_identity_and_preserve_lenient_receivers() {
    check_native_handler_receivers("document", "60");
}

#[test]
fn shadow_root_handlers_resolve_native_proxy_identity_and_callee_error_realms() {
    check_native_handler_receivers("shadow", "30");
}
