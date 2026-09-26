use super::*;

#[tokio::test(flavor = "current_thread")]
async fn linked_stylesheet_client_terminal_installs_source_before_its_load_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://stylesheet-client-terminal.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/css,.target%7Bcolor%3Argb(1%2C2%2C3)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__linkedStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __linkedStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __linkedStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("linked stylesheet listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("data stylesheet terminal should reach its Page Networking source")
    );

    assert_eq!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("linked stylesheet computed color"),
        "rgb(1, 2, 3)",
        "the exact client terminal must install the retained response body"
    );
    assert_eq!(
        vm.eval("__linkedStyleEvents.join(',')")
            .expect("pre-event linked stylesheet state"),
        "",
        "source installation must not synchronously dispatch the link event"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "linked stylesheet load event body should be ready"
    );
    assert_eq!(
        vm.eval("__linkedStyleEvents.join(',')")
            .expect("linked stylesheet event state"),
        "load"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn failed_linked_stylesheet_client_terminal_installs_empty_source_before_its_error_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://failed-stylesheet-client-terminal.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/plain,.target%7Bcolor%3Argb(11%2C12%2C13)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__failedLinkedStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __failedLinkedStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __failedLinkedStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("failed linked stylesheet listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("failed data stylesheet terminal should reach its Page Networking source")
    );

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              sheetIsNull: document.getElementById("sheet").sheet === null,
              styleSheetCount: document.styleSheets.length,
              color: getComputedStyle(document.querySelector(".target")).color,
              events: __failedLinkedStyleEvents,
            })"#,
        )
        .expect("failed linked stylesheet state"),
        r#"{"sheetIsNull":false,"styleSheetCount":1,"color":"rgb(0, 0, 0)","events":[]}"#,
        "an unusable typed terminal must install an empty stylesheet without applying its body or synchronously dispatching"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "failed linked stylesheet error event body should be ready"
    );
    assert_eq!(
        vm.eval("__failedLinkedStyleEvents.join(',')")
            .expect("failed linked stylesheet event state"),
        "error"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn style_preload_client_terminal_dispatches_load_without_installing_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://stylesheet-preload-client.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='preload' rel='preload' as='style' ",
                "href='data:text/css,.target%7Bcolor%3Argb(7%2C8%2C9)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__stylePreloadEvents = [];
        document.getElementById("preload").addEventListener("load", () => {
          __stylePreloadEvents.push("load");
        });
        document.getElementById("preload").addEventListener("error", () => {
          __stylePreloadEvents.push("error");
        });
        "#,
        None,
    )
    .expect("style preload listeners should install");

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_page_task_executor_work_arrival_for_test(),
        )
        .await
        .expect("style preload terminal should wake the Page executor"),
        "style preload terminal should publish a Page task"
    );
    assert!(
        vm.apply_next_stylesheet_networking_body_for_test(),
        "style preload terminal should reach its Page Networking source"
    );

    assert_ne!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("preload target computed color"),
        "rgb(7, 8, 9)",
        "a preload client must not install the retained CSS source"
    );
    assert_eq!(
        vm.eval("String(document.styleSheets.length)")
            .expect("stylesheet list length"),
        "0"
    );
    assert_eq!(
        vm.eval("__stylePreloadEvents.join(',')")
            .expect("pre-event style preload state"),
        "",
        "the preload event must remain asynchronous"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "style preload event body should be ready"
    );
    assert_eq!(
        vm.eval("__stylePreloadEvents.join(',')")
            .expect("style preload event state"),
        "load"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn ownerless_stylesheet_terminal_installs_for_a_late_link_client() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let stylesheet_url =
        Url::parse("data:text/css,.target%7Bcolor%3Argb(4%2C5%2C6)%7D").expect("stylesheet URL");
    let (mut vm, _resource_completion_queue) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            "https://ownerless-stylesheet.test/page.html",
            concat!(
                "<!doctype html><html><head>",
                "<link id='sheet' rel='stylesheet' ",
                "href='data:text/css,.target%7Bcolor%3Argb(4%2C5%2C6)%7D'>",
                "</head><body><div class='target'></div></body></html>",
            ),
            &loader,
        );
    vm.exec(
        r#"
        globalThis.__ownerlessStyleEvents = [];
        document.getElementById("sheet").addEventListener("load", () => {
          __ownerlessStyleEvents.push("load");
        });
        document.getElementById("sheet").addEventListener("error", () => {
          __ownerlessStyleEvents.push("error");
        });
        "#,
        None,
    )
    .expect("linked stylesheet listeners should install");

    let speculative_fetch = vm
        .document_runtime
        .preload_stylesheet(
            stylesheet_url,
            crate::stylesheet_blocking::StylesheetFetchOptions::default(),
        )
        .expect("response CSP should admit the ownerless resource");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
        )
        .await
        .expect("ownerless data stylesheet should reach its Page Networking source")
    );
    assert!(
        speculative_fetch
            .terminal()
            .is_some_and(|terminal| terminal.is_ready())
    );

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    assert!(
        !vm.apply_next_stylesheet_networking_body_for_test(),
        "late client attachment must not enqueue another physical network terminal"
    );
    assert_eq!(
        vm.eval("getComputedStyle(document.querySelector('.target')).color")
            .expect("late linked stylesheet computed color"),
        "rgb(4, 5, 6)"
    );
    assert_eq!(
        vm.eval("__ownerlessStyleEvents.join(',')")
            .expect("pre-event linked stylesheet state"),
        "",
        "late terminal delivery must keep the link event asynchronous"
    );
    assert!(
        vm.apply_next_connected_style_event_body_for_test(),
        "linked stylesheet load event body should be ready"
    );
    assert_eq!(
        vm.eval("__ownerlessStyleEvents.join(',')")
            .expect("linked stylesheet event state"),
        "load"
    );
}
#[test]
fn connected_stylesheet_plan_commits_its_lease_before_same_turn_apply() {
    let mut vm = new_parsed_test_vm(
        "https://style-plan-commit.test/",
        concat!(
            "<!doctype html><html><head>",
            "<style id='sheet'>body { color: black; }</style>",
            "</head><body></body></html>",
        ),
    );
    let owner = vm
        .current_main_document_task_owner()
        .expect("main Document owner");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    vm.replace_document_resource_runtime(&loader);
    let mut prepared = vm.document_runtime.prepare_initial_connected_style_loads();
    assert_eq!(prepared.len(), 1);
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "the pure prepare phase must not touch the Document load gate"
    );

    let prepared = prepared.pop().expect("one prepared style owner");
    let inline_source = vm
        ._context_host
        .borrow()
        .owner_style_sheet_processing_source(prepared.owner());
    let admission = vm
        ._context_host
        .borrow_mut()
        .commit_connected_style_load_event_plan(prepared.event_plan())
        .expect("the current main Document should commit the style lease");
    assert_eq!(
        vm.current_main_document_has_style_load_event_delay(owner),
        Some(true),
        "commit must acquire the lease synchronously, before runtime apply"
    );

    let host_ptr = vm._context_host.as_ref().as_ptr();
    vm.document_runtime.apply_prepared_connected_style_load(
        prepared,
        inline_source,
        admission,
        host_ptr,
    );
    let ready = vm
        .take_next_connected_style_event_body_for_test()
        .expect("same-turn apply should publish the inline style event")
        .into_ready();
    assert_eq!(
        ready
            .load_event_binding()
            .expect("stylesheet ready event must retain the committed lease")
            .owner(),
        owner
    );
}
#[tokio::test]
async fn web_font_requests_and_registration_follow_effective_stylesheet_media() {
    let (font_url, mut request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let document_url = font_url.replace("/print-only.woff2", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  const style = document.createElement("style");
  style.media = "print";
  style.textContent = `
    @font-face {{ font-family: PrintOnly; src: url({font_url:?}); }}
  `;
  (document.head || document.documentElement || document).appendChild(style);
  (document.body || document.documentElement).style.fontFamily = "PrintOnly, sans-serif";
}})()
"#
    ))
    .expect("print-only web-font fixture should evaluate");

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("screen layout refresh should succeed")
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "a print-only font must not create a screen-media request or load delay"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut request_rx)
            .await
            .is_err(),
        "the server must not observe a request while owner media is ineffective"
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("print layout refresh should succeed")
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1,
        "activating print media must admit exactly one font request"
    );
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), &mut request_rx)
        .await
        .expect("the effective font request should reach the server")
        .expect("font request channel should remain open");
    assert!(request.starts_with("GET /print-only.woff2 HTTP/1.1"));

    release_tx.send(()).expect("release font response");
    server.await.expect("font server should finish");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("web font completion should reach the Networking source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("web font completion must retain its typed terminal");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("web font completion should apply to its current document owner");
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 1, 1),
        "an effective completed font must be registered for layout"
    );

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("restored screen layout refresh should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (0, 0, 0),
        "leaving print media must remove the slot and registered layout font"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}
#[tokio::test(flavor = "current_thread")]
async fn imported_web_font_keeps_its_response_base_slot_through_layout_reconciliation() {
    let (base_url, mut requests, font_release, shutdown, server) =
        spawn_imported_font_graph_server().await;
    let document_url = base_url.join("page.html").expect("document URL");
    let root_stylesheet_url = base_url.join("css/root.css").expect("root stylesheet URL");
    let html = format!(
        concat!(
            "<!doctype html><html><head>",
            "<link rel='stylesheet' href='{}'>",
            "</head><body><span id='font-probe'>MMMM</span></body></html>",
        ),
        root_stylesheet_url,
    );
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_parsed_test_vm_with_loader_and_resource_completion_queue(
            document_url.as_str(),
            &html,
            &loader,
        );

    vm.queue_initial_connected_style_loads_for_current_owner();
    vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
    for phase in ["root stylesheet", "import graph"] {
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                vm.wait_for_and_apply_stylesheet_networking_body_for_test(),
            )
            .await
            .unwrap_or_else(|_| panic!("{phase} completion timed out")),
            "{phase} completion should reach the stylesheet task source",
        );
    }

    let mut observed_paths = Vec::new();
    while !observed_paths
        .iter()
        .any(|path| path == "/theme/fonts/imported.woff2")
    {
        let path = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
            .await
            .expect("expected imported stylesheet/font request")
            .expect("imported-font server request stream");
        observed_paths.push(path);
    }
    assert!(observed_paths.iter().any(|path| path == "/css/root.css"));
    assert!(
        observed_paths
            .iter()
            .any(|path| path == "/theme/imported.css")
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("layout reconciliation should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 0, 0),
        "retained reconciliation must preserve the one pending response-base slot",
    );
    let fallback_probe_width = vm
        .eval("String(document.getElementById('font-probe').getBoundingClientRect().width)")
        .expect("fallback probe width")
        .parse::<f64>()
        .expect("numeric fallback probe width");
    if let Ok(Some(path)) =
        tokio::time::timeout(std::time::Duration::from_millis(150), requests.recv()).await
    {
        observed_paths.push(path);
    }
    while let Ok(path) = requests.try_recv() {
        observed_paths.push(path);
    }
    assert!(
        !observed_paths
            .iter()
            .any(|path| path == "/css/fonts/imported.woff2"),
        "layout reconciliation must not request the imported rule against the root CSS base: {observed_paths:?}",
    );

    font_release
        .send(true)
        .expect("release imported Ahem response");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("imported font completion should reach the resource source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("imported font completion body");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("the response-base slot should accept its font completion");
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (1, 1, 1),
        "the correct slot and registered layout font must survive reconciliation",
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("web-font layout refresh should succeed")
    );
    let probe_width = vm
        .eval("String(document.getElementById('font-probe').getBoundingClientRect().width)")
        .expect("Ahem probe width")
        .parse::<f64>()
        .expect("numeric Ahem probe width");
    assert!(
        (probe_width - 48.0).abs() <= 0.05,
        "the four-glyph probe should use the deterministic Ahem fixture metrics after registration; got {probe_width}",
    );
    assert!(
        (probe_width - fallback_probe_width).abs() > 0.5,
        "registered Ahem metrics must replace the pending fallback metrics; before={fallback_probe_width}, after={probe_width}",
    );

    let _ = shutdown.send(());
    server.await.expect("imported-font server should finish");
}
#[tokio::test]
async fn pending_web_font_response_is_stale_after_stylesheet_media_stops_matching() {
    let (font_url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let document_url = font_url.replace("/print-only.woff2", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, mut resource_completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);
    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
        media: Some("print".to_owned()),
        ..Default::default()
    });
    vm.eval(&format!(
        r#"
const style = document.createElement("style");
style.media = "print";
style.textContent = '@font-face {{ font-family: PendingPrint; src: url({font_url:?}); }}';
(document.head || document.documentElement || document).appendChild(style);
"#
    ))
    .expect("pending print font fixture should evaluate");

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("print layout refresh should succeed")
    );
    request_rx
        .await
        .expect("effective pending font request should reach the server");
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 0, 0));

    vm.set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides::default());
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("screen layout refresh should succeed")
    );
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        (0, 0, 0),
        "the new resource generation must revoke a now-ineffective pending slot"
    );

    release_tx.send(()).expect("release stale font response");
    server.await.expect("font server should finish");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            resource_completions.wait_for_arrival_without_timeout(),
        )
        .await
        .expect("stale font completion should reach the Networking source")
    );
    let completion = resource_completions
        .pop_next_async_subresource_event()
        .expect("stale font completion must retain its typed terminal");
    let _ = vm
        .complete_async_subresource_fetch_event_body(completion)
        .expect("stale font completion should settle without mutating layout fonts");
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}
#[tokio::test]
async fn far_lazy_image_network_request_waits_for_sampled_scroll_reveal() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(404).await;
    let document_url = image_url.replace("/image.png", "/page");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__lazyNetworkImageEvents = [];
  const spacer = document.createElement("div");
  spacer.style.height = "3000px";
  document.body.appendChild(spacer);
  const image = document.createElement("img");
  image.id = "far-lazy-network-image";
  image.loading = "lazy";
  image.width = 32;
  image.height = 32;
  image.onload = () => __lazyNetworkImageEvents.push("load:" + image.complete);
  image.onerror = () => __lazyNetworkImageEvents.push("error:" + image.complete);
  image.src = {image_url:?};
  document.body.appendChild(image);
}})()
"#
    ))
    .expect("far lazy network image setup should evaluate");
    let image = vm
        .document_runtime
        .get_element_by_id("far-lazy-network-image")
        .expect("far lazy image handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none(),
        "setting src must not start a lazy request without sampled eligibility"
    );

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("lazy-image layout refresh should succeed")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_image_load_event(image)
            .is_none(),
        "a fragment beyond the Chromium-aligned preload margin must remain unadmitted"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "sampling far geometry must not create a network request"
    );

    vm.eval("document.getElementById('far-lazy-network-image').scrollIntoView()")
        .expect("scroll reveal should evaluate");
    let pending = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("live scroll delta should admit the far lazy image");
    assert!(pending.network_request_id().is_some());
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1
    );
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), request_rx)
        .await
        .expect("admitted lazy-image request should reach the server")
        .expect("lazy-image request channel should remain open");
    assert!(request.starts_with("GET /image.png HTTP/1.1"));

    release_tx.send(()).expect("release lazy image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "lazy image network completion",
    )
    .await;
    wait_for_image_load_event_executor_test_task(&mut vm, "lazy image decode completion").await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("lazy image load event turn")
    );
    assert_eq!(
        vm.eval("__lazyNetworkImageEvents.join('|')")
            .expect("lazy image event trace"),
        "error:true",
        "an admitted failed request must deliver its terminal instead of becoming lazy-deferred again"
    );
    server.await.expect("lazy image server should finish");
}
#[tokio::test]
async fn removing_pending_image_preserves_request_and_document_delay_until_event() {
    let (image_url, request_rx, release_tx, server) = spawn_gated_image_resource_server(200).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &image_url.replace("/image.png", "/page"),
        &loader,
    );
    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__detachedImageEvents = [];
  const image = document.createElement("img");
  image.id = "detached-pending-image";
  image.onload = () => __detachedImageEvents.push("load");
  image.src = {image_url:?};
  (document.body || document.documentElement || document).appendChild(image);
}})()
"#
    ))
    .expect("pending image setup should evaluate");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let image = vm
        .document_runtime
        .get_element_by_id("detached-pending-image")
        .expect("pending image handle");
    let before = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("connected image sequence");
    assert!(matches!(
        before.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.load_delay_token().is_some()
    ));
    request_rx
        .await
        .expect("pending image request should arrive");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );

    vm.eval("document.getElementById('detached-pending-image').remove()")
        .expect("pending image removal should evaluate");
    let after = vm
        ._context_host
        .borrow()
        .pending_image_load_event(image)
        .expect("detached image must retain its request sequence");
    assert_eq!(after.id(), before.id());
    assert_eq!(after.network_request_id(), before.network_request_id());
    assert!(matches!(
        after.owner(),
        crate::native_bridge::PendingImageLoadEventOwner::Main(binding)
            if binding.load_delay_token().is_some()
    ));
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "same-document removal must preserve the request's exact document delay"
    );

    release_tx
        .send(())
        .expect("release detached image response");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "detached image network completion",
    )
    .await;
    wait_for_image_load_event_executor_test_task(&mut vm, "detached image decode completion").await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            &loader,
        )
        .await
        .expect("detached image event should release the document delay")
    );
    assert_eq!(
        vm.eval("__detachedImageEvents.join('|')")
            .expect("detached image event trace"),
        "load"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "the later detached image event must release its exact document delay"
    );
    server.await.expect("detached image server should finish");
}
#[test]
fn media_preload_none_defers_automatic_load_until_explicit_load() {
    let mut vm = new_parsed_test_vm(
        "https://media-preload-none.test/",
        concat!(
            "<!doctype html><html><head></head><body>",
            "<audio id='clip' preload='none'>",
            "<source src='data:audio/mpeg;base64,AA=='>",
            "</audio></body></html>",
        ),
    );
    vm.exec(
        r#"
        globalThis.__preloadNoneEvents = [];
        const clip = document.getElementById("clip");
        clip.addEventListener("loadstart", () => __preloadNoneEvents.push("loadstart"));
        "#,
        None,
    )
    .expect("preload-none listener should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("preload-none media element");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should apply");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_none(),
        "automatic resource selection must not start preload=none media"
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({ready: clip.readyState, network: clip.networkState, events: __preloadNoneEvents})"
        )
        .expect("deferred media state should evaluate"),
        r#"{"ready":0,"network":1,"events":[]}"#
    );
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    assert_eq!(
        vm._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "preload=none must not create a window-load delay"
    );

    vm.exec("clip.load()", None)
        .expect("explicit media load should evaluate");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some(),
        "load() must override preload=none deferral"
    );
    assert_eq!(
        vm.eval("clip.networkState")
            .expect("explicit media network state should evaluate"),
        "2"
    );
}
#[tokio::test]
async fn media_bit_does_not_enable_an_html_video_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    loader
        .set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::MEDIA);
    let (mut vm, _resource_completion_queue) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(
            "https://media-bit-isolation.test/page.html",
            &loader,
        );

    vm.eval(
        r#"
const video = document.createElement("video");
video.id = "media-bit-video";
video.src = "https://media-bit-isolation.test/video.mp4";
(document.body || document.documentElement || document).appendChild(video);
"#,
    )
    .expect("media-bit video should initialize");

    let media = vm
        .document_runtime
        .get_element_by_id("media-bit-video")
        .expect("video handle");
    assert!(
        vm._context_host
            .borrow()
            .pending_media_load_sequence(media)
            .is_some_and(|pending| pending.network_request_id().is_none()),
        "the generic Media bit must remain distinct from the Video bit"
    );
    assert!(!vm.take_network_output().into_items().any(|item| matches!(
        item,
        crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(_)
    )));
}
#[tokio::test]
async fn default_text_track_policy_loads_an_empty_track_without_a_network_request() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let markup = concat!(
        "<!doctype html><html><body>",
        "<video id='clip' src='data:video/webm;base64,AA=='>",
        "<track id='captions' default src='https://track-policy.test/captions.vtt'>",
        "</video></body></html>"
    );
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://track-policy.test/page.html",
        markup,
        &loader,
    );
    vm.exec(
        r#"
globalThis.__defaultTrackPolicyEvents = [];
const captions = document.getElementById("captions");
captions.addEventListener("load", () => __defaultTrackPolicyEvents.push("load"));
captions.addEventListener("error", () => __defaultTrackPolicyEvents.push("error"));
void captions.track;
"#,
        None,
    )
    .expect("default-policy track listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should discover the default track");
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default track mode-selection task")
    );
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default-policy track start task")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_some_and(|pending| pending.network_request_id().is_none()),
        "the default policy must reject text-track network ownership"
    );
    assert!(!vm.take_network_output().into_items().any(|item| {
        matches!(
            item,
            crate::types::ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                if request.resource_type()
                    == crate::types::SubresourceResourceType::TextTrack
        )
    }));

    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("default-policy track terminal task")
    );
    assert_eq!(
        vm.eval(
            "JSON.stringify({events: __defaultTrackPolicyEvents, ready: captions.readyState, cues: captions.track.cues.length})"
        )
        .expect("default-policy track terminal state"),
        r#"{"events":["load"],"ready":2,"cues":0}"#
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );
}
#[tokio::test]
async fn stale_text_track_start_releases_media_canplay_gate() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://text-track-stale-gate.test/page.html",
        concat!(
            "<!doctype html><html><body>",
            "<video id='clip' src='data:video/webm;base64,AA=='>",
            "<track id='captions' default src='data:text/vtt,WEBVTT'>",
            "</video></body></html>"
        ),
        &loader,
    );
    vm.exec(
        r#"
        globalThis.__staleTrackGateEvents = [];
        const clip = document.getElementById("clip");
        const captions = document.getElementById("captions");
        for (const type of ["loadstart", "loadedmetadata", "loadeddata", "canplay"]) {
          clip.addEventListener(type, () => __staleTrackGateEvents.push(type));
        }
        for (const type of ["load", "error"]) {
          captions.addEventListener(type, () => __staleTrackGateEvents.push(`track-${type}`));
        }
        void captions.track;
        "#,
        None,
    )
    .expect("stale-gate listeners should install");
    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let media = vm
        .document_runtime
        .get_element_by_id("clip")
        .expect("media handle");
    let track = vm
        .document_runtime
        .get_element_by_id("captions")
        .expect("track handle");

    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive should start media and text-track sequences");
    vm.dispatch_main_document_domcontentloaded_lifecycle(owner);
    let media_sequence = vm
        ._context_host
        .borrow()
        .pending_media_load_sequence(media)
        .expect("media sequence")
        .id();

    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadstart owner turn")
        .await;
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::TextTrackDefaultMode,
            &loader,
        )
        .await
        .expect("default-mode DOM turn")
    );
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadedmetadata owner turn")
        .await;
    run_next_page_media_element_event_for_test(&mut vm, &loader, "media loadeddata owner turn")
        .await;
    assert_eq!(
        vm.eval("__staleTrackGateEvents.join('|')")
            .expect("pre-stale event trace"),
        "loadstart|loadedmetadata|loadeddata"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(1),
        "loadeddata must remain gated by the selected text track"
    );

    vm.exec("document.getElementById('captions').remove();", None)
        .expect("track removal should make the queued start stale");
    assert!(
        vm.run_one_text_track_networking_task_executor_turn(&loader)
            .await
            .expect("stale text-track networking task should retire")
    );
    assert!(
        vm._context_host
            .borrow()
            .pending_text_track_load_sequence(track)
            .is_none()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_media_text_track_count(media, media_sequence),
        Some(0),
        "stale retirement must settle the media selection gate"
    );

    run_next_page_media_element_event_for_test(
        &mut vm,
        &loader,
        "media canplay follow-up owner turn",
    )
    .await;
    assert_eq!(
        vm.eval("__staleTrackGateEvents.join('|')")
            .expect("post-stale event trace"),
        "loadstart|loadedmetadata|loadeddata|canplay",
        "settling a stale track must naturally publish the blocked media follow-up"
    );
}
#[tokio::test]
async fn connected_preload_stylesheet_consumers_keep_their_load_delay() {
    for rel in ["preload", "preload stylesheet", "stylesheet preload"] {
        let mut vm = new_parsed_test_vm(
            "https://example.test/page.html",
            &format!(
                "<!doctype html><link id=preload rel='{rel}' as=style href='data:text/css,body{{}}'>"
            ),
        );
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        vm.replace_document_resource_runtime(&loader);
        let owner = vm.current_main_document_task_owner().unwrap();
        vm.queue_initial_connected_style_loads_for_current_owner();
        assert_eq!(
            vm.current_main_document_has_style_load_event_delay(owner),
            Some(rel.contains("stylesheet")),
            "only stylesheet consumers of the preload acquire a load delay: {rel}"
        );
        if rel == "preload" {
            vm.exec(
                "document.getElementById('preload').rel = 'stylesheet'",
                None,
            )
            .unwrap();
            vm.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            assert_eq!(
                vm.current_main_document_has_style_load_event_delay(owner),
                Some(true),
                "converting an in-flight preload to a stylesheet acquires the consumer's load delay"
            );
        }
    }
}
