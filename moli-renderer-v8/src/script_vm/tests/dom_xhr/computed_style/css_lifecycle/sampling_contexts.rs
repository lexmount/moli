use super::*;

fn assert_sample(html: &str, script: &str, expected: &str) {
    let mut vm = new_parsed_test_vm("https://animation-contract.test/sampling", html);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    let actual = vm.eval(script).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
        serde_json::from_str::<serde_json::Value>(expected).unwrap()
    );
}

async fn assert_async_sample(html: &str, script: &str, expected: &str) {
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
fn commit_endpoint_999() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.opacity=".2";const a=el.animate({opacity:[0,1]},1000);a.pause();a.currentTime=999;a.commitStyles();return el.style.opacity;})())"###,
        r###""0.999""###,
    );
}

#[test]
fn commit_endpoint_1000() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.opacity=".2";const a=el.animate({opacity:[0,1]},1000);a.pause();a.currentTime=1000;a.commitStyles();return el.style.opacity;})())"###,
        r###""1""###,
    );
}

#[test]
fn commit_endpoint_1001() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.opacity=".2";const a=el.animate({opacity:[0,1]},1000);a.pause();a.currentTime=1001;a.commitStyles();return el.style.opacity;})())"###,
        r###""0.2""###,
    );
}

#[tokio::test]
async fn commit_natural_finish() {
    assert_async_sample(r###"<!doctype html><style></style><div id="box"></div>"###, r###"globalThis.result=null; (async()=>{const el=document.getElementById("box");el.style.opacity=".2";const a=el.animate({opacity:[0,1]},1);await a.finished;a.commitStyles();return el.style.opacity;})().then(value=>result=JSON.stringify(value),error=>result=JSON.stringify({error:error.name,message:error.message}));"###, r###""1""###).await;
}

#[test]
fn pseudo_inheritance() {
    assert_sample(
        r###"<!doctype html><style>#box{font-size:10px;color:black}#box::before{content:"XX";font-size:inherit;color:inherit}</style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({fontSize:["10px","30px"],color:["black","rgb(200,0,0)"]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;const s=getComputedStyle(el,"::before");return[s.fontSize,s.color];})())"###,
        r###"["20px","rgb(100, 0, 0)"]"###,
    );
}

#[test]
fn animation_admission() {
    assert_sample(
        r###"<!doctype html><style>#box{opacity:0;transition:opacity 1s linear}@keyframes fixed{from,to{opacity:1}}</style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");getComputedStyle(el).opacity;el.style.animation="fixed 10s linear both paused";return[getComputedStyle(el).opacity,el.getAnimations().map(a=>a.constructor.name)];})())"###,
        r###"["1",["CSSAnimation"]]"###,
    );
}

#[test]
fn pseudo_layout_inherits_the_same_sample_as_cssom() {
    assert_sample(
        r#"<!doctype html><style>#box{display:inline-block;font-size:10px;font-family:monospace}#box::before{content:'XX';font-size:inherit}</style><div id=box></div>"#,
        r#"JSON.stringify((()=>{
            const box=document.getElementById('box');
            const a=box.animate({fontSize:['10px','30px']},{duration:1000,fill:'both'});
            a.pause();a.currentTime=500;
            const animated=box.getBoundingClientRect().width;
            a.cancel();box.style.fontSize='20px';
            const fixed=box.getBoundingClientRect().width;
            return animated>0 && Math.abs(animated-fixed)<.01;
        })())"#,
        "true",
    );
}

#[test]
fn commit_longhands_updates_native_style_once() {
    assert_sample(
        r###"<!doctype html><div></div>"###,
        r###"JSON.stringify((()=>{
 const calls=[];customElements.define('x-commit-test',class extends HTMLElement{static get observedAttributes(){return ['style']}attributeChangedCallback(){calls.push(this.getAttribute('style'))}});
 const el=document.createElement('x-commit-test');document.body.append(el);
 el.style.cssText='--token: keep; opacity: .4 !important';calls.length=0;
 const style=el.style;let getters=0;Object.defineProperty(el,'style',{get(){getters++;throw new Error('author getter')}});
 const a=el.animate({borderColor:['rgb(0,0,255)','rgb(0,0,255)']},{duration:1,fill:'both'});a.pause();a.currentTime=1;a.commitStyles();
 return [calls.length,getters,style.borderColor,style.getPropertyValue('--token'),style.getPropertyPriority('opacity')];
})())"###,
        r###"[1,0,"rgb(0, 0, 255)","keep","important"]"###,
    );
}
