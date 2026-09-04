use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_makes_explicit_flex_cross_stretch_definite_for_intrinsic_basis() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-cross-stretch-sizing.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
.container{display:flex;width:50px;height:50px}
.column{flex-direction:column}
.item{min-width:0;min-height:0}
.row .explicit{height:stretch}
.column .explicit{width:stretch}
.row canvas{display:block;height:100%}
.column canvas{display:block;width:100%}
</style>`;
const root=document.createDocumentFragment();
for(const direction of ['row','column']){
  for(const explicit of [false,true]){
    for(const basis of ['auto','content','min-content','fit-content','max-content']){
      const container=document.createElement('div');
      container.className=`container ${direction}`;
      const item=document.createElement('div');
      item.id=`${direction}-${explicit?'explicit':'implicit'}-${basis}`;
      item.className=`item ${explicit?'explicit':''}`;
      item.style.flexBasis=basis;
      const canvas=document.createElement('canvas');
      canvas.width=canvas.height=5;
      item.append(canvas);
      container.append(item);
      root.append(container);
    }
  }
}
document.body.append(root);
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 1_100, 1.0))?
            .expect("flex stretch fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([...document.querySelectorAll('.item')].map(item=>{const itemRect=item.getBoundingClientRect();const canvasRect=item.firstElementChild.getBoundingClientRect();return [item.id,[itemRect.width,itemRect.height,canvasRect.width,canvasRect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for direction in ["row", "column"] {
            for sizing in ["implicit", "explicit"] {
                for basis in ["auto", "content", "min-content", "fit-content", "max-content"] {
                    let id = format!("{direction}-{sizing}-{basis}");
                    assert_eq!(
                        geometry[&id],
                        serde_json::json!([50, 50, 50, 50]),
                        "Chromium-calibrated flex stretch geometry mismatch for {id}: {geometry}"
                    );
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex stretch fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn cross_axis_auto_margins_suppress_stretch_without_making_percentages_definite() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-auto-margin-cross-size.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0;padding:0}
x-flexbox{display:flex;width:100px;height:200px}
#item{margin:auto;background:green}
#percentage{height:100%;width:100px;background:red}
#fixed{height:100px}
</style>`;
document.body.innerHTML = `<x-flexbox><div id="item"><div id="percentage"></div><div id="fixed"></div></div></x-flexbox>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(200, 240, 1.0))?
            .expect("flex auto-margin fixture must retain a layout root");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['item','percentage','fixed'].map(id=>{const rect=document.getElementById(id).getBoundingClientRect();return [id,[rect.x,rect.y,rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "item": [0, 50, 100, 100],
                "percentage": [0, 50, 100, 0],
                "fixed": [0, 50, 100, 100]
            }),
            "cross-axis auto margins must keep an auto block size indefinite for descendants"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("flex auto-margin fixture should run");
}
