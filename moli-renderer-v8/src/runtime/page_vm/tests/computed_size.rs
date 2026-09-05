use super::*;

mod capture;
mod chromium;
mod lifecycle;
mod logical;

fn page_with_size_fixture(html: &str) -> anyhow::Result<PageVm> {
    let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
    let mut page_vm = test_page_vm_with_loader_and_document_url(
        &loader,
        Vec::new(),
        Url::parse("https://example.com/computed-size.html")?,
    );
    page_vm
        .vm_mut()
        .set_layout_policy(crate::real_layout_test_policy());
    page_vm.vm_mut().eval(&format!(
        "document.head.innerHTML='<style>html,body{{margin:0}}</style>';document.body.innerHTML={};'installed'",
        serde_json::to_string(html)?,
    ))?;
    page_vm.vm_mut().sync_live_document_style_sources();
    Ok(page_vm)
}

fn read_sizes(page_vm: &mut PageVm, ids: &[&str]) -> anyhow::Result<serde_json::Value> {
    read_without_layout(
        page_vm,
        &format!(
            "Object.fromEntries({}.map(id=>{{const s=getComputedStyle(document.getElementById(id));return [id,[s.width,s.height]]}}))",
            serde_json::to_string(ids)?,
        ),
    )
}

// Assert the cost contract as well as the returned values. A cache hit is
// insufficient: CSSOM must not call the geometry provider at all, even cold.
fn read_without_layout(
    page_vm: &mut PageVm,
    expression: &str,
) -> anyhow::Result<serde_json::Value> {
    let passes = page_vm.vm().layout_pass_observability_for_test().1;
    let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
    let values = page_vm
        .vm_mut()
        .eval(&format!("JSON.stringify({expression})"))?;
    assert_eq!(
        page_vm.vm().layout_pass_observability_for_test().1,
        passes,
        "CSSOM read must not execute layout: {expression}"
    );
    assert_eq!(
        page_vm.vm().layout_snapshot_cache_observability_for_test(),
        cache,
        "CSSOM read must not request, publish, or retire layout: {expression}"
    );
    Ok(serde_json::from_str(&values)?)
}

fn publish_size_layout(page_vm: &mut PageVm) -> anyhow::Result<()> {
    publish_size_layout_at(page_vm, 320, 240)
}

fn publish_size_layout_at(page_vm: &mut PageVm, width: u32, height: u32) -> anyhow::Result<()> {
    let passes = page_vm.vm().layout_pass_observability_for_test().1;
    let publishes = page_vm
        .vm()
        .layout_snapshot_cache_observability_for_test()
        .2;
    page_vm
        .vm_mut()
        .screenshot_layout_snapshot(moli_layout::LayoutViewport::new(width, height, 1.0))?
        .expect("fixture Document should publish a layout snapshot");
    assert_eq!(
        page_vm.vm().layout_pass_observability_for_test().1,
        passes + 1
    );
    assert_eq!(
        page_vm
            .vm()
            .layout_snapshot_cache_observability_for_test()
            .2,
        publishes + 1
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_reads_never_cold_start_layout() {
    run_page_vm_async_test(async move {
        for policy in [moli_page_types::LayoutPolicy::Mock, crate::real_layout_test_policy()] {
            let mut page_vm = page_with_size_fixture(
                r#"<div id=auto></div><canvas id=canvas></canvas>
                <div id=clamped style="width:500px;height:400px;max-width:120px;max-height:80px"></div>"#,
            )?;
            page_vm.vm_mut().set_layout_policy(policy);
            let before = page_vm.vm().layout_pass_observability_for_test().1;
            let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
            assert!(cache.3.is_none());
            assert_eq!(
                read_sizes(&mut page_vm, &["auto", "canvas", "clamped"] )?,
                json!({"auto":["auto","auto"],"canvas":["auto","auto"],"clamped":["500px","400px"]}),
            );
            assert_eq!(page_vm.vm_mut().eval(r#"
(() => {
  const element = document.getElementById('auto');
  const held = getComputedStyle(element);
  for (let i = 0; i < 20; i++) {
    element.style.width = i + 'px';
    if (held.width !== i + 'px' || held.getPropertyValue('height') !== 'auto') return 'wrong';
  }
  return 'read';
})()
"#)?, "read");
            assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
            assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache,
                "CSSOM size reads must not even request a layout pass on a cold Document");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("cold computed-size reads should stay style-only");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_uses_frozen_box_sizing_constraints_and_replaced_geometry() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(r#"
<div id=auto><div style="height:24px"></div></div>
<div id=empty></div>
<div id=content style="width:100.25px;height:40.5px;padding:10px;border:5px solid"></div>
<div id=border style="box-sizing:border-box;width:100.25px;height:70.5px;padding:10px;border:5px solid"></div>
<div id=empty-border style="box-sizing:border-box;width:100px;padding:10px;border:5px solid"></div>
<div id=max style="width:500px;height:400px;max-width:120px;max-height:80px"></div>
<div id=min style="width:10px;height:5px;min-width:150px;min-height:25px"></div>
<canvas id=canvas></canvas>
"#)?;
        publish_size_layout(&mut page_vm)?;
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!(
            read_sizes(&mut page_vm, &["auto", "empty", "content", "border", "empty-border", "max", "min", "canvas"] )?,
            json!({
                // This tall fixture reserves a 15px viewport scrollbar.
                "auto":["305px","24px"], "empty":["305px","0px"],
                "content":["100.25px","40.5px"], "border":["100.25px","70.5px"],
                "empty-border":["100px","30px"], "max":["120px","80px"],
                "min":["150px","25px"], "canvas":["300px","150px"]
            }),
        );
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache,
            "used-size reads consume the snapshot without invoking the layout provider");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("computed sizes should use the sampled CSS sizing box");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_uses_layout_for_flex_grid_positioned_and_shrink_to_fit_boxes() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(r#"
<div style="display:flex;width:200px;height:60px"><div id=flex style="flex:1"></div></div>
<div style="display:grid;width:210px;height:90px;grid-template-columns:1fr 2fr"><div id=grid-a></div><div id=grid-b></div></div>
<div id=shrink style="display:inline-block"><div style="width:45px;height:25px"></div></div>
<div style="position:relative;width:100px;height:80px"><div id=positioned style="position:absolute;left:10px;right:20px;top:5px;bottom:15px"></div></div>
<div style="width:240px;padding:10px;border:5px solid"><div id=percent style="width:50%;height:20px"></div></div>
<div id=vertical style="writing-mode:vertical-rl;width:100px;height:80px"></div>
"#)?;
        publish_size_layout(&mut page_vm)?;
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(
            read_sizes(&mut page_vm, &["flex", "grid-a", "grid-b", "shrink", "positioned", "percent", "vertical"] )?,
            json!({
                "flex":["200px","60px"], "grid-a":["70px","90px"], "grid-b":["140px","90px"],
                "shrink":["45px","25px"], "positioned":["70px","60px"],
                "percent":["120px","20px"], "vertical":["100px","80px"]
            }),
        );
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM should consume the numeric layout algorithm's used sizes");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_preserves_subpixels_and_removes_zoom_and_transforms() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(r#"
<div id=zoomed style="zoom:2;width:100.25px;height:50.5px;padding:2px;border:3px solid"></div>
<div style="zoom:2"><div id=nested style="zoom:2;width:40.25px;height:20.5px"></div></div>
<div id=transformed style="width:80px;height:25px;transform:scale(2)"></div>
<div id=border style="zoom:2;box-sizing:border-box;width:100.25px;height:70.5px;padding:10px;border:5px solid"></div>
"#)?;
        publish_size_layout(&mut page_vm)?;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!(
            read_sizes(&mut page_vm, &["zoomed", "nested", "transformed", "border"] )?,
            json!({
                "zoomed":["100.25px","50.5px"], "nested":["40.25px","20.5px"],
                "transformed":["80px","25px"], "border":["100.25px","70.5px"]
            }),
        );
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM sizes should stay in untransformed, unzoomed CSS pixels");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_uses_the_content_area_after_classic_scrollbar_reservation() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(r#"
<div id=content style="width:100px;height:80px;padding:10px;border:5px solid;overflow:scroll;scrollbar-width:auto"></div>
<div id=border style="box-sizing:border-box;width:130px;height:110px;padding:10px;border:5px solid;overflow:scroll;scrollbar-width:auto"></div>
"#)?;
        publish_size_layout(&mut page_vm)?;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!(read_sizes(&mut page_vm, &["content", "border"] )?,
            json!({"content":["85px","65px"],"border":["130px","110px"]}),
            "content-box resolved sizes exclude classic scrollbars, while border-box sizes include them");
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM should use the sampled scrollport geometry");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_keeps_computed_values_without_an_applicable_snapshot_box() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(r#"
<span id=inline style="width:60px;height:30px;max-width:10px;max-height:10px"></span>
<span id=auto>inline</span>
<div id=none style="display:none;width:60px;height:30px;max-width:10px;max-height:10px"></div>
<div id=contents style="display:contents;width:40%;height:20%"><div style="width:50px;height:25px"></div></div>
<div style="display:none"><div id=hidden-child></div></div>
<div id=removed style="width:55px;height:25px"></div>
"#)?;
        publish_size_layout(&mut page_vm)?;
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        page_vm.vm_mut().eval(r#"
globalThis.removed = document.getElementById('removed');
removed.remove();
const added = document.createElement('div');
added.id = 'added';
document.body.appendChild(added);
'changed'
"#)?;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert_eq!(
            read_sizes(&mut page_vm, &["inline", "auto", "none", "contents", "hidden-child", "added"] )?,
            json!({
                "inline":["60px","30px"], "auto":["auto","auto"], "none":["60px","30px"],
                "contents":["40%","20%"], "hidden-child":["auto","auto"], "added":["auto","auto"]
            }),
        );
        assert_eq!(page_vm.vm_mut().eval(
            "JSON.stringify([getComputedStyle(removed).width,getComputedStyle(removed).height])"
        )?, r#"["",""]"#, "a disconnected element cannot reuse its old box");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("missing or inapplicable boxes should keep the style-only fallback");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_keeps_the_sampled_basis_until_explicit_layout_refresh() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(
            r#"<div id=target style="width:100px;height:40px;padding:10px;border:5px solid"></div>"#,
        )?;
        page_vm.vm_mut().eval("globalThis.held=getComputedStyle(document.getElementById('target'));'held'")?;
        publish_size_layout(&mut page_vm)?;
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        page_vm.vm_mut().eval(r#"
document.getElementById('target').style.cssText='box-sizing:border-box;width:250px;height:100px;padding:20px;border:5px solid;zoom:2';
'changed'
"#)?;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        assert!(cache.3.is_some(), "style mutation deliberately leaves sampled geometry available");
        for _ in 0..3 {
            assert_eq!(read_sizes(&mut page_vm, &["target"] )?, json!({"target":["100px","40px"]}));
            assert_eq!(page_vm.vm_mut().eval(
                "JSON.stringify([held.width,held.getPropertyValue('height'),held.boxSizing,held.zoom])"
            )?, r#"["100px","40px","border-box","2"]"#,
                "old geometry must use its own sizing basis and zoom while style-only properties remain live");
        }
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache);
        publish_size_layout(&mut page_vm)?;
        let refreshed = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(refreshed, before + 1);
        assert_eq!(page_vm.vm_mut().eval(
            "JSON.stringify([held.width,held.getPropertyValue('height')])"
        )?, r#"["250px","100px"]"#, "a held declaration observes the newly published snapshot");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, refreshed);
        page_vm.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        page_vm.vm_mut().eval("document.getElementById('target').style.width='275px';'changed'")?;
        assert_eq!(page_vm.vm_mut().eval("held.width")?, "275px");
        assert!(page_vm.vm().layout_snapshot_cache_observability_for_test().3.is_none());
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, refreshed);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("CSSOM sizes should respect the explicit snapshot lifecycle");
}

#[tokio::test(flavor = "current_thread")]
async fn computed_size_selects_the_exact_nested_iframe_document_snapshot() {
    run_page_vm_async_test(async move {
        let mut page_vm = page_with_size_fixture(
            r#"<div id=target></div><iframe id=frame style="width:180px;height:120px;border:0"></iframe>"#,
        )?;
        page_vm.vm_mut().eval(r#"
globalThis.frame = document.getElementById('frame');
const child = frame.contentDocument;
child.body.style.margin='0';
child.body.innerHTML='<div id=target></div><iframe id=nested style="width:90px;height:60px;border:0"></iframe>';
globalThis.nested = child.getElementById('nested');
const inner = nested.contentDocument;
inner.body.style.margin='0';
inner.body.innerHTML='<div id=target></div>';
globalThis.childStyle = frame.contentWindow.getComputedStyle(child.getElementById('target'));
globalThis.nestedStyle = nested.contentWindow.getComputedStyle(inner.getElementById('target'));
'installed'
"#)?;
        let cold = page_vm.vm().layout_pass_observability_for_test().1;
        assert_eq!(page_vm.vm_mut().eval("JSON.stringify([childStyle.width,nestedStyle.width])")?, r#"["auto","auto"]"#);
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, cold);
        assert!(page_vm.vm().layout_snapshot_cache_observability_for_test().3.is_none());
        publish_size_layout(&mut page_vm)?;
        let before = page_vm.vm().layout_pass_observability_for_test().1;
        let cache = page_vm.vm().layout_snapshot_cache_observability_for_test();
        for _ in 0..3 {
            assert_eq!(page_vm.vm_mut().eval(r#"
JSON.stringify([
  getComputedStyle(document.getElementById('target')).width,
  childStyle.width, nestedStyle.width,
  getComputedStyle(frame.contentDocument.getElementById('target')).width,
  getComputedStyle(nested.contentDocument.getElementById('target')).width
])
"#)?, r#"["320px","180px","90px","180px","90px"]"#);
        }
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test(), cache);
        page_vm.vm_mut().eval(r#"
const freshFrame = document.createElement('iframe');
document.body.appendChild(freshFrame);
const moved = frame.contentDocument.getElementById('target');
freshFrame.contentDocument.body.appendChild(moved);
globalThis.movedStyle = freshFrame.contentWindow.getComputedStyle(moved);
'moved'
"#)?;
        assert_eq!(page_vm.vm_mut().eval("movedStyle.width")?, "auto",
            "adopting a node into an unsampled Document must not read its previous Document's box");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, before);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("computed size should use only the owning Document's existing snapshot");
}
