use super::*;

#[test]
fn csp_metadata_uses_native_referrers_without_reading_document_overrides() {
    let mut vm = new_storage_page_task_executor_test_vm("https://native-csp-metadata.test/page");
    let mut policy = vm.document_runtime.document_policy_container().clone();
    policy.document_referrer = "https://native-referrer.test/previous".to_owned();
    vm.document_runtime
        .set_main_navigation_policy_container(policy);
    let document = vm.document_runtime.document_handle();
    vm.document_runtime
        .dom_host_mut()
        .set_document_source_response_status_for_handle(document, Some(201));
    vm.eval(r#"
globalThis.metadataFrame = document.body.appendChild(document.createElement('iframe'));
globalThis.metadataPopup = open();
globalThis.metadataRows = [];
const views = [window, metadataFrame.contentWindow, metadataPopup];
for (const [index, view] of views.entries()) {
  const doc = view.document;
  const nativeReferrer = Object.getOwnPropertyDescriptor(Document.prototype, 'referrer').get.call(doc);
  let authorReads = 0;
  Object.defineProperty(doc, 'referrer', {configurable:true, get() { authorReads++; throw Error('author referrer'); }});
  const meta = doc.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy'; meta.content = "connect-src 'none'";
  doc.head.appendChild(meta);
  doc.addEventListener('securitypolicyviolation', event => {
    metadataRows.push({index, matched:event.referrer === nativeReferrer, referrer:event.referrer,
      status:event.statusCode, trusted:event.isTrusted, target:event.target === doc, authorReads});
    delete doc.referrer;
  }, {once:true});
  view.fetch('data:text/plain,blocked').catch(error => {
    if (!(error instanceof (view === metadataPopup ? TypeError : view.TypeError))) throw error;
  });
}
"#).unwrap();
    let result: serde_json::Value = serde_json::from_str(
        &vm.eval_after_selected_page_tasks(
            "JSON.stringify(metadataRows.sort((a,b) => a.index-b.index))",
        )
        .unwrap(),
    )
    .unwrap();
    let rows = result.as_array().unwrap();
    assert_eq!(rows.len(), 3, "{result}");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["index"], index, "{result}");
        assert_eq!(row["matched"], true, "{result}");
        assert_eq!(row["trusted"], true, "{result}");
        assert_eq!(row["target"], true, "{result}");
        assert_eq!(row["authorReads"], 0, "{result}");
        assert_eq!(row["status"], if index == 0 { 201 } else { 0 }, "{result}");
    }
    assert_eq!(rows[0]["referrer"], "https://native-referrer.test/previous");
    vm.eval("metadataPopup.close(); metadataFrame.remove()")
        .unwrap();
}
