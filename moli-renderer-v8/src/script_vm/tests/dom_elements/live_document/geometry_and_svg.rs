use super::*;

#[test]
#[ignore = "requires native DOM attribute UTF-16 storage migration"]
fn svg_animated_string_attributes_preserve_lone_surrogates() {
    let mut vm = new_parsed_test_vm("https://svg-utf16.test/", "<body></body>");
    assert_eq!(
        vm.eval(r#"(() => {
            const value = 'before\uD800after';
            for (const [tag, property, attribute] of [['feTile', 'in1', 'in'], ['mpath', 'href', 'href']]) {
                const element = document.createElementNS('http://www.w3.org/2000/svg', tag);
                const animated = element[property];
                animated.baseVal = value;
                if (animated.baseVal !== value || animated.animVal !== value || element.getAttribute(attribute) !== value) return false;
            }
            return true;
        })()"#).unwrap(),
        "true"
    );
}

#[test]
fn svg_animated_lengths_reflect_initial_and_content_attribute_values() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-length-reflection.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const cases = [
                ["filter", SVGFilterElement, [
                  ["x", "-10%"], ["y", "-10%"],
                  ["width", "120%"], ["height", "120%"],
                ]],
                ["feBlend", SVGFEBlendElement, [
                  ["x", "0%"], ["y", "0%"],
                  ["width", "100%"], ["height", "100%"],
                ]],
                ["linearGradient", SVGLinearGradientElement, [
                  ["x1", "0%"], ["y1", "0%"], ["x2", "100%"], ["y2", "0%"],
                ]],
                ["marker", SVGMarkerElement, [
                  ["refX", "0"], ["refY", "0"],
                  ["markerWidth", "3"], ["markerHeight", "3"],
                ]],
                ["mask", SVGMaskElement, [
                  ["x", "-10%"], ["y", "-10%"],
                  ["width", "120%"], ["height", "120%"],
                ]],
                ["pattern", SVGPatternElement, [
                  ["x", "0"], ["y", "0"], ["width", "0"], ["height", "0"],
                ]],
                ["radialGradient", SVGRadialGradientElement, [
                  ["cx", "50%"], ["cy", "50%"], ["r", "50%"],
                  ["fx", "50%"], ["fy", "50%"], ["fr", "0%"],
                ]],
                ["svg", SVGSVGElement, [
                  ["x", "0"], ["y", "0"], ["width", "100%"], ["height", "100%"],
                ]],
                ["text", SVGTextContentElement, [["textLength", "0"]]],
                ["textPath", SVGTextPathElement, [["startOffset", "0"]]],
              ];

              for (const [tag, owner, attributes] of cases) {
                const element = document.createElementNS(ns, tag);
                for (const [name, initial] of attributes) {
                  const descriptor = Object.getOwnPropertyDescriptor(owner.prototype, name);
                  assert(typeof descriptor?.get === "function", `${owner.name}.${name} getter`);
                  assert(descriptor.set === undefined, `${owner.name}.${name} setter`);
                  assert(descriptor.enumerable && descriptor.configurable,
                    `${owner.name}.${name} flags`);

                  const animated = element[name];
                  assert(animated instanceof SVGAnimatedLength, `${tag}.${name} interface`);
                  assert(element[name] === animated, `${tag}.${name} SameObject`);
                  assert(animated.baseVal.valueAsString === initial,
                    `${tag}.${name} initial baseVal`);
                  assert(animated.animVal.valueAsString === initial,
                    `${tag}.${name} initial animVal`);

                  element.setAttribute(name, "42");
                  assert(element[name] === animated, `${tag}.${name} SameObject after set`);
                  assert(animated.baseVal.valueAsString === "42" &&
                    animated.animVal.valueAsString === "42", `${tag}.${name} content update`);

                  element.setAttribute(name, "foobar");
                  assert(element[name] === animated, `${tag}.${name} SameObject after invalid`);
                  assert(animated.baseVal.valueAsString === initial &&
                    animated.animVal.valueAsString === initial, `${tag}.${name} invalid fallback`);

                  element.removeAttribute(name);
                  assert(element[name] === animated, `${tag}.${name} SameObject after remove`);
                  assert(animated.baseVal.valueAsString === initial &&
                    animated.animVal.valueAsString === initial, `${tag}.${name} removed fallback`);
                }
              }
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated length reflection probe should evaluate");

    assert_eq!(result, "ok");
}

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
fn svg_historical_interfaces_keep_current_element_constructors_only() {
    let mut vm = new_parsed_test_vm(
        "https://svg-historical-interfaces.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const interfaces = [
                ["clipPath", "SVGClipPathElement"],
                ["filter", "SVGFilterElement"],
                ["mask", "SVGMaskElement"],
                ["view", "SVGViewElement"],
              ];
              const removedUnitTypeConstants = [
                "SVG_UNIT_TYPE_UNKNOWN",
                "SVG_UNIT_TYPE_USERSPACEONUSE",
                "SVG_UNIT_TYPE_OBJECTBOUNDINGBOX",
              ];

              for (const [localName, interfaceName] of interfaces) {
                const constructor = globalThis[interfaceName];
                assert(typeof constructor === "function", `${interfaceName} constructor`);
                assert(Object.getPrototypeOf(constructor.prototype) === SVGElement.prototype,
                  `${interfaceName} prototype parent`);
                assert(document.createElementNS(ns, localName) instanceof constructor,
                  `${localName} interface`);
                for (const constant of removedUnitTypeConstants) {
                  assert(!(constant in constructor), `${interfaceName}.${constant} removed`);
                }
              }

              assert(!("viewTarget" in SVGViewElement.prototype),
                "SVGViewElement.prototype.viewTarget removed");
              return "ok";
            })()
            "#,
        )
        .expect("SVG historical interface probe should evaluate");

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

              document.querySelector("#container").remove();
              assert(circle.ownerSVGElement === outer, "detached outer SVG descendant");
              assert(inner.ownerSVGElement === outer, "detached nested SVG");
              assert(rect.ownerSVGElement === inner, "detached nested SVG descendant");

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
fn svg_historical_interfaces_keep_current_element_constructors_only() {
    let mut vm = new_parsed_test_vm(
        "https://svg-historical-interfaces.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const interfaces = [
                ["clipPath", "SVGClipPathElement"],
                ["filter", "SVGFilterElement"],
                ["mask", "SVGMaskElement"],
                ["view", "SVGViewElement"],
              ];
              const removedUnitTypeConstants = [
                "SVG_UNIT_TYPE_UNKNOWN",
                "SVG_UNIT_TYPE_USERSPACEONUSE",
                "SVG_UNIT_TYPE_OBJECTBOUNDINGBOX",
              ];

              for (const [localName, interfaceName] of interfaces) {
                const constructor = globalThis[interfaceName];
                assert(typeof constructor === "function", `${interfaceName} constructor`);
                assert(Object.getPrototypeOf(constructor.prototype) === SVGElement.prototype,
                  `${interfaceName} prototype parent`);
                assert(document.createElementNS(ns, localName) instanceof constructor,
                  `${localName} interface`);
                for (const constant of removedUnitTypeConstants) {
                  assert(!(constant in constructor), `${interfaceName}.${constant} removed`);
                }
              }

              assert(!("viewTarget" in SVGViewElement.prototype),
                "SVGViewElement.prototype.viewTarget removed");
              return "ok";
            })()
            "#,
        )
        .expect("SVG historical interface probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_animated_enumerations_reflect_typed_content_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-enumerations.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const unitUser = SVGUnitTypes.SVG_UNIT_TYPE_USERSPACEONUSE;
              const unitBox = SVGUnitTypes.SVG_UNIT_TYPE_OBJECTBOUNDINGBOX;
              const cases = [
                ["clipPath", "SVGClipPathElement", "clipPathUnits", unitUser, unitBox, "objectBoundingBox", 3],
                ["filter", "SVGFilterElement", "filterUnits", unitBox, unitUser, "userSpaceOnUse", 3],
                ["filter", "SVGFilterElement", "primitiveUnits", unitUser, unitBox, "objectBoundingBox", 3],
                ["linearGradient", "SVGGradientElement", "gradientUnits", unitBox, unitUser, "userSpaceOnUse", 3],
                ["linearGradient", "SVGGradientElement", "spreadMethod", SVGGradientElement.SVG_SPREADMETHOD_PAD, SVGGradientElement.SVG_SPREADMETHOD_REPEAT, "repeat", 4],
                ["mask", "SVGMaskElement", "maskUnits", unitBox, unitUser, "userSpaceOnUse", 3],
                ["mask", "SVGMaskElement", "maskContentUnits", unitUser, unitBox, "objectBoundingBox", 3],
                ["pattern", "SVGPatternElement", "patternUnits", unitBox, unitUser, "userSpaceOnUse", 3],
                ["pattern", "SVGPatternElement", "patternContentUnits", unitUser, unitBox, "objectBoundingBox", 3],
                ["text", "SVGTextContentElement", "lengthAdjust", SVGTextContentElement.LENGTHADJUST_SPACING, SVGTextContentElement.LENGTHADJUST_SPACINGANDGLYPHS, "spacingAndGlyphs", 3],
              ];

              assert(typeof SVGUnitTypes === "function", "SVGUnitTypes interface");
              for (const [name, value] of [
                ["SVG_UNIT_TYPE_UNKNOWN", 0],
                ["SVG_UNIT_TYPE_USERSPACEONUSE", 1],
                ["SVG_UNIT_TYPE_OBJECTBOUNDINGBOX", 2],
              ]) {
                assert(SVGUnitTypes[name] === value, `SVGUnitTypes.${name}`);
                assert(SVGUnitTypes.prototype[name] === value, `SVGUnitTypes.prototype.${name}`);
              }
              let illegalConstructor = false;
              try {
                new SVGUnitTypes();
              } catch (error) {
                illegalConstructor = error instanceof TypeError;
              }
              assert(illegalConstructor, "SVGUnitTypes illegal constructor");

              for (const [tag, interfaceName, property, initial, alternate, serialized, invalid] of cases) {
                const element = document.createElementNS(ns, tag);
                const animated = element[property];
                assert(animated instanceof SVGAnimatedEnumeration, `${property} interface`);
                assert(element[property] === animated, `${property} SameObject`);
                assert(animated.baseVal === initial && animated.animVal === initial,
                  `${property} initial value`);

                element.setAttribute(property, serialized);
                assert(animated.baseVal === alternate && animated.animVal === alternate,
                  `${property} content attribute update`);
                element.setAttribute(property, "invalid");
                assert(animated.baseVal === initial, `${property} invalid content attribute`);
                element.removeAttribute(property);
                assert(animated.baseVal === initial, `${property} removed content attribute`);

                animated.baseVal = alternate;
                assert(element.getAttribute(property) === serialized, `${property} reflected value`);
                assert(animated.baseVal === alternate && animated.animVal === alternate,
                  `${property} reflected enumeration`);
                let rejected = false;
                try {
                  animated.baseVal = invalid;
                } catch (error) {
                  rejected = error instanceof TypeError;
                }
                assert(rejected, `${property} invalid IDL value`);
                assert(animated.baseVal === alternate, `${property} preserved after rejection`);

                const owner = globalThis[interfaceName].prototype;
                const descriptor = Object.getOwnPropertyDescriptor(owner, property);
                assert(typeof descriptor.get === "function", `${interfaceName}.${property} getter`);
                assert(descriptor.enumerable && descriptor.configurable,
                  `${interfaceName}.${property} flags`);
              }
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated enumeration probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_animated_boolean_reflects_preserve_alpha() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-boolean.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const element = document.createElementNS(ns, "feConvolveMatrix");

              assert(typeof SVGAnimatedBoolean === "function", "animated boolean constructor");
              assert(typeof SVGFEConvolveMatrixElement === "function", "element constructor");
              assert(element instanceof SVGFEConvolveMatrixElement, "element interface");
              assert(Object.getPrototypeOf(SVGFEConvolveMatrixElement.prototype) === SVGElement.prototype,
                "element prototype parent");

              for (const constructor of [SVGAnimatedBoolean, SVGFEConvolveMatrixElement]) {
                let illegalConstructor = false;
                try {
                  new constructor();
                } catch (error) {
                  illegalConstructor = error instanceof TypeError;
                }
                assert(illegalConstructor, `${constructor.name} illegal constructor`);
              }

              const elementDescriptor = Object.getOwnPropertyDescriptor(
                SVGFEConvolveMatrixElement.prototype,
                "preserveAlpha",
              );
              assert(typeof elementDescriptor.get === "function", "preserveAlpha getter");
              assert(elementDescriptor.set === undefined, "preserveAlpha readonly");
              assert(elementDescriptor.enumerable && elementDescriptor.configurable,
                "preserveAlpha descriptor flags");

              const animated = element.preserveAlpha;
              assert(animated instanceof SVGAnimatedBoolean, "animated boolean interface");
              assert(Object.prototype.toString.call(animated) === "[object SVGAnimatedBoolean]",
                "animated boolean tag");
              assert(element.preserveAlpha === animated, "preserveAlpha SameObject");
              assert(!element.hasAttribute("preserveAlpha"), "getter does not create attribute");
              assert(animated.baseVal === false && animated.animVal === false, "initial value");

              const baseDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedBoolean.prototype,
                "baseVal",
              );
              const animDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedBoolean.prototype,
                "animVal",
              );
              assert(typeof baseDescriptor.get === "function" &&
                typeof baseDescriptor.set === "function", "baseVal descriptor");
              assert(typeof animDescriptor.get === "function" && animDescriptor.set === undefined,
                "animVal descriptor");
              assert(baseDescriptor.enumerable && baseDescriptor.configurable &&
                animDescriptor.enumerable && animDescriptor.configurable,
                "animated boolean descriptor flags");

              element.setAttribute("preserveAlpha", "true");
              assert(animated.baseVal === true && animated.animVal === true,
                "true content attribute");
              element.setAttribute("preserveAlpha", "false");
              assert(animated.baseVal === false && animated.animVal === false,
                "false content attribute");
              element.setAttribute("preserveAlpha", "TRUE");
              assert(animated.baseVal === false && animated.animVal === false,
                "invalid content attribute uses initial value");
              element.removeAttribute("preserveAlpha");
              assert(animated.baseVal === false && animated.animVal === false,
                "removed content attribute uses initial value");

              animated.baseVal = true;
              assert(element.getAttribute("preserveAlpha") === "true", "true reflection");
              assert(animated.baseVal === true && animated.animVal === true,
                "true reflected values");
              animated.baseVal = null;
              assert(element.getAttribute("preserveAlpha") === "false", "false reflection");
              assert(animated.baseVal === false && animated.animVal === false,
                "false reflected values");
              animated.baseVal = {};
              assert(element.getAttribute("preserveAlpha") === "true", "WebIDL ToBoolean");

              let incompatibleAnimatedReceiver = false;
              try {
                baseDescriptor.get.call({});
              } catch (error) {
                incompatibleAnimatedReceiver = error instanceof TypeError;
              }
              assert(incompatibleAnimatedReceiver, "animated boolean receiver brand");

              let incompatibleElementReceiver = false;
              try {
                elementDescriptor.get.call(document.createElementNS(ns, "rect"));
              } catch (error) {
                incompatibleElementReceiver = error instanceof TypeError;
              }
              assert(incompatibleElementReceiver, "element receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated boolean probe should evaluate");

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
fn svg_marker_orient_angle_is_a_live_animated_angle() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-angle.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const close = (actual, expected) => Math.abs(actual - expected) < 1e-9;
              const ns = "http://www.w3.org/2000/svg";
              const marker = document.createElementNS(ns, "marker");

              for (const constructor of [SVGMarkerElement, SVGAnimatedAngle, SVGAngle]) {
                assert(typeof constructor === "function", `${constructor.name} constructor`);
                let illegalConstructor = false;
                try {
                  new constructor();
                } catch (error) {
                  illegalConstructor = error instanceof TypeError;
                }
                assert(illegalConstructor, `${constructor.name} illegal constructor`);
              }
              assert(marker instanceof SVGMarkerElement, "marker interface");
              assert(Object.getPrototypeOf(SVGMarkerElement.prototype) === SVGElement.prototype,
                "marker prototype parent");

              const markerDescriptor = Object.getOwnPropertyDescriptor(
                SVGMarkerElement.prototype,
                "orientAngle",
              );
              assert(typeof markerDescriptor.get === "function" &&
                markerDescriptor.set === undefined, "orientAngle readonly descriptor");
              assert(markerDescriptor.enumerable && markerDescriptor.configurable,
                "orientAngle descriptor flags");

              const animated = marker.orientAngle;
              const base = animated.baseVal;
              const anim = animated.animVal;
              assert(animated instanceof SVGAnimatedAngle, "animated angle interface");
              assert(base instanceof SVGAngle && anim instanceof SVGAngle, "angle interfaces");
              assert(base !== anim, "base and animated values are distinct");
              assert(marker.orientAngle === animated, "orientAngle SameObject");
              assert(base.value === 0 && anim.value === 0, "initial values");
              assert(base.unitType === SVGAngle.SVG_ANGLETYPE_UNSPECIFIED, "initial unit");
              assert(!marker.hasAttribute("orient"), "getter does not create attribute");
              assert(marker.orientType instanceof SVGAnimatedEnumeration, "orientType interface");
              assert(marker.orientType.baseVal === SVGMarkerElement.SVG_MARKER_ORIENT_ANGLE,
                "initial orientType");
              assert(marker.markerUnits instanceof SVGAnimatedEnumeration,
                "markerUnits interface");
              assert(marker.markerUnits.baseVal ===
                SVGMarkerElement.SVG_MARKERUNITS_STROKEWIDTH, "initial markerUnits");
              marker.markerUnits.baseVal = SVGMarkerElement.SVG_MARKERUNITS_USERSPACEONUSE;
              assert(marker.getAttribute("markerUnits") === "userSpaceOnUse",
                "markerUnits reflection");

              base.value = 100;
              assert(base.value === 100 && marker.orientAngle.baseVal.value === 100,
                "cached base value is live");
              assert(marker.getAttribute("orient") === "100", "value reflects to orient");
              marker.orientAngle.baseVal = -1;
              assert(marker.orientAngle.baseVal === base && base.value === 100,
                "baseVal assignment is ignored");

              marker.setAttribute("orient", "1.5707963267948966rad");
              assert(close(base.value, 90), "content attribute converts to degrees");
              assert(base.unitType === SVGAngle.SVG_ANGLETYPE_RAD, "content attribute unit");
              assert(close(base.valueInSpecifiedUnits, Math.PI / 2), "specified value");
              assert(anim.unitType === SVGAngle.SVG_ANGLETYPE_RAD && close(anim.value, 90),
                "animVal tracks content attribute");
              assert(marker.orientType.baseVal === SVGMarkerElement.SVG_MARKER_ORIENT_ANGLE,
                "angle orientType");

              marker.setOrientToAuto();
              assert(marker.getAttribute("orient") === "auto", "setOrientToAuto reflection");
              assert(marker.orientType.baseVal === SVGMarkerElement.SVG_MARKER_ORIENT_AUTO,
                "automatic orientType");
              assert(base.value === 0 && base.unitType === SVGAngle.SVG_ANGLETYPE_UNSPECIFIED,
                "automatic orient angle");

              marker.setAttribute("orient", "400grad");
              assert(base.value === 360 && base.unitType === SVGAngle.SVG_ANGLETYPE_GRAD,
                "grad content attribute");

              const svg = document.createElementNS(ns, "svg");
              const standalone = svg.createSVGAngle();
              assert(standalone instanceof SVGAngle, "createSVGAngle result");
              standalone.newValueSpecifiedUnits(SVGAngle.SVG_ANGLETYPE_RAD, Math.PI);
              assert(close(standalone.value, 180), "newValueSpecifiedUnits conversion");
              standalone.convertToSpecifiedUnits(SVGAngle.SVG_ANGLETYPE_GRAD);
              assert(close(standalone.valueInSpecifiedUnits, 200), "unit conversion");
              assert(standalone.valueAsString === "200grad", "unit serialization");
              marker.setOrientToAngle(standalone);
              assert(marker.getAttribute("orient") === "200grad", "setOrientToAngle reflection");
              assert(marker.orientAngle.baseVal.value === 180, "setOrientToAngle value");

              let readonlyAnim = false;
              try {
                anim.value = 1;
              } catch (error) {
                readonlyAnim = error.name === "NoModificationAllowedError";
              }
              assert(readonlyAnim, "animVal is read-only");

              let incompatibleReceiver = false;
              try {
                markerDescriptor.get.call(document.createElementNS(ns, "rect"));
              } catch (error) {
                incompatibleReceiver = error instanceof TypeError;
              }
              assert(incompatibleReceiver, "marker receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated angle probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_filter_and_text_path_enumerations_reflect_typed_content_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-filter-enumerations.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const interfaceCases = [
                ["feBlend", SVGFEBlendElement, SVGElement],
                ["feColorMatrix", SVGFEColorMatrixElement, SVGElement],
                ["feComposite", SVGFECompositeElement, SVGElement],
                ["feConvolveMatrix", SVGFEConvolveMatrixElement, SVGElement],
                ["feDisplacementMap", SVGFEDisplacementMapElement, SVGElement],
                ["feMorphology", SVGFEMorphologyElement, SVGElement],
                ["feTurbulence", SVGFETurbulenceElement, SVGElement],
                ["feFuncA", SVGFEFuncAElement, SVGComponentTransferFunctionElement],
                ["feFuncB", SVGFEFuncBElement, SVGComponentTransferFunctionElement],
                ["feFuncG", SVGFEFuncGElement, SVGComponentTransferFunctionElement],
                ["feFuncR", SVGFEFuncRElement, SVGComponentTransferFunctionElement],
                ["textPath", SVGTextPathElement, SVGTextContentElement],
              ];

              for (const [tag, constructor, parent] of interfaceCases) {
                const element = document.createElementNS(ns, tag);
                assert(element instanceof constructor, `${tag} interface`);
                assert(Object.getPrototypeOf(constructor.prototype) === parent.prototype,
                  `${constructor.name} parent`);
                let illegalConstructor = false;
                try {
                  new constructor();
                } catch (error) {
                  illegalConstructor = error instanceof TypeError;
                }
                assert(illegalConstructor, `${constructor.name} illegal constructor`);
              }

              const constantGroups = [
                [SVGComponentTransferFunctionElement, [
                  ["SVG_FECOMPONENTTRANSFER_TYPE_UNKNOWN", 0],
                  ["SVG_FECOMPONENTTRANSFER_TYPE_IDENTITY", 1],
                  ["SVG_FECOMPONENTTRANSFER_TYPE_TABLE", 2],
                  ["SVG_FECOMPONENTTRANSFER_TYPE_DISCRETE", 3],
                  ["SVG_FECOMPONENTTRANSFER_TYPE_LINEAR", 4],
                  ["SVG_FECOMPONENTTRANSFER_TYPE_GAMMA", 5],
                ]],
                [SVGFEBlendElement, [
                  ["SVG_FEBLEND_MODE_UNKNOWN", 0],
                  ["SVG_FEBLEND_MODE_NORMAL", 1],
                  ["SVG_FEBLEND_MODE_MULTIPLY", 2],
                  ["SVG_FEBLEND_MODE_SCREEN", 3],
                  ["SVG_FEBLEND_MODE_DARKEN", 4],
                  ["SVG_FEBLEND_MODE_LIGHTEN", 5],
                  ["SVG_FEBLEND_MODE_OVERLAY", 6],
                  ["SVG_FEBLEND_MODE_COLOR_DODGE", 7],
                  ["SVG_FEBLEND_MODE_COLOR_BURN", 8],
                  ["SVG_FEBLEND_MODE_HARD_LIGHT", 9],
                  ["SVG_FEBLEND_MODE_SOFT_LIGHT", 10],
                  ["SVG_FEBLEND_MODE_DIFFERENCE", 11],
                  ["SVG_FEBLEND_MODE_EXCLUSION", 12],
                  ["SVG_FEBLEND_MODE_HUE", 13],
                  ["SVG_FEBLEND_MODE_SATURATION", 14],
                  ["SVG_FEBLEND_MODE_COLOR", 15],
                  ["SVG_FEBLEND_MODE_LUMINOSITY", 16],
                ]],
                [SVGFEColorMatrixElement, [
                  ["SVG_FECOLORMATRIX_TYPE_UNKNOWN", 0],
                  ["SVG_FECOLORMATRIX_TYPE_MATRIX", 1],
                  ["SVG_FECOLORMATRIX_TYPE_SATURATE", 2],
                  ["SVG_FECOLORMATRIX_TYPE_HUEROTATE", 3],
                  ["SVG_FECOLORMATRIX_TYPE_LUMINANCETOALPHA", 4],
                ]],
                [SVGFECompositeElement, [
                  ["SVG_FECOMPOSITE_OPERATOR_UNKNOWN", 0],
                  ["SVG_FECOMPOSITE_OPERATOR_OVER", 1],
                  ["SVG_FECOMPOSITE_OPERATOR_IN", 2],
                  ["SVG_FECOMPOSITE_OPERATOR_OUT", 3],
                  ["SVG_FECOMPOSITE_OPERATOR_ATOP", 4],
                  ["SVG_FECOMPOSITE_OPERATOR_XOR", 5],
                  ["SVG_FECOMPOSITE_OPERATOR_LIGHTER", 6],
                  ["SVG_FECOMPOSITE_OPERATOR_ARITHMETIC", 7],
                ]],
                [SVGFEConvolveMatrixElement, [
                  ["SVG_EDGEMODE_UNKNOWN", 0],
                  ["SVG_EDGEMODE_DUPLICATE", 1],
                  ["SVG_EDGEMODE_WRAP", 2],
                  ["SVG_EDGEMODE_NONE", 3],
                ]],
                [SVGFEDisplacementMapElement, [
                  ["SVG_CHANNEL_UNKNOWN", 0],
                  ["SVG_CHANNEL_R", 1],
                  ["SVG_CHANNEL_G", 2],
                  ["SVG_CHANNEL_B", 3],
                  ["SVG_CHANNEL_A", 4],
                ]],
                [SVGFEMorphologyElement, [
                  ["SVG_MORPHOLOGY_OPERATOR_UNKNOWN", 0],
                  ["SVG_MORPHOLOGY_OPERATOR_ERODE", 1],
                  ["SVG_MORPHOLOGY_OPERATOR_DILATE", 2],
                ]],
                [SVGFETurbulenceElement, [
                  ["SVG_TURBULENCE_TYPE_UNKNOWN", 0],
                  ["SVG_TURBULENCE_TYPE_FRACTALNOISE", 1],
                  ["SVG_TURBULENCE_TYPE_TURBULENCE", 2],
                  ["SVG_STITCHTYPE_UNKNOWN", 0],
                  ["SVG_STITCHTYPE_STITCH", 1],
                  ["SVG_STITCHTYPE_NOSTITCH", 2],
                ]],
                [SVGTextPathElement, [
                  ["TEXTPATH_METHODTYPE_UNKNOWN", 0],
                  ["TEXTPATH_METHODTYPE_ALIGN", 1],
                  ["TEXTPATH_METHODTYPE_STRETCH", 2],
                  ["TEXTPATH_SPACINGTYPE_UNKNOWN", 0],
                  ["TEXTPATH_SPACINGTYPE_AUTO", 1],
                  ["TEXTPATH_SPACINGTYPE_EXACT", 2],
                  ["TEXTPATH_SIDETYPE_UNKNOWN", 0],
                  ["TEXTPATH_SIDETYPE_LEFT", 1],
                  ["TEXTPATH_SIDETYPE_RIGHT", 2],
                ]],
              ];
              for (const [constructor, constants] of constantGroups) {
                for (const [name, expected] of constants) {
                  assert(constructor[name] === expected, `${constructor.name}.${name}`);
                  assert(constructor.prototype[name] === expected,
                    `${constructor.name}.prototype.${name}`);
                }
              }

              const blend = document.createElementNS(ns, "feBlend");
              const blendModes = [
                ["normal", 1], ["multiply", 2], ["screen", 3], ["darken", 4],
                ["lighten", 5], ["overlay", 6], ["color-dodge", 7], ["color-burn", 8],
                ["hard-light", 9], ["soft-light", 10], ["difference", 11],
                ["exclusion", 12], ["hue", 13], ["saturation", 14], ["color", 15],
                ["luminosity", 16],
              ];
              for (const [keyword, value] of blendModes) {
                blend.setAttribute("mode", keyword);
                assert(blend.mode.baseVal === value, `feBlend parses ${keyword}`);
                blend.mode.baseVal = value;
                assert(blend.getAttribute("mode") === keyword, `feBlend serializes ${keyword}`);
              }

              const cases = [
                ["feFuncR", SVGComponentTransferFunctionElement, "type", 1, 5, "gamma", 6],
                ["feBlend", SVGFEBlendElement, "mode", 1, 16, "luminosity", 17],
                ["feColorMatrix", SVGFEColorMatrixElement, "type", 1, 4, "luminanceToAlpha", 5],
                ["feComposite", SVGFECompositeElement, "operator", 1, 7, "arithmetic", 8],
                ["feConvolveMatrix", SVGFEConvolveMatrixElement, "edgeMode", 1, 3, "none", 4],
                ["feDisplacementMap", SVGFEDisplacementMapElement, "xChannelSelector", 4, 1, "R", 5],
                ["feDisplacementMap", SVGFEDisplacementMapElement, "yChannelSelector", 4, 3, "B", 5],
                ["feMorphology", SVGFEMorphologyElement, "operator", 1, 2, "dilate", 3],
                ["feTurbulence", SVGFETurbulenceElement, "stitchTiles", 2, 1, "stitch", 3],
                ["feTurbulence", SVGFETurbulenceElement, "type", 2, 1, "fractalNoise", 3],
                ["textPath", SVGTextPathElement, "method", 1, 2, "stretch", 3],
                ["textPath", SVGTextPathElement, "spacing", 2, 1, "auto", 3],
                ["textPath", SVGTextPathElement, "side", 1, 2, "right", 3],
              ];

              for (const [tag, owner, property, initial, alternate, serialized, invalid] of cases) {
                const element = document.createElementNS(ns, tag);
                const animated = element[property];
                assert(animated instanceof SVGAnimatedEnumeration, `${tag}.${property} interface`);
                assert(element[property] === animated, `${tag}.${property} SameObject`);
                assert(animated.baseVal === initial && animated.animVal === initial,
                  `${tag}.${property} initial value`);

                element.setAttribute(property, serialized);
                assert(animated.baseVal === alternate && animated.animVal === alternate,
                  `${tag}.${property} content attribute update`);
                element.setAttribute(property, "invalid");
                assert(animated.baseVal === initial, `${tag}.${property} invalid content attribute`);
                element.removeAttribute(property);
                assert(animated.baseVal === initial, `${tag}.${property} removed content attribute`);

                animated.baseVal = alternate;
                assert(element.getAttribute(property) === serialized,
                  `${tag}.${property} reflected value`);
                let rejected = false;
                try {
                  animated.baseVal = invalid;
                } catch (error) {
                  rejected = error instanceof TypeError;
                }
                assert(rejected, `${tag}.${property} invalid IDL value`);
                assert(animated.baseVal === alternate,
                  `${tag}.${property} preserved after rejection`);

                const descriptor = Object.getOwnPropertyDescriptor(owner.prototype, property);
                assert(typeof descriptor.get === "function", `${owner.name}.${property} getter`);
                assert(descriptor.enumerable && descriptor.configurable,
                  `${owner.name}.${property} flags`);
              }
              return "ok";
            })()
            "#,
        )
        .expect("SVG filter enumeration probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_animated_integers_reflect_filter_content_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-integer.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const convolve = document.createElementNS(ns, "feConvolveMatrix");
              const turbulence = document.createElementNS(ns, "feTurbulence");

              assert(typeof SVGAnimatedInteger === "function", "constructor exposed");
              let illegalConstructor = false;
              try {
                new SVGAnimatedInteger();
              } catch (error) {
                illegalConstructor = error instanceof TypeError;
              }
              assert(illegalConstructor, "illegal constructor");

              const baseDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedInteger.prototype,
                "baseVal",
              );
              const animDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedInteger.prototype,
                "animVal",
              );
              assert(typeof baseDescriptor.get === "function" &&
                typeof baseDescriptor.set === "function", "baseVal descriptor");
              assert(typeof animDescriptor.get === "function" && animDescriptor.set === undefined,
                "animVal descriptor");
              assert(baseDescriptor.enumerable && baseDescriptor.configurable &&
                animDescriptor.enumerable && animDescriptor.configurable,
                "animated integer descriptor flags");

              const orderX = convolve.orderX;
              const orderY = convolve.orderY;
              const targetX = convolve.targetX;
              const targetY = convolve.targetY;
              const numOctaves = turbulence.numOctaves;
              for (const [owner, property, value, initial] of [
                [convolve, "orderX", orderX, 3],
                [convolve, "orderY", orderY, 3],
                [convolve, "targetX", targetX, 0],
                [convolve, "targetY", targetY, 0],
                [turbulence, "numOctaves", numOctaves, 1],
              ]) {
                assert(value instanceof SVGAnimatedInteger, `${property} interface`);
                assert(Object.prototype.toString.call(value) === "[object SVGAnimatedInteger]",
                  `${property} tag`);
                assert(owner[property] === value, `${property} SameObject`);
                assert(value.baseVal === initial && value.animVal === initial,
                  `${property} initial value`);
                const descriptor = Object.getOwnPropertyDescriptor(
                  Object.getPrototypeOf(owner),
                  property,
                );
                assert(typeof descriptor.get === "function" && descriptor.set === undefined,
                  `${property} element descriptor`);
                assert(descriptor.enumerable && descriptor.configurable,
                  `${property} element descriptor flags`);
              }

              convolve.setAttribute("order", "5");
              assert(orderX.baseVal === 5 && orderY.baseVal === 5, "single order value");
              convolve.setAttribute("order", "5 7");
              assert(orderX.baseVal === 5 && orderY.baseVal === 7, "paired order values");
              convolve.setAttribute("order", "5, 7");
              assert(orderX.baseVal === 5 && orderY.baseVal === 7, "comma order values");
              convolve.setAttribute("order", "-1.5");
              assert(orderX.baseVal === 3 && orderY.baseVal === 3, "invalid order value");
              convolve.removeAttribute("order");
              assert(orderX.baseVal === 3 && orderY.baseVal === 3, "removed order value");

              convolve.setAttribute("targetX", "42");
              convolve.setAttribute("targetY", "-4");
              turbulence.setAttribute("numOctaves", "6");
              assert(targetX.baseVal === 42 && targetY.baseVal === -4,
                "target content attributes");
              assert(numOctaves.baseVal === 6, "numOctaves content attribute");
              turbulence.setAttribute("numOctaves", "invalid");
              assert(numOctaves.baseVal === 1, "invalid numOctaves value");

              targetX.baseVal = -1;
              assert(convolve.getAttribute("targetX") === "-1", "negative reflection");
              assert(targetX.baseVal === -1 && targetX.animVal === -1,
                "negative reflected values");
              targetX.baseVal = 300;
              assert(targetX.baseVal === 300, "positive assignment");
              targetX.baseVal = "aString";
              assert(targetX.baseVal === 0 && convolve.getAttribute("targetX") === "0",
                "WebIDL string conversion");
              targetX.baseVal = convolve;
              assert(targetX.baseVal === 0, "WebIDL object conversion");

              orderX.baseVal = -2;
              assert(orderX.baseVal === -2 && orderY.baseVal === 3,
                "orderX reflected independently");
              assert(convolve.getAttribute("order") === "-2 3", "orderX serialization");
              orderY.baseVal = 8;
              assert(orderX.baseVal === -2 && orderY.baseVal === 8,
                "orderY reflected independently");
              assert(convolve.getAttribute("order") === "-2 8", "orderY serialization");

              let incompatibleAnimatedReceiver = false;
              try {
                baseDescriptor.get.call({});
              } catch (error) {
                incompatibleAnimatedReceiver = error instanceof TypeError;
              }
              assert(incompatibleAnimatedReceiver, "animated integer receiver brand");

              const orderDescriptor = Object.getOwnPropertyDescriptor(
                SVGFEConvolveMatrixElement.prototype,
                "orderX",
              );
              let incompatibleElementReceiver = false;
              try {
                orderDescriptor.get.call(turbulence);
              } catch (error) {
                incompatibleElementReceiver = error instanceof TypeError;
              }
              assert(incompatibleElementReceiver, "element receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated integer probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_svg_element_value_factories_create_typed_objects() {
    let mut vm = new_parsed_test_vm(
        "https://svg-value-factories.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const rejectsTypeError = callback => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error instanceof TypeError;
                }
              };
              const rejectsDom = (name, callback) => {
                try {
                  callback();
                  return false;
                } catch (error) {
                  return error.name === name;
                }
              };
              const ns = "http://www.w3.org/2000/svg";
              const svg = document.createElementNS(ns, "svg");
              const wrongReceiver = document.createElementNS(ns, "rect");

              const factoryNames = [
                "createSVGNumber",
                "createSVGLength",
                "createSVGAngle",
                "createSVGPoint",
                "createSVGMatrix",
                "createSVGRect",
                "createSVGTransform",
                "createSVGTransformFromMatrix",
              ];
              for (const name of factoryNames) {
                const descriptor = Object.getOwnPropertyDescriptor(SVGSVGElement.prototype, name);
                assert(typeof descriptor.value === "function", `${name} method`);
                assert(descriptor.value.length === 0, `${name} arity`);
                assert(descriptor.enumerable && descriptor.configurable && descriptor.writable,
                  `${name} descriptor flags`);
                assert(rejectsTypeError(() => descriptor.value.call(wrongReceiver)),
                  `${name} receiver brand`);
              }

              const number = svg.createSVGNumber();
              const length = svg.createSVGLength();
              const point = svg.createSVGPoint();
              const rect = svg.createSVGRect();
              assert(number instanceof SVGNumber && number.value === 0, "SVGNumber result");
              assert(length instanceof SVGLength && length.value === 0 &&
                length.unitType === SVGLength.SVG_LENGTHTYPE_NUMBER, "SVGLength result");
              assert(point instanceof DOMPoint && point instanceof SVGPoint,
                "SVGPoint result interface");
              assert(point.x === 0 && point.y === 0 && point.z === 0 && point.w === 1,
                "SVGPoint defaults");
              assert(rect instanceof SVGRect && !(rect instanceof DOMRect),
                "SVGRect result interface");
              assert(rect.x === 0 && rect.y === 0 && rect.width === 0 && rect.height === 0,
                "SVGRect defaults");
              assert(svg.createSVGNumber() !== number && svg.createSVGLength() !== length &&
                svg.createSVGPoint() !== point && svg.createSVGRect() !== rect,
                "factories return new objects");

              number.value = 2;
              assert(number.value === 2, "SVGNumber assignment");
              assert(rejectsTypeError(() => { number.value = NaN; }),
                "SVGNumber rejects NaN");
              assert(number.value === 2, "SVGNumber preserves rejected assignment");

              length.valueAsString = "2px";
              assert(length.value === 2 && length.valueAsString === "2px",
                "SVGLength assignment");
              assert(rejectsDom("NotSupportedError", () => {
                length.convertToSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_UNKNOWN);
              }), "SVGLength rejects unsupported unit");
              assert(rejectsDom("SyntaxError", () => { length.valueAsString = "10deg"; }),
                "SVGLength rejects invalid syntax");
              assert(rejectsTypeError(() => { length.value = NaN; }),
                "SVGLength rejects NaN");
              assert(length.value === 2 && length.valueAsString === "2px",
                "SVGLength preserves rejected assignments");

              point.x = 100;
              point.y = 200;
              assert(point.x === 100 && point.y === 200, "SVGPoint assignment");
              for (const invalid of [point, NaN, Infinity]) {
                assert(rejectsTypeError(() => { point.x = invalid; }),
                  "SVGPoint rejects non-finite values");
                assert(point.x === 100, "SVGPoint preserves rejected assignment");
              }
              point.y = null;
              assert(point.y === 0, "SVGPoint converts null");

              Object.assign(rect, { x: 100, y: 200, width: 300, height: 400 });
              assert(rect.x === 100 && rect.y === 200 && rect.width === 300 &&
                rect.height === 400, "SVGRect assignment");
              for (const [property, invalid] of [
                ["x", rect],
                ["y", "aString"],
                ["width", svg],
                ["height", NaN],
              ]) {
                assert(rejectsTypeError(() => { rect[property] = invalid; }),
                  `SVGRect rejects invalid ${property}`);
              }
              rect.y = null;
              assert(rect.y === 0, "SVGRect converts null");

              const regularPoint = new DOMPoint();
              const regularRect = new DOMRect();
              regularPoint.x = NaN;
              regularRect.x = Infinity;
              assert(Number.isNaN(regularPoint.x) && regularRect.x === Infinity,
                "regular geometry objects remain unrestricted");
              return "ok";
            })()
            "#,
        )
        .expect("SVG value factory probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_fit_to_view_box_attributes_are_live_and_reflected() {
    let mut vm = new_parsed_test_vm(
        "https://svg-fit-to-view-box.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const throws = (callback, expected, message) => {
                try {
                  callback();
                } catch (error) {
                  assert(error.name === expected, `${message}: ${error.name}`);
                  return;
                }
                throw new Error(`${message}: did not throw`);
              };
              const accessor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                assert(typeof descriptor?.get === "function", `${prototype.constructor.name}.${name} getter`);
                assert(descriptor.set === undefined, `${prototype.constructor.name}.${name} setter`);
                assert(descriptor.enumerable && descriptor.configurable,
                  `${prototype.constructor.name}.${name} flags`);
                return descriptor;
              };
              const ns = "http://www.w3.org/2000/svg";

              for (const constructor of [
                SVGAnimatedRect,
                SVGPreserveAspectRatio,
                SVGAnimatedPreserveAspectRatio,
              ]) {
                assert(typeof constructor === "function", `${constructor.name} constructor`);
                throws(() => new constructor(), "TypeError", `${constructor.name} illegal constructor`);
              }

              const fitCases = [
                ["svg", SVGSVGElement],
                ["symbol", SVGSymbolElement],
                ["marker", SVGMarkerElement],
                ["pattern", SVGPatternElement],
                ["view", SVGViewElement],
              ];
              for (const [localName, constructor] of fitCases) {
                accessor(constructor.prototype, "viewBox");
                accessor(constructor.prototype, "preserveAspectRatio");
                const element = document.createElementNS(ns, localName);
                const viewBox = element.viewBox;
                const aspectRatio = element.preserveAspectRatio;
                assert(viewBox instanceof SVGAnimatedRect, `${localName}.viewBox interface`);
                assert(aspectRatio instanceof SVGAnimatedPreserveAspectRatio,
                  `${localName}.preserveAspectRatio interface`);
                assert(element.viewBox === viewBox, `${localName}.viewBox SameObject`);
                assert(element.preserveAspectRatio === aspectRatio,
                  `${localName}.preserveAspectRatio SameObject`);
              }

              const imageDescriptor = accessor(
                SVGImageElement.prototype,
                "preserveAspectRatio",
              );
              assert(!("viewBox" in SVGImageElement.prototype), "image has no viewBox");
              const image = document.createElementNS(ns, "image");
              assert(image.preserveAspectRatio instanceof SVGAnimatedPreserveAspectRatio,
                "image preserveAspectRatio interface");
              assert(image.preserveAspectRatio === image.preserveAspectRatio,
                "image preserveAspectRatio SameObject");

              const svg = document.createElementNS(ns, "svg");
              const animatedRect = svg.viewBox;
              const baseRect = animatedRect.baseVal;
              const animRect = animatedRect.animVal;
              assert(baseRect instanceof SVGRect && animRect instanceof SVGRect,
                "viewBox rectangle interfaces");
              for (const rect of [baseRect, animRect]) {
                assert(!(rect instanceof DOMRect), "viewBox uses native SVGRect, not DOMRect");
                assert(Object.getPrototypeOf(rect) === SVGRect.prototype,
                  "viewBox native rectangle prototype");
                assert(Object.prototype.toString.call(rect) === "[object SVGRect]",
                  "viewBox native rectangle tag");
              }
              assert(baseRect !== animRect, "viewBox base and animated rectangles are distinct");
              assert(Object.prototype.toString.call(animatedRect) === "[object SVGAnimatedRect]",
                "animated rectangle tag");
              assert(baseRect.x === 0 && baseRect.y === 0 &&
                baseRect.width === 0 && baseRect.height === 0, "initial viewBox");
              assert(!svg.hasAttribute("viewBox"), "viewBox getter does not create attribute");

              svg.setAttribute("viewBox", "10 20 30 40");
              assert(baseRect.x === 10 && baseRect.y === 20 &&
                baseRect.width === 30 && baseRect.height === 40,
                "cached base rectangle tracks content attribute");
              assert(animRect.x === 10 && animRect.y === 20 &&
                animRect.width === 30 && animRect.height === 40,
                "cached animated rectangle tracks content attribute");
              baseRect.x = 11;
              assert(svg.getAttribute("viewBox") === "11 20 30 40",
                "base rectangle mutation reflects to viewBox");
              assert(baseRect.x === 11 && animRect.x === 11,
                "viewBox values remain synchronized after mutation");
              throws(() => { animRect.x = 12; }, "NoModificationAllowedError",
                "animated rectangle is read-only");
              svg.setAttribute("viewBox", "0 0 -1 2");
              assert(baseRect.x === 0 && baseRect.width === 0,
                "invalid viewBox uses the initial value");
              svg.removeAttribute("viewBox");
              assert(baseRect.x === 0 && baseRect.height === 0,
                "removed viewBox uses the initial value");

              const animatedAspectRatio = svg.preserveAspectRatio;
              const baseAspectRatio = animatedAspectRatio.baseVal;
              const animAspectRatio = animatedAspectRatio.animVal;
              assert(baseAspectRatio instanceof SVGPreserveAspectRatio &&
                animAspectRatio instanceof SVGPreserveAspectRatio,
                "preserveAspectRatio value interfaces");
              assert(baseAspectRatio !== animAspectRatio,
                "preserveAspectRatio base and animated values are distinct");
              assert(baseAspectRatio.align ===
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMIDYMID,
                "initial preserveAspectRatio align");
              assert(baseAspectRatio.meetOrSlice ===
                SVGPreserveAspectRatio.SVG_MEETORSLICE_MEET,
                "initial preserveAspectRatio meetOrSlice");
              assert(!svg.hasAttribute("preserveAspectRatio"),
                "preserveAspectRatio getter does not create attribute");

              svg.setAttribute("preserveAspectRatio", "xMinYMax slice");
              assert(baseAspectRatio.align ===
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMINYMAX,
                "cached base aspect ratio tracks content attribute");
              assert(animAspectRatio.meetOrSlice ===
                SVGPreserveAspectRatio.SVG_MEETORSLICE_SLICE,
                "cached animated aspect ratio tracks content attribute");
              baseAspectRatio.align =
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMAXYMIN;
              baseAspectRatio.meetOrSlice = SVGPreserveAspectRatio.SVG_MEETORSLICE_SLICE;
              assert(svg.getAttribute("preserveAspectRatio") === "xMaxYMin slice",
                "base aspect ratio mutation reflects canonical syntax");
              assert(animAspectRatio.align ===
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMAXYMIN,
                "animated aspect ratio tracks base mutation");
              baseAspectRatio.align = SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_UNKNOWN;
              baseAspectRatio.meetOrSlice = SVGPreserveAspectRatio.SVG_MEETORSLICE_UNKNOWN;
              assert(svg.getAttribute("preserveAspectRatio") === "xMaxYMin slice",
                "unknown enumeration assignments are ignored");
              throws(() => { animAspectRatio.align = 1; }, "NoModificationAllowedError",
                "animated aspect ratio align is read-only");
              throws(() => { animAspectRatio.meetOrSlice = 1; },
                "NoModificationAllowedError", "animated aspect ratio meetOrSlice is read-only");
              svg.setAttribute("preserveAspectRatio", "invalid");
              assert(baseAspectRatio.align ===
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMIDYMID &&
                animAspectRatio.meetOrSlice === SVGPreserveAspectRatio.SVG_MEETORSLICE_MEET,
                "invalid preserveAspectRatio uses the initial value");
              svg.removeAttribute("preserveAspectRatio");
              assert(baseAspectRatio.align ===
                SVGPreserveAspectRatio.SVG_PRESERVEASPECTRATIO_XMIDYMID,
                "removed preserveAspectRatio uses the initial value");

              const constants = [
                ["SVG_PRESERVEASPECTRATIO_UNKNOWN", 0],
                ["SVG_PRESERVEASPECTRATIO_NONE", 1],
                ["SVG_PRESERVEASPECTRATIO_XMINYMIN", 2],
                ["SVG_PRESERVEASPECTRATIO_XMIDYMIN", 3],
                ["SVG_PRESERVEASPECTRATIO_XMAXYMIN", 4],
                ["SVG_PRESERVEASPECTRATIO_XMINYMID", 5],
                ["SVG_PRESERVEASPECTRATIO_XMIDYMID", 6],
                ["SVG_PRESERVEASPECTRATIO_XMAXYMID", 7],
                ["SVG_PRESERVEASPECTRATIO_XMINYMAX", 8],
                ["SVG_PRESERVEASPECTRATIO_XMIDYMAX", 9],
                ["SVG_PRESERVEASPECTRATIO_XMAXYMAX", 10],
                ["SVG_MEETORSLICE_UNKNOWN", 0],
                ["SVG_MEETORSLICE_MEET", 1],
                ["SVG_MEETORSLICE_SLICE", 2],
              ];
              for (const [name, value] of constants) {
                assert(SVGPreserveAspectRatio[name] === value, `constructor ${name}`);
                assert(SVGPreserveAspectRatio.prototype[name] === value, `prototype ${name}`);
              }

              const viewBoxDescriptor = Object.getOwnPropertyDescriptor(
                SVGSVGElement.prototype,
                "viewBox",
              );
              throws(() => viewBoxDescriptor.get.call(document.createElementNS(ns, "rect")),
                "TypeError", "viewBox receiver brand");
              throws(() => imageDescriptor.get.call(document.createElementNS(ns, "rect")),
                "TypeError", "image preserveAspectRatio receiver brand");
              const animatedRectDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedRect.prototype,
                "baseVal",
              );
              throws(() => animatedRectDescriptor.get.call({}), "TypeError",
                "animated rectangle receiver brand");
              const alignDescriptor = Object.getOwnPropertyDescriptor(
                SVGPreserveAspectRatio.prototype,
                "align",
              );
              throws(() => alignDescriptor.get.call({}), "TypeError",
                "preserveAspectRatio receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG fit-to-view-box reflection probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_view_box_rects_preserve_native_float_and_reentrant_setter_semantics() {
    let mut vm = new_parsed_test_vm(
        "https://svg-view-box-native-rect.test/",
        "<!doctype html><html><head></head><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const throws = (callback, name) => {
                try { callback(); } catch (error) {
                  assert(error.name === name, `expected ${name}, got ${error.name}`);
                  return;
                }
                throw new Error(`expected ${name}`);
              };
              const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
              const fields = ["x", "y", "width", "height"];
              const values = rect => fields.map(field => rect[field]);
              svg.setAttribute("viewBox", "1.2 2.4 3.6 4.8");
              const base = svg.viewBox.baseVal;
              const anim = svg.viewBox.animVal;
              const assertValues = expected => {
                for (const rect of [base, anim]) {
                  assert(JSON.stringify(values(rect)) === JSON.stringify(expected.map(Math.fround)),
                    `float values: ${values(rect)}`);
                }
              };
              assertValues([1.2, 2.4, 3.6, 4.8]);
              svg.setAttribute("viewBox", "5.5 6.6 7.7 8.8");
              assertValues([5.5, 6.6, 7.7, 8.8]);
              base.x = "1.2";
              base.width = 1e20;
              assertValues([1.2, 6.6, 1e20, 8.8]);
              const reflected = svg.getAttribute("viewBox");
              assert(JSON.stringify(reflected.split(" ").map(Number).map(Math.fround)) ===
                JSON.stringify(values(base)), "float serialization round trip");
              for (const invalid of [NaN, Infinity, -Infinity, 1e40, undefined, Symbol()]) {
                throws(() => { base.x = invalid; }, "TypeError");
                assert(svg.getAttribute("viewBox") === reflected,
                  "rejected float assignment preserves the attribute");
              }
              throws(() => { anim.width = 1; }, "NoModificationAllowedError");
              const domRectX = Object.getOwnPropertyDescriptor(DOMRect.prototype, "x");
              throws(() => domRectX.get.call(base), "TypeError");
              throws(() => domRectX.set.call(anim, 7), "TypeError");
              assert(svg.getAttribute("viewBox") === reflected,
                "read-only and foreign-interface setters preserve the attribute");

              base.x = {
                valueOf() {
                  svg.setAttribute("viewBox", "10 20 30 40");
                  return 9;
                }
              };
              assert(svg.getAttribute("viewBox") === "9 20 30 40",
                "numeric coercion preserves reentrant changes to the other fields");
              assertValues([9, 20, 30, 40]);
              svg.setAttribute("viewBox", "0 0 1e40 1");
              assertValues([0, 0, 0, 0]);
              return "ok";
            })()
            "#,
        )
        .expect("native SVG viewBox rectangle probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_animated_numbers_reflect_filter_and_stop_content_attributes() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animated-number.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const ns = "http://www.w3.org/2000/svg";
              const cases = [
                ["feFuncA", SVGFEFuncAElement, SVGComponentTransferFunctionElement, [
                  ["slope", "slope", 1], ["intercept", "intercept", 0],
                  ["amplitude", "amplitude", 1], ["exponent", "exponent", 1],
                  ["offset", "offset", 0],
                ]],
                ["feComposite", SVGFECompositeElement, SVGFECompositeElement, [
                  ["k1", "k1", 0], ["k2", "k2", 0],
                  ["k3", "k3", 0], ["k4", "k4", 0],
                ]],
                ["feConvolveMatrix", SVGFEConvolveMatrixElement,
                  SVGFEConvolveMatrixElement, [
                    ["divisor", "divisor", 1], ["bias", "bias", 0],
                    ["kernelUnitLengthX", "kernelUnitLength", 0],
                    ["kernelUnitLengthY", "kernelUnitLength", 0],
                  ]],
                ["feDiffuseLighting", SVGFEDiffuseLightingElement,
                  SVGFEDiffuseLightingElement, [
                    ["surfaceScale", "surfaceScale", 1],
                    ["diffuseConstant", "diffuseConstant", 1],
                    ["kernelUnitLengthX", "kernelUnitLength", 0],
                    ["kernelUnitLengthY", "kernelUnitLength", 0],
                  ]],
                ["feDisplacementMap", SVGFEDisplacementMapElement,
                  SVGFEDisplacementMapElement, [["scale", "scale", 0]]],
                ["feDistantLight", SVGFEDistantLightElement,
                  SVGFEDistantLightElement, [
                    ["azimuth", "azimuth", 0], ["elevation", "elevation", 0],
                  ]],
                ["feDropShadow", SVGFEDropShadowElement, SVGFEDropShadowElement, [
                  ["dx", "dx", 2], ["dy", "dy", 2],
                  ["stdDeviationX", "stdDeviation", 2],
                  ["stdDeviationY", "stdDeviation", 2],
                ]],
                ["feGaussianBlur", SVGFEGaussianBlurElement,
                  SVGFEGaussianBlurElement, [
                    ["stdDeviationX", "stdDeviation", 0],
                    ["stdDeviationY", "stdDeviation", 0],
                  ]],
                ["feMorphology", SVGFEMorphologyElement, SVGFEMorphologyElement, [
                  ["radiusX", "radius", 0], ["radiusY", "radius", 0],
                ]],
                ["feOffset", SVGFEOffsetElement, SVGFEOffsetElement, [
                  ["dx", "dx", 0], ["dy", "dy", 0],
                ]],
                ["fePointLight", SVGFEPointLightElement, SVGFEPointLightElement, [
                  ["x", "x", 0], ["y", "y", 0], ["z", "z", 0],
                ]],
                ["feSpecularLighting", SVGFESpecularLightingElement,
                  SVGFESpecularLightingElement, [
                    ["surfaceScale", "surfaceScale", 1],
                    ["specularConstant", "specularConstant", 1],
                    ["specularExponent", "specularExponent", 1],
                    ["kernelUnitLengthX", "kernelUnitLength", 0],
                    ["kernelUnitLengthY", "kernelUnitLength", 0],
                  ]],
                ["feSpotLight", SVGFESpotLightElement, SVGFESpotLightElement, [
                  ["x", "x", 0], ["y", "y", 0], ["z", "z", 0],
                  ["pointsAtX", "pointsAtX", 0], ["pointsAtY", "pointsAtY", 0],
                  ["pointsAtZ", "pointsAtZ", 0],
                  ["specularExponent", "specularExponent", 1],
                  ["limitingConeAngle", "limitingConeAngle", 0],
                ]],
                ["feTurbulence", SVGFETurbulenceElement, SVGFETurbulenceElement, [
                  ["baseFrequencyX", "baseFrequency", 0],
                  ["baseFrequencyY", "baseFrequency", 0], ["seed", "seed", 0],
                ]],
                ["stop", SVGStopElement, SVGStopElement, [["offset", "offset", 0]]],
              ];

              assert(typeof SVGAnimatedNumber === "function", "constructor exposed");
              let illegalConstructor = false;
              try {
                new SVGAnimatedNumber();
              } catch (error) {
                illegalConstructor = error instanceof TypeError;
              }
              assert(illegalConstructor, "illegal constructor");

              const baseDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedNumber.prototype,
                "baseVal",
              );
              const animDescriptor = Object.getOwnPropertyDescriptor(
                SVGAnimatedNumber.prototype,
                "animVal",
              );
              assert(typeof baseDescriptor.get === "function" &&
                typeof baseDescriptor.set === "function", "baseVal descriptor");
              assert(typeof animDescriptor.get === "function" && animDescriptor.set === undefined,
                "animVal descriptor");

              for (const [tag, constructor, owner, properties] of cases) {
                const element = document.createElementNS(ns, tag);
                assert(element instanceof constructor, `${tag} interface`);
                for (const [property, attribute, initial] of properties) {
                  const descriptor = Object.getOwnPropertyDescriptor(owner.prototype, property);
                  assert(typeof descriptor.get === "function" && descriptor.set === undefined,
                    `${owner.name}.${property} descriptor`);
                  assert(descriptor.enumerable && descriptor.configurable,
                    `${owner.name}.${property} descriptor flags`);

                  const animated = element[property];
                  assert(animated instanceof SVGAnimatedNumber, `${tag}.${property} interface`);
                  assert(element[property] === animated, `${tag}.${property} SameObject`);
                  assert(animated.baseVal === initial && animated.animVal === initial,
                    `${tag}.${property} initial value`);

                  element.setAttribute(attribute, "42");
                  assert(animated.baseVal === 42 && animated.animVal === 42,
                    `${tag}.${property} live content update`);
                  element.setAttribute(attribute, "foobar");
                  assert(animated.baseVal === initial && animated.animVal === initial,
                    `${tag}.${property} invalid fallback`);
                  element.removeAttribute(attribute);
                  assert(animated.baseVal === initial && animated.animVal === initial,
                    `${tag}.${property} removed fallback`);
                }
              }

              const gaussian = document.createElementNS(ns, "feGaussianBlur");
              const deviationX = gaussian.stdDeviationX;
              const deviationY = gaussian.stdDeviationY;
              gaussian.setAttribute("stdDeviation", "5");
              assert(deviationX.baseVal === 5 && deviationY.baseVal === 5,
                "single optional-number value");
              gaussian.setAttribute("stdDeviation", "5, 7");
              assert(deviationX.baseVal === 5 && deviationY.baseVal === 7,
                "paired optional-number values");
              deviationX.baseVal = 3;
              assert(gaussian.getAttribute("stdDeviation") === "3 7",
                "first pair component reflection");
              deviationY.baseVal = 9;
              assert(gaussian.getAttribute("stdDeviation") === "3 9" &&
                deviationX.baseVal === 3 && deviationY.animVal === 9,
                "second pair component reflection");

              const specular = document.createElementNS(ns, "feSpecularLighting");
              const surfaceScale = specular.surfaceScale;
              surfaceScale.baseVal = -1;
              assert(surfaceScale.baseVal === -1 && surfaceScale.animVal === -1 &&
                specular.getAttribute("surfaceScale") === "-1", "scalar reflection");
              surfaceScale.baseVal = 300;
              for (const invalid of ["aString", NaN, Infinity, specular]) {
                let rejected = false;
                try {
                  surfaceScale.baseVal = invalid;
                } catch (error) {
                  rejected = error instanceof TypeError;
                }
                assert(rejected && surfaceScale.baseVal === 300,
                  "restricted float rejects invalid values");
              }

              const stop = document.createElementNS(ns, "stop");
              const stopOffset = stop.offset;
              stop.setAttribute("offset", "50%");
              assert(stopOffset.baseVal === 0.5 && stopOffset.animVal === 0.5,
                "stop percentage content value");
              stopOffset.baseVal = 0.25;
              assert(stop.getAttribute("offset") === "0.25", "stop number reflection");

              let incompatibleAnimatedReceiver = false;
              try {
                baseDescriptor.get.call({});
              } catch (error) {
                incompatibleAnimatedReceiver = error instanceof TypeError;
              }
              assert(incompatibleAnimatedReceiver, "animated number receiver brand");

              const k1Descriptor = Object.getOwnPropertyDescriptor(
                SVGFECompositeElement.prototype,
                "k1",
              );
              let incompatibleElementReceiver = false;
              try {
                k1Descriptor.get.call(specular);
              } catch (error) {
                incompatibleElementReceiver = error instanceof TypeError;
              }
              assert(incompatibleElementReceiver, "element receiver brand");
              return "ok";
            })()
            "#,
        )
        .expect("SVG animated number probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_lengths_preserve_specified_units_and_resolve_live_context() {
    let mut vm = new_parsed_test_vm(
        "https://svg-length-context.test/",
        r#"<!doctype html>
        <html style="font-size:20px;line-height:24px">
          <body>
            <svg id="svg" width="150" height="50" viewBox="0 0 150 50">
              <rect id="rect" style="font-size:12px"/>
              <text id="text" x="10ch" style="font-size:20px"></text>
              <text id="cap" x="10cap" style="font-size:20px"></text>
              <text id="ic" x="10ic" style="font-size:20px"></text>
              <text id="lh" x="10lh" style="font-size:20px;line-height:24px"></text>
              <text id="rlh" x="10rlh" style="font-size:10px"></text>
              <rect id="rem" x="5rem"/>
              <rect id="viewport" x="10vw"/>
            </svg>
          </body>
        </html>"#,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const assert = (condition, message) => {
                if (!condition) throw new Error(message);
              };
              const close = (actual, expected, tolerance = 1e-6) =>
                Math.abs(actual - expected) <= tolerance;
              const svg = document.getElementById("svg");

              const standalone = svg.createSVGLength();
              standalone.valueAsString = "48px";
              standalone.convertToSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_CM);
              assert(standalone.valueAsString === "1.27cm",
                `absolute serialization: ${standalone.valueAsString}`);
              assert(close(standalone.valueInSpecifiedUnits, 1.27),
                "absolute specified value");
              assert(close(standalone.value, 48), "absolute user value");
              standalone.value = 96;
              assert(close(standalone.valueInSpecifiedUnits, 2.54),
                "value setter preserves unit");
              standalone.valueInSpecifiedUnits = 1;
              assert(close(standalone.value, 96 / 2.54),
                "specified setter resolves user value");

              standalone.valueAsString = "40q";
              assert(standalone.unitType === SVGLength.SVG_LENGTHTYPE_UNKNOWN,
                "modern unit type");
              assert(close(standalone.value, 96 / 2.54), "quarter millimeter value");
              standalone.convertToSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_MM);
              assert(standalone.valueAsString === "10mm", "quarter millimeter conversion");

              const rect = document.getElementById("rect");
              const x = rect.x.baseVal;
              x.valueAsString = "3px";
              x.convertToSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_PERCENTAGE);
              assert(x.valueAsString === "2%" && close(x.value, 3),
                `percentage conversion uses SVG viewport: ${x.valueAsString}|${x.value}|${x.valueInSpecifiedUnits}`);
              x.valueAsString = "6px";
              x.convertToSpecifiedUnits(SVGLength.SVG_LENGTHTYPE_EMS);
              assert(x.valueAsString === "0.5em" && close(x.value, 6),
                "em conversion uses owner font size");

              svg.removeAttribute("viewBox");
              x.valueAsString = "50%";
              assert(close(x.value, 75), "initial percentage basis");
              svg.width.baseVal.value = 300;
              assert(close(x.value, 150), "live percentage basis");

              const ch = document.getElementById("text").x.baseVal[0];
              assert(ch.unitType === SVGLength.SVG_LENGTHTYPE_UNKNOWN,
                "list modern unit type");
              assert(close(ch.value, 100), "ch resolves from owner font size");
              ch.value = 200;
              assert(close(ch.valueInSpecifiedUnits, 20), "ch reverse conversion");

              for (const [id, expected] of [
                ["cap", 140],
                ["ic", 200],
                ["lh", 240],
                ["rlh", 240],
                ["rem", 100],
              ]) {
                const modern = document.getElementById(id).x.baseVal[0] ??
                  document.getElementById(id).x.baseVal;
                assert(modern.unitType === SVGLength.SVG_LENGTHTYPE_UNKNOWN,
                  `${id} unit type`);
                assert(close(modern.value, expected), `${id} context value`);
                const specified = modern.valueInSpecifiedUnits;
                modern.value = expected * 2;
                assert(close(modern.valueInSpecifiedUnits, specified * 2),
                  `${id} reverse conversion`);
              }

              const viewport = document.getElementById("viewport").x.baseVal;
              const viewportValue = viewport.value;
              viewport.value = viewportValue * 2;
              assert(close(viewport.valueInSpecifiedUnits, 20),
                "viewport reverse conversion");
              return "ok";
            })()
            "#,
        )
        .expect("SVG length context probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn svg_switch_and_mpath_use_native_interfaces_and_live_href() {
    let mut vm = new_parsed_test_vm("https://svg-switch-mpath.test/", "<body></body>");
    assert_eq!(
        vm.eval(include_str!("svg_switch_mpath.js"))
            .expect("SVG switch and mpath fixture should evaluate"),
        "true"
    );
}

#[test]
fn svg_animation_elements_use_native_interfaces_and_event_handlers() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animation.test/",
        "<body><iframe id=child></iframe></body>",
    );
    vm.eval(include_str!("svg_animation_interfaces.js"))
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__nodeReplacementResults.failures)")
            .unwrap(),
        "[]"
    );
}

#[test]
fn svg_animation_timing_shim_validates_arguments_before_reporting_unsupported() {
    let mut vm = new_parsed_test_vm(
        "https://svg-animation-timing.test/",
        "<body><iframe id=child></iframe></body>",
    );
    assert_eq!(vm.eval(r#"
    (() => {
      const other = document.getElementById('child').contentWindow;
      for (const realm of [window, other]) {
        const element = document.createElementNS('http://www.w3.org/2000/svg', 'animate');
        for (const name of ['beginElement', 'endElement', 'beginElementAt', 'endElementAt']) {
          let error, conversions = 0;
          try {realm.SVGAnimationElement.prototype[name].call(element, {valueOf() {conversions++; return 0.25;}});}
          catch (caught) {error = caught;}
          if (!(error instanceof realm.DOMException) || error.name !== 'NotSupportedError') throw Error(name + ' shim error');
          if (conversions !== (name.endsWith('At') ? 1 : 0)) throw Error(name + ' argument conversion');
          if (element.getCurrentTime() !== 0) throw Error('shim cannot advance the timeline');
        }
      }
      return true;
    })()
    "#).unwrap(), "true");
}

#[test]
fn svg_filter_primitives_expose_native_dom_attributes() {
    let mut vm = new_parsed_test_vm("https://svg-filter-dom.test/", "<html><body></body></html>");
    assert_eq!(
        vm.eval(include_str!("svg_filter_primitives.js"))
            .expect("SVG filter DOM fixture should evaluate"),
        "true"
    );
}
