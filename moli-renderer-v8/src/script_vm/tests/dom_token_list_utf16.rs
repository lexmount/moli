use super::*;

#[test]
fn dom_token_list_utf16_tokens_preserve_units_and_validate_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://token-units.test/");
    vm.eval("document.body.innerHTML='<iframe></iframe>'")
        .unwrap();
    vm.eval(include_str!("dom_token_list_utf16.js")).unwrap();
    assert_eq!(vm.eval("__uiEventResults.complete").unwrap(), "true");
    assert_eq!(vm.eval("__uiEventResults.total").unwrap(), "8892");
    assert_eq!(
        vm.eval("JSON.stringify(__uiEventResults.checks.filter(row=>!row.passed).slice(0,30))")
            .unwrap(),
        "[]"
    );
    assert_eq!(vm.eval("__uiEventResults.passed").unwrap(), "8892");
    assert_eq!(
        vm.eval("DOMTokenList.prototype.add.length+':'+DOMTokenList.prototype.remove.length")
            .unwrap(),
        "0:0"
    );
}

#[test]
fn dom_token_list_utf16_registered_proxies_share_tokens_without_author_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://token-proxies.test/");
    vm.eval(
        r#"
        document.body.innerHTML='<iframe></iframe>';
        const child=document.querySelector('iframe').contentWindow;
        const detached=child.document.implementation.createHTMLDocument('');
        globalThis.__tokenReceivers={};globalThis.__tokenTraps=0;
        globalThis.__tokenTrap=function(){__tokenTraps++;throw Error('proxy trap');};
        for(const [name,ns,tag,key,attr] of [
          ['class',null,'div','classList','class'],['part',null,'div','part','part'],
          ['rel',null,'a','relList','rel'],['sandbox',null,'iframe','sandbox','sandbox'],
          ['for',null,'output','htmlFor','for'],['sizes',null,'link','sizes','sizes'],
          ['svg','http://www.w3.org/2000/svg','a','relList','rel']]){
          const element=ns?detached.createElementNS(ns,tag):detached.createElement(tag);
          __tokenReceivers[name]={element,target:element[key],attr};
        }
        "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "__tokenReceivers");
        let receivers = global.get(scope, key.into()).unwrap();
        let receivers = v8::Local::<v8::Object>::try_from(receivers).unwrap();
        let key = crate::util::v8str(scope, "__tokenTrap");
        let trap = global.get(scope, key.into()).unwrap();
        for name in ["class", "part", "rel", "sandbox", "for", "sizes", "svg"] {
            let key = crate::util::v8str(scope, name);
            let entry = receivers.get(scope, key.into()).unwrap();
            let entry = v8::Local::<v8::Object>::try_from(entry).unwrap();
            let key = crate::util::v8str(scope, "target");
            let target = entry.get(scope, key.into()).unwrap();
            let target = v8::Local::<v8::Object>::try_from(target).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let key = crate::util::v8str(scope, "get");
            assert_eq!(
                handler.create_data_property(scope, key.into(), trap),
                Some(true)
            );
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, "proxy");
            assert_eq!(
                entry.create_data_property(scope, key.into(), proxy.into()),
                Some(true)
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval(
            r#"(() => {
            for(const {element,target,proxy,attr} of Object.values(__tokenReceivers)){
              for(const realm of [globalThis,child]){
                const p=realm.DOMTokenList.prototype;
                const value=Object.getOwnPropertyDescriptor(p,'value');
                const length=Object.getOwnPropertyDescriptor(p,'length');
                value.set.call(proxy,'\ud800 \udfff');
                if(value.get.call(proxy)!=='\ud800 \udfff'||p.toString.call(proxy)!=='\ud800 \udfff'||length.get.call(proxy)!==2)return 'properties';
                if(p.item.call(proxy,0)!=='\ud800'||!p.contains.call(proxy,'\udfff'))return 'lookup';
                p.add.call(proxy,'\ufffd');p.remove.call(proxy,'\udfff');
                if(element.getAttribute(attr)!=='\ud800 \ufffd')return 'mutations';
                if(!p.replace.call(proxy,'\ufffd','last')||!p.toggle.call(proxy,'\udfff'))return 'replace/toggle';
                if(target.value!=='\ud800 last \udfff')return 'shared producer';
                const wrapped=new Proxy(proxy,{get:__tokenTrap}), revoked=Proxy.revocable(proxy,{});
                revoked.revoke();
                for(const receiver of [wrapped,revoked.proxy,Object.create(proxy)]){
                  let conversions=0,error;
                  try{p.add.call(receiver,{toString(){conversions++;return 'bad';}});}catch(e){error=e;}
                  if(!(error instanceof realm.TypeError)||conversions!==0)return 'author receiver';
                }
              }
            }
            return __tokenTraps===0?'ok':'trap';
            })()"#,
        )
        .unwrap(),
        "ok"
    );
}
