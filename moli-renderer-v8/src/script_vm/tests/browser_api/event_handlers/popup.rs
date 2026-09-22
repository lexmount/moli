use super::*;

#[test]
fn popup_dom_events_propagate_through_their_window_without_reaching_the_opener() {
    let mut vm = new_storage_test_vm("https://popup-event-path.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const popup = open();
  try {
    const doc = popup.document;
    const button = doc.body.appendChild(doc.createElement('button'));
    const trace = [];
    const label = node => node === popup ? 'popup' : node === doc ? 'document' :
      node === button ? 'button' : node === doc.body ? 'body' :
      node === doc.documentElement ? 'html' : 'foreign';
    let expectedTarget;
    const record = name => function(event) {
      trace.push([name, event.eventPhase, event.target === expectedTarget,
        this === event.currentTarget, event.composedPath().map(label)]);
    };
    window.addEventListener('popup-probe', record('opener-capture'), true);
    window.addEventListener('popup-probe', record('opener-bubble'));
    popup.addEventListener('popup-probe', record('window-capture'), true);
    popup.addEventListener('popup-probe', record('window-bubble'));
    doc.addEventListener('popup-probe', record('document-capture'), true);
    doc.addEventListener('popup-probe', record('document-bubble'));
    button.addEventListener('popup-probe', record('button'));
    const results = [];
    for (const target of [doc, button]) {
      for (const bubbles of [false, true]) {
        trace.length = 0;
        expectedTarget = target;
        const event = new Event('popup-probe', {bubbles});
        target.dispatchEvent(event);
        results.push({trace:trace.slice(), clean:event.currentTarget === null &&
          event.eventPhase === 0 && event.composedPath().length === 0 && event.target === target});
      }
    }
    return JSON.stringify(results);
  } finally { popup.close(); }
})()
"#,
        )
        .expect("popup event path probe should run");
    let doc_path = serde_json::json!(["document", "popup"]);
    let button_path = serde_json::json!(["button", "body", "html", "document", "popup"]);
    let row = |name: &str, phase: u8, path: &serde_json::Value| {
        serde_json::json!([name, phase, true, true, path])
    };
    let doc_capture = vec![
        row("window-capture", 1, &doc_path),
        row("document-capture", 2, &doc_path),
        row("document-bubble", 2, &doc_path),
    ];
    let mut doc_bubble = doc_capture.clone();
    doc_bubble.push(row("window-bubble", 3, &doc_path));
    let button_capture = vec![
        row("window-capture", 1, &button_path),
        row("document-capture", 1, &button_path),
        row("button", 2, &button_path),
    ];
    let mut button_bubble = button_capture.clone();
    button_bubble.extend([
        row("document-bubble", 3, &button_path),
        row("window-bubble", 3, &button_path),
    ]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([
            {"trace":doc_capture, "clean":true},
            {"trace":doc_bubble, "clean":true},
            {"trace":button_capture, "clean":true},
            {"trace":button_bubble, "clean":true},
        ])
    );
}

#[test]
fn popup_dom_event_paths_keep_child_frames_and_load_events_isolated() {
    let mut vm = new_storage_test_vm("https://popup-event-boundaries.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const popup = open();
  const sibling = open();
  try {
    const doc = popup.document;
    const frame = doc.body.appendChild(doc.createElement('iframe'));
    const child = frame.contentWindow;
    const trace = [];
    for (const [name, target] of [['opener',window], ['sibling',sibling], ['popup',popup]]) {
      target.addEventListener('child-probe', () => trace.push(name), true);
      target.addEventListener('load', () => trace.push(name + '-load'), true);
    }
    child.addEventListener('child-probe', event => {
      trace.push(['child-capture',event.eventPhase,event.target === child.document,
        event.composedPath().length === 2,event.composedPath().at(-1) === child]);
    }, true);
    child.addEventListener('child-probe', event => trace.push(['child-bubble',event.eventPhase]));
    child.document.dispatchEvent(new Event('child-probe', {bubbles:true}));
    doc.addEventListener('load', event => trace.push(['document-load',event.eventPhase,
      event.composedPath().at(-1) === doc]));
    doc.dispatchEvent(new Event('load', {bubbles:true}));
    const image = doc.body.appendChild(doc.createElement('img'));
    image.dispatchEvent(new Event('load', {bubbles:true}));
    return JSON.stringify(trace);
  } finally { popup.close(); sibling.close(); }
})()
"#,
        )
        .expect("popup document event boundaries should be preserved");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([
            ["child-capture", 1, true, true, true],
            ["child-bubble", 3],
            ["document-load", 2, true],
            ["document-load", 3, true],
        ])
    );
}

#[test]
fn popup_dom_event_path_preserves_listener_mutation_cancellation_and_stop_rules() {
    let mut vm = new_storage_test_vm("https://popup-event-listeners.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const popup = open();
  try {
    const button = popup.document.body.appendChild(popup.document.createElement('button'));
    const trace = [];
    const removed = () => trace.push('removed');
    const added = () => trace.push('added');
    popup.addEventListener('click', () => {
      trace.push('capture');
      popup.removeEventListener('click', removed, true);
      popup.addEventListener('click', added);
    }, {capture:true, once:true});
    popup.addEventListener('click', removed, true);
    popup.addEventListener('click', event => {
      event.preventDefault();
      trace.push('passive:' + event.defaultPrevented);
    }, {passive:true});
    popup.onclick = function(event) {
      trace.push('handler:' + (this === popup && event.currentTarget === popup));
      return false;
    };
    button.addEventListener('click', () => trace.push('target'));
    const canceled = [];
    for (let n = 0; n < 2; ++n) {
      canceled.push(!button.dispatchEvent(new Event('click', {bubbles:true, cancelable:true})));
    }
    const stopped = [];
    for (const immediate of [false, true]) {
      const type = 'stop-' + immediate;
      popup.addEventListener(type, event => {
        stopped.push('first:' + immediate);
        if (immediate) event.stopImmediatePropagation(); else event.stopPropagation();
      }, true);
      popup.addEventListener(type, () => stopped.push('second:' + immediate), true);
      button.addEventListener(type, () => stopped.push('target'));
      button.dispatchEvent(new Event(type, {bubbles:true}));
    }
    return JSON.stringify({trace,canceled,stopped});
  } finally { popup.close(); }
})()
"#,
        )
        .expect("popup listener semantics probe should run");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "trace":["capture", "target", "passive:false", "handler:true", "added",
                     "target", "passive:false", "handler:true", "added"],
            "canceled":[true,true],
            "stopped":["first:false", "second:false", "first:true"],
        })
    );
}

#[test]
fn popup_dom_event_paths_retarget_closed_shadows_and_retire_replaced_documents() {
    let mut vm = new_storage_test_vm("https://popup-event-retarget.test/");
    let result = vm.eval(r#"
(() => {
  const popup = open('about:blank', 'popup-event-retarget');
  try {
    const doc = popup.document;
    const host = doc.body.appendChild(doc.createElement('div'));
    const shadow = host.attachShadow({mode:'closed'});
    const button = shadow.appendChild(doc.createElement('button'));
    const trace = [];
    popup.addEventListener('shadow-probe', event => {
      const path = event.composedPath();
      trace.push(['shadow', event.target === host, event.currentTarget === popup,
        path[0] === host, path.at(-1) === popup, !path.includes(button), !path.includes(shadow)]);
    });
    button.dispatchEvent(new Event('shadow-probe', {bubbles:true, composed:true}));
    let openerCalls = 0;
    window.addEventListener('retained-probe', () => ++openerCalls);
    popup.addEventListener('retained-probe', () => trace.push('retired-window'));
    open('about:blank', 'popup-event-retarget');
    const replaced = doc !== popup.document;
    popup.addEventListener('retained-probe', event => trace.push(['current', event.target === popup.document]));
    doc.dispatchEvent(new Event('retained-probe', {bubbles:true}));
    popup.document.dispatchEvent(new Event('retained-probe', {bubbles:true}));
    return JSON.stringify({trace,replaced,openerCalls});
  } finally { popup.close(); }
})()
"#).expect("popup retargeting and replacement probe should run");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "trace":[["shadow",true,true,true,true,true,true],["current",true]],
            "replaced":true,"openerCalls":0,
        })
    );
}

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
