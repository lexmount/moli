use super::*;

#[test]
fn css_escape_preserves_surrogate_code_units() {
    let mut vm = new_storage_test_vm("https://css-escape-surrogates.test/");

    let result = vm
        .eval(
            r#"
(() => {
  return [
    CSS.escape('\uD834\uDF06') === '\uD834\uDF06',
    CSS.escape('\uDF06') === '\uDF06',
    CSS.escape('\uD834') === '\uD834',
    CSS.escape('\0') === '\uFFFD',
    CSS.escape('1a') === '\\31 a',
    CSS.escape('-') === '\\-'
  ].join('|');
})()
"#,
        )
        .expect("CSS.escape surrogate behavior should evaluate");

    assert_eq!(result, "true|true|true|true|true|true");
}
#[test]
fn css_escape_rethrows_string_conversion_errors() {
    let mut vm = new_storage_test_vm("https://css-escape-conversion-errors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  return [
    probe(() => CSS.escape()),
    probe(() => CSS.escape({ toString() { throw new RangeError("escape"); } }))
  ].join("|");
})()
"#,
        )
        .expect("CSS.escape conversion error probe should evaluate");

    assert_eq!(result, "TypeError|RangeError");
}
#[test]
fn get_computed_style_accepts_adopted_child_frame_node_wrapper() {
    let mut vm = new_storage_html_test_vm("https://style-adopted-child-frame-node.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              const host = document.body || document.documentElement || document;
              host.appendChild(frame);
              const div = frame.contentDocument.createElement("div");
              const foreignOwner = div.ownerDocument;

              host.appendChild(div);
              div.style.backgroundColor = "blue";
              const first = getComputedStyle(div).getPropertyValue("background-color");

              frame.remove();
              div.style.backgroundColor = "green";
              const second = getComputedStyle(div).getPropertyValue("background-color");

              return JSON.stringify({
                adopted: div.ownerDocument === document,
                foreignOwnerChanged: foreignOwner !== div.ownerDocument,
                connected: host.contains(div),
                first,
                second
              });
            })()
            "#,
        )
        .expect("getComputedStyle should accept an adopted child-frame node wrapper");

    assert_eq!(
        result,
        r#"{"adopted":true,"foreignOwnerChanged":true,"connected":true,"first":"rgb(0, 0, 255)","second":"rgb(0, 128, 0)"}"#
    );
}
#[test]
fn empty_main_and_section_use_ordinary_css_block_geometry() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = eval_with_layout_publications(
        &mut vm,
        r#"
            (function* () {
              if (!document.documentElement) {
                document.appendChild(document.createElement("html"));
              }
              if (!document.body) {
                document.documentElement.appendChild(document.createElement("body"));
              }
              document.body.innerHTML = "";
              for (let i = 0; i < 40; i++) {
                document.body.appendChild(document.createElement("nav"));
              }
              const main = document.createElement("main");
              const child = document.createElement("section");
              child.id = "message-list";
              main.appendChild(child);
              document.body.appendChild(main);
              yield; // Publish this scene before reading its geometry.
const mainRect = main.getBoundingClientRect();
              const childRect = child.getBoundingClientRect();
              return JSON.stringify({
                main: {
                  top: mainRect.top,
                  width: mainRect.width,
                  height: mainRect.height,
                  clientWidth: main.clientWidth,
                  clientHeight: main.clientHeight
                },
                child: {
                  top: childRect.top,
                  width: childRect.width,
                  height: childRect.height,
                  offsetTop: child.offsetTop
                }
              });
            })()
            "#,
    )
    .expect("main geometry probe should evaluate");

    assert_eq!(
        result,
        r#"{"main":{"top":8,"width":1904,"height":0,"clientWidth":1904,"clientHeight":0},"child":{"top":8,"width":1904,"height":0,"offsetTop":8}}"#
    );
}
#[test]
fn dom_matrix_exposes_webkit_css_matrix_alias() {
    let mut vm = new_storage_test_vm("https://dommatrix-webkit-alias.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const webkitDescriptor = Object.getOwnPropertyDescriptor(globalThis, "WebKitCSSMatrix");
  const matrix = new WebKitCSSMatrix();
  return [
    WebKitCSSMatrix === DOMMatrix,
    WebKitCSSMatrix.prototype === DOMMatrix.prototype,
    WebKitCSSMatrix.name,
    matrix instanceof DOMMatrix,
    matrix instanceof DOMMatrixReadOnly,
    [webkitDescriptor.writable, webkitDescriptor.enumerable, webkitDescriptor.configurable].join(",")
  ].join("|");
})()
"#,
        )
        .expect("DOMMatrix legacy Window aliases should evaluate");

    assert_eq!(result, "true|true|DOMMatrix|true|true|true,false,true");
}

#[test]
fn dom_matrix_window_operations_use_webidl_descriptors() {
    let mut vm = new_storage_test_vm("https://dommatrix-operation-descriptors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptorShape = (owner, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      typeof descriptor.value,
      descriptor.value.length,
      descriptor.enumerable,
      descriptor.writable,
      descriptor.configurable
    ].join(",");
  };
  return [
    descriptorShape(DOMMatrixReadOnly.prototype, "toString"),
    descriptorShape(DOMMatrix.prototype, "setMatrixValue")
  ].join("|");
})()
"#,
        )
        .expect("DOMMatrix Window operation descriptors should evaluate");

    assert_eq!(
        result,
        "function,0,true,true,true|function,1,true,true,true"
    );
}
