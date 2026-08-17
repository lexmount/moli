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
