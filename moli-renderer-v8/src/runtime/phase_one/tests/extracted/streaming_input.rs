use super::*;

#[test]
fn buffered_html_preload_scan_collects_future_external_scripts_without_importmaps() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let urls = collect_preloadable_external_script_urls_from_html(
        &final_url,
        r#"
                <script src="/vendor.js"></script>
                <script defer src="/defer.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
                <script nomodule type="module" src="/module-nomodule.mjs"></script>
                <script type="importmap" src="/importmap.json"></script>
                <script nomodule src="/legacy.js"></script>
                <script>window.inline = true;</script>
            "#,
    );

    assert_eq!(
        urls,
        vec![
            Url::parse("https://example.test/vendor.js").expect("vendor url"),
            Url::parse("https://example.test/defer.js").expect("defer url"),
            Url::parse("https://example.test/async.js").expect("async url"),
            Url::parse("https://example.test/module.mjs").expect("module url"),
            Url::parse("https://example.test/module-nomodule.mjs").expect("nomodule module url"),
        ]
    );
}
#[test]
fn buffered_html_preload_scan_leaves_modulepreload_to_native_module_map() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <link rel="dns-prefetch modulepreload" href="/entry.mjs">
                <link rel="MODULEPRELOAD" href="/entry.mjs">
                <link rel="preload" as="script" href="/classic.js">
                <link rel="modulepreload" as="style" href="/theme.css">
                <link rel="modulepreload" href="/theme.css?version=1">
                <link rel="modulepreload" href="data:text/javascript,export%20default%201">
            "#,
    );

    assert_eq!(
        requests,
        Vec::new(),
        "modulepreload must not enter the legacy script-text preload cache; the parser publishes exact link candidates to the native module map"
    );
}
#[test]
fn incremental_html_preload_scanner_handles_split_script_tag_boundaries() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    assert!(scanner.scan_script_chunk("<script sr").is_empty());
    assert_eq!(
        preload_request_urls(scanner.scan_script_chunk("c=\"/split.js\"></script>")),
        vec![Url::parse("https://example.test/split.js").expect("split url")]
    );
    assert!(scanner.finish_script_scan().is_empty());
}
#[test]
fn incremental_html_preload_scanner_handles_split_stylesheet_tag_boundaries() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    let first = scanner.scan_chunk("<link rel=\"style");
    assert!(first.script_requests.is_empty());
    assert!(first.stylesheet_requests.is_empty());

    let second = scanner.scan_chunk("sheet\" href=\"/split.css\">");
    assert!(second.script_requests.is_empty());
    assert_eq!(
        second
            .stylesheet_requests
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>(),
        vec![Url::parse("https://example.test/split.css").expect("split url")]
    );
    assert!(scanner.finish_scan().stylesheet_requests.is_empty());
}
#[test]
fn incremental_html_preload_scanner_ignores_nested_template_contents_across_chunks() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    let first = scanner.scan_chunk(
        r#"
                <template>
                    <script src="/inside-script.js"></script>
                    <template><link rel="stylesheet" href="/nested.css">
            "#,
    );
    assert!(first.script_requests.is_empty());
    assert!(first.stylesheet_requests.is_empty());

    let second = scanner.scan_chunk(
        r#"
                    </template>
                    <link rel="preload" as="style" href="/inside-preload.css">
                </template>
                <script src="/outside-script.js"></script>
                <link rel="stylesheet" href="/outside.css">
            "#,
    );
    assert_eq!(
        preload_request_urls(second.script_requests),
        vec![Url::parse("https://example.test/outside-script.js").expect("outside script url")]
    );
    assert_eq!(
        second
            .stylesheet_requests
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>(),
        vec![Url::parse("https://example.test/outside.css").expect("outside stylesheet url")]
    );
    let finished = scanner.finish_scan();
    assert!(finished.script_requests.is_empty());
    assert!(finished.stylesheet_requests.is_empty());
}
#[test]
fn incremental_html_preload_scanner_collects_descriptors_after_meta_csp() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
    let batch = scanner.scan_chunk(
        r#"
                <script src="/before.js"></script>
                <meta http-equiv="Content-Security-Policy"
                      content="script-src 'none'">
                <script src="/after.js"></script>
            "#,
    );

    assert_eq!(batch.discovered_meta_csp_count, 1);
    assert_eq!(
        preload_request_urls(batch.script_requests),
        vec![
            Url::parse("https://example.test/before.js").expect("before url"),
            Url::parse("https://example.test/after.js").expect("after url"),
        ],
        "the scanner must preserve descriptors on both sides of the policy boundary"
    );
}
#[test]
fn incremental_html_preload_scanner_matches_chromium_http_equiv_whitespace_behavior() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
    let batch = scanner.scan_chunk(
        r#"<meta http-equiv=" content-security-policy " content="script-src 'none'">
               <script src="/ordinary.js"></script>"#,
    );

    assert_eq!(batch.discovered_meta_csp_count, 0);
    assert_eq!(batch.script_requests.len(), 1);
}
#[test]
fn incremental_html_preload_scanner_reports_split_meta_once_and_keeps_collecting() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    let first = scanner.scan_chunk(r#"<meta http-equiv="Content-Security-"#);
    assert_eq!(first.discovered_meta_csp_count, 0);
    assert!(first.script_requests.is_empty());
    let second = scanner
        .scan_chunk(r#"Policy" content="script-src 'none'"><script src="/pending.js"></script>"#);
    assert_eq!(second.discovered_meta_csp_count, 1);
    assert_eq!(
        preload_request_urls(second.script_requests),
        vec![Url::parse("https://example.test/pending.js").expect("pending url")]
    );
    let finished = scanner.finish_scan();
    assert_eq!(finished.discovered_meta_csp_count, 0);
    assert!(finished.script_requests.is_empty());
}
#[test]
fn incremental_html_preload_scanner_ignores_empty_script_src() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    assert!(
        scanner
            .scan_script_chunk("<script src=\"\"></script>")
            .is_empty()
    );
    assert!(
        scanner
            .scan_script_chunk("<script src=\"  \"></script>")
            .is_empty()
    );
    assert!(scanner.finish_script_scan().is_empty());
}
#[test]
fn incremental_html_preload_scanner_dedupes_urls_across_multiple_appends() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    assert_eq!(
        preload_request_urls(scanner.scan_script_chunk("<script src=\"/dup.js\"></script>")),
        vec![Url::parse("https://example.test/dup.js").expect("dup url")]
    );
    assert!(
        scanner
            .scan_script_chunk("<script src=\"/dup.js\"></script>")
            .is_empty()
    );
    assert!(scanner.finish_script_scan().is_empty());
}
#[test]
fn incremental_html_preload_scanner_keeps_classic_and_module_requests_distinct() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script src="/shared.js"></script>
                <script type="module" src="/shared.js"></script>
            "#,
    );

    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].cache_key(),
        BufferedScriptPreloadKey::new(
            Url::parse("https://example.test/shared.js").expect("shared url"),
            crate::types::ScriptKind::Classic,
            &crate::planning::ScriptFetchMetadata::default(),
        )
        .expect("classic key")
    );
    assert_eq!(
        requests[1].cache_key(),
        BufferedScriptPreloadKey::new(
            Url::parse("https://example.test/shared.js").expect("shared url"),
            crate::types::ScriptKind::Module,
            &crate::planning::ScriptFetchMetadata::default(),
        )
        .expect("module key")
    );
}
#[test]
fn incremental_html_preload_scanner_keeps_crossorigin_requests_distinct() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script src="/shared.js" crossorigin="anonymous"></script>
                <script src="/shared.js" crossorigin="use-credentials"></script>
            "#,
    );

    assert_eq!(requests.len(), 2);
    assert_ne!(requests[0].cache_key(), requests[1].cache_key());
    assert_eq!(
        requests[0].request_metadata_for_testing().0,
        Some("anonymous")
    );
    assert_eq!(
        requests[1].request_metadata_for_testing().0,
        Some("use-credentials")
    );
}
#[test]
fn incremental_html_preload_scanner_dedupes_fetchpriority_variants() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script src="/shared.js" fetchpriority="low"></script>
                <script src="/shared.js" fetchpriority="high"></script>
            "#,
    );

    assert_eq!(
        requests.len(),
        1,
        "fetchpriority is a scheduling hint, not a preload identity key"
    );
    assert_eq!(requests[0].request_metadata_for_testing().5, Some("low"));
}
#[test]
fn incremental_html_preload_scanner_preserves_script_raw_text_state_across_chunks() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    assert!(
        scanner
            .scan_script_chunk("<script>window.fake = \"<script src='/bad.js'></script>")
            .is_empty()
    );
    assert_eq!(
        preload_request_urls(
            scanner.scan_script_chunk("\";</script><script src=\"/real.js\"></script>")
        ),
        vec![Url::parse("https://example.test/real.js").expect("real url")]
    );
    assert!(scanner.finish_script_scan().is_empty());
}
#[test]
fn buffered_html_preload_scan_ignores_script_like_text_inside_script_bodies() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let urls = collect_preloadable_external_script_urls_from_html(
        &final_url,
        r#"
                <script>
                    window.fake = "<script src='/should-not-preload.js'></script>";
                </script>
                <script src="/real.js"></script>
            "#,
    );

    assert_eq!(
        urls,
        vec![Url::parse("https://example.test/real.js").expect("real url")]
    );
}
#[test]
fn buffered_html_preload_scan_ignores_data_url_scripts() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let urls = collect_preloadable_external_script_urls_from_html(
        &final_url,
        r#"
                <script src="data:text/javascript,window.data=1"></script>
                <script src="/real.js"></script>
            "#,
    );

    assert_eq!(
        urls,
        vec![Url::parse("https://example.test/real.js").expect("real url")]
    );
}
#[test]
fn parser_blocking_pending_preload_returns_streaming_boundary_without_wait() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &blocking_script,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                driver.advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><script src="/blocking.js"></script><script defer src="/later.js"></script></head></html>"#,
                    None,
                ),
            )
            .await
            .expect("pending parser-blocking source load must not hold the streaming boundary")
            .expect("parser step should succeed");

            let ParserStepAdvanceOutcome::BlockedOnExternalSource(pending) = outcome else {
                panic!("pending parser-blocking preload should return a streaming boundary");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url,
                blocking_script.url
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(
                parser_blocking_classic_source_load_for_test(pending_script).is_some(),
                "streaming driver needs the pending source load as a wake interest"
            );
            assert!(matches!(
                parser_blocking_classic_source_load_for_test(pending_script),
                Some(PendingParserBlockingSourceLoad::ReusablePreload(_))
            ));
        }));
}
#[test]
fn full_body_phase_one_parks_on_pending_parser_blocking_source_load() {
    run_phase_one_large_stack_test(
        "phase-one-pending-parser-blocking-source-park",
        full_body_phase_one_parks_on_pending_parser_blocking_source_load_inner,
    );
}
#[test]
fn full_body_phase_one_parks_async_subresource_terminal_for_page_owner() {
    run_phase_one_large_stack_test(
        "phase-one-async-subresource-before-source-park",
        full_body_phase_one_parks_async_subresource_terminal_for_page_owner_inner,
    );
}
