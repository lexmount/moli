use super::*;

#[test]
fn popup_window_content_handlers_share_body_and_frameset_state() {
    let mut vm = new_storage_test_vm("https://popup-window-content-state.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const body = w.document.body;
    const frameset = w.document.createElement('frameset');
    const names = ['onblur','onerror','onfocus','onload','onresize','onscroll',
      'onafterprint','onbeforeprint','onbeforeunload','onhashchange','onlanguagechange',
      'onmessage','onmessageerror','onoffline','ononline','onpagehide','onpagereveal',
      'onpageshow','onpageswap','onpopstate','onrejectionhandled','onstorage',
      'onunhandledrejection','onunload'];
    const failures = [];
    for (const name of names) {
      body.setAttribute(name, 'return 1');
      const first = w[name];
      if (typeof first !== 'function' || first() !== 1 ||
          first !== body[name] || first !== frameset[name]) failures.push(name + ':body');
      frameset.setAttribute(name, 'return 2');
      const second = frameset[name];
      if (typeof second !== 'function' || second() !== 2 || second === first ||
          second !== w[name] || second !== body[name]) failures.push(name + ':frameset');
      w[name] = null;
      if (body[name] !== null || frameset[name] !== null) failures.push(name + ':IDL clear');
      body.setAttribute(name, 'return 3');
      frameset.removeAttribute(name);
      if (w[name] !== null || body[name] !== null) failures.push(name + ':remove');
      if (window[name] !== null) failures.push(name + ':opener');
    }
    return JSON.stringify(failures);
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup content attributes should share Window state");
    assert_eq!(result, "[]");
}

#[test]
fn popup_window_content_handler_syntax_errors_are_lazy_and_retain_listener_order() {
    let mut vm = new_storage_test_vm("https://popup-window-content-errors.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const trace = [];
    const errors = [];
    w.addEventListener('error', event => {
      errors.push(event.error.name);
      event.preventDefault();
    });
    w.addEventListener('resize', () => trace.push('before'));
    w.document.body.setAttribute('onresize', '}');
    w.addEventListener('resize', () => trace.push('after'));
    const before = errors.length;
    const firstNull = w.onresize === null;
    const secondNull = w.document.body.onresize === null;
    w.dispatchEvent(new Event('resize'));
    w.onresize = () => trace.push('replacement');
    w.dispatchEvent(new Event('resize'));
    return JSON.stringify({before, firstNull, secondNull, errors, trace});
  } finally { w.close(); }
})()
"#,
        )
        .expect("failed popup content handlers should retain their registration position");
    assert_eq!(
        result,
        r#"{"before":0,"firstNull":true,"secondNull":true,"errors":["SyntaxError"],"trace":["before","after","before","replacement","after"]}"#
    );
}

#[test]
fn popup_window_content_handler_mutations_preserve_dispatch_snapshots() {
    let mut vm = new_storage_test_vm("https://popup-window-content-mutations.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const body = w.document.body;
    const trace = w.__trace = [];
    let count = 0;
    w.addEventListener('resize', () => {
      trace.push('before:' + ++count);
      if (count === 1) body.setAttribute('onresize', `__trace.push('replacement'); return false;`);
      if (count === 2) {
        body.removeAttribute('onresize');
        body.setAttribute('onresize', `__trace.push('readded');`);
      }
    });
    body.setAttribute('onresize', `__trace.push('stale');`);
    w.addEventListener('resize', () => trace.push('after:' + count));
    for (let i = 0; i < 3; ++i) {
      trace.push('result:' + w.dispatchEvent(new Event('resize', {cancelable:true})));
    }
    return trace.join(',');
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup content mutations should preserve dispatch snapshots");
    assert_eq!(
        result,
        "before:1,replacement,after:1,result:false,before:2,after:2,result:true,before:3,after:3,readded,result:true"
    );
}

#[test]
fn popup_window_content_handler_error_reentry_preserves_replacements() {
    let mut vm = new_storage_test_vm("https://popup-window-content-reentry.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const results = [];
  for (const fromDispatch of [false, true]) {
    const w = open();
    try {
      const trace = w.__trace = [];
      const body = w.document.body;
      w.addEventListener('error', event => {
        trace.push([event.target === w, event.error.name, w.onresize === null]);
        body.setAttribute('onresize', `__trace.push('replacement');`);
        event.preventDefault();
      });
      body.setAttribute('onresize', '}');
      const before = trace.length;
      const first = fromDispatch ? w.dispatchEvent(new Event('resize')) : w.onresize === null;
      const after = trace.length;
      w.dispatchEvent(new Event('resize'));
      results.push({before, first, after, trace});
    } finally { w.close(); }
  }
  return JSON.stringify(results);
})()
"#,
        )
        .expect("lazy compilation should preserve reentrant content replacements");
    let expected = serde_json::json!({
        "before":0, "first":true, "after":1,
        "trace":[[true,"SyntaxError",true],"replacement"]
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected])
    );
}

#[test]
fn popup_window_content_handlers_use_only_the_window_scope_and_error_arguments() {
    let mut vm = new_storage_test_vm("https://popup-window-content-scope.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const trace = w.__trace = [];
    w.scopeToken = 'window';
    w.document.scopeToken = 'document';
    for (const tag of ['body', 'frameset']) {
      const element = w.document.createElement(tag);
      element.scopeToken = 'element';
      element.setAttribute('onresize', `__trace.push([
        this === window, globalThis === window, document === window.document,
        scopeToken, arguments.length, event.type]); return false;`);
      trace.push(w.dispatchEvent(new Event('resize', {cancelable:true})));
      element.setAttribute('onerror', `__trace.push([
        this === window, arguments.length, event, source, lineno, colno, error.message]);
        return true;`);
      trace.push(w.dispatchEvent(new ErrorEvent('error', {message:'message',
        filename:'source', lineno:3, colno:4, error:new Error('value'), cancelable:true})));
    }
    return JSON.stringify(trace);
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup body handlers should have Window-only scope and special onerror arguments");
    let pair = serde_json::json!([
        [true, true, true, "window", 1, "resize"],
        false,
        [true, 5, "message", "source", 3, 4, "value"],
        false
    ]);
    let mut expected = pair.as_array().unwrap().clone();
    expected.extend(expected.clone());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!(expected)
    );
}

#[test]
fn popup_window_content_handlers_keep_their_window_after_source_adoption() {
    let mut vm = new_storage_test_vm("https://popup-window-content-adoption.test/");
    let result = vm.eval(r#"
(() => {
  const w = open();
  try {
    const body = w.document.createElement('body');
    const trace = w.__trace = [];
    w.scopeToken = 'popup';
    window.scopeToken = 'opener';
    body.setAttribute('onresize', `__trace.push([scopeToken,this === window,globalThis === window]);`);
    document.adoptNode(body);
    body.setAttribute('onresize', 'window.__adoptMutation = true;');
    const compiled = typeof w.onresize;
    w.dispatchEvent(new Event('resize'));
    return JSON.stringify({compiled, trace, adopted:body.ownerDocument === document,
      openerUntouched:!window.__adoptMutation && typeof window.onresize === 'function' });
  } finally { w.close(); }
})()
"#).expect("adopting the source body must not move the previously registered Window handler");
    assert_eq!(
        result,
        r#"{"compiled":"function","trace":[["popup",true,true]],"adopted":true,"openerUntouched":true}"#
    );
}

#[test]
fn popup_window_content_handlers_ignore_expandos_and_retire_on_navigation() {
    let mut vm = new_storage_test_vm("https://popup-window-content-retirement.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open('about:blank', 'handler-retirement');
  try {
    const body = w.document.body;
    const trace = w.__trace = [];
    let traps = 0;
    Object.defineProperty(w, 'onresize', {
      configurable:true, get() { ++traps; }, set() { ++traps; }
    });
    body.getAttribute = () => { ++traps; throw new Error('author getAttribute'); };
    body.setAttribute('onresize', `__trace.push('first');`);
    w.dispatchEvent(new Event('resize'));
    body.setAttribute('onresize', `__trace.push('stale');`);
    open('about:blank', 'handler-retirement');
    w.dispatchEvent(new Event('resize'));
    return JSON.stringify({trace, traps, cleared:body.onresize === null});
  } finally { w.close(); }
})()
"#,
        )
        .expect(
            "Document retirement should clear uncompiled handlers without author property access",
        );
    assert_eq!(result, r#"{"trace":["first"],"traps":0,"cleared":true}"#);
}

#[tokio::test]
async fn popup_parser_window_content_handlers_preserve_order_and_idl_clearing() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-parser-window-content.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(
        r#"
(() => {
  window.__popupInlineDone = 0;
  window.__popupInlineResults = [];
  window.__popupInlineWindows = [];
  for (const cleared of [false, true]) {
    const markup = `<!doctype html><head><script>
      window.__trace = [];
      window.addEventListener('load', () => window.__trace.push('head'));
      window.addEventListener('pageshow', () => {
        opener.__popupInlineResults.push(window.__trace);
        ++opener.__popupInlineDone;
      });
    </script></head><body onload="__trace.push('body:' + (this === window) + ':' +
        (event.target === document) + ':' + event.isTrusted)">
    <script>
      if (${cleared}) window.onload = null;
      window.addEventListener('load', () => window.__trace.push('tail'));
    </script>`;
    const url = URL.createObjectURL(new Blob([markup], {type:'text/html'}));
    __popupInlineWindows.push(open(url));
  }
})()
"#,
    )
    .expect("popup parser handler fixtures should open");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupInlineDone)",
        "2",
        "popup parser inline handlers",
    )
    .await;
    let result = vm
        .eval("JSON.stringify(__popupInlineResults.sort((a,b) => b.length - a.length))")
        .expect("popup load traces should be available");
    assert_eq!(
        result,
        r#"[["head","body:true:true:true","tail"],["head","tail"]]"#
    );
    vm.eval("__popupInlineWindows.forEach(w => w.close())")
        .expect("close popup fixtures");
}

#[test]
fn popup_window_content_handlers_obey_the_popup_csp_without_blocking_idl_handlers() {
    let mut vm = new_storage_test_vm("https://popup-window-content-csp.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open();
  try {
    const trace = w.__trace = [];
    const meta = w.document.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = "script-src-attr 'none'";
    w.document.head.appendChild(meta);
    w.addEventListener('resize', () => trace.push('before'));
    w.document.body.setAttribute('onresize', `__trace.push('blocked');`);
    w.addEventListener('resize', () => trace.push('after'));
    const blocked = w.onresize === null;
    w.dispatchEvent(new Event('resize'));
    w.onresize = () => trace.push('IDL');
    w.dispatchEvent(new Event('resize'));
    const body = document.createElement('body');
    body.setAttribute('onresize', 'return 7');
    const openerAllowed = typeof window.onresize === 'function' && window.onresize() === 7;
    window.onresize = null;
    return JSON.stringify({blocked, openerAllowed, trace});
  } finally { w.close(); }
})()
"#,
        )
        .expect("popup content handler CSP should be document scoped");
    assert_eq!(
        result,
        r#"{"blocked":true,"openerAllowed":true,"trace":["before","after","before","IDL","after"]}"#
    );
}
