use super::playback_contracts::assert_async_sample;

const HTML: &str = "<!doctype html><div id=box></div>";

#[tokio::test]
async fn play_pause_cancel_keeps_then_rejects_the_pending_ready() {
    assert_async_sample(HTML, r#"
        globalThis.result=null;
        (async()=>{
            const a=new Animation(new KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000));
            a.finished.catch(()=>{});
            a.play();const ready=a.ready;
            a.pause();const pending=[a.pending,a.playState,a.ready===ready];
            a.cancel();
            const outcome=await ready.then(()=> 'resolved',e=>e.name);
            await a.ready;
            return [pending,outcome,a.ready===ready,a.playState];
        })().then(v=>result=JSON.stringify(v));
    "#, r#"[[true,"paused",true],"AbortError",false,"idle"]"#).await;
}

#[tokio::test]
async fn pending_pause_switches_to_play_without_replacing_ready() {
    assert_async_sample(HTML, r#"
        globalThis.result=null;
        (async()=>{
            const a=new Animation(new KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000));
            a.pause();const ready=a.ready;
            const pause=[a.pending,a.playState];
            a.play();const play=[a.pending,a.playState,a.ready===ready];
            await ready;
            const done=[a.pending,a.playState,a.ready===ready];
            a.cancel();return [pause,play,done];
        })().then(v=>result=JSON.stringify(v));
    "#, r#"[[true,"paused"],[true,"running",true],[false,"running",true]]"#).await;
}

#[tokio::test]
async fn seek_completes_pending_pause_without_an_active_timeline() {
    assert_async_sample(HTML, r#"
        globalThis.result=null;
        (async()=>{
            const a=new Animation(new KeyframeEffect(document.getElementById('box'),{opacity:[0,1]},1000),null);
            const initial=a.ready;
            a.pause();const ready=a.ready;
            await Promise.resolve();
            const pending=[a.pending,a.playState,a.ready===initial];
            a.currentTime=500;
            const seek=[a.pending,a.currentTime,a.ready===ready];
            await ready;
            return [pending,seek];
        })().then(v=>result=JSON.stringify(v));
    "#, r#"[[true,"paused",false],[false,500,true]]"#).await;
}

#[tokio::test]
async fn running_pause_has_one_new_ready_and_one_completion() {
    assert_async_sample(
        HTML,
        r#"
        globalThis.result=null;
        (async()=>{
            const a=document.getElementById('box').animate({opacity:[0,1]},10000);
            await a.ready;const running=a.ready;
            a.pause();const ready=a.ready;
            const first=[a.pending,a.playState,ready===running];
            a.pause();const second=[a.pending,a.ready===ready];
            await ready;
            const done=[a.pending,a.playState,a.ready===ready];
            a.cancel();return [first,second,done];
        })().then(v=>result=JSON.stringify(v));
    "#,
        r#"[[true,"paused",false],[true,true],[false,"paused",true]]"#,
    )
    .await;
}

#[tokio::test]
async fn native_css_cancellation_rejects_the_pending_pause() {
    assert_async_sample("<!doctype html><style>@keyframes fade{from{opacity:0}to{opacity:1}}#box{animation:fade 100s linear}</style><div id=box></div>", r#"
        globalThis.result=null;
        (async()=>{
            const box=document.getElementById('box');
            const a=box.getAnimations()[0];
            await a.ready;
            a.pause();const ready=a.ready;
            box.style.animationName='none';
            getComputedStyle(box).opacity;
            const outcome=await ready.then(()=> 'resolved',e=>e.name);
            return [outcome,a.playState];
        })().then(v=>result=JSON.stringify(v));
    "#, r#"["AbortError","idle"]"#).await;
}

#[tokio::test]
async fn cancelled_promises_remain_rejected_without_unhandled_notifications() {
    use super::*;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://animation-contract.test/cancelled-promises",
        HTML,
        &loader,
    );
    vm.eval(
        r#"
        globalThis.unhandled=[];
        const ordinary=new Error('ordinary');
        window.addEventListener('unhandledrejection',e=>{
            unhandled.push(e.reason===ordinary?'ordinary':e.reason.name);
            e.preventDefault();
        });
        const a=document.getElementById('box').animate({opacity:[0,1]},1000);
        globalThis.old=[a.ready,a.finished];
        a.cancel();
        Promise.reject(ordinary);
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(vm.eval("unhandled.join('|')").unwrap(), "ordinary");
    vm.eval(
        r#"
        globalThis.result=null;
        Promise.all(old.map(p=>p.then(()=> 'resolved',e=>e.name)))
            .then(values=>result=values.join('|'));
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 64)
        .await
        .unwrap();
    assert_eq!(vm.eval("String(result)").unwrap(), "AbortError|AbortError");
}
