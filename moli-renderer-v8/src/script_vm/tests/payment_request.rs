use super::*;

#[test]
fn payment_request_constructor_values_handlers_and_receivers_follow_webidl() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-request.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("../../../tests/fixtures/payment-request.js"))
        .unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "266");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval(
            "__uiEventResults.total === new Set(__uiEventResults.checks.map(row=>row.name)).size"
        )
        .unwrap(),
        "true"
    );
}

#[test]
fn payment_request_accepts_registered_native_proxy_and_rejects_author_wrappers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-request.test/");
    vm.eval("globalThis.payment = new PaymentRequest([{supportedMethods:'not-available'}],{id:'native',total:{label:'total',amount:{currency:'USD',value:'1'}}})").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let value = global
            .get(scope, crate::util::v8str(scope, "payment").into())
            .unwrap();
        let target = v8::Local::<v8::Object>::try_from(value).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, target, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        assert_eq!(
            global.create_data_property(
                scope,
                crate::util::v8str(scope, "nativePayment").into(),
                proxy.into()
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    vm.eval(r#"globalThis.nativePaymentResult=false;
    (async()=>{if(nativePayment.id!=='native'||nativePayment.shippingOption!==null||await nativePayment.canMakePayment()!==false)return;
      let calls=0;nativePayment.onpaymentmethodchange=()=>calls++;payment.dispatchEvent(new Event('paymentmethodchange'));if(calls!==1)return;
      let conversions=0;try{await PaymentRequest.prototype.show.call(new Proxy(nativePayment,{}),{get then(){conversions++;return undefined;}});return;}
      catch(error){nativePaymentResult=error instanceof TypeError&&conversions===0;}
    })()"#).unwrap();
    assert_eq!(vm.eval("nativePaymentResult").unwrap(), "true");
}

#[test]
fn payment_request_no_handler_show_closes_state_and_rejects_new_promises() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-request.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let host = crate::util::context_host_ptr_from_global_bridge(scope).unwrap();
        unsafe { &mut *host }.begin_protocol_user_gesture_activation();
        Ok(())
    })
    .unwrap();
    let evaluated=vm.eval(r#"globalThis.paymentShowResult=false;
    (async()=>{const q=new PaymentRequest([{supportedMethods:'not-available'}],{total:{label:'x',amount:{currency:'USD',value:'1'}}});
      const errors=[];for(const call of [()=>q.show(),()=>q.show(),()=>q.canMakePayment(),()=>q.abort()]){try{await call();return;}catch(error){if(!(error instanceof DOMException))return;errors.push(error.name);}}
      paymentShowResult=errors.join()==='NotSupportedError,InvalidStateError,InvalidStateError,InvalidStateError';
    })()"#);
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let host = crate::util::context_host_ptr_from_global_bridge(scope).unwrap();
        unsafe { &mut *host }.end_protocol_user_gesture_activation();
        Ok(())
    })
    .unwrap();
    evaluated.unwrap();
    assert_eq!(vm.eval("paymentShowResult").unwrap(), "true");
}

#[test]
fn payment_request_retained_realms_keep_values_but_lose_active_document_authority() {
    let mut vm = new_storage_page_task_executor_test_vm("https://payment-request.test/");
    vm.eval(r#"globalThis.paymentRetiredResult=false;(async()=>{
      const f=document.createElement('iframe');document.body.appendChild(f);const w=f.contentWindow,C=w.PaymentRequest;
      const methods=[{supportedMethods:'not-available'}],details={id:'retained',total:{label:'x',amount:{currency:'USD',value:'1'}}};
      const q=new C(methods,details);f.remove();if(q.id!=='retained'||q.shippingAddress!==null)return;
      try{new C(methods,details);return;}catch(error){if(!(error instanceof w.DOMException)||error.name!=='SecurityError')return;}
      try{await q.canMakePayment();return;}catch(error){paymentRetiredResult=error instanceof w.DOMException&&error.name==='InvalidStateError';}
    })()"#).unwrap();
    assert_eq!(vm.eval("paymentRetiredResult").unwrap(), "true");
    let mut insecure = new_storage_page_task_executor_test_vm("http://payment-request.test/");
    assert_eq!(
        insecure.eval("'PaymentRequest' in globalThis").unwrap(),
        "false"
    );
}
