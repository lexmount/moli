use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_absolute_intrinsic_and_stretch_definiteness() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/absolute-intrinsic-sizing.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.host{position:relative;width:200px;height:200px}
.flex{display:flex}
.grid{display:grid}
.absolute{position:absolute;width:100px}
.percentage{height:100%}
</style>`;
const cases = [
  ['fit', 'fit-content', 80, 'inset:0'],
  ['max', 'max-content', 60, 'inset:0'],
  ['min', 'min-content', 40, 'inset:0'],
  ['auto', 'auto', 80, 'inset:0'],
  ['stretch', 'stretch', 80, 'inset:0'],
  ['stretch-top', 'stretch', 80, 'top:10px;left:0'],
  ['stretch-bottom', 'stretch', 80, 'bottom:10px;left:0'],
  ['stretch-static', 'stretch', 80, 'left:0'],
  ['stretch-margin', 'stretch', 80, 'top:10px;left:0;margin-top:7px;margin-bottom:11px'],
];
document.body.innerHTML = ['block', 'flex', 'grid'].flatMap(display =>
  cases.map(([name, height, contentHeight, position]) =>
    `<div class="host ${display}"><div id="${display}-${name}" class="absolute" style="height:${height};${position}"><div class="percentage"><div style="height:${contentHeight}px"></div></div></div></div>`
  )
).join('');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 3_200, 1.0))?
            .expect("absolute intrinsic sizing fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('[id]')].map(element=>{const child=element.firstElementChild;return [element.id,[element.offsetHeight,child.offsetHeight]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for display in ["block", "flex", "grid"] {
            for (name, expected) in [
                ("fit", 80),
                ("max", 60),
                ("min", 40),
                ("auto", 200),
                ("stretch", 200),
                ("stretch-top", 190),
                ("stretch-bottom", 190),
                ("stretch-static", 200),
                ("stretch-margin", 172),
            ] {
                let id = format!("{display}-{name}");
                assert_eq!(
                    geometry[&id],
                    serde_json::json!([expected, expected]),
                    "absolute intrinsic and inset-stretch definiteness must agree for {id}: {geometry}",
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("absolute intrinsic sizing fixture should run");
}
