use super::*;

#[test]
fn hash_change_event_payload_accessors_use_native_state_and_callee_receiver_checks() {
    let mut vm = new_parsed_test_vm(
        "https://hash-change-event-accessors.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    assert_eq!(vm.eval(r#"(() => {
        const realms=[window,document.querySelector('iframe').contentWindow];
        for (const owner of realms) for (const callee of realms) {
            const event=new owner.HashChangeEvent('hashchange',{
                oldURL:'https://example.test/#old',newURL:'https://example.test/#new'
            });
            event.initEvent('initialized',false,false);
            for (const [key,value] of [['oldURL','https://example.test/#old'],['newURL','https://example.test/#new']]) {
                const descriptor=Object.getOwnPropertyDescriptor(callee.HashChangeEvent.prototype,key);
                const getter=descriptor.get;
                if (!descriptor.enumerable || !descriptor.configurable || descriptor.set!==undefined ||
                    getter.length!==0 || getter.name!=='get '+key || Object.hasOwn(event,key) || getter.call(event)!==value) throw Error('payload descriptor');
                Object.defineProperty(event,key,{value:'author value',configurable:true});
                if (getter.call(event)!==value) throw Error('author field replaced native state');
                let traps=0;
                const author=new Proxy(event,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
                const revoked=Proxy.revocable(event,{});revoked.revoke();
                for (const receiver of [{},new owner.Event('event'),Object.create(event),author,revoked.proxy]) {
                    let error;
                    try { getter.call(receiver); } catch(e) { error=e; }
                    if (!error || Object.getPrototypeOf(error)!==callee.TypeError.prototype || traps) throw Error('receiver accepted');
                }
            }
        }
        return true;
    })()"#).unwrap(), "true");
}

#[test]
fn document_create_event_preserves_legacy_interfaces_and_native_dispatch() {
    let mut vm = new_parsed_test_vm(
        "https://document-create-event.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    vm.eval(include_str!("document_create_event.js")).unwrap();
    assert_eq!(
        vm.eval("__documentCreateEventResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("__documentCreateEventResults.total").unwrap(),
        "1648"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__documentCreateEventResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn document_create_event_uses_intrinsics_and_registered_native_receivers() {
    let mut vm =
        new_storage_page_task_executor_test_vm("https://document-create-event-native.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'; globalThis.child=document.querySelector('iframe').contentWindow;")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "document");
        let document =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let handler = crate::util::new_null_prototype_object(scope);
        let proxy = v8::Proxy::new(scope, document, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
        let key = crate::util::v8str(scope, "nativeDocument");
        assert_eq!(
            global.create_data_property(scope, key.into(), proxy.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const names=['DragEvent','HashChangeEvent','TouchEvent'];
        const originals=names.map(name=>window[name]);
        let constructorCalls=0;
        try {
            for (const name of names) window[name]=function(){constructorCalls++;throw 42;};
            for (const realm of [window,child]) {
                const create=realm.Document.prototype.createEvent;
                for (const [index,name] of names.entries()) {
                    const event=create.call(nativeDocument,name);
                    if (Object.getPrototypeOf(event)!==originals[index].prototype || event.type!=='' || event.isTrusted) throw Error('intrinsic creation');
                    event.initEvent('native',false,false);
                    if (!new EventTarget().dispatchEvent(event)) throw Error('native dispatch');
                }
                let conversions=0, traps=0;
                const author=new Proxy(nativeDocument,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
                const revoked=Proxy.revocable(nativeDocument,{});revoked.revoke();
                for (const receiver of [author,revoked.proxy,Object.create(nativeDocument)]) {
                    let error;
                    try { create.call(receiver,{toString(){conversions++;throw 42;}}); } catch(e) { error=e; }
                    if (!error || Object.getPrototypeOf(error)!==realm.TypeError.prototype || conversions || traps) throw Error('author receiver accepted');
                }
            }
            return constructorCalls===0;
        } finally {
            names.forEach((name,index)=>window[name]=originals[index]);
        }
    })()"#).unwrap(), "true");
}
