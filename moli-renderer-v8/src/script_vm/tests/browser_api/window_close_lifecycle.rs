use super::*;

#[tokio::test]
async fn popup_close_beforeunload_keeps_history_until_retirement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://popup-close-lifecycle.test/",
        &loader,
    );
    let result = vm
        .eval(
            r#"
(() => {
  const popup = globalThis.closingPopup = open('about:blank', 'closing-history');
  globalThis.closingDocument = popup.document;
  globalThis.closingHistory = popup.history;
  const state = {calls:0, listeners:0, closed:null, history:null, active:null, error:null};
  popup.onbeforeunload = () => {
    state.calls++;
    state.closed = popup.closed;
    state.active = closingDocument.location === popup.location;
    try { state.history = closingHistory.length; }
    catch (error) { state.error = error.name; }
    popup.close();
  };
  popup.addEventListener('beforeunload', () => { state.listeners++; });
  popup.close();
  globalThis.afterClose = JSON.stringify({closed: popup.closed, history: closingHistory.length});
  return JSON.stringify(state);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"calls":1,"listeners":1,"closed":false,"history":1,"active":true,"error":null}"#
    );
    assert_eq!(
        vm.eval("afterClose").unwrap(),
        r#"{"closed":true,"history":1}"#
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(closingPopup.closed && closingDocument.defaultView === null)",
        "true",
        "queued popup close retires its Document",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"(() => {
      try { return String(closingHistory.length); }
      catch (error) { return error.name; }
    })()"#
        )
        .unwrap(),
        "SecurityError"
    );
    assert_eq!(
        vm.eval("String(closingDocument.location === null)")
            .unwrap(),
        "true"
    );
}
