use super::*;

#[test]
fn buffered_modulepreload_does_not_feed_later_module_script_text_cache() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut cache = BufferedDocumentPreloadState::default();

            cache.append_to_main_document_scan(
                &final_url,
                r#"<link rel="modulepreload" href="/entry.mjs">"#,
                &loader,
            );

            let consumer = prepared_external_module("https://example.test/entry.mjs");
            assert!(
                cache.shared_preload_for_script(&consumer).is_none(),
                "modulepreload should reserve the native module map entry instead of becoming reusable script text"
            );
        });
}
#[test]
fn prebootstrap_preload_filter_skips_async_classic_scripts() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let urls = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script src="/normal.js"></script>
                <script defer src="/defer.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
            "#,
    )
    .into_iter()
    .filter(prebootstrap_preload_request_is_dcl_relevant)
    .map(|request| request.url)
    .collect::<Vec<_>>();

    assert_eq!(
        urls,
        vec![
            Url::parse("https://example.test/normal.js").expect("normal url"),
            Url::parse("https://example.test/defer.js").expect("defer url"),
            Url::parse("https://example.test/module.mjs").expect("module url"),
        ]
    );
}
#[test]
fn html_preload_scanner_marks_late_classic_script_after_image() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script src="/early.js"></script>
                <img src="/hero.png">
                <script src="/late.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
            "#,
    );

    let resource_types = requests
        .iter()
        .map(|request| (request.url.path().to_owned(), request.resource_type_hint))
        .collect::<Vec<_>>();
    assert_eq!(
        resource_types,
        vec![
            (
                "/early.js".to_owned(),
                moli_fetch::RequestResourceType::ParserBlockingScript,
            ),
            (
                "/late.js".to_owned(),
                moli_fetch::RequestResourceType::LatePreloadScript,
            ),
            (
                "/async.js".to_owned(),
                moli_fetch::RequestResourceType::ClassicAsyncOrDeferScript,
            ),
            (
                "/module.mjs".to_owned(),
                moli_fetch::RequestResourceType::Script,
            ),
        ]
    );
}
#[test]
fn html_preload_scanner_collects_only_exact_eager_image_candidates() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
    let batch = scanner.scan_chunk(
        r#"
                <img src="hero.png" fetchpriority="HIGH">
                <img src="hero.png">
                <img src="lazy.png" loading="lazy">
                <img src="responsive-fallback.png" srcset="responsive.png 1x">
                <img src="cors.png" crossorigin>
                <picture><source srcset="wide.png"><img src="picture-fallback.png"></picture>
                <img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yw=">
                <template><img src="template.png"></template>
                <img src="/plain.png">
            "#,
    );

    assert_eq!(
        batch
            .image_requests
            .iter()
            .map(|request| request.url.as_str())
            .collect::<Vec<_>>(),
        [
            "https://example.test/docs/hero.png",
            "https://example.test/plain.png",
        ]
    );
    assert_eq!(
        batch.image_requests[0].fetch_priority,
        Some(moli_fetch::FetchPriorityHint::High)
    );
    assert_eq!(batch.image_requests[1].fetch_priority, None);
}
#[test]
fn html_preload_scanner_stops_module_preloads_after_import_map() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script type="module" src="/before.mjs"></script>
                <script type="importmap">{"integrity": {}}</script>
                <script type="module" src="/after.mjs"></script>
                <script src="/classic.js"></script>
            "#,
    );

    assert_eq!(
        preload_request_urls(requests),
        vec![
            Url::parse("https://example.test/before.mjs").expect("before module url"),
            Url::parse("https://example.test/classic.js").expect("classic url"),
        ]
    );
}
#[test]
fn html_preload_scanner_remembers_import_map_across_chunks() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

    assert!(
        scanner
            .scan_script_chunk(r#"<script type="importmap">{}</script>"#)
            .is_empty()
    );
    assert!(
        scanner
            .scan_script_chunk(r#"<script type="module" src="/after.mjs"></script>"#)
            .is_empty()
    );
    assert!(scanner.finish_script_scan().is_empty());
}
#[test]
fn html_preload_scanner_uses_first_valid_base_for_later_resources() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let stylesheets = collect_preloadable_stylesheet_requests_from_html(
        &final_url,
        r#"
                <link rel="stylesheet" href="before.css">
                <base href="http://[">
                <base href="https://cdn.example/assets/">
                <link rel="stylesheet" href="after.css">
                <base href="https://ignored.example/">
                <link rel="stylesheet" href="last.css">
            "#,
    );
    let scripts = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <base href="https://cdn.example/assets/">
                <script src="entry.js"></script>
            "#,
    );

    assert_eq!(
        stylesheets
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>(),
        vec![
            Url::parse("https://example.test/docs/before.css").expect("before url"),
            Url::parse("https://cdn.example/assets/after.css").expect("after url"),
            Url::parse("https://cdn.example/assets/last.css").expect("last url"),
        ]
    );
    assert_eq!(
        preload_request_urls(scripts),
        vec![Url::parse("https://cdn.example/assets/entry.js").expect("script url")]
    );
}
#[test]
fn html_preload_scanner_collects_stylesheets_after_meta_csp_for_owner_admission() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_stylesheet_requests_from_html(
        &final_url,
        r#"
                <link rel="stylesheet" href="/before.css">
                <meta http-equiv="CONTENT-SECURITY-POLICY" content="style-src 'none'">
                <link rel="preload" as="style" href="/after.css">
            "#,
    );

    assert_eq!(
        requests
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>(),
        vec![
            Url::parse("https://example.test/before.css").expect("before url"),
            Url::parse("https://example.test/after.css").expect("after url"),
        ]
    );
}
#[test]
fn html_preload_scanner_keeps_script_and_stylesheet_same_url_distinct() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
    let batch = scanner.scan_chunk(
        r#"
                <script src="/shared"></script>
                <link rel="stylesheet" href="/shared">
            "#,
    );

    assert_eq!(batch.script_requests.len(), 1);
    assert_eq!(batch.stylesheet_requests.len(), 1);
    assert_eq!(
        batch.script_requests[0].url,
        batch.stylesheet_requests[0].url
    );
}
#[test]
fn html_preload_scanner_filters_and_carries_stylesheet_attributes() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_stylesheet_requests_from_html(
        &final_url,
        r#"
                <link rel=" STYLEsheet " href="/theme.css"
                      media="screen and (min-width: 400px)"
                      crossorigin="anonymous"
                      referrerpolicy="no-referrer"
                      integrity="sha256-test"
                      nonce="nonce-1"
                      charset="utf-8"
                      fetchpriority="high">
                <link rel="preload" as="STYLE" href="/preload.css">
                <link rel="preload" as="script" href="/not-style.css">
                <link rel="preload" as=" style " href="/padded.css">
                <link rel="preload" as="&#9;style&#10;" href="/ascii-space.css">
                <link rel="preload" as="&#160;style" href="/unicode-space.css">
                <link rel="stylesheet" disabled href="/disabled.css">
                <link rel="stylesheet" type="text/plain" href="/wrong-type.css">
                <link rel="stylesheet" href="data:text/css,body{}">
                <link rel="stylesheet" href="">
            "#,
    );

    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        Url::parse("https://example.test/theme.css").expect("theme url")
    );
    assert_eq!(
        requests[0].media.as_deref(),
        Some("screen and (min-width: 400px)")
    );
    assert_eq!(requests[0].options.cross_origin(), Some("anonymous"));
    assert_eq!(requests[0].options.referrer_policy(), Some("no-referrer"));
    assert_eq!(requests[0].options.integrity(), Some("sha256-test"));
    assert_eq!(requests[0].options.nonce(), Some("nonce-1"));
    assert_eq!(requests[0].options.charset(), Some("utf-8"));
    assert_eq!(requests[0].options.fetch_priority(), Some("high"));
    assert_eq!(
        requests[1].url,
        Url::parse("https://example.test/preload.css").expect("preload url")
    );
}
#[test]
fn html_preload_scanner_dedupes_stylesheet_and_style_preload_resource() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let requests = collect_preloadable_stylesheet_requests_from_html(
        &final_url,
        r#"
                <link rel="preload" as="style" href="/shared.css">
                <link rel="stylesheet" href="/shared.css">
            "#,
    );

    assert_eq!(
        requests.len(),
        1,
        "scanner descriptors represent physical resources rather than DOM clients"
    );
}
#[test]
fn scanned_stylesheet_starts_without_waiting_for_parser_dom_ownership() {
    run_phase_one_large_stack_test("scanned-stylesheet-ownerless-admission", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("stylesheet probe server should bind");
                    let addr = listener
                        .local_addr()
                        .expect("stylesheet probe server should expose address");
                    let server = tokio::spawn(async move {
                        let (mut stream, _) = listener
                            .accept()
                            .await
                            .expect("stylesheet probe server should accept request");
                        let path = read_http_request_path(&mut stream)
                            .await
                            .expect("stylesheet probe server should read request");
                        let body = "body { color: black; }";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("stylesheet probe server should write response");
                        path
                    });

                    let mut scanner = IncrementalHtmlPreloadScanner::new(
                        Url::parse(&format!("http://{addr}/page.html")).expect("document url"),
                    );
                    let batch = scanner.scan_chunk(
                        r#"<script src="/blocking.js"></script><link rel="stylesheet" href="/app.css">"#,
                    );
                    assert_eq!(batch.script_requests.len(), 1);
                    assert_eq!(batch.stylesheet_requests.len(), 1);

                    let mut page_vm = new_phase_one_page_vm_for_test();
                    admit_stylesheet_preloads(&mut page_vm, batch.stylesheet_requests);
                    let path = tokio::time::timeout(std::time::Duration::from_secs(2), server)
                        .await
                        .expect("ownerless stylesheet request should start before parser input")
                        .expect("stylesheet probe server should finish");
                    assert_eq!(path, "/app.css");
                    assert!(
                        !native_dom_has_element_id(
                            &page_vm.vm().snapshot_live_document(),
                            "sheet"
                        ),
                        "speculative admission must not synthesize a DOM owner"
                    );
                }));
    });
}
#[test]
fn scanned_stylesheet_admission_respects_media_and_fetch_interception() {
    run_phase_one_large_stack_test("scanned-stylesheet-admission-gates", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let options = crate::stylesheet_blocking::StylesheetFetchOptions::default();

            assert_eq!(
                page_vm.admit_scanned_stylesheet_preload(
                    Url::parse("data:text/css,body{}").expect("data stylesheet"),
                    Some("print"),
                    options.clone(),
                    moli_fetch::RequestResourceType::CssStyleSheet,
                    false,
                ),
                ScannedStylesheetAdmission::DeferredToParser(
                    ScannedStylesheetDeferral::MediaMismatch
                ),
                "a nonmatching media query must leave the request to the real DOM owner"
            );

            page_vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Stylesheet),
            );
            assert_eq!(
                page_vm.admit_scanned_stylesheet_preload(
                    Url::parse("data:text/css,html{}").expect("data stylesheet"),
                    Some("screen"),
                    options.clone(),
                    moli_fetch::RequestResourceType::CssStyleSheet,
                    false,
                ),
                ScannedStylesheetAdmission::DeferredToParser(
                    ScannedStylesheetDeferral::FetchInterception
                ),
                "stylesheet interception must conservatively disable speculative admission"
            );

            page_vm.set_fetch_subresource_interception(
                true,
                Some(crate::types::SubresourceResourceType::Script),
            );
            assert_eq!(
                page_vm.admit_scanned_stylesheet_preload(
                    Url::parse("data:text/css,p{}").expect("data stylesheet"),
                    Some("screen"),
                    options.clone(),
                    moli_fetch::RequestResourceType::CssStyleSheet,
                    false,
                ),
                ScannedStylesheetAdmission::Admitted,
                "interception for another resource type must not disable stylesheet scanning"
            );

            page_vm
                .vm_mut()
                .set_response_content_security_policies(&["style-src 'none'".to_owned()]);
            assert_eq!(
                page_vm.admit_scanned_stylesheet_preload(
                    Url::parse("https://example.test/blocked.css").expect("blocked stylesheet"),
                    Some("screen"),
                    options,
                    moli_fetch::RequestResourceType::CssStyleSheet,
                    false,
                ),
                ScannedStylesheetAdmission::DeferredToParser(
                    ScannedStylesheetDeferral::ContentSecurityPolicy
                ),
                "response CSP must defer speculative admission to the DOM owner"
            );
        }));
    });
}
#[test]
fn scanned_stylesheet_admission_uses_processed_meta_csp() {
    run_phase_one_large_stack_test("scanned-stylesheet-meta-csp", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let meta = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let meta = dom_host.create_parser_element_without_attributes(
                    "meta".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(meta, "http-equiv", "content-security-policy"));
                assert!(dom_host.set_attribute(meta, "content", "style-src 'none'"));
                assert!(dom_host.append_child(body, meta));
                meta
            };
            page_vm
                .vm()
                .document_runtime
                .process_parser_meta_content_security_policy(meta);

            assert_eq!(
                page_vm.admit_scanned_stylesheet_preload(
                    Url::parse("https://example.test/blocked-by-meta.css")
                        .expect("blocked stylesheet"),
                    Some("screen"),
                    crate::stylesheet_blocking::StylesheetFetchOptions::default(),
                    moli_fetch::RequestResourceType::CssStyleSheet,
                    false,
                ),
                ScannedStylesheetAdmission::DeferredToParser(
                    ScannedStylesheetDeferral::ContentSecurityPolicy
                ),
                "ownerless stylesheet admission must include already processed meta policies"
            );
        }));
    });
}
#[test]
fn insertion_preload_scanner_remains_conservative_after_meta_csp() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut scanner = IncrementalHtmlPreloadScanner::new_conservative(final_url);
    let batch = scanner.scan_chunk(
        r#"<script src="/before.js"></script>
               <meta http-equiv="content-security-policy" content="script-src 'self'">
               <script src="/after.js"></script>"#,
    );

    assert_eq!(batch.discovered_meta_csp_count, 1);
    assert_eq!(
        preload_request_urls(batch.script_requests),
        vec![Url::parse("https://example.test/before.js").expect("before url")]
    );
}
#[test]
fn response_csp_defers_script_preloads_to_parser_admission() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();
    cache.set_response_csp_requires_parser_admission(true);

    cache.append_to_main_document_scan(
        &final_url,
        r#"<script src="/blocked.js"></script>"#,
        &loader,
    );

    assert!(
        cache.entries.is_empty(),
        "an enforced response policy must be evaluated by the parser policy owner before any script request starts"
    );
    assert_eq!(cache.pending_preload_counts_for_test(), (1, 0));
}
#[test]
fn script_disabled_preload_scan_defers_every_script_to_the_page_owner() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let env = default_test_page_vm_env_config_with(|env| {
        env.script_execution_disabled = true;
    });
    let mut cache = BufferedDocumentPreloadState::default();
    cache.set_script_fetch_requires_owner_admission(script_preloads_require_owner_admission(&env));

    cache.append_to_main_document_scan(
        &final_url,
        concat!(
            "<script src='/blocking.js'></script>",
            "<script defer src='/defer.js'></script>",
            "<script async src='/async.js'></script>",
            "<script type='module' src='/module.js'></script>",
        ),
        &loader,
    );

    assert!(
        cache.entries.is_empty(),
        "the preload scanner must not start a physical script fetch before the disabled page owner can reject it"
    );
    assert_eq!(
        preload_request_urls(cache.take_pending_script_preloads_for_test()),
        ["blocking.js", "defer.js", "async.js", "module.js"].map(|name| {
            Url::parse(&format!("https://example.test/{name}")).expect("script preload URL")
        })
    );
}
#[test]
fn meta_csp_gate_waits_for_every_scanner_seen_policy() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();
    cache.append_to_main_document_scan(
        &final_url,
        r#"
                <meta http-equiv="content-security-policy" content="script-src 'self'">
                <script src="/first.js"></script>
                <meta http-equiv="content-security-policy" content="script-src 'self'">
                <script src="/second.js"></script>
            "#,
        &loader,
    );

    assert_eq!(cache.meta_csp_counts_for_test(), (2, 0));
    assert_eq!(cache.pending_preload_counts_for_test(), (2, 0));
    cache.note_parser_processed_meta_csp(1);
    assert!(cache.take_pending_script_preloads_for_test().is_empty());
    assert_eq!(cache.meta_csp_counts_for_test(), (2, 1));
    cache.note_parser_processed_meta_csp(1);
    assert_eq!(cache.take_pending_script_preloads_for_test().len(), 2);
    assert_eq!(cache.meta_csp_counts_for_test(), (2, 2));
}
#[test]
fn meta_csp_pending_descriptor_budget_falls_back_to_parser() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut html =
        r#"<meta http-equiv="content-security-policy" content="script-src 'self'">"#.to_owned();
    for index in 0..(MAX_PENDING_CSP_PRELOAD_CANDIDATES + 4) {
        html.push_str(&format!(r#"<script src="/{index}.js"></script>"#));
    }
    let mut cache = BufferedDocumentPreloadState::default();
    cache.append_to_main_document_scan(&final_url, &html, &loader);

    assert_eq!(
        cache.pending_preload_counts_for_test(),
        (MAX_PENDING_CSP_PRELOAD_CANDIDATES, 0),
        "overflow candidates must be left to their real parser elements"
    );
}
#[test]
fn parser_meta_csp_acknowledgement_admits_all_scanned_scripts_at_first_handoff() {
    run_phase_one_large_stack_test("parser-meta-csp-preload-admission", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let html = r#"
                    <!doctype html><html><head>
                    <meta http-equiv="content-security-policy" content="script-src 'self'">
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                    <script src="/third.js"></script>
                    </head></html>
                "#;
                let PhaseOnePageVmHarness {
                    mut page_vm,
                    loader,
                    state,
                } = new_phase_one_page_vm_harness_for_test();
                activate_standalone_main_parser_continuation_for_test(&mut page_vm);
                let final_url = state.final_url.clone();
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(&final_url, html, loader);
                assert_eq!(
                    state
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (3, 0)
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
                let _ = driver
                    .advance_parser_step(&mut page_vm, html, None)
                    .await
                    .expect("parser should reach its first external script boundary");

                assert_eq!(
                    driver
                        .buffered_document_preloads
                        .meta_csp_counts_for_test(),
                    (1, 1),
                    "the parser-connected meta must acknowledge exactly one scanner checkpoint"
                );
                assert_eq!(
                    driver.buffered_document_preloads.entries.len(),
                    3,
                    "one policy acknowledgement must admit later descriptors before the parser reaches them"
                );
                assert_eq!(
                    driver
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (0, 0)
                );
            }));
    });
}
#[test]
fn parser_meta_csp_admission_starts_later_request_before_first_settles() {
    run_phase_one_large_stack_test("parser-meta-csp-concurrent-preloads", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let _js_runtime = crate::JsRuntime::initialize();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("script barrier server should bind");
                let addr = listener
                    .local_addr()
                    .expect("script barrier server should expose address");
                let server = tokio::spawn(async move {
                    let mut accepted = Vec::new();
                    for _ in 0..2 {
                        let (mut stream, _) = listener
                            .accept()
                            .await
                            .expect("two speculative requests should connect");
                        let path = read_http_request_path(&mut stream)
                            .await
                            .expect("script barrier server should read request");
                        accepted.push((stream, path));
                    }
                    let body = "window.cspConcurrentPreload = true;";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    for (stream, _) in &mut accepted {
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("script barrier server should release request");
                    }
                    accepted
                        .into_iter()
                        .map(|(_, path)| path)
                        .collect::<Vec<_>>()
                });

                let final_url =
                    Url::parse(&format!("http://{addr}/page.html")).expect("document url");
                let html = r#"
                    <meta http-equiv="content-security-policy" content="script-src 'self'">
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                "#;
                let loader = ResourceRequestClient::new(&FetchConfig::default())
                    .expect("default loader");
                let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone());
                let parser_dom_host = state
                    .parser_session
                    .stream_handle()
                    .borrow_mut()
                    .take_parser_stream_dom_host();
                let local_executor = JsLocalExecutor::new();
                let runtime_hooks =
                    PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
                state.buffered_document_preloads.bind_resource_runtime(
                    runtime_hooks.owner_wake(),
                    runtime_hooks.resource_task_runner(),
                );
                let mut page_vm = PageVm::new(
                    PageId::new_for_testing(91),
                    local_executor,
                    &loader,
                    &default_test_page_vm_env_config(),
                    runtime_hooks,
                    parser_dom_host,
                    Instant::now(),
                )
                .expect("page vm");
                activate_standalone_main_parser_continuation_for_test(&mut page_vm);
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(&final_url, html, &loader);

                let mut driver = ParserDriver {
                    loader: &loader,
                    final_url: &state.final_url,
                    parser_session: &mut state.parser_session,
                    scheduler: &mut state.scheduler,
                    buffered_document_preloads: &mut state.buffered_document_preloads,
                    service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                    input_closed: &state.input_closed,
                };
                let _ = driver
                    .advance_parser_step(&mut page_vm, html, None)
                    .await
                    .expect("parser should stop at its first pending external source");

                let mut paths = tokio::time::timeout(std::time::Duration::from_secs(2), server)
                    .await
                    .expect("the second preload must arrive before the first response settles")
                    .expect("script barrier server should finish");
                paths.sort();
                assert_eq!(paths, vec!["/first.js", "/second.js"]);
            }));
    });
}
#[test]
fn parser_meta_csp_owner_admission_blocks_without_starting_preload() {
    run_phase_one_large_stack_test("parser-meta-csp-blocked-preload", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"
                    <meta http-equiv="content-security-policy" content="script-src 'none'">
                    <script src="/blocked.js"></script>
                "#;
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);
            let final_url = state.final_url.clone();
            state
                .buffered_document_preloads
                .append_to_main_document_scan(&final_url, html, loader);

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let _ = driver
                .advance_parser_step(&mut page_vm, html, None)
                .await
                .expect("CSP-blocked parser script should remain a normal parser decision");

            assert_eq!(
                driver.buffered_document_preloads.meta_csp_counts_for_test(),
                (1, 1)
            );
            assert!(
                driver.buffered_document_preloads.entries.is_empty(),
                "silent speculative admission must not start a CSP-blocked physical request"
            );
            assert_eq!(
                driver
                    .buffered_document_preloads
                    .pending_preload_counts_for_test(),
                (0, 0)
            );
        }));
    });
}
#[test]
fn response_csp_owner_admission_allows_self_and_blocks_none() {
    run_phase_one_large_stack_test("response-csp-preload-admission", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            state
                .buffered_document_preloads
                .set_response_csp_requires_parser_admission(true);
            let final_url = state.final_url.clone();
            state
                .buffered_document_preloads
                .append_to_main_document_scan(
                    &final_url,
                    r#"<script src="/allowed.js"></script>"#,
                    loader,
                );
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&["script-src 'self'".to_owned()]);
            admit_pending_preloads(
                &mut page_vm,
                &mut state.buffered_document_preloads,
                loader,
                None,
            );
            assert!(
                state
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/allowed.js")),
                "an allowed response-CSP request should enter the shared preload map"
            );

            state
                .buffered_document_preloads
                .append_to_main_document_scan(
                    &final_url,
                    r#"<script src="/blocked.js"></script>"#,
                    loader,
                );
            page_vm
                .vm_mut()
                .set_response_content_security_policies(&["script-src 'none'".to_owned()]);
            admit_pending_preloads(
                &mut page_vm,
                &mut state.buffered_document_preloads,
                loader,
                None,
            );
            assert!(
                !state
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/blocked.js")),
                "a blocked response-CSP descriptor must not start a physical preload"
            );
            assert_eq!(
                state
                    .buffered_document_preloads
                    .pending_preload_counts_for_test(),
                (0, 0),
                "the real parser remains authoritative after a blocked descriptor is discarded"
            );
        }));
    });
}
#[test]
fn buffered_script_preload_cache_reuses_load_with_different_fetchpriority() {
    let mut cache = BufferedDocumentPreloadState::default();
    let mut preloaded = prepared_external_classic("https://example.test/vendor.js");
    preloaded.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
        None,
        None,
        None,
        None,
        None,
        Some("low"),
    );
    cache.entries.insert(
        BufferedScriptPreloadKey::from_script(&preloaded).expect("preload key"),
        ready_preload_entry_for_script(&preloaded, "window.vendor = 1;"),
    );

    let mut consumer = prepared_external_classic("https://example.test/vendor.js");
    consumer.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
        None,
        None,
        None,
        None,
        None,
        Some("high"),
    );

    assert!(
        cache.shared_preload_for_script(&consumer).is_some(),
        "same request identity should reuse the preload even when fetchpriority differs"
    );
}
#[test]
fn html_preload_scanner_carries_script_request_metadata() {
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let mut requests = collect_preloadable_external_script_requests_from_html(
        &final_url,
        r#"
                <script
                    type="module"
                    src="/module.mjs"
                    crossorigin="anonymous"
                    referrerpolicy="no-referrer"
                    charset="utf-8"
                    integrity="sha256-test"
                    nonce="nonce-1"
                    fetchpriority="high">
                </script>
            "#,
    );

    assert_eq!(requests.len(), 1);
    let request = requests.pop().expect("request");
    assert_eq!(
        request.url,
        Url::parse("https://example.test/module.mjs").expect("module url")
    );
    assert_eq!(request.kind_hint, crate::types::ScriptKind::Module);
    assert_eq!(request.mode_hint, crate::types::ScriptMode::ModuleDefer);
    assert_eq!(
        request.request_metadata_for_testing(),
        (
            Some("anonymous"),
            Some("no-referrer"),
            Some("utf-8"),
            Some("sha256-test"),
            Some("nonce-1"),
            Some("high"),
        )
    );
}
#[test]
fn buffered_script_preload_cache_starts_loads_during_scan_before_handoff() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();
        bind_preload_state_to_current_test_runtime(&mut cache);

        cache.append_to_main_document_scan(
            &final_url,
            r#"
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                "#,
            &loader,
        );

        let first = prepared_external_classic("https://example.test/first.js");
        let second = prepared_external_classic("https://example.test/second.js");

        assert!(
            cache.shared_preload_for_script(&first).is_some(),
            "scanner must create the first script's shared load before parser handoff"
        );
        assert!(
            cache.shared_preload_for_script(&second).is_some(),
            "scanner must create later script shared loads before parser reaches them"
        );
    });
}
#[test]
fn buffered_script_preload_cache_applies_ready_source_to_matching_script() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/vendor.js");
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            ready_preload_entry_for_script(&script, "window.vendor = 1;"),
        );

        assert!(
            cache
                .apply_preloaded_source_to_script_if_available(&mut script, false)
                .await
                .is_some()
        );
        assert!(matches!(
            &script.source,
            ScriptSource::Loaded(source) if source == "window.vendor = 1;"
        ));
    });
}
#[test]
fn buffered_script_preload_cache_keeps_spawn_time_source_after_late_document_charset() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/legacy.js");
        let source_bytes = encoding_rs::GBK
            .encode("window.legacyMarker = '家居';")
            .0
            .into_owned();
        let stale_utf8_source = String::from_utf8_lossy(&source_bytes).into_owned();
        assert!(
            !stale_utf8_source.contains("家居"),
            "test fixture should be non-UTF-8 bytes"
        );

        let response = crate::protocol_types::NavigationResponse::from_text_body(
            script.url.clone(),
            200,
            vec![("Content-Type".to_owned(), b"text/javascript".to_vec())],
            String::new(),
        );
        let response = crate::protocol_types::NavigationResponse::from_head_and_body(
            response.head(),
            stale_utf8_source.clone(),
            source_bytes,
        );
        let request = BufferedScriptPreloadRequest {
            url: script.url.clone(),
            initiator_url: script.initiator_url.clone(),
            kind_hint: script.kind,
            mode_hint: script.mode,
            resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
            fetch_metadata: script.fetch_metadata.clone(),
        };
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            BufferedScriptPreloadEntry {
                request,
                load: SharedScriptSourceLoad::ready_outcome(
                    Ok(stale_utf8_source.clone()),
                    Some(std::sync::Arc::new(Ok(response))),
                ),
            },
        );

        cache.set_document_character_set("GBK");
        assert!(
            cache
                .apply_preloaded_source_to_script_if_available(&mut script, false)
                .await
                .is_some()
        );
        assert!(matches!(
            &script.source,
            ScriptSource::Loaded(source) if source == &stale_utf8_source
        ));
    });
}
#[test]
fn buffered_script_preload_cache_keeps_source_when_late_document_charset_is_utf8() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/app.js");
        let source = "window.predecoded = true;".to_owned();
        let raw_bytes = encoding_rs::GBK
            .encode("window.predecoded = '家居';")
            .0
            .into_owned();
        let response = crate::protocol_types::NavigationResponse::from_text_body(
            script.url.clone(),
            200,
            vec![("Content-Type".to_owned(), b"text/javascript".to_vec())],
            String::new(),
        );
        let response = crate::protocol_types::NavigationResponse::from_head_and_body(
            response.head(),
            String::from_utf8_lossy(&raw_bytes).into_owned(),
            raw_bytes,
        );
        let request = BufferedScriptPreloadRequest {
            url: script.url.clone(),
            initiator_url: script.initiator_url.clone(),
            kind_hint: script.kind,
            mode_hint: script.mode,
            resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
            fetch_metadata: script.fetch_metadata.clone(),
        };
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            BufferedScriptPreloadEntry {
                request,
                load: SharedScriptSourceLoad::ready_outcome(
                    Ok(source.clone()),
                    Some(std::sync::Arc::new(Ok(response))),
                ),
            },
        );

        cache.set_document_character_set("UTF-8");
        assert!(
            cache
                .apply_preloaded_source_to_script_if_available(&mut script, false)
                .await
                .is_some()
        );
        assert!(matches!(
            &script.source,
            ScriptSource::Loaded(loaded) if loaded == &source
        ));
    });
}
#[test]
fn buffered_script_preload_cache_does_not_reuse_load_with_different_crossorigin() {
    let mut cache = BufferedDocumentPreloadState::default();
    let mut preloaded = prepared_external_classic("https://example.test/vendor.js");
    preloaded.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
        Some("anonymous"),
        None,
        None,
        None,
        None,
        None,
    );
    cache.entries.insert(
        BufferedScriptPreloadKey::from_script(&preloaded).expect("preload key"),
        ready_preload_entry_for_script(&preloaded, "window.vendor = 1;"),
    );

    let mut consumer = prepared_external_classic("https://example.test/vendor.js");
    consumer.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
        Some("use-credentials"),
        None,
        None,
        None,
        None,
        None,
    );

    assert!(
        cache.shared_preload_for_script(&consumer).is_none(),
        "same URL with different crossorigin cannot reuse the preload handle"
    );
}
#[test]
fn buffered_script_preload_cache_can_await_pending_source_for_blocking_script() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/vendor.js");
        let load = SharedScriptSourceLoad::spawn_for_test(async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok("window.vendor = 2;".to_owned())
        });
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: BufferedScriptPreloadRequest {
                    url: script.url.clone(),
                    initiator_url: script.initiator_url.clone(),
                    kind_hint: script.kind,
                    mode_hint: script.mode,
                    resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
                    fetch_metadata: script.fetch_metadata.clone(),
                },
                load,
            },
        );

        assert!(
            cache
                .apply_preloaded_source_to_script_if_available(&mut script, true)
                .await
                .is_some()
        );
        assert!(matches!(
            &script.source,
            ScriptSource::Loaded(source) if source == "window.vendor = 2;"
        ));
    });
}
#[test]
fn buffered_script_preload_cache_does_not_wait_for_pending_late_parser_blocking_preload() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/late.js");
        let load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &script,
                    moli_fetch::RequestResourceType::LatePreloadScript,
                ),
                load,
            },
        );

        let applied = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            cache.apply_preloaded_source_to_script_if_available(&mut script, true),
        )
        .await
        .expect("pending late preload should not hold the parser-blocking consumer");

        assert!(applied.is_none());
        assert!(matches!(script.source, ScriptSource::External));
    });
}
#[test]
fn buffered_script_preload_cache_classifies_parser_blocking_preload_states() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut missing_cache = BufferedDocumentPreloadState::default();
        let mut missing = prepared_external_classic("https://example.test/missing.js");
        assert!(matches!(
            missing_cache.parser_blocking_preload_disposition_for_script(&mut missing),
            ParserBlockingPreloadDisposition::Missing
        ));

        let mut ready_cache = BufferedDocumentPreloadState::default();
        let mut ready = prepared_external_classic("https://example.test/ready.js");
        ready_cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&ready).expect("preload key"),
            ready_preload_entry_for_script(&ready, "window.readyPreload = 1;"),
        );
        assert!(matches!(
            ready_cache.parser_blocking_preload_disposition_for_script(&mut ready),
            ParserBlockingPreloadDisposition::Ready(_)
        ));
        assert!(matches!(
            &ready.source,
            ScriptSource::Loaded(source) if source == "window.readyPreload = 1;"
        ));

        let mut ready_error_cache = BufferedDocumentPreloadState::default();
        let mut ready_error = prepared_external_classic("https://example.test/ready-error.js");
        ready_error_cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&ready_error).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &ready_error,
                    moli_fetch::RequestResourceType::ParserBlockingScript,
                ),
                load: SharedScriptSourceLoad::ready_err("failed preload"),
            },
        );
        let ready_error_load = match ready_error_cache
            .parser_blocking_preload_disposition_for_script(&mut ready_error)
        {
            ParserBlockingPreloadDisposition::ReusableSourceLoad(load) => load,
            _ => panic!("completed preload failure must remain attached to PendingScript"),
        };
        assert!(
            ready_error_load
                .try_outcome()
                .is_some_and(|outcome| outcome.source_result.is_err()),
            "completed preload failure must retain its terminal source result"
        );
        assert!(matches!(ready_error.source, ScriptSource::External));

        let mut pending_cache = BufferedDocumentPreloadState::default();
        let mut pending = prepared_external_classic("https://example.test/pending.js");
        pending_cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&pending).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &pending,
                    moli_fetch::RequestResourceType::ParserBlockingScript,
                ),
                load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
            },
        );
        assert!(matches!(
            pending_cache.parser_blocking_preload_disposition_for_script(&mut pending),
            ParserBlockingPreloadDisposition::ReusableSourceLoad(_)
        ));

        let mut pending_late_cache = BufferedDocumentPreloadState::default();
        let mut pending_late = prepared_external_classic("https://example.test/pending-late.js");
        pending_late_cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&pending_late).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &pending_late,
                    moli_fetch::RequestResourceType::LatePreloadScript,
                ),
                load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
            },
        );
        assert!(matches!(
            pending_late_cache.parser_blocking_preload_disposition_for_script(&mut pending_late),
            ParserBlockingPreloadDisposition::ExistingButNotReusable
        ));
        assert!(matches!(pending_late.source, ScriptSource::External));
    });
}
#[test]
fn parser_blocking_ready_preload_executes_without_source_boundary() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    "document.body.setAttribute('data-ready-preload', 'used');",
                ),
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
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                super::access::run_named_owner_local_task(
                    local_executor,
                    "phase-one ready preload parser-blocking test channel closed",
                    async move {
                        let page_vm = unsafe { &mut *page_vm_ptr };
                        let driver = unsafe { &mut *driver_ptr };
                        driver
                            .advance_parser_step(
                                page_vm,
                                r#"<!doctype html><html><body><script src="/blocking.js"></script><div id="after-ready-preload"></div></body></html>"#,
                                None,
                            )
                            .await
                    },
                ),
            )
            .await
            .expect("ready parser-blocking preload must not wait on a source boundary")
            .expect("ready preload parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preloads must be consumed, not replaced by ParserDiscovered"
            );
            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let ready_marker = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-ready-preload"));
            assert_eq!(ready_marker, Some("used"));
            assert!(
                native_dom_has_element_id(&snapshot, "after-ready-preload"),
                "parser should continue after executing the ready preloaded blocking script"
            );
        }));
}
#[test]
fn parser_blocking_external_script_without_preload_starts_source_load_after_input_close() {
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
            state.input_closed = true;
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
            .expect("parser-discovered source load must not hold the streaming boundary")
            .expect("parser step should succeed");

            let ParserStepAdvanceOutcome::BlockedOnExternalSource(pending) = outcome else {
                panic!("parser-discovered parser-blocking load should return a streaming boundary");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url
                    .as_str(),
                "https://example.test/blocking.js"
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(matches!(
                parser_blocking_classic_source_load_for_test(pending_script),
                Some(PendingParserBlockingSourceLoad::ParserDiscovered(_))
            ));
        }));
}
#[test]
fn parser_blocking_pending_late_preload_starts_parser_discovered_source_load() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
        let PhaseOnePageVmHarness {
            mut page_vm,
            loader,
            state,
        } = new_phase_one_page_vm_harness_for_test();
        activate_standalone_main_parser_continuation_for_test(&mut page_vm);
        let mut script = prepared_external_classic("https://example.test/late.js");
        state.buffered_document_preloads.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            BufferedScriptPreloadEntry {
                request: preload_request_for_script(
                    &script,
                    moli_fetch::RequestResourceType::LatePreloadScript,
                ),
                load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
            },
        );

        let decision = prepare_main_parser_blocking_source_load(
            &mut page_vm,
            loader,
            &mut state.buffered_document_preloads,
            &mut script,
        );

        assert!(matches!(
            decision.disposition,
            MainParserBlockingSourceDisposition::Pending(
                PendingParserBlockingSourceLoad::ParserDiscovered(_)
            )
        ));
        assert!(
            decision.applied_preload.is_none(),
            "a pending late preload must not become the parser-blocking request fact source"
        );
    }));
}
#[test]
fn parser_blocking_source_boundary_scans_later_preloads_without_parsing_later_dom() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
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

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><script src="/blocking.js"></script><script defer src="/later.js"></script></head><body><div id="after-blocking">after</div></body></html>"#,
                    None,
                )
                .await
                .expect("parser step should reach the external source boundary");

            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "external parser-blocking source should yield a streaming boundary"
            );
            assert!(
                driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/later.js")),
                "future defer scripts should be visible to the preload scanner at the parser-blocking boundary"
            );
            assert!(
                !native_dom_has_element_id(
                    &page_vm.vm().snapshot_live_document(),
                    "after-blocking"
                ),
                "parser-visible DOM must not advance past the parser-blocking script"
            );
        }));
}
#[test]
fn stylesheet_gated_parser_blocking_script_reuses_completed_preload() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let script_body = "document.documentElement.setAttribute('data-preload-used', 'yes');";
            let (script_url, script_requests, stylesheet_release, server) =
                spawn_counting_parser_asset_server(script_body).await;
            let stylesheet_url = script_url.join("app.css").expect("stylesheet url");
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let final_url = state.final_url.clone();
            state.buffered_document_preloads.append_to_main_document_scan(
                &final_url,
                &format!(r#"<script src="{script_url}"></script>"#),
                loader,
            );
            let preload = state
                .buffered_document_preloads
                .entries
                .load_for_key(&classic_preload_key(script_url.as_str()))
                .expect("preload scanner should start the parser-blocking script");
            let preload_outcome = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                preload.wait_outcome(),
            )
            .await
            .expect("script preload should complete before the stylesheet gate is released");
            assert_eq!(
                preload_outcome
                    .source_result
                    .expect("script preload should load source"),
                script_body
            );
            assert_eq!(script_requests.load(Ordering::SeqCst), 1);

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    &format!(
                        r#"<!doctype html><html><head><link rel="stylesheet" href="{stylesheet_url}"><script src="{script_url}"></script></head></html>"#
                    ),
                    None,
                )
                .await
                .expect("parser step should reach the stylesheet gate");
            let ParserStepAdvanceOutcome::BlockedOnStylesheet(mut pending) = outcome else {
                panic!("parser-blocking script should remain gated on the stylesheet");
            };

            pending
                .script_mut()
                .context_mut()
                .blocking_signatures_before
                .clear();
            page_vm.vm_mut().document_runtime.install_pending_main_parser_script(pending.into_script());
            stylesheet_release.notify_one();
            let mut owner = ParseTimeOwner::Parser;
            let mut parser_step_ready = true;
            let mut pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
            let parser_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("parser document owner should be current");
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let owner_ptr: *mut ParseTimeOwner = &mut owner;
            let parser_step_ready_ptr: *mut bool = &mut parser_step_ready;
            let pending_parsing_blocking_wait_ptr: *mut PendingParsingBlockingWait =
                &mut pending_parsing_blocking_wait;
            let progress = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                super::access::run_named_owner_local_task(
                    local_executor,
                    "stylesheet-gated preload reuse test channel closed",
                    async move {
                        let page_vm = unsafe { &mut *page_vm_ptr };
                        let driver = unsafe { &mut *driver_ptr };
                        let owner = unsafe { &mut *owner_ptr };
                        let parser_step_ready = unsafe { &mut *parser_step_ready_ptr };
                        let pending_parsing_blocking_wait =
                            unsafe { &mut *pending_parsing_blocking_wait_ptr };
                        driver
                            .drive_owner_step(
                                owner,
                                parser_step_ready,
                                pending_parsing_blocking_wait,
                                parser_document_owner,
                                page_vm,
                            )
                            .await
                    },
                ),
            )
            .await
            .expect("stylesheet-unblocked parser script should finish")
            .expect("stylesheet-unblocked parser step should run on the owner lane");
            let observed_script_requests = script_requests.load(Ordering::SeqCst);
            let snapshot = page_vm.vm().snapshot_live_document();
            let document_element = snapshot
                .document_element_handle()
                .expect("parser should create a document element");
            let preload_marker = snapshot
                .node(document_element)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-preload-used"));
            server.abort();
            let _ = server.await;

            assert_eq!(progress, OwnerStepProgress::Continue);
            assert_eq!(preload_marker, Some("yes"));
            assert_eq!(
                observed_script_requests, 1,
                "the completed speculative preload must satisfy the stylesheet-unblocked parser script"
            );
        }));
}
#[test]
fn buffered_script_preload_cache_reuses_ready_late_parser_blocking_preload() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut script = prepared_external_classic("https://example.test/late-ready.js");
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
            ready_preload_entry_for_script_with_resource_type(
                &script,
                "window.lateReady = 1;",
                moli_fetch::RequestResourceType::LatePreloadScript,
            ),
        );

        assert!(
            cache
                .apply_preloaded_source_to_script_if_available(&mut script, true)
                .await
                .is_some()
        );
        assert!(matches!(
            &script.source,
            ScriptSource::Loaded(source) if source == "window.lateReady = 1;"
        ));
    });
}
#[tokio::test]
async fn buffered_script_preload_cache_uses_bound_owner_wake() {
    let script_body = "window.bufferedPreloadWake = true;";
    let (script_url, server) = spawn_single_script_server(script_body).await;
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut cache = BufferedDocumentPreloadState::default();
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let wake_page_id = PageId::new_for_testing(89);
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(wake_page_id),
    );
    cache.bind_resource_runtime(
        Some(owner_wake),
        Some(
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("Tokio test should expose its resource task runner"),
        ),
    );
    cache.append_to_main_document_scan(
        &final_url,
        &format!(r#"<script defer src="{script_url}"></script>"#),
        &loader,
    );

    let preload = cache
        .entries
        .load_for_key(&classic_preload_key(script_url.as_str()))
        .expect("main-document scan should create script preload");
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), preload.wait_outcome())
        .await
        .expect("ordinary buffered preload should finish");
    assert_eq!(
        outcome
            .source_result
            .expect("ordinary buffered preload should load source"),
        script_body
    );
    let wake = tokio::time::timeout(std::time::Duration::from_secs(1), wake_rx.recv())
        .await
        .expect("ordinary buffered preload should signal owner wake")
        .expect("owner wake channel should remain open");
    assert_eq!(wake.page_id(), wake_page_id);
    assert!(matches!(
        wake,
        crate::page_task_queue::RendererOwnerWake::Page {
            source: crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork,
            ..
        }
    ));
    server.await.expect("test script server should finish");
}
#[tokio::test]
async fn parser_driver_finish_parser_blocking_pause_uses_service_worker_preload_context() {
    let script_body = "window.documentWritePreload = true;";
    let (script_url, server) = spawn_single_script_server(script_body).await;
    let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
    let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url.clone());
    let browser_context_owner = crate::runtime::RendererBrowserContextRuntime::new();
    let browser_context_runtime = browser_context_owner.handle();
    let completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
    let client_id = browser_context_runtime.register_service_worker_client(
        final_url.clone(),
        moli_storage_key::MoliStorageKey::first_party_from_url(&final_url, None)
            .serialized_storage_key(),
        crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
        Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
        completion_queue.sender(),
    );
    let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let wake_page_id = PageId::new_for_testing(88);
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(wake_page_id),
    );
    state.buffered_document_preloads.bind_resource_runtime(
        Some(owner_wake.clone()),
        Some(
            crate::network::RendererResourceTaskRunner::from_current_tokio()
                .expect("service-worker preload test requires its Tokio runtime"),
        ),
    );
    state.service_worker_preload_context = Some(ServiceWorkerScriptPreloadContext::new(
        browser_context_runtime,
        client_id,
        final_url.clone(),
        Some(owner_wake),
    ));
    let session = state
        .parser_session
        .stream_handle()
        .borrow()
        .script_input_session();
    session.enqueue_script_input_preload_html(format!(r#"<script src="{script_url}"></script>"#));

    let mut driver = ParserDriver {
        loader: &loader,
        final_url: &state.final_url,
        parser_session: &mut state.parser_session,
        scheduler: &mut state.scheduler,
        buffered_document_preloads: &mut state.buffered_document_preloads,
        service_worker_preload_context: state.service_worker_preload_context.as_ref(),
        input_closed: &state.input_closed,
    };
    driver.finish_parser_blocking_pause();

    let preload = driver
        .buffered_document_preloads
        .entries
        .load_for_key(&classic_preload_key(script_url.as_str()))
        .expect("document.write insertion should create script preload");
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), preload.wait_outcome())
        .await
        .expect("document.write insertion preload should finish");
    assert_eq!(
        outcome
            .source_result
            .expect("document.write insertion preload should load source"),
        script_body
    );
    let wake = tokio::time::timeout(std::time::Duration::from_secs(1), wake_rx.recv())
        .await
        .expect("service-worker-aware insertion preload should signal owner wake")
        .expect("owner wake channel should remain open");
    assert_eq!(wake.page_id(), wake_page_id);
    assert!(matches!(
        wake,
        crate::page_task_queue::RendererOwnerWake::Page {
            source: crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork,
            ..
        }
    ));
    server.await.expect("test script server should finish");
    drop(completion_queue);
}
#[test]
fn parser_driver_finish_parser_blocking_pause_resets_insertion_scanner_state() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime should build");

    runtime.block_on(async move {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
        bind_preload_state_to_current_test_runtime(&mut state.buffered_document_preloads);
        let session = state
            .parser_session
            .stream_handle()
            .borrow()
            .script_input_session();

        let mut driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        session.enqueue_script_input_preload_html("<script sr".to_owned());
        driver.finish_parser_blocking_pause();
        session.enqueue_script_input_preload_html("c=\"/write.js\"></script>".to_owned());
        driver.finish_parser_blocking_pause();

        assert!(
            !driver
                .buffered_document_preloads
                .entries
                .contains_key(&classic_preload_key("https://example.test/write.js")),
            "partial insertion scanner state must not leak across separate parser-blocking pauses"
        );
    });
}
