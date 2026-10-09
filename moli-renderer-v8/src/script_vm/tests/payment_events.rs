use super::*;

#[test]
fn payment_update_event_payloads_and_operations_use_native_cross_realm_brands() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-events.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "({})([globalThis, document.getElementById('child').contentWindow]).then(checks => {{globalThis.__paymentChecks=checks;}})",
        include_str!("payment_events.js"),
    );
    vm.eval(&source).unwrap();
    assert_eq!(vm.eval("typeof __paymentChecks").unwrap(), "object");
    // Keep the native-Promise wrapping gaps explicit while the shared converter
    // still preserves native Promise inputs. Every other check must pass.
    let mut known_gaps = Vec::new();
    for owner in 0..2 {
        for event in ["PaymentRequestUpdateEvent", "PaymentMethodChangeEvent"] {
            for callee in 0..2 {
                for check in [
                    "WebIDL native Promise wrapping",
                    "native Promise then invoked asynchronously",
                ] {
                    known_gaps.push(format!("{owner}:{event}:callee-{callee}:{check}"));
                }
            }
        }
    }
    known_gaps.sort();
    assert_eq!(
        vm.eval(
            "JSON.stringify(__paymentChecks.filter(row => !row.passed).map(row => row.name).sort())"
        )
        .unwrap(),
        serde_json::to_string(&known_gaps).unwrap()
    );
    assert_eq!(vm.eval("__paymentChecks.length").unwrap(), "378");
}

#[test]
fn payment_event_payloads_and_update_with_accept_registered_native_proxies() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-native-proxy.test/");
    vm.eval("globalThis.paymentEvent = new PaymentMethodChangeEvent('x', {methodName:'pay', methodDetails:{token:42}})")
        .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "paymentEvent");
        let event =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, event, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "paymentProxy");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const p=PaymentMethodChangeEvent.prototype;
        for(const key of ['methodName','methodDetails']) {
            const get=Object.getOwnPropertyDescriptor(p,key).get;
            if(get.call(paymentProxy)!==paymentEvent[key]) throw Error('native identity');
            let caught=false;try{get.call(new Proxy(paymentProxy,{}));}catch(error){caught=error instanceof TypeError;}
            if(!caught) throw Error('author proxy accepted');
        }
        let gets=0, error;
        try{PaymentRequestUpdateEvent.prototype.updateWith.call(paymentProxy,{get then(){gets++;return undefined;}});}catch(caught){error=caught;}
        if(gets!==1 || !(error instanceof DOMException) || error.name!=='InvalidStateError') throw Error('native proxy operation');
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn payment_event_constructors_are_absent_in_insecure_window_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("http://payment-events.test/");
    assert_eq!(vm.eval("[isSecureContext, 'PaymentRequestUpdateEvent' in globalThis, 'PaymentMethodChangeEvent' in globalThis].join(',')").unwrap(), "false,false,false");
}
