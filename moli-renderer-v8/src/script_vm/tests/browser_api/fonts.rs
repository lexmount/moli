use super::*;
use base64::Engine;
use std::time::Duration;

pub(super) fn fixture_font_url() -> String {
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem.woff2"
    ));
    format!(
        "data:font/woff2;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[test]
fn font_face_missing_local_source_rejects_instead_of_matching_css_fallback() {
    let mut vm = new_storage_test_vm("https://font-sources.test/");
    vm.eval(r#"
globalThis.fontResult = [];
const face = new FontFace('Missing Face', 'local("MoliDefinitelyNotAFont-0001345")');
fontResult.push(face.status, face.loaded === face.loaded);
document.fonts.add(face);
fontResult.push(document.fonts.check('12px "Missing Face"'), document.fonts.check('12px UnregisteredFace'));
const promise = face.load();
fontResult.push(promise === face.loaded, face.status);
promise.then(() => fontResult.push('WRONG success'), error => fontResult.push(error.name, face.status));
document.fonts.load('12px "Missing Face"').then(() => fontResult.push('WRONG set success'), error => fontResult.push(error.name));
"#).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"["unloaded",true,false,true,true,"error","NetworkError","error","NetworkError"]"#
    );
}

#[test]
fn font_face_valid_data_sources_register_and_unregister_real_font_bytes() {
    let mut vm = new_storage_test_vm("https://font-sources.test/");
    let url = fixture_font_url();
    vm.eval(&format!(r#"
globalThis.fontResult = [];
globalThis.face = new FontFace('Actual Face', 'local("MoliDefinitelyNotAFont"), url("{url}") format("woff2")');
document.fonts.add(face);
fontResult.push(face.status, document.fonts.check('italic 12px "Actual Face", serif'));
document.fonts.load('12px "Actual Face", serif').then(faces => fontResult.push(faces.length, faces[0] === face, face.status, document.fonts.check('12px "Actual Face"')));
"#)).unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"["unloaded",false,1,true,"loaded",true]"#
    );
    assert_eq!(vm.document_web_font_counts_for_test().2, 1);
    vm.eval("face.family = 'Renamed Face'").unwrap();
    assert_eq!(
        vm.document_web_font_counts_for_test().2,
        1,
        "descriptor replacement must not leak the old registration"
    );
    vm.eval("document.fonts.delete(face)").unwrap();
    assert_eq!(vm.document_web_font_counts_for_test().2, 0);
}

#[test]
fn font_face_binary_sources_require_real_decodable_fonts() {
    let mut vm = new_storage_test_vm("https://font-sources.test/");
    let encoded = fixture_font_url().split_once(',').unwrap().1.to_owned();
    vm.eval(&format!(
        r#"
globalThis.fontResult = [];
const bytes = Uint8Array.from(atob('{encoded}'), c => c.charCodeAt(0));
const face = new FontFace('Binary Face', bytes);
fontResult.push(face.status);
face.loaded.then(value => fontResult.push(value === face));
document.fonts.add(face);
for (const header of [[0,1,0,0], [79,84,84,79], [119,79,70,70], [119,79,70,50]]) {{
  const bad = new FontFace('Bad Face', new Uint8Array(header));
  fontResult.push(bad.status);
  bad.loaded.catch(error => fontResult.push(error.name));
}}
"#
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"["loaded","error","error","error","error",true,"SyntaxError","SyntaxError","SyntaxError","SyntaxError"]"#
    );
    assert_eq!(vm.document_web_font_counts_for_test().2, 1);
    vm.eval("document.fonts.clear()").unwrap();
    assert_eq!(vm.document_web_font_counts_for_test().2, 0);
}

#[test]
fn font_set_load_and_check_reject_invalid_shorthand_without_page_property_reads() {
    let mut vm = new_storage_test_vm("https://font-sources.test/");
    vm.eval(
        r#"
globalThis.fontResult = [];
const face = new FontFace('Test', 'local("Missing Font 9944")');
document.fonts.add(face);
Object.defineProperty(face, 'family', {get() {throw new Error('must not read author getter');}});
fontResult.push(document.fonts.check('12px Test'));
try { document.fonts.check('not a font'); } catch(e) { fontResult.push(e.name); }
document.fonts.load('not a font').catch(e => fontResult.push(e.name));
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"[false,"SyntaxError","SyntaxError"]"#
    );
}

#[tokio::test]
async fn font_face_url_loading_waits_for_the_network_terminal_and_registers_the_response() {
    let (url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let document_url = url.replace("/print-only.woff2", "/page");
    let (mut vm, mut completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);
    vm.eval(&format!(r#"
globalThis.fontResult = [];
globalThis.face = new FontFace('Network Face', 'url("{url}")');
document.fonts.add(face);
document.fonts.load('12px "Network Face"').then(faces => fontResult.push(faces[0] === face, face.status));
"#)).unwrap();
    assert_eq!(
        vm.eval("[face.status, document.fonts.status, fontResult.length].join('|')")
            .unwrap(),
        "loading|loading|0"
    );
    assert_eq!(vm.document_web_font_counts_for_test().2, 0);
    let request = tokio::time::timeout(Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        request
            .to_ascii_lowercase()
            .contains("sec-fetch-dest: font"),
        "{request}"
    );
    release_tx.send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        "[true,\"loaded\"]"
    );
    assert_eq!(vm.document_web_font_counts_for_test().2, 1);
    assert_eq!(vm.eval("document.fonts.status").unwrap(), "loaded");
}

#[test]
fn font_face_url_sources_enforce_font_src_instead_of_connect_src() {
    let mut vm = new_storage_test_vm("https://font-sources.test/");
    vm.set_response_content_security_policies(&["font-src 'none'; connect-src * data:".to_owned()]);
    vm.eval(&format!(
        r#"
globalThis.fontResult = [];
const face = new FontFace('Blocked Font', 'url("{}")');
face.load().catch(error => fontResult.push(error.name, face.status));
"#,
        fixture_font_url()
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        "[\"NetworkError\",\"error\"]"
    );
    assert_eq!(vm.document_web_font_counts_for_test().2, 0);
}

async fn assert_css_and_js_share_font_loading(css_starts: bool, remove_rule: bool) {
    let (url, request_rx, release_tx, server) = spawn_gated_font_resource_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let document_url = url.replace("/print-only.woff2", "/page");
    let (mut vm, mut completions) =
        new_storage_test_vm_with_loader_and_resource_completion_queue(&document_url, &loader);
    vm.eval(&format!(
        r#"
const html = document.createElement('html');
html.innerHTML = '<head></head><body>AAAA</body>';
document.append(html);
const style = document.createElement('style');
style.textContent = '@font-face {{ font-family: SharedFace; src: url("{url}"); }}';
(document.head || document.documentElement).append(style);
globalThis.face = [...document.fonts][0];
globalThis.fontResult = [];
globalThis.fontPromise = face.loaded;
fontPromise.then(value => fontResult.push(value === face, face.status));
"#
    ))
    .unwrap();
    assert_eq!(vm.eval("face.status").unwrap(), "unloaded");
    if !css_starts {
        assert_eq!(vm.eval("face.load() === fontPromise").unwrap(), "true");
    }
    vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
        .unwrap();
    assert_eq!(
        vm.eval("[face.status, face.load() === fontPromise, document.fonts.status, fontResult.length].join('|')").unwrap(),
        "loading|true|loading|0"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        1,
        "CSS and JS must reuse the same in-flight font, whichever starts first"
    );
    tokio::time::timeout(Duration::from_secs(2), request_rx)
        .await
        .unwrap()
        .unwrap();
    if remove_rule {
        vm.eval("style.remove()").unwrap();
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
            .unwrap();
        assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    }
    release_tx.send(()).unwrap();
    server.await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(2),
            completions.wait_for_arrival_without_timeout()
        )
        .await
        .unwrap()
    );
    let completion = completions.pop_next_async_subresource_event().unwrap();
    let activity = vm
        .complete_async_subresource_fetch_event_body(completion)
        .unwrap();
    vm.finish_async_subresource_body_checkpoint_for_test(activity)
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"[true,"loaded"]"#
    );
    assert_eq!(vm.eval("document.fonts.status").unwrap(), "loaded");
    assert_eq!(
        vm.document_web_font_counts_for_test(),
        if remove_rule { (0, 0, 0) } else { (1, 1, 1) }
    );
    assert_eq!(
        vm.eval("document.fonts.check('12px SharedFace')").unwrap(),
        "true"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0
    );
}

#[tokio::test]
async fn font_face_observes_the_stylesheet_request_without_starting_another() {
    assert_css_and_js_share_font_loading(true, false).await;
}

#[tokio::test]
async fn stylesheet_reuses_the_css_connected_font_face_started_by_javascript() {
    assert_css_and_js_share_font_loading(false, false).await;
}

#[tokio::test]
async fn removed_stylesheet_font_settles_javascript_without_restoring_registration() {
    assert_css_and_js_share_font_loading(true, true).await;
}

#[tokio::test]
async fn failed_stylesheet_font_rejects_the_connected_font_face_promise() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, _) = new_storage_test_vm_with_loader_and_resource_completion_queue(
        "https://font-sources.test/",
        &loader,
    );
    vm.eval(r#"
const html = document.createElement('html');
html.innerHTML = '<head></head><body>AAAA</body>';
document.append(html);
const style = document.createElement('style');
style.textContent = '@font-face { font-family: FailedFace; src: url("data:font/woff2;base64,YmFk"); }';
(document.head || document.documentElement).append(style);
globalThis.face = [...document.fonts][0];
globalThis.fontResult = [];
face.loaded.catch(error => fontResult.push(error.name, face.status));
"#).unwrap();
    vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
        .unwrap();
    // Layout demand is body-only. This evaluation observes the native terminal
    // and runs the normal input-independent microtask checkpoint for its promise.
    assert_eq!(vm.eval("face.status").unwrap(), "error");
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"["NetworkError","error"]"#
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 0, 0));
    assert_eq!(
        vm.eval("document.fonts.check('12px FailedFace')").unwrap(),
        "false"
    );
}

#[tokio::test]
async fn stylesheet_font_source_fallback_settles_the_connected_font_face() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, _) = new_storage_test_vm_with_loader_and_resource_completion_queue(
        "https://font-sources.test/",
        &loader,
    );
    vm.eval(&format!(
        r#"
const html = document.createElement('html');
html.innerHTML = '<head></head><body>AAAA</body>';
document.append(html);
const style = document.createElement('style');
style.textContent = `@font-face {{
    font-family: FallbackFace;
    src: local("MoliDefinitelyNotAFont-0001345"),
         url("data:font/woff2;base64,YmFk"), url("{}");
}}`;
(document.head || document.documentElement).append(style);
globalThis.face = [...document.fonts][0];
globalThis.fontResult = [];
face.loaded.then(value => fontResult.push(value === face, face.status));
"#,
        fixture_font_url()
    ))
    .unwrap();
    vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
        .unwrap();
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(
        vm.eval("JSON.stringify(fontResult)").unwrap(),
        r#"[true,"loaded"]"#
    );
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 1, 1));
    vm.eval("style.remove()").unwrap();
    vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
        .unwrap();
    assert_eq!(vm.document_web_font_counts_for_test(), (0, 0, 0));
    assert_eq!(
        vm.eval("face.status").unwrap(),
        "loaded",
        "removing CSS membership does not unload the held FontFace"
    );
}

#[tokio::test]
async fn loaded_stylesheet_font_is_reused_by_a_late_native_wrapper() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
    let (mut vm, _) = new_storage_test_vm_with_loader_and_resource_completion_queue(
        "https://font-sources.test/",
        &loader,
    );
    vm.eval(&format!(r#"
const html = document.createElement('html');
html.innerHTML = '<head></head><body>AAAA</body>';
document.append(html);
const style = document.createElement('style');
style.textContent = '@font-face {{ font-family: LateFace; src: url("{}"); font-weight: 700; font-style: italic; }}';
document.head.append(style);
globalThis.NativeFontFace = FontFace;
globalThis.FontFace = function() {{ throw new Error('CSS must not call the page constructor'); }};
"#, fixture_font_url())).unwrap();
    vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0))
        .unwrap();
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 1, 1));
    assert_eq!(
        vm.eval(
            r#"
globalThis.face = [...document.fonts][0];
globalThis.fontResult = [];
face.load().then(value => fontResult.push(value === face));
[face instanceof NativeFontFace, face.status, face.weight, face.style].join('|')
"#
        )
        .unwrap(),
        "true|loaded|700|italic"
    );
    assert_eq!(vm.eval("JSON.stringify(fontResult)").unwrap(), "[true]");
    assert_eq!(vm.document_web_font_counts_for_test(), (1, 1, 1));
}
