use super::*;

#[test]
fn css_keyframes_receivers_preserve_cached_list_prototypes_across_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "(() => {{const probe={};const realms=[globalThis,child.contentWindow];globalThis.cachedChecks=[];for(let a=0;a<2;a++)for(let b=0;b<2;b++)cachedChecks.push(...probe(realms[a],realms[b],a+'-'+b));return JSON.stringify(cachedChecks.filter(row=>!row.passed));}})()",
        include_str!("css_keyframes_cached_list.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "[]");
    assert_eq!(vm.eval("cachedChecks.length").unwrap(), "16");
}

#[test]
fn css_keyframes_receivers_use_native_brands_before_cross_realm_argument_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "(() => {{const probe={};const realms=[globalThis,child.contentWindow];globalThis.checks=[];for(let a=0;a<2;a++)for(let b=0;b<2;b++)checks.push(...probe(realms[a],realms[b],a+'-'+b));return JSON.stringify(checks.filter(row=>!row.passed));}})()",
        include_str!("css_keyframes_receivers.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "[]");
    assert_eq!(vm.eval("checks.length").unwrap(), "848");
}

#[test]
fn css_keyframes_receivers_share_native_proxy_backing_slots_and_owner_realm_caches() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval(
        r#"
      document.body.innerHTML='<iframe id=child></iframe>';
      globalThis.other=child.contentWindow;
      const sheet=new other.CSSStyleSheet();
      sheet.replaceSync('@keyframes registered {from {left:0px} to {left:100px}}');
      globalThis.keyframes=sheet.cssRules[0];
      globalThis.frame=keyframes[0];
    "#,
    )
    .unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("keyframes", "framesProxy"), ("frame", "frameProxy")] {
            let key = crate::util::v8str(scope, name);
            let target =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let handler = crate::util::new_null_prototype_object(scope);
            let proxy = v8::Proxy::new(scope, target, handler).unwrap();
            moli_webapi_declare::register_web_api_proxy(scope, proxy).unwrap();
            let key = crate::util::v8str(scope, proxy_name);
            global.create_data_property(scope, key.into(), proxy.into());
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"
(() => {
  const assert=(value,message)=>{if(!value)throw Error(message);};
  for(const bindings of [globalThis,other]) {
    const fp=bindings.CSSKeyframesRule.prototype,kp=bindings.CSSKeyframeRule.prototype;
    const descriptor=(p,n)=>Object.getOwnPropertyDescriptor(p,n);
    assert(descriptor(fp,'name').get.call(framesProxy)===keyframes.name,'native name');
    descriptor(fp,'name').set.call(framesProxy,'renamed');
    assert(keyframes.name==='renamed','name slots shared');
    const rules=descriptor(fp,'cssRules').get.call(framesProxy);
    assert(rules===keyframes.cssRules && rules instanceof other.CSSRuleList,'owner list realm and identity');
    assert(descriptor(fp,'length').get.call(framesProxy)===2,'native length');
    fp.appendRule.call(framesProxy,'75% {left:75px}');
    assert(keyframes.length===3 && fp.findRule.call(framesProxy,'75%')===keyframes[2],'native append/find');
    fp.deleteRule.call(framesProxy,'75%');assert(keyframes.length===2,'native delete');
    descriptor(kp,'keyText').set.call(frameProxy,'25%');
    assert(descriptor(kp,'keyText').get.call(frameProxy)==='25%' && frame.keyText==='25%','selector slots shared');
    const style=descriptor(kp,'style').get.call(frameProxy);
    assert(style===frame.style && style instanceof other.CSSStyleDeclaration,'owner style realm and identity');
    descriptor(kp,'style').set.call(frameProxy,'left:12px');
    assert(style.getPropertyValue('left')==='12px','PutForwards native mutation');
    let traps=0,conversions=0;
    for(const [p,name,kind,real] of [[fp,'name','get',framesProxy],[fp,'name','set',framesProxy],
      [fp,'cssRules','get',framesProxy],[fp,'length','get',framesProxy],
      [fp,'appendRule','value',framesProxy],[fp,'deleteRule','value',framesProxy],[fp,'findRule','value',framesProxy],
      [kp,'keyText','get',frameProxy],[kp,'keyText','set',frameProxy],[kp,'style','get',frameProxy],[kp,'style','set',frameProxy]]) {
      const proxy=new Proxy(real,{get(){traps++;throw Error('trap');},getPrototypeOf(){traps++;throw Error('trap');}});
      let error;try {descriptor(p,name)[kind].call(proxy,{toString(){conversions++;return 'changed';}});} catch(caught){error=caught;}
      assert(error instanceof bindings.TypeError,name+' rejects author Proxy around native Proxy');
    }
    assert(traps===0 && conversions===0,'brand check precedes author code');
  }
  return true;
})()
"#).unwrap(), "true");
}

#[test]
fn css_keyframes_receivers_keep_live_rules_mutable_after_stylesheet_removal() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert=(value,message)=>{if(!value)throw Error(message);};
  const style=document.createElement('style');
  style.textContent='@keyframes attached {from {left:0px} to {left:100px}}';
  document.head.appendChild(style);
  const sheet=style.sheet,frames=sheet.cssRules[0],frame=frames[0],rules=frames.cssRules,declaration=frame.style;
  const name=Object.getOwnPropertyDescriptor(CSSKeyframesRule.prototype,'name');
  const keyText=Object.getOwnPropertyDescriptor(CSSKeyframeRule.prototype,'keyText');
  const putForwards=Object.getOwnPropertyDescriptor(CSSKeyframeRule.prototype,'style').set;
  name.set.call(frames,'live');keyText.set.call(frame,'20%');putForwards.call(frame,'left:5px');
  assert(sheet.cssRules[0]===frames && frames.findRule('20%')===frame && declaration.left==='5px','live native mutation');
  style.remove();sheet.deleteRule(0);
  name.set.call(frames,'detached');keyText.set.call(frame,'30%');putForwards.call(frame,'left:9px');
  frames.appendRule('75% {left:75px}');
  assert(frames.name==='detached' && frames.cssRules===rules && frames.findRule('30%')===frame,'detached keyframes identity');
  assert(frame.style===declaration && declaration.left==='9px' && frames.length===3,'detached declaration and append');
  frames.deleteRule('75%');assert(frames.length===2 && frames.findRule('75%')===null,'detached delete');
  return true;
})()
"#).unwrap(), "true");
}
