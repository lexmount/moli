use super::*;

#[tokio::test(flavor = "current_thread")]
async fn document_open_checks_entry_origin_without_document_domain_relaxation() {
    const HOST: &str = "document-open-origin.test";
    let child = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/document-open-origin-child.html"
    ));
    let same = StaticHttpServer::spawn_with_bodies(vec![child.to_owned()]).await;
    let cross = StaticHttpServer::spawn_with_bodies(vec![child.to_owned()]).await;
    let loader = static_http_loader([same.resolve_entry(HOST), cross.resolve_entry(HOST)]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        same.url_for_host(HOST, "/entry.html").as_str(),
        "<!doctype html><body>parent",
        &loader,
    );
    let script = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/document-open-origin.js"
    ));
    vm.exec(
        &format!(
            r#"
globalThis.__documentOpenOriginResult = null;
({script})({{sameURL:{same_url:?},crossURL:{cross_url:?}}}).then(
  result => {{__documentOpenOriginResult = result}},
  error => {{__documentOpenOriginResult = {{error:String(error)}}}}
);
"#,
            same_url = same.url_for_host(HOST, "/child.html").as_str(),
            cross_url = cross.url_for_host(HOST, "/child.html").as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__documentOpenOriginResult !== null)",
        "true",
        "document.open origin checks should finish",
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval("JSON.stringify(__documentOpenOriginResult)")
            .unwrap(),
    )
    .unwrap();
    assert!(
        result["checks"].as_u64().is_some_and(|count| count > 100),
        "{result}"
    );
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    assert_eq!(same.finish_targets().await, ["/child.html"]);
    assert_eq!(cross.finish_targets().await, ["/child.html"]);
}

#[test]
fn document_open_accepts_inherited_opaque_origins() {
    let mut vm = new_parsed_test_vm("about:blank", "<!doctype html><body>opaque");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.body.appendChild(document.createElement('iframe'));
  const documents = [frame.contentDocument,
    document.implementation.createHTMLDocument('opaque'),
    new DOMParser().parseFromString('<p>opaque</p>', 'text/html')];
  for (const doc of documents) {
    if (doc.open() !== doc) throw new Error('open return value');
    doc.write('<p>allowed</p>'); doc.close();
    if (doc.body.textContent !== 'allowed') throw new Error('inherited opaque origin rejected');
  }
  return 'ok';
})()
"#,
        )
        .unwrap(),
        "ok"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn document_open_keeps_popup_origins_after_the_popup_closes() {
    const HOST: &str = "popup-open-origin.test";
    let server = StaticHttpServer::spawn_with_bodies(vec![
        r#"<!doctype html><body>popup<script>
document.domain = location.hostname;
window.originDocument = document.implementation.createHTMLDocument('popup');
window.originParser = new DOMParser();
opener.postMessage('popup-origin-ready', '*');
</script>"#
            .to_owned(),
    ])
    .await;
    let loader = static_http_loader([server.resolve_entry(HOST)]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        "http://popup-open-origin.test:8443/entry.html",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.exec(
        &format!(
            r#"
document.domain = location.hostname;
globalThis.__popupOriginReady = false;
addEventListener('message', event => {{
  if (event.data === 'popup-origin-ready') __popupOriginReady = true;
}});
globalThis.__originPopup = open({url:?});
"#,
            url = server.url_for_host(HOST, "/popup.html").as_str(),
        ),
        None,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupOriginReady)",
        "true",
        "cross-origin popup should finish initializing",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const popup = __originPopup;
  const documents = [popup.document, popup.originDocument,
    DOMParser.prototype.parseFromString.call(popup.originParser, '<p>parsed</p>', 'text/html')];
  for (const close of [false, true]) {
    if (close) popup.close();
    for (const doc of documents) {
      const original = doc.documentElement;
      for (const method of ['open', 'write', 'writeln']) {
        let caught;
        try { Document.prototype[method].call(doc, 'replacement'); }
        catch (error) { caught = error; }
        if (!(caught instanceof DOMException) || caught.name !== 'SecurityError' || caught.code !== 18)
          throw new Error('popup origin was lost: ' + close + '/' + method + '/' + caught);
        if (doc.documentElement !== original) throw new Error('rejected write changed popup document');
      }
    }
  }
  return 'ok';
})()
"#,
        )
        .unwrap(),
        "ok"
    );
    assert_eq!(server.finish_targets().await, ["/popup.html"]);
}
