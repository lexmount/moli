use super::*;

#[test]
fn rectangle_receivers_preserve_brands_and_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rectangle-receivers.test/");
    vm.eval("document.body.innerHTML = '<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("rectangle_receivers.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "11054");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row => !row.passed).slice(0, 20))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "11054");
}

#[test]
fn rectangle_registered_proxies_share_native_values() {
    let mut vm = new_storage_page_task_executor_test_vm("https://rectangle-proxies.test/");
    vm.eval(
        r#"
      document.body.innerHTML='<iframe></iframe>';
      const child=document.querySelector('iframe').contentWindow;
      const detached=child.document.implementation.createHTMLDocument('');
      const root=detached.createElementNS('http://www.w3.org/2000/svg','svg');
      root.setAttribute('viewBox','10 20 30 40');
      const animated=root.viewBox;
      globalThis.__rectangleEntries={
        mutable:new child.DOMRect(1,2,-3,-4),readonly:new child.DOMRectReadOnly(5,6,-7,-8),
        base:animated.baseVal,anim:animated.animVal,animated,root
      };
      globalThis.__rectangleProxies={};
      globalThis.__rectangleTraps=0;
      globalThis.__rectangleProxyGet=function(){__rectangleTraps++;throw Error('proxy trap');};
    "#,
    )
    .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let entries_key = crate::util::v8str(scope, "__rectangleEntries");
        let entries = global.get(scope, entries_key.into()).unwrap();
        let entries = v8::Local::<v8::Object>::try_from(entries).unwrap();
        let proxies_key = crate::util::v8str(scope, "__rectangleProxies");
        let proxies = global.get(scope, proxies_key.into()).unwrap();
        let proxies = v8::Local::<v8::Object>::try_from(proxies).unwrap();
        let get_key = crate::util::v8str(scope, "__rectangleProxyGet");
        let get_trap = global.get(scope, get_key.into()).unwrap();
        for name in ["mutable", "readonly", "base", "anim", "animated", "root"] {
            let key = crate::util::v8str(scope, name);
            let target = entries.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let get_key = crate::util::v8str(scope, "get");
            assert_eq!(
                handler.create_data_property(scope, get_key.into(), get_trap),
                Some(true)
            );
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            assert_eq!(
                proxies.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
      const p=__rectangleProxies,t=__rectangleEntries,own=(kind,name)=>Object.getOwnPropertyDescriptor(child[kind].prototype,name);
      for(const [name,kind] of [['mutable','DOMRect'],['readonly','DOMRectReadOnly']]){
        const json=child.DOMRectReadOnly.prototype.toJSON.call(p[name]);
        if(Object.getPrototypeOf(json)!==child.Object.prototype)return name+' JSON realm';
        for(const field of ['x','y','width','height','top','right','bottom','left']){
          const proto=['x','y','width','height'].includes(field)?kind:'DOMRectReadOnly';
          if(own(proto,field).get.call(p[name])!==t[name][field]||json[field]!==t[name][field])return name+' '+field;
        }
        if(kind==='DOMRect'){
          own(kind,'x').set.call(p[name],9);
          if(t[name].x!==9)return name+' shared mutation';
        }else{
          let converted=0,error;
          try{own('DOMRect','x').set.call(p[name],{valueOf(){converted++;return 9;}});}catch(e){error=e;}
          if(!(error instanceof child.TypeError)||converted!==0||t[name].x!==5)return name+' mutation rejected';
        }
      }
      for(const [name,entry] of [['baseVal','base'],['animVal','anim']]){
        if(own('SVGAnimatedRect',name).get.call(p.animated)!==t[entry])return 'animated '+name;
      }
      const x=own('SVGRect','x');
      if(x.get.call(p.base)!==10||x.get.call(p.anim)!==10)return 'SVG shared reads';
      root.setAttribute('viewBox','50 60 70 80');
      if(x.get.call(p.base)!==50||x.get.call(p.anim)!==50)return 'SVG retained synchronization';
      x.set.call(p.base,{valueOf(){root.setAttribute('viewBox','1 2 3 4');return 7;}});
      if(root.getAttribute('viewBox')!=='7 2 3 4'||x.get.call(p.anim)!==7)return 'SVG reentrant write';
      let conversions=0,error;
      try{x.set.call(p.anim,{valueOf(){conversions++;return 8;}});}catch(e){error=e;}
      if(conversions!==1||!(error instanceof child.DOMException)||error.name!=='NoModificationAllowedError')return 'SVG readonly conversion';
      const sentinel={};
      try{x.set.call(p.anim,{valueOf(){throw sentinel;}});return 'missing conversion error';}catch(e){if(e!==sentinel)return 'wrong conversion error';}
      const rect=child.SVGSVGElement.prototype.createSVGRect.call(p.root);
      if(Object.getPrototypeOf(rect)!==child.SVGRect.prototype||rect.x!==0)return 'factory native receiver';
      rect.x=5;if(root.getAttribute('viewBox')!=='7 2 3 4')return 'factory detached';
      for(const receiver of [new Proxy(p.base,{}),Object.create(p.base)]){
        let converted=0,error;
        try{x.set.call(receiver,{valueOf(){converted++;return 1;}});}catch(e){error=e;}
        if(!(error instanceof child.TypeError)||converted!==0)return 'author proxy rejected';
      }
      return __rectangleTraps===0?'ok':'unexpected proxy trap';
    })()"#,
        )
        .unwrap(),
        "ok"
    );
}
