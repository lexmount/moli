use super::*;

#[test]
fn css_keyframes_mutations_use_cssom_error_semantics_across_realms_and_rule_lifetimes() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "(() => {{const probe={};const realms=[globalThis,child.contentWindow];globalThis.mutationChecks=[];for(let a=0;a<2;a++)for(let b=0;b<2;b++)mutationChecks.push(...probe(realms[a],realms[b],a+'-'+b));return JSON.stringify(mutationChecks.filter(row=>!row.passed));}})()",
        include_str!("css_keyframes_mutations.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "[]");
    assert_eq!(vm.eval("mutationChecks.length").unwrap(), "1068");
}

#[test]
fn css_keyframes_mutations_accept_native_proxies_and_preserve_intrinsic_exception_realms() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval(r#"
      document.body.innerHTML='<iframe id=child></iframe>';
      globalThis.other=child.contentWindow;
      const sheet=new other.CSSStyleSheet();sheet.replaceSync('@keyframes native {from {left:0px}}');
      globalThis.keyframes=sheet.cssRules[0];globalThis.frame=keyframes[0];
    "#).unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("keyframes", "keyframesProxy"), ("frame", "frameProxy")] {
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
  const realms=[globalThis,other],exceptions=realms.map(realm=>realm.DOMException);
  const descriptors=realms.map(realm=>Object.getOwnPropertyDescriptor(realm,'DOMException'));
  let constructorReads=0;
  try {
    for(const realm of realms)Object.defineProperty(realm,'DOMException',{configurable:true,get(){constructorReads++;throw Error('public constructor');}});
    for(let index=0;index<realms.length;index++) {
      const realm=realms[index],setter=Object.getOwnPropertyDescriptor(realm.CSSKeyframeRule.prototype,'keyText').set;
      for(const supplied of [true,false]) {
        let error;try{if(supplied)setter.call(frameProxy,'body');else setter.call(frameProxy);}catch(caught){error=caught;}
        assert(error instanceof exceptions[index] && !(error instanceof exceptions[1-index]) && error.name==='SyntaxError' && error.code===12,'native SyntaxError uses callee intrinsic');
      }
      assert(frame.keyText==='0%' && keyframes[0]===frame,'invalid setter preserves backing rule');
      assert(realm.CSSKeyframesRule.prototype.appendRule.call(keyframesProxy,'body {}')===undefined && keyframes.length===1,'invalid native append ignored');
      setter.call(frameProxy,'50%');assert(frame.keyText==='50%','valid native selector update');
      realm.CSSKeyframesRule.prototype.appendRule.call(keyframesProxy,'to {left:100px}');
      assert(keyframes.length===2 && keyframes[1].keyText==='100%','valid native append');
      realm.CSSKeyframesRule.prototype.deleteRule.call(keyframesProxy,'to');setter.call(frameProxy,'from');
    }
    return constructorReads===0;
  } finally {for(let index=0;index<realms.length;index++)Object.defineProperty(realms[index],'DOMException',descriptors[index]);}
})()
"#).unwrap(), "true");
}

#[test]
fn css_keyframes_mutations_invalid_input_preserves_native_cascade_and_animation_identity() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-keyframes.test/");
    vm.eval(r#"
      const style=document.createElement('style');style.textContent='@keyframes stable {from {left:0px} to {left:100px}}';document.head.appendChild(style);
      globalThis.keyframes=style.sheet.cssRules[0];globalThis.frame=keyframes[0];
      globalThis.item=document.createElement('div');document.body.appendChild(item);item.style.animation='stable 100s';
      globalThis.animation=item.getAnimations()[0];
    "#).unwrap();
    crate::live_stylesheet::reset_live_stylesheet_mutation_metrics_for_test();
    assert_eq!(vm.eval(r#"
(() => {
  for(let index=0;index<20;index++) {
    let error;try{frame.keyText='body';}catch(caught){error=caught;}
    if(!(error instanceof DOMException) || error.name!=='SyntaxError')throw Error('invalid selector');
    if(keyframes.appendRule('body {}')!==undefined)throw Error('invalid append');
  }
  return frame.keyText==='0%' && keyframes.length===2 && keyframes[0]===frame && item.getAnimations()[0]===animation;
})()
"#).unwrap(), "true");
    let metrics = crate::live_stylesheet::live_stylesheet_mutation_metrics_for_test();
    assert_eq!(metrics.native_keyframe_mutations, 0);
    assert_eq!(metrics.native_rule_value_mutations, 0);
    assert_eq!(metrics.native_top_level_mutations, 0);
    assert_eq!(metrics.native_nested_mutations, 0);
}
