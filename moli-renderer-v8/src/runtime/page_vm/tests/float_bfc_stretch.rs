use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_stretch_against_the_available_band_beside_a_float() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/float-bfc-stretch.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.fixture{width:200px;height:100px}
.float{float:left;width:100px;height:100px}
.bfc{display:flow-root}
#preferred>.bfc{width:stretch;height:100px}
#range>.float{height:75px}
#range>.bfc{height:25px}
#minimum{width:0;min-width:stretch}
#maximum{width:1000px;max-width:stretch}
#edges{width:stretch;margin-left:10px;padding:0 10px;border:solid transparent;border-width:0 10px}
</style>`;
document.body.innerHTML = `
  <div id=preferred class=fixture><div class=float></div><div class=bfc></div></div>
  <div id=range class=fixture><div class=float></div><div id=minimum class=bfc></div><div id=maximum class=bfc></div><div id=edges class=bfc></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 240, 1.0))?
            .expect("float BFC stretch screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['preferred','minimum','maximum','edges'].map(id=>{const element=id==='preferred'?document.querySelector('#preferred>.bfc'):document.getElementById(id),rect=element.getBoundingClientRect();return [id,[rect.x,rect.y,rect.width,rect.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "preferred": [100, 0, 100, 100],
                "minimum": [100, 100, 100, 25],
                "maximum": [100, 125, 100, 25],
                "edges": [100, 150, 100, 25],
            }),
            "stretch sizing must use the selected float-exclusion opportunity",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("float BFC stretch fixture should run");
}
