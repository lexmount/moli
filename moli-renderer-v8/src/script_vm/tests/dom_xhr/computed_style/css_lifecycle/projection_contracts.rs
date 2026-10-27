use super::*;

fn sample(html: &str, script: &str) -> String {
    let mut vm = new_parsed_test_vm("https://animation-projection.test/", html);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(script).unwrap()
}

#[test]
fn property_order_true() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({width:['1em','3em'],fontSize:['10px','30px']},{duration:1000,fill:'both'});a.pause();a.currentTime=500;return [getComputedStyle(box).fontSize,getComputedStyle(box).width,box.getBoundingClientRect().width]})())"###
        ),
        r###"["20px","40px",40]"###
    );
}

#[test]
fn effect_order_true() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({width:['1em','3em']},{duration:1000,fill:'both'}),b=box.animate({fontSize:['10px','30px']},{duration:1000,fill:'both'});for(const effect of [a,b]){effect.pause();effect.currentTime=500}return [getComputedStyle(box).fontSize,getComputedStyle(box).width]})())"###
        ),
        r###"["20px","40px"]"###
    );
}

#[test]
fn css_order_true() {
    assert_eq!(
        sample(
            r###"<!doctype html><style>@keyframes width{from{width:1em}to{width:3em}}@keyframes font{from{font-size:10px}to{font-size:30px}}#box{animation:width,font;animation-duration:1s;animation-fill-mode:both;animation-timing-function:linear;animation-play-state:paused}</style><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{for(const a of box.getAnimations())a.currentTime=500;return [getComputedStyle(box).fontSize,getComputedStyle(box).width]})())"###
        ),
        r###"["20px","40px"]"###
    );
}

#[test]
fn property_order_false() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({fontSize:['10px','30px'],width:['1em','3em']},{duration:1000,fill:'both'});a.pause();a.currentTime=500;return [getComputedStyle(box).fontSize,getComputedStyle(box).width,box.getBoundingClientRect().width]})())"###
        ),
        r###"["20px","40px",40]"###
    );
}

#[test]
fn effect_order_false() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({fontSize:['10px','30px']},{duration:1000,fill:'both'}),b=box.animate({width:['1em','3em']},{duration:1000,fill:'both'});for(const effect of [a,b]){effect.pause();effect.currentTime=500}return [getComputedStyle(box).fontSize,getComputedStyle(box).width]})())"###
        ),
        r###"["20px","40px"]"###
    );
}

#[test]
fn css_order_false() {
    assert_eq!(
        sample(
            r###"<!doctype html><style>@keyframes width{from{width:1em}to{width:3em}}@keyframes font{from{font-size:10px}to{font-size:30px}}#box{animation:font,width;animation-duration:1s;animation-fill-mode:both;animation-timing-function:linear;animation-play-state:paused}</style><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{for(const a of box.getAnimations())a.currentTime=500;return [getComputedStyle(box).fontSize,getComputedStyle(box).width]})())"###
        ),
        r###"["20px","40px"]"###
    );
}

#[test]
fn commit_full_context_false() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({fontSize:['10px','30px'],width:['1em','3em']},{duration:1000,fill:'both'});a.pause();a.currentTime=500;const before=getComputedStyle(box).width;a.commitStyles();const inline=[box.style.fontSize,box.style.width];a.cancel();return [before,inline,getComputedStyle(box).width]})())"###
        ),
        r###"["40px",["20px","40px"],"40px"]"###
    );
}

#[test]
fn commit_full_context_true() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="font-size:10px;width:2em;height:10px"></div>"###,
            r###"JSON.stringify((()=>{const a=box.animate({fontSize:['10px','30px'],width:['1em','3em']},{duration:1000,fill:'both'});a.pause();a.currentTime=500;const b=box.animate({fontSize:['100px','100px'],width:['700px','700px']},{duration:1000,fill:'both'});b.pause();b.currentTime=500;const before=getComputedStyle(box).width;a.commitStyles();const inline=[box.style.fontSize,box.style.width];a.cancel();b.cancel();return [before,inline,getComputedStyle(box).width]})())"###
        ),
        r###"["700px",["20px","40px"],"40px"]"###
    );
}

#[test]
fn sampled_axis_shorthand_background() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="background-position:0px 0px;transition:background-position 1s linear"></div>"###,
            r###"JSON.stringify((()=>{getComputedStyle(box).getPropertyValue('background-position');box.style.setProperty('background-position','100px 200px');for(const a of box.getAnimations()){a.pause();a.currentTime=500}const cs=getComputedStyle(box);return ['-position','-position-x','-position-y'].map(p=>cs.getPropertyValue('background'+p))})())"###
        ),
        r###"["50px 100px","50px","100px"]"###
    );
}

#[test]
fn sampled_axis_shorthand_mask() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box style="mask-position:0px 0px;transition:mask-position 1s linear"></div>"###,
            r###"JSON.stringify((()=>{getComputedStyle(box).getPropertyValue('mask-position');box.style.setProperty('mask-position','100px 200px');for(const a of box.getAnimations()){a.pause();a.currentTime=500}const cs=getComputedStyle(box);return ['-position'].map(p=>cs.getPropertyValue('mask'+p))})())"###
        ),
        r###"["50px 100px"]"###
    );
}

#[test]
fn css_animation_covered_value_does_not_create_a_transition() {
    assert_eq!(
        sample(
            r###"<!doctype html><style>@keyframes hold{from{opacity:1}to{opacity:1}}#box{opacity:1;animation:hold 10s linear both paused;transition:opacity 1s linear}</style><div id=box></div>"###,
            r###"JSON.stringify((()=>{const first=getComputedStyle(box).opacity;box.style.opacity='0';getComputedStyle(box).opacity;for(const a of box.getAnimations())if(a instanceof CSSTransition){a.pause();a.currentTime=500}return [first,getComputedStyle(box).opacity,box.getAnimations().filter(a=>a instanceof CSSTransition).length]})())"###
        ),
        r###"["1","1",0]"###
    );
}

#[tokio::test]
async fn clear_start_keeps_completed_time() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/clear_start_keeps_completed_time",
        r###"<!doctype html><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.animate({opacity:[0,1]},1000);a.finish();await box.animate([],{duration:100}).finished;const first=a.currentTime;a.startTime=null;return [first,a.currentTime,a.startTime,a.playState]})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(
        vm.eval("result").unwrap(),
        r###"[1000,1000,null,"paused"]"###
    );
}

#[test]
fn clear_start_keeps_unresolved_time() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box></div>"###,
            r###"JSON.stringify((()=>{const a=new Animation(new KeyframeEffect(box,{opacity:[0,1]},1000),null);a.startTime=0;const first=a.currentTime;a.startTime=null;return [first,a.currentTime,a.startTime,a.playState]})())"###
        ),
        r###"[null,null,null,"idle"]"###
    );
}

#[tokio::test]
async fn detach_explicit_finished_hold_false() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/detach_explicit_finished_hold_false",
        r###"<!doctype html><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;a.currentTime=2000;const first=[a.currentTime,a.playState];a.timeline=null;const second=[a.currentTime,a.playState];a.timeline=document.timeline;return [first,second,[a.currentTime,a.playState]]})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(
        vm.eval("result").unwrap(),
        r###"[[2000,"finished"],[null,"running"],[1000,"finished"]]"###
    );
}

#[tokio::test]
async fn detach_explicit_finished_hold_true() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/detach_explicit_finished_hold_true",
        r###"<!doctype html><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;a.pause();await a.ready;a.currentTime=2000;const first=[a.currentTime,a.playState];a.timeline=null;const second=[a.currentTime,a.playState];a.timeline=document.timeline;return [first,second,[a.currentTime,a.playState]]})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(
        vm.eval("result").unwrap(),
        r###"[[2000,"paused"],[2000,"paused"],[2000,"paused"]]"###
    );
}

#[tokio::test]
async fn transient_seek_does_not_publish_css_phases() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/transient_seek_does_not_publish_css_phases",
        r###"<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 1s linear both paused}</style><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.getAnimations()[0];a.currentTime=500;await box.animate([],{duration:20}).finished;const events=[];box.onanimationstart=e=>events.push([e.type,e.elapsedTime]);box.onanimationend=e=>events.push([e.type,e.elapsedTime]);a.currentTime=1000;a.currentTime=500;await box.animate([],{duration:20}).finished;return events})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(vm.eval("result").unwrap(), r###"[]"###);
}

#[test]
fn detached_document_timeline_is_inactive() {
    assert_eq!(
        sample(
            r###"<!doctype html><div id=box></div>"###,
            r###"JSON.stringify((()=>{const d=document.implementation.createHTMLDocument('detached');return [d.timeline.currentTime,document.timeline.currentTime!==null]})())"###
        ),
        r###"[null,true]"###
    );
}

#[tokio::test]
async fn iframe_timeline_uses_document_origin() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/iframe_timeline_uses_document_origin",
        r###"<!doctype html><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{await box.animate([],{duration:80}).finished;const frame=document.createElement('iframe');const loaded=new Promise(r=>frame.onload=r);frame.srcdoc='<!doctype html><body></body>';document.body.append(frame);await loaded;const parent=document.timeline.currentTime,child=frame.contentDocument.timeline.currentTime;return [typeof child==='number',parent-child>40]})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(vm.eval("result").unwrap(), r###"[true,true]"###);
}

#[test]
fn removed_iframe_timeline_becomes_inactive() {
    assert_eq!(
        sample(
            r###"<!doctype html><iframe id=frame></iframe>"###,
            r###"JSON.stringify((()=>{const t=frame.contentDocument.timeline;const first=t.currentTime!==null;frame.remove();return [first,t.currentTime]})())"###
        ),
        r###"[true,null]"###
    );
}

#[tokio::test]
async fn clipped_css_phase_elapsed_false() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/clipped_css_phase_elapsed_false",
        r###"<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 1s linear both paused}</style><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.getAnimations()[0];a.effect.updateTiming({endDelay:-500});a.currentTime=100;await box.animate([],{duration:20}).finished;const events=[];box.onanimationstart=e=>events.push([e.type,e.elapsedTime]);box.onanimationend=e=>events.push([e.type,e.elapsedTime]);a.currentTime=500;await box.animate([],{duration:20}).finished;return events})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(vm.eval("result").unwrap(), r###"[["animationend",0.5]]"###);
}

#[tokio::test]
async fn clipped_css_phase_elapsed_true() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-projection.test/clipped_css_phase_elapsed_true",
        r###"<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 1s linear both paused}</style><div id=box></div>"###,
        &loader,
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(r###"globalThis.result='pending';(async()=>{const a=box.getAnimations()[0];a.effect.updateTiming({endDelay:-500});a.currentTime=500;await box.animate([],{duration:20}).finished;const events=[];box.onanimationstart=e=>events.push([e.type,e.elapsedTime]);box.onanimationend=e=>events.push([e.type,e.elapsedTime]);a.currentTime=250;await box.animate([],{duration:20}).finished;return events})().then(value=>result=JSON.stringify(value));"###).unwrap();
    wait_for_css_events(&mut vm, &loader, "(result!=='pending').toString()", "true").await;
    assert_eq!(
        vm.eval("result").unwrap(),
        r###"[["animationstart",0.5]]"###
    );
}
