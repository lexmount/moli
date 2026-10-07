use super::*;

#[test]
fn touch_events_use_typed_dictionaries_native_sequences_and_receivers() {
    let mut vm = new_parsed_test_vm(
        "https://touch-event-webidl.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    vm.eval(include_str!("touch_event_webidl.js")).unwrap();
    assert_eq!(
        vm.eval("__touchEventWebIdlResults.complete").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("__touchEventWebIdlResults.total").unwrap(), "1044");
    assert_eq!(
        vm.eval("JSON.stringify(__touchEventWebIdlResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn touch_native_proxies_retain_identity_while_author_proxies_are_rejected() {
    let mut vm = new_storage_page_task_executor_test_vm("https://touch-native.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.child=document.querySelector('iframe').contentWindow;globalThis.touch=new Touch({identifier:7,target:document,clientX:2.5});globalThis.event=new TouchEvent('touchstart',{touches:[touch],altKey:true,modifierAltGraph:true});globalThis.list=event.touches;")
        .unwrap();
    let context_ptr = &vm.page_default_context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [
            ("touch", "native_touch"),
            ("event", "native_event"),
            ("list", "native_list"),
        ] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            assert_eq!(
                global.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      for(const realm of [window,child]) {
        for(const [name,receiver,key,expected] of [
          ['Touch',native_touch,'clientX',2.5],['TouchEvent',native_event,'touches',list],
          ['TouchEvent',native_event,'altKey',true],['TouchList',native_list,'length',1]]) {
          const getter=Object.getOwnPropertyDescriptor(realm[name].prototype,key).get;
          if(getter.call(receiver)!==expected)throw Error('native Proxy payload');
          let traps=0;const author=new Proxy(receiver,{get(){traps++;throw 42;},getPrototypeOf(){traps++;throw 42;}});
          const revoked=Proxy.revocable(receiver,{});revoked.revoke();
          for(const fake of [author,revoked.proxy,Object.create(receiver)]) {
            let error;try{getter.call(fake);}catch(e){error=e;}
            if(!error || Object.getPrototypeOf(error)!==realm.TypeError.prototype || traps)throw Error('author receiver');
          }
        }
        if(realm.TouchList.prototype.item.call(native_list,0)!==touch ||
          !realm.TouchEvent.prototype.getModifierState.call(native_event,'AltGraph'))throw Error('native Proxy method');
        const copied=new realm.TouchEvent('copy',{touches:[native_touch]});
        if(copied.touches[0]!==native_touch || copied.touches.item(0)!==native_touch)throw Error('native Proxy sequence');
        const target=new realm.Touch({identifier:1,target:document.implementation.createHTMLDocument('').createElement('select')});
        if(target.target.localName!=='select')throw Error('native EventTarget proxy');
      }
      return true;
    })()"#).unwrap(), "true");
}
