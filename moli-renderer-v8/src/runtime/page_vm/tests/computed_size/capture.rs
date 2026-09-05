use super::*;

#[tokio::test(flavor = "current_thread")]
async fn computed_size_dom_snapshot_capture_preserves_rows_and_never_starts_layout() {
    run_page_vm_async_test(async move {
        let mut page = page_with_size_fixture(r#"
<section style="width:500px;height:400px;max-width:120px;max-height:80px"></section>
<article style="writing-mode:vertical-rl;width:500px;height:400px;max-width:90px;max-height:30px"></article>
"#)?;
        for sampled in [false, true] {
            if sampled { publish_size_layout(&mut page)?; }
            let cache = page.vm().layout_snapshot_cache_observability_for_test();
            let passes = page.vm().layout_pass_observability_for_test().1;
            let payload = page.dom_snapshot_capture_payload("top", crate::runtime::RendererDomSnapshotCaptureOptions {
                computed_styles: ["width","height","inline-size","block-size"].map(String::from).to_vec(),
                ..Default::default()
            }).expect("Document capture").into_protocol_payload();
            let strings = payload["strings"].as_array().expect("string table");
            let decode = |index: &serde_json::Value| strings[index.as_u64().unwrap() as usize].as_str().unwrap();
            let document = &payload["documents"][0];
            let nodes = document["nodes"]["nodeName"].as_array().expect("node names");
            let layout = &document["layout"];
            let indices = layout["nodeIndex"].as_array().expect("layout indices");
            assert_eq!(indices.len(), layout["styles"].as_array().unwrap().len());
            for (tag, expected) in if sampled {
                [("SECTION", ["120px","80px","120px","80px"]), ("ARTICLE", ["90px","30px","30px","90px"])]
            } else {
                [("SECTION", ["500px","400px","500px","400px"]), ("ARTICLE", ["500px","400px","400px","500px"])]
            } {
                let node = nodes.iter().position(|index| decode(index) == tag).expect("fixture element");
                let row = indices.iter().position(|index| index.as_u64() == Some(node as u64)).expect("element style row");
                let values = layout["styles"][row].as_array().unwrap().iter().map(decode).collect::<Vec<_>>();
                assert_eq!(values, expected, "{tag}, sampled={sampled}");
            }
            assert_eq!(page.vm().layout_snapshot_cache_observability_for_test(), cache);
            assert_eq!(page.vm().layout_pass_observability_for_test().1, passes);
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("DOMSnapshot rows must consume the same demand-driven size projection");
}
