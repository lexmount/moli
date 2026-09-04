use super::*;

#[tokio::test(flavor = "current_thread")]
async fn screenshot_expands_grid_auto_fit_from_the_contained_intrinsic_size() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/grid-contained-auto-fit.html")?,
        );
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body{margin:0}
.grid{
  border:3px solid black;
  display:grid;
  contain-intrinsic-size:70px 80px;
  contain:size;
  width:max-content;
  gap:5px;
}
.rows{grid-template:repeat(auto-fit,10px)/3fr 4fr}
.columns{grid-template:1fr 2fr/repeat(auto-fit,15px)}
.both{grid-template:repeat(auto-fit,10px)/repeat(auto-fit,15px)}
.item{height:100%}
</style>`;
const items='<div class=item></div>'.repeat(6);
document.body.innerHTML=`<div class="grid rows">${items}</div><div class="grid columns">${items}</div><div class="grid both">${items}</div>`;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();

        page_vm
            .vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 300, 1.0))?
            .expect("contained Grid fixture must retain a layout root");

        let result = page_vm.vm_mut().eval(
            r#"JSON.stringify((()=>{
const geometry=e=>{const r=e.getBoundingClientRect();return [r.x,r.y,r.width,r.height]};
const grid=e=>({
  rect:geometry(e),
  rows:getComputedStyle(e).gridTemplateRows,
  columns:getComputedStyle(e).gridTemplateColumns,
  items:[...e.children].map(geometry),
});
return {
  rows:grid(document.querySelector('.rows')),
  columns:grid(document.querySelector('.columns')),
  both:grid(document.querySelector('.both')),
};
})())"#,
        )?;
        let result: serde_json::Value = serde_json::from_str(&result)?;

        assert_eq!(result["rows"]["rows"], "10px 10px 10px 0px 0px");
        assert_eq!(
            result["rows"]["columns"]
                .as_str()
                .expect("resolved column tracks")
                .split_ascii_whitespace()
                .count(),
            2
        );
        assert_eq!(result["columns"]["rows"], "25px 50px");
        assert_eq!(result["columns"]["columns"], "15px 15px 15px");
        assert_eq!(result["both"]["rows"], "10px 10px 0px 0px 0px");
        assert_eq!(result["both"]["columns"], "15px 15px 15px");

        for (name, expected) in [
            ("rows", [0.0, 0.0, 76.0, 86.0]),
            ("columns", [0.0, 86.0, 76.0, 86.0]),
            ("both", [0.0, 172.0, 76.0, 86.0]),
        ] {
            let rect = result[name]["rect"]
                .as_array()
                .unwrap_or_else(|| panic!("missing {name} geometry: {result}"));
            for (axis, expected) in expected.into_iter().enumerate() {
                let actual = rect[axis].as_f64().expect("numeric geometry");
                assert!(
                    (actual - expected).abs() <= 0.01,
                    "{name}[{axis}]: expected {expected}, got {actual}; result={result}"
                );
            }
        }

        assert_eq!(
            result["columns"]["items"],
            serde_json::json!([
                [3, 89, 15, 25],
                [23, 89, 15, 25],
                [43, 89, 15, 25],
                [3, 119, 15, 50],
                [23, 119, 15, 50],
                [43, 119, 15, 50],
            ]),
            "contained inline size must determine the three auto-fit columns"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("contained Grid auto-fit fixture should run");
}
