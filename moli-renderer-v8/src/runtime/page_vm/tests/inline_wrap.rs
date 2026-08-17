use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_matches_chromium_atomic_wrap_state_across_inline_edges() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/atomic-inline-wrap-state.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.case{font-size:0;line-height:0;margin-bottom:10px}
.atomic{display:inline-block;width:64px;height:20px;vertical-align:top}
.edge{padding-inline:8px}
#root{width:192px;white-space:nowrap}
#decorated{width:64px;white-space:normal}
#nested{width:100px;white-space:normal}
#nested .edge{white-space:nowrap}
#restored{width:64px;white-space:nowrap}
#restored .edge{white-space:normal}
.text-case{font:20px/20px monospace}
#text-close{width:64px;white-space:nowrap}
#text-close .edge{white-space:normal}
#text-open{width:64px;white-space:normal}
#text-open .edge{white-space:nowrap}
</style>`;
document.body.innerHTML = `
<div id=root class=case><i id=r1 class=atomic></i><i id=r2 class=atomic></i><i id=r3 class=atomic></i><i id=r4 class=atomic></i></div>
<div id=decorated class=case><span class=edge><i id=d1 class=atomic></i></span><i id=d2 class=atomic></i></div>
<div id=nested class=case><span class=edge><i id=n1 class=atomic></i><i id=n2 class=atomic></i></span><i id=n3 class=atomic></i></div>
<div id=restored class=case><span class=edge><i id=s1 class=atomic></i><i id=s2 class=atomic></i></span><i id=s3 class=atomic></i></div>
<div id=text-close class="case text-case"><span class=edge>x</span><i id=t1 class=atomic></i></div>
<div id=text-open class="case text-case">x<span class=edge><i id=t2 class=atomic></i></span></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 340, 1.0))?
            .expect("atomic inline wrap-state screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"
JSON.stringify(Object.fromEntries(
  ['r1','r2','r3','r4','d1','d2','n1','n2','n3','s1','s2','s3','t1','t2'].map(id => {
    const element = document.getElementById(id);
    const rect = element.getBoundingClientRect();
    const container = element.closest('.case').getBoundingClientRect();
    return [id, [rect.x-container.x, rect.y-container.y, rect.width, rect.height]];
  })
))
"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("r1", [0.0, 0.0, 64.0, 20.0]),
            ("r2", [64.0, 0.0, 64.0, 20.0]),
            ("r3", [128.0, 0.0, 64.0, 20.0]),
            ("r4", [192.0, 0.0, 64.0, 20.0]),
            ("d1", [8.0, 0.0, 64.0, 20.0]),
            ("d2", [0.0, 20.0, 64.0, 20.0]),
            ("n1", [8.0, 0.0, 64.0, 20.0]),
            ("n2", [72.0, 0.0, 64.0, 20.0]),
            ("n3", [0.0, 20.0, 64.0, 20.0]),
            ("s1", [8.0, 0.0, 64.0, 20.0]),
            ("s2", [0.0, 20.0, 64.0, 20.0]),
            ("s3", [0.0, 40.0, 64.0, 20.0]),
            ("t1", [0.0, 20.0, 64.0, 20.0]),
            ("t2", [8.0, 20.0, 64.0, 20.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (index, expected) in expected.into_iter().enumerate() {
                let actual = actual[index].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{index}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("atomic inline wrap-state fixture should run");
}
