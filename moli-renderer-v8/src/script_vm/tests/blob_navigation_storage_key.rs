use super::*;

async fn observe_blob_navigation(
    mode: &str,
    consumer_host: &str,
) -> (
    serde_json::Value,
    Vec<crate::RendererPendingPopupActivation>,
) {
    let fixture = include_str!("../../../tests/fixtures/blob-navigation-storage-key.js");
    let server = StaticHttpServer::spawn_with_bodies(vec![format!(
        "<!doctype html><body><script>{fixture}</script>"
    )])
    .await;
    let origin_url = server.url_for_host("blob-origin.test", "/creator");
    let top_url = server.url_for_host("blob-top.test", "/top");
    let child_url = server.url_for_host(consumer_host, "/consumer");
    let loader = static_http_loader([
        server.resolve_entry("blob-origin.test"),
        server.resolve_entry("blob-top.test"),
        server.resolve_entry("sub.blob-top.test"),
    ]);
    let browser = crate::runtime::RendererBrowserContextRuntime::new();
    let mut creator =
        crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
            origin_url,
            &loader,
            browser.handle(),
        );
    let foreign_url = creator
        .eval("URL.createObjectURL(new Blob(['<p id=marker>foreign</p>'], {type:'text/html'}))")
        .unwrap();
    let mut page = crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
        top_url,
        &loader,
        browser.handle(),
    );
    let parameters = serde_json::json!({
        "mode": mode,
        "foreign": foreign_url,
        "sameSiteSource": consumer_host == "sub.blob-top.test",
    });
    page.eval(&format!(
        r#"
        globalThis.blobNavigationResult = null;
        addEventListener('message', event => {{
          if (event.data?.kind === 'blob-navigation-ready') {{
            event.source.postMessage({parameters}, '*');
          }} else if (event.data?.kind === 'blob-navigation-result') {{
            blobNavigationResult = event.data;
          }}
        }});
        const frame = document.createElement('iframe');
        frame.src = {child:?};
        document.body.appendChild(frame);
        "#,
        child = child_url.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(blobNavigationResult !== null)",
        "true",
        "the cross-site child must report its Blob navigation observations",
    )
    .await;
    let result =
        serde_json::from_str(&page.eval("JSON.stringify(blobNavigationResult)").unwrap()).unwrap();
    let activations = page.take_pending_popup_activations();
    assert_eq!(server.finish_targets().await, ["/consumer"]);
    (result, activations)
}

#[tokio::test]
async fn blob_navigation_storage_keys_block_foreign_child_documents_but_allow_own_blobs() {
    let (result, activations) = observe_blob_navigation("frames", "blob-origin.test").await;
    assert_eq!(result["error"], serde_json::Value::Null, "{result}");
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{result}");
    for row in rows {
        assert_eq!(row["foreignMarker"], serde_json::Value::Null, "{row}");
        assert_eq!(row["ownMarker"], "own", "{row}");
    }
    assert!(activations.is_empty());
}

#[tokio::test]
async fn blob_navigation_storage_keys_apply_top_level_site_noopener_to_all_popup_entries() {
    for (consumer_host, expected_rows, cross_site) in [
        ("blob-origin.test", 9, true),
        ("sub.blob-top.test", 6, false),
    ] {
        let (result, activations) = observe_blob_navigation("popups", consumer_host).await;
        assert_eq!(result["error"], serde_json::Value::Null, "{result}");
        let rows = result["rows"].as_array().unwrap();
        assert_eq!(rows.len(), expected_rows, "{result}");
        assert_eq!(activations.len(), rows.len(), "{result}");
        for (row, activation) in rows.iter().zip(&activations) {
            let suppress_opener = cross_site && row["case"] != "data";
            if row["method"] == "open" {
                assert_eq!(row["nullHandle"], suppress_opener, "{row}");
            }
            let crate::RendererPopupActivationSource::Window { exposes_opener, .. } =
                activation.source()
            else {
                panic!("the actual child callback must retain its Window source");
            };
            assert_eq!(*exposes_opener, !suppress_opener, "{row}");
        }
    }
    // Chromium preserves an existing named Window's opener even when this
    // cross-top-level-site Blob navigation would isolate a newly opened one.
    let (result, _) = observe_blob_navigation("namedPopups", "blob-origin.test").await;
    assert_eq!(result["error"], serde_json::Value::Null, "{result}");
    let rows = result["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{result}");
    for row in rows {
        assert_eq!(row["marker"], "own", "{row}");
        assert_eq!(row["openerPreserved"], true, "{row}");
        if row["method"] == "open" || row["method"] == "frame" {
            assert_eq!(row["nullHandle"], true, "{row}");
        }
    }
}
