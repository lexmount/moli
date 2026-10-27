use super::*;

#[test]
fn css_animation_wrappers_preserve_instances_and_sample_seeked_time() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/identity",
        r#"<!doctype html><style>
      @keyframes move {from {left:0px} to {left:100px}}
      #box {position:relative;animation:move 1s linear both, move 2s linear both}
    </style><div id=box></div>"#,
    );
    let result = vm
        .eval(
            r#"(() => {
      const box=document.getElementById('box');const first=box.getAnimations();
      const [a,b]=first; a.pause();b.pause();a.currentTime=250;b.currentTime=500;
      const second=box.getAnimations();
      return JSON.stringify([first.length,a===second[0],b===second[1],a!==b,
        a instanceof CSSAnimation,b instanceof CSSAnimation,a.animationName,a.effect.target===box,
        a.playState,a.currentTime,a.effect.getTiming().duration,b.effect.getTiming().duration,
        getComputedStyle(box).left]);
    })()"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"[2,true,true,true,true,true,"move",true,"paused",250,1000,2000,"25px"]"#
    );
}

#[test]
fn script_animation_clock_drives_effect_timing_and_style_without_inline_mutation() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/script",
        "<!doctype html><div id=box></div>",
    );
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box');
      const a=box.animate([{opacity:0},{opacity:1}],{duration:1000,easing:'linear',fill:'both'});
      a.pause();a.currentTime=250;
      const values=[a===box.getAnimations()[0],a.currentTime,getComputedStyle(box).opacity,box.style.opacity];
      a.effect.updateTiming({duration:2000});values.push(getComputedStyle(box).opacity,a.effect.getTiming().duration);
      a.effect.setKeyframes([{opacity:0},{opacity:.5}]);values.push(getComputedStyle(box).opacity);
      a.finish();values.push(a.playState,getComputedStyle(box).opacity,box.style.opacity);
      return JSON.stringify(values);
    })()"#).unwrap();
    assert_eq!(
        result,
        r#"[true,250,"0.25","","0.125",2000,"0.0625","finished","0.5",""]"#
    );
}

#[test]
fn css_cancel_replay_and_script_timing_edits_share_the_existing_instance() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/css-control",
        r#"<!doctype html><style>
      @keyframes move {from{left:0px}to{left:100px}}
      #box{position:relative;left:3px;animation:move 1s linear both}
    </style><div id=box></div>"#,
    );
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box');const a=box.getAnimations()[0];a.pause();a.currentTime=250;
      a.effect.updateTiming({duration:2000});document.body.className='unrelated';
      const values=[a.effect.getTiming().duration,getComputedStyle(box).left];
      box.style.animationDuration='3s';getComputedStyle(box).left;values.push(a.effect.getTiming().duration);
      a.cancel();values.push(a.playState,box.getAnimations().length,getComputedStyle(box).left);
      document.body.className='again';values.push(box.getAnimations().length);
      a.play();a.pause();a.currentTime=750;values.push(box.getAnimations()[0]===a,getComputedStyle(box).left);
      return JSON.stringify(values);
    })()"#).unwrap();
    assert_eq!(
        result,
        r#"[2000,"12.5px",2000,"idle",0,"3px",0,true,"37.5px"]"#
    );
}

#[test]
fn replacing_effects_and_targets_updates_the_same_native_animation() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/effect-transfer",
        r#"<!doctype html><div id=a style="position:relative;left:3px"></div><div id=b style="position:relative;left:7px"></div>"#,
    );
    let result=vm.eval(r#"(() => {
      const a=document.getElementById('a'),b=document.getElementById('b');
      const animation=a.animate([{left:'0px'},{left:'100px'}],{duration:1000,fill:'both'});
      animation.pause();animation.currentTime=250;
      const old=animation.effect;
      const replacement=new KeyframeEffect(b,[{left:'0px'},{left:'200px'}],{duration:1000,fill:'both'});
      animation.effect=replacement;
      const values=[old!==animation.effect,old.target===a,old.getTiming().duration,
        a.getAnimations().length,b.getAnimations()[0]===animation,
        getComputedStyle(a).left,getComputedStyle(b).left];
      replacement.target=a;
      values.push(a.getAnimations()[0]===animation,b.getAnimations().length,getComputedStyle(a).left,getComputedStyle(b).left);
      animation.effect=null;values.push(a.getAnimations().length,getComputedStyle(a).left,replacement.target===a);
      const another=new Animation(old);another.pause();another.currentTime=500;
      values.push(another.effect===old,getComputedStyle(a).left);
      return JSON.stringify(values);
    })()"#).unwrap();
    assert_eq!(
        result,
        r#"[true,true,1000,0,true,"3px","50px",true,0,"50px","7px",0,"3px",true,true,"50px"]"#
    );
}

#[test]
fn seeked_effects_drive_typed_layout_and_leave_authored_style_unchanged() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/layout",
        r#"<!doctype html><style>body{margin:0}#box{position:absolute;left:0;top:0;width:10px;height:20px}</style><div id=box></div>"#,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box');
      const a=box.animate([{width:'10px',transform:'translateX(0px)'},{width:'110px',transform:'translateX(100px)'}],{duration:1000,easing:'linear',fill:'both'});
      a.pause();a.currentTime=250;
      const first=box.getBoundingClientRect();
      const values=[getComputedStyle(box).width,box.offsetWidth,first.width,first.x];
      a.currentTime=750;
      const second=box.getBoundingClientRect();
      values.push(getComputedStyle(box).width,box.offsetWidth,second.width,second.x,box.style.cssText);
      a.cancel();values.push(box.getBoundingClientRect().width,box.getBoundingClientRect().x);
      return JSON.stringify(values);
    })()"#).unwrap();
    assert_eq!(result, r#"["35px",35,35,25,"85px",85,85,75,"",10,0]"#);
}

#[tokio::test]
async fn playback_promises_observe_committed_state_before_finish_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-core.test/promises",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(
        r#"
      globalThis.trace=[];
      const box=document.getElementById('box');
      globalThis.animation=box.animate([{opacity:0},{opacity:1}],{duration:40,fill:'both'});
      animation.ready.then(()=>trace.push('ready'));
      animation.finished.then(()=>trace.push('promise:'+animation.playState));
      animation.onfinish=()=>trace.push('event:'+getComputedStyle(box).opacity);
    "#,
    )
    .unwrap();
    wait_for_css_events(&mut vm, &loader, "trace.length.toString()", "3").await;
    assert_eq!(
        vm.eval("JSON.stringify(trace)").unwrap(),
        r#"["ready","promise:finished","event:1"]"#
    );
}

#[tokio::test]
async fn stale_finish_notification_does_not_resolve_the_replayed_finished_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-core.test/notification-cycles",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(r#"
      globalThis.trace=[];globalThis.newSettled=false;
      globalThis.animation=document.getElementById('box').animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});
      animation.finished.then(()=>trace.push('old-promise'));
      animation.onfinish=()=>trace.push('old-event');
      animation.finish();animation.play();animation.pause();animation.currentTime=200;
      animation.finished.then(()=>newSettled=true);
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([trace,newSettled,animation.playState,animation.currentTime])")
            .unwrap(),
        r#"[["old-promise","old-event"],false,"paused",200]"#
    );
}

#[tokio::test]
async fn detached_script_effects_keep_their_timeline_and_do_not_retain_wrapper_cycles() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-core.test/gc",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(
        r#"
      globalThis.box=document.getElementById('box');
      globalThis.animation=box.animate([{opacity:0},{opacity:1}],{duration:40,fill:'both'});
      box.remove();globalThis.finished=false;animation.finished.then(()=>finished=true);
    "#,
    )
    .unwrap();
    wait_for_css_events(&mut vm, &loader, "finished.toString()", "true").await;
    assert_eq!(
        vm.eval("box.getAnimations()[0]===animation").unwrap(),
        "true"
    );
    let (wrapper, instance) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let object = global
                .get(scope, crate::util::v8str(scope, "animation").into())
                .unwrap();
            let object = v8::Local::<v8::Object>::try_from(object).unwrap();
            let instance = crate::animation_bindings::instance(scope, object).unwrap();
            Ok((
                v8::Weak::new(scope, object),
                std::rc::Rc::downgrade(&instance),
            ))
        })
        .unwrap();
    // A forward-filling effect is intentionally retained by its target. Cancel
    // it first, then verify the weak native index does not add another root.
    vm.eval("animation.cancel();animation=box=null; 'released'")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            isolate.low_memory_notification();
            Ok(())
        })
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        assert!(
            wrapper.to_local(scope).is_none(),
            "private wrapper cycles must be collectible"
        );
        assert!(
            instance.upgrade().is_none(),
            "the target index must not own native effects"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn reversing_a_transition_starts_at_its_sampled_value_and_shortens_the_return() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/reversal",
        r#"<!doctype html><div id=box style="opacity:0;transition:opacity 1s linear"></div>"#,
    );
    let result=vm.eval(r#"(() => {
      const box=document.getElementById('box');getComputedStyle(box).opacity;box.style.opacity=1;
      const old=box.getAnimations()[0];old.pause();old.currentTime=400;
      const before=getComputedStyle(box).opacity;
      box.style.opacity=0;const a=box.getAnimations()[0];a.pause();a.currentTime=200;
      return JSON.stringify([a!==old,old.playState,a.effect.getTiming().duration,before,getComputedStyle(box).opacity]);
    })()"#).unwrap();
    assert_eq!(result, r#"[true,"idle",400,"0.4","0.2"]"#);
}

#[tokio::test]
async fn infinite_effects_keep_demand_sampling_without_arming_periodic_deadlines() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-core.test/infinite-service",
        "<!doctype html><div id=box style='width:10px;height:20px'></div>",
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(
        r#"
      globalThis.animation=document.getElementById('box').animate([{width:'10px'},{width:'110px'}],
        {duration:1000,iterations:Infinity,easing:'linear',fill:'both'});
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, runtime| {
        let runtime = unsafe { &*runtime };
        let owner = runtime
            .current_window_document_task_target(scope)
            .unwrap()
            .owner();
        assert!(runtime.animation_transition_state().has_ticking_effects());
        assert!(
            crate::native_bridge::element::next_animation_transition_deadline(runtime, owner)
                .is_none(),
            "an active infinite effect has no lifecycle boundary to poll"
        );
        Ok(())
    })
    .unwrap();
    vm.eval("document.getElementById('box').getBoundingClientRect().width")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let value = global
            .get(scope, crate::util::v8str(scope, "animation").into())
            .unwrap();
        let value = v8::Local::<v8::Object>::try_from(value).unwrap();
        let animation = crate::animation_bindings::instance(scope, value).unwrap();
        // Advance only the clock. No DOM mutation or JS currentTime setter
        // invalidates layout on behalf of this demand.
        animation
            .borrow_mut()
            .playback
            .command(
                moli_animation::AnimationCommand::Seek(0.5),
                std::time::Instant::now(),
            )
            .unwrap();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        vm.eval("Math.round(document.getElementById('box').getBoundingClientRect().width)")
            .unwrap(),
        "60"
    );
}

#[test]
fn animation_and_effect_brands_do_not_accept_each_others_native_instance_id() {
    let mut vm = new_parsed_test_vm(
        "https://animation-core.test/brands",
        "<!doctype html><div id=box></div>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const a=document.getElementById('box').animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});
      a.pause();a.currentTime=250;let rejected=0,reads=0;
      const options={get duration(){reads++;return 2;}};
      for(const call of [()=>Animation.prototype.cancel.call(a.effect),()=>KeyframeEffect.prototype.updateTiming.call(a,options)]){
        try{call();}catch(error){if(error instanceof TypeError)rejected++;else throw error;}
      }
      return JSON.stringify([rejected,reads,a.playState,a.currentTime,getComputedStyle(document.getElementById('box')).opacity]);
    })()"#).unwrap(),r#"[2,0,"paused",250,"0.25"]"#);
}

#[tokio::test]
async fn foreign_effect_wrappers_keep_their_creation_realm_and_control_the_same_clock() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-core.test/foreign-effect",
        "<!doctype html><div id=box style='position:relative'></div><iframe id=child></iframe>",
        &loader,
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(vm.eval(r#"(() => {
      const child=document.getElementById('child').contentWindow,box=document.getElementById('box');
      const effect=new child.KeyframeEffect(box,[{left:'0px'},{left:'100px'}],{duration:2000,fill:'both'});
      const a=new Animation(effect);a.pause();a.currentTime=250;effect.updateTiming({duration:1000});
      return JSON.stringify([a.effect===effect,effect instanceof child.KeyframeEffect,effect.getTiming().duration,
        box.getAnimations()[0]===a,getComputedStyle(box).left]);
    })()"#).unwrap(),r#"[true,true,1000,true,"25px"]"#);
}
