use super::*;

const OBJECT_VALUES_PROBE: &str = r#"
(() => {
  const settings = __CASE__;
  const frame = settings.kind.startsWith('child-') ? document.createElement('iframe') : null;
  if (frame) document.body.appendChild(frame);
  const w = frame ? frame.contentWindow : window;
  let target, holder, type, cleanup = () => {};
  const kind = settings.kind.replace(/^child-/, '');
  switch (kind) {
    case 'window': target = w; type = 'resize'; break;
    case 'document': target = w.document; type = 'click'; break;
    case 'button': target = w.document.createElement('button'); type = 'click'; break;
    case 'svg': target = w.document.createElementNS('http://www.w3.org/2000/svg', 'svg'); type = 'click'; break;
    case 'body': holder = w.document.createElement('body'); target = w; type = 'resize'; break;
    case 'frameset': holder = w.document.createElement('frameset'); target = w; type = 'resize'; break;
    case 'shadow': target = w.document.createElement('div').attachShadow({mode:'open'}); type = 'slotchange'; break;
    case 'windowless': target = w.document.implementation.createHTMLDocument('').createElement('button'); type = 'click'; break;
    default: throw new Error('unknown target ' + kind);
  }
  holder ||= target;
  const property = 'on' + type;
  const trace = [];
  const errors = [];
  const errorListener = event => { errors.push(event.error instanceof w.TypeError); event.preventDefault(); };
  w.addEventListener('error', errorListener);
  const dispatch = () => target.dispatchEvent(new w.Event(type, {cancelable:true}));
  const objectValues = [{}, Object.create(null), new w.Number(42), new w.String('text'), [], /pattern/];
  const objectIdentity = objectValues.map(value => { holder[property] = value; return holder[property] === value; });
  const primitiveNull = [undefined, null, false, 42, 'text', Symbol('value'), 1n].map(value => {
    holder[property] = value; return holder[property] === null;
  });

  const before = () => trace.push('before');
  const middle = () => trace.push('middle');
  const after = () => trace.push('after');
  const late = () => trace.push('late');
  const replacement = () => { trace.push('replacement'); return false; };
  target.addEventListener(type, before);
  holder[property] = {};
  target.addEventListener(type, middle);
  holder[property] = replacement;
  target.addEventListener(type, after);
  const firstResult = dispatch();
  const firstOrder = trace.splice(0);
  let operationReads = 0;
  const listenerObject = {get handleEvent() { operationReads++; return () => trace.push('object-listener'); }};
  holder[property] = listenerObject;
  const silentResult = dispatch();
  const silentOrder = trace.splice(0);
  const handlerReads = operationReads;
  target.addEventListener(type, listenerObject);
  dispatch();
  const interfaceOrder = trace.splice(0);
  const interfaceReads = operationReads;
  target.removeEventListener(type, listenerObject);
  holder[property] = replacement;
  dispatch();
  const preservedOrder = trace.splice(0);
  holder[property] = 42;
  target.addEventListener(type, late);
  holder[property] = replacement;
  dispatch();
  const reactivatedOrder = trace.splice(0);
  holder[property] = null;
  for (const listener of [before, middle, after, late]) target.removeEventListener(type, listener);

  let proxyGets = 0;
  let proxyCalls = 0;
  let receiverOK = false;
  const callable = new Proxy(function() { trace.push('called'); return false; }, {
    get() { proxyGets++; throw new Error('callback property read'); },
    apply(fn, receiver, args) {
      proxyCalls++; receiverOK = receiver === target && args.length === 1 && args[0].currentTarget === target;
      return Reflect.apply(fn, receiver, args);
    }
  });
  holder[property] = callable;
  const proxyIdentity = holder[property] === callable;
  const proxyResult = dispatch();
  const proxyTrace = trace.splice(0);
  const revokedObject = Proxy.revocable({}, {});
  revokedObject.revoke();
  holder[property] = revokedObject.proxy;
  const revokedObjectIdentity = holder[property] === revokedObject.proxy;
  const revokedObjectResult = dispatch();
  const revokedFunction = Proxy.revocable(function(){}, {});
  holder[property] = revokedFunction.proxy;
  revokedFunction.revoke();
  const revokedFunctionIdentity = holder[property] === revokedFunction.proxy;
  const revokedFunctionResult = dispatch();
  holder[property] = null;
  w.removeEventListener('error', errorListener);
  cleanup();
  if (frame) frame.remove();
  return {objectIdentity, primitiveNull, firstResult, firstOrder, silentResult, silentOrder,
    handlerReads, interfaceOrder, interfaceReads, preservedOrder, reactivatedOrder,
    proxyIdentity, proxyResult, proxyTrace, proxyGets, proxyCalls, receiverOK,
    revokedObjectIdentity, revokedObjectResult, revokedFunctionIdentity, revokedFunctionResult, errors};
})()
"#;

const EXTRA_PROBE: &str = r#"
(() => {
  const mode = __MODE__;
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const child = frame.contentWindow;
  const rows = [];
  let reads = 0;
  if (mode === 'retired-object-realm') {
    const donorFrame = document.createElement('iframe');
    document.body.appendChild(donorFrame);
    const value = new donorFrame.contentWindow.Object();
    Object.defineProperty(value, 'handleEvent', {get() { reads++; throw new Error('unexpected operation lookup'); }});
    const targets = [window, document, document.createElement('button'), child,
      child.document, child.document.createElement('button')];
    const types = ['resize', 'click', 'click', 'resize', 'click', 'click'];
    for (let i = 0; i < targets.length; i++) targets[i]['on' + types[i]] = value;
    donorFrame.remove();
    for (let i = 0; i < targets.length; i++) {
      rows.push([targets[i]['on' + types[i]] === value,
        targets[i].dispatchEvent(new Event(types[i], {cancelable:true}))]);
      targets[i]['on' + types[i]] = null;
    }
  } else {
    for (const w of [window, child]) {
      for (const type of ['error', 'beforeunload']) {
        const name = 'on' + type;
        const value = {get handleEvent() { reads++; throw new Error('unexpected operation lookup'); }};
        const event = () => type === 'error'
          ? new w.ErrorEvent(type, {cancelable:true, message:'message', filename:'source', lineno:3, colno:4, error:'reason'})
          : new w.Event(type, {cancelable:true});
        w[name] = value;
        const identity = w[name] === value;
        const silent = w.dispatchEvent(event());
        let args, receiver;
        w[name] = function() { args = Array.from(arguments); receiver = this === w; return type === 'error' ? true : 123; };
        const result = w.dispatchEvent(event());
        rows.push([identity, silent, result, receiver, args.length,
          type === 'error' ? args.join('|') : args[0].type]);
        w[name] = null;
      }
    }
  }
  frame.remove();
  return {rows, reads};
})()
"#;

fn eval_object_handler_probe(source: &str) -> serde_json::Value {
    let mut vm = new_parsed_test_vm(
        "https://event-handler-object-values.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(&format!("JSON.stringify({source})"))
        .expect("event handler object-value probe should evaluate");
    serde_json::from_str(&result).unwrap()
}

#[test]
fn dom_and_window_handlers_preserve_object_values_and_listener_order() {
    for kind in [
        "window",
        "document",
        "button",
        "svg",
        "body",
        "frameset",
        "shadow",
        "windowless",
        "child-window",
        "child-document",
        "child-button",
        "child-body",
        "child-frameset",
        "child-svg",
    ] {
        let settings = serde_json::json!({"kind":kind});
        let source = OBJECT_VALUES_PROBE.replace("__CASE__", &settings.to_string());
        let errors: Vec<bool> = if kind.starts_with("child-") {
            vec![]
        } else {
            vec![true]
        };
        assert_eq!(
            eval_object_handler_probe(&source),
            serde_json::json!({
                "objectIdentity": [true,true,true,true,true,true],
                "primitiveNull": [true,true,true,true,true,true,true],
                "firstResult": false,
                "firstOrder": ["before","replacement","middle","after"],
                "silentResult": true,
                "silentOrder": ["before","middle","after"],
                "handlerReads": 0,
                "interfaceOrder": ["before","middle","after","object-listener"],
                "interfaceReads": 1,
                "preservedOrder": ["before","replacement","middle","after"],
                "reactivatedOrder": ["before","middle","after","late","replacement"],
                "proxyIdentity": true,
                "proxyResult": false,
                "proxyTrace": ["called"],
                "proxyGets": 0,
                "proxyCalls": 1,
                "receiverOK": true,
                "revokedObjectIdentity": true,
                "revokedObjectResult": true,
                "revokedFunctionIdentity": true,
                "revokedFunctionResult": true,
                "errors": errors,
            }),
            "{kind}"
        );
    }
}

#[test]
fn non_callable_handlers_survive_their_object_realms_retirement() {
    let source = EXTRA_PROBE.replace("__MODE__", r#""retired-object-realm""#);
    assert_eq!(
        eval_object_handler_probe(&source),
        serde_json::json!({
            "rows": [[true,true],[true,true],[true,true],[true,true],[true,true],[true,true]],
            "reads": 0,
        })
    );
}

#[test]
fn legacy_window_error_and_beforeunload_handlers_skip_non_callable_objects() {
    let source = EXTRA_PROBE.replace("__MODE__", r#""special-window-handlers""#);
    assert_eq!(
        eval_object_handler_probe(&source),
        serde_json::json!({
            "rows": [
                [true,true,false,true,5,"message|source|3|4|reason"],
                [true,true,true,true,1,"beforeunload"],
                [true,true,false,true,5,"message|source|3|4|reason"],
                [true,true,true,true,1,"beforeunload"],
            ],
            "reads": 0,
        })
    );
}
