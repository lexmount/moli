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
