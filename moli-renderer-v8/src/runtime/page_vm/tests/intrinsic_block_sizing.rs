use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_atomic_baselines_during_intrinsic_block_sizing() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-intrinsic-block-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.fixture{height:90px}
.case{display:inline-block;height:80px;border:2px solid;vertical-align:top;font:8px/13px monospace}
.case>span{display:inline-block;width:10px;border:2px solid}
.lr{writing-mode:vertical-lr}
.rl{writing-mode:vertical-rl}
.min{width:min-content}
.max{width:max-content}
</style>`;
const content = `<span>10px<br>atomic baseline contribution.</span><span>10px<br>atomic baseline contribution.</span>`;
document.body.innerHTML = ['lr','rl'].flatMap(mode => ['auto','min','max'].map(size => `<div class=fixture><div id=${mode}-${size} class="case ${mode} ${size}">${content}</div></div>`)).join('');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 600, 1.0))?
            .expect("intrinsic block-size screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['lr-auto','lr-min','lr-max','rl-auto','rl-min','rl-max'].map(id=>{const e=document.getElementById(id),r=e.getBoundingClientRect();return [id,{width:r.width,children:[...e.children].map(c=>{const q=c.getBoundingClientRect();return [q.x-r.x,q.width]})}]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for mode in ["lr", "rl"] {
            let auto = &geometry[format!("{mode}-auto")];
            let auto_width = auto["width"].as_f64().expect("numeric auto width") as f32;
            assert!(
                auto_width > 20.0,
                "fixture must expose baseline-driven block overflow: {geometry}"
            );
            for size in ["min", "max"] {
                let actual = &geometry[format!("{mode}-{size}")];
                let actual_width = actual["width"].as_f64().expect("numeric intrinsic width") as f32;
                assert!(
                    (actual_width - auto_width).abs() <= 0.05,
                    "{mode} {size}-content block size must equal auto layout: {geometry}"
                );
                assert_eq!(
                    actual["children"], auto["children"],
                    "{mode} {size}-content must retain the same atomic baseline placement"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic intrinsic block-sizing fixture should run");
}
