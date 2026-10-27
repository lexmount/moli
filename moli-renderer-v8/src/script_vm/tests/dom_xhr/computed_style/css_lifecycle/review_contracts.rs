use super::*;

fn sample(html: &str, script: &str) -> String {
    let mut vm = new_parsed_test_vm("https://animation-contract.test/", html);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(script).unwrap()
}

#[test]
fn sampled_font_recomputes_the_targets_own_relative_lengths() {
    for transition in [false, true] {
        let animation = if transition {
            "getComputedStyle(box).fontSize; box.style.fontSize='110px'; const a=box.getAnimations()[0];"
        } else {
            "const a=box.animate([{fontSize:'10px'},{fontSize:'110px'}],1000);"
        };
        let script = format!(
            r#"
            const box=document.getElementById('box');
            {animation}
            a.pause(); a.currentTime=250;
            const style=getComputedStyle(box);
            JSON.stringify([style.fontSize,style.width,style.height,
                style.paddingLeft,box.getBoundingClientRect().width]);
        "#
        );
        assert_eq!(
            sample(
                "<!doctype html><div id=box style='font-size:10px;width:2em;height:1em;padding-left:.5em;transition:font-size 1s linear'></div>",
                &script,
            ),
            r#"["35px","70px","35px","17.5px",87.5]"#,
            "transition={transition}"
        );
    }
}

#[test]
fn sampled_custom_property_recomputes_its_targets_dependents() {
    assert_eq!(
        sample(
            "<!doctype html><div id=box></div>",
            r#"
        CSS.registerProperty({name:'--size',syntax:'<length>',inherits:false,initialValue:'10px'});
        const box=document.getElementById('box');
        box.style.cssText='font-size:var(--size);width:2em;height:var(--size)';
        const a=box.animate([{'--size':'10px'},{'--size':'110px'}],1000);
        a.pause(); a.currentTime=250;
        const style=getComputedStyle(box);
        JSON.stringify([style.fontSize,style.width,style.height,box.getBoundingClientRect().width]);
    "#
        ),
        r#"["35px","70px","35px",70]"#
    );
}

#[test]
fn repeated_dependency_sampling_preserves_live_values_and_the_underlying_style() {
    assert_eq!(
        sample(
            "<!doctype html><div id=box style='font-size:110px;width:2em;height:1em'></div>",
            r#"
            const box=document.getElementById('box');
            const a=box.animate([{fontSize:'10px'},{fontSize:'110px'}],1000);
            a.pause();a.currentTime=250;
            const style=getComputedStyle(box);
            for(let i=0;i<1024;i++) {
                if(style.fontSize!=='35px'||style.width!=='70px')throw new Error('invalid sampled style');
            }
            a.cancel();
            JSON.stringify([style.fontSize,style.width,box.getBoundingClientRect().width]);
        "#,
        ),
        r#"["110px","220px",220]"#
    );
}

#[test]
fn css_default_easing_is_applied_to_each_keyframe_interval() {
    assert_eq!(
        sample(
            r#"<!doctype html><style>
        @keyframes pulse {0%{opacity:0}50%{opacity:1}100%{opacity:0}}
        #box {animation:pulse 1s steps(1,end) both paused}
        </style><div id=box></div>"#,
            r#"
        const box=document.getElementById('box'),a=box.getAnimations()[0];
        a.currentTime=750;
        JSON.stringify([getComputedStyle(box).opacity,a.effect.getTiming().easing]);
    "#
        ),
        r#"["1","linear"]"#
    );
}

#[test]
fn css_keyframe_easing_overrides_the_interval_default() {
    assert_eq!(
        sample(
            r#"<!doctype html><style>
        @keyframes fade {from{opacity:0;animation-timing-function:steps(1,end)}to{opacity:1}}
        #box {animation:fade 1s linear both paused}
        </style><div id=box></div>"#,
            r#"
        const box=document.getElementById('box'),a=box.getAnimations()[0];
        a.currentTime=500;
        JSON.stringify([getComputedStyle(box).opacity,a.effect.getTiming().easing]);
    "#
        ),
        r#"["0","linear"]"#
    );
}

#[test]
fn synthesized_css_keyframes_use_the_default_interval_easing() {
    assert_eq!(
        sample(
            r#"<!doctype html><style>
        @keyframes pulse {50%{opacity:1}100%{opacity:0}}
        #box {opacity:0;animation:pulse 1s steps(1,end) both paused}
        </style><div id=box></div>"#,
            r#"
        const box=document.getElementById('box'),a=box.getAnimations()[0];
        a.currentTime=250;const first=getComputedStyle(box).opacity;
        a.currentTime=750;JSON.stringify([first,getComputedStyle(box).opacity]);
    "#
        ),
        r#"["0","1"]"#
    );
}

#[tokio::test]
async fn running_reverse_settles_ready_without_replacing_an_existing_pending_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/ready",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval("globalThis.a=document.getElementById('box').animate([{opacity:0},{opacity:1}],60000)")
        .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    vm.eval(
        r#"
        globalThis.readySettled=false;
        const oldReady=a.ready;
        a.currentTime=30000; a.reverse();
        globalThis.pendingReady=a.ready;
        globalThis.replaced=oldReady!==pendingReady;
        a.reverse();
        globalThis.preserved=pendingReady===a.ready;
        pendingReady.then(()=>readySettled=true);
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([replaced,preserved,readySettled,a.pending])")
            .unwrap(),
        "[true,true,true,false]"
    );
}

#[tokio::test]
async fn reverse_end_delay_preserves_target_registration_until_no_longer_relevant() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/relevance",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.box=document.getElementById('box');
        globalThis.a=box.animate([{opacity:0},{opacity:1}],{duration:60000,endDelay:40000});
        a.finish(); a.reverse();
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([a.playState,box.getAnimations().includes(a)])")
            .unwrap(),
        r#"["running",true]"#
    );
    vm.eval("a.finish()").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([a.currentTime,a.playState,box.getAnimations().includes(a)])")
            .unwrap(),
        r#"[0,"finished",false]"#
    );
}

#[tokio::test]
async fn queued_finish_keeps_its_event_without_settling_a_new_finished_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/finished-generation",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.a=document.getElementById('box').animate([{opacity:0},{opacity:1}],1000);
        globalThis.finishEvents=0;a.onfinish=()=>finishEvents++;
        globalThis.oldFinished=a.finished;
        a.finish();a.pause();a.currentTime=100;
        globalThis.currentFinished=a.finished;
        globalThis.resolved=false;currentFinished.then(()=>resolved=true);
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(vm.eval("JSON.stringify([oldFinished!==currentFinished,resolved,currentFinished===a.finished,finishEvents,a.playState,a.currentTime])").unwrap(),
        r#"[true,false,true,1,"paused",100]"#);
    vm.eval("a.finish()").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([resolved,currentFinished===a.finished,finishEvents])")
            .unwrap(),
        "[true,true,2]"
    );
}

#[tokio::test]
async fn start_time_setter_resolves_the_existing_ready_on_a_detached_timeline() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/start-time",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(r#"
        globalThis.a=new Animation(new KeyframeEffect(document.getElementById('box'),[{opacity:0},{opacity:1}],1000),null);
        a.play();globalThis.ready=a.ready;globalThis.resolved=false;
        ready.then(()=>resolved=true);a.startTime=0;
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([resolved,a.pending,a.startTime,ready===a.ready])")
            .unwrap(),
        "[true,false,0,true]"
    );
}

#[test]
fn inactive_timeline_preserves_the_running_mapping_or_paused_hold() {
    assert_eq!(
        sample(
            "<!doctype html><div id=box></div>",
            r#"
        const a=document.getElementById('box').animate([{opacity:0},{opacity:1}],10000);
        a.startTime=document.timeline.currentTime-500;
        const start=a.startTime;
        a.timeline=null;const inactive=[a.currentTime,a.startTime===start,a.playState];
        a.timeline=document.timeline;
        a.pause();a.currentTime=500;a.timeline=null;
        const paused=[a.currentTime,a.startTime,a.playState];
        a.timeline=document.timeline;
        JSON.stringify([inactive,paused,a.currentTime,a.playState]);
    "#
        ),
        r#"[[null,true,"running"],[500,null,"paused"],500,"paused"]"#
    );
}

#[test]
fn implicit_keyframe_endpoints_use_the_live_underlying_value() {
    for frames in ["[{opacity:1}]", "[{opacity:1,offset:0}]"] {
        let script = format!(
            r#"
            const box=document.getElementById('box');
            const a=box.animate({frames},{{duration:1000,fill:'both'}});
            a.pause();a.currentTime=500;
            const first=getComputedStyle(box).opacity;
            box.style.opacity='.4';
            JSON.stringify([first,getComputedStyle(box).opacity]);
        "#
        );
        assert_eq!(
            sample(
                "<!doctype html><div id=box style='opacity:0'></div>",
                &script
            ),
            r#"["0.5","0.7"]"#
        );
    }
}

#[test]
fn implicit_endpoints_compose_through_lower_replace_effects_and_commit_styles() {
    assert_eq!(
        sample(
            "<!doctype html><div id=box style='opacity:0'></div>",
            r#"
        const box=document.getElementById('box');
        const lower=box.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});
        lower.pause();lower.currentTime=400;
        const upper=box.animate([{opacity:1}],{duration:1000,fill:'both'});
        upper.pause();upper.currentTime=500;
        const first=getComputedStyle(box).opacity;
        lower.currentTime=800;
        const second=getComputedStyle(box).opacity;
        upper.commitStyles();upper.cancel();lower.cancel();
        JSON.stringify([first,second,box.style.opacity]);
    "#
        ),
        r#"["0.7","0.9","0.9"]"#
    );
}

#[test]
fn transition_all_and_shorthands_select_timing_for_each_longhand() {
    for (declaration, expected) in [
        (
            "all 1s linear,margin 2s linear",
            r#"[["margin-bottom",2000],["margin-left",2000],["margin-right",2000],["margin-top",2000]]"#,
        ),
        (
            "margin 2s linear",
            r#"[["margin-bottom",2000],["margin-left",2000],["margin-right",2000],["margin-top",2000]]"#,
        ),
        (
            "margin 2s linear,margin-left 3s linear",
            r#"[["margin-bottom",2000],["margin-left",3000],["margin-right",2000],["margin-top",2000]]"#,
        ),
    ] {
        let html =
            format!("<!doctype html><div id=box style='margin:0;transition:{declaration}'></div>");
        assert_eq!(
            sample(
                &html,
                r#"
            const box=document.getElementById('box');getComputedStyle(box).margin;
            box.style.margin='10px';getComputedStyle(box).margin;
            JSON.stringify(box.getAnimations().map(a=>[a.transitionProperty,a.effect.getTiming().duration]).sort((a,b)=>a[0].localeCompare(b[0])));
        "#
            ),
            expected,
            "{declaration}"
        );
    }
    assert_eq!(
        sample(
            "<!doctype html><div id=box style='font-size:10px;transition:all 1s linear,font-size 2s linear'></div>",
            r#"
        const box=document.getElementById('box');getComputedStyle(box).fontSize;
        box.style.fontSize='30px';getComputedStyle(box).fontSize;
        JSON.stringify(box.getAnimations().filter(a=>a.transitionProperty==='font-size').map(a=>a.effect.getTiming().duration));
    "#
        ),
        "[2000]"
    );
}

#[test]
fn replacing_a_css_effect_takes_over_timing_but_update_timing_only_takes_its_fields() {
    const HTML: &str = "<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 10s linear both paused}</style><div id=box></div>";
    for replace in [false, true] {
        let change = if replace {
            "a.effect=new KeyframeEffect(box,[{opacity:0},{opacity:1}],{duration:2000,fill:'both'});a.effect.setKeyframes([{opacity:0},{opacity:1}]);"
        } else {
            "a.effect.updateTiming({duration:2000});"
        };
        let script = format!(
            r#"
            const box=document.getElementById('box'),a=box.getAnimations()[0];
            {change}
            box.style.animationDuration='3s';box.style.animationDelay='1s';
            getComputedStyle(box).opacity;
            JSON.stringify([a.effect.getTiming().duration,a.effect.getTiming().delay]);
        "#
        );
        assert_eq!(
            sample(HTML, &script),
            if replace { "[2000,0]" } else { "[2000,1000]" }
        );
    }
}

#[tokio::test]
async fn reverse_only_overrides_css_play_state_when_paused_state_changes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for paused in [false, true] {
        let html = format!(
            "<!doctype html><style>@keyframes fade{{from{{opacity:0}}to{{opacity:1}}}}#box{{animation:fade 60s linear both {}}}</style><div id=box></div>",
            if paused { "paused" } else { "running" }
        );
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://animation-contract.test/css-play-state",
            &html,
            &loader,
        );
        vm.eval(
            r#"
            globalThis.box=document.getElementById('box');globalThis.a=box.getAnimations()[0];
            a.currentTime=30000;a.reverse();
        "#,
        )
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
            .await
            .unwrap();
        assert_eq!(
            vm.eval(
                r#"
            box.style.animationPlayState='running';getComputedStyle(box).opacity;
            box.style.animationPlayState='paused';getComputedStyle(box).opacity;
            a.playState;
        "#
            )
            .unwrap(),
            if paused { "running" } else { "paused" }
        );
    }
}

#[test]
fn authored_font_relative_keyframes_are_resolved_at_sample_time() {
    for (frames, expected) in [
        ("[{width:'1em'},{width:'3em'}]", r#"["20px","40px",40]"#),
        ("[{width:'6em'}]", r#"["40px","80px",80]"#),
    ] {
        let script = format!(
            r#"
            const box=document.getElementById('box');
            const a=box.animate({frames},1000);a.pause();a.currentTime=500;
            const before=getComputedStyle(box).width;
            box.style.fontSize='20px';
            JSON.stringify([before,getComputedStyle(box).width,box.getBoundingClientRect().width]);
        "#
        );
        assert_eq!(
            sample(
                "<!doctype html><div id=box style='font-size:10px;width:2em'></div>",
                &script
            ),
            expected
        );
    }
}

#[test]
fn authored_variable_keyframes_continue_to_use_current_custom_properties() {
    assert_eq!(
        sample(
            "<!doctype html><div id=box style='--x:10px'></div>",
            r#"
        const box=document.getElementById('box');
        const a=box.animate([{width:'var(--x)'},{width:'calc(var(--x) * 3)'}],1000);
        a.pause();a.currentTime=500;
        const before=getComputedStyle(box).width;
        box.style.setProperty('--x','20px');
        JSON.stringify([before,getComputedStyle(box).width]);
    "#
        ),
        r#"["20px","40px"]"#
    );
}

#[tokio::test]
async fn an_unpublished_finish_does_not_queue_an_event_or_replace_a_pending_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for play in [false, true] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://animation-contract.test/pending-finish",
            "<!doctype html><div id=box></div>",
            &loader,
        );
        vm.eval(
            "globalThis.a=document.getElementById('box').animate([{opacity:0},{opacity:1}],1000)",
        )
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
            .await
            .unwrap();
        vm.eval(&format!(
            r#"
            globalThis.promise=a.finished;globalThis.resolved=false;globalThis.events=0;
            promise.then(()=>resolved=true);a.onfinish=()=>events++;
            a.currentTime=1000;{}a.pause();a.currentTime=100;
        "#,
            if play { "a.play();" } else { "" }
        ))
        .unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
            .await
            .unwrap();
        assert_eq!(
            vm.eval(
                "JSON.stringify([promise===a.finished,resolved,events,a.playState,a.currentTime])"
            )
            .unwrap(),
            r#"[true,false,0,"paused",100]"#,
            "play={play}"
        );
    }
}

#[tokio::test]
async fn finishing_an_inactive_timeline_does_not_settle_its_finished_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/inactive-finish",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(r#"
        globalThis.a=new Animation(new KeyframeEffect(document.getElementById('box'),[{opacity:0},{opacity:1}],1000),null);
        globalThis.resolved=false;globalThis.events=0;
        a.finished.then(()=>resolved=true);a.onfinish=()=>events++;a.finish();
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([a.currentTime,a.startTime,a.playState,resolved,events])")
            .unwrap(),
        r#"[1000,null,"paused",false,0]"#
    );
}
