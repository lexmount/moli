use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_preferred_ratios_at_layout_boundaries() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/preferred-aspect-ratio.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.ordinary{display:block;width:100px;border:20px solid}
#ratio-content{aspect-ratio:2/1}
#ratio-border{box-sizing:border-box;aspect-ratio:2/1}
#auto-content{aspect-ratio:auto 2/1}
#auto-border{box-sizing:border-box;aspect-ratio:auto 2/1}
svg{display:block;width:120px;height:auto}
.asymmetric-insets{
  box-sizing:border-box;
  padding:3px 10px;
  border:solid;
  border-width:2px 5px;
}
#natural-auto{aspect-ratio:auto}
#natural-auto-fallback{aspect-ratio:auto 3/2}
#fallback-no-natural{aspect-ratio:auto 3/2}
#specified-border{aspect-ratio:3/2}
#natural-border{aspect-ratio:auto}
#height-only{width:auto;height:50px;aspect-ratio:2/1}
#both-definite{width:130px;height:70px;aspect-ratio:1/1}
#degenerate{aspect-ratio:0/1}
#auto-degenerate{aspect-ratio:auto 0/1}
</style>`;
document.body.innerHTML = `
<div class=ordinary id=ratio-content></div>
<div class=ordinary id=ratio-border></div>
<div class=ordinary id=auto-content></div>
<div class=ordinary id=auto-border></div>
<svg id=natural-auto viewBox="0 0 200 100"></svg>
<svg id=natural-auto-fallback viewBox="0 0 200 100"></svg>
<svg class=asymmetric-insets id=fallback-no-natural></svg>
<svg class=asymmetric-insets id=specified-border viewBox="0 0 200 100"></svg>
<svg class=asymmetric-insets id=natural-border viewBox="0 0 200 100"></svg>
<svg id=height-only viewBox="0 0 200 100"></svg>
<svg id=both-definite viewBox="0 0 200 100"></svg>
<svg id=degenerate viewBox="0 0 200 100"></svg>
<svg id=auto-degenerate viewBox="0 0 200 100"></svg>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(640, 1_400, 1.0))?
            .expect("preferred aspect-ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const rect=element.getBoundingClientRect();return [element.id,[rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("ratio-content", [140, 90]),
            ("ratio-border", [100, 50]),
            ("auto-content", [140, 90]),
            ("auto-border", [100, 70]),
            ("natural-auto", [120, 60]),
            ("natural-auto-fallback", [120, 60]),
            ("fallback-no-natural", [120, 70]),
            ("specified-border", [120, 80]),
            ("natural-border", [120, 55]),
            ("height-only", [100, 50]),
            ("both-definite", [130, 70]),
            ("degenerate", [120, 60]),
            ("auto-degenerate", [120, 60]),
        ] {
            assert_eq!(
                geometry[id],
                serde_json::json!(expected),
                "Chromium-calibrated geometry mismatch for {id}: {geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("preferred aspect-ratio fixture should run");
}

/// Regression for WPT css/css-sizing/aspect-ratio/table-element-001.html.
#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_preferred_ratios_out_of_internal_table_box_sizing() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/internal-table-box-aspect-ratio.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;background:white}
table{border-collapse:collapse}
th,td{padding:0}
</style>`;
document.body.innerHTML = `
<table id=internal>
  <tr id=row>
    <th id=cell style="background:green;width:100px;aspect-ratio:1/1"></th>
    <td id=empty-a style="background:red;height:50px;aspect-ratio:4/1"></td>
    <td id=empty-b style="background:red;height:50px;min-width:min-content;aspect-ratio:4/1"></td>
  </tr>
</table>
<table id=wrapper style="background:green;width:100px;aspect-ratio:2/1"></table>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        let snapshot = page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(220, 120, 1.0))?
            .expect("internal-table-box aspect-ratio fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['internal','row','cell','empty-a','empty-b','wrapper'].map(id=>{const element=document.getElementById(id);const rect=element.getBoundingClientRect();return [id,{rect:[rect.x,rect.y,rect.width,rect.height],ratio:getComputedStyle(element).aspectRatio}]})))"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&geometry)?,
            serde_json::json!({
                "internal": {"rect": [0, 0, 100, 50], "ratio": "auto"},
                "row": {"rect": [0, 0, 100, 50], "ratio": "auto"},
                "cell": {"rect": [0, 0, 100, 50], "ratio": "1 / 1"},
                "empty-a": {"rect": [100, 0, 0, 50], "ratio": "4 / 1"},
                "empty-b": {"rect": [100, 0, 0, 50], "ratio": "4 / 1"},
                "wrapper": {"rect": [0, 50, 100, 50], "ratio": "2 / 1"},
            }),
            "internal table boxes must retain computed ratios without consuming them as used sizes, while the table wrapper still consumes its ratio",
        );

        let raster = moli_paint::raster_snapshot(&snapshot)?;
        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let offset = ((y * raster.width + x) * 4) as usize;
            raster.rgba[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(99, 99), [0, 128, 0, 255]);
        assert_eq!(pixel(100, 99), [255, 255, 255, 255]);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("internal-table-box aspect-ratio fixture should run");
}
