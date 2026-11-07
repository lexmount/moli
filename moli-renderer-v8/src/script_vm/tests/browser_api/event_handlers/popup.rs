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
