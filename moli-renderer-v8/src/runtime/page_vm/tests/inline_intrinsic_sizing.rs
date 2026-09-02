use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_separates_atomic_intrinsic_widths_at_forced_breaks() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-forced-break-intrinsic-widths.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;font-size:0;line-height:0}
.probe{display:inline-block;vertical-align:top;background:rgb(1,2,3)}
.vertical{writing-mode:vertical-lr}
.probe span{display:inline-block;block-size:10px}
.first{inline-size:20px}
.second{inline-size:30px}
</style>`;
document.body.innerHTML = `
  <div id=horizontal class=probe><span class=first></span><br><span class=second></span></div>
  <div id=vertical class="probe vertical"><span class=first></span><br><span class=second></span></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 100, 1.0))?
            .expect("forced-break intrinsic sizing screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['horizontal','vertical'].map(id=>{
  const host=document.getElementById(id),parent=host.getBoundingClientRect();
  return [id,{box:[parent.width,parent.height],children:[...host.querySelectorAll('span')].map(child=>{
    const rect=child.getBoundingClientRect();
    return [rect.x-parent.x,rect.y-parent.y,rect.width,rect.height];
  })}];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "horizontal": {
                    "box": [30, 20],
                    "children": [[0, 0, 20, 10], [0, 10, 30, 10]],
                },
                "vertical": {
                    "box": [20, 30],
                    "children": [[0, 0, 10, 20], [10, 0, 10, 30]],
                },
            }),
            "a forced break must end the current intrinsic inline measure before the next atomic box in every writing mode",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("forced-break intrinsic sizing fixture should run");
}
