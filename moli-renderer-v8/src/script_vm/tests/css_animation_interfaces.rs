use super::*;

#[test]
fn css_animation_interfaces_use_native_cross_realm_receivers_and_dictionary_conversion() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    vm.eval("document.body.innerHTML='<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        "(() => {{const probe={};const realms=[globalThis,child.contentWindow];globalThis.checks=[];for(let a=0;a<2;a++)for(let b=0;b<2;b++)checks.push(...probe(realms[a],realms[b],a+'-'+b));return JSON.stringify(checks.filter(row=>!row.passed));}})()",
        include_str!("css_animation_interfaces.js")
    );
    assert_eq!(vm.eval(&source).unwrap(), "[]");
    assert_eq!(vm.eval("checks.length").unwrap(), "348");
}

#[test]
fn css_animation_interfaces_read_decoded_names_from_the_current_native_cascade() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => {if (!ok) throw Error(message);};
  const style = document.createElement('style');
  style.textContent = `
    @keyframes xyz { to {left:100px} }
    @keyframes ABC {}
    @keyframes "comma,name" {}
    @keyframes "none" {}
    @media not all { @keyframes unavailable {} }
    @media all { @keyframes available {} }
    @supports (display: block) { @keyframes supported {} }
    @supports (definitely-unknown-property: value) { @keyframes unsupported {} }
  `;
  document.head.appendChild(style);
  const item = document.createElement('div'); document.body.appendChild(item);
  for (const [value, expected] of [
    ['xyz 100s','xyz'], ['x\\yz 100s','xyz'], ['x\\79 z 100s','xyz'],
    ['ABC 100s','ABC'], ['"comma,name" 100s','comma,name'], ['"none" 100s','none'],
    ['available 100s','available'], ['supported 100s','supported']
  ]) {
    item.style.animation = value;
    const result = item.getAnimations();
    assert(result.length===1 && result[0] instanceof CSSAnimation && result[0].animationName===expected, value);
  }
  for (const value of ['abc 100s','unavailable 100s','unsupported 100s','missing 100s','none']) {
    item.style.animation=value; assert(item.getAnimations().length===0,value+' has no active keyframes');
  }
  item.style.animation='xyz 100s'; item.style.display='none';
  assert(item.getAnimations().length===0,'display:none');
  item.style.display='block'; assert(item.getAnimations().length===1,'display restored');
  item.remove(); assert(item.getAnimations().length===0,'detached target');
  return 'passed';
})()
"#).unwrap(), "passed");
}

#[test]
fn css_animation_interfaces_keep_identity_and_match_duplicate_names_from_the_end() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert = (ok, message) => {if (!ok) throw Error(message);};
  document.head.appendChild(document.createElement('style')).textContent='@keyframes first {} @keyframes second {}';
  const item=document.createElement('div');document.body.appendChild(item);
  item.style.animation='first 100s, second 100s';
  const [first,second]=item.getAnimations();first.id='saved';
  assert(item.getAnimations()[0]===first && item.getAnimations()[1]===second,'stable identity');
  item.style.animation='second 100s, first 100s';
  assert(item.getAnimations()[0]===second && item.getAnimations()[1]===first && first.id==='saved','reordered identity');
  item.style.animation='first 100s, first 100s';
  const pair=item.getAnimations();assert(pair.length===2 && pair[0]!==first && pair[1]===first,'new duplicate prepended');
  item.style.animation='first 100s';
  assert(item.getAnimations()[0]===first,'rightmost duplicate retained');
  item.style.animation='second 100s';const replacement=item.getAnimations()[0];
  assert(first.animationName==='first' && replacement.animationName==='second','original name retained');
  item.style.animation='none';assert(item.getAnimations().length===0,'remove animation');
  item.style.animation='second 100s';assert(item.getAnimations()[0]!==replacement,'new object after removal');
  return 'passed';
})()
"#).unwrap(), "passed");
}

#[test]
fn css_animation_interfaces_resolve_shadow_keyframes_without_document_name_leaks() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  const host=document.createElement('div');document.body.appendChild(host);
  const root=host.attachShadow({mode:'open'});
  const style=document.createElement('style');style.textContent='@keyframes shadowOnly {}';root.appendChild(style);
  const inner=document.createElement('span');root.appendChild(inner);inner.style.animation='shadowOnly 100s';
  const outer=document.createElement('span');document.body.appendChild(outer);outer.style.animation='shadowOnly 100s';
  assert(inner.getAnimations().length===1 && inner.getAnimations()[0].animationName==='shadowOnly','shadow keyframes');
  assert(outer.getAnimations().length===0,'keyframes do not escape shadow scope');
  style.sheet.cssRules[0].name='renamed';
  assert(inner.getAnimations().length===0,'CSSOM rename removes old match');
  inner.style.animationName='renamed';assert(inner.getAnimations()[0].animationName==='renamed','CSSOM rename creates new match');
  return 'passed';
})()
"#).unwrap(), "passed");
}

#[test]
fn css_animation_interfaces_factories_keep_intrinsics_after_public_constructor_overrides() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    assert_eq!(vm.eval(r#"
(() => {
  const css=CSSAnimation, effect=KeyframeEffect, base=Animation;
  const descriptors=['CSSAnimation','KeyframeEffect','Animation'].map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
  let calls=0;
  try {
    for(const [name] of descriptors)Object.defineProperty(globalThis,name,{configurable:true,get(){calls++;throw Error(name);}});
    document.head.appendChild(document.createElement('style')).textContent='@keyframes intact {}';
    const item=document.createElement('div');document.body.appendChild(item);item.style.animation='intact 100s';
    const animation=item.getAnimations()[0];
    return animation instanceof css && animation instanceof base && animation.effect instanceof effect && animation.animationName==='intact' && calls===0;
  } finally {for(const [name,descriptor] of descriptors)Object.defineProperty(globalThis,name,descriptor);}
})()
"#).unwrap(), "true");
}

#[test]
fn css_animation_interfaces_accept_registered_native_proxies_without_author_proxy_traps() {
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    vm.eval("document.head.appendChild(document.createElement('style')).textContent='@keyframes nativeProxy {}';globalThis.item=document.createElement('div');document.body.appendChild(item);item.style.animation='nativeProxy 100s';globalThis.animation=item.getAnimations()[0];").unwrap();
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let global = scope.get_current_context().global(scope);
        for (name, proxy_name) in [("animation", "animationProxy"), ("item", "itemProxy")] {
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
  const getter=Object.getOwnPropertyDescriptor(CSSAnimation.prototype,'animationName').get;
  if(getter.call(animationProxy)!=='nativeProxy' || Element.prototype.getAnimations.call(itemProxy)[0]!==animation)throw Error('native proxy');
  const event=new AnimationEvent('test',{animation:animationProxy});if(event.animation!==animationProxy)throw Error('argument identity');
  let conversions=0,traps=0,error;
  try {new AnimationEvent('test',{animation:new Proxy(animationProxy,{get(){traps++;},getPrototypeOf(){traps++;}}),get animationName(){conversions++;return '';}});}catch(caught){error=caught;}
  return error instanceof TypeError && conversions===0 && traps===0;
})()
"#).unwrap(), "true");
}

#[tokio::test]
async fn css_animation_interfaces_native_start_events_share_the_get_animations_objects() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation.test/",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
      document.head.appendChild(document.createElement('style')).textContent='@keyframes first {to {left:100px}} @keyframes second {to {left:200px}}';
      globalThis.item=document.createElement('div');document.body.appendChild(item);item.style.animation='first 100s,second 100s';
      globalThis.animations=item.getAnimations();globalThis.events=[];
      item.addEventListener('animationstart',event=>events.push(event));
      globalThis.animationEventCtor=AnimationEvent;globalThis.nativeGetterReads=0;
      Object.defineProperty(globalThis,'AnimationEvent',{configurable:true,get(){nativeGetterReads++;throw Error('public constructor');}});
      Object.defineProperty(Object.prototype,'animation',{configurable:true,get(){nativeGetterReads++;throw Error('prototype');}});
    "#).unwrap();
    assert!(!vm.has_ready_timeout());
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(vm.eval(r#"
(() => {
  delete Object.prototype.animation;Object.defineProperty(globalThis,'AnimationEvent',{configurable:true,writable:true,value:animationEventCtor});
  const facts=events.map((event,index)=>({instance:event instanceof AnimationEvent,trusted:event.isTrusted,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,identity:event.animation===animations[index],name:event.animationName,expectedName:animations[index]?.animationName,pseudoElement:event.pseudoElement}));
  const passed=events.length===2 && events.every((event,index)=>event instanceof AnimationEvent && event.isTrusted && event.bubbles && !event.cancelable && !event.composed && event.animation===animations[index] && event.animationName===animations[index].animationName && event.pseudoElement==='') && nativeGetterReads===0;
  if(!passed)throw Error(JSON.stringify({length:events.length,facts,nativeGetterReads}));
  return true;
})()
"#).unwrap(), "true");
}

#[test]
fn css_animation_interfaces_transition_payload_uses_a_native_typed_association() {
    use moli_webapi_declare::WebApiObject;
    #[derive(WebApiObject)]
    #[webapi(interface = crate::web_api_interfaces::CSSTransition, require_prototype)]
    struct NativeTransitionForTest {
        #[webapi(slot = "__moliCssTransitionProperty")]
        property: &'static str,
    }
    // Exercise the typed binding independently of the pending CSS transition
    // producer. This does not claim that style changes create transitions yet.
    let mut vm = new_storage_page_task_executor_test_vm("https://css-animation.test/");
    let context_ptr = &vm.page_default_runtime.context as *const _;
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        let transition = NativeTransitionForTest::new("left").bind(scope).unwrap();
        let global = scope.get_current_context().global(scope);
        let key = crate::util::v8str(scope, "transition");
        global.create_data_property(scope, key.into(), transition.into());
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval(r#"
(() => {
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  document.body.innerHTML='<iframe id=frame></iframe>';const other=frame.contentWindow;
  assert(transition instanceof CSSTransition && transition instanceof Animation,'native inheritance');
  assert(Object.getOwnPropertyDescriptor(other.CSSTransition.prototype,'transitionProperty').get.call(transition)==='left','cross realm property');
  const event=new other.TransitionEvent('test',{animation:transition});
  assert(event.animation===transition && event instanceof other.TransitionEvent,'native association');
  let laterReads=0,error;
  try{new other.AnimationEvent('test',{animation:transition,get animationName(){laterReads++;return 'wrong';}});}catch(caught){error=caught;}
  assert(error instanceof other.TypeError && !(error instanceof TypeError) && laterReads===0,'wrong CSS animation interface');
  return 'passed';
})()
"#).unwrap(), "passed");
}

#[tokio::test]
async fn css_animation_interfaces_native_start_elapsed_time_uses_css_delay_and_active_duration() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://css-animation.test/",
        "<html><head></head><body></body></html>",
        &loader,
    );
    vm.eval(r#"
      document.head.appendChild(document.createElement('style')).textContent='@keyframes start {to {left:100px}}';
      globalThis.item=document.createElement('div');document.body.appendChild(item);
      item.style.animation='start 100s -0.5s,start 1s -3s 2 forwards,start 100s 2s';
      globalThis.elapsed=[];item.addEventListener('animationstart',event=>elapsed.push(event.elapsedTime));
    "#).unwrap();
    assert!(
        vm.run_one_rendering_update_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(vm.eval("JSON.stringify(elapsed)").unwrap(), "[0.5,2,0]");
}
