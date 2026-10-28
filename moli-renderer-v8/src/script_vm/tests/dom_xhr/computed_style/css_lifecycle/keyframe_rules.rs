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

#[test]
fn css_keyframes_keep_per_property_easing_at_the_same_offset() {
    for first in [
        "opacity:0;animation-timing-function:steps(1,end)",
        "opacity:0",
    ] {
        assert_eq!(
            sample(
                &format!(
                    "@keyframes mixed{{from{{{first}}}from{{left:0px;animation-timing-function:linear}}to{{opacity:1;left:100px}}}}"
                ),
                r#"JSON.stringify((()=>{
                    const box=document.getElementById('box');
                    box.style.cssText='position:relative;animation:mixed 1s steps(1,end) both paused';
                    box.getAnimations()[0].currentTime=500;
                    const style=getComputedStyle(box);return[+style.opacity,parseFloat(style.left)];
                })())"#,
            ),
            "[0,50]",
        );
    }
}

#[test]
fn css_keyframes_with_distinct_easings_keep_independent_interval_endpoints() {
    assert_eq!(
        sample(
            "@keyframes mixed{from{opacity:0}50%{opacity:.5;animation-timing-function:steps(1,end)}50%{opacity:.7;animation-timing-function:linear}to{opacity:1}}",
            r#"JSON.stringify((()=>{
                const box=document.getElementById('box');box.style.animation='mixed 1s linear both paused';
                const a=box.getAnimations()[0];
                return[250,500,750].map(t=>{a.currentTime=t;return +getComputedStyle(box).opacity});
            })())"#,
        ),
        "[0.25,0.7,0.85]",
    );
}

#[test]
fn css_keyframe_groups_use_the_effective_canonical_easing() {
    for (first, last, expected) in [
        ("", "animation-timing-function:linear", "0.35"),
        (
            "animation-timing-function:step-end",
            "animation-timing-function:steps(1,end)",
            "0.35",
        ),
        (
            "animation-timing-function:ease",
            "animation-timing-function:cubic-bezier(.25,.1,.25,1)",
            "0.25",
        ),
    ] {
        assert_eq!(
            sample(
                &format!(
                    "@keyframes mixed{{from{{opacity:0}}50%{{opacity:.5;{first}}}50%{{opacity:.7;{last}}}to{{opacity:1}}}}"
                ),
                "const box=document.getElementById('box');box.style.animation='mixed 1s linear both paused';box.getAnimations()[0].currentTime=250;getComputedStyle(box).opacity",
            ),
            expected,
            "{first} / {last}",
        );
    }
}

#[test]
fn css_nonadjacent_matching_groups_cascade_at_their_earliest_position() {
    // CSS Animations 2 preserves the group's earliest position. Blink records
    // its remaining rightmost-merge difference in crbug.com/1408702.
    assert_eq!(
        sample(
            "@keyframes mixed{from{opacity:0}50%{opacity:.5;animation-timing-function:steps(1,end)}50%{opacity:.7;animation-timing-function:linear}50%{opacity:.6;animation-timing-function:steps(1,end)}to{opacity:1}}",
            r#"JSON.stringify((()=>{
                const box=document.getElementById('box');box.style.animation='mixed 1s linear both paused';
                const a=box.getAnimations()[0];
                return[250,750].map(t=>{a.currentTime=t;return +getComputedStyle(box).opacity});
            })())"#,
        ),
        "[0.3,0.85]",
    );
}
