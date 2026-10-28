use super::*;

fn sample(css: &str, script: &str) -> String {
    let mut vm = new_parsed_test_vm(
        "https://animation-contract.test/keyframe-rules",
        &format!("<!doctype html><style>{css}</style><div id=box></div>"),
    );
    vm.set_layout_policy(moli_page_types::LayoutPolicy::OnDemand);
    vm.eval(script).unwrap()
}

#[test]
fn css_keyframes_use_effective_conditions_and_case_sensitive_names() {
    for (extra, expected) in [
        ("@media print{@keyframes fade{from,to{opacity:.8}}}", "0.2"),
        (
            "@media(min-width:999999px){@keyframes fade{from,to{opacity:.8}}}",
            "0.2",
        ),
        (
            "@supports(bogus:bogus){@keyframes fade{from,to{opacity:.8}}}",
            "0.2",
        ),
        ("@keyframes FADE{from,to{opacity:.8}}", "0.2"),
        (
            "@media screen{@supports(opacity:.5){@keyframes fade{from,to{opacity:.8}}}}",
            "0.8",
        ),
    ] {
        assert_eq!(
            sample(
                &format!("@keyframes fade{{from,to{{opacity:.2}}}}{extra}"),
                "const box=document.getElementById('box');box.style.animation='fade 1s linear both paused';box.getAnimations()[0].currentTime=500;getComputedStyle(box).opacity",
            ),
            expected,
            "{extra}",
        );
    }
}

#[test]
fn css_keyframes_follow_cascade_layers_and_live_rule_changes() {
    assert_eq!(
        sample(
            "@layer first,last;@layer last{@keyframes fade{from,to{opacity:.8}}}@layer first{@keyframes fade{from,to{opacity:.4}}}",
            r#"JSON.stringify((()=>{
            const box=document.getElementById('box');box.style.animation='fade 1s linear both paused';
            box.getAnimations()[0].currentTime=500;const before=getComputedStyle(box).opacity;
            document.styleSheets[0].insertRule('@keyframes fade{from,to{opacity:.6}}',document.styleSheets[0].cssRules.length);
            return[before,getComputedStyle(box).opacity];
        })())"#,
        ),
        r#"["0.8","0.6"]"#
    );
}

#[test]
fn css_keyframes_are_resolved_in_the_elements_tree_scope() {
    assert_eq!(
        sample(
            "@keyframes fade{from,to{opacity:.2}}",
            r#"JSON.stringify((()=>{
            const host=document.getElementById('box'),root=host.attachShadow({mode:'open'});
            root.innerHTML='<style>@keyframes fade{from,to{opacity:.6}}</style><div id=child style="animation:fade 1s linear both paused"></div>';
            const child=root.getElementById('child');child.getAnimations()[0].currentTime=500;
            const peer=document.body.appendChild(document.createElement('div'));
            peer.style.animation='fade 1s linear both paused';peer.getAnimations()[0].currentTime=500;
            return[getComputedStyle(peer).opacity,getComputedStyle(child).opacity];
        })())"#,
        ),
        r#"["0.2","0.6"]"#
    );
}
