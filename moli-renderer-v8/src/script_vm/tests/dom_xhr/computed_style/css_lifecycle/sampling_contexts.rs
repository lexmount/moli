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
