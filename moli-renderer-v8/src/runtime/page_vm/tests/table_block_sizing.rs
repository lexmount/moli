use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_rowmin_as_the_table_block_size_floor() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-block-sizing.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border-spacing:0}
td{padding:0}
.cell{width:50px;height:50px}
#large-max{height:150px;max-height:max-content}
#small-min{height:1px;min-height:min-content}
#natural-max{max-height:0}
#minimum-wins{height:1px;min-height:100px;max-height:40px}
.vertical{writing-mode:vertical-lr}
#vertical-large-max{width:150px;max-width:max-content}
#vertical-small-min{width:1px;min-width:min-content}
#content-box-natural{box-sizing:content-box;padding:5px;border:3px solid;max-height:50px}
#content-box-constrained{box-sizing:content-box;padding:5px;border:3px solid;height:100px;max-height:80px}
#content-box-natural .cell,#content-box-constrained .cell{height:75px}
</style>`;
const table = (id, className='') => `<table id=${id} class=${className}><tr><td><div class=cell></div></td></tr></table>`;
document.body.innerHTML =
  table('large-max') +
  table('small-min') +
  table('natural-max') +
  table('minimum-wins') +
  table('vertical-large-max', 'vertical') +
  table('vertical-small-min', 'vertical') +
  table('content-box-natural') +
  table('content-box-constrained');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 700, 1.0))?
            .expect("table block-sizing screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['large-max','small-min','natural-max','minimum-wins','vertical-large-max','vertical-small-min','content-box-natural','content-box-constrained'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("large-max", [50.0, 150.0]),
            ("small-min", [50.0, 50.0]),
            ("natural-max", [50.0, 50.0]),
            ("minimum-wins", [50.0, 100.0]),
            ("vertical-large-max", [150.0, 50.0]),
            ("vertical-small-min", [50.0, 50.0]),
            ("content-box-natural", [66.0, 91.0]),
            ("content-box-constrained", [66.0, 96.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{axis}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table block-sizing fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_aligns_table_cell_contents_and_static_positions_like_chromium() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-cell-content-alignment.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
#container{position:relative;width:900px;height:120px}
table{border-spacing:0}
td{width:140px;height:70px;padding:7px 11px;border:3px solid}
.prefix{height:20px}
.abs{position:absolute;width:20px;height:10px}
#top{vertical-align:top}
#bottom{vertical-align:bottom}
#end-overrides-top{vertical-align:top;align-content:end}
#start-overrides-bottom{vertical-align:bottom;align-content:start}
</style>`;
const cell = id => `<td id=${id}><div id=${id}-prefix class=prefix></div><div id=${id}-abs class=abs></div></td>`;
document.body.innerHTML = `<div id=container><table><tbody><tr>${[
  'middle','top','bottom','end-overrides-top','start-overrides-bottom'
].map(cell).join('')}</tr></tbody></table></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(1000, 200, 1.0))?
            .expect("table-cell content-alignment screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['middle','top','bottom','end-overrides-top','start-overrides-bottom'].map(id=>{
const cell=document.getElementById(id),cellRect=cell.getBoundingClientRect();
const prefix=document.getElementById(`${id}-prefix`).getBoundingClientRect();
const absolute=document.getElementById(`${id}-abs`).getBoundingClientRect();
return [id,{alignContent:getComputedStyle(cell).alignContent,cellHeight:cellRect.height,prefix:[prefix.x-cellRect.x,prefix.y-cellRect.y],absolute:[absolute.x-cellRect.x,absolute.y-cellRect.y]}];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, align_content, content_y, absolute_y) in [
            ("middle", "normal", 35.0, 55.0),
            ("top", "normal", 10.0, 30.0),
            ("bottom", "normal", 60.0, 80.0),
            ("end-overrides-top", "end", 60.0, 80.0),
            ("start-overrides-bottom", "start", 10.0, 30.0),
        ] {
            let actual = &geometry[id];
            assert_eq!(actual["alignContent"], align_content, "{id}: {geometry}");
            let cell_height = actual["cellHeight"].as_f64().expect("numeric cell height") as f32;
            assert!(
                (cell_height - 90.0).abs() <= 0.05,
                "{id}: expected a 90px cell, got {cell_height}; geometry={geometry}"
            );
            for (kind, expected) in [("prefix", [14.0, content_y]), ("absolute", [14.0, absolute_y])] {
                let point = actual[kind]
                    .as_array()
                    .unwrap_or_else(|| panic!("{id}: missing {kind} geometry: {geometry}"));
                for (axis, expected) in expected.into_iter().enumerate() {
                    let coordinate = point[axis].as_f64().expect("numeric coordinate") as f32;
                    assert!(
                        (coordinate - expected).abs() <= 0.05,
                        "{id} {kind}[{axis}]: expected {expected}, got {coordinate}; geometry={geometry}"
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table-cell content-alignment fixture should run");
}
