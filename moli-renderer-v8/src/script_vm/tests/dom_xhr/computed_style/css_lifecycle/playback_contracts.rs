use super::*;

// The executor harness services rendering tasks explicitly. Browser-level
// differential cases use requestAnimationFrame; here each stage waits for its
// observable event and then verifies that draining adds no duplicate events.
#[tokio::test]
async fn css_timeline_detach_and_reattach_commit_cancel_and_restart() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    for transition in [false, true] {
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://animation-contract.test/timeline-events",
            "<!doctype html><style>@keyframes fixed{from,to{opacity:1}}#box{opacity:0;transition:opacity 100s linear}</style><div id=box></div>",
            &loader,
        );
        vm.eval(r#"
            globalThis.trace=[];
            globalThis.box=document.getElementById('box');
            for(const type of ['animationstart','animationcancel','transitionrun','transitionstart','transitioncancel'])
                box.addEventListener(type,event=>trace.push(event.type));
            getComputedStyle(box).opacity;
        "#).unwrap();
        vm.eval(if transition {
            "box.style.opacity='1';globalThis.a=box.getAnimations()[0]"
        } else {
            "box.style.animation='fixed 100s linear both';globalThis.a=box.getAnimations()[0]"
        })
        .unwrap();
        let started = if transition {
            "transitionrun|transitionstart"
        } else {
            "animationstart"
        };
        wait_for_css_events(&mut vm, &loader, "trace.join('|')", started).await;
        vm.eval("trace.length=0;a.timeline=null").unwrap();
        let cancelled = if transition {
            "transitioncancel"
        } else {
            "animationcancel"
        };
        wait_for_css_events(&mut vm, &loader, "trace.join('|')", cancelled).await;
        vm.eval("trace.length=0;a.timeline=document.timeline")
            .unwrap();
        wait_for_css_events(&mut vm, &loader, "trace.join('|')", started).await;
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
            .await
            .unwrap();
        assert_eq!(vm.eval("trace.join('|')").unwrap(), started);
    }
}

#[tokio::test]
async fn replaying_a_cancelled_transition_emits_run_before_start() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/transition-replay",
        "<!doctype html><div id=box style='opacity:0;transition:opacity 100s linear'></div>",
        &loader,
    );
    vm.eval(
        r#"
        globalThis.trace=[];
        const box=document.getElementById('box');
        for(const type of ['transitionrun','transitionstart','transitioncancel'])
            box.addEventListener(type,event=>trace.push(event.type));
        getComputedStyle(box).opacity;box.style.opacity='1';globalThis.a=box.getAnimations()[0];
    "#,
    )
    .unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "trace.join('|')",
        "transitionrun|transitionstart",
    )
    .await;
    vm.eval("trace.length=0;a.finished.catch(()=>{});a.cancel()")
        .unwrap();
    wait_for_css_events(&mut vm, &loader, "trace.join('|')", "transitioncancel").await;
    vm.eval("trace.length=0;a.play()").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "trace.join('|')",
        "transitionrun|transitionstart",
    )
    .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("trace.join('|')").unwrap(),
        "transitionrun|transitionstart"
    );
}

fn assert_sample(html: &str, script: &str, expected: &str) {
    let mut vm = new_parsed_test_vm("https://animation-contract.test/sampling", html);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let actual = vm.eval(script).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(expected).unwrap()
    );
}

pub(super) async fn assert_async_sample(html: &str, script: &str, expected: &str) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/async-sampling",
        html,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(script).unwrap();
    wait_for_css_events(&mut vm, &loader, "String(globalThis.result)", expected).await;
}

#[test]
fn zero_rate_start_hold() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},1000));a.currentTime=500;a.playbackRate=0;a.startTime=123;return[a.currentTime,a.startTime,a.playState];})())"###,
        r###"[500,123,"running"]"###,
    );
}

#[tokio::test]
async fn idle_cancel_finished() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},1000));const p=a.finished;let r="pending";p.then(()=>r="resolved",e=>r=e.name);a.cancel();await Promise.resolve();return[r,a.finished===p];})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"["pending",true]"###).await;
}

#[tokio::test]
async fn repeated_cancel_finished() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},1000);a.finished.catch(()=>{});a.cancel();const p=a.finished;let r="pending";p.then(()=>r="resolved",e=>r=e.name);a.cancel();await Promise.resolve();return[r,a.finished===p];})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"["pending",true]"###).await;
}

#[tokio::test]
async fn pending_cancel_ready() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},1000);a.finished.catch(()=>{});const p=a.ready;let r="pending";p.then(()=>r="resolved",e=>r=e.name);a.cancel();await Promise.resolve();return[r,a.ready===p];})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"["AbortError",false]"###).await;
}

#[test]
fn reverse_steps_4_end_() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},{duration:1000,delay:1000,fill:"both",direction:"reverse",easing:"steps(4,end)"});a.pause();a.currentTime=0;return getComputedStyle(el).opacity;})())"###,
        r###""1""###,
    );
}

#[test]
fn reverse_steps_4_start_() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},{duration:1000,delay:1000,fill:"both",direction:"reverse",easing:"steps(4,start)"});a.pause();a.currentTime=2500;return getComputedStyle(el).opacity;})())"###,
        r###""0""###,
    );
}

#[test]
fn zero_duration_normal() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},{duration:0,delay:1000,iterations:0.5,direction:"normal",fill:"both"});a.pause();a.currentTime=0;return getComputedStyle(el).opacity;})())"###,
        r###""0""###,
    );
}

#[test]
fn zero_duration_alternate() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},{duration:0,delay:1000,iterations:1,direction:"alternate",fill:"both"});a.pause();a.currentTime=0;return getComputedStyle(el).opacity;})())"###,
        r###""0""###,
    );
}

#[test]
fn finish_reverse_infinite() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},{duration:1000,iterations:Infinity});a.playbackRate=-1;a.currentTime=500;a.finish();return[a.currentTime,a.playState];})())"###,
        r###"[0,"finished"]"###,
    );
}

#[test]
fn unresolved_null_noop() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},1000),null);a.startTime=0;a.currentTime=null;return[a.currentTime,a.playState];})())"###,
        r###"[null,"running"]"###,
    );
}

#[tokio::test]
async fn idle_negative_pause() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},1000));a.playbackRate=-1;a.pause();await a.ready;return a.currentTime;})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"1000"###).await;
}

#[test]
fn idle_negative_infinite_pause() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},{duration:1000,iterations:Infinity}));a.playbackRate=-1;try{a.pause();return "no exception"}catch(e){return e.name}})())"###,
        r###""InvalidStateError""###,
    );
}

#[tokio::test]
async fn rewind_play_negative() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},1000);await a.ready;a.currentTime=-500;a.play();return a.currentTime;})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"0"###).await;
}

#[tokio::test]
async fn rewind_reverse_overflow() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},1000);await a.ready;a.currentTime=2000;a.reverse();return a.currentTime;})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"1000"###).await;
}

#[tokio::test]
async fn duration_shrink() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const a=el.animate({opacity:[0,1]},1000);await a.ready;a.currentTime=800;a.effect.updateTiming({duration:500});return a.currentTime;})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"800"###).await;
}

#[tokio::test]
async fn timeline_inactivation() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");const f=document.createElement("iframe");await new Promise(r=>{f.onload=r;f.srcdoc="<p>child</p>";document.body.append(f)});const a=new Animation(new KeyframeEffect(el,{opacity:[0,1]},10000),f.contentDocument.timeline);a.play();await a.ready;f.remove();return[a.timeline.currentTime,a.currentTime];})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###"[null,null]"###).await;
}
