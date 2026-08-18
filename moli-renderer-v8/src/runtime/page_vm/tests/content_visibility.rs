use super::*;

fn screenshot_layout(page_vm: &mut PageVm) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
        .expect("content-visibility screenshot layout");
    Ok(())
}

fn auto_state(page_vm: &mut PageVm) -> anyhow::Result<serde_json::Value> {
    let value = page_vm.vm_mut().eval(
        r#"
(() => {
  const target = document.getElementById('target');
  const contents = document.getElementById('contents');
  const targetRect = target.getBoundingClientRect();
  const contentsRect = contents.getBoundingClientRect();
  return JSON.stringify({
    target: [targetRect.x, targetRect.width, targetRect.height],
    contents: [contentsRect.width, contentsRect.height],
    text: target.innerText,
    defaultVisibility: target.checkVisibility(),
    autoVisibility: target.checkVisibility({contentVisibilityAuto: true}),
    childAutoVisibility: contents.checkVisibility({contentVisibilityAuto: true}),
    active: document.activeElement && document.activeElement.id
  });
})()
"#,
    )?;
    Ok(serde_json::from_str(&value)?)
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_advances_viewport_driven_auto_display_locks() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/content-visibility-auto.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#target{position:absolute;left:0;top:0;width:100px;height:max-content;content-visibility:auto;contain-intrinsic-size:auto 7px}
#contents{height:50px}
</style>`;
document.body.innerHTML = `<span id=before>before</span><div id=target><div id=contents tabindex=0>visible</div><div id=popover popover=manual></div></div><span id=after>after</span>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?,
            serde_json::json!({
                "target": [0, 100, 50],
                "contents": [100, 50],
                "text": "visible",
                "defaultVisibility": true,
                "autoVisibility": true,
                "childAutoVisibility": true,
                "active": "",
            }),
            "the first post-layout observation must synchronously unlock a near-viewport context",
        );

        page_vm.vm_mut().eval(
            "document.getElementById('target').style.left='-2000px';document.getElementById('contents').style.height='80px'",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?,
            serde_json::json!({
                "target": [-2000, 100, 80],
                "contents": [0, 0],
                "text": "",
                "defaultVisibility": true,
                "autoVisibility": true,
                "childAutoVisibility": false,
                "active": "",
            }),
            "an offscreen lock must omit descendants after remembering the size from its post-layout observation",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('contents').focus()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?,
            serde_json::json!({
                "target": [-2000, 100, 80],
                "contents": [100, 80],
                "text": "visible",
                "defaultVisibility": true,
                "autoVisibility": true,
                "childAutoVisibility": true,
                "active": "contents",
            }),
            "focus in the subtree must force an auto display lock open",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('contents').blur()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?["target"],
            serde_json::json!([-2000, 100, 80]),
            "relocking after focus leaves the newly remembered size in place",
        );
        assert_eq!(auto_state(&mut page_vm)?["childAutoVisibility"], false);

        page_vm.vm_mut().eval(
            r#"
const range = document.createRange();
range.selectNodeContents(document.getElementById('contents'));
const selection = document.getSelection();
selection.removeAllRanges();
selection.addRange(range);
'selected'
"#,
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?["childAutoVisibility"],
            true,
            "a selection intersecting the subtree must force the lock open",
        );
        page_vm
            .vm_mut()
            .eval("document.getSelection().removeAllRanges()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(auto_state(&mut page_vm)?["childAutoVisibility"], false);

        page_vm
            .vm_mut()
            .eval("history.replaceState(null, '', '#contents')")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?["childAutoVisibility"],
            true,
            "a fragment target in the subtree must force the lock open",
        );
        page_vm
            .vm_mut()
            .eval("history.replaceState(null, '', location.pathname)")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(auto_state(&mut page_vm)?["childAutoVisibility"], false);

        page_vm.vm_mut().eval(
            r#"
document.getSelection().setBaseAndExtent(
  document.getElementById('after').firstChild, 5,
  document.getElementById('before').firstChild, 0
);
'reverse-selected'
"#,
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?["childAutoVisibility"],
            true,
            "a reverse selection spanning the subtree must force the lock open",
        );
        page_vm
            .vm_mut()
            .eval("document.getSelection().removeAllRanges()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(auto_state(&mut page_vm)?["childAutoVisibility"], false);

        page_vm
            .vm_mut()
            .eval("document.getElementById('popover').showPopover()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            auto_state(&mut page_vm)?["childAutoVisibility"],
            true,
            "a top-layer descendant must force the lock open",
        );
        page_vm
            .vm_mut()
            .eval("document.getElementById('popover').hidePopover()")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(auto_state(&mut page_vm)?["childAutoVisibility"], false);

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').style.left='0'")?;
        screenshot_layout(&mut page_vm)?;
        let final_state = auto_state(&mut page_vm)?;
        assert_eq!(final_state["contents"], serde_json::json!([100, 80]));
        assert_eq!(final_state["autoVisibility"], true);

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("content-visibility:auto lifecycle fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_applies_display_lock_box_eligibility_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/content-visibility-box-eligibility.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}.lock{content-visibility:hidden}</style>';
document.body.innerHTML = `
  <table id=table class=lock><tbody><tr><td><span id=table-child>table</span></td></tr></tbody></table>
  <table><caption id=caption class=lock><span id=caption-child>caption</span></caption><tbody><tr><td>x</td></tr></tbody></table>
  <table><tbody><tr><td id=cell class=lock><span id=cell-child>cell</span></td></tr></tbody></table>
  <button class=lock style="display:inline"><span id=form-child>form</span></button>
  <output class=lock style="display:inline"><span id=output-child>output</span></output>
  <span style="display:contents;content-visibility:auto"><span id=contents-child>contents</span></span>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        screenshot_layout(&mut page_vm)?;

        let result = page_vm.vm_mut().eval(
            r#"
JSON.stringify(['table-child','caption-child','cell-child','form-child','output-child','contents-child'].map(id => {
  const r = document.getElementById(id).getBoundingClientRect();
  return [r.width, r.height];
}))
"#,
        )?;
        let geometry: Vec<[f32; 2]> = serde_json::from_str(&result)?;
        assert!(geometry[0][0] > 0.0 && geometry[0][1] > 0.0);
        assert!(geometry[1][0] > 0.0 && geometry[1][1] > 0.0);
        assert_eq!(geometry[2], [0.0, 0.0]);
        assert_eq!(geometry[3], [0.0, 0.0]);
        assert!(geometry[4][0] > 0.0 && geometry[4][1] > 0.0);
        assert!(geometry[5][0] > 0.0 && geometry[5][1] > 0.0);
        assert_eq!(
            page_vm.vm_mut().eval(
                "document.getElementById('contents-child').checkVisibility({contentVisibilityAuto:true})",
            )?,
            "true",
            "an auto request without a principal box must not retain a stale lock",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("display-lock box eligibility fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_reveals_nested_auto_contexts_to_a_stable_epoch() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/content-visibility-nested-auto.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>html,body{margin:0}.auto{width:max-content;height:max-content;content-visibility:auto;contain-intrinsic-size:auto 1px}#leaf{width:40px;height:30px}</style>';
let subtree = '<div id=leaf></div>';
for (let depth = 0; depth < 8; depth++) {
  subtree = `<div id=lock-${depth} class=auto>${subtree}</div>`;
}
document.body.innerHTML = subtree;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        screenshot_layout(&mut page_vm)?;

        let result = page_vm.vm_mut().eval(
            r#"
(() => {
  const leaf = document.getElementById('leaf').getBoundingClientRect();
  const locks = [...document.querySelectorAll('.auto')];
  return JSON.stringify({
    leaf: [leaf.width, leaf.height],
    visible: locks.every(lock => lock.checkVisibility({contentVisibilityAuto:true}))
  });
})()
"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result)?,
            serde_json::json!({"leaf": [40, 30], "visible": true}),
            "each rerun must reveal a previously unobserved nested lock until the epoch is stable",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested content-visibility:auto fixture should run");
}
