use super::*;

#[cfg(feature = "wpt-extensions")]
#[test]
fn wpt_webdriver_delete_all_cookies_is_declared_on_prototype() {
    let mut vm = new_storage_test_vm("https://wpt-webdriver-prototype.test/");
    vm.set_wpt_extensions_enabled(true)
        .expect("WPT WebDriver extension should install");

    let result = vm
        .eval(
            r#"
(() => {
  const descriptor = Object.getOwnPropertyDescriptor(WebDriver.prototype, "deleteAllCookies");
  const value = descriptor && descriptor.value;
  return [
    typeof WebDriver,
    Object.prototype.toString.call(webdriver),
    webdriver instanceof WebDriver,
    typeof webdriver.deleteAllCookies,
    typeof value,
    value && value.name,
    value && value.length,
    descriptor && descriptor.enumerable,
    descriptor && descriptor.configurable,
    descriptor && descriptor.writable,
    /\[native code\]/.test(String(value)),
    String(webdriver.deleteAllCookies())
  ].join("|");
})()
"#,
        )
        .expect("WPT WebDriver prototype method shape should evaluate");

    assert_eq!(
        result,
        "function|[object WebDriver]|true|function|function|deleteAllCookies|0|false|true|true|true|undefined"
    );
}
#[test]
fn internal_dynamic_maps_ignore_object_prototype_pollution() {
    let mut vm = new_storage_test_vm("https://internal-dynamic-map-pollution.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const out = {
    simpleCalls: 0,
    simplePoison: false,
    mqlCalls: 0,
    mqlPoison: false
  };
  try {
    Object.prototype.probe = [() => { out.simplePoison = true; }];
    const target = new EventTarget();
    target.addEventListener("probe", () => { out.simpleCalls++; });
    target.dispatchEvent(new Event("probe"));
    out.simpleProtoLength = Object.prototype.probe.length;

    Object.prototype.change = [() => { out.mqlPoison = true; }];
    const mql = matchMedia("(min-width: 0px)");
    mql.addEventListener("change", () => { out.mqlCalls++; });
    mql.dispatchEvent(new Event("change"));
    out.mqlProtoLength = Object.prototype.change.length;

    Object.prototype.load = [];
    const reader = new FileReader();
    reader.addEventListener("load", () => {});
    out.fileReaderProtoLength = Object.prototype.load.length;

    Object.prototype.loading = [];
    const fonts = document.fonts;
    fonts.addEventListener("loading", () => {});
    out.fontFaceSetProtoLength = Object.prototype.loading.length;

    Object.prototype.id = { name: "poisoned", value: "poisoned" };
    const element = document.createElement("div");
    element.setAttribute("id", "actual");
    const attr = element.getAttributeNode("id");
    out.attrName = attr && attr.name;
    out.attrValue = attr && attr.value;
    out.attrPoison = attr === Object.prototype.id;

    return JSON.stringify(out);
  } finally {
    delete Object.prototype.probe;
    delete Object.prototype.change;
    delete Object.prototype.load;
    delete Object.prototype.loading;
    delete Object.prototype.id;
  }
})()
"#,
        )
        .expect("internal dynamic map pollution probe should evaluate");

    assert_eq!(
        result,
        r#"{"simpleCalls":1,"simplePoison":false,"mqlCalls":1,"mqlPoison":false,"simpleProtoLength":1,"mqlProtoLength":1,"fileReaderProtoLength":0,"fontFaceSetProtoLength":0,"attrName":"id","attrValue":"actual","attrPoison":false}"#
    );
}
#[test]
fn webidl_attribute_setters_preserve_undefined_and_replaceable_semantics() {
    let mut vm = new_storage_test_vm("https://webidl-attribute-setters.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = callback => {
    try { callback(); return "return"; }
    catch (error) { return error && error.name; }
  };

  const animation = new Animation();
  const animationId = Object.getOwnPropertyDescriptor(Animation.prototype, "id");
  animationId.set.call(animation);

  const bodySetter = Object.getOwnPropertyDescriptor(Document.prototype, "body").set;
  const fullscreen = Object.getOwnPropertyDescriptor(Document.prototype, "fullscreenEnabled");
  const detached = new Document();
  fullscreen.set.call(detached);

  const mouseEnter = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "onmouseenter");
  const element = document.createElement("div");
  mouseEnter.set.call(element);

  const replaceableKeys = [
    "self", "parent", "origin", "innerWidth", "screen", "length", "event",
    "outerHeight", "scrollX", "screenLeft", "screenTop", "screenX", "screenY"
  ];
  const replaceableShape = replaceableKeys.every(key => {
    const descriptor = Object.getOwnPropertyDescriptor(window, key);
    return descriptor && typeof descriptor.get === "function" &&
      typeof descriptor.set === "function" && descriptor.enumerable &&
      descriptor.configurable;
  });

  Object.getOwnPropertyDescriptor(window, "scrollX").set.call(window);
  Object.getOwnPropertyDescriptor(window, "screenLeft").set.call(window, undefined);
  Object.getOwnPropertyDescriptor(window, "screenTop").set.call(window, "foo");

  Object.defineProperty(window, "self", { configurable: false });
  const selfFailure = outcome(() => { window.self = 1; });
  const screenSetter = Object.getOwnPropertyDescriptor(window, "screen").set;
  Object.defineProperty(window, "screen", {
    value: 1,
    writable: false,
    configurable: false,
    enumerable: true
  });
  const screenFailure = outcome(() => { screenSetter.call(window, 2); });

  return JSON.stringify({
    animation: [animation.id, outcome(() => animationId.set.call({}))].join(","),
    document: [outcome(() => bodySetter.call({})), detached.fullscreenEnabled].join(","),
    eventHandler: [outcome(() => mouseEnter.set.call({})), element.onmouseenter].join(","),
    replaceableShape,
    replacements: [window.scrollX, window.screenLeft, window.screenTop].join(","),
    failures: [selfFailure, screenFailure].join(",")
  });
})()
"#,
        )
        .expect("WebIDL attribute setter probe should evaluate");

    assert_eq!(
        result,
        r#"{"animation":"undefined,TypeError","document":"TypeError,false","eventHandler":"return,","replaceableShape":true,"replacements":",,foo","failures":"TypeError,TypeError"}"#
    );
}
#[test]
fn native_bridge_helper_template_preserves_declared_descriptors() {
    let mut vm = new_storage_test_vm("https://native-bridge-template.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const globalDescriptor =
                Object.getOwnPropertyDescriptor(window, "__moliNativeBridge");
              const bridge = globalDescriptor.value;
              const describe = name => {
                const descriptor = Object.getOwnPropertyDescriptor(bridge, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.get,
                  typeof descriptor?.set,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              return JSON.stringify({
                globalDescriptor: [
                  globalDescriptor.enumerable,
                  globalDescriptor.writable,
                  globalDescriptor.configurable
                ],
                keys: [
                  Object.keys(bridge).includes("window"),
                  Object.keys(bridge).includes("document"),
                  Object.keys(bridge).includes("getElementById"),
                  Object.keys(bridge).includes("__detachedGetElementById"),
                  Object.keys(bridge).includes("__detachedNodeType"),
                  Object.keys(bridge).includes("__detachedAppend"),
                  Object.keys(bridge).includes("ownerDocument"),
                  Object.keys(bridge).includes("resolveNode")
                ],
                rawResolver: [
                  typeof bridge.resolveNode,
                  Object.getOwnPropertyDescriptor(bridge, "resolveNode") === undefined
                ],
                descriptors: [
                  describe("window"),
                  describe("document"),
                  describe("getElementById"),
                  describe("__detachedGetElementById"),
                  describe("__detachedNodeType"),
                  describe("__detachedAppend"),
                  describe("ownerDocument")
                ]
              });
            })()
            "#,
        )
        .expect("native bridge helper descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"globalDescriptor":[false,true,true],"keys":[true,true,true,true,true,true,true,false],"rawResolver":["undefined",true],"descriptors":["window:true:undefined:undefined:object::0:true:true:true","document:true:undefined:undefined:object:::true:true:true","getElementById:true:undefined:undefined:function:getElementById:0:true:true:true","__detachedGetElementById:true:undefined:undefined:function:__detachedGetElementById:0:true:true:true","__detachedNodeType:true:undefined:undefined:function:__detachedNodeType:0:true:true:true","__detachedAppend:true:undefined:undefined:function:__detachedAppend:0:true:true:true","ownerDocument:true:undefined:undefined:function:ownerDocument:0:true:true:true"]}"#
    );
}
#[test]
fn web_platform_surface_stubs_are_present_and_brand_correctly() {
    let mut vm = new_storage_test_vm("https://surfaces.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const names = [
                "AnimationEffect","KeyframeEffect","AnimationTimeline","DocumentTimeline",
                "AnimationPlaybackEvent",
                "BeforeUnloadEvent","HashChangeEvent","MediaQueryListEvent",
                "SecurityPolicyViolationEvent","ToggleEvent","CommandEvent","InterestEvent",
                "ContentVisibilityAutoStateChangeEvent",
                "DOMRectReadOnly","DOMPointReadOnly","DOMQuad",
                "ByteLengthQueuingStrategy","CountQueuingStrategy",
                "CompressionStream","DecompressionStream",
                "ReadableStreamBYOBReader","ReadableStreamBYOBRequest","ReadableByteStreamController",
                "Geolocation","GeolocationPosition","GeolocationCoordinates","GeolocationPositionError",
                "MediaCapabilities","Clipboard","ClipboardItem",
              ];
              const out = [];
              for (const name of names) {
                const ctor = window[name];
                out.push(`${name}:${typeof ctor}:${ctor && typeof ctor.prototype === "object"}`);
              }
              out.push(`ToggleEvent<Event:${ToggleEvent.prototype instanceof Event}`);
              out.push(`HashChangeEvent<Event:${HashChangeEvent.prototype instanceof Event}`);
              out.push(`DOMRect<DOMRectReadOnly:${DOMRect.prototype instanceof DOMRectReadOnly}`);
              out.push(`DOMPoint<DOMPointReadOnly:${DOMPoint.prototype instanceof DOMPointReadOnly}`);
              out.push(`Clipboard<EventTarget:${Clipboard.prototype instanceof EventTarget}`);
              return out.join("|");
            })()
            "#,
        )
        .expect("web platform surface stub probe should evaluate");

    let expected_parts: Vec<&str> = vec![
        "AnimationEffect:function:true",
        "KeyframeEffect:function:true",
        "AnimationTimeline:function:true",
        "DocumentTimeline:function:true",
        "AnimationPlaybackEvent:function:true",
        "BeforeUnloadEvent:function:true",
        "HashChangeEvent:function:true",
        "MediaQueryListEvent:function:true",
        "SecurityPolicyViolationEvent:function:true",
        "ToggleEvent:function:true",
        "CommandEvent:function:true",
        "InterestEvent:function:true",
        "ContentVisibilityAutoStateChangeEvent:function:true",
        "DOMRectReadOnly:function:true",
        "DOMPointReadOnly:function:true",
        "DOMQuad:function:true",
        "ByteLengthQueuingStrategy:function:true",
        "CountQueuingStrategy:function:true",
        "CompressionStream:function:true",
        "DecompressionStream:function:true",
        "ReadableStreamBYOBReader:function:true",
        "ReadableStreamBYOBRequest:function:true",
        "ReadableByteStreamController:function:true",
        "Geolocation:function:true",
        "GeolocationPosition:function:true",
        "GeolocationCoordinates:function:true",
        "GeolocationPositionError:function:true",
        "MediaCapabilities:function:true",
        "Clipboard:function:true",
        "ClipboardItem:function:true",
        "ToggleEvent<Event:true",
        "HashChangeEvent<Event:true",
        "DOMRect<DOMRectReadOnly:true",
        "DOMPoint<DOMPointReadOnly:true",
        "Clipboard<EventTarget:true",
    ];
    assert_eq!(result, expected_parts.join("|"));
}
#[test]
fn dom_matrix_tracks_explicit_dimension_and_validates_init() {
    let mut vm = new_storage_test_vm("https://dommatrix-dimension.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const identity3d = [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1
  ];
  const outcome = callback => {
    try {
      callback();
      return "no throw";
    } catch (error) {
      return error.name;
    }
  };

  const sequence = new DOMMatrix(identity3d);
  const typed = DOMMatrixReadOnly.fromFloat64Array(new Float64Array(identity3d));
  const explicit = DOMMatrix.fromMatrix({is2D: false});
  const copied = new DOMMatrix(explicit);
  const multiplied = new DOMMatrix().multiply({is2D: false});
  const preMultiplied = new DOMMatrix().preMultiplySelf({is2D: false});

  const sticky = new DOMMatrix();
  sticky.m13 = 2;
  sticky.m13 = 0;

  const reset = new DOMMatrix(identity3d);
  reset.setMatrixValue("");

  const origin3d = new DOMMatrix().scaleSelf(1, 1, 1, 0, 0, 2);
  const axis3d = new DOMMatrix().rotateAxisAngleSelf(1, 0, 0, 0);
  const negativeZero2d = new DOMMatrix().translateSelf(0, 0, -0);
  const parsed3d = new DOMMatrix(
    "matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)"
  );

  return [
    sequence.is2D,
    typed.is2D,
    explicit.is2D,
    copied.is2D,
    multiplied.is2D,
    preMultiplied.is2D,
    sticky.is2D,
    reset.is2D,
    origin3d.is2D,
    axis3d.is2D,
    negativeZero2d.is2D,
    parsed3d.is2D,
    String(explicit),
    outcome(() => DOMMatrix.fromMatrix({a: 1, m11: 2})),
    outcome(() => DOMMatrix.fromMatrix({is2D: true, m13: 1})),
    outcome(() => new DOMMatrix(" "))
  ].join("|");
})()
"#,
        )
        .expect("DOMMatrix dimension probe should evaluate");

    assert_eq!(
        result,
        concat!(
            "false|false|false|false|false|false|false|true|false|false|true|false|",
            "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)|",
            "TypeError|TypeError|SyntaxError"
        )
    );
}

#[test]
fn dom_matrix_objects_keep_declared_brand_and_own_slots() {
    let mut vm = new_storage_test_vm("https://dommatrix-declared-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const stringify = value => value === undefined ? "undefined" : String(value);
              const internalNames = object => Object.getOwnPropertyNames(object)
                .filter(name => name.startsWith("__moliDomMatrix"))
                .sort()
                .join(",");
              const descriptorShape = (prototype, receiver, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor.get,
                  descriptor.get && descriptor.get.name,
                  descriptor.get && descriptor.get.length,
                  typeof descriptor.set,
                  descriptor.set && descriptor.set.name,
                  descriptor.set && descriptor.set.length,
                  descriptor.enumerable,
                  descriptor.configurable,
                  Object.prototype.hasOwnProperty.call(receiver, name)
                ].map(stringify).join(":");
              };
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return error.constructor.name;
                }
              };
              const mutableNames = [
                "a", "b", "c", "d", "e", "f",
                "m11", "m12", "m13", "m14",
                "m21", "m22", "m23", "m24",
                "m31", "m32", "m33", "m34",
                "m41", "m42", "m43", "m44"
              ];
              const readonlyNames = mutableNames.concat(["is2D", "isIdentity"]);
              const matrix = new DOMMatrix([1, 2, 3, 4, 5, 6]);
              const readonly = DOMMatrixReadOnly.fromMatrix(matrix);
              const initialMatrixSlots = internalNames(matrix);
              const initialReadonlySlots = internalNames(readonly);
              DOMMatrix.prototype.__moliDomMatrixReadOnlyBrand = true;
              DOMMatrix.prototype.__moliDomMatrixMutableBrand = true;
              DOMMatrixReadOnly.prototype.__moliDomMatrixReadOnlyBrand = true;
              for (const name of [
                "__moliDomMatrixM11",
                "__moliDomMatrixM12",
                "__moliDomMatrixM21",
                "__moliDomMatrixM22",
                "__moliDomMatrixM41",
                "__moliDomMatrixM42"
              ]) {
                DOMMatrix.prototype[name] = -100;
                DOMMatrixReadOnly.prototype[name] = -100;
                matrix[name] = -200;
                readonly[name] = -300;
              }
              matrix.__moliDomMatrixReadOnlyBrand = true;
              matrix.__moliDomMatrixMutableBrand = true;
              readonly.__moliDomMatrixReadOnlyBrand = true;
              const receiver = Object.create(matrix);
              const fake = Object.assign(Object.create(DOMMatrix.prototype), {
                __moliDomMatrixReadOnlyBrand: true,
                __moliDomMatrixMutableBrand: true,
                __moliDomMatrixM11: 1,
                __moliDomMatrixM12: 2,
                __moliDomMatrixM21: 3,
                __moliDomMatrixM22: 4,
                __moliDomMatrixM41: 5,
                __moliDomMatrixM42: 6
              });
              const readonlyFake = Object.assign(Object.create(DOMMatrixReadOnly.prototype), {
                __moliDomMatrixReadOnlyBrand: true,
                __moliDomMatrixM11: 1,
                __moliDomMatrixM12: 2,
                __moliDomMatrixM21: 3,
                __moliDomMatrixM22: 4,
                __moliDomMatrixM41: 5,
                __moliDomMatrixM42: 6
              });
              const readonlyDescriptor = Object.getOwnPropertyDescriptor(DOMMatrixReadOnly.prototype, "a");
              const mutableDescriptor = Object.getOwnPropertyDescriptor(DOMMatrix.prototype, "a");
              return JSON.stringify({
                matrixInstance: [
                  matrix instanceof DOMMatrix,
                  matrix instanceof DOMMatrixReadOnly,
                  Object.prototype.toString.call(matrix)
                ].join(","),
                matrixValues: [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f].join(","),
                readonlyInstance: [
                  readonly instanceof DOMMatrixReadOnly,
                  readonly instanceof DOMMatrix,
                  Object.prototype.toString.call(readonly)
                ].join(","),
                readonlyValues: [readonly.a, readonly.b, readonly.c, readonly.d, readonly.e, readonly.f].join(","),
                initialMatrixSlots,
                initialReadonlySlots,
                matrixSpoofSlots: internalNames(matrix),
                readonlySpoofSlots: internalNames(readonly),
                fakeResults: [
                  probe(() => readonlyDescriptor.get.call(receiver)),
                  probe(() => DOMMatrixReadOnly.prototype.toJSON.call(receiver)),
                  probe(() => DOMMatrixReadOnly.prototype.translate.call(receiver, 1, 2)),
                  probe(() => mutableDescriptor.get.call(fake)),
                  probe(() => mutableDescriptor.set.call(fake, 2)),
                  probe(() => DOMMatrix.prototype.translateSelf.call(fake, 1, 2)),
                  probe(() => readonlyDescriptor.get.call(readonlyFake)),
                  probe(() => DOMMatrixReadOnly.prototype.toFloat32Array.call(readonlyFake))
                ].join(","),
                fakeSlots: internalNames(fake),
                readonlyFakeSlots: internalNames(readonlyFake),
                readonlyKeys: Object.keys(DOMMatrixReadOnly.prototype)
                  .filter(name => readonlyNames.includes(name))
                  .join(","),
                mutableKeys: Object.keys(DOMMatrix.prototype)
                  .filter(name => mutableNames.includes(name))
                  .join(","),
                readonlyDescriptors: ["a", "m44", "is2D", "isIdentity"]
                  .map(name => descriptorShape(DOMMatrixReadOnly.prototype, readonly, name))
                  .join(";"),
                mutableDescriptors: ["a", "m44"]
                  .map(name => descriptorShape(DOMMatrix.prototype, matrix, name))
                  .join(";")
              });
            })()
            "#,
        )
        .expect("DOMMatrix declared-slot probe should evaluate");

    assert_eq!(
        result,
        r#"{"matrixInstance":"true,true,[object DOMMatrix]","matrixValues":"1,2,3,4,5,6","readonlyInstance":"true,false,[object DOMMatrixReadOnly]","readonlyValues":"1,2,3,4,5,6","initialMatrixSlots":"","initialReadonlySlots":"","matrixSpoofSlots":"__moliDomMatrixM11,__moliDomMatrixM12,__moliDomMatrixM21,__moliDomMatrixM22,__moliDomMatrixM41,__moliDomMatrixM42,__moliDomMatrixMutableBrand,__moliDomMatrixReadOnlyBrand","readonlySpoofSlots":"__moliDomMatrixM11,__moliDomMatrixM12,__moliDomMatrixM21,__moliDomMatrixM22,__moliDomMatrixM41,__moliDomMatrixM42,__moliDomMatrixReadOnlyBrand","fakeResults":"TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError,TypeError","fakeSlots":"__moliDomMatrixM11,__moliDomMatrixM12,__moliDomMatrixM21,__moliDomMatrixM22,__moliDomMatrixM41,__moliDomMatrixM42,__moliDomMatrixMutableBrand,__moliDomMatrixReadOnlyBrand","readonlyFakeSlots":"__moliDomMatrixM11,__moliDomMatrixM12,__moliDomMatrixM21,__moliDomMatrixM22,__moliDomMatrixM41,__moliDomMatrixM42,__moliDomMatrixReadOnlyBrand","readonlyKeys":"a,b,c,d,e,f,m11,m12,m13,m14,m21,m22,m23,m24,m31,m32,m33,m34,m41,m42,m43,m44,is2D,isIdentity","mutableKeys":"a,b,c,d,e,f,m11,m12,m13,m14,m21,m22,m23,m24,m31,m32,m33,m34,m41,m42,m43,m44","readonlyDescriptors":"a:function:get a:0:undefined:undefined:undefined:true:true:false;m44:function:get m44:0:undefined:undefined:undefined:true:true:false;is2D:function:get is2D:0:undefined:undefined:undefined:true:true:false;isIdentity:function:get isIdentity:0:undefined:undefined:undefined:true:true:false","mutableDescriptors":"a:function:get a:0:function:set a:1:true:true:false;m44:function:get m44:0:function:set m44:1:true:true:false"}"#
    );
}
#[test]
fn toggle_event_init_uses_webidl_dom_string_conversion() {
    let mut vm = new_storage_test_vm("https://toggle-event-init.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = fn => {
                try {
                  return fn();
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const source = document.createElement("button");
              const nullish = new ToggleEvent("toggle", {
                oldState: null,
                newState: undefined,
                source
              });
              const defaults = new ToggleEvent("toggle", {});
              const missing = new ToggleEvent("toggle");
              const readonly = probe(() => {
                nullish.oldState = "changed";
                return nullish.oldState;
              });
              return JSON.stringify({
                oldNull: nullish.oldState,
                newUndefined: nullish.newState,
                sourceMatches: nullish.source === source,
                defaultOld: defaults.oldState,
                defaultNew: defaults.newState,
                defaultSource: defaults.source === null,
                missingOld: missing.oldState,
                missingNew: missing.newState,
                readonly,
                symbol: probe(() => new ToggleEvent("toggle", { oldState: Symbol("x") }))
              });
            })()
            "#,
        )
        .expect("ToggleEventInit conversion probe should evaluate");

    assert_eq!(
        result,
        r#"{"oldNull":"null","newUndefined":"","sourceMatches":true,"defaultOld":"","defaultNew":"","defaultSource":true,"missingOld":"","missingNew":"","readonly":"null","symbol":"throw:TypeError"}"#
    );
}
#[test]
fn navigator_prototype_declared_methods_keep_descriptors_and_behavior() {
    let mut vm = new_storage_test_vm("https://navigator-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const proto = Navigator.prototype;
              const names = ["javaEnabled", "sendBeacon", "getBattery", "vibrate"];
              const fakeNavigator = Object.create(proto);
              const summarize = name => {
                const descriptor = Object.getOwnPropertyDescriptor(proto, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const typeErrorName = callback => {
                try {
                  callback();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              };
              const sendBeaconMissingArg = (() => {
                try {
                  navigator.sendBeacon();
                  return "ok";
                } catch (error) {
                  return error && error.name;
                }
              })();
              let iteratorReads = 0;
              const iterablePattern = {
                get [Symbol.iterator]() {
                  iteratorReads++;
                  return [1, 2, 3][Symbol.iterator];
                }
              };
              const vibrateMissingArg = typeErrorName(() => navigator.vibrate());
              const vibrateInvalidValues = [
                undefined,
                null,
                "one",
                new String("one"),
                NaN,
                {},
                iterablePattern
              ].map(value => typeErrorName(() => navigator.vibrate(value))).join(",");
              const batteryPromise = navigator.getBattery();
              const fakeBatteryPromise = proto.getBattery.call(fakeNavigator);
              globalThis.__navigatorPrototypeReceiverProbe = {
                fakeBatteryReject: "pending"
              };
              fakeBatteryPromise.then(
                () => {
                  globalThis.__navigatorPrototypeReceiverProbe.fakeBatteryReject = "resolved";
                },
                error => {
                  globalThis.__navigatorPrototypeReceiverProbe.fakeBatteryReject =
                    error && error.name;
                }
              );
              const cookieEnabledGetter =
                Object.getOwnPropertyDescriptor(proto, "cookieEnabled").get;
              return JSON.stringify({
                descriptors: names.map(summarize).join("|"),
                prototypeKeys: Object.keys(proto)
                  .filter(name => names.includes(name))
                  .join(","),
                navigatorOwnMethods: names
                  .map(name => Object.hasOwn(navigator, name))
                  .join(","),
                javaEnabledResult: navigator.javaEnabled(),
                sendBeaconMissingArg,
                vibrateMissingArg,
                vibrateInvalidValues,
                vibrateIteratorReads: iteratorReads,
                vibrateWithoutActivation: navigator.vibrate([1, 2, 3]),
                fakeJavaEnabled: typeErrorName(() => proto.javaEnabled.call(fakeNavigator)),
                fakeSendBeacon: typeErrorName(() => proto.sendBeacon.call(fakeNavigator, "/beacon")),
                fakeVibrate: typeErrorName(() => proto.vibrate.call(fakeNavigator, 1)),
                fakeCookieEnabled: typeErrorName(() => cookieEnabledGetter.call(fakeNavigator)),
                batteryPromiseTag: Object.prototype.toString.call(batteryPromise),
                batteryThenType: typeof batteryPromise.then,
                fakeBatteryPromiseTag: Object.prototype.toString.call(fakeBatteryPromise)
              });
            })()
            "#,
        )
        .expect("navigator prototype method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":"true:function:javaEnabled:0:true:true:true|true:function:sendBeacon:1:true:true:true|true:function:getBattery:0:true:true:true|true:function:vibrate:1:true:true:true","prototypeKeys":"javaEnabled,sendBeacon,getBattery,vibrate","navigatorOwnMethods":"false,false,false,false","javaEnabledResult":false,"sendBeaconMissingArg":"TypeError","vibrateMissingArg":"TypeError","vibrateInvalidValues":"ok,ok,ok,ok,ok,ok,ok","vibrateIteratorReads":1,"vibrateWithoutActivation":false,"fakeJavaEnabled":"TypeError","fakeSendBeacon":"TypeError","fakeVibrate":"TypeError","fakeCookieEnabled":"TypeError","batteryPromiseTag":"[object Promise]","batteryThenType":"function","fakeBatteryPromiseTag":"[object Promise]"}"#
    );

    let activated_vibration = vm
        .evaluate_expression_payload_with_await("navigator.vibrate([1, 2, 3])", false, true)
        .expect("activated navigator.vibrate probe should complete");
    assert_eq!(activated_vibration["type"], "boolean");
    assert_eq!(activated_vibration["value"], true);

    let receiver_result = vm
        .eval("JSON.stringify(globalThis.__navigatorPrototypeReceiverProbe)")
        .expect("navigator prototype receiver promise should settle");
    assert_eq!(receiver_result, r#"{"fakeBatteryReject":"TypeError"}"#);
}
#[test]
fn navigator_permissions_query_reparses_permission_specific_descriptors() {
    let mut vm = new_storage_test_vm("https://permission-descriptor-conversion.test/");

    vm.eval(
        r#"
            (() => {
              let midiNameReads = 0;
              let midiSysexReads = 0;
              let geolocationNameReads = 0;
              globalThis.__permissionDescriptorConversionProbe = "pending";
              Promise.all([
                navigator.permissions.query({
                  get name() {
                    midiNameReads++;
                    return "midi";
                  },
                  get sysex() {
                    midiSysexReads++;
                    return true;
                  }
                }),
                navigator.permissions.query({
                  get name() {
                    geolocationNameReads++;
                    return "geolocation";
                  }
                })
              ]).then(([midi, geolocation]) => {
                globalThis.__permissionDescriptorConversionProbe = [
                  midi instanceof PermissionStatus,
                  midi.name,
                  midi.state,
                  midiNameReads,
                  midiSysexReads,
                  geolocation.name,
                  geolocationNameReads
                ].join("|");
              });
            })()
            "#,
    )
    .expect("permission descriptor conversion probe should evaluate");

    let result = vm
        .eval("String(globalThis.__permissionDescriptorConversionProbe)")
        .expect("permission descriptor conversion promise should settle");

    assert_eq!(result, "true|midi|prompt|2|1|geolocation|1");
}
#[test]
fn navigator_declared_objects_keep_brand_and_enumerable_members() {
    let mut vm = new_storage_test_vm("https://navigator-declared-objects.test/");

    vm.eval(
        r#"
            (() => {
              globalThis.__navigatorDeclaredObjectProbe = {
                ua: "pending",
                json: "pending",
                highEntropy: "pending",
                storage: "pending"
              };
              const ua = navigator.userAgentData;
              globalThis.__navigatorDeclaredObjectProbe.ua = [
                ua instanceof NavigatorUAData,
                Object.prototype.toString.call(ua),
                Object.keys(ua).join(","),
                ua.brands.length > 0,
                Object.keys(ua.brands[0]).join(",")
              ].join("|");
              const json = ua.toJSON();
              globalThis.__navigatorDeclaredObjectProbe.json = [
                Object.prototype.toString.call(json),
                Object.keys(json).join(","),
                json.brands.length > 0,
                Object.keys(json.brands[0]).join(",")
              ].join("|");
              Promise.all([
                ua.getHighEntropyValues([]),
                ua.getHighEntropyValues([
                  "architecture",
                  "fullVersionList",
                  "formFactors",
                  "uaFullVersion"
                ])
              ]).then(([empty, high]) => {
                globalThis.__navigatorDeclaredObjectProbe.highEntropy = [
                  Object.prototype.toString.call(high),
                  Object.keys(empty).join(","),
                  Object.keys(high).join(","),
                  high.fullVersionList
                    .map(({ brand, version }) => `${brand}:${version}`)
                    .join(","),
                  high.architecture,
                  high.uaFullVersion,
                  high.formFactors.join(","),
                  "bitness" in high,
                  "formFactor" in high
                ].join("|");
              });
              navigator.storage.estimate().then((estimate) => {
                globalThis.__navigatorDeclaredObjectProbe.storage = [
                  estimate instanceof StorageEstimate,
                  Object.prototype.toString.call(estimate),
                  Object.keys(estimate).join(","),
                  estimate.quota,
                  estimate.usage,
                  Object.prototype.toString.call(estimate.usageDetails),
                  Object.keys(estimate.usageDetails).join(",")
                ].join("|");
              });
            })()
            "#,
    )
    .expect("navigator declared object probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__navigatorDeclaredObjectProbe)")
        .expect("navigator declared object promises should settle");

    assert_eq!(
        result,
        r#"{"ua":"true|[object NavigatorUAData]|brands,mobile,platform|true|brand,version","json":"[object Object]|brands,mobile,platform|true|brand,version","highEntropy":"[object Object]|brands,mobile,platform|architecture,brands,formFactors,fullVersionList,mobile,platform,uaFullVersion|Not:A-Brand:99.0.0.0,Google Chrome:145.0.0.0,Chromium:145.0.0.0|x86|145.0.0.0|Desktop|false|false","storage":"true|[object StorageEstimate]|quota,usage,usageDetails|1073741824|0|[object Object]|"}"#
    );
}
#[test]
fn navigator_runtime_subobjects_keep_declared_brand_and_methods() {
    let mut vm = new_storage_test_vm("https://navigator-subobjects.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const permissions = navigator.permissions;
              const storage = navigator.storage;
              const mediaDevices = navigator.mediaDevices;
              const clipboard = navigator.clipboard;
              const userActivation = navigator.userActivation;
              const connection = navigator.connection;
              const fakeUserActivation = Object.create(userActivation);
              const fakeConnection = Object.create(connection);
              const summarizeOwnMethod = (object, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const getterOutcome = (object, receiver, name) => {
                const getter = Object.getOwnPropertyDescriptor(object, name).get;
                try {
                  return String(getter.call(receiver));
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const methodOutcome = (object, receiver, name, ...args) => {
                try {
                  return String(object[name].call(receiver, ...args));
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              globalThis.__storageEstimateIllegalReceiverProbe = "pending";
              globalThis.__storageEstimateSpoofedReceiverProbe = "pending";
              let illegalReceiverResult = "";
              try {
                const promise = StorageManager.prototype.estimate.call({});
                illegalReceiverResult = Object.prototype.toString.call(promise);
                promise.then(
                  () => { globalThis.__storageEstimateIllegalReceiverProbe = "resolved"; },
                  (error) => { globalThis.__storageEstimateIllegalReceiverProbe = error && error.name; }
                );
              } catch (error) {
                illegalReceiverResult = `throw:${error.name}`;
              }
              let spoofedReceiverResult = "";
              try {
                const promise = StorageManager.prototype.estimate.call({
                  __moliStorageManagerBrand: true
                });
                spoofedReceiverResult = Object.prototype.toString.call(promise);
                promise.then(
                  () => { globalThis.__storageEstimateSpoofedReceiverProbe = "resolved"; },
                  (error) => { globalThis.__storageEstimateSpoofedReceiverProbe = error && error.name; }
                );
              } catch (error) {
                spoofedReceiverResult = `throw:${error.name}`;
              }
              storage.persisted().then((persisted) => {
                globalThis.__storagePersistedProbe = persisted;
              });
              storage.persist().then((persisted) => {
                globalThis.__storagePersistProbe = persisted;
              });
              clipboard.writeText("clip-text").then(() => clipboard.readText()).then((text) => {
                globalThis.__clipboardTextProbe = text;
              });
              return JSON.stringify({
                permissions: [
                  permissions instanceof Permissions,
                  Object.prototype.toString.call(permissions),
                  Object.hasOwn(permissions, "query"),
                  typeof permissions.query,
                  permissions.query && permissions.query.length
                ].join("|"),
                storage: [
                  storage instanceof StorageManager,
                  Object.prototype.toString.call(storage),
                  Object.hasOwn(storage, "estimate"),
                  Object.hasOwn(storage, "persisted"),
                  Object.hasOwn(storage, "persist"),
                  Object.hasOwn(storage, "getDirectory"),
                  Object.keys(storage).join(","),
                  Object.hasOwn(StorageManager.prototype, "persisted"),
                  Object.hasOwn(StorageManager.prototype, "persist"),
                  Object.hasOwn(StorageManager.prototype, "estimate"),
                  Object.hasOwn(StorageManager.prototype, "getDirectory"),
                  storage.persisted && storage.persisted.name,
                  storage.persisted && storage.persisted.length,
                  storage.persist && storage.persist.name,
                  storage.persist && storage.persist.length,
                  storage.estimate && storage.estimate.name,
                  storage.estimate && storage.estimate.length,
                  summarizeOwnMethod(storage, "estimate"),
                  illegalReceiverResult,
                  spoofedReceiverResult
                ].join("|"),
                connection: [
                  Object.prototype.toString.call(connection),
                  typeof connection.type,
                  connection.type,
                  connection.downlinkMax,
                  typeof connection.effectiveType,
                  connection.effectiveType,
                  connection.downlink,
                  connection.rtt,
                  connection.saveData,
                  connection.onchange === null,
                  Object.keys(connection).join(","),
                  Object.getOwnPropertyNames(connection)
                    .filter(name => name.startsWith("__moliNavigatorConnection"))
                    .join(","),
                  summarizeOwnMethod(connection, "addEventListener"),
                  summarizeOwnMethod(connection, "removeEventListener"),
                  String(connection.addEventListener("change", () => {})),
                  String(connection.removeEventListener("change", () => {})),
                  methodOutcome(connection, fakeConnection, "addEventListener", "change", () => {}),
                  methodOutcome(connection, fakeConnection, "removeEventListener", "change", () => {})
                ].join("|"),
                mediaDevices: [
                  mediaDevices instanceof MediaDevices,
                  Object.prototype.toString.call(mediaDevices),
                  Object.hasOwn(mediaDevices, "enumerateDevices"),
                  Object.hasOwn(mediaDevices, "getUserMedia"),
                  Object.keys(mediaDevices).join(","),
                  mediaDevices.enumerateDevices && mediaDevices.enumerateDevices.name,
                  mediaDevices.enumerateDevices && mediaDevices.enumerateDevices.length,
                  mediaDevices.getUserMedia && mediaDevices.getUserMedia.name,
                  mediaDevices.getUserMedia && mediaDevices.getUserMedia.length,
                  summarizeOwnMethod(MediaDevices.prototype, "enumerateDevices"),
                  summarizeOwnMethod(MediaDevices.prototype, "getUserMedia")
                ].join("|"),
                clipboard: [
                  clipboard instanceof Clipboard,
                  clipboard instanceof EventTarget,
                  Object.prototype.toString.call(clipboard),
                  Object.hasOwn(clipboard, "readText"),
                  Object.hasOwn(clipboard, "writeText"),
                  Object.keys(clipboard).join(","),
                  Object.hasOwn(Clipboard.prototype, "read"),
                  Object.hasOwn(Clipboard.prototype, "readText"),
                  Object.hasOwn(Clipboard.prototype, "write"),
                  Object.hasOwn(Clipboard.prototype, "writeText"),
                  clipboard.readText && clipboard.readText.name,
                  clipboard.readText && clipboard.readText.length,
                  clipboard.writeText && clipboard.writeText.name,
                  clipboard.writeText && clipboard.writeText.length,
                  summarizeOwnMethod(Clipboard.prototype, "readText"),
                  summarizeOwnMethod(Clipboard.prototype, "writeText"),
                  Object.prototype.toString.call(clipboard.readText()),
                  Object.prototype.toString.call(clipboard.writeText("clip-text"))
                ].join("|"),
                userActivation: [
                  typeof globalThis.UserActivation,
                  Object.prototype.toString.call(userActivation),
                  typeof userActivation.isActive,
                  userActivation.isActive,
                  userActivation.hasBeenActive,
                  Object.keys(userActivation).join(","),
                  Object.hasOwn(Object.getPrototypeOf(navigator), "userActivation"),
                  getterOutcome(userActivation, fakeUserActivation, "isActive"),
                  getterOutcome(userActivation, fakeUserActivation, "hasBeenActive")
                ].join("|")
              });
            })()
            "#,
        )
        .expect("navigator runtime subobject probe should evaluate");

    assert_eq!(
        result,
        r#"{"permissions":"true|[object Permissions]|false|function|1","storage":"true|[object StorageManager]|false|false|false|false||true|true|true|true|persisted|0|persist|0|estimate|0|false:undefined:::::|[object Promise]|[object Promise]","connection":"[object Object]|string|unknown|Infinity|string|4g|10|50|false|true|type,downlinkMax,effectiveType,downlink,rtt,saveData,onchange,addEventListener,removeEventListener||true:function:addEventListener:2:true:true:true|true:function:removeEventListener:2:true:true:true|undefined|undefined|throw:TypeError|throw:TypeError","mediaDevices":"true|[object MediaDevices]|false|false||enumerateDevices|0|getUserMedia|1|true:function:enumerateDevices:0:true:true:true|true:function:getUserMedia:1:true:true:true","clipboard":"true|true|[object Clipboard]|false|false||true|true|true|true|readText|0|writeText|1|true:function:readText:0:true:true:true|true:function:writeText:1:true:true:true|[object Promise]|[object Promise]","userActivation":"undefined|[object Object]|boolean|false|false|isActive,hasBeenActive|true|throw:TypeError|throw:TypeError"}"#
    );
    let receiver_errors = vm
        .eval(
            r#"[globalThis.__storageEstimateIllegalReceiverProbe, globalThis.__storageEstimateSpoofedReceiverProbe].join("|")"#,
        )
        .expect("StorageManager illegal receiver promises should reject");
    assert_eq!(receiver_errors, "TypeError|TypeError");
    let persisted = vm
        .eval("String(globalThis.__storagePersistedProbe)")
        .expect("StorageManager.persisted promise should settle");
    assert_eq!(persisted, "false");
    let persist = vm
        .eval("String(globalThis.__storagePersistProbe)")
        .expect("StorageManager.persist promise should settle");
    assert_eq!(persist, "false");
    let clipboard_text = vm
        .eval("String(globalThis.__clipboardTextProbe)")
        .expect("Clipboard readText/writeText promises should settle");
    assert_eq!(clipboard_text, "clip-text");
}
#[test]
fn navigator_runtime_subobject_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://navigator-subobject-prototype-methods.test/");

    vm.eval(
        r#"
            (() => {
              const summarizeMethod = (prototype, name, expectedLength) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  expectedLength,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":");
              };
              const permissions = navigator.permissions;
              const ua = navigator.userAgentData;
              const storage = navigator.storage;
              const fakePermissions = Object.create(Permissions.prototype);
              const fakeUa = Object.create(NavigatorUAData.prototype);
              const illegalUaJson = (() => {
                try {
                  NavigatorUAData.prototype.toJSON.call(fakeUa);
                  return "accepted";
                } catch (error) {
                  return error && error.name;
                }
              })();
              globalThis.__navigatorRuntimePrototypeProbe = {
                descriptors: [
                  summarizeMethod(Permissions.prototype, "query", 1),
                  summarizeMethod(NavigatorUAData.prototype, "toJSON", 0),
                  summarizeMethod(NavigatorUAData.prototype, "getHighEntropyValues", 1),
                  summarizeMethod(StorageManager.prototype, "persisted", 0),
                  summarizeMethod(StorageManager.prototype, "persist", 0),
                  summarizeMethod(StorageManager.prototype, "estimate", 0),
                  summarizeMethod(StorageManager.prototype, "getDirectory", 0)
                ],
                own: [
                  Object.hasOwn(permissions, "query"),
                  Object.hasOwn(ua, "toJSON"),
                  Object.hasOwn(ua, "getHighEntropyValues"),
                  Object.hasOwn(storage, "persisted"),
                  Object.hasOwn(storage, "persist"),
                  Object.hasOwn(storage, "estimate"),
                  Object.hasOwn(storage, "getDirectory")
                ].join(":"),
                enumerable: [
                  Object.keys(Permissions.prototype).join(","),
                  Object.keys(NavigatorUAData.prototype).join(","),
                  Object.keys(StorageManager.prototype).join(",")
                ].join("|"),
                permissionsIllegal: "pending",
                uaIllegal: `toJSON:${illegalUaJson}`
              };
              Permissions.prototype.query.call(permissions, { name: "geolocation" }).then((status) => {
                globalThis.__navigatorRuntimePrototypeProbe.permission = [
                  status instanceof PermissionStatus,
                  status.name,
                  status.state
                ].join(":");
              });
              Permissions.prototype.query.call(fakePermissions, { name: "geolocation" }).then(
                () => {
                  globalThis.__navigatorRuntimePrototypeProbe.permissionsIllegal = "resolved";
                },
                error => {
                  globalThis.__navigatorRuntimePrototypeProbe.permissionsIllegal = error && error.name;
                }
              );
              NavigatorUAData.prototype.getHighEntropyValues.call(ua, ["architecture"]).then((high) => {
                const json = NavigatorUAData.prototype.toJSON.call(ua);
                globalThis.__navigatorRuntimePrototypeProbe.ua = [
                  Object.keys(json).join(","),
                  Object.keys(high).join(","),
                  high.architecture,
                  "bitness" in high,
                  "formFactors" in high
                ].join(":");
              });
              NavigatorUAData.prototype.getHighEntropyValues.call(fakeUa, ["architecture"]).then(
                () => {
                  globalThis.__navigatorRuntimePrototypeProbe.uaIllegal += "|high:resolved";
                },
                error => {
                  globalThis.__navigatorRuntimePrototypeProbe.uaIllegal += `|high:${error && error.name}`;
                }
              );
              StorageManager.prototype.estimate.call(storage).then((estimate) => {
                globalThis.__navigatorRuntimePrototypeProbe.storage = [
                  estimate instanceof StorageEstimate,
                  estimate.quota,
                  estimate.usage
                ].join(":");
              });
            })()
            "#,
    )
    .expect("navigator runtime subobject prototype method probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__navigatorRuntimePrototypeProbe)")
        .expect("navigator runtime subobject prototype method promises should settle");

    assert_eq!(
        result,
        r#"{"descriptors":["query:function:query:1:1:true:true:true","toJSON:function:toJSON:0:0:true:true:true","getHighEntropyValues:function:getHighEntropyValues:1:1:true:true:true","persisted:function:persisted:0:0:true:true:true","persist:function:persist:0:0:true:true:true","estimate:function:estimate:0:0:true:true:true","getDirectory:function:getDirectory:0:0:true:true:true"],"own":"false:false:false:false:false:false:false","enumerable":"query|toJSON,getHighEntropyValues|persisted,persist,estimate,getDirectory","permissionsIllegal":"TypeError","uaIllegal":"toJSON:TypeError|high:TypeError","permission":"true:geolocation:prompt","ua":"brands,mobile,platform:architecture,brands,mobile,platform:x86:false:false","storage":"true:1073741824:0"}"#
    );
}
#[test]
fn navigator_plugin_collection_lengths_are_branded_readonly_prototype_attributes() {
    let mut vm = new_storage_test_vm("https://navigator-collections-length.test/");
    let result = vm.eval(r#"
        (() => {
          const collections = [navigator.plugins, navigator.mimeTypes, navigator.plugins[0]];
          function throwsTypeError(callback) {
            try { callback(); return false; } catch (error) { return error instanceof TypeError; }
          }
          for (const [index, collection] of collections.entries()) {
            const prototype = Object.getPrototypeOf(collection);
            const descriptor = Object.getOwnPropertyDescriptor(prototype, 'length');
            if (!descriptor || descriptor.get.name !== 'get length' || descriptor.get.length !== 0 ||
                descriptor.set !== undefined || !descriptor.enumerable || !descriptor.configurable ||
                Object.hasOwn(collection, 'length') || Array.isArray(collection)) return 'descriptor';
            const length = index === 0 ? 5 : 2;
            const first = collection[0];
            collection.length = 0;
            collection.__moliNavigatorCollectionLength = 0;
            if (collection.length !== length || collection[0] !== first ||
                Array.from(collection).length !== length || collection.item(0) !== first)
              return 'assignment changed collection';
            if (!throwsTypeError(() => { 'use strict'; collection.length = 0; })) return 'strict';
            for (const fake of [{}, [], Object.create(collection), prototype, collections[(index + 1) % 3]]) {
              if (!throwsTypeError(() => descriptor.get.call(fake))) return 'brand';
            }
          }
          const iframe = document.createElement('iframe');
          document.appendChild(document.createElement('html')).appendChild(iframe);
          const foreign = iframe.contentWindow.navigator;
          const foreignCollections = [foreign.plugins, foreign.mimeTypes, foreign.plugins[0]];
          return collections.every((collection, i) =>
            Object.getOwnPropertyDescriptor(Object.getPrototypeOf(collection), 'length')
              .get.call(foreignCollections[i]) === collection.length);
        })()
    "#).expect("collection length probe should evaluate");
    assert_eq!(result, "true");
}
#[test]
fn navigator_plugin_collections_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://navigator-collections-webidl.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function probe(callback) {
                try {
                  const value = callback();
                  if (value && value.name) return value.name;
                  if (value && value.type) return value.type;
                  return String(value);
                } catch (error) {
                  return 'throw:' + error.name;
                }
              }
              const plugins = navigator.plugins;
              const plugin = plugins[0];
              const mimeTypes = navigator.mimeTypes;
              return [
                probe(() => plugins.item('1.9')),
                probe(() => plugins.item(-1)),
                probe(() => plugins.item()),
                probe(() => plugins.item(Symbol())),
                probe(() => plugins.namedItem(null)),
                probe(() => plugins.namedItem(undefined)),
                probe(() => plugins.namedItem()),
                probe(() => plugins.namedItem(Symbol())),
                probe(() => plugins.namedItem({ toString() { throw new RangeError('name'); } })),
                probe(() => mimeTypes.item({ valueOf() { return 1; } })),
                probe(() => mimeTypes.namedItem('application/pdf')),
                probe(() => plugin.item('1')),
                probe(() => plugin.namedItem('text/pdf'))
              ].join('|');
            })()
            "#,
        )
        .expect("navigator plugin collection WebIDL probes should evaluate");

    assert_eq!(
        result,
        "Chrome PDF Viewer|null|throw:TypeError|throw:TypeError|null|null|throw:TypeError|throw:TypeError|throw:RangeError|text/pdf|application/pdf|text/pdf|text/pdf"
    );
}
#[test]
fn navigator_plugin_collections_declared_fixed_members_keep_descriptors() {
    let mut vm = new_storage_test_vm("https://navigator-collections-descriptors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const plugins = navigator.plugins;
              const plugin = plugins[0];
              const mimeTypes = navigator.mimeTypes;
              const fakePlugins = Object.create(plugins);
              const fakePlugin = Object.create(plugin);
              const fakeMimeTypes = Object.create(mimeTypes);
              fakePlugins[0] = plugin;
              fakePlugins["PDF Viewer"] = plugin;
              fakePlugins.__moliPluginArrayBrand = true;
              fakePlugin[0] = plugin[0];
              fakePlugin["application/pdf"] = plugin[0];
              fakePlugin.__moliPluginBrand = true;
              fakeMimeTypes[0] = mimeTypes[0];
              fakeMimeTypes["application/pdf"] = mimeTypes[0];
              fakeMimeTypes.__moliMimeTypeArrayBrand = true;
              function methodDescriptor(object, name) {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable,
                  descriptor.value.name,
                  descriptor.value.length
                ].join(',');
              }
              function valueDescriptor(object, name) {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
                return [
                  descriptor.enumerable,
                  descriptor.writable,
                  descriptor.configurable,
                  String(descriptor.value)
                ].join(',');
              }
              function outcome(callback) {
                try {
                  const value = callback();
                  return value === undefined ? 'undefined' : String(value);
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              }
              return JSON.stringify({
                pluginsPrototype: Object.getPrototypeOf(plugins) === PluginArray.prototype,
                pluginPrototype: Object.getPrototypeOf(plugin) === Plugin.prototype,
                mimeTypesPrototype: Object.getPrototypeOf(mimeTypes) === MimeTypeArray.prototype,
                pluginsItem: methodDescriptor(PluginArray.prototype, 'item'),
                pluginsNamedItem: methodDescriptor(PluginArray.prototype, 'namedItem'),
                pluginsRefresh: methodDescriptor(PluginArray.prototype, 'refresh'),
                pluginsIterator: methodDescriptor(PluginArray.prototype, Symbol.iterator),
                pluginItem: methodDescriptor(Plugin.prototype, 'item'),
                pluginNamedItem: methodDescriptor(Plugin.prototype, 'namedItem'),
                pluginIterator: methodDescriptor(Plugin.prototype, Symbol.iterator),
                mimeTypesItem: methodDescriptor(MimeTypeArray.prototype, 'item'),
                mimeTypesNamedItem: methodDescriptor(MimeTypeArray.prototype, 'namedItem'),
                mimeTypesIterator: methodDescriptor(MimeTypeArray.prototype, Symbol.iterator),
                iteratorIdentity: [
                  PluginArray.prototype,
                  Plugin.prototype,
                  MimeTypeArray.prototype
                ].every(prototype => prototype[Symbol.iterator] === Array.prototype.values),
                mimeTypesRefresh: Object.prototype.hasOwnProperty.call(mimeTypes, 'refresh'),
                pluginName: valueDescriptor(plugin, 'name'),
                pluginFilename: valueDescriptor(plugin, 'filename'),
                pluginDescription: valueDescriptor(plugin, 'description'),
                pluginFixedNames: Object.getOwnPropertyNames(plugin)
                  .filter(name => ['description', 'filename', 'item', 'name', 'namedItem'].includes(name))
                  .sort(),
                pluginArrayFixedNames: Object.getOwnPropertyNames(plugins)
                  .filter(name => ['item', 'namedItem', 'refresh'].includes(name))
                  .sort(),
                privateSlotNames: [
                  plugins,
                  plugin,
                  mimeTypes
                ].map(object => Object.getOwnPropertyNames(object)
                  .filter(name => name.startsWith('__moli'))
                  .join(',')).join('|'),
                realRefresh: outcome(() => plugins.refresh()),
                fakeReceivers: [
                  outcome(() => plugins.item.call(fakePlugins, 0)),
                  outcome(() => plugins.namedItem.call(fakePlugins, 'PDF Viewer')),
                  outcome(() => plugins.refresh.call(fakePlugins)),
                  outcome(() => plugin.item.call(fakePlugin, 0)),
                  outcome(() => plugin.namedItem.call(fakePlugin, 'application/pdf')),
                  outcome(() => mimeTypes.item.call(fakeMimeTypes, 0)),
                  outcome(() => mimeTypes.namedItem.call(fakeMimeTypes, 'application/pdf'))
                ].join('|'),
                crossReceivers: [
                  outcome(() => plugins.item.call(mimeTypes, 0)),
                  outcome(() => plugins.namedItem.call(plugin, 'PDF Viewer')),
                  outcome(() => plugins.refresh.call(plugin)),
                  outcome(() => plugin.item.call(plugins, 0)),
                  outcome(() => plugin.namedItem.call(mimeTypes, 'application/pdf')),
                  outcome(() => mimeTypes.item.call(plugin, 0)),
                  outcome(() => mimeTypes.namedItem.call(plugins, 'application/pdf'))
                ].join('|')
              });
            })()
            "#,
        )
        .expect("navigator plugin descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"pluginsPrototype":true,"pluginPrototype":true,"mimeTypesPrototype":true,"pluginsItem":"true,true,true,item,1","pluginsNamedItem":"true,true,true,namedItem,1","pluginsRefresh":"true,true,true,refresh,0","pluginsIterator":"false,true,true,values,0","pluginItem":"true,true,true,item,1","pluginNamedItem":"true,true,true,namedItem,1","pluginIterator":"false,true,true,values,0","mimeTypesItem":"true,true,true,item,1","mimeTypesNamedItem":"true,true,true,namedItem,1","mimeTypesIterator":"false,true,true,values,0","iteratorIdentity":true,"mimeTypesRefresh":false,"pluginName":"false,true,true,PDF Viewer","pluginFilename":"false,true,true,internal-pdf-viewer","pluginDescription":"false,true,true,Portable Document Format","pluginFixedNames":["description","filename","name"],"pluginArrayFixedNames":[],"privateSlotNames":"||","realRefresh":"undefined","fakeReceivers":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","crossReceivers":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError"}"#
    );
}
#[test]
fn screen_orientation_lock_converts_webidl_enum_before_device_rejection() {
    let mut vm = new_storage_test_vm("https://screen-orientation-lock.test/");

    let result = vm
        .evaluate_expression_payload_with_await(
            r#"
            (() => {
              const orientation = screen.orientation;
              const outcome = callback => {
                let promise;
                try {
                  promise = callback();
                } catch (error) {
                  return Promise.resolve(`throw:${error && error.name}`);
                }
                return Promise.resolve(promise).then(
                  () => "resolve",
                  error => `reject:${error && error.name}${
                    error && error.message === "sentinel" ? ":sentinel" : ""
                  }`
                );
              };
              let conversions = 0;
              const validObject = {
                toString() {
                  conversions++;
                  return "portrait-primary";
                }
              };
              return Promise.all([
                outcome(() => orientation.lock("invalid-orientation")),
                outcome(() => orientation.lock(null)),
                outcome(() => orientation.lock(undefined)),
                outcome(() => orientation.lock(123)),
                outcome(() => orientation.lock(window)),
                outcome(() => orientation.lock("")),
                outcome(() => orientation.lock(true)),
                outcome(() => orientation.lock(["portrait-primary", "landscape-primary"])),
                outcome(() => orientation.lock()),
                outcome(() => orientation.lock("portrait-primary")),
                outcome(() => orientation.lock({
                  toString() {
                    throw new RangeError("sentinel");
                  }
                })),
                outcome(() => orientation.lock(validObject))
              ]).then(results => JSON.stringify({ results, conversions }));
            })()
            "#,
            true,
            false,
        )
        .expect("screen orientation lock conversion probe should settle");

    assert_eq!(result["type"], "string");
    assert_eq!(
        result["value"],
        r#"{"results":["reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:TypeError","reject:NotSupportedError","reject:RangeError:sentinel","reject:NotSupportedError"],"conversions":1}"#
    );
}
#[test]
fn touch_constructor_and_list_item_parse_webidl_args() {
    let mut vm = new_storage_test_vm("https://touch-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = document.createElement("div");
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? "undefined" : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const first = new Touch({ identifier: 0, target });
  const second = new Touch({ identifier: 1, target });
  const list = new TouchEvent("touchstart", { touches: [first, second] }).touches;
  const prototype = Object.getPrototypeOf(list);
  const itemDescriptor = Object.getOwnPropertyDescriptor(prototype, "item");
  const lengthDescriptor = Object.getOwnPropertyDescriptor(prototype, "length");
  const internalNamesBefore = Object.getOwnPropertyNames(list)
    .filter(name => name.startsWith("__lmTouchList"))
    .sort();
  list.__lmTouchListLength = 99;
  const publicInternalNames = Object.getOwnPropertyNames(list)
    .filter(name => name.startsWith("__lmTouchList"))
    .sort();
  const converted = new Touch({
    identifier: "2.9",
    target,
    clientX: "3.5",
    force: null
  });
  const nullIdentifier = new Touch({ identifier: null, target });
  return JSON.stringify({
    missingInit: probe(() => new Touch()),
    nonObjectInit: probe(() => new Touch(null)),
    missingIdentifier: probe(() => new Touch({ target })),
    missingTarget: probe(() => new Touch({ identifier: 1 })),
    nullTarget: probe(() => new Touch({ identifier: 1, target: null })),
    locationTarget: probe(() => new Touch({ identifier: 1, target: location })),
    forgedTarget: probe(() => new Touch({ identifier: 1, target: Object.create(EventTarget.prototype) })),
    forgedAbortSignalTarget: probe(() => new Touch({ identifier: 1, target: Object.create(AbortSignal.prototype) })),
    constructedEventTarget: new Touch({ identifier: 1, target: new EventTarget() }).target instanceof EventTarget,
    platformEventTargets: [
      ["window", window],
      ["document", document],
      ["abortSignal", new AbortController().signal],
      ["xhr", new XMLHttpRequest()],
      ["performance", performance],
      ["screenOrientation", screen.orientation],
      ["messagePort", new MessageChannel().port1]
    ].map(([name, eventTarget], identifier) =>
      `${name}:${probe(() => new Touch({ identifier, target: eventTarget }).target === eventTarget)}`
    ),
    symbolCoordinate: probe(() => new Touch({ identifier: 1, target, clientX: Symbol() })),
    infiniteCoordinate: probe(() => new Touch({ identifier: 1, target, clientX: Infinity })),
    converted: `${converted.identifier}|${converted.clientX}|${converted.force}|${converted.target.tagName}`,
    nullIdentifier: nullIdentifier.identifier,
    itemDescriptor: `${typeof itemDescriptor?.value}|${itemDescriptor?.value?.name}|${itemDescriptor?.value?.length}|${itemDescriptor?.enumerable}|${itemDescriptor?.writable}|${itemDescriptor?.configurable}`,
    lengthDescriptor: `${typeof lengthDescriptor?.get}|${lengthDescriptor?.get?.name}|${lengthDescriptor?.get?.length}|${typeof lengthDescriptor?.set}|${lengthDescriptor?.enumerable}|${lengthDescriptor?.configurable}`,
    internalNamesBefore,
    publicInternalNames,
    lengthAfterPublicSpoof: list.length,
    fakeLength: lengthDescriptor.get.call({ __lmTouchListLength: 9 }),
    itemMissing: probe(() => list.item()),
    itemSymbol: probe(() => list.item(Symbol())),
    itemFraction: list.item("0.9") === first,
    itemNegativeNull: list.item(-1) === null,
    itemWrap: list.item(4294967297) === second
  });
})()
"#,
        )
        .expect("Touch and TouchList should parse WebIDL arguments");

    assert_eq!(
        result,
        r#"{"missingInit":"throw:TypeError","nonObjectInit":"throw:TypeError","missingIdentifier":"throw:TypeError","missingTarget":"throw:TypeError","nullTarget":"throw:TypeError","locationTarget":"throw:TypeError","forgedTarget":"throw:TypeError","forgedAbortSignalTarget":"throw:TypeError","constructedEventTarget":true,"platformEventTargets":["window:true","document:true","abortSignal:true","xhr:true","performance:true","screenOrientation:true","messagePort:true"],"symbolCoordinate":"throw:TypeError","infiniteCoordinate":"throw:TypeError","converted":"2|3.5|0|DIV","nullIdentifier":0,"itemDescriptor":"function|item|1|true|true|true","lengthDescriptor":"function|get length|0|undefined|true|true","internalNamesBefore":[],"publicInternalNames":["__lmTouchListLength"],"lengthAfterPublicSpoof":2,"fakeLength":0,"itemMissing":"throw:TypeError","itemSymbol":"throw:TypeError","itemFraction":true,"itemNegativeNull":true,"itemWrap":true}"#
    );
}
#[test]
fn node_constants_are_declared_on_constructor_and_prototype() {
    let mut vm = new_storage_test_vm("https://node-constants-declared.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const constants = [
                ["ELEMENT_NODE", 1],
                ["ATTRIBUTE_NODE", 2],
                ["TEXT_NODE", 3],
                ["CDATA_SECTION_NODE", 4],
                ["ENTITY_REFERENCE_NODE", 5],
                ["ENTITY_NODE", 6],
                ["PROCESSING_INSTRUCTION_NODE", 7],
                ["COMMENT_NODE", 8],
                ["DOCUMENT_NODE", 9],
                ["DOCUMENT_TYPE_NODE", 10],
                ["DOCUMENT_FRAGMENT_NODE", 11],
                ["NOTATION_NODE", 12],
                ["DOCUMENT_POSITION_DISCONNECTED", 1],
                ["DOCUMENT_POSITION_PRECEDING", 2],
                ["DOCUMENT_POSITION_FOLLOWING", 4],
                ["DOCUMENT_POSITION_CONTAINS", 8],
                ["DOCUMENT_POSITION_CONTAINED_BY", 16],
                ["DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32]
              ];
              const descriptorShape = (owner, name, expected) => {
                const descriptor = Object.getOwnPropertyDescriptor(owner, name);
                return [
                  name,
                  descriptor && descriptor.value,
                  descriptor && descriptor.value === expected,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.writable,
                  descriptor && descriptor.configurable
                ].join(":");
              };
              const node = document.createTextNode("x");
              return JSON.stringify({
                constructor: constants.map(([name, value]) => descriptorShape(Node, name, value)),
                prototype: constants.map(([name, value]) => descriptorShape(Node.prototype, name, value)),
                instanceOwn: constants
                  .map(([name]) => name)
                  .filter(name => Object.prototype.hasOwnProperty.call(node, name)),
                keysContainConstants: Object.keys(Node).some(name => name.endsWith("_NODE")) ||
                  Object.keys(Node.prototype).some(name => name.endsWith("_NODE"))
              });
            })()
            "#,
        )
        .expect("Node constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["ELEMENT_NODE:1:true:true:false:false","ATTRIBUTE_NODE:2:true:true:false:false","TEXT_NODE:3:true:true:false:false","CDATA_SECTION_NODE:4:true:true:false:false","ENTITY_REFERENCE_NODE:5:true:true:false:false","ENTITY_NODE:6:true:true:false:false","PROCESSING_INSTRUCTION_NODE:7:true:true:false:false","COMMENT_NODE:8:true:true:false:false","DOCUMENT_NODE:9:true:true:false:false","DOCUMENT_TYPE_NODE:10:true:true:false:false","DOCUMENT_FRAGMENT_NODE:11:true:true:false:false","NOTATION_NODE:12:true:true:false:false","DOCUMENT_POSITION_DISCONNECTED:1:true:true:false:false","DOCUMENT_POSITION_PRECEDING:2:true:true:false:false","DOCUMENT_POSITION_FOLLOWING:4:true:true:false:false","DOCUMENT_POSITION_CONTAINS:8:true:true:false:false","DOCUMENT_POSITION_CONTAINED_BY:16:true:true:false:false","DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC:32:true:true:false:false"],"prototype":["ELEMENT_NODE:1:true:true:false:false","ATTRIBUTE_NODE:2:true:true:false:false","TEXT_NODE:3:true:true:false:false","CDATA_SECTION_NODE:4:true:true:false:false","ENTITY_REFERENCE_NODE:5:true:true:false:false","ENTITY_NODE:6:true:true:false:false","PROCESSING_INSTRUCTION_NODE:7:true:true:false:false","COMMENT_NODE:8:true:true:false:false","DOCUMENT_NODE:9:true:true:false:false","DOCUMENT_TYPE_NODE:10:true:true:false:false","DOCUMENT_FRAGMENT_NODE:11:true:true:false:false","NOTATION_NODE:12:true:true:false:false","DOCUMENT_POSITION_DISCONNECTED:1:true:true:false:false","DOCUMENT_POSITION_PRECEDING:2:true:true:false:false","DOCUMENT_POSITION_FOLLOWING:4:true:true:false:false","DOCUMENT_POSITION_CONTAINS:8:true:true:false:false","DOCUMENT_POSITION_CONTAINED_BY:16:true:true:false:false","DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC:32:true:true:false:false"],"instanceOwn":[],"keysContainConstants":true}"#
    );
}
#[test]
fn node_prototype_relationship_accessors_are_visible_to_reflection() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = document.createElement("div");
              const child = document.createElement("span");
              parent.appendChild(child);

              const firstChild = Object.getOwnPropertyDescriptor(Node.prototype, "firstChild");
              const parentNode = Object.getOwnPropertyDescriptor(Node.prototype, "parentNode");
              const childNodes = Object.getOwnPropertyDescriptor(Node.prototype, "childNodes");
              const textContent = Object.getOwnPropertyDescriptor(Node.prototype, "textContent");

              return JSON.stringify({
                firstChildGetterType: typeof firstChild?.get,
                firstChildEnumerable: !!firstChild?.enumerable,
                firstChildConfigurable: !!firstChild?.configurable,
                firstChildCallWorks: typeof firstChild?.get === "function" ? firstChild.get.call(parent) === child : false,
                parentNodeGetterType: typeof parentNode?.get,
                parentNodeCallWorks: typeof parentNode?.get === "function" ? parentNode.get.call(child) === parent : false,
                childNodesGetterType: typeof childNodes?.get,
                childNodesCtor: typeof childNodes?.get === "function" ? (childNodes.get.call(parent).constructor?.name ?? null) : null,
                textContentGetterType: typeof textContent?.get,
                textContentSetterType: typeof textContent?.set
              });
            })()
            "#,
        )
        .expect("node prototype reflection probe should evaluate");

    assert_eq!(
        result,
        r#"{"firstChildGetterType":"function","firstChildEnumerable":true,"firstChildConfigurable":true,"firstChildCallWorks":true,"parentNodeGetterType":"function","parentNodeCallWorks":true,"childNodesGetterType":"function","childNodesCtor":"NodeList","textContentGetterType":"function","textContentSetterType":"function"}"#
    );
}
#[test]
fn node_prototype_mutation_methods_match_existing_child_errors() {
    let mut vm = new_storage_test_vm("https://node-prototype-mutation-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = document.createElement("div");
              const reference = document.createElement("b");
              const appended = document.createElement("span");
              const replacement = document.createElement("i");
              const finalChild = document.createElement("em");
              parent.appendChild(reference);

              const appendChild = Node.prototype.appendChild;
              const insertBefore = Node.prototype.insertBefore;
              const removeChild = Node.prototype.removeChild;
              const replaceChild = Node.prototype.replaceChild;

              function probe(fn) {
                try {
                  fn();
                  return "no-throw";
                } catch (e) {
                  return `${e.name}:${e.code || 0}:${e instanceof TypeError}`;
                }
              }

              const appendReturned = appendChild.call(parent, appended);
              const insertReturned = insertBefore.call(parent, replacement, reference);
              const removed = removeChild.call(parent, appended);
              const replaced = replaceChild.call(parent, finalChild, replacement);

              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const foreignChild = frame.contentDocument.createElement("section");
              const foreignOwner = foreignChild.ownerDocument;
              const foreignRemove = probe(() => removeChild.call(parent, foreignChild));
              const foreignInsertReference = probe(() => {
                insertBefore.call(parent, document.createElement("u"), foreignChild);
              });
              const foreignReplaceOldChild = probe(() => {
                replaceChild.call(parent, document.createElement("small"), foreignChild);
              });

              return JSON.stringify({
                appendType: typeof appendChild,
                appendLength: appendChild.length,
                insertLength: insertBefore.length,
                removeLength: removeChild.length,
                replaceLength: replaceChild.length,
                appendReturned: appendReturned === appended,
                insertReturned: insertReturned === replacement,
                removed: removed === appended,
                replaced: replaced === replacement,
                order: Array.from(parent.childNodes).map(node => node.localName).join(","),
                appendNull: probe(() => appendChild.call(parent, null)),
                insertMissingReference: probe(() => insertBefore.call(parent, document.createElement("q"))),
                foreignRemove,
                foreignInsertReference,
                foreignReplaceOldChild,
                foreignOwnerUnchanged: foreignChild.ownerDocument === foreignOwner
              });
            })()
            "#,
        )
        .expect("Node.prototype mutation methods should match WebIDL errors");

    assert_eq!(
        result,
        r#"{"appendType":"function","appendLength":1,"insertLength":2,"removeLength":1,"replaceLength":2,"appendReturned":true,"insertReturned":true,"removed":true,"replaced":true,"order":"em,b","appendNull":"TypeError:0:true","insertMissingReference":"TypeError:0:true","foreignRemove":"NotFoundError:8:false","foreignInsertReference":"NotFoundError:8:false","foreignReplaceOldChild":"NotFoundError:8:false","foreignOwnerUnchanged":true}"#
    );
}
#[test]
fn location_prototype_is_immutable_while_same_prototype_assignments_succeed() {
    let mut vm = new_storage_test_vm("https://example.com/path");

    let result = vm
        .eval(
            r#"
            (() => {
              "use strict";
              const original = Object.getPrototypeOf(location);
              const replacement = {};
              const throwsName = callback => {
                try {
                  callback();
                  return "returned";
                } catch (error) {
                  return error && error.name;
                }
              };

              const objectDifferent = throwsName(() => {
                Object.setPrototypeOf(location, replacement);
              });
              const dunderDifferent = throwsName(() => {
                location.__proto__ = replacement;
              });
              const reflectDifferent = Reflect.setPrototypeOf(location, replacement);
              const unchanged = Object.getPrototypeOf(location) === original;
              const objectSame = Object.setPrototypeOf(location, original) === location;
              const dunderSame = throwsName(() => {
                location.__proto__ = original;
              });
              const reflectSame = Reflect.setPrototypeOf(location, original);

              return JSON.stringify({
                objectDifferent,
                dunderDifferent,
                reflectDifferent,
                unchanged,
                objectSame,
                dunderSame,
                reflectSame,
                instanceofLocation: location instanceof Location,
              });
            })()
            "#,
        )
        .expect("Location immutable prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"objectDifferent":"TypeError","dunderDifferent":"TypeError","reflectDifferent":false,"unchanged":true,"objectSame":true,"dunderSame":"returned","reflectSame":true,"instanceofLocation":true}"#
    );
}
#[test]
fn details_dialog_string_boundaries_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

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

              const details = document.createElement("details");
              details.name = null;
              const detailsNull = details.name;
              details.name = undefined;
              const detailsUndefined = details.name;
              details.name = { toString() { return "group"; } };
              const detailsObject = details.name;
              const detailsSymbol = probe(() => { details.name = Symbol("name"); });
              const detailsAfterSymbol = details.name;
              const detailsThrow = probe(() => {
                details.name = { toString() { throw new RangeError("name"); } };
              });
              const detailsAfterThrow = details.name;

              const dialog = document.createElement("dialog");
              dialog.returnValue = null;
              const returnNull = dialog.returnValue;
              dialog.returnValue = undefined;
              const returnUndefined = dialog.returnValue;
              dialog.returnValue = { toString() { return "return object"; } };
              const returnObject = dialog.returnValue;
              const returnSymbol = probe(() => { dialog.returnValue = Symbol("return"); });
              const returnAfterSymbol = dialog.returnValue;

              dialog.returnValue = "seed";
              dialog.open = true;
              dialog.close();
              const closeNoArg = `${dialog.returnValue}:${dialog.open}`;
              dialog.open = true;
              dialog.close(undefined);
              const closeUndefined = `${dialog.returnValue}:${dialog.open}`;
              dialog.open = true;
              dialog.close({ toString() { return "closed object"; } });
              const closeObject = `${dialog.returnValue}:${dialog.open}`;
              dialog.open = true;
              const closeSymbol = probe(() => dialog.close(Symbol("close")));
              const closeAfterSymbol = `${dialog.returnValue}:${dialog.open}`;
              const closeThrow = probe(() => {
                dialog.close({ toString() { throw new RangeError("close"); } });
              });
              const closeAfterThrow = `${dialog.returnValue}:${dialog.open}`;

              return JSON.stringify({
                detailsNull,
                detailsUndefined,
                detailsObject,
                detailsSymbol,
                detailsAfterSymbol,
                detailsThrow,
                detailsAfterThrow,
                returnNull,
                returnUndefined,
                returnObject,
                returnSymbol,
                returnAfterSymbol,
                closeNoArg,
                closeUndefined,
                closeObject,
                closeSymbol,
                closeAfterSymbol,
                closeThrow,
                closeAfterThrow
              });
            })()
            "#,
        )
        .expect("details/dialog string boundary probe should evaluate");

    assert_eq!(
        result,
        r#"{"detailsNull":"null","detailsUndefined":"undefined","detailsObject":"group","detailsSymbol":"TypeError","detailsAfterSymbol":"group","detailsThrow":"RangeError","detailsAfterThrow":"group","returnNull":"null","returnUndefined":"undefined","returnObject":"return object","returnSymbol":"TypeError","returnAfterSymbol":"return object","closeNoArg":"seed:false","closeUndefined":"seed:false","closeObject":"closed object:false","closeSymbol":"TypeError","closeAfterSymbol":"closed object:true","closeThrow":"RangeError","closeAfterThrow":"closed object:true"}"#
    );
}
#[test]
fn notification_declared_surface_uses_private_slots_and_methods() {
    let mut vm = new_storage_test_vm("https://example.com/notification-declared");

    let result = vm
        .eval(
            r#"
            (() => {
              const notification = new Notification("real", {
                data: { answer: 42 }
              });
              const internalOwn = Object.getOwnPropertyNames(notification)
                .filter(name =>
                  name.startsWith("__lmNotification") ||
                  name === "__moliEventTargetSlot" ||
                  name === "__moliSimpleEventTargetOrderedHandlers"
                )
                .join(",");

              Object.assign(Notification.prototype, {
                __lmNotificationTitle: "prototype-poison",
                __lmNotificationData: { answer: 0 },
                __moliEventTargetSlot: "prototypeListeners",
                __moliSimpleEventTargetOrderedHandlers: false
              });
              Object.assign(notification, {
                __lmNotificationTitle: "own-poison",
                __lmNotificationData: { answer: 1 },
                __moliEventTargetSlot: "ownListeners",
                __moliSimpleEventTargetOrderedHandlers: false
              });

              const summarize = (target, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(target, name);
                return [
                  !!descriptor,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  descriptor && typeof descriptor.value,
                  descriptor && descriptor.value.length,
                  descriptor && descriptor.value.name
                ].join(":");
              };
              const accessorDescriptor = name => {
                const descriptor = Object.getOwnPropertyDescriptor(Notification.prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };

              const titleGetter =
                Object.getOwnPropertyDescriptor(Notification.prototype, "title").get;
              const dataGetter =
                Object.getOwnPropertyDescriptor(Notification.prototype, "data").get;
              const getter = name =>
                Object.getOwnPropertyDescriptor(Notification.prototype, name).get;
              const outcome = callback => {
                try {
                  const value = callback();
                  return value === undefined ? "undefined" : String(value);
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              };
              const fake = {
                __lmNotificationBrand: true,
                __lmNotificationTitle: "fake-title",
                __lmNotificationData: { answer: 99 },
                __lmNotificationBody: "fake-body",
                __lmNotificationVibrate: [3, 4],
                __lmNotificationTimestamp: 99,
                __lmNotificationRenotify: true,
                __lmNotificationSilent: false,
                __lmNotificationRequireInteraction: true,
                __lmNotificationActions: [{ action: "fake", title: "Fake" }],
                __lmNotificationRecordId: 1n,
                __lmNotificationRecordRegistrationId: 1n
              };
              const events = [];
              notification.addEventListener("show", () => events.push("listener"));
              const allowed = notification.dispatchEvent(new Event("show"));

              return [
                internalOwn || "none",
                [accessorDescriptor("title"), accessorDescriptor("data"), accessorDescriptor("actions")].join(","),
                notification.title,
                notification.data.answer,
                notification.actions.length,
                [
                  outcome(() => titleGetter.call(fake)),
                  outcome(() => dataGetter.call(fake)),
                  outcome(() => getter("body").call(fake)),
                  outcome(() => getter("vibrate").call(fake)),
                  outcome(() => getter("timestamp").call(fake)),
                  outcome(() => getter("renotify").call(fake)),
                  outcome(() => getter("silent").call(fake)),
                  outcome(() => getter("requireInteraction").call(fake)),
                  outcome(() => getter("actions").call(fake)),
                  outcome(() => notification.close.call(fake))
                ].join(","),
                [
                  outcome(() => notification.addEventListener.call(fake, "show", () => {})),
                  outcome(() => notification.removeEventListener.call(fake, "show", () => {})),
                  outcome(() => notification.dispatchEvent.call(fake, new Event("show")))
                ].join(","),
                Object.keys(Notification).includes("requestPermission"),
                summarize(Notification, "requestPermission"),
                Object.keys(notification).includes("addEventListener"),
                summarize(notification, "addEventListener"),
                summarize(notification, "removeEventListener"),
                summarize(notification, "dispatchEvent"),
                allowed,
                events.join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("Notification declared surface probe should evaluate");

    assert_eq!(
        result,
        "none|title:function:get title:0:undefined:true:true,data:function:get data:0:undefined:true:true,actions:function:get actions:0:undefined:true:true|real|42|0|throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError,throw:TypeError|throw:TypeError,throw:TypeError,throw:TypeError|true|true:true:true:true:function:0:requestPermission|true|true:true:true:true:function:0:addEventListener|true:true:true:true:function:0:removeEventListener|true:true:true:true:function:0:dispatchEvent|true|listener"
    );
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
    descriptorShape(DOMMatrix.prototype, "setMatrixValue"),
    descriptorShape(DOMMatrixReadOnly, "fromMatrix"),
    descriptorShape(DOMMatrixReadOnly, "fromFloat32Array"),
    descriptorShape(DOMMatrix, "fromMatrix"),
    descriptorShape(DOMMatrix, "fromFloat64Array")
  ].join("|");
})()
"#,
        )
        .expect("DOMMatrix Window operation descriptors should evaluate");

    assert_eq!(
        result,
        "function,0,true,true,true|function,1,true,true,true|function,0,true,true,true|function,1,true,true,true|function,0,true,true,true|function,1,true,true,true"
    );
}
