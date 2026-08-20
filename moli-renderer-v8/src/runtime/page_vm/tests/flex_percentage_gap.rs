use super::*;

fn assert_geometry(
    geometry: &serde_json::Value,
    id: &str,
    expected_size: [f32; 2],
    expected_children: [[f32; 2]; 2],
) {
    let actual = &geometry[id];
    for (axis, expected) in expected_size.into_iter().enumerate() {
        let value = actual["size"][axis]
            .as_f64()
            .unwrap_or_else(|| panic!("missing {id} size axis {axis}: {geometry}"))
            as f32;
        assert!(
            (value - expected).abs() <= 0.05,
            "{id}.size[{axis}]: expected {expected}, got {value}; geometry={geometry}"
        );
    }
    for (child, expected) in expected_children.into_iter().enumerate() {
        for (axis, expected) in expected.into_iter().enumerate() {
            let value = actual["children"][child][axis]
                .as_f64()
                .unwrap_or_else(|| panic!("missing {id} child {child} axis {axis}: {geometry}"))
                as f32;
            assert!(
                (value - expected).abs() <= 0.05,
                "{id}.children[{child}][{axis}]: expected {expected}, got {value}; geometry={geometry}"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_resolves_percentage_flex_gaps_in_logical_axes() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/flex-percentage-gaps.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.container{display:flex;row-gap:20%;column-gap:10%;align-content:start;justify-content:start}
.item{width:50px;height:50px;flex:none}
#row{width:50px;flex-flow:row wrap}
#column{width:50px;flex-flow:column wrap}
#vertical-row{writing-mode:vertical-lr;flex-flow:row wrap}
#vertical-column{writing-mode:vertical-lr;flex-flow:column wrap}
</style>`;
document.body.innerHTML = ['row','column','vertical-row','vertical-column']
  .map(id=>`<div id=${id} class=container><div class=item></div><div class=item></div></div>`)
  .join('');
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 600, 1.0))?
            .expect("percentage flex-gap screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['row','column','vertical-row','vertical-column'].map(id=>{
  const container=document.getElementById(id);
  const c=container.getBoundingClientRect();
  return [id,{
    size:[c.width,c.height],
    children:[...container.children].map(child=>{
      const r=child.getBoundingClientRect();
      return [r.x-c.x,r.y-c.y];
    })
  }];
})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;

        assert_geometry(
            &geometry,
            "row",
            [50.0, 100.0],
            [[0.0, 0.0], [0.0, 50.0]],
        );
        assert_geometry(
            &geometry,
            "column",
            [50.0, 100.0],
            [[0.0, 0.0], [0.0, 50.0]],
        );
        assert_geometry(
            &geometry,
            "vertical-row",
            [100.0, 100.0],
            [[0.0, 0.0], [50.0, 0.0]],
        );
        assert_geometry(
            &geometry,
            "vertical-column",
            [100.0, 50.0],
            [[0.0, 0.0], [50.0, 0.0]],
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("percentage flex-gap fixture should run");
}
