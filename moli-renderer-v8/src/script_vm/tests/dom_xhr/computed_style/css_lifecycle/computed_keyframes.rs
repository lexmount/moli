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
fn font_shorthand_dependencies_ignore_property_order() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.cssText="font-size:10px;width:2em";const a=el.animate({width:["1em","3em"],font:["10px serif","30px serif"]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;const s=getComputedStyle(el);return[s.fontSize,s.width];})())"###,
        r###"["20px","40px"]"###,
    );
}

#[test]
fn font_shorthand_before_width_uses_sampled_font() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.cssText="font-size:10px;width:2em";const a=el.animate({font:["10px serif","30px serif"],width:["1em","3em"]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;const s=getComputedStyle(el);return[s.fontSize,s.width];})())"###,
        r###"["20px","40px"]"###,
    );
}

#[test]
fn longhand_wins_over_later_shorthand() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({marginLeft:["20px","40px"],margin:["0px","100px"]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;return getComputedStyle(el).marginLeft;})())"###,
        r###""30px""###,
    );
}

#[test]
fn longhand_wins_over_earlier_shorthand() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");const a=el.animate({margin:["0px","100px"],marginLeft:["20px","40px"]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;return getComputedStyle(el).marginLeft;})())"###,
        r###""30px""###,
    );
}

#[test]
fn invalid_value_interval() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.width="10px";const a=el.animate({width:["0px","bogus","100px"]},{duration:1000,fill:"both"});a.pause();a.currentTime=250;return getComputedStyle(el).width;})())"###,
        r###""25px""###,
    );
}

#[test]
fn important_animation_origin() {
    assert_sample(
        r###"<!doctype html><style></style><div id="box"></div>"###,
        r###"JSON.stringify((()=>{const el=document.getElementById("box");el.style.setProperty("opacity",".8","important");const a=el.animate({opacity:[0,1]},{duration:1000,fill:"both"});a.pause();a.currentTime=500;return getComputedStyle(el).opacity;})())"###,
        r###""0.8""###,
    );
}

#[test]
fn separate_keyframes_with_equal_offsets_keep_their_order() {
    assert_sample(
        "<!doctype html><div id=box></div>",
        r#"JSON.stringify((()=>{
        const box=document.getElementById('box');
        const a=box.animate([{offset:0,marginLeft:'20px'},{offset:0,margin:'0px'},{offset:1,marginLeft:'40px'}],{duration:1000,fill:'both'});
        a.pause();a.currentTime=500;return getComputedStyle(box).marginLeft;
    })())"#,
        r#""20px""#,
    );
}
