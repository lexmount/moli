use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rotated_client_rects_retain_matrix_precision_until_point_mapping() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/transform-precision.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
div{position:absolute;left:0;top:0;width:100px;height:100px}
</style>`;
for (const angle of [0,15,30,45,135,-45]) {
  for (const property of ['transform', 'rotate']) {
    const element = document.createElement('div');
    element.id = property + angle;
    element.style[property] = property === 'transform' ? `rotate(${angle}deg)` : `${angle}deg`;
    document.body.appendChild(element);
  }
}
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 200, 1.0))?
            .expect("transform fixture must retain a layout root");

        // Chromium maps points with a double-precision transform, then rounds
        // the resulting geometry to float. Rounding the matrix first changes
        // x/y even when the final bounding width and height happen to match.
        // Non-right angles exercise matrix coefficient precision. Exact
        // quarter-turn snapping is a separate transform contract.
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const r = document.getElementById('transform45').getClientRects()[0];
return [r.x, r.y, r.width, r.height, r.right, r.bottom].join('|');
})()"#,
            )?,
            "-20.710678100585938|-20.710678100585938|141.42135620117188|141.42135620117188|120.71067810058594|120.71067810058594",
        );
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"(() => {
const geometry = id => {
  const r = document.getElementById(id).getBoundingClientRect();
  return [r.x, r.y, r.width, r.height].join('|');
};
return [0,15,30,45,135,-45].filter(angle =>
  geometry('transform' + angle) !== geometry('rotate' + angle)
).join(',');
})()"#,
            )?,
            "",
            "transform:rotate() and the individual rotate property must map identical points",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("transform precision fixture should run");
}
