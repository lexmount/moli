use super::*;

#[test]
fn auto_width_does_not_transition() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/auto_width_does_not_transition",
        r###"<!doctype html><div style="width:300px"><div id=box style="width:auto;height:20px;transition:width 1s linear"></div></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');getComputedStyle(b).width;b.style.width='100px';const list=b.getAnimations();for(const a of list){a.pause();a.currentTime=250}return [list.length,getComputedStyle(b).width,b.getBoundingClientRect().width];})())"###).unwrap();
    assert_eq!(result, r###"[0,"100px",100]"###);
}

#[test]
fn numeric_width_transitions() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/numeric_width_transitions",
        r###"<!doctype html><div style="width:300px"><div id=box style="width:50px;height:20px;transition:width 1s linear"></div></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');getComputedStyle(b).width;b.style.width='100px';const list=b.getAnimations();for(const a of list){a.pause();a.currentTime=250}return [list.length,getComputedStyle(b).width,b.getBoundingClientRect().width];})())"###).unwrap();
    assert_eq!(result, r###"[1,"62.5px",62.5]"###);
}

#[test]
fn position_transition_normal() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/position_transition_normal",
        r###"<!doctype html><div id=box style="transition:position 1s linear;transition-behavior:normal"></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');getComputedStyle(b).position;b.style.position='absolute';const list=b.getAnimations();for(const a of list){a.pause();a.currentTime=250}return [list.length,getComputedStyle(b).position];})())"###).unwrap();
    assert_eq!(result, r###"[0,"absolute"]"###);
}

#[test]
fn position_transition_allow_discrete() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/position_transition_allow_discrete",
        r###"<!doctype html><div id=box style="transition:position 1s linear;transition-behavior:allow-discrete"></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');getComputedStyle(b).position;b.style.position='absolute';const list=b.getAnimations();for(const a of list){a.pause();a.currentTime=250}return [list.length,getComputedStyle(b).position];})())"###).unwrap();
    assert_eq!(result, r###"[1,"static"]"###);
}

#[test]
fn sampled_parent_drives_inheritance_and_relative_units() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/sampled_parent_drives_inheritance_and_relative_units",
        r###"<!doctype html><div id=box style="font-size:10px;color:rgb(0,0,0);transition:font-size 1s linear,color 1s linear"><span id=child>text<span id=grand style="font-size:2em;display:inline-block;width:2em">x</span></span><span id=override style="font-size:17px;color:rgb(1,2,3)">x</span></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),c=document.getElementById('child'),g=document.getElementById('grand'),o=document.getElementById('override');getComputedStyle(b).fontSize;b.style.fontSize='110px';b.style.color='rgb(200,200,200)';for(const a of b.getAnimations()){a.pause();a.currentTime=250}return [getComputedStyle(b).fontSize,getComputedStyle(b).color,getComputedStyle(c).fontSize,getComputedStyle(c).color,getComputedStyle(g).fontSize,getComputedStyle(g).width,g.getBoundingClientRect().width,getComputedStyle(o).fontSize,getComputedStyle(o).color];})())"###).unwrap();
    assert_eq!(
        result,
        r###"["35px","rgb(50, 50, 50)","35px","rgb(50, 50, 50)","70px","140px",140,"17px","rgb(1, 2, 3)"]"###
    );
}

#[test]
fn keyframe_offsets_interpolate_adjacent_explicit_offsets() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/keyframe_offsets_interpolate_adjacent_explicit_offsets",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');const a=b.animate([{opacity:0,offset:0},{opacity:0,offset:.6},{opacity:1,offset:null},{opacity:1,offset:1}],{duration:1000,fill:'both'});a.pause();a.currentTime=700;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0.5""###);
}

#[test]
fn keyframe_easing_applies_within_selected_interval() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/keyframe_easing_applies_within_selected_interval",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');const a=b.animate([{opacity:0,easing:'steps(1,end)'},{opacity:1}],{duration:1000,fill:'both'});a.pause();a.currentTime=500;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0""###);
}

#[test]
fn invalid_keyframe_offset_order_preserves_old_effect() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/invalid_keyframe_offset_order_preserves_old_effect",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');const a=b.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});a.pause();a.currentTime=500;let error='';try{a.effect.setKeyframes([{opacity:0,offset:.8},{opacity:1,offset:.2}])}catch(e){error=e.name}return [error,getComputedStyle(b).opacity];})())"###).unwrap();
    assert_eq!(result, r###"["TypeError","0.5"]"###);
}

#[test]
fn invalid_keyframe_easing_preserves_old_effect() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/invalid_keyframe_easing_preserves_old_effect",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');const a=b.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});a.pause();a.currentTime=500;let error='';try{a.effect.setKeyframes([{opacity:0,easing:"invalid"},{opacity:1}])}catch(e){error=e.name}return [error,getComputedStyle(b).opacity];})())"###).unwrap();
    assert_eq!(result, r###"["TypeError","0.5"]"###);
}

#[test]
fn commit_styles_through_a() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/commit_styles_through_a",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const box=document.getElementById('box'),a=box.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'}),b=box.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});a.pause();b.pause();a.currentTime=200;b.currentTime=800;a.commitStyles();const inline=box.style.opacity;a.cancel();b.cancel();return [inline,getComputedStyle(box).opacity];})())"###).unwrap();
    assert_eq!(result, r###"["0.2","0.2"]"###);
}

#[test]
fn commit_styles_through_b() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/commit_styles_through_b",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const box=document.getElementById('box'),a=box.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'}),b=box.animate([{opacity:0},{opacity:1}],{duration:1000,fill:'both'});a.pause();b.pause();a.currentTime=200;b.currentTime=800;b.commitStyles();const inline=box.style.opacity;a.cancel();b.cancel();return [inline,getComputedStyle(box).opacity];})())"###).unwrap();
    assert_eq!(result, r###"["0.8","0.8"]"###);
}

#[test]
fn document_animation_time_is_stable_during_javascript() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/document_animation_time_is_stable_during_javascript",
        r###"<!doctype html><style>@keyframes move{from{left:0px}to{left:100px}}#box{position:relative;animation:move 1s linear infinite}</style><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),a=b.getAnimations()[0];const t=document.timeline.currentTime,c=a.currentTime,left=getComputedStyle(b).left;const end=performance.now()+20;while(performance.now()<end){}return [t===document.timeline.currentTime,c===a.currentTime,left===getComputedStyle(b).left];})())"###).unwrap();
    assert_eq!(result, r###"[true,true,true]"###);
}

#[tokio::test]
async fn completed_transition_cleanup_does_not_dispatch_cancel() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for cleanup in [
        "box.style.display='none'",
        "box.remove()",
        "container.style.display='none'",
    ] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://animation-review.test/completed-cleanup",
            "<!doctype html><div id=parent><div id=box style='opacity:0;transition:opacity 1s linear'></div></div>",
            &loader,
        );
        vm.eval(r#"
            globalThis.events=[];
            globalThis.box=document.getElementById('box');globalThis.container=document.getElementById('parent');
            for(const type of ['transitionrun','transitionstart','transitionend','transitioncancel']) {
                box.addEventListener(type,e=>events.push(e.type));
            }
            getComputedStyle(box).opacity;box.style.opacity=1;
            globalThis.a=box.getAnimations()[0];
        "#).unwrap();
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        vm.eval(&format!(
            "box.addEventListener('transitionend',()=>{{ {cleanup} }});a.finish()"
        ))
        .unwrap();
        wait_for_css_events(
            &mut vm,
            &loader,
            "events.join('|')",
            "transitionrun|transitionstart|transitionend",
        )
        .await;
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["transitionrun","transitionstart","transitionend"]"#,
            "{cleanup}"
        );
    }
}

#[test]
fn document_animation_time_is_stable_in_promise_callbacks() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/microtask-clock",
        "<!doctype html><div id=box></div>",
    );
    vm.eval(
        r#"
        globalThis.result=null;
        Promise.resolve().then(()=>{
            const first=document.timeline.currentTime;
            // Deliberate script execution time, not a readiness wait.
            const end=performance.now()+20;while(performance.now()<end){}
            result=first===document.timeline.currentTime;
        });
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("result").unwrap(), "true");
}

#[test]
fn property_indexed_complete_offsets() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/property_indexed_complete_offsets",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),a=b.animate({opacity:[0,0,1,1],offset:[0,.6,null,1]},{duration:1000,fill:'both'});a.pause();a.currentTime=700;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0.5""###);
}

#[test]
fn property_indexed_short_offsets() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/property_indexed_short_offsets",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),a=b.animate({opacity:[0,0,1,1],offset:[0,.6]},{duration:1000,fill:'both'});a.pause();a.currentTime=700;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0.5""###);
}

#[test]
fn property_indexed_frame_easing() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/property_indexed_frame_easing",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),a=b.animate({opacity:[0,1],easing:['steps(1,end)','linear']},{duration:1000,fill:'both'});a.pause();a.currentTime=500;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0""###);
}

#[test]
fn internal_boundary_uses_following_keyframe_easing() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/internal_boundary_uses_following_keyframe_easing",
        r###"<!doctype html><div id=box></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box'),a=b.animate([{opacity:0,offset:0},{opacity:.5,offset:.5,easing:'steps(1,start)'},{opacity:1,offset:1}],{duration:1000,fill:'both'});a.pause();a.currentTime=500;return getComputedStyle(b).opacity;})())"###).unwrap();
    assert_eq!(result, r###""1""###);
}

#[test]
fn inline_ancestor_animation_is_admitted_from_child_read() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/inline_ancestor_animation_is_admitted_from_child_read",
        r###"<!doctype html><style>@keyframes grow{from{font-size:10px}to{font-size:110px}}</style><div id=box style="animation:grow 1s linear both paused;animation-delay:-.25s"><span id=child>x</span></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const child=document.getElementById('child');return getComputedStyle(child).fontSize;})())"###).unwrap();
    assert_eq!(result, r###""35px""###);
}

#[test]
fn registered_number_transition_keeps_typed_interpolation() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/registered_number_transition_keeps_typed_interpolation",
        r###"<!doctype html><style>@property --amount {syntax:"<number>";inherits:false;initial-value:0}</style><div id=box style="--amount:0;transition:--amount 1s linear"></div>"###,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let result = vm.eval(r###"JSON.stringify((()=>{const b=document.getElementById('box');getComputedStyle(b).getPropertyValue('--amount');b.style.setProperty('--amount','100');const animations=b.getAnimations();for(const a of animations){a.pause();a.currentTime=250}return [animations.length,getComputedStyle(b).getPropertyValue('--amount')];})())"###).unwrap();
    assert_eq!(result, r###"[1,"25"]"###);
}

#[test]
fn paused_css_admission_uses_the_script_sample_time() {
    let mut vm = new_parsed_test_vm(
        "https://animation-review.test/paused_css_admission_uses_the_script_sample_time",
        r###"<!doctype html><div id=box></div>"###,
    );
    let result = vm.eval(r###"JSON.stringify((()=>{const end=performance.now()+20;while(performance.now()<end){}const style=document.createElement('style');style.textContent='@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 1s -.5s linear paused}';document.head.append(style);return getComputedStyle(document.getElementById('box')).opacity;})())"###).unwrap();
    assert_eq!(result, r###""0.5""###);
}
