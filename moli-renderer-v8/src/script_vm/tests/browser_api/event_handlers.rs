use super::*;

mod popup;

#[test]
fn native_mouse_event_coordinate_accessors() {
    let mut vm = new_storage_test_vm("https://event-accessors.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const get = Object.getOwnPropertyDescriptor(MouseEvent.prototype, 'clientX').get;
  const event = new MouseEvent('mousedown', {clientX: 7});
  let rejected = false;
  try { get.call({}); } catch (error) { rejected = error instanceof TypeError; }
  return String(get.call(event) === 7 && rejected);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn event_attribute_handlers_use_html_scope_chain_and_report_compile_errors() {
    let mut vm = new_storage_test_vm("https://event-attribute-scopes.test/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              document.body.innerHTML = `
                <table><tbody><tr><td id="cell"><img id="cell-inner"></td></tr></tbody></table>
                <form id="owner" onsubmit="return false">
                  <button id="button" type="button"><q id="button-inner"></q></button>
                  <img id="form-image" alt="form image">
                </form>
                <a id="error-inner"></a>
              `;

              const cell = document.getElementById("cell");
              const cellInner = document.getElementById("cell-inner");
              cell.cellOwn = true;
              cellInner.innerOwn = true;
              cell.setAttribute("onclick", `
                globalThis.__cellScope = [
                  typeof cellIndex,
                  typeof domain,
                  typeof print,
                  typeof cellOwn,
                  typeof innerOwn,
                  typeof event
                ];
              `);
              cellInner.click();
              cell.setAttribute(
                "onclick",
                `globalThis.__cellScope.push("updated");`
              );
              cellInner.click();

              const form = document.getElementById("owner");
              const button = document.getElementById("button");
              const buttonInner = document.getElementById("button-inner");
              button.buttonOwn = true;
              form.formOwn = true;
              buttonInner.innerOwn = true;
              button.setAttribute("onclick", `
                globalThis.__formScope = [
                  typeof autofocus,
                  typeof form,
                  typeof encoding,
                  typeof domain,
                  typeof buttonOwn,
                  typeof formOwn,
                  typeof innerOwn,
                  typeof event
                ];
              `);
              buttonInner.click();

              const formImage = document.getElementById("form-image");
              globalThis.elements = "global-elements";
              formImage.setAttribute(
                "onclick",
                `globalThis.__imageFormScope = elements;`
              );
              formImage.click();

              globalThis.__windowScope = null;
              globalThis.__compileErrorEvents = 0;
              document.body.bodyOwn = true;
              document.body.setAttribute("onerror", `
                globalThis.__windowScope = [
                  typeof domain,
                  typeof print,
                  typeof bodyOwn,
                  typeof event
                ];
              `);
              window.addEventListener("error", () => {
                globalThis.__compileErrorEvents++;
              });
              const errorInner = document.getElementById("error-inner");
              errorInner.setAttribute("onclick", "cause a compilation error");
              errorInner.click();

              return JSON.stringify({
                cell: globalThis.__cellScope,
                form: globalThis.__formScope,
                imageForm: globalThis.__imageFormScope === form.elements,
                imageIsNotListed: !Array.from(form.elements).includes(formImage),
                window: globalThis.__windowScope,
                errors: globalThis.__compileErrorEvents,
              });
            })()
            "#,
        )
        .expect("event attribute scope probe should evaluate");

    assert_eq!(
        result,
        r#"{"cell":["number","string","function","boolean","undefined","object","updated"],"form":["boolean","object","string","string","boolean","boolean","undefined","object"],"imageForm":true,"imageIsNotListed":true,"window":["undefined","function","undefined","string"],"errors":1}"#,
    );
}

#[test]
fn body_and_frameset_onerror_handlers_use_window_handler_source_text() {
    let mut vm = new_parsed_test_vm(
        "https://window-event-handler-source-text.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const sourceText = element => {
    element.setAttribute("onerror", "foo");
    return element.onerror.toString();
  };
  const div = document.createElement("div");
  const body = document.createElement("body");
  const frameset = document.createElement("frameset");
  const generic = sourceText(div);
  const disconnectedBody = sourceText(body);
  const disconnectedFrameset = sourceText(frameset);
  document.body.setAttribute("onerror", "foo");

  return JSON.stringify({
    generic,
    disconnectedBody,
    disconnectedFrameset,
    connectedBody: window.onerror.toString(),
    bodyOwnAccessor: Object.hasOwn(HTMLBodyElement.prototype, "onerror"),
    framesetOwnAccessor: Object.hasOwn(HTMLFrameSetElement.prototype, "onerror")
  });
})()
"#,
        )
        .expect("body and frameset error handler source-text probe should evaluate");

    assert_eq!(
        result,
        r#"{"generic":"function onerror(event) {\nfoo\n}","disconnectedBody":"function onerror(event, source, lineno, colno, error) {\nfoo\n}","disconnectedFrameset":"function onerror(event, source, lineno, colno, error) {\nfoo\n}","connectedBody":"function onerror(event, source, lineno, colno, error) {\nfoo\n}","bodyOwnAccessor":true,"framesetOwnAccessor":true}"#,
    );
}

#[test]
fn parser_inserted_frameset_window_event_handlers_reflect_on_window() {
    let mut vm = new_storage_test_vm("https://parser-frameset-window-handlers.test/");

    let result = vm
        .eval(
            r#"
(() => {
  window.onload = null;
  window.onerror = null;
  document.open();
  document.write(`
    <!doctype html>
    <html>
      <head></head>
      <frameset
        onload="globalThis.__parserFramesetLoad = this === window"
        onerror="globalThis.__parserFramesetError = [event, source, lineno, colno, error.message].join('|')"
      ></frameset>
    </html>
  `);
  document.close();

  const frameset = document.querySelector("frameset");
  const loadHandler = frameset.onload;
  const errorHandler = window.onerror;
  const beforeInvocation = [
    typeof loadHandler,
    typeof errorHandler,
    window.onload === loadHandler,
    frameset.onerror === errorHandler
  ];
  loadHandler.call(window, new Event("load"));
  errorHandler.call(window, "message", "source", 3, 4, new Error("error"));
  return JSON.stringify({
    beforeInvocation,
    loadResult: globalThis.__parserFramesetLoad,
    errorResult: globalThis.__parserFramesetError
  });
})()
"#,
        )
        .expect("parser-inserted frameset Window handler probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeInvocation":["function","function",true,true],"loadResult":true,"errorResult":"message|source|3|4|error"}"#,
    );
}

mod node_compilation;

mod document_replacement;

#[test]
fn native_wheel_event_delta_accessors() {
    let mut vm = new_storage_test_vm("https://event-accessors.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const get = Object.getOwnPropertyDescriptor(WheelEvent.prototype, 'deltaY').get;
  const event = new WheelEvent('wheel', {deltaY: 7});
  let rejected = false;
  try { get.call({}); } catch (error) { rejected = error instanceof TypeError; }
  return String(get.call(event) === 7 && rejected);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn added_window_and_dom_handler_properties_share_native_registration() {
    let mut vm = new_parsed_test_vm(
        "https://native-handler-surface.test/",
        "<body><iframe id=child></iframe></body>",
    );
    assert_eq!(
        vm.eval(include_str!("event_handlers/native_surface_split.js"))
            .unwrap(),
        "true"
    );
}

#[test]
fn device_window_handler_properties_follow_secure_context_exposure() {
    for url in [
        "https://native-device-handler.test/",
        "http://native-device-handler.test/",
    ] {
        let mut vm = new_parsed_test_vm(url, "<body><iframe id=child></iframe></body>");
        assert_eq!(
            vm.eval(include_str!("event_handlers/native_secure_split.js"))
                .unwrap(),
            "true",
            "{url}"
        );
    }
}

#[test]
fn pointer_raw_update_handler_properties_follow_secure_context_exposure() {
    for (url, secure_context) in [
        ("http://native-pointer-raw-update.test/", false),
        ("https://native-pointer-raw-update.test/", true),
        ("http://localhost/pointer-raw-update", true),
        ("http://127.0.0.1/pointer-raw-update", true),
    ] {
        let mut vm = new_parsed_test_vm(url, "<body><iframe id=child></iframe></body>");
        let script = format!(
            "{}({secure_context})",
            include_str!("event_handlers/pointer_raw_update_secure_split.js")
        );
        assert_eq!(vm.eval(&script).unwrap(), "true", "{url}");
    }
}

#[test]
fn native_pointer_event_field_accessors() {
    let mut vm = new_storage_test_vm("https://event-accessors.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const get = Object.getOwnPropertyDescriptor(PointerEvent.prototype, 'pointerId').get;
  const event = new PointerEvent('pointermove', {pointerId: 7});
  let rejected = false;
  try { get.call({}); } catch (error) { rejected = error instanceof TypeError; }
  return String(get.call(event) === 7 && rejected);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn child_window_onerror_return_values_use_native_cancellation() {
    let mut vm = new_parsed_test_vm(
        "https://child-window-handler-return.test/",
        "<!doctype html><iframe id='child'></iframe>",
    );
    let result = vm
        .eval(
            r#"
(() => {
  const child = document.getElementById("child").contentWindow;
  const rows = [];
  child.onerror = function (...args) {
    rows.push([this === child, args.length, args[0]]);
    return true;
  };
  const error = new child.ErrorEvent("error", {cancelable: true, message: "failure"});
  const errorAllowed = child.dispatchEvent(error);
  child.onerror = function (event) {
    rows.push([this === child, arguments.length, event.type]);
    return false;
  };
  const ordinary = new child.Event("error", {cancelable: true});
  const ordinaryAllowed = child.dispatchEvent(ordinary);
  const noncancelable = new child.Event("error");
  const noncancelableAllowed = child.dispatchEvent(noncancelable);
  return JSON.stringify({rows, errorAllowed, errorPrevented:error.defaultPrevented,
    ordinaryAllowed, ordinaryPrevented:ordinary.defaultPrevented,
    noncancelableAllowed, noncancelablePrevented:noncancelable.defaultPrevented,
    topUntouched: window.onerror === null});
})()
"#,
        )
        .expect("child Window handler return should evaluate");
    assert_eq!(
        result,
        r#"{"rows":[[true,5,"failure"],[true,1,"error"],[true,1,"error"]],"errorAllowed":false,"errorPrevented":true,"ordinaryAllowed":false,"ordinaryPrevented":true,"noncancelableAllowed":true,"noncancelablePrevented":false,"topUntouched":true}"#
    );
}

#[test]
fn document_visibility_handler_is_native_and_can_be_cleared() {
    let mut vm = new_storage_html_test_vm("https://visibility-handler.test/");
    let result = vm
        .eval(
            r#"
(() => {
    const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "onvisibilitychange");
    const isAccessor = typeof descriptor?.get === "function" && typeof descriptor?.set === "function";
    const initiallyNull = document.onvisibilitychange === null;
    const calls = [];
    let listeners = 0;
    document.addEventListener("visibilitychange", () => ++listeners);
    document.onvisibilitychange = function (event) {
        calls.push(this === document && event.currentTarget === document);
    };
    document.dispatchEvent(new Event("visibilitychange"));
    document.onvisibilitychange = null;
    document.dispatchEvent(new Event("visibilitychange"));
    return JSON.stringify([isAccessor, initiallyNull, calls, listeners, document.onvisibilitychange === null]);
})()
"#,
        )
        .unwrap();
    assert_eq!(result, r#"[true,true,[true],2,true]"#);
}
