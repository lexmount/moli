//! Logical counterparts of the physical resolved sizes. Like the Chromium
//! css/css-logical/parsing/{inline,block}-size-computed.html WPTs, these read
//! through both IDL attributes and getPropertyValue. Unlike Chromium, a read
//! must never initiate layout: the fixture explicitly publishes each sample.

use super::*;

fn read_logical_sizes(page: &mut PageVm, ids: &[&str]) -> anyhow::Result<serde_json::Value> {
    read_without_layout(
        page,
        &format!(
            "Object.fromEntries({}.map(id=>{{const s=getComputedStyle(document.getElementById(id));return [id,[s.width,s.height,s.inlineSize,s.blockSize,s.getPropertyValue('inline-size'),s.getPropertyValue('block-size')]]}}))",
            serde_json::to_string(ids)?,
        ),
    )
}

fn expected_sizes(width: &str, height: &str, horizontal: bool) -> serde_json::Value {
    let (inline, block) = if horizontal {
        (width, height)
    } else {
        (height, width)
    };
    json!([width, height, inline, block, inline, block])
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_resolved_values_from_chromium_wpt_use_explicit_samples() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"
<div style="width:200px;height:300px"><div id=target style="width:0;height:0;font-size:40px"><div style="width:60px;height:80px"></div></div></div>
"#)?;
        for (property, auto, percent, intrinsic) in [
            ("inline-size", "200px", "40px", "60px"),
            ("block-size", "80px", "60px", "80px"),
        ] {
            for (specified, expected) in [
                ("auto", auto), ("10px", "10px"), ("20%", percent),
                ("calc(0.5em + 10px)", "30px"), ("calc(-0.5em + 10px)", "0px"),
                ("min-content", intrinsic), ("max-content", intrinsic),
            ] {
                page.vm_mut().eval(&format!(
                    "document.getElementById('target').style.cssText='width:0;height:0;font-size:40px;{property}:{specified}';'changed'"
                ))?;
                publish_size_layout(&mut page)?;
                assert_eq!(read_without_layout(&mut page, &format!(
                    "getComputedStyle(document.getElementById('target')).getPropertyValue('{property}')"
                ))?, json!(expected), "{property}: {specified}");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("logical resolved-value WPT cases should consume explicit Moli samples");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_axes_follow_sampled_constraints_box_sizing_and_zoom() {
    run_page_vm_async_test(async move {
        for writing_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            for direction in ["ltr", "rtl"] {
                let mut page = page_with_size_fixture(&format!(r#"
<div style="writing-mode:{writing_mode};direction:{direction};zoom:2">
  <div id=max style="width:500px;height:400px;max-width:120px;max-height:80px"></div>
  <div id=min style="width:10px;height:5px;min-width:30px;min-height:25px"></div>
  <div id=border style="box-sizing:border-box;width:90.25px;height:60.5px;padding:10px;border:5px solid;transform:scale(2)"></div>
  <div id=content style="width:90.25px;height:60.5px;padding:10px;border:5px solid"></div>
</div>"#))?;
                publish_size_layout(&mut page)?;
                let horizontal = writing_mode == "horizontal-tb";
                assert_eq!(read_logical_sizes(&mut page, &["max", "min", "border", "content"] )?,
                    json!({
                        "max":expected_sizes("120px","80px",horizontal),
                        "min":expected_sizes("30px","25px",horizontal),
                        "border":expected_sizes("90.25px","60.5px",horizontal),
                        "content":expected_sizes("90.25px","60.5px",horizontal),
                    }), "{writing_mode}, {direction}");
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("logical dimensions must select the same sampled CSS sizing box");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_cold_and_non_box_reads_stay_style_only() {
    run_page_vm_async_test(async move {
        for policy in [moli_page_types::LayoutPolicy::Mock, crate::real_layout_test_policy()] {
            let mut page = page_with_size_fixture(r#"
<div id=auto></div>
<div id=clamped style="writing-mode:vertical-rl;width:500px;height:400px;max-width:120px;max-height:80px"></div>
<span id=inline style="width:50%;height:20%"></span>
<div id=none style="display:none;width:50%;height:20%"></div>
<div id=contents style="display:contents;width:50%;height:20%"></div>
"#)?;
            page.vm_mut().set_layout_policy(policy);
            assert_eq!(read_logical_sizes(&mut page, &["auto", "clamped"] )?, json!({
                "auto":expected_sizes("auto","auto",true),
                "clamped":expected_sizes("500px","400px",false),
            }));
            let unboxed = read_logical_sizes(&mut page, &["inline", "none", "contents"] )?;
            assert_eq!(unboxed, json!({
                "inline":expected_sizes("50%","20%",true),
                "none":expected_sizes("50%","20%",true),
                "contents":expected_sizes("50%","20%",true),
            }));
            if policy.uses_real_layout() {
                publish_size_layout(&mut page)?;
                assert_eq!(read_logical_sizes(&mut page, &["inline", "none", "contents"] )?, unboxed);
                page.vm_mut().eval("globalThis.added=document.createElement('div');added.id='new';document.body.appendChild(added);'added'")?;
                assert_eq!(read_logical_sizes(&mut page, &["new"] )?, json!({"new":expected_sizes("auto","auto",true)}));
            }
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("logical dimensions must not manufacture a box or start layout");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_held_declarations_keep_sampled_writing_axes_until_refresh() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"<div id=parent><div id=target style="width:500px;height:400px;max-width:120px;max-height:80px"></div></div>"#)?;
        page.vm_mut().eval("globalThis.held=getComputedStyle(document.getElementById('target'));'held'")?;
        let read = "[held.width,held.height,held.inlineSize,held.blockSize,held.getPropertyValue('inline-size'),held.getPropertyValue('block-size')]";
        publish_size_layout(&mut page)?;
        let mut horizontal = true;
        for mode in ["vertical-rl", "vertical-lr", "horizontal-tb"] {
            page.vm_mut().eval(&format!("document.getElementById('parent').style.writingMode='{mode}';'changed'"))?;
            assert_eq!(read_without_layout(&mut page, "held.writingMode")?, json!(mode), "computed style stays live");
            assert_eq!(read_without_layout(&mut page, read)?, expected_sizes("120px","80px",horizontal), "old sample after {mode}");
            publish_size_layout(&mut page)?;
            horizontal = mode == "horizontal-tb";
            assert_eq!(read_without_layout(&mut page, read)?, expected_sizes("120px","80px",horizontal), "fresh sample after {mode}");
        }
        page.vm_mut().eval("document.getElementById('target').remove();'removed'")?;
        assert_eq!(read_without_layout(&mut page, read)?, json!(["","","","","",""]), "a held declaration must not revive a removed target's geometry");
        Ok::<_, anyhow::Error>(())
    }).await.expect("logical axes must have the same lifetime as their geometry");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_auto_and_replaced_sizes_use_existing_samples() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"
<div style="width:160px"><div id=auto><div style="height:20px"></div></div></div>
<canvas id=canvas style="writing-mode:vertical-rl"></canvas>
<div style="writing-mode:vertical-lr;width:100px;height:80px;position:relative"><div id=positioned style="position:absolute;inset:10px"></div></div>
"#)?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_logical_sizes(&mut page, &["auto", "canvas", "positioned"] )?, json!({
            "auto":expected_sizes("160px","20px",true),
            "canvas":expected_sizes("300px","150px",false),
            "positioned":expected_sizes("80px","60px",false),
        }));
        Ok::<_, anyhow::Error>(())
    }).await.expect("auto and replaced logical sizes must use numeric layout results");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_logical_iframe_and_shadow_axes_are_local_to_the_sampled_box() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"<div id=host></div><iframe id=frame style="width:180px;height:120px;border:0"></iframe>"#)?;
        page.vm_mut().eval(r#"
const root=document.getElementById('host').attachShadow({mode:'open'});
root.innerHTML='<style>:host{writing-mode:vertical-lr}#target{width:500px;height:400px;max-width:40px;max-height:20px}</style><div id=target></div>';
globalThis.shadowStyle=getComputedStyle(root.getElementById('target'));
const frame=document.getElementById('frame');
frame.contentDocument.body.innerHTML='<div id=target style="writing-mode:vertical-rl;width:500px;height:400px;max-width:90px;max-height:30px"></div>';
globalThis.childStyle=frame.contentWindow.getComputedStyle(frame.contentDocument.getElementById('target'));
'installed'
"#)?;
        publish_size_layout(&mut page)?;
        assert_eq!(read_without_layout(&mut page,
            "[shadowStyle.width,shadowStyle.height,shadowStyle.inlineSize,shadowStyle.blockSize,childStyle.width,childStyle.height,childStyle.inlineSize,childStyle.blockSize]")?,
            json!(["40px","20px","20px","40px","90px","30px","30px","90px"]));
        Ok::<_, anyhow::Error>(())
    }).await.expect("logical dimensions must use each target's own frozen axes and Document");
}
