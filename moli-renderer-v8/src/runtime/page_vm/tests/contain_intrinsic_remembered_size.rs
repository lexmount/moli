use super::*;

fn screenshot_layout(page_vm: &mut PageVm) -> anyhow::Result<()> {
    page_vm
        .vm_mut()
        .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
        .expect("contain-intrinsic-size screenshot layout");
    Ok(())
}

fn target_size(page_vm: &mut PageVm, id: &str) -> anyhow::Result<[f32; 2]> {
    let geometry = page_vm.vm_mut().eval(&format!(
        "(()=>{{const r=document.getElementById({id:?}).getBoundingClientRect();return JSON.stringify([r.width,r.height])}})()"
    ))?;
    Ok(serde_json::from_str(&geometry)?)
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_retains_logical_contain_intrinsic_auto_size_across_layout_boxes() {
    // Covers the state transitions in css-sizing/contain-intrinsic-size/
    // auto-006.html without relying on synchronous geometry to refresh layout.
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/contain-intrinsic-remembered-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#target{position:absolute;left:0;top:0;width:max-content;height:max-content;contain-intrinsic-size:auto 2px auto 1px}
#contents{width:100px;height:50px}
.skip{content-visibility:hidden}
.vertical{writing-mode:vertical-lr}
</style>`;
document.body.innerHTML = `<div id=parent><div id=target class=skip><div id=contents></div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [2.0, 1.0]);

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').className = ''")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [100.0, 50.0]);

        page_vm.vm_mut().eval(
            "document.getElementById('contents').style.cssText='width:75px;height:25px';document.getElementById('target').className='skip'",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "target")?,
            [100.0, 50.0],
            "a display-locked box must use, but not overwrite, its remembered content box",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').className='skip vertical'")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "target")?,
            [50.0, 100.0],
            "remembered inline/block sizes must project through the current writing mode",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').className='skip';document.getElementById('target').style.containIntrinsicSize='7px 9px'")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [7.0, 9.0]);

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').style.containIntrinsicSize=''")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "target")?,
            [2.0, 1.0],
            "losing auto must clear both remembered axes before auto is restored",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('target').className=''")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [75.0, 25.0]);

        page_vm
            .vm_mut()
            .eval("document.getElementById('parent').style.display='none'")?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [0.0, 0.0]);

        page_vm.vm_mut().eval(
            "document.getElementById('parent').style.display='';document.getElementById('target').className='skip'",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "target")?,
            [75.0, 25.0],
            "remembered state belongs to the element and must survive layout-box destruction",
        );

        page_vm.vm_mut().eval(
            "document.getElementById('target').style.cssText='display:none;contain-intrinsic-size:7px 9px'",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "target")?, [0.0, 0.0]);

        page_vm.vm_mut().eval(
            "document.getElementById('target').style.cssText='';document.getElementById('target').className='skip'",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "target")?,
            [2.0, 1.0],
            "losing auto while no layout box exists must still clear remembered state",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("contain-intrinsic remembered-size fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_tracks_contain_intrinsic_auto_axes_independently() {
    // Covers css-sizing/contain-intrinsic-size/auto-009.html's physical CSS
    // properties while retaining logical observer state.
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/contain-intrinsic-independent-axes.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.target{position:absolute;width:max-content;height:max-content}
.contents{width:100px;height:50px}
#inline{left:0;top:0;contain-intrinsic-width:auto 20px}
#block{left:150px;top:0;contain-intrinsic-height:auto 10px}
.skip{content-visibility:hidden;contain-intrinsic-width:auto 2px;contain-intrinsic-height:auto 1px}
</style>`;
document.body.innerHTML = `
  <div id=inline class=target><div class=contents></div></div>
  <div id=block class=target><div class=contents></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        screenshot_layout(&mut page_vm)?;
        assert_eq!(target_size(&mut page_vm, "inline")?, [100.0, 50.0]);
        assert_eq!(target_size(&mut page_vm, "block")?, [100.0, 50.0]);

        page_vm.vm_mut().eval(
            "document.getElementById('inline').classList.add('skip');document.getElementById('block').classList.add('skip')",
        )?;
        screenshot_layout(&mut page_vm)?;
        assert_eq!(
            target_size(&mut page_vm, "inline")?,
            [100.0, 1.0],
            "only the inline axis had a remembered value",
        );
        assert_eq!(
            target_size(&mut page_vm, "block")?,
            [2.0, 50.0],
            "only the block axis had a remembered value",
        );

        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("contain-intrinsic independent-axis fixture should run");
}
