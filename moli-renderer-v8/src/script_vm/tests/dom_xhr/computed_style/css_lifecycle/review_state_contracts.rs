use super::playback_contracts::assert_async_sample;
use super::*;

const HTML: &str = "<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}@keyframes fixed{from,to{opacity:.8}}</style><div id=box></div><div id=other></div>";

fn assert_sample(script: &str, expected: &str) {
    let mut vm = new_parsed_test_vm("https://animation-contract.test/state", HTML);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let actual = vm.eval(script).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(expected).unwrap(),
    );
}

#[test]
fn retained_effect_timing_reads_pending_css_without_an_element_read() {
    assert_sample(
        r#"JSON.stringify((()=>{
            const box=document.getElementById('box');
            box.style.animation='fade 1s linear both paused';
            const a=box.getAnimations()[0],effect=a.effect;
            box.style.animationDuration='2s';
            const duration=effect.getTiming().duration;
            return[duration,effect===a.effect];
        })())"#,
        "[2000,true]",
    );
}

#[test]
fn retained_css_play_state_reads_pending_start_and_removal() {
    assert_sample(
        r#"JSON.stringify((()=>{
            const box=document.getElementById('box');
            box.style.animation='fade 1s linear both paused';
            const a=box.getAnimations()[0];
            box.style.animationPlayState='running';
            const running=a.playState;
            box.style.animationName='none';
            return[running,a.playState];
        })())"#,
        r#"["running","idle"]"#,
    );
}

#[test]
fn retained_effect_flushes_its_css_owner_after_retargeting_or_detaching() {
    for target in ["document.getElementById('other')", "null"] {
        assert_sample(
            &format!(
                r#"JSON.stringify((()=>{{
                const box=document.getElementById('box');
                box.style.animation='fade 1s linear both paused';
                const a=box.getAnimations()[0],effect=a.effect;
                effect.target={target};
                box.style.animationDuration='2s';
                const duration=effect.getTiming().duration;
                box.style.animationName='none';
                return[duration,a.playState];
            }})())"#
            ),
            r#"[2000,"idle"]"#,
        );
    }
}

#[tokio::test]
async fn borrowed_effect_reads_flush_the_css_owner_in_its_original_document() {
    assert_async_sample(HTML, r#"
        globalThis.result=null;
        (async()=>{
            const frame=document.createElement('iframe');
            await new Promise(resolve=>{frame.onload=resolve;frame.srcdoc='<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}</style><div id=owner></div>';document.body.append(frame)});
            const owner=frame.contentDocument.getElementById('owner');
            owner.style.animation='fade 1s linear both paused';
            const a=owner.getAnimations()[0],effect=a.effect;
            effect.target=document.getElementById('box');
            owner.style.animationDuration='2s';
            const duration=KeyframeEffect.prototype.getTiming.call(effect).duration;
            owner.style.animationName='none';
            return[duration,a.playState,effect===a.effect];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"[2000,"idle",true]"#).await;
}

#[test]
fn css_timing_reads_preserve_explicit_script_overrides() {
    assert_sample(
        r#"JSON.stringify((()=>{
            const box=document.getElementById('box');
            box.style.animation='fade 1s linear both paused';
            const a=box.getAnimations()[0],effect=a.effect;
            effect.updateTiming({duration:3000});
            box.style.animationDuration='2s';
            return[effect.getTiming().duration,effect.getComputedTiming().activeDuration];
        })())"#,
        "[3000,3000]",
    );
}

#[test]
fn replayed_orphan_css_animation_joins_global_order_for_rendering_and_lists() {
    for css_first in [false, true] {
        for retarget in [false, true] {
            assert_sample(
                &format!(
                    r#"JSON.stringify((()=>{{
                    const box=document.getElementById('box'),other=document.getElementById('other');
                    const target={retarget}?other:box;
                    const makeScript=()=>{{const a=target.animate({{opacity:[.2,.2]}},{{duration:1000,fill:'both'}});a.pause();a.currentTime=500;return a}};
                    let script;
                    if(!{css_first})script=makeScript();
                    box.style.animation='fixed 1s linear both paused';
                    const retained=box.getAnimations().find(a=>a instanceof CSSAnimation);
                    if({css_first})script=makeScript();
                    box.style.animation='none';getComputedStyle(box).opacity;
                    if({retarget})retained.effect.target=other;
                    retained.play();retained.pause();retained.currentTime=500;
                    const label=a=>a===retained?'retained':a===script?'script':'later';
                    const replay=[+getComputedStyle(target).opacity,target.getAnimations().map(label)];
                    const later=makeScript();
                    retained.cancel();retained.play();retained.pause();retained.currentTime=500;
                    return[retained instanceof CSSAnimation,replay,+getComputedStyle(target).opacity,target.getAnimations().map(a=>a===later?'later':label(a))];
                }})())"#
                ),
                r#"[true,[0.8,["script","retained"]],0.2,["script","retained","later"]]"#,
            );
        }
    }
}

#[test]
fn commit_styles_endpoint_inclusion_only_applies_to_the_committing_effect() {
    for (lower_time, expected) in [(999, 0.8995), (1000, 0.5), (1001, 0.5)] {
        assert_sample(
            &format!(
                r#"JSON.stringify((()=>{{
                const box=document.getElementById('box');box.style.opacity='.2';
                const lower=box.animate({{opacity:[0,1]}},1000);lower.pause();lower.currentTime={lower_time};
                const upper=box.animate({{opacity:['.8']}},{{duration:2000,fill:'both'}});upper.pause();upper.currentTime=1000;
                const higher=box.animate({{opacity:[.1,.1]}},{{duration:1000,fill:'both'}});higher.pause();higher.currentTime=500;
                upper.commitStyles();const committed=+box.style.opacity;
                lower.cancel();upper.cancel();higher.cancel();
                return[committed,+getComputedStyle(box).opacity];
            }})())"#
            ),
            &format!("[{expected},{expected}]"),
        );
    }
}

#[tokio::test]
async fn zero_rate_play_renews_ready_and_cancel_rejects_the_new_pending_promise() {
    assert_async_sample(
        HTML,
        r#"
        globalThis.result=null;
        (async()=>{
            const box=document.getElementById('box'),out=[];
            for(const rate of [0,1]){
                const a=box.animate({opacity:[0,1]},{duration:1000,fill:'both'});await a.ready;
                a.playbackRate=rate;a.currentTime=500;
                const oldReady=a.ready;a.play();
                const state=[a.ready===oldReady,a.pending,a.startTime===null,a.currentTime];
                const result=a.ready.then(()=> 'resolved',error=>error.name);
                a.cancel();out.push([state,await result]);
            }
            return out;
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#,
        r#"[[[false,true,true,500],"AbortError"],[[true,false,false,500],"resolved"]]"#,
    )
    .await;
}
