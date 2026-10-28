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
