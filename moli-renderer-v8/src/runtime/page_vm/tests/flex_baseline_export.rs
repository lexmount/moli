use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_exports_reversed_flex_baselines_from_flex_flow_endpoints() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-baseline-export.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.target{display:flex;position:relative;line-height:0;font-size:20px;inline-size:200px;margin-block:10px;padding:10px;border:solid 3px}
.inner{display:flex;border:solid 5px;padding:10px}
span{display:inline-block;width:1em;height:1em}
</style>`;
const cases = [
  ['first baseline', 'row'],
  ['last baseline', 'row'],
  ['first baseline', 'row-reverse'],
  ['last baseline', 'row-reverse'],
  ['first baseline', 'column'],
  ['last baseline', 'column'],
  ['first baseline', 'column-reverse'],
  ['last baseline', 'column-reverse'],
];
document.body.innerHTML = cases.map(([alignment,direction], index) => `
  <div id=case-${index} class=target style="align-items:${alignment}">
    <div class=reference><span></span></div>
    <div class=inner style="flex-direction:${direction}">
      <div style="font-size:10px"><span></span></div>
      <div style="font-size:30px"><span></span></div>
    </div>
  </div>`).join('');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(300, 900, 1.0))?
            .expect("nested flex baseline screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(cases.map((_, index) => {
  const target = document.getElementById(`case-${index}`).getBoundingClientRect();
  return [...document.getElementById(`case-${index}`).children].map(child => {
    const rect = child.getBoundingClientRect();
    return rect.y - target.y;
  });
}))"#,
        )?;
        let geometry: Vec<[f32; 2]> = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            [
                [18.0, 13.0],
                [38.0, 13.0],
                [38.0, 13.0],
                [18.0, 13.0],
                [18.0, 13.0],
                [48.0, 13.0],
                [38.0, 13.0],
                [48.0, 13.0],
            ],
            "rect offsets include the target's 3px border; baseline selection must still follow reversed flex-flow order",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested flex baseline fixture should run");
}
