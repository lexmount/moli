use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_propagates_document_writing_direction_to_the_layout_view() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let fixtures = [
            (
                "root writing mode",
                ":root{writing-mode:vertical-lr}",
                serde_json::json!({
                    "html": [0, 0, 116, 600],
                    "body": [8, 8, 100, 584],
                    "outer": [8, 8, 100, 584],
                    "abspos": [8, 8, 100, 584],
                    "inline": [8, 8, 100, 50],
                    "probe": [8, 8, 100, 50],
                }),
            ),
            (
                "body writing mode",
                "body{writing-mode:vertical-lr}",
                serde_json::json!({
                    "html": [0, 0, 116, 600],
                    "body": [8, 8, 100, 584],
                    "outer": [8, 8, 100, 584],
                    "abspos": [8, 8, 100, 584],
                    "inline": [8, 8, 100, 50],
                    "probe": [8, 8, 100, 50],
                }),
            ),
            (
                "body layout containment",
                "body{writing-mode:vertical-lr;contain:layout}",
                serde_json::json!({
                    "html": [0, 0, 800, 16],
                    "body": [8, 8, 100, 0],
                    "outer": [8, 8, 100, 0],
                    "abspos": [8, 8, 100, 0],
                    "inline": [8, 8, 100, 50],
                    "probe": [8, 8, 100, 50],
                }),
            ),
            (
                "root style containment",
                ":root{contain:style}body{writing-mode:vertical-lr}",
                serde_json::json!({
                    "html": [0, 0, 800, 16],
                    "body": [8, 8, 100, 0],
                    "outer": [8, 8, 100, 0],
                    "abspos": [8, 8, 100, 0],
                    "inline": [8, 8, 100, 50],
                    "probe": [8, 8, 100, 50],
                }),
            ),
        ];

        for (name, css, expected) in fixtures {
            let mut page_vm = test_page_vm_with_loader_and_document_url(
                &loader,
                Vec::new(),
                Url::parse("https://example.com/viewport-writing-direction.html")?,
            );
            page_vm.vm_mut().eval(&format!(
                r#"
document.head.innerHTML = `<style>{css}</style>`;
document.body.innerHTML = `<div id=outer style="position:relative;padding-left:100px;width:0">
  <div id=abspos style="position:absolute;top:0;left:0;height:100%;width:100%">
    <div id=inline style="display:inline-block;width:100%">
      <div id=probe style="width:100%;height:50px;background:rgb(1,2,3)"></div>
    </div>
  </div>
</div>`;
"installed"
"#,
            ))?;
            page_vm.vm_mut().sync_live_document_style_sources();
            page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
                .expect("writing-direction fixture should produce a snapshot");

            let geometry = page_vm.vm_mut().eval(
                r#"JSON.stringify(Object.fromEntries(['html','body','outer','abspos','inline','probe'].map(id=>{const element=id==='html'?document.documentElement:id==='body'?document.body:document.getElementById(id);const rect=element.getBoundingClientRect();return [id,[rect.x,rect.y,rect.width,rect.height]]})))"#,
            )?;
            let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
            assert_eq!(geometry, expected, "{name}");
        }

        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/body-writing-direction-used-style.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
  html{writing-mode:horizontal-tb;font:20px/20px monospace}
  body{writing-mode:vertical-rl;width:0;height:0}
</style>`;
document.documentElement.append(document.createTextNode('MMMM'));
"installed"
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("root direct-text fixture should produce a snapshot");
        let used_style = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const node=[...document.documentElement.childNodes].find(node=>node.nodeType===Node.TEXT_NODE&&node.data==='MMMM');const range=document.createRange();range.selectNode(node);const rect=range.getBoundingClientRect();return {computed:getComputedStyle(document.documentElement).writingMode,width:rect.width,height:rect.height}})())"#,
        )?;
        let used_style: serde_json::Value = serde_json::from_str(&used_style)?;
        assert_eq!(used_style["computed"], "horizontal-tb");
        let width = used_style["width"].as_f64().expect("numeric text width");
        let height = used_style["height"].as_f64().expect("numeric text height");
        assert!(
            height > width,
            "direct root text must use the propagated vertical LayoutObject style: {used_style}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("viewport writing-direction fixtures should run");
}

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

#[tokio::test(flavor = "current_thread")]
async fn screenshot_remeasures_flex_fit_content_cross_size_after_flexing() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-fit-content-cross-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.container{display:flex;width:100px;align-items:start}
.item{flex:none;font-size:0;line-height:0}
#preferred{height:fit-content}
#minimum{height:0;min-height:fit-content}
.chunk{display:inline-block;width:120px;height:10px;vertical-align:top}
</style>`;
const item = id => `<div class=container><div id=${id} class=item><span class=chunk></span><wbr><span class=chunk></span></div></div>`;
document.body.innerHTML = item('preferred') + item('minimum');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 200, 1.0))?
            .expect("flex fit-content cross-size screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['preferred','minimum'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "preferred": [240, 10],
                "minimum": [240, 10],
            }),
            "content-based cross sizes must use the flexed 240px main size",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex fit-content cross-size fixture should run");
}
