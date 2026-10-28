use super::playback_contracts::assert_async_sample;
use super::*;

fn assert_sample(script: &str, expected: &str) {
    let mut vm = new_parsed_test_vm(
        "https://animation-contract.test/review-followups",
        "<!doctype html><div id=box></div>",
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    assert_eq!(vm.eval(script).unwrap(), expected);
}

#[tokio::test]
async fn cancel_replay_css_cancel_settles_the_new_promises_and_keeps_both_events() {
    assert_async_sample("<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 100s linear both paused}</style><div id=box></div>", r#"
        globalThis.result=null;
        (async()=>{
            const box=document.getElementById('box'),a=box.getAnimations()[0];
            await a.ready;
            let cancels=0;
            const cancelled=new Promise(resolve=>a.addEventListener('cancel',()=>{if(++cancels===2)resolve()}));
            a.cancel();a.play();
            const ready=a.ready.then(()=> 'resolved',e=>e.name);
            const finished=a.finished.then(()=> 'resolved',e=>e.name);
            box.style.animation='none';getComputedStyle(box).opacity;
            const settled=await Promise.all([ready,finished]);
            await cancelled;
            return [...settled,a.playState,a.pending,cancels];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"["AbortError","AbortError","idle",false,2]"#).await;
}

#[test]
fn shorthand_variables_resolve_dynamically_with_explicit_longhand_precedence() {
    assert_sample(
        r#"JSON.stringify((()=>{
        const box=document.getElementById('box');box.style.cssText='--m:10px 20px;margin:0';
        const a=box.animate({margin:['var(--m)','var(--m)'],marginLeft:['90px','90px']},{duration:1000,fill:'both'});
        a.pause();a.currentTime=500;
        const sample=()=>{const s=getComputedStyle(box);return[s.marginTop,s.marginRight,s.marginBottom,s.marginLeft]};
        const values=[sample()];
        for(const value of ['30px','30px 50px']){box.style.setProperty('--m',value);values.push(sample())}
        a.commitStyles();a.cancel();values.push(sample());return values;
    })())"#,
        r#"[["10px","20px","10px","90px"],["30px","30px","30px","90px"],["30px","50px","30px","90px"],["30px","50px","30px","90px"]]"#,
    );
}

#[test]
fn css_duplicate_offsets_merge_without_merging_waapi_frames() {
    assert_sample(
        r#"JSON.stringify((()=>{
        document.head.appendChild(document.createElement('style')).textContent='@keyframes duplicate{from{opacity:0}to{opacity:.4}to{opacity:1}}';
        const box=document.getElementById('box');box.style.animation='duplicate 1s linear both paused';
        let a=box.getAnimations()[0];a.currentTime=500;const css=getComputedStyle(box).opacity;
        a.cancel();box.style.animation='none';
        a=box.animate([{offset:0,opacity:0},{offset:1,opacity:.4},{offset:1,opacity:1}],{duration:1000,fill:'both'});
        a.pause();a.currentTime=500;return[css,getComputedStyle(box).opacity];
    })())"#,
        r#"["0.5","0.2"]"#,
    );
}

#[test]
fn css_keyframe_shorthand_variables_keep_their_pending_substitutions() {
    assert_sample(
        r#"JSON.stringify((()=>{
        document.head.appendChild(document.createElement('style')).textContent='@keyframes variable{from,to{margin:var(--m)}}';
        const box=document.getElementById('box');box.style.cssText='--m:10px 20px;margin:0;animation:variable 1s linear both paused';
        const a=box.getAnimations()[0];a.currentTime=500;
        const before=getComputedStyle(box).marginLeft;box.style.setProperty('--m','30px');
        return[before,getComputedStyle(box).marginLeft];
    })())"#,
        r#"["20px","30px"]"#,
    );
}

#[tokio::test]
async fn retiring_only_the_creator_document_keeps_the_animation_and_target_alive() {
    assert_async_sample("<!doctype html><div id=box style='opacity:.2'></div>", r#"
        globalThis.result=null;
        (async()=>{
            const box=document.getElementById('box'),f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><body></body>';document.body.append(f)});
            const w=f.contentWindow;
            const a=new w.Animation(new w.KeyframeEffect(box,{opacity:[0,1]},{duration:1000,fill:'both'}),document.timeline);
            a.pause();a.currentTime=500;f.remove();
            // Observe the retained state in the next task, after retirement.
            await new Promise(resolve=>setTimeout(resolve,0));
            return[a.playState,a.currentTime,getComputedStyle(box).opacity,box.getAnimations().length];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"["paused",500,"0.5",1]"#).await;
}

#[tokio::test]
async fn retiring_the_creator_keeps_native_pause_but_stops_its_ready_notification() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/retired-creator-notifications",
        "<!doctype html><div id=box></div>",
        &loader,
    );
    vm.eval(r#"
        globalThis.initialized=false;globalThis.readyStatus='pending';
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><body></body>';document.body.append(f)});
            const w=f.contentWindow;
            globalThis.a=new w.Animation(new w.KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000),document.timeline);
            a.pause();a.ready.then(()=>readyStatus='resolved',e=>readyStatus=e.name);
            f.remove();initialized=true;
        })();
    "#).unwrap();
    wait_for_css_events(
        &mut vm,
        &loader,
        "String(initialized && a.pending === false)",
        "true",
    )
    .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 32)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([a.pending,a.playState,a.currentTime,readyStatus])")
            .unwrap(),
        r#"[false,"paused",0,"pending"]"#
    );
}

#[tokio::test]
async fn playback_rate_completes_pause_on_an_inactive_associated_timeline() {
    assert_async_sample("<!doctype html><div id=box></div>", r#"
        globalThis.result=null;
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><body></body>';document.body.append(f)});
            const a=new Animation(new KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000),f.contentDocument.timeline);
            a.pause();const ready=a.ready;f.remove();
            const before=[a.currentTime,a.pending,a.playState,a.timeline.currentTime];
            a.playbackRate=2;await ready;
            return[before,a.currentTime,a.pending,a.playState,a.ready===ready];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"[[0,true,"paused",null],0,false,"paused",true]"#).await;
}
