use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_keeps_absolute_calc_terms_in_intrinsic_flex_margins() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/intrinsic-calc-margin.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.probe{display:flex;height:20px}
#mixed,#percentage,#length{width:min-content}
#mixed>i{margin-left:calc(10% + 100px)}
#percentage>i{margin-left:10%}
#length>i{margin-left:100px}
#definite{width:200px}
#definite>i{width:1px;height:1px;margin-left:calc(10% + 100px)}
</style>`;
document.body.innerHTML = `
  <div id=mixed class=probe><i></i></div>
  <div id=percentage class=probe><i></i></div>
  <div id=length class=probe><i></i></div>
  <div id=definite class=probe><i id=definite-child></i></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 100, 1.0))?
            .expect("intrinsic percentage-resolution screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{const rect=id=>document.getElementById(id).getBoundingClientRect();const definite=rect('definite'),child=rect('definite-child');return {mixed:rect('mixed').width,percentage:rect('percentage').width,length:rect('length').width,definite:definite.width,definiteChildOffset:child.x-definite.x}})())"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (name, expected) in [
            ("mixed", 100.0),
            ("percentage", 0.0),
            ("length", 100.0),
            ("definite", 200.0),
            ("definiteChildOffset", 120.0),
        ] {
            let actual = geometry[name]
                .as_f64()
                .unwrap_or_else(|| panic!("missing numeric {name}: {geometry}"))
                as f32;
            assert!(
                (actual - expected).abs() <= 0.05,
                "{name}: expected {expected}, got {actual}; geometry={geometry}"
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("intrinsic calc-margin fixture should run");
}
