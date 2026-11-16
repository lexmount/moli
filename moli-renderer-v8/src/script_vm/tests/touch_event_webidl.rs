use super::*;

#[test]
fn native_touch_events_preserve_coordinates_identity_and_ignore_public_constructors() {
    let mut vm = new_storage_html_test_vm("https://native-touch-factory.test/");
    vm.eval(
        r#"
        globalThis.__nativeTouchTarget = document.body;
        globalThis.__nativeTouchConstructors = [Touch, TouchList, TouchEvent];
        globalThis.__nativeTouchConstructorReads = 0;
        for (const name of ['Touch', 'TouchList', 'TouchEvent']) {
            Object.defineProperty(globalThis, name, {configurable: true, get() {
                __nativeTouchConstructorReads++;
                throw new Error('public touch constructor must not run');
            }});
        }
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__nativeTouchTarget");
        let target =
            v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
        let point = crate::native_bridge::element::TouchEventPoint {
            identifier: 7,
            x: 17.25,
            y: 29.5,
            target,
            is_target_touch: true,
        };
        let event = crate::native_bridge::element::construct_touch_event_with_points(
            scope,
            "touchstart",
            &[point],
            &[point],
        )
        .expect("native touch input must bypass public constructors");
        let key = crate::util::v8str(scope, "__nativeTouchFactoryEvent");
        assert_eq!(
            global.create_data_property(scope, key.into(), event.into()),
            Some(true)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"(() => {
        const event = __nativeTouchFactoryEvent;
        const touch = event.touches[0];
        const [TouchClass, ListClass, EventClass] = __nativeTouchConstructors;
        return JSON.stringify({
            type: event.type, trusted: event.isTrusted,
            flags: [event.bubbles, event.cancelable, event.composed],
            brands: [event instanceof EventClass, event.touches instanceof ListClass, touch instanceof TouchClass],
            lengths: [event.touches.length, event.targetTouches.length, event.changedTouches.length],
            identity: touch === event.targetTouches[0] && touch === event.touches.item(0),
            target: touch.target === __nativeTouchTarget,
            coordinates: [touch.screenX, touch.screenY, touch.clientX, touch.clientY, touch.pageX, touch.pageY],
            constructorReads: __nativeTouchConstructorReads
        });
    })()"#).unwrap(), r#"{"type":"touchstart","trusted":true,"flags":[true,true,true],"brands":[true,true,true],"lengths":[1,1,1],"identity":true,"target":true,"coordinates":[17.25,29.5,17.25,29.5,17.25,29.5],"constructorReads":0}"#);
}

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
    assert_eq!(vm.eval("__touchEventWebIdlResults.total").unwrap(), "1046");
    assert_eq!(
        vm.eval("JSON.stringify(__touchEventWebIdlResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
}

#[test]
fn touch_native_proxies_retain_identity_while_author_proxies_are_rejected() {
    let mut vm = new_storage_html_test_vm("https://touch-native.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>';globalThis.child=document.querySelector('iframe').contentWindow;globalThis.touch=new Touch({identifier:7,target:document,clientX:2.5});globalThis.event=new TouchEvent('touchstart',{touches:[touch],altKey:true,modifierAltGraph:true});globalThis.list=event.touches;")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
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
