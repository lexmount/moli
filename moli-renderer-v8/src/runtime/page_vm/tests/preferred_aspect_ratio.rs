use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_the_content_box_for_auto_preferred_aspect_ratios() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/preferred-aspect-ratio-box.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.item{display:block;border:20px solid;width:100px}
.ratio{aspect-ratio:2/1}
.auto-ratio{aspect-ratio:auto 2/1}
.border-box{box-sizing:border-box}
</style>`;
document.body.innerHTML = `
  <div id=ratio-content class="item ratio"></div>
  <div id=ratio-border class="item ratio border-box"></div>
  <div id=auto-content class="item auto-ratio"></div>
  <div id=auto-border class="item auto-ratio border-box"></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 400, 1.0))?
            .expect("preferred aspect-ratio screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['ratio-content','ratio-border','auto-content','auto-border'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [
            ("ratio-content", [140.0, 90.0]),
            ("ratio-border", [100.0, 50.0]),
            ("auto-content", [140.0, 90.0]),
            ("auto-border", [100.0, 70.0]),
        ] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{axis}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("preferred aspect-ratio fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_ignores_preferred_aspect_ratios_on_internal_table_boxes() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-internal-aspect-ratio.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border-collapse:collapse}
th,td{padding:0}
</style>`;
document.body.innerHTML = `
  <table id=internal>
    <tr><th style="width:100px;aspect-ratio:1/1"></th><td id=internal-cell style="height:50px;aspect-ratio:4/1"></td><td style="height:50px;min-width:min-content;aspect-ratio:4/1"></td></tr>
  </table>
  <table id=outer style="width:100px;aspect-ratio:2/1"></table>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 200, 1.0))?
            .expect("internal table aspect-ratio screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['internal','outer'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        for (id, expected) in [("internal", [100.0, 50.0]), ("outer", [100.0, 50.0])] {
            let actual = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = actual[axis].as_f64().expect("numeric geometry") as f32;
                assert!(
                    (actual - expected).abs() <= 0.05,
                    "{id}[{axis}]: expected {expected}, got {actual}; geometry={geometry}"
                );
            }
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("getComputedStyle(document.getElementById('internal-cell')).aspectRatio")?,
            "4 / 1"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("internal table aspect-ratio fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_applies_aspect_ratio_automatic_minimum_to_menu_list_selects() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/select-aspect-ratio-minimum.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
select{margin:0}
.ratio{height:50px;aspect-ratio:1/1}
</style>`;
document.body.innerHTML = `
  <select id=empty class=ratio><option></option></select><br>
  <select id=zero class=ratio style="min-width:0"><option>The long text is selected</option></select><br>
  <select id=auto class=ratio><option>The long text is selected</option></select><br>
  <select id=widest><option selected>x</option><option>The long text is selected</option></select>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(480, 260, 1.0))?
            .expect("menu-list select aspect-ratio screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['empty','zero','auto','widest'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let size = |id: &str| {
            let values = geometry[id]
                .as_array()
                .unwrap_or_else(|| panic!("missing geometry for {id}: {geometry}"));
            [
                values[0].as_f64().expect("numeric width") as f32,
                values[1].as_f64().expect("numeric height") as f32,
            ]
        };
        let empty = size("empty");
        let zero = size("zero");
        let auto = size("auto");
        let widest = size("widest");
        for (id, actual) in [("empty", empty), ("zero", zero)] {
            assert!(
                (actual[0] - 50.0).abs() <= 0.05 && (actual[1] - 50.0).abs() <= 0.05,
                "{id}: expected 50x50, got {actual:?}; geometry={geometry}"
            );
        }
        assert!(
            auto[0] > 50.0 && (auto[1] - 50.0).abs() <= 0.05,
            "auto minimum must preserve intrinsic option width, got {auto:?}; geometry={geometry}"
        );
        assert!(
            (widest[0] - auto[0]).abs() <= 0.05,
            "a non-selected widest option must define the menu-list intrinsic width; auto={auto:?}, widest={widest:?}"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("getComputedStyle(document.getElementById('auto')).minWidth")?,
            "auto"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("menu-list select aspect-ratio fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_uses_distinct_menu_list_and_listbox_layout_objects() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/select-layout-object-kinds.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.column{display:flex;flex-direction:column;width:100px}
.wide{width:300px}
</style>`;
document.body.innerHTML = `
  <select id=menu><option>one</option><option>longest option</option></select>
  <select id=popup multiple size=1><option>one</option><option>two</option><option>three</option></select>
  <div class=column><select id=list multiple><option>one</option><option>two</option><option>three</option></select></div>
  <select id=sized size=4><option>one</option><option>two</option><option>three</option></select>
  <div class=wide><select id=block-list size=4 style="display:block"><option>one</option><option>two</option><option>three</option></select></div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(480, 300, 1.0))?
            .expect("select layout-object screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries(['menu','popup','list','sized','block-list'].map(id=>{const e=document.getElementById(id),r=e.getBoundingClientRect(),o=e.options[0].getBoundingClientRect();return [id,{box:[r.width,r.height],option:[o.width,o.height],display:getComputedStyle(e).display}]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        let pair = |id: &str, member: &str| {
            let values = geometry[id][member]
                .as_array()
                .unwrap_or_else(|| panic!("missing {id}.{member}: {geometry}"));
            [
                values[0].as_f64().expect("numeric width") as f32,
                values[1].as_f64().expect("numeric height") as f32,
            ]
        };
        let menu_option = pair("menu", "option");
        let popup = pair("popup", "box");
        let popup_option = pair("popup", "option");
        let list = pair("list", "box");
        let list_option = pair("list", "option");
        let sized = pair("sized", "box");
        let sized_option = pair("sized", "option");
        let block_list = pair("block-list", "box");
        let block_list_option = pair("block-list", "option");

        for (id, expected) in [
            ("menu", "inline-block"),
            ("popup", "inline-block"),
            // CSS Display blockifies an inline-block that becomes a flex item.
            ("list", "block"),
            ("sized", "inline-block"),
            ("block-list", "block"),
        ] {
            assert_eq!(geometry[id]["display"], expected, "{id}");
        }
        for (id, option) in [("menu", menu_option), ("popup", popup_option)] {
            assert_eq!(
                option,
                [0.0, 0.0],
                "{id} must expose browser-owned menu-list content, not option layout boxes"
            );
        }
        for (id, option) in [
            ("list", list_option),
            ("sized", sized_option),
            ("block-list", block_list_option),
        ] {
            assert!(
                option[0] > 0.0 && option[1] > 0.0,
                "{id} must retain real listbox option layout; geometry={geometry}"
            );
        }
        assert!(
            (list[0] - 100.0).abs() <= 0.05,
            "a listbox must participate in flex cross-axis stretch; geometry={geometry}"
        );
        assert!(
            (block_list[0] - sized[0]).abs() <= 0.05 && block_list[0] < 300.0,
            "a block-context listbox must retain its intrinsic inline size; geometry={geometry}"
        );
        assert!(
            popup[1] < sized[1],
            "explicit multiple size=1 must remain a popup rather than a listbox; geometry={geometry}"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("select layout-object fixture should run");
}
