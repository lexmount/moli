use super::*;

#[test]
fn svg_hyperlinks_reflect_attributes_and_preserve_conversion_order() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-hyperlinks.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("svg_hyperlinks.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "1656");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed).slice(0,20))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "1656");
}

#[test]
fn svg_hyperlink_registered_proxies_share_native_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://svg-link-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML='<iframe></iframe>';
        const child=document.querySelector('iframe').contentWindow;
        const detached=child.document.implementation.createHTMLDocument('');
        const anchor=detached.createElementNS('http://www.w3.org/2000/svg','a');
        anchor.setAttribute('href','https://example.test/old');
        globalThis.__linkTargets={anchor,target:anchor.target,href:anchor.href};
        globalThis.__linkProxies={};globalThis.__linkTraps=0;
        globalThis.__linkTrap=function(){__linkTraps++;throw Error('proxy trap');};
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let targets_key = crate::util::v8str(scope, "__linkTargets");
        let targets = global.get(scope, targets_key.into()).unwrap();
        let targets = v8::Local::<v8::Object>::try_from(targets).unwrap();
        let proxies_key = crate::util::v8str(scope, "__linkProxies");
        let proxies = global.get(scope, proxies_key.into()).unwrap();
        let trap_key = crate::util::v8str(scope, "__linkTrap");
        let trap = global.get(scope, trap_key.into()).unwrap();
        let proxies = v8::Local::<v8::Object>::try_from(proxies).unwrap();
        for name in ["anchor", "target", "href"] {
            let key = crate::util::v8str(scope, name);
            let target = targets.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let get_key = crate::util::v8str(scope, "get");
            assert_eq!(
                handler.create_data_property(scope, get_key.into(), trap),
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
            const own=(realm,kind,key)=>Object.getOwnPropertyDescriptor(realm[kind].prototype,key);
            const proxy=__linkProxies.anchor,real=__linkTargets.anchor;
            for(const realm of [globalThis,child]){
              for(const [property,value,attr] of [['download','\ud800name','download'],['rel','noopener','rel'],['hreflang','en','hreflang'],['type','text/html','type'],['referrerPolicy','ORIGIN','referrerpolicy'],['ping','\ud800path','ping']]){
                const d=own(realm,'SVGAElement',property);d.set.call(proxy,value);
                const expected=property==='ping'?'\ufffdpath':value;
                if(real.getAttribute(attr)!==expected||d.get.call(proxy)!==(property==='referrerPolicy'?'origin':expected))return property;
              }
              for(const property of ['target','href'])if(own(realm,'SVGAElement',property).get.call(proxy)!==__linkTargets[property])return property+' identity';
              const list=own(realm,'SVGAElement','relList').get.call(proxy);
              if(Object.getPrototypeOf(list)!==child.DOMTokenList.prototype)return 'list realm';
              own(realm,'SVGAElement','relList').set.call(proxy,'\ud800 token');
              if(real.rel!=='\ud800 token')return 'list value';
              list.value='\udfff value';
              if(real.rel!=='\udfff value'||list.value!=='\udfff value')return 'DOMTokenList UTF16';
              own(realm,'SVGAElement','pathname').set.call(proxy,'/new');
              if(own(realm,'SVGAElement','host').get.call(proxy)!=='example.test'||real.href.baseVal!=='https://example.test/new')return 'URL';
              own(realm,'SVGAnimatedString','baseVal').set.call(__linkProxies.target,'_blank');
              if(real.getAttribute('target')!=='_blank')return 'target mutation';
              const wrapped=new Proxy(proxy,{get:__linkTrap});let converted=0,error;
              try{own(realm,'SVGAElement','download').set.call(wrapped,{toString(){converted++;return 'bad';}});}catch(e){error=e;}
              if(!(error instanceof realm.TypeError)||converted!==0)return 'author wrapper';
            }
            return __linkTraps===0?'ok':'proxy trap';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
