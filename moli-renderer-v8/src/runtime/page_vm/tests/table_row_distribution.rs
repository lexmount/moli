use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_defers_table_block_intrinsic_constraints_until_row_layout() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-block-intrinsic-constraints.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
table{border:2px solid black}
td{border:2px solid lime}
.item{width:50px;height:50px;border:1px solid blue}
.vertical{writing-mode:vertical-lr}
.big-height{height:150px}
.small-height{height:1px}
.big-width{width:150px}
.small-width{width:1px}
.max-height-min{max-height:min-content}
.max-height-max{max-height:max-content}
.min-height-min{min-height:min-content}
.min-height-max{min-height:max-content}
.max-width-min{max-width:min-content}
.max-width-max{max-width:max-content}
.min-width-min{min-width:min-content}
.min-width-max{min-width:max-content}
</style>`;
const table = (id, classes) => `<table id=${id} class="${classes}"><tbody><tr><td><div class=item></div></td></tr></tbody></table>`;
document.body.innerHTML = `<div class=vertical>
${table('vertical-big-min', 'big-width max-width-min')}
${table('vertical-big-max', 'big-width max-width-max')}
${table('vertical-small-min', 'small-width min-width-min')}
${table('vertical-small-max', 'small-width min-width-max')}
</div>
${table('horizontal-big-min', 'big-height max-height-min')}
${table('horizontal-big-max', 'big-height max-height-max')}
${table('horizontal-small-min', 'small-height min-height-min')}
${table('horizontal-small-max', 'small-height min-height-max')}`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))?
            .expect("table intrinsic block-size screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify(Object.fromEntries([
'vertical-big-min','vertical-big-max','vertical-small-min','vertical-small-max',
'horizontal-big-min','horizontal-big-max','horizontal-small-min','horizontal-small-max'
].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return [id,[r.x,r.y,r.width,r.height]]})))"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "vertical-big-min": [0, 0, 150, 66],
                "vertical-big-max": [150, 0, 150, 66],
                "vertical-small-min": [300, 0, 66, 66],
                "vertical-small-max": [366, 0, 66, 66],
                "horizontal-big-min": [0, 66, 66, 150],
                "horizontal-big-max": [0, 216, 66, 150],
                "horizontal-small-min": [0, 366, 66, 66],
                "horizontal-small-max": [0, 432, 66, 66],
            }),
            "table block-axis intrinsic constraints must use their initial behavior before row sizing",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table intrinsic block-size fixture should run");
}

#[tokio::test(flavor = "current_thread")]
async fn screenshot_distributes_table_block_size_by_row_and_section_constraints() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/table-row-distribution.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
table{width:40px;border-collapse:collapse;margin-bottom:4px}
td{padding:0;font:10px/10px sans-serif}
#fixed{height:100px}#fixed tr:first-child td{height:30px}
#percentage{height:100px}#percentage tr:first-child{height:30%}
#constrained{height:120px}#constrained tr:first-child td{height:20px}#constrained tr:last-child td{height:40px}
#sections{height:100px}#sections thead td{height:20px}
#groups{height:120px}#groups tbody:first-of-type{height:80px}
#separated{height:112px;border-collapse:separate;border-spacing:4px}#separated tr:first-child td{height:30px}
</style>`;
const row = id => `<tr id=${id}><td>x</td></tr>`;
document.body.innerHTML = `
<table id=fixed>${row('fixed-a')}${row('fixed-b')}</table>
<table id=percentage>${row('percentage-a')}${row('percentage-b')}</table>
<table id=constrained>${row('constrained-a')}${row('constrained-b')}</table>
<table id=sections><thead id=sections-head>${row('sections-a')}</thead><tbody id=sections-body>${row('sections-b')}</tbody></table>
<table id=groups><tbody id=groups-first>${row('groups-a')}${row('groups-b')}</tbody><tbody id=groups-second>${row('groups-c')}</tbody></table>
<table id=separated>${row('separated-a')}${row('separated-b')}</table>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 800, 1.0))?
            .expect("table row-distribution screenshot layout");

        let geometry = page_vm.vm_mut().eval(
            r#"JSON.stringify({
fixed:['fixed-a','fixed-b'].map(id=>document.getElementById(id).getBoundingClientRect().height),
percentage:['percentage-a','percentage-b'].map(id=>document.getElementById(id).getBoundingClientRect().height),
constrained:['constrained-a','constrained-b'].map(id=>document.getElementById(id).getBoundingClientRect().height),
sections:['sections-a','sections-b','sections-head','sections-body'].map(id=>document.getElementById(id).getBoundingClientRect().height),
groups:['groups-a','groups-b','groups-c','groups-first','groups-second'].map(id=>document.getElementById(id).getBoundingClientRect().height),
separated:(()=>{const table=document.getElementById('separated'),first=document.getElementById('separated-a'),second=document.getElementById('separated-b'),t=table.getBoundingClientRect(),a=first.getBoundingClientRect(),b=second.getBoundingClientRect();return [t.height,a.height,b.height,a.y-t.y,b.y-a.bottom]})()
})"#,
        )?;
        let geometry: serde_json::Value = serde_json::from_str(&geometry)?;
        assert_eq!(
            geometry,
            serde_json::json!({
                "fixed": [30, 70],
                "percentage": [30, 70],
                "constrained": [40, 80],
                "sections": [20, 80, 20, 80],
                "groups": [40, 40, 40, 80, 40],
                "separated": [112, 30, 70, 4, 4],
            }),
            "table row and section excess block-size distribution must match Chromium",
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("table row-distribution fixture should run");
}
