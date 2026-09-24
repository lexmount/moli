use super::*;

#[test]
fn main_and_child_parser_mutation_notifications_preserve_order_and_identity() {
    let mut vm = new_storage_test_vm("https://parser-mutation-parity.test/");
    let result = vm.eval(r#"
      (() => {
        function exercise(w) {
          const d = w.document;
          d.open(); d.write('<!doctype html><body>');
          const log = [], records = [];
          const collect = entries => records.push(...entries.map(r => [
            r.target.nodeName, Array.from(r.addedNodes, n => n.nodeName), r.removedNodes.length
          ]));
          const observer = new w.MutationObserver(collect);
          observer.observe(d.body, {childList:true, subtree:true});
          class Probe extends w.HTMLElement {
            static get observedAttributes() { return ['title']; }
            constructor() { super(); log.push('construct'); }
            attributeChangedCallback(name, oldValue, value) { log.push(name + ':' + value); }
            connectedCallback() { log.push('connected:' + this.childNodes.length); }
          }
          let internals;
          class Face extends w.HTMLElement {
            static formAssociated = true;
            constructor() { super(); internals = this.attachInternals(); }
            connectedCallback() { log.push('face:' + (internals.form?.id || 'null')); }
            formAssociatedCallback(form) { log.push('form:' + (form?.id || 'null')); }
          }
          w.customElements.define('x-mutation-probe', Probe);
          w.customElements.define('x-mutation-face', Face);
          d.write('<x-mutation-probe title=parsed>text</x-mutation-probe>' +
            '<x-mutation-face form=owner></x-mutation-face><form id=owner></form><iframe id=nested></iframe>');
          collect(observer.takeRecords());
          observer.disconnect();
          const nested = d.getElementById('nested');
          const result = {log, records, form:internals.form === d.getElementById('owner'),
            childIdentity:nested.contentDocument === nested.contentWindow.document};
          d.close();
          return result;
        }
        const main = exercise(window);
        const frame = document.createElement('iframe');
        document.body.appendChild(frame);
        const child = exercise(frame.contentWindow);
        frame.remove();
        return JSON.stringify([main, child]);
      })()
    "#).unwrap();
    let expected = serde_json::json!({
        "log": ["construct", "title:parsed", "connected:0", "face:null", "form:owner"],
        "records": [
            ["BODY", ["X-MUTATION-PROBE"], 0],
            ["X-MUTATION-PROBE", ["#text"], 0],
            ["BODY", ["X-MUTATION-FACE"], 0],
            ["BODY", ["FORM"], 0],
            ["BODY", ["IFRAME"], 0]
        ],
        "form": true,
        "childIdentity": true
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn main_and_child_parser_styles_are_current_in_reactions_and_preserve_cssom_edits() {
    let mut vm = new_storage_test_vm("https://parser-style-mutation-parity.test/");
    let result = vm.eval(r#"
      (() => {
        function exercise(w) {
          const d = w.document;
          d.open(); d.write('<!doctype html><body><style id=first>.first { color: red; }</style>');
          const first = d.getElementById('first').sheet;
          first.insertRule('.kept { color: green; }', first.cssRules.length);
          const log = [];
          class Probe extends w.HTMLElement {
            connectedCallback() {
              log.push({color:w.getComputedStyle(this).color,
                selectors:Array.from(d.getElementById('second').sheet.cssRules, r => r.selectorText)});
            }
          }
          w.customElements.define('x-style-probe', Probe);
          d.write('<style id=second>x-style-probe { color: rgb(1, 2, 3); }</style><x-style-probe></x-style-probe>');
          d.write('<div>unrelated insertion</div>');
          const result = {log, identity:first === d.getElementById('first').sheet,
            firstSelectors:Array.from(first.cssRules, r => r.selectorText)};
          d.close();
          return result;
        }
        const main = exercise(window);
        const frame = document.createElement('iframe');
        document.body.appendChild(frame);
        const child = exercise(frame.contentWindow);
        frame.remove();
        return JSON.stringify([main, child]);
      })()
    "#).unwrap();
    let expected = serde_json::json!({
        "log": [{"color": "rgb(1, 2, 3)", "selectors": ["x-style-probe"]}],
        "identity": true,
        "firstSelectors": [".first", ".kept"]
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!([expected, expected]),
    );
}

#[test]
fn child_parser_write_constructs_elements_and_runs_reactions_synchronously() {
    let mut vm = new_storage_test_vm("https://child-parser-custom-elements.test/");
    let result = vm.eval(r#"
      (() => {
        const results = [];
        for (const method of ['write', 'writeln']) {
          for (const explicitOpen of [false, true]) {
            const frame = document.createElement('iframe');
            (document.body || document.documentElement || document).appendChild(frame);
            const w = frame.contentWindow, d = w.document;
            if (explicitOpen) d.open();
            const log = [];
            class Written extends w.HTMLElement {
              constructor() { super(); log.push('construct:' + (this.ownerDocument === d)); }
              static get observedAttributes() { return ['title']; }
              attributeChangedCallback(name, oldValue, value) { log.push(name + ':' + oldValue + ':' + value); }
              connectedCallback() { log.push('connected'); }
            }
            class WrittenButton extends w.HTMLButtonElement {
              constructor() { super(); log.push('button'); }
              connectedCallback() { log.push('button-connected'); }
            }
            w.customElements.define('child-written', Written);
            w.customElements.define('child-button', WrittenButton, {extends:'button'});
            d[method]('<!doctype html><body><child-written title="parsed"></child-written><button is="child-button"></button>');
            results.push({
              custom:d.querySelector('child-written') instanceof Written,
              button:d.querySelector('button') instanceof WrittenButton,
              log:log.slice()
            });
            d.close();
            frame.remove();
          }
        }
        return JSON.stringify(results);
      })()
    "#).expect("child parser write and writeln should construct custom elements");
    let expected = serde_json::json!({
        "custom": true,
        "button": true,
        "log": ["construct:true", "title:null:parsed", "connected", "button", "button-connected"]
    });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::Value::Array(vec![expected; 4])
    );
}

#[test]
fn child_parser_write_invokes_connected_before_children_and_next_constructor() {
    let mut vm = new_storage_test_vm("https://child-parser-insertion-reactions.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = [];
        class A extends w.HTMLElement {
          constructor() { super(); log.push('ctor:a'); }
          connectedCallback() { log.push('connected:a:' + this.childNodes.length); }
        }
        class B extends w.HTMLElement {
          constructor() { super(); log.push('ctor:b'); }
          connectedCallback() { log.push('connected:b:' + this.childNodes.length); }
        }
        w.customElements.define('x-a', A);
        w.customElements.define('x-b', B);
        d.write('<x-a>hello <b>world</b></x-a><x-b></x-b>');
        const result = {log, text:d.querySelector('x-a').textContent};
        d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["ctor:a", "connected:a:0", "ctor:b", "connected:b:0"],
            "text": "hello world"
        })
    );
}

#[test]
fn child_parser_write_associates_existing_face_when_form_is_inserted() {
    let mut vm = new_storage_test_vm("https://child-parser-form-reactions.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = [];
        let internals;
        class Face extends w.HTMLElement {
          static formAssociated = true;
          constructor() { super(); internals = this.attachInternals(); log.push('ctor'); }
          connectedCallback() { log.push('connected:' + (internals.form?.id || 'null')); }
          formAssociatedCallback(form) { log.push('form:' + (form?.id || 'null')); }
        }
        class Tail extends w.HTMLElement {
          constructor() { super(); log.push('tail'); }
        }
        w.customElements.define('x-face', Face);
        w.customElements.define('x-tail', Tail);
        d.write('<x-face form=owner></x-face><form id=owner></form><x-tail></x-tail>');
        const result = {log:log.slice(), owner:internals.form?.id};
        d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["ctor", "connected:null", "form:owner", "tail"],
            "owner": "owner"
        })
    );
}

#[test]
fn child_parser_connected_callback_can_write_at_current_insertion_point() {
    let mut vm = new_storage_test_vm("https://child-parser-connected-write.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = [];
        class A extends w.HTMLElement {
          connectedCallback() {
            log.push('connected:' + this.childNodes.length);
            d.write('<i id=nested>nested</i>');
            log.push('after-nested-write:' + !!d.getElementById('nested'));
          }
        }
        w.customElements.define('x-a', A);
        d.write('<x-a><b id=original>original</b></x-a>');
        const a = d.querySelector('x-a'), nested = d.getElementById('nested');
        const result = {log, nestedParent:nested?.parentElement.localName,
          children:Array.from(a.children, node => node.id)};
        d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["connected:0", "after-nested-write:true"],
            "nestedParent": "body", "children": ["original"]
        })
    );
}

#[test]
fn child_parser_connected_write_preserves_script_and_parent_input_order() {
    let mut vm = new_storage_test_vm("https://child-parser-connected-script.test/");
    let result = vm.eval(r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = w.log = [];
        class A extends w.HTMLElement {
          connectedCallback() {
            log.push('connected');
            d.write('<i id=first></i><script>log.push("script");document.write("<u id=inner></u>")</script><i id=last></i>');
            log.push('returned:' + !!d.getElementById('last'));
          }
        }
        w.customElements.define('x-a', A);
        d.write('<x-a><b id=original></b></x-a>');
        const result = {log, body:Array.from(d.body.children, n => n.localName + ':' + n.id),
          children:Array.from(d.querySelector('x-a').children, n => n.id)};
        d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["connected", "script", "returned:true"],
            "body": ["x-a:", "i:first", "script:", "u:inner", "i:last"],
            "children": ["original"]
        })
    );
}

#[tokio::test]
async fn child_parser_connected_write_resumes_after_external_script_load() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "external script request must complete");
            request.extend_from_slice(&buffer[..read]);
        }
        assert!(request.starts_with(b"GET /blocked.js "));
        released.await.unwrap();
        let body = "externalRuns++; trace.push('external');";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader(&format!("http://{address}/"), &loader);
    vm.eval(
        r#"
      globalThis.childParserLoaded = false;
      globalThis.childParserFrame = document.createElement('iframe');
      (document.body || document.documentElement || document).appendChild(childParserFrame);
      const w = childParserFrame.contentWindow, d = w.document;
      d.open(); d.write('<!doctype html><body>');
      w.externalRuns = 0; w.tailConstructions = 0; w.outerScriptRuns = 0; w.trace = [];
      class A extends w.HTMLElement {
        connectedCallback() {
          d.write('<script src="/blocked.js"></script><i id=inner></i>');
          d.write('<p id=queued>queued while loading</p>');
        }
      }
      class B extends w.HTMLElement { constructor() { super(); w.tailConstructions++; } }
      w.customElements.define('x-a', A);
      w.customElements.define('x-b', B);
      childParserFrame.onload = () => { childParserLoaded = true; };
      d.write('<x-a></x-a><script>outerScriptRuns++;trace.push("outer")</script><x-b></x-b>');
      d.close();
      'queued';
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval(r#"JSON.stringify({scripts:w.outerScriptRuns, inner:!!d.getElementById('inner'), queued:!!d.getElementById('queued')})"#).unwrap(),
        r#"{"scripts":1,"inner":false,"queued":false}"#,
        "every active parser feed must stop behind the unreleased external script");
    release.send(()).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        "String(childParserLoaded)",
        "true",
        "nested parser external script",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({runs:w.externalRuns, outerScripts:w.outerScriptRuns, constructors:w.tailConstructions,
      innerParent:d.getElementById('inner').parentElement.localName,
      outer:!!d.querySelector('x-b'), queued:d.getElementById('queued').textContent, trace:w.trace})"#
        )
        .unwrap(),
        r#"{"runs":1,"outerScripts":1,"constructors":1,"innerParent":"body","outer":true,"queued":"queued while loading","trace":["outer","external"]}"#
    );
    server.await.unwrap();
}

#[test]
fn child_parser_inline_close_waits_for_script_to_return() {
    let mut vm = new_storage_test_vm("https://child-parser-inline-close.test/");
    let result = vm.eval(r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        w.log = [];
        class Tail extends w.HTMLElement {
          constructor() { super(); w.log.push('tail'); }
        }
        w.customElements.define('x-tail', Tail);
        d.write('<script>document.close();log.push(document.readyState+":"+!!document.querySelector("x-tail"))</script><x-tail></x-tail>');
        return JSON.stringify({log:w.log, state:d.readyState, tail:!!d.querySelector('x-tail')});
      })()
    "#).unwrap();
    assert_eq!(
        result,
        r#"{"log":["loading:false","tail"],"state":"complete","tail":true}"#
    );
}

#[tokio::test]
async fn child_parser_nested_close_dispatches_readiness_once() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://child-parser-close-readiness.test/",
        &loader,
    );
    vm.eval(
        r#"
      globalThis.childParserLoaded = false;
      const frame = document.createElement('iframe');
      (document.body || document.documentElement || document).appendChild(frame);
      const w = frame.contentWindow, d = w.document;
      d.open();
      globalThis.readiness = [d.readyState];
      d.addEventListener('readystatechange', () => readiness.push(d.readyState));
      class Close extends w.HTMLElement { connectedCallback() { d.close(); } }
      w.customElements.define('x-close', Close);
      frame.onload = () => { childParserLoaded = true; };
      d.write('<!doctype html><body><x-close></x-close><p>tail</p>');
      JSON.stringify(readiness);
    "#,
    )
    .map(|result| assert_eq!(result, r#"["loading","interactive","complete"]"#))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        "String(childParserLoaded)",
        "true",
        "closed parser load",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(readiness)").unwrap(),
        r#"["loading","interactive","complete"]"#
    );
}

#[test]
fn child_parser_connected_close_finishes_after_outer_insertion() {
    let mut vm = new_storage_test_vm("https://child-parser-connected-close.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = [];
        class A extends w.HTMLElement {
          connectedCallback() { log.push('connected'); d.close(); log.push('after-close'); }
        }
        class B extends w.HTMLElement { constructor() { super(); log.push('tail'); } }
        w.customElements.define('x-a', A);
        w.customElements.define('x-b', B);
        d.write('<x-a></x-a><x-b></x-b>');
        return JSON.stringify({log, tail:!!d.querySelector('x-b'), state:d.readyState});
      })()
    "#,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["connected", "after-close", "tail"], "tail": true, "state": "complete"
        })
    );
}

#[test]
fn child_parser_connected_callback_can_replace_document_before_next_constructor() {
    let mut vm = new_storage_test_vm("https://child-parser-connected-open.test/");
    let result = vm.eval(r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const log = [];
        class A extends w.HTMLElement {
          connectedCallback() {
            log.push('connected');
            d.open(); d.write('<!doctype html><body><p id=replaced>replacement</p>'); d.close();
          }
        }
        class Tail extends w.HTMLElement { constructor() { super(); log.push('tail'); } }
        w.customElements.define('x-a', A);
        w.customElements.define('x-tail', Tail);
        d.write('<x-a></x-a><x-tail></x-tail>');
        const result = {log, replaced:!!d.getElementById('replaced'), tail:!!d.querySelector('x-tail')};
        d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "log": ["connected"], "replaced": true, "tail": false
        })
    );
}

#[test]
fn child_parser_write_preserves_outer_javascript_microtask_boundary() {
    let mut vm = new_storage_test_vm("https://child-parser-microtasks.test/");
    let result = vm
        .eval(
            r#"
      globalThis.childParserLog = [];
      const frame = document.createElement('iframe');
      (document.body || document.documentElement || document).appendChild(frame);
      const w = frame.contentWindow, d = w.document;
      class Written extends w.HTMLElement {
        constructor() {
          super(); childParserLog.push('constructor');
          Promise.resolve().then(() => childParserLog.push('constructor-microtask'));
        }
        connectedCallback() {
          childParserLog.push('connected');
          Promise.resolve().then(() => childParserLog.push('connected-microtask'));
        }
      }
      w.customElements.define('child-written', Written);
      Promise.resolve().then(() => childParserLog.push('earlier-microtask'));
      d.write('<!doctype html><body><child-written></child-written>');
      childParserLog.push('after-write');
      d.close();
      JSON.stringify(childParserLog);
    "#,
        )
        .expect("child document.write should keep outer JavaScript on the stack");
    assert_eq!(result, r#"["constructor","connected","after-write"]"#);
    assert_eq!(
        vm.eval("JSON.stringify(childParserLog)").unwrap(),
        r#"["constructor","connected","after-write","earlier-microtask","constructor-microtask","connected-microtask"]"#
    );
}

#[tokio::test]
async fn child_parser_srcdoc_invokes_insertion_reactions_before_following_tokens() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://child-parser-srcdoc-reactions.test/",
        &loader,
    );
    let html = r#"<!doctype html><body><script>
      window.log = [];
      class Face extends HTMLElement {
        static formAssociated = true;
        constructor() { super(); this.internals = this.attachInternals(); log.push('ctor:a'); }
        connectedCallback() { log.push('connected:a:' + this.childNodes.length); }
        formAssociatedCallback(form) { log.push('form:' + form.id); }
      }
      class B extends HTMLElement {
        constructor() { super(); log.push('ctor:b'); }
        connectedCallback() { log.push('connected:b:' + this.childNodes.length); }
      }
      customElements.define('x-face', Face);
      customElements.define('x-b', B);
    </script><x-face form=owner>hello <b>world</b></x-face><form id=owner></form><x-b></x-b>"#;
    vm.eval(&format!(
        r#"
      globalThis.childParserLoaded = false;
      globalThis.childParserFrame = document.createElement('iframe');
      childParserFrame.onload = () => {{ childParserLoaded = true; }};
      childParserFrame.srcdoc = {html:?};
      (document.body || document.documentElement || document).appendChild(childParserFrame);
      'queued';
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        "String(childParserLoaded)",
        "true",
        "child srcdoc reaction order",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(childParserFrame.contentWindow.log)")
            .unwrap(),
        r#"["ctor:a","connected:a:0","form:owner","ctor:b","connected:b:0"]"#
    );
}

#[test]
fn child_parser_write_queues_mutation_records_for_parser_insertions() {
    let mut vm = new_storage_test_vm("https://child-parser-observers.test/");
    let result = vm
        .eval(
            r#"
      (() => {
        const frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        const w = frame.contentWindow, d = w.document;
        d.open(); d.write('<!doctype html><body>');
        const observer = new w.MutationObserver(() => {});
        observer.observe(d.body, {childList:true, subtree:true});
        d.write('<b><i>hello</i></b>');
        const result = observer.takeRecords().map(r => [
          r.type, r.target.nodeName,
          Array.from(r.addedNodes, n => n.nodeName), r.removedNodes.length
        ]);
        observer.disconnect(); d.close(); frame.remove();
        return JSON.stringify(result);
      })()
    "#,
        )
        .expect("child parser mutations should be observable before write returns");
    assert_eq!(
        result,
        r##"[["childList","BODY",["B"],0],["childList","B",["I"],0],["childList","I",["#text"],0]]"##
    );
}

#[test]
fn child_parser_guards_dynamic_markup_during_construction_and_attribute_reactions() {
    let mut vm = new_storage_test_vm("https://child-parser-dynamic-markup.test/");
    let result = vm.eval(r#"
      (() => {
        const frame = document.createElement('iframe');
        const other = document.createElement('iframe');
        const root = document.body || document.documentElement || document;
        root.appendChild(frame); root.appendChild(other);
        const w = frame.contentWindow, d = w.document;
        const log = [];
        const probe = phase => {
          for (const method of ['open', 'close', 'write', 'writeln']) {
            try { d[method](''); log.push(phase + ':' + method + ':allowed'); }
            catch (e) { log.push(phase + ':' + method + ':' + e.name + ':' + (e instanceof w.DOMException)); }
          }
          other.contentDocument.write('<p>' + phase + '</p>');
        };
        class Written extends w.HTMLElement {
          constructor() { super(); probe('construct'); }
          static get observedAttributes() { return ['title']; }
          attributeChangedCallback() { probe('attribute'); }
        }
        w.customElements.define('child-written', Written);
        d.write('<!doctype html><body><child-written title="parsed"></child-written>');
        d.write('<p>after</p>');
        d.close(); other.contentDocument.close();
        return JSON.stringify({log, after:d.querySelector('p').textContent,
          other:Array.from(other.contentDocument.querySelectorAll('p'), p => p.textContent)});
      })()
    "#).expect("dynamic markup guards should cover construction and token attribute reactions");
    let mut log = Vec::new();
    for phase in ["construct", "attribute"] {
        for method in ["open", "close", "write", "writeln"] {
            log.push(format!("{phase}:{method}:InvalidStateError:true"));
        }
    }
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({"log": log, "after":"after", "other":["construct", "attribute"]})
    );
}

#[tokio::test]
async fn child_parser_srcdoc_checkpoints_mutations_before_custom_element_construction() {
    for (markup, before) in [
        (
            "<b><child-parsed></child-parsed></b>",
            serde_json::json!([["BODY", "B"]]),
        ),
        (
            "<b><i>hello</b><child-parsed></child-parsed>",
            serde_json::json!([["BODY", "B"], ["B", "I"], ["I", "#text"], ["BODY", "I"]]),
        ),
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_page_task_executor_test_vm_with_loader(
            "https://child-parser-checkpoint.test/",
            &loader,
        );
        let html = format!(
            r#"<!doctype html><body><script>
          window.batches = [];
          const describe = records => records.map(r => [r.target.nodeName,
            Array.from(r.addedNodes, n => n.nodeName).join(',')]);
          class Parsed extends HTMLElement {{
            constructor() {{ super(); window.beforeConstructor = batches.slice(); }}
          }}
          customElements.define('child-parsed', Parsed);
          new MutationObserver(records => batches.push(describe(records)))
            .observe(document.body, {{childList:true, subtree:true}});
        </script>{markup}</body>"#
        );
        vm.eval(&format!(
            r#"
          globalThis.childParserLoaded = false;
          globalThis.childParserFrame = document.createElement('iframe');
          childParserFrame.onload = () => {{ childParserLoaded = true; }};
          childParserFrame.srcdoc = {html:?};
          (document.body || document.documentElement || document).appendChild(childParserFrame);
          'queued';
        "#
        ))
        .expect("srcdoc parser probe should queue");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            "String(childParserLoaded)",
            "true",
            "child parser load",
        )
        .await;
        let result = vm
            .eval(
                r#"JSON.stringify({before:childParserFrame.contentWindow.beforeConstructor,
          after:childParserFrame.contentWindow.batches})"#,
            )
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            result["before"],
            serde_json::json!([before.clone()]),
            "markup: {markup}"
        );
        let parent = if markup.contains("<i>") { "I" } else { "B" };
        assert_eq!(
            result["after"],
            serde_json::json!([before, [[parent, "CHILD-PARSED"]]]),
            "markup: {markup}"
        );
    }
}
