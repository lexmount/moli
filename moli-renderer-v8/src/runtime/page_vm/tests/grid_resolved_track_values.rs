use super::*;
use base64::Engine as _;

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
async fn screenshot_distributes_fit_content_growth_limits_for_spanning_items() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-fit-content-growth-limits.html")?,
        );
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        )));
        page_vm.vm_mut().eval(&format!(
            r#"
document.head.innerHTML = `<style>
@font-face {{ font-family:MoliAhem; src:url(data:font/woff2;base64,{font}) format('woff2') }}
html,body {{ margin:0 }}
.grid {{ display:grid; justify-content:start; align-content:start; font:10px/1 MoliAhem }}
.column {{ width:100px; grid-template-rows:10px 10px; column-gap:5px }}
.column .span {{ grid-column:1 / -1 }}
#column-finite {{ grid-template-columns:fit-content(110px) fit-content(40px) }}
#column-finite .item {{ grid-column:2 }}
#column-shared {{ grid-template-columns:auto fit-content(110px) auto }}
.row {{ width:40px; height:100px; grid-template-columns:10px 10px; row-gap:5px }}
.row > * {{ writing-mode:vertical-lr }}
.row .span {{ grid-row:1 / -1 }}
#row-finite {{ grid-template-rows:fit-content(110px) fit-content(40px) }}
#row-finite .item {{ grid-row:2 }}
#row-shared {{ grid-template-rows:auto fit-content(110px) auto }}
</style>`;
document.body.innerHTML = `
  <div class="grid column" id=column-finite>
    <div class=item>XX</div><div class=span>XXX XXX</div>
  </div>
  <div class="grid column" id=column-shared>
    <div class=span>XXXX XXXX XXXX XXXX</div><div class=span>XXX XXX</div>
  </div>
  <div class="grid row" id=row-finite>
    <div class=item>XX</div><div class=span>XXX XXX</div>
  </div>
  <div class="grid row" id=row-shared>
    <div class=span>XXXX XXXX XXXX XXXX</div><div class=span>XXX XXX</div>
  </div>`;
'installed'
"#,
        ))?;
        page_vm
            .vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
            .expect("fit-content growth-limit screenshot layout");

        let tracks = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{
const read=(id,property)=>getComputedStyle(document.getElementById(id))[property].split(' ').map(parseFloat);
return {
  columnFinite:read('column-finite','gridTemplateColumns'),
  columnShared:read('column-shared','gridTemplateColumns'),
  rowFinite:read('row-finite','gridTemplateRows'),
  rowShared:read('row-shared','gridTemplateRows')
};
})())"#,
        )?;
        let tracks: serde_json::Value = serde_json::from_str(&tracks)?;
        for (name, expected) in [
            ("columnFinite", &[25.0, 12.0][..]),
            ("columnShared", &[30.0, 30.0, 30.0][..]),
            ("rowFinite", &[25.0, 12.0][..]),
            ("rowShared", &[30.0, 30.0, 30.0][..]),
        ] {
            let actual = tracks[name]
                .as_array()
                .unwrap_or_else(|| panic!("missing {name} tracks: {tracks}"));
            assert_eq!(
                actual.len(),
                expected.len(),
                "unexpected {name} track count: {tracks}"
            );
            for (index, expected) in expected.iter().copied().enumerate() {
                let actual = actual[index].as_f64().expect("numeric track size");
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{name}[{index}]: expected {expected}, got {actual}; tracks={tracks}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid fit-content growth-limit fixture should run");
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

#[tokio::test(flavor = "current_thread")]
async fn screenshot_selects_grid_minimum_contributions_from_authored_sizing_sources() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-minimum-contribution-source.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.column{display:grid;width:10px;height:10px;grid-template-columns:minmax(auto,auto);grid-template-rows:10px}
.row{display:grid;width:10px;height:10px;grid-template-columns:10px;grid-template-rows:minmax(auto,auto)}
#fixed>div{width:60px;min-width:90px}
#smaller-min>div{width:60px;min-width:40px}
#ratio>div{width:auto;height:60px;min-width:90px;aspect-ratio:1}
#content-box>div{box-sizing:content-box;width:60px;min-width:90px;padding:0 8px;border:0 solid;border-width:0 5px}
#block>div{width:10px;height:60px;min-height:90px}
.fixed-max{display:grid;width:200px;grid-template-rows:10px}
#fixed-max-floor{grid-template-columns:minmax(auto,0)}
#fixed-max-clamp{grid-template-columns:minmax(auto,20px)}
.fixed-max>div{justify-self:start;margin:0 10px 0 5px;border:0 solid;border-width:0 2px}
.fixed-max span{display:block;width:100px;height:10px}
</style>`;
document.body.innerHTML = `
  <div class=column id=fixed><div></div></div>
  <div class=column id=smaller-min><div></div></div>
  <div class=column id=ratio><div></div></div>
  <div class=column id=content-box><div></div></div>
  <div class=row id=block><div></div></div>
  <div class=fixed-max id=fixed-max-floor><div><span></span></div></div>
  <div class=fixed-max id=fixed-max-clamp><div><span></span></div></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 300, 1.0))?
            .expect("Grid minimum-contribution screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['fixed','smaller-min','ratio','content-box','block','fixed-max-floor','fixed-max-clamp'].map(id=>{
  const grid=document.getElementById(id);
  const child=grid.firstElementChild.getBoundingClientRect();
  const style=getComputedStyle(grid);
  return [id,{columns:style.gridTemplateColumns,rows:style.gridTemplateRows,size:[child.width,child.height]}];
})))"#,
        )?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&geometry)?,
            serde_json::json!({
                "fixed": {"columns": "90px", "rows": "10px", "size": [90, 10]},
                "smaller-min": {"columns": "60px", "rows": "10px", "size": [60, 10]},
                "ratio": {"columns": "90px", "rows": "10px", "size": [90, 60]},
                "content-box": {"columns": "116px", "rows": "10px", "size": [116, 10]},
                "block": {"columns": "10px", "rows": "90px", "size": [10, 90]},
                "fixed-max-floor": {"columns": "19px", "rows": "10px", "size": [104, 10]},
                "fixed-max-clamp": {"columns": "20px", "rows": "10px", "size": [104, 10]},
            }),
            "Grid must preserve authored minimum-contribution provenance and clamp the complete outer automatic minimum without crossing its border floor",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("Grid minimum-contribution fixture should run");
}
