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
fn typed_css_timing_preserves_none_positions_and_cycles_each_list_independently() {
    assert_eq!(
        sample(
            r#"@keyframes "a,b"{from{opacity:0}to{opacity:1}}@keyframes plain{from{left:0px}to{left:100px}}"#,
            r#"JSON.stringify((()=>{
                const box=document.getElementById('box');
                box.style.cssText='position:relative;animation-name:none,"a,b",plain;animation-duration:.25s,.5s;animation-delay:-.125s,.25s;animation-iteration-count:1.5,2;animation-fill-mode:backwards,both;animation-direction:alternate,reverse;animation-timing-function:steps(2,end),linear;animation-play-state:running,paused';
                const animations=box.getAnimations();
                const timings=animations.map(a=>{const t=a.effect.getTiming();return[a.animationName,t.duration,t.delay,t.iterations,t.fill,t.direction,t.easing,a.playState]});
                animations[0].currentTime=500;animations[1].currentTime=-62.5;
                const style=getComputedStyle(box);return[timings,+style.opacity,parseFloat(style.left)];
            })())"#,
        ),
        r#"[[["a,b",500,250,2,"both","reverse","linear","paused"],["plain",250,-125,1.5,"backwards","alternate","linear","running"]],0.5,0]"#,
    );
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

#[test]
fn css_logical_keyframes_map_before_cascading_and_building_tracks() {
    for (rules, style, property, expected) in [
        (
            "to{margin-inline-start:100px}",
            "margin-left:20px",
            "marginInlineStart",
            "[20,60,100]",
        ),
        (
            "from{margin-left:20px}to{margin-inline-start:100px}",
            "margin-left:0px",
            "marginInlineStart",
            "[20,60,100]",
        ),
        (
            "to{margin-inline-start:100px}",
            "direction:rtl;margin-right:30px",
            "marginInlineStart",
            "[30,65,100]",
        ),
        (
            "to{margin-inline-start:100px}",
            "writing-mode:vertical-lr;margin-top:40px",
            "marginInlineStart",
            "[40,70,100]",
        ),
        (
            "to{inset-inline-start:100px}",
            "position:relative;left:20px",
            "insetInlineStart",
            "[20,60,100]",
        ),
        (
            "from{margin-inline-start:10px;margin-left:20px}to{margin-inline-start:100px}",
            "",
            "marginInlineStart",
            "[20,60,100]",
        ),
    ] {
        assert_eq!(
            sample(
                &format!("@keyframes move{{{rules}}}"),
                &format!(
                    r#"
            const box=document.getElementById('box');box.style.cssText='{style};animation:move 1s linear both paused';
            const a=box.getAnimations()[0];
            JSON.stringify([0,500,1000].map(t=>{{a.currentTime=t;return parseFloat(getComputedStyle(box).{property})}}))
        "#
                )
            ),
            expected,
            "{rules} / {style}"
        );
    }
}

#[test]
fn css_logical_tracks_follow_updated_writing_direction() {
    assert_eq!(
        sample(
            "@keyframes move{to{margin-inline-start:100px}}",
            r#"
        const box=document.getElementById('box');box.style.cssText='margin-left:20px;margin-right:30px;animation:move 1s linear both paused';
        box.getAnimations()[0].currentTime=500;const before=parseFloat(getComputedStyle(box).marginInlineStart);
        box.style.direction='rtl';JSON.stringify([before,parseFloat(getComputedStyle(box).marginInlineStart)])
    "#
        ),
        "[60,65]"
    );
}

#[test]
fn css_animation_names_preserve_decoded_tokens_and_list_positions() {
    for (css, declaration, expected) in [
        (
            r"@keyframes \31 abc{from{opacity:0}to{opacity:1}}",
            r"animation:\31 abc 1s linear both paused",
            r#"[[["1abc",1000]],0.5]"#,
        ),
        (
            r#"@keyframes "a,b"{from{opacity:0}to{opacity:1}}@keyframes plain{from{left:0px}to{left:100px}}"#,
            r#"animation-name:"a,b",plain;animation-duration:1s,2s,3s;animation-timing-function:linear;animation-play-state:paused;animation-fill-mode:both"#,
            r#"[[["a,b",1000],["plain",2000]],0.5]"#,
        ),
        (
            r#"@keyframes "none"{from{opacity:0}to{opacity:1}}@keyframes plain{from{left:0px}to{left:100px}}"#,
            r#"animation-name:none,"none",plain;animation-duration:1s,2s,3s;animation-timing-function:linear;animation-play-state:paused;animation-fill-mode:both"#,
            r#"[[["none",2000],["plain",3000]],0.25]"#,
        ),
    ] {
        assert_eq!(
            sample(
                css,
                &format!(
                    r#"
            const box=document.getElementById('box');box.style.cssText=String.raw`{declaration}`;
            const animations=box.getAnimations();animations.forEach(a=>a.currentTime=500);
            JSON.stringify([animations.map(a=>[a.animationName,a.effect.getTiming().duration]),+getComputedStyle(box).opacity])
        "#
                )
            ),
            expected,
            "{declaration}"
        );
    }
}

#[test]
fn css_keyframe_wide_easing_uses_initial_and_inherited_computed_values() {
    for (keyword, parent, expected) in [
        ("initial", "linear", "802"),
        ("unset", "linear", "802"),
        ("inherit", "ease", "802"),
        ("inherit", "steps(1,end)", "0"),
    ] {
        assert_eq!(
            sample(
                &format!(
                    "@keyframes fade{{from{{opacity:0;animation-timing-function:{keyword}}}to{{opacity:1}}}}"
                ),
                &format!(
                    r#"
            document.body.style.animationTimingFunction='{parent}';
            const box=document.getElementById('box');box.style.animation='fade 1s linear both paused';
            box.getAnimations()[0].currentTime=500;Math.round(+getComputedStyle(box).opacity*1000)
        "#
                )
            ),
            expected,
            "{keyword} / {parent}"
        );
    }
}
