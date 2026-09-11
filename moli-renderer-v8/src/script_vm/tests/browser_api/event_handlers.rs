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
fn body_and_frameset_window_event_handlers_share_owner_and_content_sources() {
    let mut vm = new_parsed_test_vm(
        "https://body-window-event-handler-owner.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const reflectedGlobalHandlers = [
    "onblur", "onerror", "onfocus", "onload", "onresize", "onscroll"
  ];
  const windowEventHandlers = [
    "onafterprint", "onbeforeprint", "onbeforeunload", "onhashchange",
    "onlanguagechange", "onmessage", "onmessageerror", "onoffline",
    "ononline", "onpagehide", "onpagereveal", "onpageshow", "onpageswap",
    "onpopstate", "onrejectionhandled", "onstorage",
    "onunhandledrejection", "onunload"
  ];
  const nonReflectedGlobalHandlers = [
    "onbeforeinput", "onbeforematch", "oncommand", "oncontextlost",
    "oncontextrestored", "oncuechange", "onformdata",
    "onsecuritypolicyviolation", "onwebkitanimationend",
    "onwebkitanimationiteration", "onwebkitanimationstart",
    "onwebkittransitionend"
  ];
  const handlers = [...reflectedGlobalHandlers, ...windowEventHandlers];
  const body = document.createElement("body");
  const frameset = document.createElement("frameset");

  const idlReflection = handlers.every(name => {
    const handler = () => name;
    window[name] = null;
    body[name] = handler;
    const bodyToWindow = window[name] === handler && frameset[name] === handler;
    window[name] = null;
    frameset[name] = handler;
    const framesetToWindow = window[name] === handler && body[name] === handler;
    window[name] = null;
    return bodyToWindow && framesetToWindow;
  });

  const contentReflection = handlers.every(name => {
    window[name] = null;
    body.setAttribute(name, "return 1");
    const bodyHandler = window[name];
    const bodyReflected =
      typeof bodyHandler === "function" && body[name] === bodyHandler;
    frameset.setAttribute(name, "return 2");
    const framesetHandler = window[name];
    const framesetReflected =
      typeof framesetHandler === "function" &&
      frameset[name] === framesetHandler &&
      framesetHandler !== bodyHandler;
    frameset.removeAttribute(name);
    const removalReflected = window[name] === null && body[name] === null;
    body.removeAttribute(name);
    return bodyReflected && framesetReflected && removalReflected;
  });

  let resizeCurrentTargetIsWindow = false;
  body.onresize = event => {
    resizeCurrentTargetIsWindow = event.currentTarget === window;
  };
  window.dispatchEvent(new Event("resize"));
  window.onresize = null;

  const nonReflectedGlobalHandler = nonReflectedGlobalHandlers.every(name => {
    const handler = () => name;
    window[name] = null;
    body[name] = handler;
    const staysOnBody =
      body[name] === handler && frameset[name] === null && window[name] === null;
    body[name] = null;
    window[name] = handler;
    const staysOnWindow =
      window[name] === handler && body[name] === null && frameset[name] === null;
    window[name] = null;
    return staysOnBody && staysOnWindow;
  });

  const prefixedEventTypes = new Map([
    ["onwebkitanimationend", "webkitAnimationEnd"],
    ["onwebkitanimationiteration", "webkitAnimationIteration"],
    ["onwebkitanimationstart", "webkitAnimationStart"],
    ["onwebkittransitionend", "webkitTransitionEnd"]
  ]);
  globalThis.__prefixedEventAttributeRuns = 0;
  const prefixedEventResults = [...prefixedEventTypes].map(([name, type]) => {
    const element = document.createElement("meta");
    let propertyRuns = 0;
    element[name] = () => { propertyRuns++; };
    const propertyEvent = new Event(type);
    element.dispatchEvent(propertyEvent);
    const attributeRunsBefore = globalThis.__prefixedEventAttributeRuns;
    element.setAttribute(name, "globalThis.__prefixedEventAttributeRuns++");
    const attributeEvent = new Event(type);
    element.dispatchEvent(attributeEvent);
    return [
      name,
      propertyEvent.type,
      attributeEvent.type,
      propertyRuns,
      globalThis.__prefixedEventAttributeRuns - attributeRunsBefore
    ];
  });
  const prefixedEventTypeMapping = prefixedEventResults.every(result =>
    result[3] === 1 && result[4] === 1
  );

  const windowlessDocument = new DOMParser().parseFromString("", "text/html");
  const windowlessBody = windowlessDocument.createElement("body");
  const windowlessFrameset = windowlessDocument.createElement("frameset");
  const windowlessHandlersStayNull = handlers.every(name => {
    const windowHandler = () => name;
    window[name] = windowHandler;
    const initiallyNull =
      windowlessBody[name] === null && windowlessFrameset[name] === null;
    windowlessBody[name] = () => "body";
    windowlessFrameset[name] = () => "frameset";
    const settersIgnored =
      window[name] === windowHandler &&
      windowlessBody[name] === null &&
      windowlessFrameset[name] === null;
    window[name] = null;
    return initiallyNull && settersIgnored;
  });

  return JSON.stringify({
    idlReflection,
    contentReflection,
    resizeCurrentTargetIsWindow,
    nonReflectedGlobalHandler,
    prefixedEventTypeMapping,
    windowlessHandlersStayNull,
    bodyOwnAccessors: handlers.every(name =>
      Object.hasOwn(HTMLBodyElement.prototype, name)
    ),
    framesetOwnAccessors: handlers.every(name =>
      Object.hasOwn(HTMLFrameSetElement.prototype, name)
    )
  });
})()
"#,
        )
        .expect("body and frameset Window event handler owner probe should evaluate");

    assert_eq!(
        result,
        r#"{"idlReflection":true,"contentReflection":true,"resizeCurrentTargetIsWindow":true,"nonReflectedGlobalHandler":true,"prefixedEventTypeMapping":true,"windowlessHandlersStayNull":true,"bodyOwnAccessors":true,"framesetOwnAccessors":true}"#,
    );
}

#[test]
fn child_frameset_event_handler_property_reflects_on_its_window() {
    let mut vm = new_parsed_test_vm(
        "https://child-frameset-window-handler.test/",
        "<!doctype html><iframe id='child'></iframe>",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("child");
  const childWindow = frame.contentWindow;
  const childDocument = frame.contentDocument;
  const frameset = childDocument.createElement("frameset");
  childDocument.documentElement.replaceChild(frameset, childDocument.body);
  const errorCalls = [];
  const errorHandler = function (...args) {
    errorCalls.push([
      args.length,
      this === childWindow,
      args[0],
      args[1],
      args[2],
      args[3],
      args[4]
    ]);
    return true;
  };
  frameset.onerror = errorHandler;
  const reflectedErrorHandler =
    frameset.onerror === errorHandler && childWindow.onerror === errorHandler;
  const error = new ErrorEvent("error", {
    bubbles: true,
    cancelable: true,
    message: "message",
    filename: "source",
    lineno: 3,
    colno: 4,
    error: "reason"
  });
  frameset.dispatchEvent(error);

  const ordinaryCalls = [];
  const ordinaryHandler = function (...args) {
    ordinaryCalls.push([args.length, this === childWindow, args[0].type]);
    return true;
  };
  frameset.onerror = ordinaryHandler;
  const reflectedOrdinaryHandler =
    frameset.onerror === ordinaryHandler && childWindow.onerror === ordinaryHandler;
  const ordinary = new Event("error", { bubbles: true, cancelable: true });
  frameset.dispatchEvent(ordinary);

  return JSON.stringify({
    reflectedErrorHandler,
    reflectedOrdinaryHandler,
    mainWindowUntouched: window.onerror === null,
    errorCalls,
    errorDefaultPrevented: error.defaultPrevented,
    ordinaryCalls,
    ordinaryDefaultPrevented: ordinary.defaultPrevented
  });
})()
"#,
        )
        .expect("child frameset Window event handler probe should evaluate");

    assert_eq!(
        result,
        r#"{"reflectedErrorHandler":true,"reflectedOrdinaryHandler":true,"mainWindowUntouched":true,"errorCalls":[[5,true,"message","source",3,4,"reason"]],"errorDefaultPrevented":true,"ordinaryCalls":[[1,true,"error"]],"ordinaryDefaultPrevented":false}"#,
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
