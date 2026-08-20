use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_sideways_writing_modes_through_stretch_layout() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/sideways-writing-mode.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.mode{display:inline-block;vertical-align:top}
.container{width:50px;height:50px;position:relative}
.top{border-top:5px solid}
.right{border-right:5px solid}
.child{margin:1px 3px 5px 7px;font-size:0;writing-mode:horizontal-tb}
</style>`;
const fixture = (mode, border, axis) => `<div id="${mode}-${border}-${axis}" class=mode style="writing-mode:${mode}"><div class="container ${border}"><div class=child style="${axis==='height'?'width:20px;height:stretch':'width:stretch;height:20px'}"></div></div></div>`;
document.body.innerHTML = ['sideways-rl','sideways-lr'].flatMap(mode => ['top','right'].flatMap(border => ['height','width'].map(axis => fixture(mode,border,axis)))).join('');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 200, 1.0))?
            .expect("sideways writing-mode screenshot layout");

        let actual = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('.mode')].map(fixture=>{const child=fixture.querySelector('.child'),rect=child.getBoundingClientRect();return [fixture.id,{writingMode:getComputedStyle(fixture).writingMode,size:[rect.width,rect.height]}]})))"#,
        )?;
        let actual: serde_json::Value = serde_json::from_str(&actual)?;
        assert_eq!(
            actual,
            serde_json::json!({
                "sideways-rl-top-height": {"writingMode": "sideways-rl", "size": [20, 44]},
                "sideways-rl-top-width": {"writingMode": "sideways-rl", "size": [50, 20]},
                "sideways-rl-right-height": {"writingMode": "sideways-rl", "size": [20, 44]},
                "sideways-rl-right-width": {"writingMode": "sideways-rl", "size": [47, 20]},
                "sideways-lr-top-height": {"writingMode": "sideways-lr", "size": [20, 44]},
                "sideways-lr-top-width": {"writingMode": "sideways-lr", "size": [50, 20]},
                "sideways-lr-right-height": {"writingMode": "sideways-lr", "size": [20, 44]},
                "sideways-lr-right-width": {"writingMode": "sideways-lr", "size": [47, 20]},
            }),
            "sideways values and their physical axes must survive the style-to-layout boundary",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("sideways writing-mode fixture should run");
}
