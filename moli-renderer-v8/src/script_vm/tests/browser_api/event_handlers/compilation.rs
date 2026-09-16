use super::*;

const TARGET_KINDS: [&str; 4] = ["top-window", "top-node", "child-window", "child-node"];

const COMPILATION_PROBE: &str = r#"
(() => {
  const settings = __CASE__;
  const child = settings.kind.startsWith('child');
  const frame = child ? document.createElement('iframe') : null;
  if (frame) document.body.appendChild(frame);
  const w = frame ? frame.contentWindow : window;
  const element = settings.kind.endsWith('node')
    ? w.document.body.appendChild(w.document.createElement('button'))
    : w.document.createElement('body');
  const target = settings.kind.endsWith('node') ? element : w;
  const type = settings.kind.endsWith('node') ? 'click' : 'resize';
  const property = 'on' + type;
  const trace = w.__handlerTrace = [];
  const errors = [];
  let stackReads = 0;
  const oldStack = Object.getOwnPropertyDescriptor(w.Error, 'prepareStackTrace');
  if (settings.action === 'stack') {
    Object.defineProperty(w.Error, 'prepareStackTrace', {
      configurable: true, get() { stackReads++; return () => 'author stack'; }
    });
  }
  const replacement = () => { trace.push('replacement'); return false; };
  const before = () => trace.push('before');
  const after = () => trace.push('after');
  const late = () => trace.push('late');
  let sawNull;
  const onError = event => {
    trace.push('error');
    errors.push(event.error instanceof w.SyntaxError);
    event.preventDefault();
    sawNull = target[property] === null;
    switch (settings.action) {
      case 'idl': target[property] = replacement; break;
      case 'attribute':
        element.setAttribute(property, "__handlerTrace.push('replacement'); return false;");
        break;
      case 'clear': target[property] = null; break;
      case 'remove': element.removeAttribute(property); break;
      case 'open':
        w.document.open();
        w.document.write('<!doctype html><body onresize="__handlerTrace.push(\'replacement\'); return false;">');
        w.document.close();
        break;
    }
  };
  w.addEventListener('error', onError);
  target.addEventListener(type, before);
  element.setAttribute(property, '}');
  target.addEventListener(type, after);
  const initial = settings.trigger === 'read'
    ? target[property] === null
    : target.dispatchEvent(new w.Event(type, {cancelable: true}));
  const firstTrace = trace.splice(0);
  const currentType = typeof target[property];
  target.addEventListener(type, late);
  if (['none', 'clear', 'remove', 'stack'].includes(settings.action)) {
    target[property] = replacement;
  }
  const result = target.dispatchEvent(new w.Event(type, {cancelable: true}));
  const secondTrace = trace.splice(0);
  const hasAttribute = element.hasAttribute(property);
  target[property] = null;
  target.removeEventListener(type, before);
  target.removeEventListener(type, after);
  target.removeEventListener(type, late);
  w.removeEventListener('error', onError);
  if (settings.action === 'stack') {
    if (oldStack) Object.defineProperty(w.Error, 'prepareStackTrace', oldStack);
    else delete w.Error.prepareStackTrace;
  }
  if (frame) frame.remove();
  return {initial, sawNull, errors, firstTrace, currentType, result, secondTrace, hasAttribute, stackReads};
})()
"#;

fn assert_compilation_error_reentry(kind: &str, action: &str, trigger: &str) {
    let mut vm = new_parsed_test_vm(
        "https://handler-compilation-reentry.test/",
        "<!doctype html><body></body>",
    );
    let settings = serde_json::json!({"kind": kind, "action": action, "trigger": trigger});
    let source = COMPILATION_PROBE.replace("__CASE__", &settings.to_string());
    let result = vm
        .eval(&format!("JSON.stringify({source})"))
        .expect("a handler parse error should report without throwing from the getter");
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    let first_trace: &[&str] = if trigger == "read" {
        &["error"]
    } else {
        &["before", "error", "after"]
    };
    let second_trace: &[&str] = match action {
        "clear" | "remove" => &["before", "after", "late", "replacement"],
        "open" => &["replacement", "late"],
        _ => &["before", "replacement", "after", "late"],
    };
    let current_type = if matches!(action, "idl" | "attribute" | "open") {
        "function"
    } else {
        "object"
    };
    assert_eq!(
        result,
        serde_json::json!({
            "initial": true,
            "sawNull": true,
            "errors": [true],
            "firstTrace": first_trace,
            "currentType": current_type,
            "result": false,
            "secondTrace": second_trace,
            "hasAttribute": action != "remove",
            "stackReads": 0,
        }),
        "{settings}"
    );
}

#[test]
fn handler_parse_errors_preserve_reentrant_idl_and_attribute_replacements() {
    for kind in TARGET_KINDS {
        for action in ["idl", "attribute"] {
            for trigger in ["read", "dispatch"] {
                assert_compilation_error_reentry(kind, action, trigger);
            }
        }
    }
}

#[test]
fn handler_parse_errors_preserve_reentrant_listener_deactivation() {
    for kind in TARGET_KINDS {
        for action in ["clear", "remove"] {
            for trigger in ["read", "dispatch"] {
                assert_compilation_error_reentry(kind, action, trigger);
            }
        }
    }
}

#[test]
fn handler_parse_errors_cache_null_without_removing_the_listener_slot() {
    for kind in TARGET_KINDS {
        for trigger in ["read", "dispatch"] {
            assert_compilation_error_reentry(kind, "none", trigger);
        }
    }
}

#[test]
fn handler_parse_errors_do_not_evaluate_author_stack_hooks() {
    for kind in TARGET_KINDS {
        assert_compilation_error_reentry(kind, "stack", "read");
    }
}

#[test]
fn handler_parse_errors_do_not_overwrite_document_open_handlers() {
    for kind in ["top-window", "child-window"] {
        assert_compilation_error_reentry(kind, "open", "read");
    }
}
