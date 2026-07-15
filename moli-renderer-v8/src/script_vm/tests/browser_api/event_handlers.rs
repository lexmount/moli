use super::*;

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
                window: globalThis.__windowScope,
                errors: globalThis.__compileErrorEvents,
              });
            })()
            "#,
        )
        .expect("event attribute scope probe should evaluate");

    assert_eq!(
        result,
        r#"{"cell":["number","string","function","boolean","undefined","object","updated"],"form":["boolean","object","string","string","boolean","boolean","undefined","object"],"window":["undefined","function","undefined","string"],"errors":1}"#,
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
fn inline_event_handlers_retain_listener_registration_order() {
    let mut vm = new_parsed_test_vm(
        "https://inline-event-handler-order.test/",
        "<!doctype html><html><head></head><body><div id=parsed onclick=\"this.order.push('HANDLER')\"></div></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const invalidForced = [];
  const forced = document.createElement("div");
  forced.order = invalidForced;
  forced.addEventListener("click", () => invalidForced.push("ONE"));
  forced.setAttribute("onclick", "window.open(");
  forced.addEventListener("click", () => invalidForced.push("THREE"));
  void forced.onclick;
  forced.setAttribute("onclick", "this.order.push('TWO')");
  forced.dispatchEvent(new Event("click"));

  const invalidDispatched = [];
  const dispatched = document.createElement("div");
  dispatched.order = invalidDispatched;
  dispatched.addEventListener("click", () => invalidDispatched.push("ONE"));
  dispatched.setAttribute("onclick", "window.open(");
  dispatched.addEventListener("click", () => invalidDispatched.push("THREE"));
  dispatched.dispatchEvent(new Event("click"));
  dispatched.setAttribute("onclick", "this.order.push('TWO')");
  dispatched.dispatchEvent(new Event("click"));

  const property = [];
  const propertyTarget = document.createElement("div");
  propertyTarget.addEventListener("click", () => property.push("ONE"));
  propertyTarget.onclick = () => property.push("OLD");
  propertyTarget.addEventListener("click", () => property.push("THREE"));
  propertyTarget.onclick = () => property.push("TWO");
  propertyTarget.dispatchEvent(new Event("click"));
  const propertyReplacement = property.splice(0);
  propertyTarget.onclick = null;
  propertyTarget.onclick = () => property.push("RE-ADDED");
  propertyTarget.dispatchEvent(new Event("click"));

  const removedAttribute = [];
  const attributeTarget = document.createElement("div");
  attributeTarget.order = removedAttribute;
  attributeTarget.addEventListener("click", () => removedAttribute.push("ONE"));
  attributeTarget.setAttribute("onclick", "this.order.push('OLD')");
  attributeTarget.addEventListener("click", () => removedAttribute.push("THREE"));
  attributeTarget.removeAttribute("onclick");
  attributeTarget.setAttribute("onclick", "this.order.push('RE-ADDED')");
  attributeTarget.dispatchEvent(new Event("click"));

  const capture = [];
  const captureTarget = document.createElement("div");
  captureTarget.onclick = () => capture.push("HANDLER");
  captureTarget.addEventListener("click", event => {
    capture.push("CAPTURE");
    event.stopPropagation();
  }, true);
  captureTarget.addEventListener("click", () => capture.push("CAPTURE-2"), true);
  captureTarget.addEventListener("click", () => capture.push("THREE"));
  captureTarget.dispatchEvent(new Event("click"));

  const parsed = [];
  const parsedTarget = document.getElementById("parsed");
  parsedTarget.order = parsed;
  parsedTarget.addEventListener("click", () => parsed.push("LISTENER"));
  parsedTarget.dispatchEvent(new Event("click"));

  const bubbling = [];
  const parent = document.createElement("div");
  const child = parent.appendChild(document.createElement("span"));
  parent.order = bubbling;
  parent.addEventListener("click", () => bubbling.push("ONE"));
  parent.setAttribute("onclick", "this.order.push('TWO')");
  parent.addEventListener("click", () => bubbling.push("THREE"));
  child.dispatchEvent(new Event("click", { bubbles: true }));

  return JSON.stringify({
    invalidForced,
    invalidDispatched,
    propertyReplacement,
    propertyReadded: property,
    removedAttribute,
    capture,
    parsed,
    bubbling
  });
})()
"#,
        )
        .expect("inline event handler ordering probe should evaluate");

    assert_eq!(
        result,
        r#"{"invalidForced":["ONE","TWO","THREE"],"invalidDispatched":["ONE","THREE","ONE","TWO","THREE"],"propertyReplacement":["ONE","TWO","THREE"],"propertyReadded":["ONE","THREE","RE-ADDED"],"removedAttribute":["ONE","THREE","RE-ADDED"],"capture":["CAPTURE","CAPTURE-2"],"parsed":["HANDLER","LISTENER"],"bubbling":["ONE","TWO","THREE"]}"#,
    );
}

#[test]
fn window_reflecting_handlers_share_listener_registration_order() {
    let mut vm = new_parsed_test_vm(
        "https://window-event-handler-order.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__bodyErrorOrder = [];
  window.addEventListener("error", () => __bodyErrorOrder.push("ONE"));
  document.body.setAttribute("onerror", "__bodyErrorOrder.push('TWO'); return true");
  window.addEventListener("error", () => __bodyErrorOrder.push("THREE"));
  window.dispatchEvent(new ErrorEvent("error"));

  globalThis.__bodyLoadOrder = [];
  window.addEventListener("load", () => __bodyLoadOrder.push("ONE"));
  document.body.setAttribute("onload", "__bodyLoadOrder.push('TWO')");
  window.addEventListener("load", () => __bodyLoadOrder.push("THREE"));
  window.dispatchEvent(new Event("load"));

  globalThis.__bodyMessageErrorOrder = [];
  window.addEventListener("messageerror", () => __bodyMessageErrorOrder.push("ONE"));
  document.body.setAttribute(
    "onmessageerror",
    "__bodyMessageErrorOrder.push('TWO')"
  );
  window.addEventListener("messageerror", () => __bodyMessageErrorOrder.push("THREE"));
  window.dispatchEvent(new Event("messageerror"));

  const rejectionOrder = [];
  window.addEventListener("unhandledrejection", () => rejectionOrder.push("ONE"));
  window.onunhandledrejection = () => rejectionOrder.push("TWO");
  window.addEventListener("unhandledrejection", () => rejectionOrder.push("THREE"));
  const promise = Promise.resolve();
  window.dispatchEvent(new PromiseRejectionEvent("unhandledrejection", { promise }));

  return JSON.stringify({
    bodyError: __bodyErrorOrder,
    bodyLoad: __bodyLoadOrder,
    bodyMessageError: __bodyMessageErrorOrder,
    rejection: rejectionOrder
  });
})()
"#,
        )
        .expect("window-reflecting event handler ordering probe should evaluate");

    assert_eq!(
        result,
        r#"{"bodyError":["ONE","TWO","THREE"],"bodyLoad":["ONE","TWO","THREE"],"bodyMessageError":["ONE","TWO","THREE"],"rejection":["ONE","TWO","THREE"]}"#,
    );
}

#[test]
fn window_document_and_shadow_handlers_share_listener_registration_order() {
    let mut vm = new_parsed_test_vm(
        "https://event-handler-owner-order.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const windowOrder = [];
  window.addEventListener("message", () => windowOrder.push("ONE"));
  window.onmessage = () => windowOrder.push("TWO");
  window.addEventListener("message", () => windowOrder.push("THREE"));
  window.dispatchEvent(new MessageEvent("message"));

  const documentOrder = [];
  document.addEventListener("readystatechange", () => documentOrder.push("ONE"));
  document.onreadystatechange = () => documentOrder.push("TWO");
  document.addEventListener("readystatechange", () => documentOrder.push("THREE"));
  document.dispatchEvent(new Event("readystatechange"));

  const shadowOrder = [];
  const shadow = document.createElement("div").attachShadow({ mode: "open" });
  shadow.addEventListener("slotchange", () => shadowOrder.push("ONE"));
  shadow.onslotchange = () => shadowOrder.push("TWO");
  shadow.addEventListener("slotchange", () => shadowOrder.push("THREE"));
  shadow.dispatchEvent(new Event("slotchange"));

  let customExpandoCalled = false;
  const customTarget = document.createElement("div");
  customTarget.onmolicustom = () => { customExpandoCalled = true; };
  customTarget.dispatchEvent(new Event("molicustom"));

  return JSON.stringify({
    window: windowOrder,
    document: documentOrder,
    shadow: shadowOrder,
    customExpandoCalled
  });
})()
"#,
        )
        .expect("event handler owner ordering probe should evaluate");

    assert_eq!(
        result,
        r#"{"window":["ONE","TWO","THREE"],"document":["ONE","TWO","THREE"],"shadow":["ONE","TWO","THREE"],"customExpandoCalled":false}"#,
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
