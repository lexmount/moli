use super::*;

#[test]
fn child_body_and_frameset_content_handlers_use_window_arguments_and_realm() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
  const d = w.document;
  const results = [];
  for (const tag of ['body', 'frameset']) {
    for (const mode of ['attribute', 'outerHTML']) {
      w.onerror = null;
      w.__calls = [];
      const source = `__calls.push([arguments.length, arguments.callee.length,
        this === window, arguments[0] === 'message', source, lineno, colno, error]);
        return true;`;
      if (mode === 'outerHTML') {
        d.body.outerHTML = '<' + tag + ' onerror="' + source + '"></' + tag + '>';
      } else {
        d.documentElement.replaceChild(d.createElement(tag), d.body);
        d.body.setAttribute('onerror', source);
      }
      const error = new w.ErrorEvent('error', {cancelable:true, message:'message',
        filename:'source', lineno:3, colno:4, error:'reason'});
      const errorResult = w.dispatchEvent(error);
      const ordinaryResult = w.dispatchEvent(new w.Event('error', {cancelable:true}));
      const handler = w.onerror;
      results.push([tag, mode, w.__calls, errorResult, ordinaryResult,
        handler === d.body.onerror, handler instanceof w.Function,
        handler instanceof Function, window.onerror === null]);
    }
  }
  return JSON.stringify(results);
})()
"#,
        )
        .expect("child content handlers should dispatch in their Window realm");
    let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
    for row in rows.as_array().unwrap() {
        assert_eq!(
            &row.as_array().unwrap()[2..],
            serde_json::json!([
                [
                    [5, 5, true, true, "source", 3, 4, "reason"],
                    [1, 5, true, false, null, null, null, null]
                ],
                false,
                true,
                true,
                true,
                false,
                true
            ])
            .as_array()
            .unwrap(),
            "{row}"
        );
    }
    assert_eq!(rows.as_array().unwrap().len(), 4);
}

#[test]
fn child_body_and_frameset_content_handlers_share_window_state() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler-state.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
  const body = w.document.createElement('body');
  const frameset = w.document.createElement('frameset');
  const names = ['onblur', 'onerror', 'onfocus', 'onload', 'onresize', 'onscroll',
    'onafterprint', 'onbeforeprint', 'onbeforeunload', 'onhashchange',
    'onlanguagechange', 'onmessage', 'onmessageerror', 'onoffline', 'ononline',
    'onpagehide', 'onpagereveal', 'onpageshow', 'onpageswap', 'onpopstate',
    'onrejectionhandled', 'onstorage', 'onunhandledrejection', 'onunload'];
  const failures = [];
  for (const name of names) {
    body.setAttribute(name, 'return 1');
    const first = w[name];
    frameset.setAttribute(name, 'return 2');
    const second = body[name];
    if (!(first instanceof w.Function) || first() !== 1 ||
        !(second instanceof w.Function) || second() !== 2 ||
        second === first || second !== w[name] || second !== frameset[name]) {
      failures.push(name + ':replace');
    }
    frameset.removeAttribute(name);
    if (w[name] !== null || body[name] !== null) failures.push(name + ':remove');
    body.setAttribute(name, 'return 3');
    w[name] = null;
    if (body[name] !== null || w[name] !== null || !body.hasAttribute(name)) {
      failures.push(name + ':clear');
    }
    body.setAttribute(name, '');
    if (!(w[name] instanceof w.Function) || w[name]() !== undefined) {
      failures.push(name + ':empty');
    }
    body.removeAttribute(name);
  }
  return JSON.stringify(failures);
})()
"#,
        )
        .expect("child Window reflected content attributes should share state");
    assert_eq!(result, "[]");
}

#[test]
fn child_content_handler_compile_errors_are_lazy_and_preserve_listener_order() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler-errors.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
  const body = w.document.body;
  const trace = [];
  const errors = [];
  w.addEventListener('error', event => {
    errors.push([event.error instanceof w.SyntaxError, event.error instanceof SyntaxError]);
    event.preventDefault();
  });
  w.addEventListener('resize', () => trace.push('before'));
  body.setAttribute('onresize', '}');
  w.addEventListener('resize', () => trace.push('after'));
  const beforeRead = errors.length;
  const firstNull = w.onresize === null;
  const secondNull = body.onresize === null;
  w.dispatchEvent(new w.Event('resize'));
  w.onresize = () => trace.push('replacement');
  w.dispatchEvent(new w.Event('resize'));
  return JSON.stringify({beforeRead, firstNull, secondNull, errors, trace});
})()
"#,
        )
        .expect("child content handler syntax errors should preserve a null listener slot");
    assert_eq!(
        result,
        r#"{"beforeRead":0,"firstNull":true,"secondNull":true,"errors":[[true,false]],"trace":["before","after","before","replacement","after"]}"#
    );
}

#[test]
fn child_content_handler_mutations_preserve_dispatch_snapshots() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler-mutation.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
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
  for (let i = 0; i < 3; i++) {
    trace.push('result:' + w.dispatchEvent(new w.Event('resize', {cancelable:true})));
  }
  return trace.join(',');
})()
"#,
        )
        .expect("content handler replacement and reactivation should follow dispatch snapshots");
    assert_eq!(
        result,
        "before:1,replacement,after:1,result:false,before:2,after:2,result:true,before:3,after:3,readded,result:true"
    );
}

#[test]
fn child_content_handler_error_reporting_preserves_reentrant_replacements() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler-reentry.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
  const trace = [];
  const replacement = () => trace.push('replacement');
  w.addEventListener('error', event => {
    trace.push(w.onresize === null ? 'null-during-report' : 'unexpected');
    w.onresize = replacement;
    event.preventDefault();
  });
  w.document.body.setAttribute('onresize', '}');
  const firstNull = w.onresize === null;
  const retained = w.onresize === replacement;
  w.dispatchEvent(new w.Event('resize'));
  return JSON.stringify({firstNull, retained, trace});
})()
"#,
        )
        .expect("error reporting must not overwrite an author replacement");
    assert_eq!(
        result,
        r#"{"firstNull":true,"retained":true,"trace":["null-during-report","replacement"]}"#
    );
}

#[tokio::test]
async fn child_parser_content_handlers_use_registry_and_respect_idl_clearing() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://child-parser-content-handler.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(
        r#"
(() => {
  globalThis.__trace = [];
  for (const cleared of [false, true]) {
    const frame = document.createElement('iframe');
    frame.id = cleared ? 'cleared' : 'active';
    frame.srcdoc = `<!doctype html><body
      onload="parent.__trace.push('load:' + frameElement.id)"
      onstorage="parent.__trace.push('storage:' + frameElement.id + ':' + (this === window))">
      <script>
        if (frameElement.id === 'cleared') onload = onstorage = null;
        document.body.getAttribute = () => { throw new Error('author getAttribute'); };
      </script>`;
    document.body.appendChild(frame);
  }
})()
"#,
    )
    .expect("child parser handler setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    let result = vm
        .eval(
            r#"
(() => {
  for (const id of ['active', 'cleared']) {
    const w = document.getElementById(id).contentWindow;
    w.dispatchEvent(new w.StorageEvent('storage'));
    if (id === 'cleared') w.dispatchEvent(new w.Event('load'));
  }
  return JSON.stringify(__trace);
})()
"#,
        )
        .expect("parser-installed handlers should use the child Window registry");
    assert_eq!(result, r#"["load:active","storage:active:true"]"#);
}

#[test]
fn child_document_open_clears_uncompiled_window_content_handlers() {
    let mut vm = new_parsed_test_vm(
        "https://child-content-handler-document-open.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const w = document.getElementById('child').contentWindow;
  w.__trace = [];
  w.document.body.setAttribute('onresize', `__trace.push('stale');`);
  w.document.open();
  w.document.write('<!doctype html><body onresize="__trace.push(\'new\');">');
  w.document.close();
  w.dispatchEvent(new w.Event('resize'));
  const handler = w.onresize;
  return JSON.stringify([w.__trace, typeof handler, handler === w.document.body.onresize]);
})()
"#,
        )
        .expect("document replacement should retire uncompiled handlers");
    assert_eq!(result, r#"[["new"],"function",true]"#);
}

#[tokio::test]
async fn child_parser_script_body_replacement_installs_window_handlers_immediately() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://child-parser-handler-replacement.test/",
        "<!doctype html><body></body>",
        &loader,
    );
    vm.eval(r#"
(() => {
  globalThis.__parserCalls = {body: [], frameset: []};
  for (const tag of ['body', 'frameset']) {
    const frame = document.createElement('iframe');
    const source = "parent.__parserCalls['" + tag + "'].push([arguments.length, arguments.callee.length, this === window]);";
    const markup = '<' + tag + ' onerror="' + source + '"></' + tag + '>';
    const script = 'document.body.outerHTML = ' + JSON.stringify(markup) + ';' +
      'window.dispatchEvent(new ErrorEvent("error", {message:"message"}));' +
      'window.dispatchEvent(new Event("error"));';
    frame.srcdoc = '<!doctype html><body></body><script>' + script + '</script>';
    document.body.appendChild(frame);
  }
})()
"#).expect("child parser replacement setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("JSON.stringify(__parserCalls)")
            .expect("child parser handler results"),
        r#"{"body":[[5,5,true],[1,5,true]],"frameset":[[5,5,true],[1,5,true]]}"#,
    );
}
