use super::*;

#[test]
fn svg_enumeration_constants_have_webidl_descriptors() {
    let mut vm = new_storage_test_vm("https://svg-enumeration-constants.test/");
    let result = vm.eval(r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const groups = [
    [SVGUnitTypes, [["SVG_UNIT_TYPE_UNKNOWN", 0], ["SVG_UNIT_TYPE_USERSPACEONUSE", 1], ["SVG_UNIT_TYPE_OBJECTBOUNDINGBOX", 2]]],
    [SVGGradientElement, [["SVG_SPREADMETHOD_UNKNOWN", 0], ["SVG_SPREADMETHOD_PAD", 1], ["SVG_SPREADMETHOD_REFLECT", 2], ["SVG_SPREADMETHOD_REPEAT", 3]]],
  ];
  for (const [constructor, constants] of groups) {
    for (const owner of [constructor, constructor.prototype]) {
      for (const [name, value] of constants) {
        const descriptor = Object.getOwnPropertyDescriptor(owner, name);
        assert(descriptor && descriptor.value === value, name + " value");
        assert(descriptor.enumerable && !descriptor.configurable && !descriptor.writable, name + " descriptor");
      }
    }
    let caught;
    try { new constructor(); } catch (error) { caught = error; }
    assert(caught instanceof TypeError, constructor.name + " illegal constructor");
  }
  const gradient = document.createElementNS("http://www.w3.org/2000/svg", "linearGradient");
  assert(gradient.SVG_SPREADMETHOD_REPEAT === 3, "gradient inherits constants");
  assert(SVGLinearGradientElement.SVG_SPREADMETHOD_REPEAT === 3, "constructor inherits constants");
  return "ok";
})()
"#).expect("SVG enumeration constants should use WebIDL descriptors");
    assert_eq!(result, "ok");
}

#[test]
fn svg_specialized_accessors_live_on_owner_prototypes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-specialized-prototype.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on ${prototype.constructor?.name || "prototype"}`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} setter`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                return descriptor.get;
              };
              const assertNoOwn = (object, names, label) => {
                for (const name of names) {
                  assert(!own(object, name), `${label}.${name} should not be own before use`);
                  assert(!Object.keys(object).includes(name), `${label}.${name} should not be enumerable own`);
                }
              };
              const assertStaysInherited = (object, names, label) => {
                for (const name of names) {
                  const before = object[name];
                  assert(!own(object, name), `${label}.${name} should not be own after read`);
                  assert(delete object[name], `${label}.${name} delete`);
                  object[name] = "shadow";
                  assert(object[name] === before, `${label}.${name} after assignment`);
                  assert(!own(object, name), `${label}.${name} should not become own`);
                }
              };

              const ns = "http://www.w3.org/2000/svg";
              const rect = document.createElementNS(ns, "rect");
              const path = document.createElementNS(ns, "path");
              const text = document.createElementNS(ns, "text");
              const pattern = document.createElementNS(ns, "pattern");
              const linear = document.createElementNS(ns, "linearGradient");
              const radial = document.createElementNS(ns, "radialGradient");

              const transform = accessor(SVGGraphicsElement.prototype, "transform");
              const pathLength = accessor(SVGGeometryElement.prototype, "pathLength");
              const textLength = accessor(SVGTextContentElement.prototype, "textLength");
              const lengthAdjust = accessor(SVGTextContentElement.prototype, "lengthAdjust");
              const textX = accessor(SVGTextPositioningElement.prototype, "x");
              for (const name of ["y", "dx", "dy", "rotate"]) {
                accessor(SVGTextPositioningElement.prototype, name);
              }
              const patternTransform = accessor(SVGPatternElement.prototype, "patternTransform");
              const gradientTransform = accessor(SVGGradientElement.prototype, "gradientTransform");
              const rectX = accessor(SVGRectElement.prototype, "x");
              for (const name of ["y", "width", "height", "rx", "ry"]) {
                accessor(SVGRectElement.prototype, name);
              }

              assert(!own(SVGRectElement.prototype, "pathLength"), "pathLength inherited by SVGRectElement");
              assert(!own(SVGRectElement.prototype, "transform"), "transform inherited by SVGRectElement");
              assert(!own(SVGTextElement.prototype, "x"), "x inherited by SVGTextElement");
              assert(!own(SVGLinearGradientElement.prototype, "gradientTransform"),
                "gradientTransform inherited by SVGLinearGradientElement");

              assertNoOwn(rect, ["x", "y", "width", "height", "rx", "ry", "pathLength", "transform"], "rect");
              assertNoOwn(path, ["pathLength", "transform"], "path");
              assertNoOwn(text, ["textLength", "lengthAdjust", "x", "y", "dx", "dy", "rotate", "transform"], "text");
              assertNoOwn(pattern, ["patternTransform"], "pattern");
              assertNoOwn(linear, ["gradientTransform"], "linearGradient");
              assertNoOwn(radial, ["gradientTransform"], "radialGradient");

              rect.setAttribute("x", "13");
              path.setAttribute("pathLength", "7");
              text.setAttribute("x", "1 2");
              pattern.setAttribute("patternTransform", "translate(3)");
              linear.setAttribute("gradientTransform", "scale(2)");

              assert(rectX.call(rect) === rect.x, "rect x getter identity");
              assert(pathLength.call(path) === path.pathLength, "pathLength getter identity");
              assert(transform.call(rect) === rect.transform, "transform getter identity");
              assert(textLength.call(text) === text.textLength, "textLength getter identity");
              assert(lengthAdjust.call(text) === text.lengthAdjust, "lengthAdjust getter identity");
              assert(textX.call(text) === text.x, "text x getter identity");
              assert(patternTransform.call(pattern) === pattern.patternTransform, "patternTransform getter identity");
              assert(gradientTransform.call(linear) === linear.gradientTransform, "gradientTransform getter identity");

              assert(rect.x.baseVal.value === 13, "rect x reflects attribute");
              assert(path.pathLength.baseVal === 7, "pathLength reflects attribute");
              assert(text.x.baseVal.numberOfItems === 2, "text x reflects list");
              assert(pattern.patternTransform.baseVal.numberOfItems === 1, "pattern transform reflects list");
              assert(linear.gradientTransform.baseVal.numberOfItems === 1, "linear gradient transform reflects list");
              assert(radial.gradientTransform.baseVal.numberOfItems === 0, "radial gradient default transform");

              assertStaysInherited(rect, ["x", "y", "width", "height", "rx", "ry", "pathLength", "transform"], "rect");
              assertStaysInherited(text, ["textLength", "lengthAdjust", "x", "y", "dx", "dy", "rotate"], "text");
              assertStaysInherited(pattern, ["patternTransform"], "pattern");
              assertStaysInherited(linear, ["gradientTransform"], "linearGradient");
              return "ok";
            })()
            "#,
        )
        .expect("SVG specialized accessor prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_owner_svg_element_tracks_the_nearest_svg_fragment_root() {
    let mut vm = new_parsed_test_vm(
        "https://svg-owner-element.test/",
        r#"<!doctype html><html><body><div id="container">
          <svg id="outer">
            <g><circle id="circle"></circle><svg id="inner"><rect id="rect"></rect></svg></g>
            <foreignObject><svg id="foreign-svg"><svg id="foreign-inner"></svg></svg></foreignObject>
          </svg>
        </div></body></html>"#,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const outer = document.querySelector("#outer");
              const circle = document.querySelector("#circle");
              const inner = document.querySelector("#inner");
              const rect = document.querySelector("#rect");
              const foreignSvg = document.querySelector("#foreign-svg");
              const foreignInner = document.querySelector("#foreign-inner");

              assert(outer.ownerSVGElement === null, "outer SVG");
              assert(circle.ownerSVGElement === outer, "descendant of outer SVG");
              assert(inner.ownerSVGElement === outer, "nested SVG");
              assert(rect.ownerSVGElement === inner, "descendant of nested SVG");
              assert(foreignSvg.ownerSVGElement === null, "foreignObject starts a new SVG fragment");
              assert(foreignInner.ownerSVGElement === foreignSvg, "nested foreignObject SVG");

              const foreignObject = foreignSvg.parentNode;
              const foreignCircle = foreignObject.appendChild(document.createElementNS(ns, "circle"));
              const foreignGroup = foreignObject.appendChild(document.createElementNS(ns, "g"));
              const groupedSvg = foreignGroup.appendChild(document.createElementNS(ns, "svg"));
              const htmlParent = foreignObject.appendChild(document.createElement("div"));
              const htmlCircle = htmlParent.appendChild(document.createElementNS(ns, "circle"));
              const htmlSvg = htmlParent.appendChild(document.createElementNS(ns, "svg"));
              const shadow = htmlParent.attachShadow({mode: "open"});
              const shadowCircle = shadow.appendChild(document.createElementNS(ns, "circle"));
              const shadowSvg = shadow.appendChild(document.createElementNS(ns, "svg"));
              assert(foreignCircle.ownerSVGElement === outer, "non-root SVG crosses foreignObject");
              assert(groupedSvg.ownerSVGElement === outer, "only direct foreignObject roots are outermost");
              assert(htmlCircle.ownerSVGElement === outer, "non-root SVG crosses HTML ancestors");
              assert(htmlSvg.ownerSVGElement === null, "SVG with HTML parent is outermost");
              assert(shadowCircle.ownerSVGElement === outer, "non-root SVG resolves through a shadow host");
              assert(shadowSvg.ownerSVGElement === null, "SVG with a shadow root parent is outermost");

              document.querySelector("#container").remove();
              assert(circle.ownerSVGElement === outer, "detached outer SVG descendant");
              assert(inner.ownerSVGElement === outer, "detached nested SVG");
              assert(rect.ownerSVGElement === inner, "detached nested SVG descendant");
              assert(htmlCircle.ownerSVGElement === outer, "detached non-root SVG crosses HTML ancestors");
              assert(shadowCircle.ownerSVGElement === outer, "detached shadow SVG descendant");

              const standalone = document.createElementNS(ns, "ellipse");
              assert(standalone.ownerSVGElement === null, "standalone SVG element");

              const svgDocument = document.implementation.createDocument(ns, "svg", null);
              const documentRoot = svgDocument.documentElement;
              const documentRect = svgDocument.createElementNS(ns, "rect");
              documentRoot.append(documentRect);
              assert(documentRoot.ownerSVGElement === null, "SVG document root");
              assert(documentRect.ownerSVGElement === documentRoot, "SVG document child");

              const descriptor = Object.getOwnPropertyDescriptor(
                SVGElement.prototype,
                "ownerSVGElement",
              );
              assert(typeof descriptor.get === "function", "prototype getter");
              assert(descriptor.enumerable && descriptor.configurable, "getter flags");
              let incompatibleReceiver = false;
              try {
                descriptor.get.call(document.body);
              } catch (error) {
                incompatibleReceiver = error instanceof TypeError;
              }
              assert(incompatibleReceiver, "incompatible receiver");
              return "ok";
            })()
            "##,
        )
        .expect("SVG ownerSVGElement probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_geometry_queries_use_computed_paths_live_tree_and_kurbo_bounds() {
    let mut vm = new_parsed_test_vm(
        "https://svg-geometry-wiring.test/",
        r##"<!doctype html>
        <style>#css_path { d: path("M 0 0 t 0 100"); }</style>
        <svg xmlns="http://www.w3.org/2000/svg">
          <defs><rect id="definition" x="10" y="20" width="30" height="40"/></defs>
          <path id="css_path"/>
          <circle id="circle" cx="50" cy="50" r="5"/>
          <ellipse id="ellipse_rx_invalid" cx="1" cy="12" rx="-5" ry="10"/>
          <ellipse id="ellipse_ry_invalid" cx="6" cy="2" rx="5" ry="-10"/>
          <g id="aggregate">
            <path id="move" d="M 40 20 h0"/>
            <rect id="aggregate_rect" x="50" y="50" width="50" height="50"/>
          </g>
          <g id="child_transform"><rect x="1" y="2" width="3" height="4" transform="translate(10 20)"/></g>
          <g id="own_transform" transform="translate(100 200)"><rect x="1" y="2" width="3" height="4"/></g>
          <g style="display:none"><rect id="hidden_child" x="10" y="20" width="30" height="40"/></g>
          <rect id="hidden" x="10" y="20" width="30" height="40" display="none"/>
          <image id="image_box" x="2" y="3" width="4" height="5"/>
          <foreignObject id="foreign_box" x="6" y="7" width="8" height="9"/>
          <use id="use_box" href="#definition" x="5" y="7"/>
          <text y="180" font-size="100" font-family="Ahem" transform="translate(0 -100)">X<tspan id="span">X</tspan></text>
        </svg>"##,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const byId = id => document.getElementById(id);
              const clean = value => Math.abs(value - Math.round(value)) < 1e-12
                ? Math.round(value)
                : value;
              const box = id => {
                const value = byId(id).getBBox();
                return [value.x, value.y, value.width, value.height].map(clean);
              };
              const before = box("aggregate");
              const rect = byId("aggregate_rect");
              rect.remove();
              const afterRemove = box("aggregate");
              byId("aggregate").appendChild(rect);
              return JSON.stringify({
                cssLength: byId("css_path").getTotalLength(),
                circleRadius: byId("circle").r.baseVal.value,
                circleLength: Math.round(byId("circle").getTotalLength()),
                move: box("move"),
                aggregate: before,
                afterRemove,
                afterAppend: box("aggregate"),
                childTransform: box("child_transform"),
                ownTransform: box("own_transform"),
                hidden: box("hidden"),
                hiddenChild: box("hidden_child"),
                definition: box("definition"),
                image: box("image_box"),
                foreign: box("foreign_box"),
                use: box("use_box"),
                ellipseRxInvalid: box("ellipse_rx_invalid"),
                ellipseRyInvalid: box("ellipse_ry_invalid"),
                tspan: box("span"),
                foreignInterface: byId("foreign_box") instanceof SVGForeignObjectElement,
                tspanInterface: byId("span") instanceof SVGTSpanElement,
                tspanHasBBox: typeof byId("span").getBBox === "function"
              });
            })()
            "#,
        )
        .expect("SVG geometry wiring probe should evaluate");

    assert_eq!(
        result,
        r#"{"cssLength":100,"circleRadius":5,"circleLength":31,"move":[40,20,0,0],"aggregate":[40,20,60,80],"afterRemove":[40,20,0,0],"afterAppend":[40,20,60,80],"childTransform":[11,22,3,4],"ownTransform":[1,2,3,4],"hidden":[0,0,0,0],"hiddenChild":[0,0,0,0],"definition":[0,0,0,0],"image":[2,3,4,5],"foreign":[6,7,8,9],"use":[15,27,30,40],"ellipseRxInvalid":[-9,2,20,20],"ellipseRyInvalid":[1,-3,10,10],"tspan":[100,100,100,100],"foreignInterface":true,"tspanInterface":true,"tspanHasBBox":true}"#,
    );
}

#[test]
fn svg_path_errors_keep_valid_prefix_and_positive_zero_length() {
    let mut vm = new_parsed_test_vm(
        "https://svg-path-error-handling.test/",
        r##"<!doctype html>
        <svg xmlns="http://www.w3.org/2000/svg">
          <path id="invalid" d="M 10 10 L 30 10 X 50 10"/>
          <path id="empty" d=""/>
          <path id="none" d="none"/>
          <path id="missing_move" d="L 20 20"/>
        </svg>"##,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const byId = id => document.getElementById(id);
              const invalid = byId("invalid");
              const end = invalid.getPointAtLength(invalid.getTotalLength());
              return JSON.stringify({
                length: invalid.getTotalLength(),
                end: [end.x, end.y],
                emptyIsPositiveZero: Object.is(byId("empty").getTotalLength(), 0),
                noneIsPositiveZero: Object.is(byId("none").getTotalLength(), 0),
                missingMoveIsPositiveZero: Object.is(
                  byId("missing_move").getTotalLength(),
                  0,
                ),
              });
            })()
            "#,
        )
        .expect("SVG path error handling probe should evaluate");

    assert_eq!(
        result,
        r#"{"length":20,"end":[30,10],"emptyIsPositiveZero":true,"noneIsPositiveZero":true,"missingMoveIsPositiveZero":true}"#,
    );
}

#[test]
fn element_methods_and_dataset_live_on_owner_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) {
                  throw new Error(message);
                }
              };
              const own = (object, name) =>
                Object.prototype.hasOwnProperty.call(object, name);
              const method = (prototype, name, length, enumerable) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.value === "function", `${name} method`);
                assert(descriptor.value.length === length, `${name} length`);
                assert(descriptor.enumerable === enumerable, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
                assert(descriptor.writable === true, `${name} writable`);
              };
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(!!descriptor, `${name} missing on prototype`);
                assert(typeof descriptor.get === "function", `${name} getter`);
                assert(descriptor.set === undefined, `${name} readonly`);
                assert(descriptor.enumerable === true, `${name} enumerable`);
                assert(descriptor.configurable === true, `${name} configurable`);
              };

              const elementMethods = new Map([
                ["getBoundingClientRect", 0],
                ["getClientRects", 0],
                ["hasAttribute", 1],
                ["hasAttributeNS", 2],
                ["getAttribute", 1],
                ["getAttributeNS", 2],
                ["setAttribute", 2],
                ["setAttributeNS", 3],
                ["removeAttribute", 1],
                ["removeAttributeNS", 2],
                ["closest", 1],
                ["getElementsByTagName", 1],
                ["getElementsByTagNameNS", 2],
                ["getElementsByClassName", 1],
                ["getAttributeNames", 0],
                ["hasAttributes", 0],
                ["toggleAttribute", 1]
              ]);
              for (const [name, length] of elementMethods) {
                method(Element.prototype, name, length, true);
              }
              method(Element.prototype, "getElementsByName", 1, false);

              const extendedMethods = new Map([
                ["getAttributeNode", 1],
                ["getAttributeNodeNS", 2],
                ["setAttributeNode", 1],
                ["setAttributeNodeNS", 1],
                ["removeAttributeNode", 1],
                ["insertAdjacentElement", 2],
                ["insertAdjacentText", 2],
                ["insertAdjacentHTML", 2],
                ["__moliInsertAdjacentNode", 0]
              ]);
              for (const [name, length] of extendedMethods) {
                method(Element.prototype, name, length, true);
              }

              const actionMethods = [
                "focus",
                "blur",
                "click",
                "showPopover",
                "hidePopover",
                "togglePopover"
              ];
              for (const name of actionMethods) {
                method(HTMLElement.prototype, name, 0, true);
                assert(!own(Element.prototype, name), `${name} should not live on Element.prototype`);
              }
              method(HTMLElement.prototype, "scrollIntoViewIfNeeded", 0, false);

              for (const prototype of [HTMLElement.prototype, SVGElement.prototype, MathMLElement.prototype]) {
                accessor(prototype, "dataset");
              }
              assert(!own(Element.prototype, "dataset"), "dataset duplicated on Element.prototype");

              const host = document.createElement("section");
              host.innerHTML = '<p id="child" class="item" name="field"></p>';
              const child = host.firstElementChild;

              for (const name of elementMethods.keys()) {
                assert(!own(child, name), `${name} should not be own on child`);
              }
              for (const name of Array.from(extendedMethods.keys()).concat(actionMethods, ["getElementsByName", "scrollIntoViewIfNeeded", "dataset"])) {
                assert(!own(child, name), `${name} should not be own on child`);
              }

              child.setAttribute("data-token", "one");
              child.setAttributeNS("urn:test", "t:flag", "yes");
              assert(child.getAttribute("data-token") === "one", "getAttribute behavior");
              assert(child.getAttributeNS("urn:test", "flag") === "yes", "getAttributeNS behavior");
              assert(child.hasAttribute("data-token") && child.hasAttributeNS("urn:test", "flag"), "hasAttribute behavior");
              assert(child.getAttributeNames().includes("data-token"), "getAttributeNames behavior");
              assert(child.hasAttributes(), "hasAttributes behavior");
              assert(child.toggleAttribute("hidden") === true && child.hasAttribute("hidden"), "toggleAttribute behavior");
              child.removeAttribute("hidden");
              child.removeAttributeNS("urn:test", "flag");
              assert(!child.hasAttribute("hidden") && !child.hasAttributeNS("urn:test", "flag"), "removeAttribute behavior");
              assert(host.getElementsByTagName("p").length === 1, "getElementsByTagName behavior");
              assert(host.getElementsByClassName("item").length === 1, "getElementsByClassName behavior");
              assert(host.getElementsByName("field").length === 1, "getElementsByName behavior");
              assert(child.closest("section") === host, "closest behavior");

              const attr = document.createAttribute("data-node");
              attr.value = "node";
              child.setAttributeNode(attr);
              assert(child.getAttributeNode("data-node") === attr, "attributeNode behavior");
              child.insertAdjacentText("beforeend", "txt");
              child.insertAdjacentHTML("beforeend", "<span></span>");
              assert(child.textContent === "txt" && child.lastElementChild.localName === "span", "insertAdjacent behavior");

              child.dataset.fooBar = "baz";
              assert(child.getAttribute("data-foo-bar") === "baz", "html dataset behavior");
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              assert(!own(svg, "dataset"), "dataset should not be own on svg");
              svg.dataset.iconName = "home";
              assert(svg.getAttribute("data-icon-name") === "home", "svg dataset behavior");

              const fragment = document.createDocumentFragment();
              const fragmentChild = document.createElement("a");
              fragmentChild.className = "fragment-link";
              fragmentChild.setAttribute("name", "fragment-name");
              fragmentChild.id = "fragment-id";
              fragment.appendChild(fragmentChild);
              method(Document.prototype, "getElementById", 1, false);
              method(DocumentFragment.prototype, "getElementById", 1, false);
              assert(!own(document, "getElementById"), "getElementById should not be own on document");
              assert(!own(fragment, "getElementById"), "getElementById should not be own on fragment");
              method(DocumentFragment.prototype, "getElementsByTagName", 1, false);
              method(ShadowRoot.prototype, "getElementsByTagName", 1, false);
              assert(!own(fragment, "getElementsByTagName"), "getElementsByTagName should not be own on fragment");
              assert(fragment.getElementById("fragment-id") === fragmentChild, "fragment getElementById behavior");
              assert(fragment.getElementsByTagName("a").length === 1, "fragment getElementsByTagName behavior");
              assert(fragment.getElementsByClassName("fragment-link").length === 1, "fragment getElementsByClassName behavior");
              assert(fragment.getElementsByName("fragment-name").length === 1, "fragment getElementsByName behavior");

              const shadowHost = document.createElement("div");
              const shadowRoot = shadowHost.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = '<a id="shadow-id" class="shadow-link" name="shadow-name"></a>';
              assert(!own(shadowRoot, "getElementById"), "getElementById should not be own on shadow root");
              assert(!own(shadowRoot, "getElementsByTagName"), "getElementsByTagName should not be own on shadow root");
              assert(shadowRoot.getElementById("shadow-id")?.localName === "a", "shadow getElementById behavior");
              assert(shadowRoot.getElementsByTagName("a").length === 1, "shadow getElementsByTagName behavior");
              assert(shadowRoot.getElementsByClassName("shadow-link").length === 1, "shadow getElementsByClassName behavior");
              assert(shadowRoot.getElementsByName("shadow-name").length === 1, "shadow getElementsByName behavior");

              return "ok";
            })()
            "#,
        )
        .expect("Element methods and dataset prototype probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_string_lists_reflect_conditional_processing_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-string-list.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r##"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const text = document.createElementNS(ns, "text");

              assert(typeof SVGStringList === "function", "SVGStringList constructor");
              let illegalConstructor = false;
              try {
                new SVGStringList();
              } catch (error) {
                illegalConstructor = error instanceof TypeError;
              }
              assert(illegalConstructor, "SVGStringList illegal constructor");

              for (const name of ["requiredExtensions", "systemLanguage"]) {
                const descriptor = Object.getOwnPropertyDescriptor(
                  SVGGraphicsElement.prototype,
                  name,
                );
                assert(typeof descriptor.get === "function" && descriptor.set === undefined,
                  `${name} readonly descriptor`);
                assert(descriptor.enumerable && descriptor.configurable, `${name} flags`);
              }
              for (const name of ["length", "numberOfItems"]) {
                const descriptor = Object.getOwnPropertyDescriptor(SVGStringList.prototype, name);
                assert(typeof descriptor.get === "function" && descriptor.set === undefined,
                  `${name} descriptor`);
                assert(descriptor.enumerable && descriptor.configurable, `${name} flags`);
              }
              const methodLengths = {
                clear: 0,
                initialize: 1,
                getItem: 1,
                insertItemBefore: 2,
                replaceItem: 2,
                removeItem: 1,
                appendItem: 1,
              };
              for (const [name, length] of Object.entries(methodLengths)) {
                const descriptor = Object.getOwnPropertyDescriptor(SVGStringList.prototype, name);
                assert(typeof descriptor.value === "function", `${name} method`);
                assert(descriptor.value.length === length, `${name} arity`);
                assert(descriptor.enumerable && descriptor.configurable && descriptor.writable,
                  `${name} flags`);
              }

              const languages = text.systemLanguage;
              assert(languages instanceof SVGStringList, "systemLanguage interface");
              assert(Object.prototype.toString.call(languages) === "[object SVGStringList]",
                "SVGStringList tag");
              assert(text.systemLanguage === languages, "systemLanguage SameObject");
              assert(languages.length === 0 && languages.numberOfItems === 0,
                "absent attribute empty list");
              assert(!text.hasAttribute("systemLanguage"), "getter does not create attribute");

              const parsingCases = [
                ["en,fr,de", ["en", "fr", "de"]],
                ["en, fr, de", ["en", "fr", "de"]],
                ["en ,fr ,de", ["en", "fr", "de"]],
                ["en , fr , de", ["en", "fr", "de"]],
                ["  en, fr  ", ["en", "fr"]],
                [" \t\nen, fr\t\n ", ["en", "fr"]],
                ["en", ["en"]],
                ["en-US, zh-Hans, pt-BR", ["en-US", "zh-Hans", "pt-BR"]],
                ["en,,fr", ["en", "", "fr"]],
                ["", [""]],
                [",", ["", ""]],
                ["123, 456", ["123", "456"]],
                ["not-a-lang, ???, @#$", ["not-a-lang", "???", "@#$"]],
              ];
              for (const [raw, expected] of parsingCases) {
                text.setAttribute("systemLanguage", raw);
                assert(text.systemLanguage === languages, `SameObject after ${raw}`);
                assert(languages.length === expected.length, `length for ${raw}`);
                assert(languages.numberOfItems === expected.length,
                  `numberOfItems for ${raw}`);
                assert(Object.keys(languages).join() === expected.map((_, index) => index).join(),
                  `supported indices for ${raw}`);
                for (let index = 0; index < expected.length; index++) {
                  assert(languages.getItem(index) === expected[index],
                    `getItem ${index} for ${raw}`);
                  assert(languages[index] === expected[index], `index ${index} for ${raw}`);
                }
              }

              text.removeAttribute("systemLanguage");
              assert(languages.length === 0, "removed attribute empty list");
              const extensions = text.requiredExtensions;
              assert(text.requiredExtensions === extensions, "requiredExtensions SameObject");
              text.setAttribute("requiredExtensions", "  one\t two\nthree  ");
              assert(extensions.length === 3 && extensions[0] === "one" &&
                extensions[1] === "two" && extensions[2] === "three",
                "requiredExtensions space-separated parsing");

              assert(languages.initialize("en") === "en", "initialize return");
              assert(text.getAttribute("systemLanguage") === "en", "initialize reflection");
              assert(languages.appendItem("fr") === "fr", "append return");
              assert(text.getAttribute("systemLanguage") === "en,fr", "append reflection");
              assert(languages.insertItemBefore("de", 1) === "de", "insert return");
              assert(text.getAttribute("systemLanguage") === "en,de,fr", "insert reflection");
              assert(languages.insertItemBefore("it", 99) === "it", "clamped insert return");
              assert(text.getAttribute("systemLanguage") === "en,de,fr,it",
                "clamped insert reflection");
              assert(languages.replaceItem("zh", 1) === "zh", "replace return");
              assert(text.getAttribute("systemLanguage") === "en,zh,fr,it",
                "replace reflection");
              languages[2] = "pt-BR";
              assert(text.getAttribute("systemLanguage") === "en,zh,pt-BR,it",
                "indexed setter reflection");
              Object.defineProperty(languages, "0", {value: "es"});
              assert(text.getAttribute("systemLanguage") === "es,zh,pt-BR,it",
                "indexed definer reflection");
              const indexDescriptor = Object.getOwnPropertyDescriptor(languages, "0");
              assert(indexDescriptor.value === "es" && indexDescriptor.writable &&
                indexDescriptor.enumerable && indexDescriptor.configurable,
                "indexed property descriptor");
              assert(delete languages[0] === false, "supported index cannot be deleted");
              assert(languages.removeItem(1) === "zh", "remove return");
              assert(text.getAttribute("systemLanguage") === "es,pt-BR,it",
                "remove reflection");
              languages.clear();
              assert(text.getAttribute("systemLanguage") === "" && languages.length === 0,
                "clear reflection");

              extensions.initialize("alpha");
              extensions.appendItem("beta");
              assert(text.getAttribute("requiredExtensions") === "alpha beta",
                "requiredExtensions space serialization");
              assert(extensions.appendItem(null) === "null", "DOMString conversion");
              assert(text.getAttribute("requiredExtensions") === "alpha beta null",
                "converted string reflection");

              text.setAttribute("systemLanguage", "en,fr");
              for (const operation of [
                () => languages.getItem(9),
                () => languages.replaceItem("x", 9),
                () => languages.removeItem(9),
                () => { languages[9] = "x"; },
              ]) {
                let indexError = false;
                try {
                  operation();
                } catch (error) {
                  indexError = error instanceof DOMException && error.name === "IndexSizeError";
                }
                assert(indexError, "out-of-range operation");
              }

              let incompatibleListReceiver = false;
              try {
                SVGStringList.prototype.getItem.call({}, 0);
              } catch (error) {
                incompatibleListReceiver = error instanceof TypeError;
              }
              assert(incompatibleListReceiver, "SVGStringList receiver brand");

              const systemLanguageGetter = Object.getOwnPropertyDescriptor(
                SVGGraphicsElement.prototype,
                "systemLanguage",
              ).get;
              let incompatibleElementReceiver = false;
              try {
                systemLanguageGetter.call(document.createElementNS(ns, "filter"));
              } catch (error) {
                incompatibleElementReceiver = error instanceof TypeError;
              }
              assert(incompatibleElementReceiver, "SVGGraphicsElement receiver brand");
              return "ok";
            })()
            "##,
        )
        .expect("SVG string list probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_string_lists_use_native_identity_in_windowless_and_child_documents() {
    let mut vm = new_parsed_test_vm(
        "https://svg-string-list-identity.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      for (const realm of [window, child]) {
        for (const doc of [realm.document, realm.document.implementation.createHTMLDocument('')]) {
          const rect = doc.createElementNS(ns, 'rect');
          const getter = Object.getOwnPropertyDescriptor(window.SVGGraphicsElement.prototype, 'systemLanguage').get;
          const list = getter.call(rect);
          check(list instanceof realm.SVGStringList, 'producer realm');
          check(list === rect.systemLanguage && list.length === 0, 'retained native identity');
          rect.setAttribute('systemLanguage', 'en, fr');
          check(list.length === 2 && list[0] === 'en' && list[1] === 'fr', 'attribute synchronization');
          list.appendItem('ja');
          check(rect.getAttribute('systemLanguage') === 'en,fr,ja', 'windowless native reflection');
          rect.removeAttribute('systemLanguage');
          check(list.length === 0, 'removed attribute default');
          const extensions = rect.requiredExtensions;
          extensions.appendItem('urn:test');
          check(rect.getAttribute('requiredExtensions') === 'urn:test', 'extension reflection');
        }
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn svg_string_list_conversion_precedes_state_reads_and_preserves_argument_order() {
    let mut vm = new_parsed_test_vm(
        "https://svg-string-list-conversion.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      const error = f => { try { f(); } catch (e) { return e; } };
      for (const realm of [window, child]) {
        const rect = realm.document.createElementNS(ns, 'rect');
        const list = rect.systemLanguage;
        for (const method of ['replaceItem', 'insertItemBefore']) {
          rect.setAttribute('systemLanguage', 'en,fr');
          const calls = [];
          list[method]({toString() { calls.push('item'); return 'ja'; }},
            {valueOf() { calls.push('index'); return 0; }});
          check(calls.join() === 'item,index', 'DOMString precedes index conversion');
          const sentinel = new realm.Error('conversion');
          check(error(() => list[method]({toString() { throw sentinel; }},
            {valueOf() { calls.push('bad index'); return 0; }})) === sentinel, 'conversion exception');
          check(!calls.includes('bad index'), 'throwing item prevents index conversion');
        }
        rect.setAttribute('systemLanguage', 'en');
        check(list.getItem({valueOf() { rect.setAttribute('systemLanguage', 'ja,de'); return 1; }}) === 'de', 'getItem reads after index conversion');
        check(list.removeItem({valueOf() { rect.setAttribute('systemLanguage', 'fr,it'); return 1; }}) === 'it', 'removeItem reads after index conversion');
        check(rect.getAttribute('systemLanguage') === 'fr', 'remove conversion reflection');
        list.appendItem({toString() { rect.setAttribute('systemLanguage', 'ja,de'); return 'es'; }});
        check(rect.getAttribute('systemLanguage') === 'ja,de,es', 'append reads after string conversion');
        list.replaceItem({toString() { rect.setAttribute('systemLanguage', 'fr,it'); return 'en'; }}, 1);
        check(rect.getAttribute('systemLanguage') === 'fr,en', 'replace reads after string conversion');
        list[0] = {toString() { rect.setAttribute('systemLanguage', 'ja,de'); return 'fr'; }};
        check(rect.getAttribute('systemLanguage') === 'fr,de', 'indexed setter reads after string conversion');
        let conversions = 0;
        const item = {toString() { conversions++; return 'es'; }};
        const index = {valueOf() { conversions++; return 0; }};
        for (const receiver of [{}, Object.create(list), new Proxy(list, {})]) {
          check(error(() => realm.SVGStringList.prototype.replaceItem.call(receiver, item, index)) instanceof realm.TypeError, 'invalid receiver');
        }
        check(conversions === 0, 'brand validation precedes conversion');
      }
      return true;
    })()"#).unwrap(), "true");
}

#[test]
fn svg_string_lists_do_not_observe_array_prototype_accessors() {
    let mut vm = new_parsed_test_vm(
        "https://svg-string-list-storage.test/",
        "<!doctype html><html><body><iframe></iframe></body></html>",
    );
    assert_eq!(vm.eval(r#"(() => {
      const ns = 'http://www.w3.org/2000/svg';
      const child = document.querySelector('iframe').contentWindow;
      const check = (ok, label) => { if (!ok) throw Error(label); };
      for (const realm of [window, child]) {
        const rect = realm.document.createElementNS(ns, 'rect');
        const list = rect.systemLanguage;
        const prototype = realm.Array.prototype;
        const previous = Object.getOwnPropertyDescriptor(prototype, '0');
        let accesses = 0;
        Object.defineProperty(prototype, '0', {
          configurable: true,
          get() { accesses++; return 'polluted'; },
          set(value) { accesses++; },
        });
        try {
          check(list.initialize('en') === 'en' && list[0] === 'en', 'initialize');
          check(list.appendItem('fr') === 'fr', 'append');
          check(list.insertItemBefore('ja', 0) === 'ja' && list[0] === 'ja', 'insert');
          check(list.removeItem(0) === 'ja' && list[0] === 'en', 'remove');
          check(rect.getAttribute('systemLanguage') === 'en,fr', 'mutation reflection');
          list.clear();
          list.appendItem('de');
          check(list.length === 1 && list[0] === 'de', 'append to empty list');
          rect.setAttribute('systemLanguage', 'es,it');
          check(list.length === 2 && list[0] === 'es' && list[1] === 'it', 'attribute synchronization');
          check(accesses === 0, 'private list storage must not invoke author accessors');
        } finally {
          if (previous) Object.defineProperty(prototype, '0', previous);
          else delete prototype[0];
        }
      }
      return true;
    })()"#).unwrap(), "true");
}
