use super::playback_contracts::assert_async_sample;

#[tokio::test]
async fn a_retained_css_animation_can_leave_its_retired_document() {
    assert_async_sample("<!doctype html><div id=box style='opacity:.2'></div>", r#"
        globalThis.result=null;
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#x{animation:fade 1s linear both paused}</style><div id=x></div>';document.body.append(f)});
            const a=f.contentDocument.getElementById('x').getAnimations()[0];
            f.remove();await new Promise(resolve=>setTimeout(resolve,0));
            const before=[a.playState,a.currentTime];
            const box=document.getElementById('box');a.effect.target=box;a.timeline=document.timeline;
            a.play();a.pause();a.currentTime=500;
            return[before,a.playState,a.currentTime,getComputedStyle(box).opacity,box.getAnimations().length,a.effect.target===box];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"[["idle",null],"paused",500,"0.5",1,true]"#).await;
}

#[tokio::test]
async fn a_retained_css_transition_can_leave_its_retired_document() {
    assert_async_sample("<!doctype html><div id=box style='opacity:.2'></div>", r#"
        globalThis.result=null;
        (async()=>{
            const f=document.createElement('iframe');
            await new Promise(resolve=>{f.onload=resolve;f.srcdoc='<!doctype html><div id=x style="opacity:0;transition:opacity 1s linear"></div>';document.body.append(f)});
            const x=f.contentDocument.getElementById('x');getComputedStyle(x).opacity;x.style.opacity='1';
            const a=x.getAnimations()[0];
            f.remove();await new Promise(resolve=>setTimeout(resolve,0));
            const before=[a.playState,a.currentTime];
            const box=document.getElementById('box');a.effect.target=box;a.timeline=document.timeline;
            a.play();a.pause();a.currentTime=500;
            return[before,a.playState,a.currentTime,getComputedStyle(box).opacity,box.getAnimations().length];
        })().then(v=>result=JSON.stringify(v),e=>result=String(e));
    "#, r#"[["idle",null],"paused",500,"0.5",1]"#).await;
}
