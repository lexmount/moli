use super::*;

fn child_inline_test_vm(loaded_srcdoc: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://child-inline.test/parent");
    vm.eval(
        r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        globalThis.frame = document.createElement('iframe');
        body.appendChild(frame);
        "#,
    )
    .unwrap();
    if loaded_srcdoc {
        vm.eval("frame.srcdoc = '<!doctype html><html><head></head><body></body></html>'")
            .unwrap();
        vm.drain_pending_child_frame_work_for_test();
    }
    vm
}

#[test]
fn child_dynamic_inline_scripts_execute_synchronously_with_nested_current_script() {
    for loaded_srcdoc in [false, true] {
        let mut vm = child_inline_test_vm(loaded_srcdoc);
        let result = vm
            .eval(
                r#"
                (() => {
                  globalThis.events = [];
                  const child = frame.contentDocument;
                  frame.contentWindow.record = value => events.push(value);
                  const script = document.createElement('script');
                  script.id = 'outer';
                  script.textContent = `
                    record('outer:' + document.currentScript.id);
                    record('realm:' + (globalThis === document.defaultView));
                    const inner = document.createElement('script');
                    inner.id = 'inner';
                    inner.text = "record('inner:' + document.currentScript.id)";
                    document.body.appendChild(inner);
                    record('restored:' + document.currentScript.id);
                    document.body.appendChild(document.currentScript);
                    Promise.resolve().then(() => record('microtask:' + document.currentScript));
                  `;
                  child.body.appendChild(script);
                  events.push('returned:' + child.currentScript);
                  script.remove();
                  child.body.appendChild(script);
                  events.push('reinserted');
                  return JSON.stringify(events);
                })()
                "#,
            )
            .unwrap();
        assert_eq!(
            result,
            r#"["outer:outer","realm:true","inner:inner","restored:outer","returned:null","reinserted"]"#,
            "loaded_srcdoc={loaded_srcdoc}"
        );
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval("JSON.stringify(events)").unwrap(),
            r#"["outer:outer","realm:true","inner:inner","restored:outer","returned:null","reinserted","microtask:null"]"#,
            "script cleanup must neither run during the outer call nor execute the script again"
        );
    }
}

#[test]
fn child_dynamic_inline_scripts_prepare_each_connected_script_after_reentrant_mutation() {
    let mut vm = child_inline_test_vm(false);
    let result = vm
        .eval(
            r#"
            (() => {
              const child = frame.contentDocument;
              const events = [];
              frame.contentWindow.record = value => events.push(value);
              const fragment = child.createDocumentFragment();
              const first = child.createElement('script');
              const second = child.createElement('script');
              const removed = child.createElement('script');
              second.id = 'second';
              removed.id = 'removed';
              second.textContent = "record('stale source')";
              removed.textContent = "record('removed script')";
              first.textContent = `
                record('first');
                document.getElementById('removed').remove();
                document.getElementById('second').textContent = "record('updated source')";
                record('first finished');
              `;
              fragment.append(first, second, removed);
              child.body.appendChild(fragment);
              events.push('returned');
              child.body.appendChild(removed);
              return JSON.stringify(events);
            })()
            "#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["first","updated source","first finished","returned","removed script"]"#
    );
}

#[test]
fn child_dynamic_inline_script_exceptions_are_reported_before_insertion_returns() {
    for loaded_srcdoc in [false, true] {
        let mut vm = child_inline_test_vm(loaded_srcdoc);
        let result = vm
            .eval(
                r#"
                (() => {
                  const child = frame.contentDocument;
                  const win = frame.contentWindow;
                  const events = [];
                  window.onerror = () => { events.push('wrong Window'); return true; };
                  win.onerror = (message, source, line, column, error) => {
                    events.push(error.name + ':' + (error instanceof win.Error) + ':' +
                                child.currentScript.id);
                    return true;
                  };
                  for (const [id, source] of [['syntax', '{'], ['runtime', "throw new TypeError('test')"]]) {
                    const script = document.createElement('script');
                    script.id = id;
                    script.textContent = source;
                    script.onload = () => events.push('unexpected load');
                    script.onerror = () => events.push('unexpected element error');
                    try {
                      child.body.appendChild(script);
                      events.push('returned:' + child.currentScript);
                    } catch (error) {
                      events.push('escaped:' + error.name);
                    }
                  }
                  return JSON.stringify(events);
                })()
                "#,
            )
            .unwrap();
        assert_eq!(
            result,
            r#"["SyntaxError:true:syntax","returned:null","TypeError:true:runtime","returned:null"]"#,
            "loaded_srcdoc={loaded_srcdoc}"
        );
    }
}

#[test]
fn child_dynamic_inline_script_adoption_preserves_its_execution_document() {
    let mut vm = child_inline_test_vm(false);
    let result = vm
        .eval(
            r#"
            (() => {
              globalThis.events = [];
              const child = frame.contentDocument;
              const script = document.createElement('script');
              script.textContent = `
                const script = document.currentScript;
                parent.events.push('before:' + (script.ownerDocument === document));
                parent.document.body.appendChild(script);
                parent.events.push('after:' + (script.ownerDocument === parent.document));
                parent.events.push('current:' + (document.currentScript === script));
                parent.events.push('parent:' + parent.document.currentScript);
              `;
              child.body.appendChild(script);
              events.push('returned:' + child.currentScript);
              const inactiveScript = child.createElement('script');
              inactiveScript.textContent = "parent.events.push('inactive')";
              const inactiveBody = child.body;
              frame.remove();
              inactiveBody.appendChild(inactiveScript);
              return JSON.stringify(events);
            })()
            "#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["before:true","after:true","current:true","parent:null","returned:null"]"#
    );
}

#[test]
fn child_dynamic_inline_script_current_script_uses_shadow_root_at_execution_entry() {
    let mut vm = child_inline_test_vm(false);
    let result = vm
        .eval(
            r#"
            (() => {
              const child = frame.contentDocument;
              const events = [];
              frame.contentWindow.record = value => events.push(value);
              const script = child.createElement('script');
              script.id = 'outer';
              script.textContent = `
                const outer = document.currentScript;
                const host = document.createElement('div');
                document.body.appendChild(host);
                const shadow = host.attachShadow({mode: 'open'});
                shadow.appendChild(outer);
                record('moved:' + (document.currentScript === outer));
                const inner = document.createElement('script');
                inner.textContent = \`
                  record('shadow:' + document.currentScript);
                  document.body.appendChild(inner);
                  record('moved out:' + document.currentScript);
                \`;
                shadow.appendChild(inner);
                record('restored:' + (document.currentScript === outer));
              `;
              child.body.appendChild(script);
              events.push('returned:' + child.currentScript);
              return JSON.stringify(events);
            })()
            "#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["moved:true","shadow:null","moved out:null","restored:true","returned:null"]"#
    );
}
