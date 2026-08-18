use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_models_intrinsic_whitespace_across_inline_style_edges() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/inline-intrinsic-decoration.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;font:10px/1 monospace}
.probe{width:min-content;border:5px solid blue}
.probe span{padding:0 10px 0 6px;border-style:solid;border-color:gray;border-width:0 8px 0 5px;margin:0 4px 0 3px}
.clone span{box-decoration-break:clone;-webkit-box-decoration-break:clone}
.slice span{box-decoration-break:slice;-webkit-box-decoration-break:slice}
.nowrap{white-space:nowrap}
.pre{white-space:pre}
.pre-wrap{white-space:pre-wrap}
.break-spaces{white-space:break-spaces}
</style>`;
document.body.innerHTML = `
<div id=clone-adjacent class="probe clone"><span>aaa</span><span>aaa</span></div>
<div id=clone-space class="probe clone"><span>aaa</span> <span>aaa</span></div>
<div id=clone-space-before-end class="probe clone"><span>aaa </span><span>aaa</span></div>
<div id=clone-space-after-start class="probe clone"><span>aaa</span><span> aaa</span></div>
<div id=clone-trailing class="probe clone"><span>aaa </span></div>
<div id=slice-adjacent class="probe slice"><span>aaa</span><span>aaa</span></div>
<div id=slice-space class="probe slice"><span>aaa</span> <span>aaa</span></div>
<div id=clone-nowrap class="probe clone nowrap"><span>aaa</span> <span>aaa</span></div>
<div id=clone-pre class="probe clone pre"><span>aaa</span> <span>aaa</span></div>
<div id=clone-pre-wrap class="probe clone pre-wrap"><span>aaa</span> <span>aaa</span></div>
<div id=clone-break-spaces class="probe clone break-spaces"><span>aaa</span> <span>aaa</span></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?
            .expect("inline intrinsic-decoration screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['clone-adjacent','clone-space','clone-space-before-end','clone-space-after-start','clone-trailing','slice-adjacent','slice-space','clone-nowrap','clone-pre','clone-pre-wrap','clone-break-spaces'].map(id=>[id,document.getElementById(id).getBoundingClientRect().width])))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("clone-adjacent", 118.0),
            ("clone-space", 64.0),
            ("clone-space-before-end", 64.0),
            ("clone-space-after-start", 64.0),
            ("clone-trailing", 64.0),
            ("slice-adjacent", 118.0),
            ("slice-space", 64.0),
            ("clone-nowrap", 124.0),
            ("clone-pre", 124.0),
            ("clone-pre-wrap", 64.0),
            ("clone-break-spaces", 70.0),
        ] {
            let actual = geometry[id]
                .as_f64()
                .unwrap_or_else(|| panic!("missing numeric width for {id}: {geometry}"));
            assert!(
                (actual - expected).abs() <= 0.05,
                "{id}: expected {expected}, got {actual}; geometry={geometry}",
            );
        }

        let line_geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['clone-space','clone-space-before-end','clone-space-after-start','slice-space','clone-pre-wrap','clone-break-spaces','clone-nowrap','clone-pre'].map(id=>{const e=document.getElementById(id);return [id,{client:e.clientWidth,scroll:e.scrollWidth,spans:[...e.querySelectorAll('span')].map(s=>{const r=s.getBoundingClientRect();return [r.x,r.y,r.width,r.height]})}]})))"#,
        )?;
        let line_geometry: serde_json::Value = serde_json::from_str(&line_geometry)?;
        for id in [
            "clone-space",
            "clone-space-before-end",
            "clone-space-after-start",
            "slice-space",
            "clone-pre-wrap",
            "clone-break-spaces",
        ] {
            let probe = &line_geometry[id];
            assert_eq!(probe["scroll"], probe["client"], "{id}: {line_geometry}");
            let spans = probe["spans"].as_array().expect("span geometry array");
            assert_eq!(spans.len(), 2, "{id}: {line_geometry}");
            assert_eq!(spans[0][0], spans[1][0], "{id}: {line_geometry}");
            assert!(
                spans[1][1].as_f64().expect("second span y")
                    > spans[0][1].as_f64().expect("first span y"),
                "{id}: {line_geometry}"
            );
        }
        for id in ["clone-nowrap", "clone-pre"] {
            let spans = line_geometry[id]["spans"]
                .as_array()
                .expect("span geometry array");
            assert_eq!(spans.len(), 2, "{id}: {line_geometry}");
            assert_eq!(spans[0][1], spans[1][1], "{id}: {line_geometry}");
            assert!(
                spans[1][0].as_f64().expect("second span x")
                    > spans[0][0].as_f64().expect("first span x"),
                "{id}: {line_geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("inline intrinsic-decoration fixture should run");
}
