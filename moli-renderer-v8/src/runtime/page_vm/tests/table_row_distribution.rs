use super::*;

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
