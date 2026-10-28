use super::playback_contracts::assert_async_sample;
use super::*;

#[test]
fn animation_playback_event_has_nullable_readonly_native_payload() {
    let mut vm = new_parsed_test_vm(
        "https://animation-contract.test/playback-event",
        "<!doctype html>",
    );
    assert_eq!(vm.eval(r#"JSON.stringify((()=>{
        const order=[];const e=new AnimationPlaybackEvent('finish',{
            get bubbles(){order.push('bubbles');return true},
            get cancelable(){order.push('cancelable');return false},
            get composed(){order.push('composed');return true},
            get currentTime(){order.push('currentTime');return 1000},
            get timelineTime(){order.push('timelineTime');return 5},
        });
        const empty=new AnimationPlaybackEvent('cancel');
        e.currentTime=7;
        const getter=Object.getOwnPropertyDescriptor(AnimationPlaybackEvent.prototype,'currentTime').get;
        let illegal=false,nonfinite=false;
        try{getter.call(new Event('finish'))}catch(e){illegal=e instanceof TypeError}
        try{new AnimationPlaybackEvent('finish',{currentTime:Infinity})}catch(e){nonfinite=e instanceof TypeError}
        return[e instanceof Event,e.currentTime,e.timelineTime,e.bubbles,e.composed,e.isTrusted,
            empty.currentTime,empty.timelineTime,illegal,nonfinite,order];
    })())"#).unwrap(), r#"[true,1000,5,true,true,false,null,null,true,true,["bubbles","cancelable","composed","currentTime","timelineTime"]]"#);
}

#[test]
fn animation_handler_properties_share_the_standard_listener_registry() {
    let mut vm = new_parsed_test_vm(
        "https://animation-contract.test/handlers",
        "<!doctype html>",
    );
    assert_eq!(vm.eval(r#"JSON.stringify((()=>{
        const a=new Animation(),trace=[];
        const initial=[a.onfinish,a.oncancel];
        a.addEventListener('cancel',()=>trace.push('first'));
        a.oncancel=()=>trace.push('replaced');
        a.addEventListener('cancel',()=>trace.push('last'));
        a.oncancel=()=>trace.push('property');
        a.dispatchEvent(new Event('cancel'));a.oncancel=123;a.dispatchEvent(new Event('cancel'));
        const getter=Object.getOwnPropertyDescriptor(Animation.prototype,'oncancel').get;
        let illegal=false;try{getter.call(new EventTarget())}catch(e){illegal=e instanceof TypeError}
        return[initial,trace,a.oncancel,illegal,Object.hasOwn(a,'oncancel')];
    })())"#).unwrap(), r#"[[null,null],["first","property","last","first","last"],null,true,false]"#);
}

#[tokio::test]
async fn playback_events_keep_their_enqueue_time_after_seek_and_timeline_replacement() {
    assert_async_sample("<!doctype html><div id=box></div>", r#"
        globalThis.result=null;
        (async()=>{
            const box=document.getElementById('box');
            const a=box.animate({opacity:[0,1]},1000);
            const finished=new Promise(resolve=>a.onfinish=resolve);
            a.finish();a.currentTime=100;a.timeline=null;
            const finish=await finished;
            const b=box.animate({opacity:[0,1]},1000);
            let count=0;b.addEventListener('cancel',()=>++count);
            const cancelled=new Promise(resolve=>b.oncancel=resolve);
            b.cancel();b.timeline=null;
            const cancel=await cancelled;
            return[finish.constructor.name,finish.currentTime,Number.isFinite(finish.timelineTime),finish.target===a,finish.isTrusted,
                cancel.constructor.name,cancel.currentTime,Number.isFinite(cancel.timelineTime),cancel.target===b,cancel.isTrusted,count];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"["AnimationPlaybackEvent",1000,true,true,true,"AnimationPlaybackEvent",null,true,true,true,1]"#).await;
}

#[tokio::test]
async fn borrowed_playback_commands_keep_the_wrappers_promise_realm() {
    assert_async_sample("<!doctype html><div id=box></div>", r#"
        globalThis.result=null;
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><body></body>';document.body.append(f)});
            const w=f.contentWindow;
            const a=new w.Animation(new w.KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000),document.timeline);
            Animation.prototype.play.call(a);const ready=a.ready;
            Animation.prototype.finish.call(a);const finished=a.finished;
            Animation.prototype.pause.call(a);a.currentTime=100;const renewed=a.finished;
            return[ready.constructor===w.Promise,renewed.constructor===w.Promise,ready.constructor===Promise,renewed===finished];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"[true,true,false,false]"#).await;
}

#[tokio::test]
async fn retiring_a_creator_stops_its_promises_but_keeps_live_parent_event_handlers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/retired-creator-events",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(r#"
        globalThis.initialized=false;globalThis.result=null;
        globalThis.readyStatus='pending';globalThis.finishedStatus='pending';globalThis.count=0;
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><body></body>';document.body.append(f)});
            const w=f.contentWindow;
            globalThis.a=new w.Animation(new w.KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000),document.timeline);
            a.finished.then(()=>finishedStatus='resolved',e=>finishedStatus=e.name);
            a.pause();a.ready.then(()=>readyStatus='resolved',e=>readyStatus=e.name);
            a.addEventListener('finish',()=>++count);
            a.onfinish=e=>result=JSON.stringify([readyStatus,finishedStatus,count,e.currentTime,e.target===a,a.playState,a.currentTime]);
            f.remove();initialized=true;
        })();
    "#).unwrap();
    // The VM executor services rendering explicitly; wait for the pending pause
    // to commit after retirement, instead of awaiting a browser rAF callback.
    wait_for_css_events(
        &mut vm,
        &loader,
        "String(initialized && !a.pending)",
        "true",
    )
    .await;
    vm.eval("a.finish()").unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "String(result)",
        r#"["pending","pending",1,1000,true,"finished",1000]"#,
    )
    .await;
}
