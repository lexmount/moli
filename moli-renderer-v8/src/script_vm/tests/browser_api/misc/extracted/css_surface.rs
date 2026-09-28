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
    let mut vm = new_storage_test_vm("https://style-adopted-child-frame-node.test/");

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
fn dom_quad_factories_and_default_to_json_use_the_function_realm() {
    let mut vm = new_storage_test_vm("https://domquad-function-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement ||
    document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "domquad-function-realm";
  body.appendChild(frame);
})()
"#,
    )
    .expect("DOMQuad factory child frame should be created");
    materialize_single_child_default_realm_for_test(&mut vm, "DOMQuad factory child Realm");

    let result = vm
        .eval(
            r#"
(() => {
  const other = document.getElementById("domquad-function-realm").contentWindow;
  const quad = other.DOMQuad.fromRect({x: 1, y: 2, width: 3, height: 4});
  const source = new DOMQuad({x: 7, y: 8});
  const json = other.DOMQuad.prototype.toJSON.call(source);
  return [
    quad instanceof other.DOMQuad,
    quad instanceof DOMQuad,
    quad.p1 instanceof other.DOMPoint,
    quad.p1 instanceof DOMPoint,
    quad.getBounds() instanceof other.DOMRect,
    quad.getBounds() instanceof DOMRect,
    Object.getPrototypeOf(json) === other.Object.prototype,
    json.p1 === source.p1,
    json.p1 instanceof DOMPoint,
    json.p1 instanceof other.DOMPoint
  ].join("|");
})()
"#,
        )
        .expect("DOMQuad function realm probe should evaluate");

    assert_eq!(
        result,
        "true|false|true|false|true|false|true|true|true|false"
    );
}

#[test]
fn dom_quad_uses_live_same_object_points_and_geometry_dictionary_factories() {
    let mut vm = new_storage_test_vm("https://domquad-geometry.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.name;
    }
  };
  const q = new DOMQuad(
    {x: 1, y: 2, z: 3, w: 4},
    {x: 11, y: 2},
    {x: 11, y: 22},
    {x: 1, y: 22}
  );
  const initial = [q.p1.x, q.p1.y, q.p1.z, q.p1.w,
    q.getBounds().x, q.getBounds().y, q.getBounds().width,
    q.getBounds().height].join(",");
  const copy = DOMQuad.fromQuad(q);
  const copyInitial = [copy.p1.x, copy.p1.y, copy.p1.z, copy.p1.w,
    copy.p4.x, copy.p4.y].join(",");
  const originalP1 = q.p1;
  q.p1.x = -5;
  q.p4.y = 30;
  q.p1 = new DOMPoint(999, 999);
  const live = [q.p1 === originalP1, q.p1.x, q.p4.y,
    q.getBounds().x, q.getBounds().y, q.getBounds().width,
    q.getBounds().height].join(",");
  const copyAfterMutation = [copy.p1.x, copy.p4.y, copy.p1 !== q.p1].join(",");
  const negative = DOMQuad.fromRect({x: 10, y: 20, width: -100, height: -200});
  const negativeValues = [negative.p1.x, negative.p1.y, negative.p2.x,
    negative.p3.y, negative.getBounds().x, negative.getBounds().y,
    negative.getBounds().width, negative.getBounds().height].join(",");
  const nanBounds = new DOMQuad({x: 0, y: 0}, {x: 0, y: 0},
    {x: NaN, y: 0}, {x: 0, y: 0}).getBounds();
  const nanValues = [nanBounds.x, nanBounds.y, nanBounds.width,
    nanBounds.height].join(",");
  const json = q.toJSON();
  const pointDescriptor = Object.getOwnPropertyDescriptor(DOMQuad.prototype, "p1");
  const fake = Object.assign(Object.create(DOMQuad.prototype), {
    __moliDomQuadBrand: true,
    __moliDomQuadP1: new DOMPoint()
  });
  return JSON.stringify({
    interfaceShape: [DOMQuad.length, DOMQuad.fromRect.length,
      DOMQuad.fromQuad.length, q.getBounds.length, q.toJSON.length,
      Object.prototype.toString.call(q), "bounds" in q].join("|"),
    prototypeKeys: Object.keys(DOMQuad.prototype).join(","),
    pointDescriptor: [typeof pointDescriptor.get, pointDescriptor.get.name,
      pointDescriptor.get.length, pointDescriptor.set === undefined,
      pointDescriptor.enumerable, pointDescriptor.configurable].join("|"),
    sameObject: q.p1 === q.p1,
    initial,
    live,
    copyInitial,
    copyAfterMutation,
    negativeValues,
    nanValues,
    jsonShape: [Object.getPrototypeOf(json) === Object.prototype,
      Object.keys(json).join(","), json.p1 === q.p1,
      JSON.stringify(json)].join("|"),
    forged: [probe(() => pointDescriptor.get.call(fake)),
      probe(() => DOMQuad.prototype.getBounds.call(fake)),
      probe(() => DOMQuad.prototype.toJSON.call(fake))].join(","),
    visibleSlots: Object.getOwnPropertyNames(q)
      .filter(name => name.startsWith("__moliDomQuad")).join(",")
  });
})()
"#,
        )
        .expect("DOMQuad geometry probe should evaluate");

    assert_eq!(
        result,
        concat!(
            r#"{"interfaceShape":"0|0|0|0|0|[object DOMQuad]|false","prototypeKeys":"p1,p2,p3,p4,getBounds,toJSON","pointDescriptor":"function|get p1|0|true|true|true","sameObject":true,"initial":"1,2,3,4,1,2,10,20","live":"true,-5,30,-5,2,16,28","copyInitial":"1,2,3,4,1,22","copyAfterMutation":"1,22,true","negativeValues":"10,20,-90,-180,-90,-180,100,200","nanValues":"NaN,0,NaN,0","jsonShape":"true|p1,p2,p3,p4|true|"#,
            r#"{\"p1\":{\"x\":-5,\"y\":2,\"z\":3,\"w\":4},\"p2\":{\"x\":11,\"y\":2,\"z\":0,\"w\":1},\"p3\":{\"x\":11,\"y\":22,\"z\":0,\"w\":1},\"p4\":{\"x\":1,\"y\":30,\"z\":0,\"w\":1}}","forged":"TypeError,TypeError,TypeError","visibleSlots":""}"#
        )
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
