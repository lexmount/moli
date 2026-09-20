use super::*;

#[test]
fn popup_content_handlers_use_their_window_document_form_and_element_scopes() {
    let mut vm = new_storage_test_vm("https://popup-content-handler.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const popup = open();
  try {
    const d = popup.document;
    d.body.innerHTML = '<form><button id="button" type="button"></button></form>';
    const button = d.getElementById('button');
    const form = button.form;
    const trace = popup.__handlerTrace = [];
    popup.scopeToken = 'window';
    d.scopeToken = 'document';
    form.scopeToken = 'form';
    button.scopeToken = 'element';
    button.setAttribute('onclick', `
      globalThis.__handlerTrace.push([globalThis === window, window !== opener,
        document === ownerDocument, form === document.forms[0],
        this === document.getElementById('button'), scopeToken,
        event.type, arguments.length]);
      location.hash = 'handled';
      return false;
    `);
    const handler = button.onclick;
    const canceled = [];
    for (const object of [button, form, d, popup]) {
      canceled.push(!button.dispatchEvent(new Event('click', {cancelable:true})));
      delete object.scopeToken;
    }
    return JSON.stringify({type:typeof handler, cached:handler === button.onclick,
      trace, canceled, popupHash:popup.location.hash, openerHash:location.hash,
      openerUntouched:!Object.hasOwn(window, '__handlerTrace')});
  } finally {
    popup.close();
  }
})()
"#,
        )
        .expect("popup content handler scope probe should evaluate");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "type": "function",
            "cached": true,
            "trace": [
                [true, true, true, true, true, "element", "click", 1],
                [true, true, true, true, true, "form", "click", 1],
                [true, true, true, true, true, "document", "click", 1],
                [true, true, true, true, true, "window", "click", 1]
            ],
            "canceled": [true, true, true, true],
            "popupHash": "#handled",
            "openerHash": "",
            "openerUntouched": true
        })
    );
}

#[test]
fn popup_content_handler_compile_errors_report_to_the_popup_and_preserve_reentry() {
    let mut vm = new_storage_test_vm("https://popup-content-handler-error.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const popup = open();
  try {
    const button = popup.document.createElement('button');
    popup.document.body.appendChild(button);
    const trace = [];
    let openerErrors = 0;
    window.addEventListener('error', () => ++openerErrors);
    const replacement = () => trace.push('replacement');
    popup.addEventListener('error', event => {
      trace.push([event.target === popup, event.error.name === 'SyntaxError',
        button.onclick === null]);
      button.onclick = replacement;
      event.preventDefault();
    });
    button.setAttribute('onclick', '}');
    const before = trace.length;
    const firstNull = button.onclick === null;
    const replacementPreserved = button.onclick === replacement;
    button.click();
    return JSON.stringify({before, firstNull, replacementPreserved, trace, openerErrors});
  } finally {
    popup.close();
  }
})()
"#,
        )
        .expect("popup content handler syntax error probe should evaluate");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "before": 0,
            "firstNull": true,
            "replacementPreserved": true,
            "trace": [[true, true, true], "replacement"],
            "openerErrors": 0
        })
    );
}

#[tokio::test]
async fn popup_content_handler_timers_retire_with_the_popup() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://popup-content-handler-timer.test/",
        &loader,
    );
    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__popupContentTimerCalls = 0;
  globalThis.__openerContentTimerDone = false;
  const popup = open();
  const button = popup.document.createElement('button');
  popup.document.body.appendChild(button);
  button.setAttribute('onclick', `
    setTimeout(() => ++opener.__popupContentTimerCalls, 50);
  `);
  const compiled = typeof button.onclick;
  button.click();
  popup.close();
  setTimeout(() => { __openerContentTimerDone = true; }, 100);
  return compiled;
})()
"#,
        )
        .expect("popup content handler timer probe should evaluate"),
        "function"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__openerContentTimerDone)",
        "true",
        "opener timer after popup close",
    )
    .await;
    assert_eq!(vm.eval("String(__popupContentTimerCalls)").unwrap(), "0");
}
