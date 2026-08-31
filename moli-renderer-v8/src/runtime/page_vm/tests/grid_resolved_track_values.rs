use super::*;

#[tokio::test(flavor = "current_thread")]
async fn computed_style_serializes_used_grid_tracks_from_the_frozen_layout_tree() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-used-track-cssom.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:grid;width:300px}
#intrinsic{grid-template-columns:fit-content(75%)}
#intrinsic>div{width:75px}
#rows{height:100px;grid-template-rows:30px 1fr}
#named{grid-template-columns:[a] 21px [b] repeat(2,[c] 22px [d] 23px [e]) [f] 1fr [g]}
#automatic{grid-template-columns:[a] 21px [b] repeat(auto-fill,[c] 22px [d] 23px [e]) [f] 24px [g]}
#auto-fit{width:44px;grid-template-columns:1px [a] repeat(auto-fit,[b] 20px [c]) [d] 3px}
#implicit{grid-template-columns:none;grid-auto-columns:35px}
#implicit>div{grid-column:1}
#leading{grid-template-columns:[a] 40px [b];grid-auto-columns:15px}
#leading>div{grid-column:-3}
#areas{width:100px;grid-template-areas:'a a';grid-template-columns:none}
#area-repeat{width:100px;grid-template-areas:'a a a a a a a a';grid-template-columns:repeat(auto-fill,20px)}
#fractional{width:100px;grid-template-columns:repeat(3,1fr)}
#zoomed{zoom:2;width:100px;grid-template-columns:1fr 3fr}
#vertical{writing-mode:vertical-rl;width:100px;height:300px;grid-template-columns:1fr 3fr}
</style>`;
document.body.innerHTML = `
  <div class=grid id=intrinsic><div></div></div>
  <div class=grid id=rows></div>
  <div class=grid id=named></div>
  <div class=grid id=automatic></div>
  <div class=grid id=auto-fit></div>
  <div class=grid id=implicit><div></div></div>
  <div class=grid id=leading><div></div></div>
  <div class=grid id=areas></div>
  <div class=grid id=area-repeat></div>
  <div class=grid id=fractional></div>
  <div class=grid id=zoomed></div>
  <div class=grid id=vertical></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
            .expect("used Grid track CSSOM screenshot layout");

        let values = page_vm.vm_mut().eval(
            r#"JSON.stringify({
intrinsic:getComputedStyle(document.getElementById('intrinsic')).gridTemplateColumns,
rows:getComputedStyle(document.getElementById('rows')).gridTemplateRows,
named:getComputedStyle(document.getElementById('named')).gridTemplateColumns,
automatic:getComputedStyle(document.getElementById('automatic')).gridTemplateColumns,
autoFit:getComputedStyle(document.getElementById('auto-fit')).gridTemplateColumns,
implicit:getComputedStyle(document.getElementById('implicit')).gridTemplateColumns,
leading:getComputedStyle(document.getElementById('leading')).gridTemplateColumns,
areas:getComputedStyle(document.getElementById('areas')).gridTemplateColumns,
areaRepeat:getComputedStyle(document.getElementById('area-repeat')).gridTemplateColumns,
fractional:getComputedStyle(document.getElementById('fractional')).gridTemplateColumns,
zoomed:getComputedStyle(document.getElementById('zoomed')).gridTemplateColumns,
vertical:getComputedStyle(document.getElementById('vertical')).gridTemplateColumns
})"#,
        )?;
        let values: serde_json::Value = serde_json::from_str(&values)?;
        assert_eq!(
            values,
            serde_json::json!({
                "intrinsic": "75px",
                "rows": "30px 70px",
                "named": "[a] 21px [b c] 22px [d] 23px [e c] 22px [d] 23px [e f] 189px [g]",
                "automatic": "[a] 21px [b c] 22px [d] 23px [e c] 22px [d] 23px [e c] 22px [d] 23px [e c] 22px [d] 23px [e c] 22px [d] 23px [e f] 24px [g]",
                "autoFit": "1px [a b] 0px [c b] 0px [c d] 3px",
                "implicit": "35px",
                "leading": "15px [a] 40px [b]",
                "areas": "50px 50px",
                "areaRepeat": "20px 20px 20px 20px 20px 0px 0px 0px",
                "fractional": "33.3281px 33.3281px 33.3281px",
                "zoomed": "25px 75px",
                "vertical": "1fr 3fr",
            }),
            "resolved horizontal Grid longhands must expose used tracks while preserving expanded line names, without publishing physical-axis values for vertical Grid",
        );

        page_vm
            .vm_mut()
            .eval("document.getElementById('named').style.cssText='width:400px;grid-template-columns:[new] 1fr 1fr';'mutated'")?;
        assert_eq!(
            page_vm.vm_mut().eval(
                "getComputedStyle(document.getElementById('named')).gridTemplateColumns",
            )?,
            "[a] 21px [b c] 22px [d] 23px [e c] 22px [d] 23px [e f] 189px [g]",
            "a synchronous style read must stay on the last published layout epoch",
        );
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
            .expect("updated used Grid track CSSOM screenshot layout");
        assert_eq!(
            page_vm.vm_mut().eval(
                "getComputedStyle(document.getElementById('named')).gridTemplateColumns",
            )?,
            "[new] 200px 200px",
            "a screenshot must publish the new Grid track sizes",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("used Grid track CSSOM fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_preserves_the_fixed_part_of_cyclic_calc_grid_gaps() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-cyclic-calc-gap.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{display:inline-grid;grid-template:90px 90px/90px 90px}
#calc{gap:calc(20px + 5%)}
#percentage{gap:10%}
</style>`;
document.body.innerHTML = `
  <div class=grid id=calc><div></div><div></div><div></div><div></div></div>
  <div class=grid id=percentage><div></div><div></div><div></div><div></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 300, 1.0))?
            .expect("cyclic Grid gap screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['calc','percentage'].map(id=>{
  const grid=document.getElementById(id);
  const host=grid.getBoundingClientRect();
  const children=Array.from(grid.children, child=>child.getBoundingClientRect());
  return [id,{
    size:[host.width,host.height],
    second:[children[1].left-host.left,children[1].top-host.top],
    third:[children[2].left-host.left,children[2].top-host.top]
  }];
})))"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&geometry)?,
            serde_json::json!({
                "calc": {
                    "size": [200, 200],
                    "second": [120, 0],
                    "third": [0, 120],
                },
                "percentage": {
                    "size": [180, 180],
                    "second": [108, 0],
                    "third": [0, 108],
                },
            }),
            "cyclic percentages contribute zero to intrinsic Grid sizing, but a calc gap must retain its fixed component before resolving its percentage against the used content box",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("cyclic Grid gap fixture should run");
}
