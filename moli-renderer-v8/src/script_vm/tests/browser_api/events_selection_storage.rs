use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

#[test]
fn captured_mouse_event_constructor_validates_coordinates_and_inherits_event_init() {
    let mut vm = new_storage_test_vm("https://captured-mouse-event.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error.name;
    }
  };
  const defaults = new CapturedMouseEvent("default");
  const initialized = new CapturedMouseEvent("capturedmousechange", {
    bubbles: true,
    cancelable: true,
    composed: true,
    surfaceX: 12,
    surfaceY: 7
  });
  return JSON.stringify({
    constructorType: typeof CapturedMouseEvent,
    constructorLength: CapturedMouseEvent.length,
    prototypeParent: Object.getPrototypeOf(CapturedMouseEvent.prototype) === Event.prototype,
    defaults: [defaults.surfaceX, defaults.surfaceY, defaults.bubbles, defaults.cancelable, defaults.composed],
    initialized: [
      initialized.type,
      initialized.surfaceX,
      initialized.surfaceY,
      initialized.bubbles,
      initialized.cancelable,
      initialized.composed,
      initialized instanceof Event,
      initialized instanceof CapturedMouseEvent
    ],
    errors: [
      errorName(() => new CapturedMouseEvent()),
      errorName(() => new CapturedMouseEvent("x", { surfaceX: -2 })),
      errorName(() => new CapturedMouseEvent("x", { surfaceX: -1, surfaceY: 2 })),
      errorName(() => new CapturedMouseEvent("x", { surfaceY: 2147483648 })),
      errorName(() => CapturedMouseEvent("x"))
    ]
  });
})()
"#,
        )
        .expect("CapturedMouseEvent constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorType":"function","constructorLength":1,"prototypeParent":true,"defaults":[-1,-1,false,false,false],"initialized":["capturedmousechange",12,7,true,true,true,true,true],"errors":["TypeError","RangeError","RangeError","RangeError","TypeError"]}"#
    );
}

#[test]
fn event_and_mouse_event_accessors_use_prototype_receivers() {
    let mut vm = new_storage_test_vm("https://event-prototype-accessors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const mouse = new MouseEvent("mouse", { clientX: 12.5, clientY: 3.25 });
              const pointer = new PointerEvent("pointer", { clientX: 4.5, clientY: 6.5 });
              const legacy = document.createEvent("MouseEvent");
              legacy.initMouseEvent(
                "legacy", false, false, window, 0,
                0, 0, 7, 9, false, false, false, false, 0, null
              );

              const offset =
                Object.getOwnPropertyDescriptor(MouseEvent.prototype, "offsetX");
              const cancelBubble =
                Object.getOwnPropertyDescriptor(Event.prototype, "cancelBubble");
              const returnValue =
                Object.getOwnPropertyDescriptor(Event.prototype, "returnValue");
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error.name;
                }
              };

              return JSON.stringify({
                own: {
                  mouseOffset: Object.hasOwn(mouse, "offsetX"),
                  pointerOffset: Object.hasOwn(pointer, "offsetX"),
                  legacyOffset: Object.hasOwn(legacy, "offsetX"),
                  cancelBubble: Object.hasOwn(mouse, "cancelBubble"),
                  returnValue: Object.hasOwn(mouse, "returnValue"),
                  isTrusted: Object.hasOwn(mouse, "isTrusted")
                },
                prototype: {
                  mouseOffset: Object.hasOwn(MouseEvent.prototype, "offsetX"),
                  eventCancelBubble:
                    Object.hasOwn(Event.prototype, "cancelBubble"),
                  eventReturnValue:
                    Object.hasOwn(Event.prototype, "returnValue"),
                  eventIsTrusted:
                    Object.hasOwn(Event.prototype, "isTrusted")
                },
                values: [
                  mouse.offsetX,
                  mouse.offsetY,
                  pointer.offsetX,
                  pointer.offsetY,
                  legacy.offsetX,
                  legacy.offsetY,
                  offset.get.call(mouse),
                  cancelBubble.get.call(mouse),
                  returnValue.get.call(mouse)
                ],
                metadata: [
                  offset.get.name,
                  offset.get.length,
                  offset.set,
                  cancelBubble.get.name,
                  cancelBubble.get.length,
                  cancelBubble.set.name,
                  cancelBubble.set.length,
                  offset.enumerable,
                  offset.configurable
                ],
                errors: [
                  errorName(() => offset.get.call({})),
                  errorName(() => cancelBubble.get.call({})),
                  errorName(() => returnValue.set.call({}, false))
                ]
              });
            })()
            "#,
        )
        .expect("Event accessor receiver probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":{"mouseOffset":false,"pointerOffset":false,"legacyOffset":false,"cancelBubble":false,"returnValue":false,"isTrusted":true},"prototype":{"mouseOffset":true,"eventCancelBubble":true,"eventReturnValue":true,"eventIsTrusted":false},"values":[12.5,3.25,4.5,6.5,7,9,12.5,false,true],"metadata":["get offsetX",0,null,"get cancelBubble",0,"set cancelBubble",1,true,true],"errors":["TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn event_core_attribute_getters_match_chromium_and_support_framework_capture() {
    let mut vm = new_storage_test_vm("https://event-core-attribute-accessors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const names = [
                "type",
                "target",
                "currentTarget",
                "eventPhase",
                "bubbles",
                "cancelable",
                "defaultPrevented",
                "composed",
                "srcElement"
              ];
              const descriptors = names.map(name => {
                const descriptor = Object.getOwnPropertyDescriptor(Event.prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ];
              });
              const getters = Object.fromEntries(names.map(name => [
                name,
                Object.getOwnPropertyDescriptor(Event.prototype, name).get
              ]));
              const event = new Event("probe", {
                bubbles: true,
                cancelable: true,
                composed: true
              });
              let duringDispatch;
              const target = document.createElement("button");
              target.addEventListener("probe", dispatched => {
                duringDispatch = [
                  getters.target.call(dispatched) === target,
                  getters.currentTarget.call(dispatched) === target,
                  getters.eventPhase.call(dispatched)
                ];
              });
              target.dispatchEvent(event);
              let illegalInvocation;
              try {
                getters.target.call({ target: "spoofed" });
                illegalInvocation = "none";
              } catch (error) {
                illegalInvocation = error.name;
              }
              return JSON.stringify({
                descriptors,
                initial: [
                  getters.type.call(new Event("plain")),
                  getters.target.call(new Event("plain")),
                  getters.currentTarget.call(new Event("plain")),
                  getters.eventPhase.call(new Event("plain")),
                  getters.bubbles.call(event),
                  getters.cancelable.call(event),
                  getters.defaultPrevented.call(event),
                  getters.composed.call(event),
                  getters.srcElement.call(new Event("plain"))
                ],
                duringDispatch,
                afterDispatch: [
                  getters.target.call(event) === target,
                  getters.currentTarget.call(event),
                  getters.eventPhase.call(event)
                ],
                illegalInvocation
              });
            })()
            "#,
        )
        .expect("Event core attribute descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":[["type","function","get type",0,null,true,true],["target","function","get target",0,null,true,true],["currentTarget","function","get currentTarget",0,null,true,true],["eventPhase","function","get eventPhase",0,null,true,true],["bubbles","function","get bubbles",0,null,true,true],["cancelable","function","get cancelable",0,null,true,true],["defaultPrevented","function","get defaultPrevented",0,null,true,true],["composed","function","get composed",0,null,true,true],["srcElement","function","get srcElement",0,null,true,true]],"initial":["plain",null,null,0,true,true,false,true,null],"duringDispatch":[true,true,2],"afterDispatch":[true,null,0],"illegalInvocation":"TypeError"}"#
    );
}

#[test]
fn lwc_related_target_getter_capture_matches_chromium() {
    let mut vm = new_storage_test_vm("https://lwc-related-target-accessors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const shape = constructor => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  constructor.prototype,
                  "relatedTarget"
                );
                return [
                  constructor.name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ];
              };
              const focusGetter = Object.getOwnPropertyDescriptor(
                FocusEvent.prototype,
                "relatedTarget"
              ).get;
              const mouseGetter = Object.getOwnPropertyDescriptor(
                MouseEvent.prototype,
                "relatedTarget"
              ).get;
              const related = document.createElement("div");
              const focus = new FocusEvent("focus", { relatedTarget: related });
              const mouse = new MouseEvent("mouseover", { relatedTarget: related });
              const pointer = new PointerEvent("pointerover", { relatedTarget: related });
              const errorName = callback => {
                try {
                  callback();
                  return "none";
                } catch (error) {
                  return error.name;
                }
              };
              return JSON.stringify({
                descriptors: [shape(FocusEvent), shape(MouseEvent)],
                values: [
                  focusGetter.call(focus) === related,
                  mouseGetter.call(mouse) === related,
                  mouseGetter.call(pointer) === related
                ],
                errors: [
                  errorName(() => focusGetter.call(mouse)),
                  errorName(() => mouseGetter.call(focus)),
                  errorName(() => mouseGetter.call({ relatedTarget: related }))
                ]
              });
            })()
            "#,
        )
        .expect("LWC relatedTarget descriptor capture should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":[["FocusEvent","function","get relatedTarget",0,null,true,true],["MouseEvent","function","get relatedTarget",0,null,true,true]],"values":[true,true,true],"errors":["TypeError","TypeError","TypeError"]}"#
    );
}

#[test]
fn dispatch_event_rejects_uninitialized_and_non_event_objects() {
    let mut vm = new_storage_test_vm("https://event-dispatch-state.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = callback => {
                try {
                  return String(callback());
                } catch (error) {
                  return `${error && error.name}:${error && error.code}:${error instanceof DOMException}`;
                }
              };

              const created = document.createEvent("Event");
              const uninitialized = probe(() => document.dispatchEvent(created));
              created.initEvent("created", false, false);
              const initialized = probe(() => document.dispatchEvent(created));
              const constructedEmptyType = probe(() => document.dispatchEvent(new Event("")));
              const plainObject = probe(() => document.dispatchEvent({ type: "plain" }));

              return JSON.stringify({
                uninitialized,
                initialized,
                constructedEmptyType,
                plainObject
              });
            })()
            "#,
        )
        .expect("dispatchEvent initialized-state probe should evaluate");

    assert_eq!(
        result,
        r#"{"uninitialized":"InvalidStateError:11:true","initialized":"true","constructedEmptyType":"true","plainObject":"TypeError:undefined:false"}"#
    );
}
#[test]
fn create_event_legacy_aliases_create_uninitialized_events() {
    let mut vm = new_storage_test_vm("https://event-create-legacy-aliases.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const aliases = [
                "BeforeUnloadEvent",
                "CompositionEvent",
                "CustomEvent",
                "DeviceMotionEvent",
                "DeviceOrientationEvent",
                "DragEvent",
                "Event",
                "Events",
                "FocusEvent",
                "HashChangeEvent",
                "HTMLEvents",
                "KeyboardEvent",
                "MessageEvent",
                "MouseEvent",
                "MouseEvents",
                "StorageEvent",
                "SVGEvents",
                "TextEvent",
                "UIEvent",
                "UIEvents"
              ];
              const failures = [];
              for (const alias of aliases) {
                try {
                  const event = document.createEvent(alias);
                  if (event.type !== "") {
                    failures.push(`${alias}:type:${event.type}`);
                    continue;
                  }
                  try {
                    document.dispatchEvent(event);
                    failures.push(`${alias}:dispatch:no-throw`);
                  } catch (error) {
                    if (!(error instanceof DOMException) || error.name !== "InvalidStateError") {
                      failures.push(`${alias}:dispatch:${error && error.name}`);
                    }
                  }
                } catch (error) {
                  failures.push(`${alias}:create:${error && error.name}`);
                }
              }
              return failures.join("|") || "ok";
            })()
            "#,
        )
        .expect("legacy createEvent alias probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn legacy_text_event_is_illegal_to_construct_and_uses_idl_initializer_defaults() {
    let mut vm = new_storage_test_vm("https://legacy-text-event.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const errorName = callback => {
    try {
      callback();
      return "none";
    } catch (error) {
      return error && error.name;
    }
  };
  const event = document.createEvent("TextEvent");
  const noArguments = errorName(() => event.initTextEvent());
  event.initTextEvent("foo");
  return JSON.stringify({
    constructorLength: TextEvent.length,
    initializerLength: TextEvent.prototype.initTextEvent.length,
    constructorError: errorName(() => new TextEvent("textInput")),
    prototype: Object.getPrototypeOf(event) === TextEvent.prototype,
    prototypeParent: Object.getPrototypeOf(TextEvent.prototype) === UIEvent.prototype,
    noArguments,
    type: event.type,
    bubbles: event.bubbles,
    cancelable: event.cancelable,
    view: event.view === null ? null : "non-null",
    data: event.data
  });
})()
"#,
        )
        .expect("legacy TextEvent surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorLength":0,"initializerLength":1,"constructorError":"TypeError","prototype":true,"prototypeParent":true,"noArguments":"TypeError","type":"foo","bubbles":false,"cancelable":false,"view":null,"data":"undefined"}"#
    );
}

#[test]
fn event_range_wheel_constructor_constants_are_declared() {
    let mut vm = new_storage_test_vm("https://event-range-wheel-constants.test/");

    let result = vm
        .eval(
            r#"
            (() => {
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
              const eventConstants = [
                ["NONE", 0],
                ["CAPTURING_PHASE", 1],
                ["AT_TARGET", 2],
                ["BUBBLING_PHASE", 3]
              ];
              const rangeConstants = [
                ["START_TO_START", 0],
                ["START_TO_END", 1],
                ["END_TO_END", 2],
                ["END_TO_START", 3]
              ];
              const wheelConstants = [
                ["DOM_DELTA_PIXEL", 0],
                ["DOM_DELTA_LINE", 1],
                ["DOM_DELTA_PAGE", 2]
              ];
              return JSON.stringify({
                event: eventConstants.map(([name, value]) =>
                  descriptorShape(Event, name, value)
                ),
                eventPrototype: eventConstants.map(([name, value]) =>
                  descriptorShape(Event.prototype, name, value)
                ),
                range: rangeConstants.map(([name, value]) =>
                  descriptorShape(Range, name, value)
                ),
                rangePrototype: rangeConstants.map(([name, value]) =>
                  descriptorShape(Range.prototype, name, value)
                ),
                wheel: wheelConstants.map(([name, value]) =>
                  descriptorShape(WheelEvent, name, value)
                ),
                wheelPrototype: wheelConstants.map(([name, value]) =>
                  descriptorShape(WheelEvent.prototype, name, value)
                ),
                keysContainConstants:
                  Object.keys(Event).some(name =>
                    eventConstants.some(([constant]) => constant === name)
                  ) ||
                  Object.keys(Event.prototype).some(name =>
                    eventConstants.some(([constant]) => constant === name)
                  ) ||
                  Object.keys(Range).some(name =>
                    rangeConstants.some(([constant]) => constant === name)
                  ) ||
                  Object.keys(Range.prototype).some(name =>
                    rangeConstants.some(([constant]) => constant === name)
                  ) ||
                  Object.keys(WheelEvent).some(name =>
                    wheelConstants.some(([constant]) => constant === name)
                  ) ||
                  Object.keys(WheelEvent.prototype).some(name =>
                    wheelConstants.some(([constant]) => constant === name)
                  ),
                eventPhaseMatchesNone: new Event("phase").eventPhase === Event.NONE,
                deltaModeMatchesPixel: new WheelEvent("wheel").deltaMode ===
                  WheelEvent.DOM_DELTA_PIXEL
              });
            })()
            "#,
        )
        .expect("event/range/wheel constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"event":["NONE:0:true:true:false:false","CAPTURING_PHASE:1:true:true:false:false","AT_TARGET:2:true:true:false:false","BUBBLING_PHASE:3:true:true:false:false"],"eventPrototype":["NONE:0:true:true:false:false","CAPTURING_PHASE:1:true:true:false:false","AT_TARGET:2:true:true:false:false","BUBBLING_PHASE:3:true:true:false:false"],"range":["START_TO_START:0:true:true:false:false","START_TO_END:1:true:true:false:false","END_TO_END:2:true:true:false:false","END_TO_START:3:true:true:false:false"],"rangePrototype":["START_TO_START:0:true:true:false:false","START_TO_END:1:true:true:false:false","END_TO_END:2:true:true:false:false","END_TO_START:3:true:true:false:false"],"wheel":["DOM_DELTA_PIXEL:0:true:true:false:false","DOM_DELTA_LINE:1:true:true:false:false","DOM_DELTA_PAGE:2:true:true:false:false"],"wheelPrototype":["DOM_DELTA_PIXEL:0:true:true:false:false","DOM_DELTA_LINE:1:true:true:false:false","DOM_DELTA_PAGE:2:true:true:false:false"],"keysContainConstants":true,"eventPhaseMatchesNone":true,"deltaModeMatchesPixel":true}"#
    );
}

#[test]
fn element_prototype_accessors_are_hookable() {
    let mut vm = new_storage_test_vm("https://element-prototype-accessors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const clickDescriptor = Object.getOwnPropertyDescriptor(
                HTMLElement.prototype,
                "onclick"
              );
              const submitDescriptor = Object.getOwnPropertyDescriptor(
                HTMLElement.prototype,
                "onsubmit"
              );
              const iframeSrcDescriptor = Object.getOwnPropertyDescriptor(
                HTMLIFrameElement.prototype,
                "src"
              );

              const originalClickGet = clickDescriptor.get;
              const originalClickSet = clickDescriptor.set;
              Object.defineProperty(HTMLElement.prototype, "onclick", {
                get() {
                  return originalClickGet.apply(this, arguments);
                },
                set() {
                  return originalClickSet.apply(this, arguments);
                },
                configurable: true,
                enumerable: true
              });

              const originalIframeSrcGet = iframeSrcDescriptor.get;
              const originalIframeSrcSet = iframeSrcDescriptor.set;
              Object.defineProperty(HTMLIFrameElement.prototype, "src", {
                get() {
                  return originalIframeSrcGet.apply(this, arguments);
                },
                set() {
                  return originalIframeSrcSet.apply(this, arguments);
                },
                configurable: true,
                enumerable: true
              });

              const div = document.createElement("div");
              const other = document.createElement("div");
              const form = document.createElement("form");
              const iframe = document.createElement("iframe");
              const copiedIframe = document.createElement("iframe");
              function handler() {}

              div.onclick = handler;
              other.onclick = "not a function";
              form.onsubmit = handler;
              originalIframeSrcSet.apply(iframe, ["/child.html"]);
              Object.defineProperty(copiedIframe, "src", iframeSrcDescriptor);
              copiedIframe.src = "/copied.html";

              return JSON.stringify({
                clickGet: typeof clickDescriptor.get,
                clickSet: typeof clickDescriptor.set,
                clickConfigurable: clickDescriptor.configurable,
                submitGet: typeof submitDescriptor.get,
                submitSet: typeof submitDescriptor.set,
                iframeSrcGet: typeof iframeSrcDescriptor.get,
                iframeSrcSet: typeof iframeSrcDescriptor.set,
                iframeSrcConfigurable: iframeSrcDescriptor.configurable,
                divHandler: div.onclick === handler,
                otherNull: other.onclick === null,
                formHandler: form.onsubmit === handler,
                prototypeNull: HTMLElement.prototype.onsubmit === null,
                iframePrototypeSrc: HTMLIFrameElement.prototype.src,
                iframeSrcValue: originalIframeSrcGet.apply(iframe, []),
                iframeSrcAttribute: iframe.getAttribute("src"),
                copiedIframeSrcValue: copiedIframe.src,
                copiedIframeSrcAttribute: copiedIframe.getAttribute("src")
              });
            })()
            "#,
        )
        .expect("element prototype accessor probe should evaluate");

    assert_eq!(
        result,
        r#"{"clickGet":"function","clickSet":"function","clickConfigurable":true,"submitGet":"function","submitSet":"function","iframeSrcGet":"function","iframeSrcSet":"function","iframeSrcConfigurable":true,"divHandler":true,"otherNull":true,"formHandler":true,"prototypeNull":true,"iframePrototypeSrc":"","iframeSrcValue":"https://element-prototype-accessors.test/child.html","iframeSrcAttribute":"/child.html","copiedIframeSrcValue":"https://element-prototype-accessors.test/copied.html","copiedIframeSrcAttribute":"/copied.html"}"#
    );
}

#[test]
fn storage_event_constructor_matches_wpt_cross_surface() {
    let mut vm = new_storage_test_vm("https://storage-event-constructor.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const throwsName = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              const summarize = event => ({
                type: event.type,
                bubbles: event.bubbles,
                cancelable: event.cancelable,
                key: event.key,
                oldValue: event.oldValue,
                newValue: event.newValue,
                url: event.url,
                storageAreaIsNull: event.storageArea === null,
                storageAreaIsLocal: event.storageArea === localStorage,
                instance: event instanceof StorageEvent,
                eventInstance: event instanceof Event,
                tag: Object.prototype.toString.call(event)
              });
              return JSON.stringify({
                callWithoutNew: throwsName(() => StorageEvent("")),
                missingType: throwsName(() => new StorageEvent()),
                length: StorageEvent.length,
                initLength: StorageEvent.prototype.initStorageEvent.length,
                defaults: summarize(new StorageEvent("type")),
                full: summarize(new StorageEvent("storage", {
                  bubbles: true,
                  cancelable: true,
                  key: "key",
                  oldValue: "oldValue",
                  newValue: "newValue",
                  url: "url",
                  storageArea: localStorage
                })),
                nulls: summarize(new StorageEvent(null, {
                  key: null,
                  oldValue: null,
                  newValue: null,
                  url: null,
                  storageArea: null
                })),
                undefineds: summarize(new StorageEvent(undefined, {
                  key: undefined,
                  oldValue: undefined,
                  newValue: undefined,
                  url: undefined,
                  storageArea: undefined
                }))
              });
            })()
            "#,
        )
        .expect("StorageEvent constructor surface should evaluate");

    assert_eq!(
        result,
        r#"{"callWithoutNew":"TypeError","missingType":"TypeError","length":1,"initLength":1,"defaults":{"type":"type","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"","storageAreaIsNull":true,"storageAreaIsLocal":false,"instance":true,"eventInstance":true,"tag":"[object StorageEvent]"},"full":{"type":"storage","bubbles":true,"cancelable":true,"key":"key","oldValue":"oldValue","newValue":"newValue","url":"url","storageAreaIsNull":false,"storageAreaIsLocal":true,"instance":true,"eventInstance":true,"tag":"[object StorageEvent]"},"nulls":{"type":"null","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"null","storageAreaIsNull":true,"storageAreaIsLocal":false,"instance":true,"eventInstance":true,"tag":"[object StorageEvent]"},"undefineds":{"type":"undefined","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"","storageAreaIsNull":true,"storageAreaIsLocal":false,"instance":true,"eventInstance":true,"tag":"[object StorageEvent]"}}"#
    );
}

#[test]
fn storage_event_initstorageevent_matches_wpt_cross_surface() {
    let mut vm = new_storage_test_vm("https://storage-event-init.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const throwsName = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              const summarize = event => ({
                type: event.type,
                bubbles: event.bubbles,
                cancelable: event.cancelable,
                key: event.key,
                oldValue: event.oldValue,
                newValue: event.newValue,
                url: event.url,
                storageAreaIsNull: event.storageArea === null,
                storageAreaIsSession: event.storageArea === sessionStorage,
                instance: event instanceof StorageEvent,
                tag: Object.prototype.toString.call(event)
              });
              const event = document.createEvent("StorageEvent");
              const initial = summarize(event);
              const missingArg = throwsName(() => event.initStorageEvent());
              event.initStorageEvent("type");
              const oneArg = summarize(event);
              event.initStorageEvent(
                "storage",
                true,
                true,
                "key",
                "oldValue",
                "newValue",
                "url",
                sessionStorage
              );
              const full = summarize(event);
              event.initStorageEvent(null, null, null, null, null, null, null, null);
              const nulls = summarize(event);
              event.initStorageEvent(
                undefined,
                undefined,
                undefined,
                undefined,
                undefined,
                undefined,
                undefined,
                undefined
              );
              const undefineds = summarize(event);
              const descriptor = Object.getOwnPropertyDescriptor(
                StorageEvent.prototype,
                "initStorageEvent"
              );
              return JSON.stringify({
                initName: descriptor.value.name,
                initLength: StorageEvent.prototype.initStorageEvent.length,
                initEnumerable: descriptor.enumerable,
                initWritable: descriptor.writable,
                initConfigurable: descriptor.configurable,
                missingArg,
                initial,
                oneArg,
                full,
                nulls,
                undefineds
              });
            })()
            "#,
        )
        .expect("StorageEvent initStorageEvent surface should evaluate");

    assert_eq!(
        result,
        r#"{"initName":"initStorageEvent","initLength":1,"initEnumerable":true,"initWritable":true,"initConfigurable":true,"missingArg":"TypeError","initial":{"type":"","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"","storageAreaIsNull":true,"storageAreaIsSession":false,"instance":true,"tag":"[object StorageEvent]"},"oneArg":{"type":"type","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"","storageAreaIsNull":true,"storageAreaIsSession":false,"instance":true,"tag":"[object StorageEvent]"},"full":{"type":"storage","bubbles":true,"cancelable":true,"key":"key","oldValue":"oldValue","newValue":"newValue","url":"url","storageAreaIsNull":false,"storageAreaIsSession":true,"instance":true,"tag":"[object StorageEvent]"},"nulls":{"type":"null","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"null","storageAreaIsNull":true,"storageAreaIsSession":false,"instance":true,"tag":"[object StorageEvent]"},"undefineds":{"type":"undefined","bubbles":false,"cancelable":false,"key":null,"oldValue":null,"newValue":null,"url":"","storageAreaIsNull":true,"storageAreaIsSession":false,"instance":true,"tag":"[object StorageEvent]"}}"#
    );
}

#[test]
fn web_storage_preserves_wpt_dom_string_utf16_units() {
    let mut vm = new_storage_test_vm("https://web-storage-domstring-units.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const units = value => Array.from({ length: value.length }, (_, index) => value.charCodeAt(index));
  const sameUnits = (value, expected) => JSON.stringify(units(value)) === JSON.stringify(expected);
  const cases = [
    [0xD800],
    [0xDBFF],
    [0xDC00],
    [0xDFFF],
    [0xD83C, 0xDF4D],
    [0xD83C, 0x0061],
    [0x0061, 0xDF4D],
    [0xDBFF, 0xDFFF]
  ];
  const failures = [];

  for (const storageName of ["localStorage", "sessionStorage"]) {
    const storage = window[storageName];
    for (const expected of cases) {
      const value = String.fromCharCode(...expected);

      storage.clear();
      storage[value] = "user1";
      if (!(value in storage)) failures.push(`${storageName}:named-in:${expected.join(",")}`);
      if (storage.getItem(value) !== "user1") failures.push(`${storageName}:named-getItem:${expected.join(",")}`);
      if (storage[value] !== "user1") failures.push(`${storageName}:named-get:${expected.join(",")}`);
      if (!sameUnits(storage.key(0), expected)) failures.push(`${storageName}:key:${expected.join(",")}`);

      storage.clear();
      storage.setItem("name", value);
      if (!sameUnits(storage.getItem("name"), expected)) failures.push(`${storageName}:value-getItem:${expected.join(",")}`);
      if (!sameUnits(storage.name, expected)) failures.push(`${storageName}:value-named:${expected.join(",")}`);

      storage.clear();
      storage.setItem(value, value);
      if (!sameUnits(storage.getItem(value), expected)) failures.push(`${storageName}:setItem-both:${expected.join(",")}`);
      delete storage[value];
      if (value in storage) failures.push(`${storageName}:delete:${expected.join(",")}`);
    }
  }

  return failures.join("|") || "ok";
})()
"#,
        )
        .expect("WebStorage DOMString UTF-16 unit probe should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn dispatch_event_rejects_reentrant_dispatch_of_same_event() {
    let mut vm = new_storage_test_vm("https://event-dispatch-reentrant.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const event = new Event("x", { bubbles: true });
              const caught = [];
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error && error.name}:${error && error.code}:${error instanceof DOMException}`;
                }
              };

              target.addEventListener("x", () => {
                caught.push(probe(() => target.dispatchEvent(event)));
                caught.push(probe(() => document.dispatchEvent(event)));
              });

              const outer = target.dispatchEvent(event);
              caught.push(`outer:${outer}`);
              return caught.join("|");
            })()
            "#,
        )
        .expect("reentrant dispatchEvent probe should evaluate");

    assert_eq!(
        result,
        "InvalidStateError:11:true|InvalidStateError:11:true|outer:true"
    );
}
#[test]
fn event_dispatch_internal_flags_are_not_script_writable() {
    let mut vm = new_storage_test_vm("https://event-private-flags.test/");

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
              const target = document.createElement("div");
              document.body.appendChild(target);

              const passiveEvent = new Event("passive", { cancelable: true });
              const exposedBefore = [
                "__lmDispatching" in passiveEvent,
                "__lmPassive" in passiveEvent,
                "__lmSp" in passiveEvent,
                "__lmSip" in passiveEvent
              ];
              const passiveCalls = [];
              target.addEventListener("passive", event => {
                event.__lmPassive = false;
                event.__lmSip = true;
                event.preventDefault();
                passiveCalls.push(`first:${event.defaultPrevented}`);
              }, { passive: true });
              target.addEventListener("passive", event => {
                passiveCalls.push(`second:${event.defaultPrevented}`);
              });
              const passiveReturned = target.dispatchEvent(passiveEvent);

              const bubbleEvent = new Event("bubble", { bubbles: true });
              bubbleEvent.__lmSp = true;
              const bubbleCalls = [];
              document.body.addEventListener("bubble", () => bubbleCalls.push("body"), { once: true });
              target.dispatchEvent(bubbleEvent);

              return JSON.stringify({
                exposedBefore,
                passiveReturned,
                passiveDefaultPrevented: passiveEvent.defaultPrevented,
                passiveCalls,
                bubbleCalls
              });
            })()
            "#,
        )
        .expect("event private flag tamper probe should evaluate");

    assert_eq!(
        result,
        r#"{"exposedBefore":[false,false,false,false],"passiveReturned":true,"passiveDefaultPrevented":false,"passiveCalls":["first:false","second:false"],"bubbleCalls":["body"]}"#
    );
}

#[test]
fn document_level_scroll_blocking_listeners_default_to_passive() {
    let mut vm = new_storage_test_vm("https://default-passive-events.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const div = body.appendChild(document.createElement('div'));
  const cases = [
    ['window-touch-omitted', window, 'touchstart', 'omitted'],
    ['document-touch-undefined', document, 'touchmove', 'undefined'],
    ['html-wheel-omitted', root, 'wheel', 'omitted'],
    ['body-wheel-undefined', body, 'mousewheel', 'undefined'],
    ['window-touch-boolean', window, 'touchstart', 'boolean'],
    ['div-wheel-omitted', div, 'wheel', 'omitted'],
    ['window-touchend-omitted', window, 'touchend', 'omitted'],
    ['document-wheel-false', document, 'wheel', 'false'],
    ['body-touch-true', body, 'touchstart', 'true'],
    ['window-wheel-null', window, 'wheel', 'null']
  ];
  const out = [];
  for (const [name, target, type, mode] of cases) {
    let prevented = null;
    const listener = event => {
      event.preventDefault();
      prevented = event.defaultPrevented;
    };
    if (mode === 'omitted') target.addEventListener(type, listener);
    if (mode === 'undefined') target.addEventListener(type, listener, { passive: undefined });
    if (mode === 'boolean') target.addEventListener(type, listener, false);
    if (mode === 'false') target.addEventListener(type, listener, { passive: false });
    if (mode === 'true') target.addEventListener(type, listener, { passive: true });
    if (mode === 'null') target.addEventListener(type, listener, { passive: null });
    const allowed = target.dispatchEvent(new Event(type, { cancelable: true }));
    out.push(`${name}:${prevented}:${allowed}`);
  }
  return out.join('|');
})()
"#,
        )
        .expect("default passive event listener probe should evaluate");

    assert_eq!(
        result,
        "window-touch-omitted:false:true|document-touch-undefined:false:true|html-wheel-omitted:false:true|body-wheel-undefined:false:true|window-touch-boolean:false:true|div-wheel-omitted:true:false|window-touchend-omitted:true:false|document-wheel-false:true:false|body-touch-true:false:true|window-wheel-null:true:false"
    );
}

#[test]
fn event_composed_path_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://event-composed-path-private-slot.test/");

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
              const target = document.createElement("div");
              document.body.appendChild(target);

              const clean = new Event("clean", { bubbles: true, composed: true });
              const cleanBeforeOwn = Object.getOwnPropertyNames(clean).includes("__lmCp");
              let cleanDuring = null;
              target.addEventListener("clean", event => {
                const path = event.composedPath();
                cleanDuring = {
                  ownNameVisible: Object.getOwnPropertyNames(event).includes("__lmCp"),
                  firstIsTarget: path[0] === target,
                  hasPath: path.length > 0
                };
              }, { once: true });
              target.dispatchEvent(clean);

              const spoofed = new Event("spoofed", { bubbles: true, composed: true });
              spoofed.__lmCp = ["spoofed-before"];
              const spoofedBefore = spoofed.composedPath();
              let spoofedDuring = null;
              target.addEventListener("spoofed", event => {
                event.__lmCp = ["spoofed-during"];
                const path = event.composedPath();
                spoofedDuring = {
                  ownValue: event.__lmCp[0],
                  firstIsTarget: path[0] === target,
                  containsSpoof: path.includes("spoofed-before") || path.includes("spoofed-during")
                };
              }, { once: true });
              target.dispatchEvent(spoofed);

              const simpleTarget = new EventTarget();
              const simple = new Event("simple");
              simple.__lmCp = ["spoofed-simple"];
              let simpleDuring = null;
              simpleTarget.addEventListener("simple", event => {
                const path = event.composedPath();
                simpleDuring = {
                  ownValue: event.__lmCp[0],
                  firstIsSimpleTarget: path[0] === simpleTarget,
                  length: path.length,
                  containsSpoof: path.includes("spoofed-simple")
                };
              }, { once: true });
              simpleTarget.dispatchEvent(simple);

              const fakePath = Event.prototype.composedPath.call({ __lmCp: ["fake"] });
              return JSON.stringify({
                cleanBeforeOwn,
                cleanDuring,
                cleanAfterLength: clean.composedPath().length,
                spoofedBeforeLength: spoofedBefore.length,
                spoofedDuring,
                spoofedAfterLength: spoofed.composedPath().length,
                simpleDuring,
                simpleAfterLength: simple.composedPath().length,
                fakePathLength: fakePath.length
              });
            })()
            "#,
        )
        .expect("event composed path private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"cleanBeforeOwn":false,"cleanDuring":{"ownNameVisible":false,"firstIsTarget":true,"hasPath":true},"cleanAfterLength":0,"spoofedBeforeLength":0,"spoofedDuring":{"ownValue":"spoofed-during","firstIsTarget":true,"containsSpoof":false},"spoofedAfterLength":0,"simpleDuring":{"ownValue":"spoofed-simple","firstIsSimpleTarget":true,"length":1,"containsSpoof":false},"simpleAfterLength":0,"fakePathLength":0}"#
    );
}

#[test]
fn simple_event_target_routing_slot_ignores_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://simple-event-target-routing-private-slot.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = new EventTarget();
              const ownBefore = Object.getOwnPropertyNames(target)
                .includes("__moliEventTargetSlot");
              const calls = [];
              target.addEventListener("route", () => calls.push("listener"));

              target.__moliEventTargetSlot = "__wrongSlot";
              const publicSpoof = target.__moliEventTargetSlot;
              const returned = target.dispatchEvent(new Event("route"));

              return JSON.stringify({
                ownBefore,
                publicSpoof,
                returned,
                calls
              });
            })()
            "#,
        )
        .expect("simple EventTarget routing private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"ownBefore":false,"publicSpoof":"__wrongSlot","returned":true,"calls":["listener"]}"#
    );
}

#[test]
fn event_listener_object_identity_uses_dynamic_handle_event_without_hidden_cache() {
    let mut vm = new_storage_test_vm("https://event-listener-object-cache-private-slot.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  body.appendChild(target);

  const calls = [];
  const listener = {
    handleEvent(event) {
      calls.push(`${event.type}:${this === listener}`);
    }
  };
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliBoundHandleEvent'))
    .sort()
    .join(',');
  const beforeAdd = internalNames(listener);
  target.addEventListener('bound', listener);
  const afterAdd = internalNames(listener);
  const spoof = () => calls.push('spoof');
  Object.prototype.__moliBoundHandleEvent = spoof;
  listener.__moliBoundHandleEvent = spoof;
  const afterSpoof = internalNames(listener);

  target.dispatchEvent(new Event('bound'));
  listener.handleEvent = function(event) {
    calls.push(`${event.type}:replacement:${this === listener}`);
  };
  target.dispatchEvent(new Event('bound'));
  target.removeEventListener('bound', listener);
  target.dispatchEvent(new Event('bound'));
  target.addEventListener('bound', listener);
  target.dispatchEvent(new Event('bound'));
  target.removeEventListener('bound', listener);
  target.dispatchEvent(new Event('bound'));

  return JSON.stringify({
    beforeAdd,
    afterAdd,
    afterSpoof,
    publicSpoofVisible: listener.__moliBoundHandleEvent === spoof,
    calls
  });
})()
"#,
        )
        .expect("EventListener object bound cache should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"beforeAdd":"","afterAdd":"","afterSpoof":"__moliBoundHandleEvent","publicSpoofVisible":true,"calls":["bound:true","bound:replacement:true","bound:replacement:true"]}"#
    );
}

#[test]
fn document_open_retires_window_document_and_descendant_event_callbacks() {
    let mut vm = new_storage_test_vm("https://document-open-event-callbacks.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const root = document.documentElement ||
                document.appendChild(document.createElement("html"));
              const body = document.body ||
                root.appendChild(document.createElement("body"));
              const oldDocument = document;
              const oldBody = body;
              const calls = [];
              window.addEventListener("click", () => calls.push("window-listener"));
              window.onclick = () => calls.push("window-handler");
              oldDocument.addEventListener("click", () => calls.push("document-listener"));
              oldBody.addEventListener("click", () => calls.push("body-listener"));

              document.open();
              window.dispatchEvent(new Event("click"));
              oldDocument.dispatchEvent(new Event("click"));
              oldBody.dispatchEvent(new Event("click"));
              return JSON.stringify({ calls, windowOnclick: window.onclick });
            })()
            "#,
        )
        .expect("document.open event callback retirement probe should evaluate");

    assert_eq!(result, r#"{"calls":[],"windowOnclick":null}"#);
}

#[test]
fn child_document_open_retires_window_and_document_event_callbacks() {
    let mut vm = new_storage_test_vm("https://child-document-open-event-callbacks.test/");
    vm.eval(
        r#"
        globalThis.__childDocumentOpenEventCalls = [];
        const frame = document.createElement("iframe");
        frame.srcdoc = "<!doctype html><body>child</body>";
        (document.body || document.documentElement || document).appendChild(frame);
        "queued"
        "#,
    )
    .expect("child document setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const child = document.querySelector("iframe").contentWindow;
              const oldDocument = child.document;
              child.addEventListener(
                "click",
                () => __childDocumentOpenEventCalls.push("window-listener")
              );
              child.onclick = () => __childDocumentOpenEventCalls.push("window-handler");
              oldDocument.addEventListener(
                "click",
                () => __childDocumentOpenEventCalls.push("document-listener")
              );

              child.document.open();
              child.dispatchEvent(new child.Event("click"));
              oldDocument.dispatchEvent(new child.Event("click"));
              return JSON.stringify({
                calls: __childDocumentOpenEventCalls,
                windowOnclick: child.onclick
              });
            })()
            "#,
        )
        .expect("child document.open callback retirement probe should evaluate");

    assert_eq!(result, r#"{"calls":[],"windowOnclick":null}"#);
}

#[test]
fn event_subclass_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://event-subclass-private-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const getterDescriptor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
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

              const close = new CloseEvent("close", {
                wasClean: true,
                code: 1000,
                reason: "done"
              });
              const closeOwnBefore = Object.getOwnPropertyNames(close)
                .filter(name => name.startsWith("__moliCloseEvent"))
                .sort();
              close.__moliCloseEventWasClean = false;
              close.__moliCloseEventCode = 4000;
              close.__moliCloseEventReason = "spoofed";
              const closeWasCleanGetter = Object.getOwnPropertyDescriptor(
                CloseEvent.prototype,
                "wasClean"
              ).get;
              const closeCodeGetter = Object.getOwnPropertyDescriptor(
                CloseEvent.prototype,
                "code"
              ).get;
              const closeReasonGetter = Object.getOwnPropertyDescriptor(
                CloseEvent.prototype,
                "reason"
              ).get;
              const fakeClose = {
                __moliCloseEventWasClean: true,
                __moliCloseEventCode: 4999,
                __moliCloseEventReason: "fake"
              };

              const button = document.createElement("button");
              const submit = new SubmitEvent("submit", { submitter: button });
              const submitOwnBefore = Object.getOwnPropertyNames(submit)
                .filter(name => name.startsWith("__moliSubmitEvent"))
                .sort();
              submit.__moliSubmitEventSubmitter = document.createElement("input");
              const submitterGetter = Object.getOwnPropertyDescriptor(
                SubmitEvent.prototype,
                "submitter"
              ).get;
              const fakeSubmit = {
                __moliSubmitEventSubmitter: button
              };

              const formData = new FormData();
              formData.append("real", "value");
              const formDataEvent = new FormDataEvent("formdata", { formData });
              const formDataOwnBefore = Object.getOwnPropertyNames(formDataEvent)
                .filter(name => name.startsWith("__moliFormDataEvent"))
                .sort();
              const spoofedFormData = new FormData();
              spoofedFormData.append("spoofed", "value");
              formDataEvent.__moliFormDataEventFormData = spoofedFormData;
              const formDataGetter = Object.getOwnPropertyDescriptor(
                FormDataEvent.prototype,
                "formData"
              ).get;
              const fakeFormDataEvent = {
                __moliFormDataEventFormData: formData
              };

              return JSON.stringify({
                closeDescriptors: [
                  getterDescriptor(CloseEvent.prototype, "wasClean"),
                  getterDescriptor(CloseEvent.prototype, "code"),
                  getterDescriptor(CloseEvent.prototype, "reason")
                ],
                closeOwnBefore,
                closeValues: [close.wasClean, close.code, close.reason],
                closeSpoofValues: [
                  close.__moliCloseEventWasClean,
                  close.__moliCloseEventCode,
                  close.__moliCloseEventReason
                ],
                fakeClose: [
                  closeWasCleanGetter.call(fakeClose),
                  closeCodeGetter.call(fakeClose),
                  closeReasonGetter.call(fakeClose)
                ],
                submitOwnBefore,
                submitterDescriptor: getterDescriptor(SubmitEvent.prototype, "submitter"),
                submitterIsButton: submit.submitter === button,
                submitSpoofIsInput: submit.__moliSubmitEventSubmitter instanceof HTMLInputElement,
                fakeSubmitterIsNull: submitterGetter.call(fakeSubmit) === null,
                formDataOwnBefore,
                formDataDescriptor: getterDescriptor(FormDataEvent.prototype, "formData"),
                formDataIsReal: formDataEvent.formData === formData,
                formDataSpoofIsSpoofed: formDataEvent.__moliFormDataEventFormData === spoofedFormData,
                fakeFormDataIsUndefined: formDataGetter.call(fakeFormDataEvent) === undefined
              });
            })()
            "#,
        )
        .expect("event subclass private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"closeDescriptors":["wasClean:function:get wasClean:0:undefined:true:true","code:function:get code:0:undefined:true:true","reason:function:get reason:0:undefined:true:true"],"closeOwnBefore":[],"closeValues":[true,1000,"done"],"closeSpoofValues":[false,4000,"spoofed"],"fakeClose":[false,0,""],"submitOwnBefore":[],"submitterDescriptor":"submitter:function:get submitter:0:undefined:true:true","submitterIsButton":true,"submitSpoofIsInput":true,"fakeSubmitterIsNull":true,"formDataOwnBefore":[],"formDataDescriptor":"formData:function:get formData:0:undefined:true:true","formDataIsReal":true,"formDataSpoofIsSpoofed":true,"fakeFormDataIsUndefined":true}"#
    );
}

#[test]
fn event_trusted_slot_ignores_legacy_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://event-trusted-private-slot.test/");

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

              const descriptorShape = (object, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(object, name);
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
                  Object.prototype.hasOwnProperty.call(object, name)
                ].map(value => value === undefined ? "undefined" : String(value)).join(":");
              };
              const event = new Event("plain", { cancelable: true });
              const accessorDescriptors = [
                "returnValue",
                "cancelBubble",
                "isTrusted"
              ].map(name => descriptorShape(
                name === "isTrusted" ? event : Event.prototype,
                name
              ));
              const keys = Object.keys(event).join(",");
              const returnValueBefore = event.returnValue;
              event.returnValue = false;
              const returnValueAfter = event.returnValue;
              const defaultPreventedAfterReturnValue = event.defaultPrevented;
              const cancelBubbleBefore = event.cancelBubble;
              event.cancelBubble = true;
              const cancelBubbleAfter = event.cancelBubble;
              const reinitialized = new Event("before", { cancelable: true });
              reinitialized.initEvent("after", true, true);
              const reinitializedShape = {
                keys: Object.keys(reinitialized).join(","),
                accessors: [
                  "returnValue",
                  "cancelBubble",
                  "isTrusted"
                ].map(name => descriptorShape(
                  name === "isTrusted" ? reinitialized : Event.prototype,
                  name
                )),
                type: reinitialized.type,
                bubbles: reinitialized.bubbles,
                cancelable: reinitialized.cancelable,
                defaultPrevented: reinitialized.defaultPrevented,
                trusted: reinitialized.isTrusted
              };
              const plainOwnBefore = Object.getOwnPropertyNames(event).includes("__lmTrusted");
              event.__lmTrusted = true;
              const trustedGetter = Object.getOwnPropertyDescriptor(event, "isTrusted").get;
              let fakeTrusted;
              try {
                trustedGetter.call({ __lmTrusted: true });
                fakeTrusted = "none";
              } catch (error) {
                fakeTrusted = error.name;
              }

              const button = document.createElement("button");
              document.body.appendChild(button);
              button.focus();
              let trustedDispatch = null;
              button.addEventListener("keydown", dispatched => {
                const ownBefore = Object.getOwnPropertyNames(dispatched).includes("__lmTrusted");
                dispatched.__lmTrusted = false;
                trustedDispatch = {
                  ownBefore,
                  trustedAfterSpoof: dispatched.isTrusted,
                  spoofedOwnValue: dispatched.__lmTrusted
                };
              }, { once: true });
              const dispatched = __moliDispatchTrustedKey(
                "keydown",
                "Tab",
                "Tab",
                false,
                false,
                false,
                false
              );

              return JSON.stringify({
                accessorDescriptors,
                keys,
                returnValueBefore,
                returnValueAfter,
                defaultPreventedAfterReturnValue,
                cancelBubbleBefore,
                cancelBubbleAfter,
                reinitializedShape,
                plainOwnBefore,
                plainTrustedAfterSpoof: event.isTrusted,
                plainSpoofedOwnValue: event.__lmTrusted,
                fakeTrusted,
                dispatched,
                trustedDispatch
              });
            })()
            "#,
        )
        .expect("event trusted private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"accessorDescriptors":["returnValue:function:get returnValue:0:function:set returnValue:1:true:true:true","cancelBubble:function:get cancelBubble:0:function:set cancelBubble:1:true:true:true","isTrusted:function:get isTrusted:0:undefined:undefined:undefined:true:false:true"],"keys":"type,target,srcElement,currentTarget,defaultPrevented,bubbles,cancelable,isTrusted,composed,eventPhase","returnValueBefore":true,"returnValueAfter":false,"defaultPreventedAfterReturnValue":true,"cancelBubbleBefore":false,"cancelBubbleAfter":true,"reinitializedShape":{"keys":"type,target,srcElement,currentTarget,defaultPrevented,bubbles,cancelable,isTrusted,composed,eventPhase","accessors":["returnValue:function:get returnValue:0:function:set returnValue:1:true:true:true","cancelBubble:function:get cancelBubble:0:function:set cancelBubble:1:true:true:true","isTrusted:function:get isTrusted:0:undefined:undefined:undefined:true:false:true"],"type":"after","bubbles":true,"cancelable":true,"defaultPrevented":false,"trusted":false},"plainOwnBefore":false,"plainTrustedAfterSpoof":false,"plainSpoofedOwnValue":true,"fakeTrusted":"TypeError","dispatched":true,"trustedDispatch":{"ownBefore":false,"trustedAfterSpoof":true,"spoofedOwnValue":false}}"#
    );
}

#[test]
fn stop_propagation_at_target_capture_skips_target_bubble_listeners() {
    let mut vm = new_storage_test_vm("https://event-target-stop-propagation.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const nativeTarget = document.createElement("div");
              const nativeCalls = [];
              nativeTarget.addEventListener("x", event => {
                nativeCalls.push(`capture:${event.eventPhase}`);
                event.stopPropagation();
              }, { capture: true });
              nativeTarget.addEventListener("x", event => {
                nativeCalls.push(`capture2:${event.eventPhase}`);
              }, { capture: true });
              nativeTarget.addEventListener("x", () => nativeCalls.push("bubble"));
              nativeTarget.dispatchEvent(new Event("x", { bubbles: true }));

              const handlerTarget = document.createElement("div");
              const handlerCalls = [];
              handlerTarget.addEventListener("y", () => handlerCalls.push("capture"), { capture: true });
              handlerTarget.ony = event => {
                handlerCalls.push("handler");
                event.stopPropagation();
              };
              handlerTarget.addEventListener("y", () => handlerCalls.push("bubble"));
              handlerTarget.dispatchEvent(new Event("y", { bubbles: true }));

              return `${nativeCalls.join(",")}|${handlerCalls.join(",")}`;
            })()
            "#,
        )
        .expect("target stopPropagation dispatch probe should evaluate");

    assert_eq!(result, "capture:2,capture2:2|handler,capture");
}
#[test]
fn bubble_stop_propagation_keeps_current_ancestor_listeners() {
    let mut vm = new_storage_test_vm("https://event-bubble-stop-propagation.test/");

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
              const outer = document.createElement("div");
              const inner = document.createElement("span");
              outer.appendChild(inner);
              document.body.appendChild(outer);

              const stoppedCalls = [];
              outer.onx = event => {
                stoppedCalls.push("handler");
                event.stopPropagation();
              };
              outer.addEventListener("x", () => stoppedCalls.push("listener"));
              document.body.addEventListener("x", () => stoppedCalls.push("body"));
              inner.dispatchEvent(new Event("x", { bubbles: true }));

              const immediateCalls = [];
              outer.ony = event => {
                immediateCalls.push("handler");
                event.stopImmediatePropagation();
              };
              outer.addEventListener("y", () => immediateCalls.push("listener"));
              inner.dispatchEvent(new Event("y", { bubbles: true }));

              const sameTargetCalls = [];
              inner.addEventListener("z", event => {
                sameTargetCalls.push("first");
                event.stopPropagation();
              });
              inner.addEventListener("z", () => sameTargetCalls.push("second"));
              outer.addEventListener("z", () => sameTargetCalls.push("outer"));
              inner.dispatchEvent(new Event("z", { bubbles: true }));

              return `${stoppedCalls.join(",")}|${immediateCalls.join(",")}|${sameTargetCalls.join(",")}`;
            })()
            "#,
        )
        .expect("bubble stopPropagation dispatch probe should evaluate");

    assert_eq!(result, "handler,listener|handler|first,second");
}
#[test]
fn event_listener_exceptions_report_window_error_and_continue_dispatch() {
    let mut vm = new_storage_test_vm("https://event-listener-error-report.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const thrown = { name: "test" };
              const calls = [];
              const errors = [];
              window.onerror = function(message, source, line, column, error) {
                errors.push({
                  messageType: typeof message,
                  exact: error === thrown,
                  sourceType: typeof source,
                  lineType: typeof line,
                  columnType: typeof column
                });
                return true;
              };

              target.addEventListener("foo", {
                get handleEvent() {
                  calls.push("get");
                  throw thrown;
                }
              });
              target.addEventListener("foo", () => calls.push("after"));

              const returned = target.dispatchEvent(new Event("foo"));
              return JSON.stringify({ returned, calls, errors });
            })()
            "#,
        )
        .expect("event listener exception report probe should evaluate");

    assert_eq!(
        result,
        r#"{"returned":true,"calls":["get","after"],"errors":[{"messageType":"string","exact":true,"sourceType":"string","lineType":"number","columnType":"number"}]}"#
    );
}

#[test]
fn synthetic_error_event_uses_normal_window_event_handler_arguments() {
    let mut vm = new_storage_test_vm("https://synthetic-window-error-event.test/");

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
              const observations = {};
              document.body.onerror = (...args) => {
                observations.argumentCount = args.length;
                observations.receivedEvent = args[0] === event;
                return true;
              };
              const event = new Event("error", { bubbles: true, cancelable: true });
              document.body.dispatchEvent(event);
              observations.defaultPrevented = event.defaultPrevented;

              const errorEventObservations = {};
              document.body.onerror = (...args) => {
                errorEventObservations.argumentCount = args.length;
                errorEventObservations.message = args[0];
                errorEventObservations.source = args[1];
                errorEventObservations.errorMatches = args[4] === errorEvent.error;
                return true;
              };
              const errorEvent = new ErrorEvent("error", {
                cancelable: true,
                message: "boom",
                filename: "probe.js"
              });
              window.dispatchEvent(errorEvent);
              errorEventObservations.defaultPrevented = errorEvent.defaultPrevented;
              observations.errorEvent = errorEventObservations;
              return JSON.stringify(observations);
            })()
            "#,
        )
        .expect("synthetic error event handler probe should evaluate");

    assert_eq!(
        result,
        r#"{"argumentCount":1,"receivedEvent":true,"defaultPrevented":false,"errorEvent":{"argumentCount":5,"message":"boom","source":"probe.js","errorMatches":true,"defaultPrevented":true}}"#
    );
}

#[test]
fn window_onerror_null_assignment_supersedes_uncompiled_body_attribute() {
    let mut vm = new_storage_test_vm("https://window-onerror-null-override.test/");
    vm.eval(
        r#"
        (() => {
          if (!document.documentElement) {
            document.appendChild(document.createElement("html"));
          }
          if (!document.body) {
            document.documentElement.appendChild(document.createElement("body"));
          }
          globalThis.__bodyErrorAttributeRan = false;
          document.body.setAttribute("onerror", "globalThis.__bodyErrorAttributeRan = true");
          window.onerror = null;
          addEventListener("error", event => event.preventDefault(), { once: true });
        })()
        "#,
    )
    .expect("body onerror null override setup should evaluate");

    vm.report_window_script_failure_and_checkpoint_for_test(
        "window onerror null override probe",
        Some("https://window-onerror-null-override.test/probe.js"),
        None,
    );

    let result = vm
        .eval("JSON.stringify([globalThis.__bodyErrorAttributeRan, window.onerror === null])")
        .expect("body onerror null override result should evaluate");
    assert_eq!(result, "[false,true]");
}

#[test]
fn window_script_failure_report_does_not_infer_constructor_from_message_text() {
    let mut vm = new_storage_test_vm("https://window-script-failure-report.test/");
    vm.eval(
        r#"
        (() => {
          const OriginalError = Error;
          function FakeError() {}
          globalThis.Error = FakeError;
          globalThis.__windowScriptFailureReports = [];
          addEventListener("error", event => {
            __windowScriptFailureReports.push([
              event.error && event.error.constructor && event.error.constructor.name,
              event.error instanceof OriginalError,
              event.error instanceof FakeError,
              event.error instanceof SyntaxError,
              event.error instanceof WebAssembly.LinkError,
              event.message
            ].join("|"));
            event.preventDefault();
          });
          return "ready";
        })()
        "#,
    )
    .expect("window error listener setup should evaluate");

    vm.report_window_script_failure_and_checkpoint_for_test(
        "CompileError LinkError SyntaxError user-controlled text",
        Some("https://window-script-failure-report.test/script.js"),
        None,
    );

    let result = vm
        .eval("__windowScriptFailureReports.join(',')")
        .expect("window script failure report result should evaluate");
    assert_eq!(
        result,
        "Error|true|false|false|false|CompileError LinkError SyntaxError user-controlled text"
    );
}

#[test]
fn window_script_failure_error_event_is_trusted_without_trusting_synthetic_events() {
    let mut vm = new_storage_test_vm("https://window-error-trusted.test/");
    vm.eval(
        r#"
        (() => {
          globalThis.__windowErrorTrust = [];
          addEventListener("error", event => {
            __windowErrorTrust.push(event.isTrusted);
            event.preventDefault();
          });
        })()
        "#,
    )
    .expect("window error trust listener setup should evaluate");

    vm.report_window_error_body_best_effort(
        "trusted browser-generated error",
        Some("https://window-error-trusted.test/script.js"),
        None,
    );
    let result = vm
        .eval(
            r#"
            window.dispatchEvent(new ErrorEvent("error", { cancelable: true }));
            JSON.stringify(globalThis.__windowErrorTrust)
            "#,
        )
        .expect("window error trust result should evaluate");

    assert_eq!(result, "[true,false]");
}

#[test]
fn cross_realm_event_listener_object_errors_report_to_listener_window() {
    let mut vm = new_storage_test_vm("https://event-listener-cross-realm.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "eventListenerGlobalObject";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = eventListenerGlobalObject;
              const target = new EventTarget();
              const missingHandleEvent = new child.Object();
              const events = [];

              child.addEventListener("error", event => {
                events.push({
                  targetIsChild: event.target === child,
                  errorIsChildTypeError: event.error &&
                    event.error.constructor === child.TypeError
                });
                event.preventDefault();
              });

              target.addEventListener("boom", missingHandleEvent);
              target.dispatchEvent(new Event("boom"));

              return JSON.stringify(events);
            })()
            "#,
        )
        .expect("cross-realm EventListener object error probe should evaluate");

    assert_eq!(
        result,
        r#"[{"targetIsChild":true,"errorIsChildTypeError":true}]"#
    );
}
#[test]
fn cross_realm_event_listener_error_cases_report_to_listener_window() {
    let mut vm = new_storage_test_vm("https://event-listener-cross-realm-cases.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "eventListenerGlobalObject";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = eventListenerGlobalObject;
              const target = new EventTarget();
              const results = [];

              function record(name, expectedConstructor, listener) {
                function onerror(event) {
                  results.push([
                    name,
                    event.target === child,
                    !!event.error && event.error.constructor === expectedConstructor
                  ]);
                  event.preventDefault();
                }
                child.addEventListener("error", onerror);
                target.addEventListener(name, listener);
                target.dispatchEvent(new Event(name));
                child.removeEventListener("error", onerror);
              }

              record("missing", child.TypeError, new child.Object());

              const nonCallable = new child.Object();
              nonCallable.handleEvent = null;
              record("non-callable", child.TypeError, nonCallable);

              const revokedHandle = new child.Object();
              const handleProxy = child.Proxy.revocable(function() {}, {});
              revokedHandle.handleEvent = handleProxy.proxy;
              handleProxy.revoke();
              record("revoked-handle", child.TypeError, revokedHandle);

              const objectProxy = child.Proxy.revocable({}, {});
              objectProxy.revoke();
              record("revoked-object", child.TypeError, objectProxy.proxy);

              const functionProxy = child.Proxy.revocable(function() {}, {});
              functionProxy.revoke();
              record("revoked-function", child.TypeError, functionProxy.proxy);

              return JSON.stringify(results);
            })()
            "#,
        )
        .expect("cross-realm EventListener error matrix should evaluate");

    assert_eq!(
        result,
        r#"[["missing",true,true],["non-callable",true,true],["revoked-handle",true,true],["revoked-object",true,true],["revoked-function",true,true]]"#
    );
}
#[test]
fn child_window_listener_markers_avoid_public_getters_and_preserve_error_surface() {
    let mut vm = new_storage_test_vm("https://event-listener-child-marker.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              frame.name = "childMarkerWindow";
              (document.body || document.documentElement || document).appendChild(frame);
              const child = childMarkerWindow;
              const target = new EventTarget();
              const errors = [];
              const registration = [];

              const getterListener = {};
              Object.defineProperty(getterListener, "__moliCallbackErrorWindowHandle", {
                get() {
                  throw new Error("marker getter should not run");
                }
              });
              try {
                target.addEventListener("getter", getterListener);
                registration.push("ok");
              } catch (error) {
                registration.push(error.message);
              }
              try {
                matchMedia("(min-width: 0px)").addEventListener("change", getterListener);
                registration.push("mql-ok");
              } catch (error) {
                registration.push(`mql:${error.message}`);
              }

              child.onerror = function(message, source, line, column, error, extra) {
                errors.push({
                  argc: arguments.length,
                  messageType: typeof message,
                  sourceType: typeof source,
                  lineType: typeof line,
                  columnType: typeof column,
                  errorIsChildTypeError: !!error && error.constructor === child.TypeError,
                  extraIsUndefined: extra === undefined,
                  currentTargetIsChild: event.currentTarget === child
                });
                return true;
              };

              const created = child.Object.create(null);
              target.addEventListener("created", created);
              target.dispatchEvent(new Event("created"));

              let proxyCallName = "none";
              try {
                child.Proxy({}, {});
              } catch (error) {
                proxyCallName = error.name;
              }
              const proxyConstructWorks = !!new child.Proxy({}, {});

              return JSON.stringify({
                registration,
                errors,
                proxyCallName,
                proxyConstructWorks
              });
            })()
            "#,
        )
        .expect("child listener marker and error surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"registration":["ok","mql-ok"],"errors":[{"argc":5,"messageType":"string","sourceType":"string","lineType":"number","columnType":"number","errorIsChildTypeError":true,"extraIsUndefined":true,"currentTargetIsChild":true}],"proxyCallName":"TypeError","proxyConstructWorks":true}"#
    );
}

#[test]
fn child_window_forwarded_constructors_use_captured_native_intrinsics() {
    let mut vm = new_storage_test_vm("https://child-window-native-intrinsics.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const child = frame.contentWindow;
              const childDocument = frame.contentDocument;

              const object = new child.Object();
              const nullPrototypeObject = child.Object.create(null);
              const proxyTarget = {};
              const proxy = new child.Proxy(proxyTarget, {});
              const revocable = child.Proxy.revocable({}, {});
              const range = new child.Range();
              const staticRange = new child.StaticRange({
                startContainer: childDocument,
                startOffset: 0,
                endContainer: childDocument,
                endOffset: childDocument.childNodes.length
              });
              const readable = new child.ReadableStream();

              let proxyCallError = "none";
              try {
                child.Proxy({}, {});
              } catch (error) {
                proxyCallError = error.name;
              }
              revocable.revoke();

              return [
                Object.getPrototypeOf(object) === child.Object.prototype,
                Object.getPrototypeOf(nullPrototypeObject) === null,
                proxy !== proxyTarget,
                proxyCallError,
                range.startContainer === childDocument,
                range.endContainer === childDocument,
                staticRange.startContainer === childDocument,
                staticRange.endContainer === childDocument,
                typeof readable.getReader === "function"
              ].join("|");
            })()
            "#,
        )
        .expect("child window forwarded constructors should evaluate without recursion");

    assert_eq!(result, "true|true|true|TypeError|true|true|true|true|true");
}

#[test]
fn cross_realm_listener_throw_reports_listener_global_not_target_global() {
    let mut vm = new_storage_test_vm("https://event-listener-multiple-globals.test/");

    vm.eval(
        r#"
        (() => {
          const host = document.body || document.documentElement || document;
          const frameA = document.createElement("iframe");
          frameA.srcdoc = `<script>
            function listener() { throw new Error(); }
            objectListener = {};
          <\/script>`;
          const frameB = document.createElement("iframe");
          frameB.srcdoc = `<script>
            function handleEvent() { throw new Error(); }
          <\/script>`;
          const frameC = document.createElement("iframe");
          host.appendChild(frameA);
          host.appendChild(frameB);
          host.appendChild(frameC);
          globalThis.__crossRealmFrames = [frameA, frameB, frameC];
          return "ready";
        })()
        "#,
    )
    .expect("cross-realm multiple global frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
            (() => {
              const [frameA, frameB, frameC] = globalThis.__crossRealmFrames;
              const w = frameA.contentWindow;
              const w2 = frameB.contentWindow;
              const w3 = frameC.contentWindow;
              const results = [];

              function nextError(windows, callback) {
                function listener(event) {
                  results.push(callback(event));
                  event.preventDefault();
                }
                for (const current of windows) current.addEventListener("error", listener);
                return () => {
                  for (const current of windows) current.removeEventListener("error", listener);
                };
              }

              const functionTarget = new w2.EventTarget();
              functionTarget.addEventListener("party", w.listener);
              let cleanup = nextError([window, w, w2], event => [
                "function",
                event.target === w,
                !!event.error && event.error.constructor === w.Error
              ]);
              functionTarget.dispatchEvent(new Event("party"));
              results.push([
                "function-window-event-restored",
                typeof event === "undefined"
              ]);
              cleanup();

              const objectListener = w.objectListener;
              objectListener.handleEvent = w2.handleEvent;
              const objectTarget = new w3.EventTarget();
              objectTarget.addEventListener("party", objectListener);
              cleanup = nextError([window, w, w2, w3], event => [
                "object",
                event.target === w,
                !!event.error && event.error.constructor === w2.Error
              ]);
              objectTarget.dispatchEvent(new Event("party"));
              results.push([
                "object-window-event-restored",
                typeof event === "undefined"
              ]);
              cleanup();

              return JSON.stringify(results);
            })()
            "#,
        )
        .expect("cross-realm multiple global listener errors should evaluate");

    assert_eq!(
        result,
        r#"[["function",true,true],["function-window-event-restored",true],["object",true,true],["object-window-event-restored",true]]"#
    );
}
#[test]
fn event_timestamp_uses_performance_origin_and_safe_resolution() {
    let mut vm = new_storage_test_vm("https://event-timestamp.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const before = performance.now();
              const event = new MouseEvent("click");
              const after = performance.now();
              const descriptor = Object.getOwnPropertyDescriptor(Event.prototype, "timeStamp");
              const getterValue = descriptor.get.call(event);
              const delta = Math.round((new Event("b").timeStamp - event.timeStamp) * 1000);

              return JSON.stringify({
                hasGetter: typeof descriptor.get === "function",
                getterName: descriptor.get && descriptor.get.name,
                getterLength: descriptor.get && descriptor.get.length,
                enumerable: descriptor.enumerable,
                configurable: descriptor.configurable,
                hasOwnTimeStamp: Object.prototype.hasOwnProperty.call(event, "timeStamp"),
                withinNowRange: event.timeStamp >= before && event.timeStamp <= after,
                getterMatchesOwnValue: Object.is(getterValue, event.timeStamp),
                safeResolution: delta >= 0 && delta % 5 === 0
              });
            })()
            "#,
        )
        .expect("event timestamp probe should evaluate");

    assert_eq!(
        result,
        r#"{"hasGetter":true,"getterName":"get timeStamp","getterLength":0,"enumerable":true,"configurable":true,"hasOwnTimeStamp":false,"withinNowRange":true,"getterMatchesOwnValue":true,"safeResolution":true}"#
    );
}
#[test]
fn performance_entry_accessors_return_entries_sorted_by_start_time() {
    let mut vm = new_storage_test_vm("https://performance-entry-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              performance.measure("late", { start: 20, duration: 1 });
              performance.measure("early", { start: 0, duration: 1 });
              return [
                performance.getEntriesByType("measure").map(entry => entry.name).join(","),
                performance.getEntries().filter(entry => entry.entryType === "measure").map(entry => entry.name).join(",")
              ].join("|");
            })()
            "#,
        )
        .expect("performance entry ordering probe should evaluate");

    assert_eq!(result, "early,late|early,late");
}
#[test]
fn legacy_event_init_methods_short_circuit_while_dispatching() {
    let mut vm = new_storage_test_vm("https://event-init-while-dispatching.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const event = new MouseEvent("x", {
                bubbles: false,
                cancelable: false,
                screenX: 7,
                clientX: 9,
                ctrlKey: false,
                button: 0
              });
              target.addEventListener("x", () => {
                event.initMouseEvent(
                  "changed",
                  true,
                  true,
                  window,
                  1,
                  2,
                  3,
                  4,
                  5,
                  true,
                  true,
                  true,
                  true,
                  1,
                  document
                );
              });
              target.dispatchEvent(event);

              const keyboard = new KeyboardEvent("key", { key: "A", repeat: false });
              target.addEventListener("key", () => {
                keyboard.initKeyboardEvent("changed", true, true, window, "B", 1, "", true, "");
              });
              target.dispatchEvent(keyboard);

              return JSON.stringify({
                mouseType: event.type,
                mouseBubbles: event.bubbles,
                mouseCancelable: event.cancelable,
                screenX: event.screenX,
                clientX: event.clientX,
                ctrlKey: event.ctrlKey,
                button: event.button,
                keyType: keyboard.type,
                key: keyboard.key,
                repeat: keyboard.repeat,
                location: keyboard.location
              });
            })()
            "#,
        )
        .expect("legacy event init while dispatching probe should evaluate");

    assert_eq!(
        result,
        r#"{"mouseType":"x","mouseBubbles":false,"mouseCancelable":false,"screenX":7,"clientX":9,"ctrlKey":false,"button":0,"keyType":"key","key":"A","repeat":false,"location":0}"#
    );
}
#[test]
fn legacy_event_init_methods_short_circuit_for_all_wpt_event_classes() {
    let mut vm = new_storage_test_vm("https://event-init-while-dispatching-all.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const failures = [];
              const cases = {
                KeyboardEvent: {
                  event: new KeyboardEvent("type", { key: "A" }),
                  init: event => event.initKeyboardEvent("type2", true, true, null, "a", 1, "", true, ""),
                  check: event => {
                    if (event.key !== "A") failures.push("KeyboardEvent:key");
                    if (event.repeat !== false) failures.push("KeyboardEvent:repeat");
                    if (event.location !== 0) failures.push("KeyboardEvent:location");
                  }
                },
                MouseEvent: {
                  event: new MouseEvent("type"),
                  init: event => event.initMouseEvent("type2", true, true, null, 0, 1, 1, 1, 1, true, true, true, true, 1, null),
                  check: event => {
                    for (const name of ["screenX", "screenY", "clientX", "clientY", "button"]) {
                      if (event[name] !== 0) failures.push(`MouseEvent:${name}:${event[name]}`);
                    }
                    for (const name of ["ctrlKey", "altKey", "shiftKey", "metaKey"]) {
                      if (event[name] !== false) failures.push(`MouseEvent:${name}`);
                    }
                  }
                },
                CustomEvent: {
                  event: new CustomEvent("type"),
                  init: event => event.initCustomEvent("type2", true, true, 1),
                  check: event => {
                    if (event.detail !== null) failures.push(`CustomEvent:detail:${event.detail}`);
                  }
                },
                UIEvent: {
                  event: new UIEvent("type"),
                  init: event => event.initUIEvent("type2", true, true, window, 1),
                  check: event => {
                    if (event.view !== null) failures.push("UIEvent:view");
                    if (event.detail !== 0) failures.push(`UIEvent:detail:${event.detail}`);
                  }
                },
                Event: {
                  event: new Event("type"),
                  init: event => event.initEvent("type2", true, true),
                  check: event => {
                    if (event.type !== "type") failures.push(`Event:type:${event.type}`);
                    if (event.bubbles !== false) failures.push("Event:bubbles");
                    if (event.cancelable !== false) failures.push("Event:cancelable");
                  }
                }
              };

              for (const [name, entry] of Object.entries(cases)) {
                const target = document.createElement("div");
                target.addEventListener("type", () => {
                  try {
                    entry.init(entry.event);
                    entry.check(entry.event);
                  } catch (error) {
                    failures.push(`${name}:throw:${error && error.name}`);
                  }
                });
                target.dispatchEvent(entry.event);
              }
              return failures.join("|") || "ok";
            })()
            "#,
        )
        .expect("all-class legacy init short-circuit probe should evaluate");

    assert_eq!(result, "ok");
}
#[test]
fn event_subclass_constructors_expose_legacy_keyboard_codes_and_validate_view() {
    let mut vm = new_storage_test_vm("https://event-subclass-constructors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new KeyboardEvent("key");
              const nonDefaults = new KeyboardEvent("key", {
                charCode: 7,
                keyCode: 8,
                which: 9,
                view: window
              });
              let wrongViewName = "none";
              try {
                new UIEvent("x", { view: 7 });
              } catch (error) {
                wrongViewName = error && error.name;
              }
              return JSON.stringify({
                defaults: [defaults.charCode, defaults.keyCode, defaults.which],
                nonDefaults: [nonDefaults.charCode, nonDefaults.keyCode, nonDefaults.which],
                viewIsWindow: nonDefaults.view === window,
                wrongViewName,
                lengths: [
                  KeyboardEvent.prototype.initKeyboardEvent.length,
                  MouseEvent.prototype.initMouseEvent.length
                ]
              });
            })()
            "#,
        )
        .expect("event subclass constructor probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":[0,0,0],"nonDefaults":[7,8,9],"viewIsWindow":true,"wrongViewName":"TypeError","lengths":[7,15]}"#
    );
}

#[test]
fn mouse_event_exposes_pointer_lock_movement_dictionary_members() {
    let mut vm = new_storage_test_vm("https://mouse-event-movement.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new MouseEvent("default");
              const initialized = new MouseEvent("initialized", {
                movementX: 10,
                movementY: -11
              });
              const wheel = new WheelEvent("wheel", {
                movementX: 12,
                movementY: -13
              });
              const pointer = new PointerEvent("pointer", {
                movementX: 14,
                movementY: -15
              });
              return JSON.stringify({
                present: "movementX" in defaults && "movementY" in defaults,
                defaults: [defaults.movementX, defaults.movementY],
                initialized: [initialized.movementX, initialized.movementY],
                wheel: [wheel.movementX, wheel.movementY],
                pointer: [pointer.movementX, pointer.movementY]
              });
            })()
            "#,
        )
        .expect("MouseEvent movement member probe should evaluate");

    assert_eq!(
        result,
        r#"{"present":true,"defaults":[0,0],"initialized":[10,-11],"wheel":[12,-13],"pointer":[14,-15]}"#
    );
}

#[test]
fn pointer_event_converts_tilt_and_spherical_angles() {
    let mut vm = new_storage_test_vm("https://pointer-event-angles.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const defaults = new PointerEvent('pointer');
              const fromTilt = new PointerEvent('pointer', { tiltX: -45 });
              const fromSpherical = new PointerEvent('pointer', {
                azimuthAngle: 3 * Math.PI / 2,
                altitudeAngle: Math.PI / 4
              });
              const mixed = new PointerEvent('pointer', {
                tiltX: 45,
                azimuthAngle: Math.PI / 4
              });
              let tiltReads = 0;
              const observed = new PointerEvent('pointer', {
                get tiltX() {
                  tiltReads += 1;
                  return 45;
                }
              });
              let nonFinite = 'accepted';
              try {
                new PointerEvent('pointer', { altitudeAngle: Infinity });
              } catch (error) {
                nonFinite = error && error.name;
              }
              return JSON.stringify({
                defaults: [
                  defaults.tiltX,
                  defaults.tiltY,
                  defaults.azimuthAngle,
                  defaults.altitudeAngle
                ],
                fromTilt: [
                  fromTilt.tiltX,
                  fromTilt.tiltY,
                  fromTilt.azimuthAngle,
                  fromTilt.altitudeAngle
                ],
                fromSpherical: [
                  fromSpherical.tiltX,
                  fromSpherical.tiltY,
                  fromSpherical.azimuthAngle,
                  fromSpherical.altitudeAngle
                ],
                mixed: [
                  mixed.tiltX,
                  mixed.tiltY,
                  mixed.azimuthAngle,
                  mixed.altitudeAngle
                ],
                observed: [tiltReads, observed.tiltX, observed.altitudeAngle],
                nonFinite
              });
            })()
            "#,
        )
        .expect("PointerEvent angle conversion probe should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":[0,0,0,1.5707963267948966],"fromTilt":[-45,0,3.141592653589793,0.7853981633974483],"fromSpherical":[0,-45,4.71238898038469,0.7853981633974483],"mixed":[45,0,0.7853981633974483,1.5707963267948966],"observed":[1,45,0.7853981633974483],"nonFinite":"TypeError"}"#
    );
}

#[test]
fn pointer_event_sequences_preserve_identity_and_secure_context_exposure() {
    let mut secure_vm = new_storage_test_vm("https://pointer-event-sequences.test/");
    let secure = secure_vm
        .eval(
            r#"
            (() => {
              try {
                Object.defineProperty(globalThis, "isSecureContext", { value: false });
              } catch {}
              const publicSecureContextSpoofed = isSecureContext === false;
              const predicted = new PointerEvent("pointermove", { clientX: 20 });
              const coalesced = new PointerEvent("pointermove", { clientX: 5 });
              const event = new PointerEvent("pointermove", {
                predictedEvents: [predicted],
                coalescedEvents: [coalesced]
              });
              const firstPredicted = event.getPredictedEvents();
              const secondPredicted = event.getPredictedEvents();
              const firstCoalesced = event.getCoalescedEvents();
              const secondCoalesced = event.getCoalescedEvents();
              firstPredicted.length = 0;
              firstCoalesced.length = 0;
              let invalidEntry = "accepted";
              try {
                new PointerEvent("pointermove", { predictedEvents: [{}] });
              } catch (error) {
                invalidEntry = error && error.name;
              }
              let fakeReceiver = "accepted";
              try {
                PointerEvent.prototype.getPredictedEvents.call({});
              } catch (error) {
                fakeReceiver = error && error.name;
              }
              return JSON.stringify({
                methods: [
                  typeof event.getPredictedEvents,
                  typeof event.getCoalescedEvents,
                  event.getPredictedEvents.length,
                  event.getCoalescedEvents.length
                ],
                predictedIdentity: secondPredicted[0] === predicted,
                coalescedIdentity: secondCoalesced[0] === coalesced,
                arraysAreFresh: firstPredicted !== secondPredicted &&
                  firstCoalesced !== secondCoalesced,
                mutationIsolated: secondPredicted.length === 1 &&
                  secondCoalesced.length === 1,
                nestedDefaults: [
                  predicted.getPredictedEvents().length,
                  predicted.getCoalescedEvents().length
                ],
                secureBindingIndependent: !publicSecureContextSpoofed ||
                  "getCoalescedEvents" in event,
                invalidEntry,
                fakeReceiver
              });
            })()
            "#,
        )
        .expect("secure PointerEvent sequence probe should evaluate");
    assert_eq!(
        secure,
        r#"{"methods":["function","function",0,0],"predictedIdentity":true,"coalescedIdentity":true,"arraysAreFresh":true,"mutationIsolated":true,"nestedDefaults":[0,0],"secureBindingIndependent":true,"invalidEntry":"TypeError","fakeReceiver":"TypeError"}"#
    );

    let mut insecure_vm = new_storage_test_vm("http://pointer-event-sequences.test/");
    let insecure = insecure_vm
        .eval(
            r#"
            (() => {
              const originalSecureContext = isSecureContext;
              try {
                Object.defineProperty(globalThis, "isSecureContext", { value: true });
              } catch {}
              const publicSecureContextSpoofed = isSecureContext === true;
              const event = new PointerEvent("pointermove");
              return [
                originalSecureContext,
                typeof event.getPredictedEvents,
                "getCoalescedEvents" in event,
                !publicSecureContextSpoofed ||
                  !("getCoalescedEvents" in event)
              ].join("|");
            })()
            "#,
        )
        .expect("insecure PointerEvent exposure probe should evaluate");
    assert_eq!(insecure, "false|function|false|true");
}

#[test]
fn character_data_setters_apply_replace_all_live_range_offsets() {
    let mut vm = new_storage_test_vm("https://character-data-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode("abc");
              const dataRange = document.createRange();
              dataRange.setStart(text, 1);
              dataRange.setEnd(text, 2);
              text.data = "abc";
              const textContentRange = document.createRange();
              textContentRange.setStart(text, 1);
              textContentRange.setEnd(text, 3);
              text.textContent = "abc";
              const nodeValueRange = document.createRange();
              nodeValueRange.setStart(text, 1);
              nodeValueRange.setEnd(text, 3);
              text.nodeValue = "abc";

              const foreignDoc = document.implementation.createHTMLDocument("");
              const foreignText = foreignDoc.createTextNode("abc");
              const foreignRange = foreignDoc.createRange();
              foreignRange.setStart(foreignText, 0);
              foreignRange.setEnd(foreignText, 1);
              foreignText.textContent = "foo";

              const xmlDoc = document.implementation.createDocument(null, "root");
              const xmlComment = xmlDoc.createComment("abc");
              const xmlRange = xmlDoc.createRange();
              xmlRange.setStart(xmlComment, 1);
              xmlRange.setEnd(xmlComment, xmlComment.length);
              xmlComment.textContent = "foo";

              const detachedForeignText = foreignDoc.createTextNode("abcdef");
              const detachedForeignRange = foreignDoc.createRange();
              detachedForeignRange.setStart(detachedForeignText, 1);
              detachedForeignRange.setEnd(detachedForeignText, detachedForeignText.length);
              detachedForeignText.textContent += "foo";

              return [
                dataRange.startContainer === text,
                dataRange.endContainer === text,
                dataRange.startOffset,
                dataRange.endOffset,
                textContentRange.startOffset,
                textContentRange.endOffset,
                nodeValueRange.startOffset,
                nodeValueRange.endOffset,
                text.data,
                foreignRange.startContainer === foreignText,
                foreignRange.startOffset,
                foreignRange.endContainer === foreignText,
                foreignRange.endOffset,
                xmlRange.startContainer === xmlComment,
                xmlRange.startOffset,
                xmlRange.endContainer === xmlComment,
                xmlRange.endOffset,
                detachedForeignRange.startContainer === detachedForeignText,
                detachedForeignRange.startOffset,
                detachedForeignRange.endContainer === detachedForeignText,
                detachedForeignRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("character data setters should apply replace-all live range offsets");

    assert_eq!(
        result,
        "true|true|0|0|0|0|0|0|abc|true|0|true|0|true|0|true|0|true|0|true|0"
    );
}

#[test]
fn range_clone_contents_empty_character_data_range_returns_empty_fragment() {
    let mut vm = new_storage_test_vm("https://range-clone-empty-character-data.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const text = document.createTextNode("abc");
              const range = document.createRange();
              range.setStart(text, 0);
              range.setEnd(text, 0);
              const fragment = range.cloneContents();
              return [
                fragment.nodeType,
                fragment.childNodes.length,
                fragment.textContent,
                range.startContainer === text,
                range.endContainer === text,
                range.startOffset,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("empty CharacterData Range.cloneContents should evaluate");

    assert_eq!(result, "11|0||true|true|0|0");
}
#[test]
fn detached_character_data_edits_update_live_ranges() {
    let mut vm = new_storage_test_vm("https://detached-character-data-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");

              const deleteText = doc.createTextNode("abcdef");
              const deleteRange = doc.createRange();
              deleteRange.setStart(deleteText, 1);
              deleteRange.setEnd(deleteText, 4);
              deleteText.deleteData(1, 2);

              const insertText = doc.createTextNode("abcdef");
              const insertRange = doc.createRange();
              insertRange.setStart(insertText, 1);
              insertRange.setEnd(insertText, 4);
              insertText.insertData(2, "XYZ");

              const replaceText = doc.createTextNode("abcdef");
              const replaceRange = doc.createRange();
              replaceRange.setStart(replaceText, 1);
              replaceRange.setEnd(replaceText, 4);
              replaceText.replaceData(1, 2, "XYZ");

              const resetText = doc.createTextNode("abcdef");
              const resetRange = doc.createRange();
              resetRange.setStart(resetText, 1);
              resetRange.setEnd(resetText, 4);
              resetText.data = resetText.data;

              const nodeValueText = doc.createTextNode("abcdef");
              const nodeValueRange = doc.createRange();
              nodeValueRange.setStart(nodeValueText, 1);
              nodeValueRange.setEnd(nodeValueText, 4);
              nodeValueText.nodeValue = nodeValueText.nodeValue;

              const splitHost = doc.createElement("p");
              const splitText = doc.createTextNode("abcdef");
              splitHost.appendChild(splitText);
              doc.body.appendChild(splitHost);
              const splitTextRange = doc.createRange();
              splitTextRange.setStart(splitText, 1);
              splitTextRange.setEnd(splitText, 4);
              const splitParentRange = doc.createRange();
              splitParentRange.setStart(splitHost, 1);
              splitParentRange.setEnd(splitHost, 1);
              const splitNew = splitText.splitText(2);

              const detachedSplitText = doc.createTextNode("abcdef");
              const detachedSplitRange = doc.createRange();
              detachedSplitRange.setStart(detachedSplitText, 1);
              detachedSplitRange.setEnd(detachedSplitText, 4);
              detachedSplitText.splitText(2);

              return [
                deleteText.data,
                deleteRange.startOffset,
                deleteRange.endOffset,
                insertText.data,
                insertRange.startOffset,
                insertRange.endOffset,
                replaceText.data,
                replaceRange.startOffset,
                replaceRange.endOffset,
                resetRange.startOffset,
                resetRange.endOffset,
                nodeValueRange.startOffset,
                nodeValueRange.endOffset,
                splitText.data,
                splitNew.data,
                splitTextRange.startContainer === splitText,
                splitTextRange.startOffset,
                splitTextRange.endContainer === splitNew,
                splitTextRange.endOffset,
                splitParentRange.startOffset,
                splitParentRange.endOffset,
                detachedSplitText.data,
                detachedSplitRange.startContainer === detachedSplitText,
                detachedSplitRange.startOffset,
                detachedSplitRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached character data edits should update live ranges");

    assert_eq!(
        result,
        "adef|1|2|abXYZcdef|1|7|aXYZdef|1|5|0|0|0|0|ab|cdef|true|1|true|2|2|2|ab|true|1|2"
    );
}
#[test]
fn range_select_node_rejects_parentless_nodes_and_doctype_contents() {
    let mut vm = new_storage_test_vm("https://range-select-node-errors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const range = document.createRange();
              const detachedElement = document.createElement("div");
              const detachedText = document.createTextNode("abc");
              const fragment = document.createDocumentFragment();
              const docType = document.implementation.createDocumentType("html", "", "");
              const host = document.createElement("section");
              const child = document.createElement("span");
              host.appendChild(child);
              (document.body || document.documentElement || document).appendChild(host);

              function thrownName(callback) {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error.name}:${error.code}`;
                }
              }

              const parentlessElement = thrownName(() => range.selectNode(detachedElement));
              const parentlessText = thrownName(() => range.selectNode(detachedText));
              const parentlessFragment = thrownName(() => range.selectNode(fragment));
              const parentlessDocument = thrownName(() => range.selectNode(document));
              const doctypeContents = thrownName(() => range.selectNodeContents(docType));

              range.selectNode(child);
              const selectNodeState = [
                range.startContainer === host,
                range.startOffset,
                range.endContainer === host,
                range.endOffset
              ].join(",");

              range.selectNodeContents(child);
              const contentsState = [
                range.startContainer === child,
                range.startOffset,
                range.endContainer === child,
                range.endOffset
              ].join(",");

              return [
                parentlessElement,
                parentlessText,
                parentlessFragment,
                parentlessDocument,
                doctypeContents,
                selectNodeState,
                contentsState
              ].join("|");
            })()
            "#,
        )
        .expect("Range selectNode error checks should evaluate");

    assert_eq!(
        result,
        "InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|InvalidNodeTypeError:24|true,0,true,1|true,0,true,0"
    );
}
#[test]
fn range_intersects_node_uses_exclusive_adjacent_boundaries() {
    let mut vm = new_storage_test_vm("https://range-intersects-node.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const a = document.createElement("a");
              const b = document.createElement("b");
              const c = document.createElement("c");
              host.append(a, b, c);
              (document.body || document.documentElement || document).appendChild(host);
              const range = document.createRange();
              range.setStart(host, 1);
              range.setEnd(host, 2);

              function thrownName(callback) {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error.name;
                }
              }

              return [
                range.intersectsNode(a),
                range.intersectsNode(b),
                range.intersectsNode(c),
                thrownName(() => range.intersectsNode()),
                thrownName(() => range.intersectsNode(null)),
                thrownName(() => range.intersectsNode({}))
              ].join("|");
            })()
            "#,
        )
        .expect("Range intersectsNode checks should evaluate");

    assert_eq!(result, "false|true|false|TypeError|TypeError|TypeError");
}
#[test]
fn static_range_constructor_sets_immutable_abstract_range_boundaries() {
    let mut vm = new_storage_test_vm("https://static-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              host.id = "host";
              host.append("a", document.createElement("span"), "b");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              const doctype = document.implementation.createDocumentType("html", "", "");
              const range = new StaticRange({
                startContainer: text,
                startOffset: 0,
                endContainer: host,
                endOffset: 3
              });
              host.insertBefore(document.createTextNode("x"), text);
              const attrError = (() => {
                try {
                  new StaticRange({
                    startContainer: host.getAttributeNode("id"),
                    startOffset: 0,
                    endContainer: host,
                    endOffset: 0
                  });
                } catch (error) {
                  return error.name;
                }
              })();
              const doctypeError = (() => {
                try {
                  new StaticRange({
                    startContainer: doctype,
                    startOffset: 0,
                    endContainer: doctype,
                    endOffset: 0
                  });
                } catch (error) {
                  return error.name;
                }
              })();
              const missingError = (() => {
                try {
                  new StaticRange({ startOffset: 0, endContainer: host, endOffset: 0 });
                } catch (error) {
                  return error.name;
                }
              })();
              return JSON.stringify({
                ctor: typeof StaticRange,
                abstract: range instanceof AbstractRange,
                staticRange: range instanceof StaticRange,
                startSame: range.startContainer === text,
                startOffset: range.startOffset,
                endSame: range.endContainer === host,
                endOffset: range.endOffset,
                collapsed: range.collapsed,
                attrError,
                doctypeError,
                missingError
              });
            })()
            "#,
        )
        .expect("StaticRange constructor checks should evaluate");

    assert_eq!(
        result,
        r#"{"ctor":"function","abstract":true,"staticRange":true,"startSame":true,"startOffset":0,"endSame":true,"endOffset":3,"collapsed":false,"attrError":"InvalidNodeTypeError","doctypeError":"InvalidNodeTypeError","missingError":"TypeError"}"#
    );
}

#[test]
fn range_uses_native_record_storage_and_static_range_keeps_boundary_slots() {
    let mut vm = new_storage_test_vm("https://range-declared-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const accessorDescriptor = (prototype, name) => {
                const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                return [
                  name,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  descriptor?.enumerable,
                  typeof descriptor?.set,
                  descriptor?.configurable
                ].join(":");
              };
              const host = document.createElement("div");
              host.append("abc");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              const constructed = new Range();
              const created = document.createRange();
              const staticRange = new StaticRange({
                startContainer: text,
                startOffset: 1,
                endContainer: text,
                endOffset: 2
              });
              text.insertData(0, "z");
              return JSON.stringify({
                rangeTag: Object.prototype.toString.call(constructed),
                rangeCtor: constructed.constructor && constructed.constructor.name,
                rangeKeys: Object.keys(constructed).join(","),
                rangeOwnInternalSlotCount: Object.getOwnPropertyNames(constructed)
                  .filter((name) => name.startsWith("__moli")).length,
                rangeStartIsDocument: constructed.startContainer === document,
                rangeStartOffset: constructed.startOffset,
                rangeEndIsDocument: constructed.endContainer === document,
                rangeEndOffset: constructed.endOffset,
                rangeCollapsed: constructed.collapsed,
                rangeStartEnumerable: Object.prototype.propertyIsEnumerable.call(constructed, "startContainer"),
                createdTag: Object.prototype.toString.call(created),
                createdStartIsDocument: created.startContainer === document,
                createdOwnInternalSlotCount: Object.getOwnPropertyNames(created)
                  .filter((name) => name.startsWith("__moli")).length,
                staticTag: Object.prototype.toString.call(staticRange),
                staticCtor: staticRange.constructor && staticRange.constructor.name,
                staticAbstract: staticRange instanceof AbstractRange,
                staticKeys: Object.keys(staticRange).join(","),
                staticOwnInternalSlotCount: Object.getOwnPropertyNames(staticRange)
                  .filter((name) => name.startsWith("__moli")).length,
                staticStartSame: staticRange.startContainer === text,
                staticStartOffset: staticRange.startOffset,
                staticEndSame: staticRange.endContainer === text,
                staticEndOffset: staticRange.endOffset,
                staticCollapsed: staticRange.collapsed,
                staticStartEnumerable: Object.prototype.propertyIsEnumerable.call(staticRange, "startContainer"),
                abstractAccessors: [
                  accessorDescriptor(AbstractRange.prototype, "startContainer"),
                  accessorDescriptor(AbstractRange.prototype, "startOffset"),
                  accessorDescriptor(AbstractRange.prototype, "endContainer"),
                  accessorDescriptor(AbstractRange.prototype, "endOffset"),
                  accessorDescriptor(AbstractRange.prototype, "collapsed"),
                  accessorDescriptor(AbstractRange.prototype, "commonAncestorContainer")
                ]
              });
            })()
            "#,
        )
        .expect("Range native record storage probe should evaluate");

    assert_eq!(
        result,
        r#"{"rangeTag":"[object Range]","rangeCtor":"Range","rangeKeys":"","rangeOwnInternalSlotCount":0,"rangeStartIsDocument":true,"rangeStartOffset":0,"rangeEndIsDocument":true,"rangeEndOffset":0,"rangeCollapsed":true,"rangeStartEnumerable":false,"createdTag":"[object Range]","createdStartIsDocument":true,"createdOwnInternalSlotCount":0,"staticTag":"[object StaticRange]","staticCtor":"StaticRange","staticAbstract":true,"staticKeys":"","staticOwnInternalSlotCount":0,"staticStartSame":true,"staticStartOffset":1,"staticEndSame":true,"staticEndOffset":2,"staticCollapsed":false,"staticStartEnumerable":false,"abstractAccessors":["startContainer:function:get startContainer:0:true:undefined:true","startOffset:function:get startOffset:0:true:undefined:true","endContainer:function:get endContainer:0:true:undefined:true","endOffset:function:get endOffset:0:true:undefined:true","collapsed:function:get collapsed:0:true:undefined:true","commonAncestorContainer:function:get commonAncestorContainer:0:true:undefined:true"]}"#
    );
}

#[test]
fn selection_record_handle_ignores_legacy_slot_name_property() {
    let mut vm = new_storage_test_vm("https://selection-record-internal-field.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.body || document.documentElement || document;
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              container.appendChild(host);

              const selection = getSelection();
              selection.__moliSelectionRecordId = 0n;
              selection.setBaseAndExtent(text, 1, text, 3);
              const range = selection.getRangeAt(0);

              return [
                selection.__moliSelectionRecordId === 0n,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                range.startContainer === text,
                range.startOffset,
                range.endContainer === text,
                range.endOffset,
                selection.toString()
              ].join("|");
            })()
            "#,
        )
        .expect("Selection native record handle should ignore public legacy-slot-name spoofing");

    assert_eq!(result, "true|true|1|true|3|true|1|true|3|bc");
}

#[test]
fn document_create_range_declared_method_keeps_descriptor_and_behavior() {
    let mut vm = new_storage_test_vm("https://document-create-range-method.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(Document.prototype, "createRange");
              const range = document.createRange();
              return JSON.stringify({
                descriptor: [
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable
                ].join(":"),
                prototypeKeys: Object.keys(Document.prototype).includes("createRange"),
                ownOnDocument: Object.hasOwn(document, "createRange"),
                tag: Object.prototype.toString.call(range),
                instance: range instanceof Range,
                startIsDocument: range.startContainer === document,
                collapsed: range.collapsed
              });
            })()
            "#,
        )
        .expect("Document.createRange method probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptor":"true:function:createRange:0:true:true:true","prototypeKeys":true,"ownOnDocument":false,"tag":"[object Range]","instance":true,"startIsDocument":true,"collapsed":true}"#
    );
}

#[test]
fn range_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://range-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const methods = [
                ["setStart", 2],
                ["setEnd", 2],
                ["selectNodeContents", 1],
                ["cloneContents", 0],
                ["collapse", 0],
                ["selectNode", 1],
                ["setStartBefore", 1],
                ["setStartAfter", 1],
                ["setEndBefore", 1],
                ["setEndAfter", 1],
                ["cloneRange", 0],
                ["toString", 0],
                ["comparePoint", 2],
                ["isPointInRange", 2],
                ["intersectsNode", 1],
                ["compareBoundaryPoints", 2],
                ["insertNode", 1],
                ["createContextualFragment", 1],
                ["deleteContents", 0],
                ["extractContents", 0],
                ["surroundContents", 1],
                ["getBoundingClientRect", 0],
                ["getClientRects", 0],
                ["detach", 0]
              ];
              const range = document.createRange();
              const descriptors = methods.map(([name, length]) => {
                const descriptor = Object.getOwnPropertyDescriptor(Range.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable,
                  Object.hasOwn(range, name),
                  descriptor?.value === range[name],
                  descriptor?.value?.length === length
                ].join(":");
              });
              const enumerableMethods = Object.keys(Range.prototype)
                .filter((name) => methods.some(([method]) => method === name))
                .join(",");

              const host = document.createElement("div");
              host.append("abcdef");
              (document.body || document.documentElement || document).append(host);
              const text = host.firstChild;
              range.setStart(text, 1);
              range.setEnd(text, 4);
              const clone = range.cloneRange();
              const rect = range.getBoundingClientRect();
              const rects = range.getClientRects();
              const behavior = [
                range.toString(),
                clone.toString(),
                range.comparePoint(text, 2),
                range.isPointInRange(text, 2),
                range.intersectsNode(text),
                rect && rect.constructor && rect.constructor.name,
                rects.length
              ].join(":");
              range.collapse(true);
              range.detach();
              return JSON.stringify({
                descriptors,
                enumerableMethods,
                behavior,
                afterCollapse: [
                  range.collapsed,
                  range.startContainer === text,
                  range.startOffset,
                  range.endContainer === text,
                  range.endOffset
                ].join(":")
              });
            })()
            "#,
        )
        .expect("Range prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["setStart:true:function:setStart:2:true:true:true:false:true:true","setEnd:true:function:setEnd:2:true:true:true:false:true:true","selectNodeContents:true:function:selectNodeContents:1:true:true:true:false:true:true","cloneContents:true:function:cloneContents:0:true:true:true:false:true:true","collapse:true:function:collapse:0:true:true:true:false:true:true","selectNode:true:function:selectNode:1:true:true:true:false:true:true","setStartBefore:true:function:setStartBefore:1:true:true:true:false:true:true","setStartAfter:true:function:setStartAfter:1:true:true:true:false:true:true","setEndBefore:true:function:setEndBefore:1:true:true:true:false:true:true","setEndAfter:true:function:setEndAfter:1:true:true:true:false:true:true","cloneRange:true:function:cloneRange:0:true:true:true:false:true:true","toString:true:function:toString:0:true:true:true:false:true:true","comparePoint:true:function:comparePoint:2:true:true:true:false:true:true","isPointInRange:true:function:isPointInRange:2:true:true:true:false:true:true","intersectsNode:true:function:intersectsNode:1:true:true:true:false:true:true","compareBoundaryPoints:true:function:compareBoundaryPoints:2:true:true:true:false:true:true","insertNode:true:function:insertNode:1:true:true:true:false:true:true","createContextualFragment:true:function:createContextualFragment:1:true:true:true:false:true:true","deleteContents:true:function:deleteContents:0:true:true:true:false:true:true","extractContents:true:function:extractContents:0:true:true:true:false:true:true","surroundContents:true:function:surroundContents:1:true:true:true:false:true:true","getBoundingClientRect:true:function:getBoundingClientRect:0:true:true:true:false:true:true","getClientRects:true:function:getClientRects:0:true:true:true:false:true:true","detach:true:function:detach:0:true:true:true:false:true:true"],"enumerableMethods":"setStart,setEnd,selectNodeContents,cloneContents,collapse,selectNode,setStartBefore,setStartAfter,setEndBefore,setEndAfter,cloneRange,toString,comparePoint,isPointInRange,intersectsNode,compareBoundaryPoints,insertNode,createContextualFragment,deleteContents,extractContents,surroundContents,getBoundingClientRect,getClientRects,detach","behavior":"bcd:bcd:0:true:true:DOMRect:1","afterCollapse":"true:true:1:true:1"}"#
    );
}

#[test]
fn range_clone_contents_returns_fragment_for_ancestor_to_descendant_boundary() {
    let mut vm = new_storage_test_vm("https://range-clone-ancestor-boundary.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const line = document.createElement("div");
              line.className = "ace-line";
              line.innerHTML = '<span data-marker="1">Alpha</span><span data-hit="1">Beta</span>';
              (document.body || document.documentElement || document).append(line);

              const range = document.createRange();
              range.setStart(line, 0);
              range.setEnd(line.querySelector("[data-hit]").firstChild, 2);
              const fragment = range.cloneContents();

              return JSON.stringify({
                tag: Object.prototype.toString.call(fragment),
                instance: fragment instanceof DocumentFragment,
                hasQuerySelectorAll: typeof fragment.querySelectorAll === "function",
                text: fragment.textContent,
                hitCount: fragment.querySelectorAll("[data-hit]").length,
                hitText: fragment.querySelector("[data-hit]").textContent,
                originalText: line.textContent
              });
            })()
            "#,
        )
        .expect("ancestor-to-descendant Range.cloneContents probe should evaluate");

    assert_eq!(
        result,
        r#"{"tag":"[object DocumentFragment]","instance":true,"hasQuerySelectorAll":true,"text":"AlphaBe","hitCount":1,"hitText":"Be","originalText":"AlphaBeta"}"#
    );
}

#[test]
fn range_clone_contents_preserves_partial_boundary_structure() {
    let mut vm = new_storage_test_vm("https://range-clone-partial-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("main");
                document.appendChild(parent);
              }

              const ancestorEnd = document.createElement("div");
              ancestorEnd.innerHTML =
                '<span data-left="1">Alpha</span><span data-mid="1">Beta</span><span data-right="1">Gamma</span>';
              parent.append(ancestorEnd);
              const ancestorRange = document.createRange();
              ancestorRange.setStart(ancestorEnd.querySelector("[data-left]").firstChild, 2);
              ancestorRange.setEnd(ancestorEnd, 2);
              const ancestorFragment = ancestorRange.cloneContents();

              const cross = document.createElement("div");
              cross.innerHTML =
                '<section data-left-branch="1"><b>Alpha</b><i>Aft</i></section>' +
                '<p data-middle="1">Middle</p>' +
                '<section data-right-branch="1"><b>Omega</b><i>Tail</i></section>';
              parent.append(cross);
              const crossRange = document.createRange();
              crossRange.setStart(cross.querySelector("[data-left-branch] b").firstChild, 2);
              crossRange.setEnd(cross.querySelector("[data-right-branch] b").firstChild, 2);
              const crossFragment = crossRange.cloneContents();

              const childOffset = document.createElement("div");
              childOffset.innerHTML = '<a>One</a><b>Two</b><c>Three</c>';
              parent.append(childOffset);
              const childOffsetRange = document.createRange();
              childOffsetRange.setStart(childOffset, 1);
              childOffsetRange.setEnd(childOffset, 2);
              const childOffsetFragment = childOffsetRange.cloneContents();

              return JSON.stringify({
                ancestorText: ancestorFragment.textContent,
                ancestorLeftText: ancestorFragment.querySelector("[data-left]").textContent,
                ancestorMidCount: ancestorFragment.querySelectorAll("[data-mid]").length,
                ancestorRightCount: ancestorFragment.querySelectorAll("[data-right]").length,
                ancestorOriginal: ancestorEnd.textContent,
                crossText: crossFragment.textContent,
                crossLeftText: crossFragment.querySelector("[data-left-branch]").textContent,
                crossMiddleCount: crossFragment.querySelectorAll("[data-middle]").length,
                crossRightText: crossFragment.querySelector("[data-right-branch]").textContent,
                crossOriginal: cross.textContent,
                childOffsetText: childOffsetFragment.textContent,
                childOffsetChildNames: Array.from(childOffsetFragment.childNodes)
                  .map((node) => node.localName)
                  .join(",")
              });
            })()
            "#,
        )
        .expect("partial-boundary Range.cloneContents probes should evaluate");

    assert_eq!(
        result,
        r#"{"ancestorText":"phaBeta","ancestorLeftText":"pha","ancestorMidCount":1,"ancestorRightCount":0,"ancestorOriginal":"AlphaBetaGamma","crossText":"phaAftMiddleOm","crossLeftText":"phaAft","crossMiddleCount":1,"crossRightText":"Om","crossOriginal":"AlphaAftMiddleOmegaTail","childOffsetText":"Two","childOffsetChildNames":"b"}"#
    );
}

#[test]
fn range_extract_delete_contents_preserve_partial_boundary_structure() {
    let mut vm = new_storage_test_vm("https://range-extract-delete-partial-boundaries.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("main");
                document.appendChild(parent);
              }

              const ancestorEnd = document.createElement("div");
              ancestorEnd.innerHTML =
                '<span data-left="1">Alpha</span><span data-mid="1">Beta</span><span data-right="1">Gamma</span>';
              parent.append(ancestorEnd);
              const ancestorRange = document.createRange();
              ancestorRange.setStart(ancestorEnd.querySelector("[data-left]").firstChild, 2);
              ancestorRange.setEnd(ancestorEnd, 2);
              const ancestorFragment = ancestorRange.extractContents();

              const cross = document.createElement("div");
              cross.innerHTML =
                '<section data-left-branch="1"><b>Alpha</b><i>Aft</i></section>' +
                '<p data-middle="1">Middle</p>' +
                '<section data-right-branch="1"><b>Omega</b><i>Tail</i></section>';
              parent.append(cross);
              const crossRange = document.createRange();
              crossRange.setStart(cross.querySelector("[data-left-branch] b").firstChild, 2);
              crossRange.setEnd(cross.querySelector("[data-right-branch] b").firstChild, 2);
              const crossFragment = crossRange.extractContents();

              const childOffset = document.createElement("div");
              childOffset.innerHTML = '<a>One</a><b>Two</b><c>Three</c>';
              parent.append(childOffset);
              const childOffsetRange = document.createRange();
              childOffsetRange.setStart(childOffset, 1);
              childOffsetRange.setEnd(childOffset, 2);
              const childOffsetFragment = childOffsetRange.extractContents();

              const deletion = document.createElement("div");
              deletion.innerHTML = '<span data-left="1">Alpha</span><span data-hit="1">Beta</span>';
              parent.append(deletion);
              const deleteRange = document.createRange();
              deleteRange.setStart(deletion, 0);
              deleteRange.setEnd(deletion.querySelector("[data-hit]").firstChild, 2);
              const deleteReturn = deleteRange.deleteContents();

              return JSON.stringify({
                ancestorFragmentText: ancestorFragment.textContent,
                ancestorFragmentLeftText: ancestorFragment.querySelector("[data-left]").textContent,
                ancestorFragmentMidCount: ancestorFragment.querySelectorAll("[data-mid]").length,
                ancestorOriginal: ancestorEnd.textContent,
                ancestorCollapsed: [
                  ancestorRange.collapsed,
                  ancestorRange.startContainer === ancestorEnd,
                  ancestorRange.startOffset
                ].join(":"),
                crossFragmentText: crossFragment.textContent,
                crossLeftText: crossFragment.querySelector("[data-left-branch]").textContent,
                crossMiddleCount: crossFragment.querySelectorAll("[data-middle]").length,
                crossRightText: crossFragment.querySelector("[data-right-branch]").textContent,
                crossOriginal: cross.textContent,
                crossCollapsed: [
                  crossRange.collapsed,
                  crossRange.startContainer === cross,
                  crossRange.startOffset
                ].join(":"),
                childOffsetFragmentText: childOffsetFragment.textContent,
                childOffsetOriginal: childOffset.textContent,
                childOffsetCollapsed: [
                  childOffsetRange.collapsed,
                  childOffsetRange.startContainer === childOffset,
                  childOffsetRange.startOffset
                ].join(":"),
                deleteReturn: String(deleteReturn),
                deleteOriginal: deletion.textContent,
                deleteHitText: deletion.querySelector("[data-hit]").textContent,
                deleteCollapsed: [
                  deleteRange.collapsed,
                  deleteRange.startContainer === deletion,
                  deleteRange.startOffset
                ].join(":")
              });
            })()
            "#,
        )
        .expect("partial-boundary Range.extractContents/deleteContents probes should evaluate");

    assert_eq!(
        result,
        r#"{"ancestorFragmentText":"phaBeta","ancestorFragmentLeftText":"pha","ancestorFragmentMidCount":1,"ancestorOriginal":"AlGamma","ancestorCollapsed":"true:true:1","crossFragmentText":"phaAftMiddleOm","crossLeftText":"phaAft","crossMiddleCount":1,"crossRightText":"Om","crossOriginal":"AlegaTail","crossCollapsed":"true:true:1","childOffsetFragmentText":"Two","childOffsetOriginal":"OneThree","childOffsetCollapsed":"true:true:1","deleteReturn":"undefined","deleteOriginal":"ta","deleteHitText":"ta","deleteCollapsed":"true:true:0"}"#
    );
}

#[test]
fn range_contents_handles_cdata_pi_foreign_text_and_doctype_edges() {
    let mut vm = new_storage_test_vm("https://range-contents-character-data-edges.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const thrownName = (callback) => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error && error.name}:${error && error.code}`;
                }
              };

              const xmlDoc = document.implementation.createDocument(null, "root");
              const cdataForClone = xmlDoc.createCDATASection("1234");
              xmlDoc.documentElement.appendChild(cdataForClone);
              const cdataCloneRange = xmlDoc.createRange();
              cdataCloneRange.setStart(cdataForClone, 1);
              cdataCloneRange.setEnd(cdataForClone, 3);
              const cdataClone = cdataCloneRange.cloneContents().firstChild;

              const cdataForDelete = xmlDoc.createCDATASection("5678");
              xmlDoc.documentElement.appendChild(cdataForDelete);
              const cdataDeleteRange = xmlDoc.createRange();
              cdataDeleteRange.setStart(cdataForDelete, 1);
              cdataDeleteRange.setEnd(cdataForDelete, 3);
              cdataDeleteRange.deleteContents();

              const piForClone = xmlDoc.createProcessingInstruction("somePI", "abcdef");
              xmlDoc.documentElement.appendChild(piForClone);
              const piCloneRange = xmlDoc.createRange();
              piCloneRange.setStart(piForClone, 1);
              piCloneRange.setEnd(piForClone, 4);
              const piClone = piCloneRange.cloneContents().firstChild;

              const piForExtract = xmlDoc.createProcessingInstruction("otherPI", "uvwxyz");
              xmlDoc.documentElement.appendChild(piForExtract);
              const piExtractRange = xmlDoc.createRange();
              piExtractRange.setStart(piForExtract, 2);
              piExtractRange.setEnd(piForExtract, 5);
              const piExtract = piExtractRange.extractContents().firstChild;

              const foreignDoc = document.implementation.createHTMLDocument("");
              const foreignText = foreignDoc.createTextNode("Efghijkl");
              foreignDoc.body.appendChild(foreignText);
              const foreignRange = foreignDoc.createRange();
              foreignRange.setStart(foreignText, 2);
              foreignRange.setEnd(foreignText, 8);
              const foreignFragment = foreignRange.extractContents();

              const doctypeDoc = document.implementation.createHTMLDocument("");
              if (!doctypeDoc.doctype) {
                doctypeDoc.insertBefore(
                  document.implementation.createDocumentType("html", "", ""),
                  doctypeDoc.firstChild
                );
              }
              const doctypeCloneRange = doctypeDoc.createRange();
              doctypeCloneRange.setStart(doctypeDoc, 0);
              doctypeCloneRange.setEnd(doctypeDoc, 1);
              const doctypeExtractRange = doctypeDoc.createRange();
              doctypeExtractRange.setStart(doctypeDoc, 0);
              doctypeExtractRange.setEnd(doctypeDoc, 1);

              return JSON.stringify({
                cdataClone: [
                  cdataClone.nodeType,
                  cdataClone.nodeName,
                  cdataClone.data,
                  cdataForClone.data
                ],
                cdataDelete: [
                  cdataForDelete.data,
                  cdataDeleteRange.collapsed,
                  cdataDeleteRange.startContainer === cdataForDelete,
                  cdataDeleteRange.startOffset
                ],
                piClone: [
                  piClone.nodeType,
                  piClone.target,
                  piClone.data,
                  piForClone.data
                ],
                piExtract: [
                  piExtract.nodeType,
                  piExtract.target,
                  piExtract.data,
                  piForExtract.data,
                  piExtractRange.collapsed,
                  piExtractRange.startContainer === piForExtract,
                  piExtractRange.startOffset
                ],
                foreignText: [
                  foreignFragment.firstChild.data,
                  foreignText.data,
                  foreignRange.collapsed,
                  foreignRange.startContainer === foreignText,
                  foreignRange.startOffset
                ],
                doctype: [
                  thrownName(() => doctypeCloneRange.cloneContents()),
                  thrownName(() => doctypeExtractRange.extractContents()),
                  doctypeDoc.doctype.parentNode === doctypeDoc
                ]
              });
            })()
            "#,
        )
        .expect("Range contents character data edge probe should evaluate");

    assert_eq!(
        result,
        r##"{"cdataClone":[4,"#cdata-section","23","1234"],"cdataDelete":["58",true,true,1],"piClone":[7,"somePI","bcd","abcdef"],"piExtract":[7,"otherPI","wxy","uvz",true,true,2],"foreignText":["ghijkl","Ef",true,true,2],"doctype":["HierarchyRequestError:3","HierarchyRequestError:3",true]}"##
    );
}

#[test]
fn selection_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://selection-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const methods = [
                ["getRangeAt", 1],
                ["addRange", 1],
                ["removeRange", 1],
                ["removeAllRanges", 0],
                ["empty", 0],
                ["collapse", 1],
                ["setPosition", 1],
                ["collapseToStart", 0],
                ["collapseToEnd", 0],
                ["extend", 1],
                ["selectAllChildren", 1],
                ["setBaseAndExtent", 4],
                ["containsNode", 1],
                ["deleteFromDocument", 0],
                ["modify", 0],
                ["toString", 0]
              ];
              const accessors = [
                "anchorNode",
                "anchorOffset",
                "focusNode",
                "focusOffset",
                "isCollapsed",
                "rangeCount",
                "type",
                "direction"
              ];
              const stringify = (value) =>
                value === undefined ? "undefined" : String(value);
              const selection = getSelection();
              const descriptors = methods.map(([name, length]) => {
                const descriptor = Object.getOwnPropertyDescriptor(Selection.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.value,
                  descriptor?.value?.name,
                  descriptor?.value?.length,
                  descriptor?.enumerable,
                  descriptor?.writable,
                  descriptor?.configurable,
                  Object.hasOwn(selection, name),
                  descriptor?.value === selection[name],
                  descriptor?.value?.length === length
                ].join(":");
              });
              const accessorDescriptors = accessors.map((name) => {
                const descriptor = Object.getOwnPropertyDescriptor(Selection.prototype, name);
                return [
                  name,
                  !!descriptor,
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.enumerable,
                  descriptor?.configurable,
                  Object.hasOwn(selection, name)
                ].join(":");
              });
              const enumerableMethods = Object.keys(Selection.prototype)
                .filter((name) => methods.some(([method]) => method === name))
                .join(",");
              const enumerableAccessors = Object.keys(Selection.prototype)
                .filter((name) => accessors.includes(name))
                .join(",");

              const host = document.createElement("div");
              const text = document.createTextNode("abcdef");
              host.appendChild(text);
              let parent = document.body || document.documentElement;
              if (!parent) {
                parent = document.createElement("html");
                document.appendChild(parent);
              }
              parent.appendChild(host);
              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 4);
              selection.removeAllRanges();
              selection.addRange(range);
              const ownSlots = Object.getOwnPropertyNames(selection)
                .filter((name) => name.startsWith("__moliSelection"))
                .sort();
              for (const slot of [
                "__moliSelectionRange",
                "__moliSelectionAnchorNode",
                "__moliSelectionAnchorOffset",
                "__moliSelectionFocusNode",
                "__moliSelectionFocusOffset",
                "__moliSelectionDirection"
              ]) {
                Selection.prototype[slot] = "prototype-spoof";
                selection[slot] = "own-spoof";
              }
              const behavior = [
                selection.getRangeAt(0) === range,
                selection.toString(),
                selection.containsNode(text, true),
                selection.rangeCount
              ].join(":");
              const attributeValues = [
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                selection.isCollapsed,
                selection.rangeCount,
                selection.type,
                selection.direction
              ].join(":");
              const fake = Object.create(Selection.prototype);
              const fakeValues = [
                fake.anchorNode,
                fake.anchorOffset,
                fake.focusNode,
                fake.focusOffset,
                fake.isCollapsed,
                fake.rangeCount,
                fake.type,
                fake.direction
              ].map(stringify).join(":");
              selection.empty();
              return JSON.stringify({
                descriptors,
                accessorDescriptors,
                enumerableMethods,
                enumerableAccessors,
                behavior,
                attributeValues,
                fakeValues,
                ownSlots,
                afterEmpty: [
                  selection.rangeCount,
                  selection.anchorNode === null,
                  selection.focusNode === null
                ].join(":")
              });
            })()
            "#,
        )
        .expect("Selection prototype method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["getRangeAt:true:function:getRangeAt:1:true:true:true:false:true:true","addRange:true:function:addRange:1:true:true:true:false:true:true","removeRange:true:function:removeRange:1:true:true:true:false:true:true","removeAllRanges:true:function:removeAllRanges:0:true:true:true:false:true:true","empty:true:function:empty:0:true:true:true:false:true:true","collapse:true:function:collapse:1:true:true:true:false:true:true","setPosition:true:function:setPosition:1:true:true:true:false:true:true","collapseToStart:true:function:collapseToStart:0:true:true:true:false:true:true","collapseToEnd:true:function:collapseToEnd:0:true:true:true:false:true:true","extend:true:function:extend:1:true:true:true:false:true:true","selectAllChildren:true:function:selectAllChildren:1:true:true:true:false:true:true","setBaseAndExtent:true:function:setBaseAndExtent:4:true:true:true:false:true:true","containsNode:true:function:containsNode:1:true:true:true:false:true:true","deleteFromDocument:true:function:deleteFromDocument:0:true:true:true:false:true:true","modify:true:function:modify:0:true:true:true:false:true:true","toString:true:function:toString:0:true:true:true:false:true:true"],"accessorDescriptors":["anchorNode:true:function:get anchorNode:0:undefined:true:true:false","anchorOffset:true:function:get anchorOffset:0:undefined:true:true:false","focusNode:true:function:get focusNode:0:undefined:true:true:false","focusOffset:true:function:get focusOffset:0:undefined:true:true:false","isCollapsed:true:function:get isCollapsed:0:undefined:true:true:false","rangeCount:true:function:get rangeCount:0:undefined:true:true:false","type:true:function:get type:0:undefined:true:true:false","direction:true:function:get direction:0:undefined:true:true:false"],"enumerableMethods":"getRangeAt,addRange,removeRange,removeAllRanges,empty,collapse,setPosition,collapseToStart,collapseToEnd,extend,selectAllChildren,setBaseAndExtent,containsNode,deleteFromDocument,modify,toString","enumerableAccessors":"anchorNode,anchorOffset,focusNode,focusOffset,isCollapsed,rangeCount,type,direction","behavior":"true:bcd:true:1","attributeValues":"true:1:true:4:false:1:Range:forward","fakeValues":"null:0:null:0:true:0:None:undefined","ownSlots":[],"afterEmpty":"0:true:true"}"#
    );
}

#[test]
fn selection_delete_from_document_uses_utf16_range_offsets() {
    let mut vm = new_storage_test_vm("https://selection-delete-utf16.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("a\uD83D\uDE00b");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);

              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 3);
              const selection = getSelection();
              selection.removeAllRanges();
              selection.addRange(range);
              selection.deleteFromDocument();

              const selectedRange = selection.getRangeAt(0);
              return [
                text.data,
                text.data.length,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset,
                selectedRange.startContainer === text,
                selectedRange.startOffset,
                selectedRange.collapsed
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.deleteFromDocument should use UTF-16 offsets");

    assert_eq!(result, "ab|2|true|1|true|1|true|1|true");
}

#[test]
fn child_window_range_constructors_match_child_document_ranges() {
    let mut vm = new_storage_test_vm("https://child-window-range-constructors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childWindow = iframe.contentWindow;
              const childDocument = iframe.contentDocument;

              const range = childDocument.createRange();
              range.selectNodeContents(childDocument.body);
              const constructedRange = new childWindow.Range();
              const staticRange = new childWindow.StaticRange({
                startContainer: childDocument,
                startOffset: 0,
                endContainer: childDocument,
                endOffset: childDocument.childNodes.length
              });

              return [
                typeof childWindow.AbstractRange,
                typeof childWindow.Range,
                typeof childWindow.StaticRange,
                childWindow.Range.length,
                childWindow.StaticRange.length,
                range instanceof childWindow.Range,
                range instanceof childWindow.AbstractRange,
                constructedRange.startContainer === childDocument,
                constructedRange.endContainer === childDocument,
                constructedRange.collapsed,
                staticRange instanceof childWindow.StaticRange,
                staticRange instanceof childWindow.AbstractRange,
                staticRange.startContainer === childDocument,
                staticRange.endOffset === childDocument.childNodes.length
              ].join("|");
            })()
            "#,
        )
        .expect("child window Range constructors should evaluate");

    assert_eq!(
        result,
        "function|function|function|0|1|true|true|true|true|true|true|true|true|true"
    );
}

#[test]
fn detached_document_remove_child_updates_live_range_boundaries() {
    let mut vm = new_storage_test_vm("https://detached-document-range-remove.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const doc = document.implementation.createHTMLDocument("");
              const documentRange = doc.createRange();
              documentRange.setStart(doc, 0);
              documentRange.setEnd(doc, doc.childNodes.length);
              doc.removeChild(doc.documentElement);
              const expectedDocumentEnd = doc.childNodes.length;

              const descendantDoc = document.implementation.createHTMLDocument("");
              const child = descendantDoc.createElement("span");
              const text = descendantDoc.createTextNode("x");
              child.appendChild(text);
              descendantDoc.body.appendChild(child);
              const descendantRange = descendantDoc.createRange();
              descendantRange.setStart(text, 0);
              descendantRange.setEnd(child, 1);
              descendantDoc.body.removeChild(child);

              return [
                documentRange.startContainer === doc,
                documentRange.startOffset,
                documentRange.endContainer === doc,
                documentRange.endOffset,
                expectedDocumentEnd,
                descendantRange.startContainer === descendantDoc.body,
                descendantRange.startOffset,
                descendantRange.endContainer === descendantDoc.body,
                descendantRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached document removeChild should update live ranges");

    assert_eq!(result, "true|0|true|1|1|true|0|true|0");
}
#[test]
fn detached_document_adopted_live_container_keeps_range_mutation_updates() {
    let mut vm = new_storage_test_vm("https://range-adopt-live-container.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              function createRangeWithUnparentedContainerOfSingleElement() {
                const range = document.createRange();
                const container = document.createElement("container");
                const element = document.createElement("element");
                container.appendChild(element);
                range.selectNode(element);
                return range;
              }
              function nestRangeInOuterContainer(range) {
                range.startContainer.ownerDocument.createElement("outer").appendChild(range.startContainer);
              }
              function moveNodeToNewlyCreatedDocumentWithAppendChild(node) {
                document.implementation.createDocument(null, null).appendChild(node);
              }

              const direct = createRangeWithUnparentedContainerOfSingleElement();
              let directError = "";
              try { direct.startContainer.removeChild(direct.startContainer.firstChild); } catch (error) { directError = error.name; }

              const parentedMoved = createRangeWithUnparentedContainerOfSingleElement();
              nestRangeInOuterContainer(parentedMoved);
              let parentedMovedError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(parentedMoved.startContainer); } catch (error) { parentedMovedError = error.name; }

              const parentlessMoved = createRangeWithUnparentedContainerOfSingleElement();
              let parentlessMovedError = "";
              let parentlessRemoveError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(parentlessMoved.startContainer); } catch (error) { parentlessMovedError = error.name; }
              try { parentlessMoved.startContainer.removeChild(parentlessMoved.startContainer.firstChild); } catch (error) { parentlessRemoveError = error.name; }

              const outerMoved = createRangeWithUnparentedContainerOfSingleElement();
              nestRangeInOuterContainer(outerMoved);
              let outerMovedError = "";
              let outerRemoveError = "";
              try { moveNodeToNewlyCreatedDocumentWithAppendChild(outerMoved.startContainer.parentNode); } catch (error) { outerMovedError = error.name; }
              try { outerMoved.startContainer.removeChild(outerMoved.startContainer.firstChild); } catch (error) { outerRemoveError = error.name; }

              const errors = [
                directError,
                parentedMovedError,
                parentlessMovedError,
                parentlessRemoveError,
                outerMovedError,
                outerRemoveError
              ].filter(Boolean).join(",");

              return [
                errors,
                direct.endOffset,
                parentedMoved.endOffset,
                parentlessMoved.endOffset,
                outerMoved.endOffset,
                parentlessMoved.endContainer === parentlessMoved.startContainer,
                outerMoved.endContainer === outerMoved.startContainer
              ].join("|");
            })()
            "#,
        )
        .expect("adopted live container range mutation checks should evaluate");

    assert_eq!(result, "|0|0|0|0|true|true");
}
#[test]
fn removing_shadow_host_keeps_shadow_range_boundaries() {
    let mut vm = new_storage_test_vm("https://range-shadow-host-remove.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<div id="in-shadow">ABC</div>';
              const shadowChild = root.firstChild;
              (document.body || document.documentElement || document).appendChild(host);
              const hostRange = document.createRange();
              hostRange.setStart(shadowChild, 1);
              host.remove();

              const wrapper = document.createElement("div");
              const nestedHost = document.createElement("div");
              const nestedRoot = nestedHost.attachShadow({ mode: "open" });
              nestedRoot.innerHTML = '<div id="in-shadow">ABC</div>';
              const nestedShadowChild = nestedRoot.firstChild;
              wrapper.appendChild(nestedHost);
              (document.body || document.documentElement || document).appendChild(wrapper);
              const wrapperRange = document.createRange();
              wrapperRange.setStart(nestedShadowChild, 1);
              wrapper.remove();

              return [
                hostRange.startContainer === shadowChild,
                hostRange.startOffset,
                wrapperRange.startContainer === nestedShadowChild,
                wrapperRange.startOffset
              ].join("|");
            })()
            "#,
        )
        .expect("shadow host removal range checks should evaluate");

    assert_eq!(result, "true|1|true|1");
}
#[test]
fn pre_insert_updates_live_ranges_for_moved_nodes() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-move.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createElement("a");
              const moved = document.createElement("b");
              const last = document.createElement("c");
              host.append(first, moved, last);
              (document.body || document.documentElement || document).appendChild(host);

              const movedRange = document.createRange();
              movedRange.setStart(moved, 0);
              movedRange.setEnd(host, host.childNodes.length);
              host.appendChild(moved);

              const detachedDoc = document.implementation.createHTMLDocument("");
              const parent = detachedDoc.createElement("div");
              const left = detachedDoc.createElement("l");
              const right = detachedDoc.createElement("r");
              parent.append(left, right);
              detachedDoc.body.appendChild(parent);
              const detachedRange = detachedDoc.createRange();
              detachedRange.setStart(parent, 1);
              detachedRange.setEnd(parent, 2);
              parent.insertBefore(detachedDoc.createElement("x"), left);

              return [
                Array.from(host.childNodes).map(node => node.localName).join(","),
                movedRange.startContainer === host,
                movedRange.startOffset,
                movedRange.endContainer === host,
                movedRange.endOffset,
                Array.from(parent.childNodes).map(node => node.localName).join(","),
                detachedRange.startContainer === parent,
                detachedRange.startOffset,
                detachedRange.endContainer === parent,
                detachedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("pre-insert live range checks should evaluate");

    assert_eq!(result, "a,c,b|true|1|true|2|x,l,r|true|2|true|3");
}
#[test]
fn detached_document_accepts_live_node_insert_and_updates_range() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-live-to-detached.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const detachedDoc = document.implementation.createHTMLDocument("");
              const detachedRange = detachedDoc.createRange();
              detachedRange.setStart(detachedDoc, 0);
              detachedRange.setEnd(detachedDoc, detachedDoc.childNodes.length);
              const liveComment = document.createComment("live");
              let detachedInsert = "no-throw";
              try {
                detachedDoc.insertBefore(liveComment, detachedDoc.documentElement);
              } catch (error) {
                detachedInsert = `${error.name}:${error.code}`;
              }

              return [
                detachedInsert,
                detachedDoc.childNodes[1] === liveComment,
                liveComment.parentNode === detachedDoc,
                liveComment.ownerDocument === detachedDoc,
                detachedRange.startContainer === detachedDoc,
                detachedRange.startOffset,
                detachedRange.endContainer === detachedDoc,
                detachedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("live node to detached document insert should evaluate");

    assert_eq!(result, "no-throw|true|true|true|true|0|true|3");
}
#[test]
fn live_document_adopts_detached_node_insert_and_updates_range() {
    let mut vm = new_storage_test_vm("https://range-pre-insert-detached-to-live.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createTextNode("first");
              host.appendChild(first);
              (document.body || document.documentElement || document).appendChild(host);

              const xmlDoc = document.implementation.createDocument(null, null, null);
              const xmlElement = xmlDoc.createElement("x");
              const xmlText = xmlDoc.createTextNode("foreign");
              xmlElement.appendChild(xmlText);
              xmlDoc.appendChild(xmlElement);

              const range = document.createRange();
              range.setStart(host, 0);
              range.setEnd(host, 1);
              let thrown = "no-throw";
              try {
                host.insertBefore(xmlText, first);
              } catch (error) {
                thrown = `${error.name}:${error.code}`;
              }

              return [
                thrown,
                xmlText.parentNode === xmlElement,
                host.firstChild === first,
                range.startContainer === host,
                range.startOffset,
                range.endContainer === host,
                range.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("detached node to live document insert should evaluate");

    assert_eq!(result, "no-throw|false|false|true|0|true|2");
}
#[test]
fn replace_child_updates_live_ranges_in_remove_then_insert_order() {
    let mut vm = new_storage_test_vm("https://range-replace-child-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const first = document.createElement("a");
              const second = document.createElement("b");
              const oldText = document.createTextNode("old");
              first.appendChild(oldText);
              host.append(first, second);
              (document.body || document.documentElement || document).appendChild(host);

              const sameRange = document.createRange();
              sameRange.setStart(first, 0);
              sameRange.setEnd(first, 1);
              host.replaceChild(first, first);

              const movingHost = document.createElement("div");
              const movingOld = document.createTextNode("old");
              const movingNew = document.createElement("n");
              const spare = document.createElement("s");
              movingHost.append(movingOld, spare);
              const staging = document.createElement("div");
              staging.appendChild(movingNew);
              (document.body || document.documentElement || document).appendChild(staging);
              (document.body || document.documentElement || document).appendChild(movingHost);
              const movingRange = document.createRange();
              movingRange.setStart(movingHost, 0);
              movingRange.setEnd(movingHost, 1);
              movingHost.replaceChild(movingNew, movingOld);

              const xmlDoc = document.implementation.createDocument(null, null, null);
              const xmlElement = xmlDoc.createElement("root");
              const xmlText = xmlDoc.createTextNode("xml");
              xmlElement.appendChild(xmlText);
              xmlDoc.appendChild(xmlElement);
              const foreignTextHost = document.createElement("p");
              foreignTextHost.appendChild(document.createTextNode("old"));
              (document.body || document.documentElement || document).appendChild(foreignTextHost);
              const foreignTextRange = document.createRange();
              foreignTextRange.setStart(foreignTextHost, 0);
              foreignTextRange.setEnd(foreignTextHost, 1);
              foreignTextHost.replaceChild(xmlText, foreignTextHost.firstChild);

              const foreignDoc = document.implementation.createHTMLDocument("");
              const invalidHost = document.createElement("p");
              invalidHost.appendChild(document.createTextNode("old"));
              (document.body || document.documentElement || document).appendChild(invalidHost);
              const invalidRange = document.createRange();
              invalidRange.setStart(invalidHost, 0);
              invalidRange.setEnd(invalidHost, 1);
              let invalidThrown = "no-throw";
              try {
                invalidHost.replaceChild(foreignDoc, invalidHost.firstChild);
              } catch (error) {
                invalidThrown = error.name;
              }

              return [
                sameRange.startContainer === host,
                sameRange.startOffset,
                sameRange.endContainer === host,
                sameRange.endOffset,
                movingHost.firstChild === movingNew,
                movingOld.parentNode === null,
                movingNew.parentNode === movingHost,
                movingRange.startContainer === movingHost,
                movingRange.startOffset,
                movingRange.endContainer === movingHost,
                movingRange.endOffset,
                foreignTextHost.firstChild.data,
                foreignTextHost.childNodes.length,
                foreignTextRange.startContainer === foreignTextHost,
                foreignTextRange.startOffset,
                foreignTextRange.endContainer === foreignTextHost,
                foreignTextRange.endOffset,
                invalidThrown,
                invalidHost.firstChild.nodeValue,
                invalidRange.startContainer === invalidHost,
                invalidRange.startOffset,
                invalidRange.endContainer === invalidHost,
                invalidRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("replaceChild live range order should evaluate");

    assert_eq!(
        result,
        "true|0|true|0|true|true|true|true|0|true|0|xml|1|true|0|true|0|HierarchyRequestError|old|true|0|true|1"
    );
}
#[test]
fn element_append_child_rejects_document_type_without_mutating_selection_range() {
    let mut vm = new_parsed_test_vm(
        "https://range-pre-insert-doctype.test/",
        "<!doctype html><html><body></body></html>",
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abc");
              host.appendChild(text);
              document.body.appendChild(host);
              const doctype = document.doctype;
              const range = document.createRange();
              range.setStart(host, 0);
              range.setEnd(host, 1);
              getSelection().removeAllRanges();
              getSelection().addRange(range);
              const selectedRange = getSelection().getRangeAt(0);
              let thrown = "no";
              try {
                host.appendChild(doctype);
              } catch (error) {
                thrown = `${error.name}:${error.code}`;
              }
              return [
                thrown,
                document.doctype === doctype,
                doctype.parentNode === document,
                selectedRange.startContainer === host,
                selectedRange.startOffset,
                selectedRange.endContainer === host,
                selectedRange.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("doctype insertion rejection should evaluate");

    assert_eq!(result, "HierarchyRequestError:3|true|true|true|0|true|1");
}
#[tokio::test]
async fn selectionchange_is_queued_and_coalesced_per_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-coalesce.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              globalThis.__selectionChangeLog = [];
              globalThis.__selectionChangeCount = 0;
              document.addEventListener("selectionchange", () => {
                globalThis.__selectionChangeCount += 1;
                globalThis.__selectionChangeLog.push(
                  `event:${globalThis.__selectionChangeCount}:${selection.anchorOffset}:${selection.focusOffset}`
                );
              });
              selection.collapse(text, 1);
              globalThis.__selectionChangeLog.push(`after-collapse:${globalThis.__selectionChangeCount}`);
              selection.extend(text, 2);
              globalThis.__selectionChangeLog.push(`after-extend:${globalThis.__selectionChangeCount}`);
              return `${globalThis.__selectionChangeLog.join("|")}|count:${globalThis.__selectionChangeCount}`;
            })()
            "#,
        )
        .expect("selectionchange setup should evaluate");

    assert_eq!(
        result, "after-collapse:0|after-extend:0|count:0",
        "selectionchange must not fire synchronously"
    );

    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionChangeLog.join('|')}|count:${globalThis.__selectionChangeCount}`")
            .expect("selectionchange task result should evaluate"),
        "after-collapse:0|after-extend:0|event:1:1:2|count:1"
    );
}
#[tokio::test]
async fn storage_mutations_queue_events_to_child_window_body_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    parent.__storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      url: event.url,
      storageArea: event.storageArea === localStorage,
      instance: event instanceof StorageEvent,
      tag: Object.prototype.toString.call(event)
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage child event setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval("localStorage.setItem('k', 'v'); __storageEvents.length")
            .expect("storage mutation should evaluate"),
        "0",
        "storage events must be queued instead of dispatched synchronously"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance queued storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("storage event result should evaluate"),
        r#"[{"key":"k","oldValue":null,"newValue":"v","url":"https://storage-events.test/page","storageArea":true,"instance":true,"tag":"[object StorageEvent]"}]"#
    );

    vm.eval("localStorage.setItem('k', 'v')")
        .expect("same-value storage mutation should evaluate");
    assert!(
        !vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should observe no same-value storage event")
    );
    assert_eq!(
        vm.eval("__storageEvents.length")
            .expect("same-value storage event count should evaluate"),
        "1"
    );
}

#[tokio::test]
async fn queued_storage_event_init_object_ignores_object_prototype_setters() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-init-data-property.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  globalThis.__storageInitSetterHits = [];
  globalThis.__captureStorageInitSetters = false;
  for (const name of ["key", "oldValue", "newValue", "url", "storageArea"]) {
    Object.defineProperty(Object.prototype, name, {
      configurable: true,
      get() { return undefined; },
      set(value) {
        if (globalThis.__captureStorageInitSetters) {
          const receiverKind = this instanceof StorageEvent ? "event" : "plain";
          globalThis.__storageInitSetterHits.push(`${receiverKind}:${name}`);
        }
        Object.defineProperty(this, name, {
          configurable: true,
          enumerable: true,
          writable: true,
          value
        });
      }
    });
  }
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    parent.__storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      url: event.url,
      storageArea: event.storageArea === localStorage,
      instance: event instanceof StorageEvent
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage event init data-property setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__captureStorageInitSetters = true;
  localStorage.setItem("__storage-init-key", "__storage-init-value");
  return __storageEvents.length;
})()
"#
        )
        .expect("storage mutation with Object.prototype setters should evaluate"),
        "0",
        "storage events must remain queued"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance queued storage event")
    );

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__captureStorageInitSetters = false;
  return JSON.stringify({
    events: globalThis.__storageEvents,
    plainSetterHits: globalThis.__storageInitSetterHits.filter(hit => hit.startsWith("plain:"))
  });
})()
"#
        )
        .expect("storage event init data-property result should evaluate"),
        r#"{"events":[{"key":"__storage-init-key","oldValue":null,"newValue":"__storage-init-value","url":"https://storage-event-init-data-property.test/page","storageArea":true,"instance":true}],"plainSetterHits":[]}"#
    );
}

#[tokio::test]
async fn storage_event_promise_continuation_after_child_dispatch_keeps_top_scope() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-continuation.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  sessionStorage.clear();
  globalThis.__storageContinuationDone = false;
  globalThis.__storageContinuationLog = [];
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__storageContinuationFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("storage continuation frame setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    vm.eval(
        r#"
(() => {
  const frame = __storageContinuationFrame;
  const record = event => [
    event.key,
    event.oldValue,
    event.newValue,
    event.storageArea === frame.contentWindow.sessionStorage
  ].join(":");
  const waitForStorage = () => new Promise(resolve => {
    const listener = event => {
      frame.contentWindow.removeEventListener("storage", listener);
      resolve(event);
    };
    frame.contentWindow.addEventListener("storage", listener);
  });

  waitForStorage()
    .then(event => {
      __storageContinuationLog.push(record(event));
      return waitForStorage();
    })
    .then(event => {
      __storageContinuationLog.push(record(event));
      const next = waitForStorage().then(event => {
        __storageContinuationLog.push(record(event));
      });
      sessionStorage.removeItem("missing-continuation-key");
      sessionStorage.setItem("continuation-second", "foo");
      return next;
    })
    .then(
      () => { __storageContinuationDone = true; },
      error => { __storageContinuationDone = "error:" + (error && error.name); }
    );

  sessionStorage.setItem("continuation-first", "foo");
  sessionStorage.setItem("continuation-first", "foo");
  sessionStorage.setItem("continuation-first", "bar");
  return "started";
})()
"#,
    )
    .expect("storage continuation promise chain should evaluate");

    for _ in 0..8 {
        if vm
            .eval("String(globalThis.__storageContinuationDone === true)")
            .expect("storage continuation done flag should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::StorageEvent,
                &loader,
            )
            .await
            .expect("storage continuation selected dispatcher should advance");
    }

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  done: globalThis.__storageContinuationDone,
  log: globalThis.__storageContinuationLog
})"#
        )
        .expect("storage continuation log should evaluate"),
        r#"{"done":true,"log":["continuation-first::foo:true","continuation-first:foo:bar:true","continuation-second::foo:true"]}"#
    );
}

#[tokio::test]
async fn child_message_handler_can_reply_through_event_source_origin() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-message-source-origin.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__sourceOriginReplies = [];
  addEventListener("message", event => {
    __sourceOriginReplies.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `<script>
    addEventListener("message", event => {
      event.source.postMessage("reply:" + event.origin + ":" + event.source.origin, event.source.origin);
    });
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__sourceOriginFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("source-origin frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    vm.eval(
        r#"
__sourceOriginFrame.contentWindow.postMessage(
  { command: "create ID" },
  __sourceOriginFrame.origin
);
"#,
    )
    .expect("source-origin postMessage should evaluate");

    for _ in 0..6 {
        if vm
            .eval("__sourceOriginReplies.length")
            .expect("source-origin reply length should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("source-origin wait driver should advance");
    }

    assert_eq!(
        vm.eval("__sourceOriginReplies.join('|')")
            .expect("source-origin reply should evaluate"),
        "reply:https://child-message-source-origin.test:https://child-message-source-origin.test"
    );
}

#[tokio::test]
async fn http_child_load_message_roundtrip_accepts_frame_origin_default() {
    let (child_url, server) = spawn_wpt_style_web_storage_message_child_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&child_url)
        .expect("child url")
        .join("/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__wptStyleRoundtrips = [];
  addEventListener("message", event => {{
    __wptStyleRoundtrips.push(JSON.stringify(event.data));
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  frame.addEventListener("load", () => {{
    frame.contentWindow.postMessage({{ command: "create ID", key: "userID" }}, frame.origin);
  }}, {{ once: true }});
  return "queued";
}})()
"#
    ))
    .expect("WPT-style child message setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__wptStyleRoundtrips.length",
        "1",
        "WPT-style child message roundtrip",
    )
    .await;

    assert_eq!(
        vm.eval("__wptStyleRoundtrips.join('|')")
            .expect("WPT-style roundtrip should evaluate"),
        r#"{"message":"ID created","userID":"created"}"#
    );
    let requests = server.await.expect("WPT-style child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn http_child_message_sent_before_navigation_commit_reaches_loaded_child() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&child_url)
        .expect("child url")
        .join("/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__pendingChildMessages = [];
  addEventListener("message", event => {{
    __pendingChildMessages.push(String(event.data));
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentWindow.postMessage({{ type: "getmessages" }}, "*");
  return "queued";
}})()
"#
    ))
    .expect("pending child message setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("pre-commit child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "pending-message child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("loaded child setup should use only child selected tasks");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__pendingChildMessages.length",
        "1",
        "message queued before child navigation commit",
    )
    .await;

    assert_eq!(
        vm.eval("__pendingChildMessages.join('|')")
            .expect("pending child message should evaluate"),
        "child:object:true"
    );
    let requests = server.await.expect("pending child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn captured_cross_origin_content_window_matches_message_source_after_child_navigation() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://localhost:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__capturedWindowMessages = [];
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  const captured = frame.contentWindow;
  globalThis.__capturedCrossOriginWindow = captured;
  addEventListener("message", event => {{
    __capturedWindowMessages.push({{
      data: String(event.data),
      sourceIsCaptured: event.source === captured,
      sourceIsCurrent: event.source === frame.contentWindow,
      currentIsCaptured: frame.contentWindow === captured,
      sourceIsTop: event.source === globalThis,
      sourceIsNull: event.source === null
    }});
  }});
  return "queued";
}})()
"#
    ))
    .expect("captured contentWindow message setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("captured-window child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "captured-window child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("captured-window loaded child should use only child selected tasks");
    assert_eq!(
        vm.eval("__capturedCrossOriginWindow === document.querySelector('iframe').contentWindow")
            .expect("captured WindowProxy identity should evaluate after navigation"),
        "true"
    );
    vm.eval("__capturedCrossOriginWindow.postMessage({ type: 'getmessages' }, '*')")
        .expect("captured WindowProxy should post to the replacement LocalWindow");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__capturedWindowMessages.length",
        "1",
        "captured WindowProxy message roundtrip",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(__capturedWindowMessages)")
            .expect("captured-window message should evaluate"),
        r#"[{"data":"child:object:true","sourceIsCaptured":true,"sourceIsCurrent":true,"currentIsCaptured":true,"sourceIsTop":false,"sourceIsNull":false}]"#
    );
    let requests = server.await.expect("captured child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn captured_cross_origin_content_window_keeps_safe_surface_during_realm_gap() {
    let (child_url, server) = spawn_pending_child_message_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://localhost:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");

    vm.eval(&format!(
        r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__realmGapFrame = frame;
  globalThis.__realmGapWindow = frame.contentWindow;
  return "queued";
}})()
"#
    ))
    .expect("realm-gap child setup should evaluate");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("realm-gap child setup should use only child selected tasks");
    wait_for_one_page_resource_completion_executor_test_turn(
        &mut vm,
        "realm-gap child document completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("realm-gap loaded child should use only child selected tasks");

    let child_handle = {
        let host = vm._context_host.borrow();
        let handles = host.child_browsing_context_handles_in_document_order();
        assert_eq!(handles.len(), 1, "realm-gap fixture should have one child");
        handles[0]
    };
    vm.retire_child_frame_realm_for_test(child_handle);

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `${error && error.name}:${error && error.message}`;
    }
  };
  return [
    __realmGapWindow === __realmGapFrame.contentWindow,
    probe(() => typeof __realmGapWindow.postMessage),
    probe(() => {
      __realmGapWindow.postMessage({ type: "watchcat" }, "*");
      return "no-throw";
    }),
    probe(() => __realmGapWindow.document)
  ].join("|");
})()
"#,
        )
        .expect("cross-origin WindowProxy realm-gap probes should evaluate");

    assert_eq!(
        result,
        concat!(
            "true|function|no-throw|",
            "SecurityError:Blocked a frame with a different origin from accessing a cross-origin frame."
        ),
        "a live browsing context must retain only its safe cross-origin WindowProxy surface while its LocalWindow realm is between generations"
    );
    let requests = server.await.expect("realm-gap child server should finish");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn same_origin_nested_children_share_window_security_token_without_top_access() {
    let (root_url, server) = spawn_nested_same_origin_window_access_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let root_url_parsed = Url::parse(&root_url).expect("root child URL");
    let top_url = format!(
        "http://localhost:{}/top.html",
        root_url_parsed
            .port()
            .expect("root child URL should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&top_url, &loader);
    let root_url_literal = serde_json::to_string(&root_url).expect("serialize root child URL");

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__nestedOriginResult = null;
  const frame = document.createElement("iframe");
  frame.src = {root_url_literal};
  addEventListener("message", event => {{
    globalThis.__nestedOriginResult = {{
      data: event.data,
      sourceIsRoot: event.source === frame.contentWindow
    }};
  }});
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__nestedOriginRootFrame = frame;
  return "queued";
}})()
"#
    ))
    .expect("nested same-origin child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__nestedOriginResult !== null)",
        "true",
        "nested same-origin child result",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__nestedOriginResult)")
            .expect("nested origin result should evaluate"),
        r#"{"data":{"marker":"nested","parentIsRoot":true,"topDenied":true,"wasmModule":true},"sourceIsRoot":true}"#
    );
    let requests = server.await.expect("nested origin server should finish");
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn queued_window_message_does_not_rebind_to_replacement_child_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-local-window-owner.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__windowMessageOwnerEvents = [];
  addEventListener("message", event => {
    __windowMessageOwnerEvents.push(String(event.data));
  });
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("old-local-window-received:" + event.data, "*");
    });
    parent.postMessage("initial-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__windowMessageOwnerFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("initial child window-message owner setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("initial child setup should use the selected-task dispatcher");
    for _ in 0..8 {
        if vm
            .eval("String(__windowMessageOwnerEvents.includes('initial-ready'))")
            .expect("initial child readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial child readiness should advance");
    }

    vm.eval(
        r#"
(() => {
  __windowMessageOwnerFrame.contentWindow.postMessage("stale-target", "*");
  __windowMessageOwnerFrame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("replacement-local-window-received:" + event.data, "*");
    });
    parent.postMessage("replacement-ready", "*");
  <\/script>`;
  return "navigating";
})()
"#,
    )
    .expect("queued message plus child replacement should evaluate");
    assert!(
        vm.run_one_child_frame_task_executor_turn(
            crate::frame_owner_model::ChildFrameSemanticTurnKind::NavigationCommit,
            &loader,
        )
        .await
        .expect("replacement child navigation commit should use the selected-task dispatcher"),
        "replacement child navigation commit should be runnable before the stale message"
    );
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("replacement child setup should use the selected-task dispatcher");

    for _ in 0..12 {
        let events = vm
            .eval("JSON.stringify(__windowMessageOwnerEvents)")
            .expect("window-message owner events should evaluate");
        if events.contains("replacement-ready") {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child replacement should advance");
    }
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("stale window-message timer should drain");
    }

    assert_eq!(
        vm.eval("JSON.stringify(__windowMessageOwnerEvents)")
            .expect("window-message owner result should evaluate"),
        r#"["initial-ready","replacement-ready"]"#
    );
}

#[tokio::test]
async fn queued_window_message_survives_same_local_window_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-document-open.test/",
        &loader,
    );
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("main window-message owner should exist");

    assert_eq!(
        vm.eval(
            r#"
postMessage("before-document-open", "*");
document.open();
document.close();
globalThis.__documentOpenWindowMessages = [];
onmessage = event => __documentOpenWindowMessages.push(event.data);
"queued"
"#,
        )
        .expect("queued postMessage plus document.open should evaluate"),
        "queued"
    );
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main window-message owner should exist");
    assert_ne!(after_owner, before_owner);
    assert_eq!(
        after_owner.local_window_id, before_owner.local_window_id,
        "document.open must retain the window-message target LocalWindow"
    );

    for _ in 0..4 {
        if vm
            .eval("__documentOpenWindowMessages.length")
            .expect("document.open window-message count should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("document.open window message should advance");
    }
    assert_eq!(
        vm.eval("__documentOpenWindowMessages.join('|')")
            .expect("document.open window-message result should evaluate"),
        "before-document-open"
    );
}

#[tokio::test]
async fn window_timer_survives_same_local_window_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-document-open.test/", &loader);
    let before_owner = vm
        .current_main_document_task_owner()
        .expect("main timer owner should exist");

    assert_eq!(
        vm.eval(
            r#"
globalThis.__documentOpenTimerEvents = [];
setTimeout(() => __documentOpenTimerEvents.push("preserved"), 0);
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"queued"
"#,
        )
        .expect("queued timer plus document.open should evaluate"),
        "queued"
    );
    let after_owner = vm
        .current_main_document_task_owner()
        .expect("replacement main timer owner should exist");
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("preserved LocalWindow timer should drain");
    assert_eq!(
        vm.eval("__documentOpenTimerEvents.join('|')")
            .expect("document.open timer result should evaluate"),
        "preserved"
    );
}

#[tokio::test]
async fn child_navigation_retires_old_local_window_timers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-local-window-owner.test/", &loader);

    vm.eval(
        r#"
globalThis.__childTimerOwnerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  parent.__childTimerOwnerEvents.push("old-ready");
  setTimeout(() => parent.__childTimerOwnerEvents.push("stale-timer"), 0);
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__childTimerOwnerFrame = frame;
"queued"
"#,
    )
    .expect("old child timer setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("__childTimerOwnerEvents.join('|')")
            .expect("old child timer setup result should evaluate"),
        "old-ready"
    );

    vm.eval(
        r#"
__childTimerOwnerFrame.srcdoc = `<!doctype html><script>
  parent.__childTimerOwnerEvents.push("replacement-ready");
<\/script>`;
"navigating"
"#,
    )
    .expect("child timer replacement should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired child timer should leave the timer queue quiescent");

    assert_eq!(
        vm.eval("__childTimerOwnerEvents.join('|')")
            .expect("child timer replacement result should evaluate"),
        "old-ready|replacement-ready"
    );
}

#[tokio::test]
async fn child_realm_retirement_cancels_callback_targeting_live_top_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-callback-relevant-realm.test/", &loader);

    vm.eval(
        r#"
globalThis.__timerRelevantRealmEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  parent.__timerRelevantRealmEvents.push("old-ready");
  parent.setTimeout(
    () => parent.__timerRelevantRealmEvents.push("stale-callback-realm"),
    0
  );
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__timerRelevantRealmFrame = frame;
"queued"
"#,
    )
    .expect("child callback-realm timer setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
__timerRelevantRealmFrame.srcdoc = `<!doctype html><script>
  parent.__timerRelevantRealmEvents.push("replacement-ready");
<\/script>`;
"navigating"
"#,
    )
    .expect("child callback-realm replacement should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("destroyed callback-realm timer should leave the queue quiescent");

    assert_eq!(
        vm.eval("__timerRelevantRealmEvents.join('|')")
            .expect("child callback-realm timer result should evaluate"),
        "old-ready|replacement-ready"
    );
}

#[tokio::test]
async fn child_window_timer_accepts_before_target_realm_materialization() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-pending-child-realm.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__pendingRealmTimerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const childWindow = frame.contentWindow;
const timerId = childWindow.setTimeout(
  () => __pendingRealmTimerEvents.push("fired"),
  0
);
const sourceTimerId = childWindow.setTimeout(
  "parent.__pendingRealmTimerEvents.push('source-fired')",
  0
);
String(timerId > 0 && sourceTimerId > 0)
"#,
        )
        .expect("pre-materialization child timer acceptance should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("materialized child timer should drain");
    assert_eq!(
        vm.eval("__pendingRealmTimerEvents.join('|')")
            .expect("pre-materialization child timer result should evaluate"),
        "fired|source-fired"
    );
}

#[tokio::test]
async fn initial_empty_child_timer_survives_superseded_precommit_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-pending-child-replacement.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__initialEmptyTimerEvents = [];
globalThis.__initialEmptyMainWindow = window;
globalThis.__initialEmptyMainDocument = document;
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>superseded child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const initialWindow = frame.contentWindow;
const initialDocument = initialWindow.document;
initialWindow.__initialEmptyMarker = "preserved";
initialWindow.addEventListener(
  "initial-empty-transition",
  () => __initialEmptyTimerEvents.push("preserved-listener")
);
initialDocument.addEventListener(
  "initial-empty-document-transition",
  () => __initialEmptyTimerEvents.push("stale-document-listener")
);
const timerId = initialWindow.setTimeout(
  () => __initialEmptyTimerEvents.push("preserved-timer"),
  0
);
frame.srcdoc = `<!doctype html><script>
  parent.__initialEmptyTimerEvents.push(
    "committed-ready:" + window.__initialEmptyMarker
  );
  window.dispatchEvent(new Event("initial-empty-transition"));
  document.dispatchEvent(new Event("initial-empty-document-transition"));
<\/script>`;
String(timerId > 0)
"#,
        )
        .expect("initial-empty timer setup should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("preserved initial-empty timer should drain");
    assert_eq!(
        vm.eval("__initialEmptyTimerEvents.join('|')")
            .expect("initial-empty timer result should evaluate"),
        "committed-ready:preserved|preserved-listener|preserved-timer"
    );
    assert_eq!(
        vm.eval(
            "String(window === __initialEmptyMainWindow && document === __initialEmptyMainDocument)"
        )
        .expect("main identity after child initial-empty transition should evaluate"),
        "true",
        "child initial-empty reuse must not replace the main Window or Document"
    );
}

#[tokio::test]
async fn initial_empty_document_domain_prevents_local_window_reuse() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://sub.initial-empty-domain.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__initialEmptyDomainEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = "<!doctype html><title>superseded child</title>";
(document.body || document.documentElement || document).appendChild(frame);
const initialWindow = frame.contentWindow;
initialWindow.__initialEmptyDomainMarker = "must-not-survive";
const timerId = initialWindow.setTimeout(
  () => __initialEmptyDomainEvents.push("stale-timer"),
  0
);
initialWindow.document.domain = "initial-empty-domain.test";
frame.srcdoc = `<!doctype html><script>
  parent.__initialEmptyDomainEvents.push(
    "committed-ready:" + String(window.__initialEmptyDomainMarker)
  );
<\/script>`;
String(timerId > 0)
"#,
        )
        .expect("document.domain transition setup should evaluate"),
        "true"
    );

    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("retired initial-empty LocalWindow timer should leave the queue quiescent");
    assert_eq!(
        vm.eval("__initialEmptyDomainEvents.join('|')")
            .expect("document.domain transition result should evaluate"),
        "committed-ready:undefined"
    );
}

#[tokio::test]
async fn window_clear_timer_is_scoped_to_its_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-clear-owner.test/", &loader);

    assert_eq!(
        vm.eval(
            r#"
globalThis.__scopedTimerEvents = [];
const popup = open("about:blank");
const popupTimer = popup.setTimeout(() => __scopedTimerEvents.push("popup"), 1);
clearTimeout(popupTimer);
const popupSourceTimer = popup.setTimeout(
  "globalThis.__scopedTimerEvents.push('popup-source')",
  1
);
clearTimeout(popupSourceTimer);
const topTimer = setTimeout(() => __scopedTimerEvents.push("top"), 1);
popup.clearTimeout(topTimer);
const cancelledPopupTimer = popup.setTimeout(
  () => __scopedTimerEvents.push("cancel-failed"),
  1
);
popup.clearTimeout(cancelledPopupTimer);
"queued"
"#,
        )
        .expect("cross-LocalWindow timer cancellation setup should evaluate"),
        "queued"
    );

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("scoped LocalWindow timers should drain");
    assert_eq!(
        vm.eval("JSON.stringify(__scopedTimerEvents.sort())")
            .expect("scoped timer result should evaluate"),
        r#"["popup","popup-source","top"]"#
    );
}

#[tokio::test]
async fn lightweight_popup_close_retires_its_local_window_timers() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://timer-popup-close.test/", &loader);

    vm.eval(
        r#"
globalThis.__closedPopupTimerEvents = [];
const popup = open("about:blank");
popup.setTimeout(() => __closedPopupTimerEvents.push("stale-popup-timer"), 0);
popup.close();
"closed"
"#,
    )
    .expect("popup timer close setup should evaluate");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("closed popup timer should leave the timer queue quiescent");
    assert_eq!(
        vm.eval("__closedPopupTimerEvents.length")
            .expect("closed popup timer result should evaluate"),
        "0"
    );
}

#[tokio::test]
async fn child_interval_does_not_reschedule_after_local_window_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://timer-interval-navigation.test/", &loader);

    vm.eval(
        r#"
globalThis.__childIntervalOwnerEvents = [];
const frame = document.createElement("iframe");
frame.srcdoc = `<!doctype html><script>
  setInterval(() => {
    parent.__childIntervalOwnerEvents.push("tick");
    parent.__childIntervalOwnerFrame.srcdoc =
      "<!doctype html><script>parent.__childIntervalOwnerEvents.push('replacement-ready');<\\/script>";
  }, 0);
<\/script>`;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__childIntervalOwnerFrame = frame;
"queued"
"#,
    )
    .expect("child interval navigation setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("first child interval turn should run")
    );
    vm.drain_pending_child_frame_work_for_test();
    let events_after_replacement = vm
        .eval("__childIntervalOwnerEvents.join('|')")
        .expect("child interval replacement state should evaluate");
    assert_eq!(events_after_replacement, "tick|replacement-ready");
    vm.advance_timers_until_deadline_for_test_with_deadline(
        &loader,
        std::time::Instant::now() + std::time::Duration::from_millis(20),
    )
    .await
    .expect("retired child interval queue should stay quiescent");

    assert_eq!(
        vm.eval("__childIntervalOwnerEvents.join('|')")
            .expect("child interval navigation result should evaluate"),
        "tick|replacement-ready"
    );
}

#[tokio::test]
async fn queued_window_message_survives_source_local_window_retirement() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-source-retirement.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__sourceRetirementEvents = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      parent.postMessage("from-retiring-source:" + event.data, "*");
      parent.__sourceRetirementFrame.srcdoc =
        "<!doctype html><script>parent.postMessage('replacement-ready', '*');<\\/script>";
    });
    parent.postMessage("initial-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__sourceRetirementFrame = frame;
  addEventListener("message", event => {
    __sourceRetirementEvents.push({
      data: String(event.data),
      sourceIsStableProxy: event.source === frame.contentWindow
    });
  });
  return "queued";
})()
"#,
    )
    .expect("source-retirement window-message setup should evaluate");
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial child setup should use the selected-task dispatcher")
        {
            break;
        }
    }
    for _ in 0..8 {
        if vm
            .eval("String(__sourceRetirementEvents.some(event => event.data === 'initial-ready'))")
            .expect("initial source-retirement readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("initial source-retirement readiness should advance");
    }

    vm.eval("__sourceRetirementFrame.contentWindow.postMessage('go', '*')")
        .expect("message to retiring source should queue");
    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("target child message should run")
    );
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("replacement child setup should use the selected-task dispatcher")
        {
            break;
        }
    }

    for _ in 0..8 {
        if vm
            .eval(
                "String(__sourceRetirementEvents.some(event => event.data === 'replacement-ready'))",
            )
            .expect("replacement source readiness should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("source-retirement messages should advance");
    }

    assert_eq!(
        vm.eval("JSON.stringify(__sourceRetirementEvents)")
            .expect("source-retirement result should evaluate"),
        r#"[{"data":"initial-ready","sourceIsStableProxy":true},{"data":"from-retiring-source:go","sourceIsStableProxy":true},{"data":"replacement-ready","sourceIsStableProxy":true}]"#
    );
}

#[tokio::test]
async fn child_post_message_reply_is_delivered_after_sender_installs_late_listener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-postmessage-late-listener.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__lateChildReplyMessages = [];
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>
      addEventListener("message", event => {
        parent.postMessage("reply:" + event.data, "*");
      });
    </` + `script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__lateChildReplyFrame = frame;
  return "ready";
})()
"#,
    )
    .expect("late listener child setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    vm.eval(
        r#"
(() => {
  __lateChildReplyFrame.contentWindow.postMessage("go", "*");
  addEventListener("message", event => {
    __lateChildReplyMessages.push({
      data: event.data,
      sourceIsChild: event.source === __lateChildReplyFrame.contentWindow
    });
  });
  return "posted";
})()
"#,
    )
    .expect("late listener postMessage should evaluate");

    for _ in 0..4 {
        if vm
            .eval("String(globalThis.__lateChildReplyMessages.length)")
            .expect("late listener message length should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("late listener reply should advance");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__lateChildReplyMessages)")
            .expect("late listener messages should evaluate"),
        r#"[{"data":"reply:go","sourceIsChild":true}]"#
    );
}

#[tokio::test]
async fn storage_events_preserve_wpt_dom_string_utf16_units() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-event-domstring-units.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  const frame = document.createElement('iframe');
  frame.srcdoc = `<body onstorage="
    const units = value => value === null ? null : Array.from({ length: value.length }, (_, index) => value.charCodeAt(index));
    parent.__storageEvents.push({
      key: units(event.key),
      oldValue: units(event.oldValue),
      newValue: units(event.newValue),
      storageArea: event.storageArea === localStorage
    });
  "></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("storage event UTF-16 setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
const key = String.fromCharCode(0xD800);
const value = String.fromCharCode(0xDC00);
localStorage.setItem(key, value);
return __storageEvents.length;
})()
"#
        )
        .expect("first surrogate storage mutation should evaluate"),
        "0"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance first surrogate storage event")
    );

    vm.eval(
        r#"
(() => {
const key = String.fromCharCode(0xD800);
const value = String.fromCharCode(0xD83C, 0xDF4D);
localStorage.setItem(key, value);
})()
"#,
    )
    .expect("second surrogate storage mutation should evaluate");
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance second surrogate storage event")
    );

    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("storage surrogate event result should evaluate"),
        r#"[{"key":[55296],"oldValue":null,"newValue":[56320],"storageArea":true},{"key":[55296],"oldValue":[56320],"newValue":[55356,57165],"storageArea":true}]"#
    );
}

#[tokio::test]
async fn repeated_iframe_src_assignment_reloads_child_storage_event_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-events-reload.test/page",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  globalThis.__storageEvents = [];
  globalThis.__storageChildLoads = 0;
  const childMarkup = `<!doctype html><body onstorage="
    parent.__storageEvents.push([
      event.key,
      event.oldValue === null,
      event.newValue,
      event.url,
      event.storageArea === localStorage
    ].join('|'));
  "></body>`;
  const childUrl = URL.createObjectURL(new Blob([childMarkup], { type: "text/html" }));
  const frame = document.createElement("iframe");
  frame.onload = () => { __storageChildLoads += 1; };
  frame.src = childUrl;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__storageFrame = frame;
  globalThis.__storageChildUrl = childUrl;
  return "queued";
})()
"#,
        )
        .expect("initial child storage event frame setup should evaluate");
    assert_eq!(setup, "queued");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageChildLoads")
            .expect("initial child storage frame load count should evaluate"),
        "1"
    );

    let before_reload = vm
        .eval("__storageFrame.src = __storageChildUrl; __storageChildLoads")
        .expect("same-src child reload should evaluate");
    assert_eq!(
        before_reload, "1",
        "same-src assignment should queue a new navigation instead of firing load synchronously"
    );
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageChildLoads")
            .expect("same-src child reload count should evaluate"),
        "2",
        "outside an active load event, assigning the same iframe src must reload"
    );

    assert_eq!(
        vm.eval("localStorage.setItem('k', 'v'); __storageEvents.length")
            .expect("storage mutation after child reload should evaluate"),
        "0",
        "storage events must remain queued after the reload"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance reloaded child storage event")
    );
    assert_eq!(
        vm.eval("__storageEvents.join('||')")
            .expect("reloaded child storage event result should evaluate"),
        "k|true|v|https://storage-events-reload.test/page|true"
    );
}

#[test]
fn document_open_preserves_message_ports_owned_by_main_local_window() {
    let mut vm = new_storage_test_vm("https://message-port-document-open.test/");

    vm.eval(
        r#"
(() => {
  const channel = new MessageChannel();
  globalThis.__documentOpenPort1 = channel.port1;
  globalThis.__documentOpenPort2 = channel.port2;
  return "created";
})()
"#,
    )
    .expect("main MessagePort pair should be created");

    let before_owner = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("main document owner should exist");
    let before_ports = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(before_ports.len(), 2);
    assert!(before_ports.iter().all(|(_, owner, _)| {
        *owner
            == crate::native_bridge::WindowExecutionContextOwner::Frame(
                before_owner.local_window_id,
            )
    }));

    vm.eval(
        r#"
document.open();
document.write("<!doctype html><title>replacement</title>");
document.close();
"opened"
"#,
    )
    .expect("document.open replacement should evaluate");

    let after_owner = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("replacement main document owner should exist");
    assert_eq!(after_owner.local_window_id, before_owner.local_window_id);
    assert_ne!(after_owner.document_id, before_owner.document_id);
    assert_eq!(
        vm._context_host
            .borrow()
            .message_port_execution_context_owners_for_test(),
        before_ports,
        "document.open must not retire or rebind LocalWindow-owned MessagePorts"
    );
}

#[tokio::test]
async fn transferred_child_message_port_rehomes_and_retires_with_local_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-local-window-owner.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__messagePortOwnerEvents = [];
  addEventListener("message", event => {
    __messagePortOwnerEvents.push(String(event.data));
  });
  const channel = new MessageChannel();
  globalThis.__messagePortOwnerLocalPort = channel.port1;
  globalThis.__messagePortOwnerTransferredPort = channel.port2;
  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    addEventListener("message", event => {
      globalThis.__ownedTransferredPort = event.ports[0];
      __ownedTransferredPort.start();
      parent.postMessage("port-bound", "*");
    });
    parent.postMessage("child-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__messagePortOwnerFrame = frame;
  return "queued";
})()
"#,
    )
    .expect("MessagePort execution-context owner setup should evaluate");

    let main_local_window_id = vm
        ._context_host
        .borrow()
        .current_main_document_task_owner()
        .expect("main document owner should exist")
        .local_window_id;
    let main_owner = crate::native_bridge::WindowExecutionContextOwner::Frame(main_local_window_id);
    let initial_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(initial_owners.len(), 2);
    assert!(
        initial_owners
            .iter()
            .all(|(_, owner, _)| *owner == main_owner)
    );

    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('child-ready'))")
            .expect("child-ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance child readiness");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('child-ready'))")
            .expect("child-ready completion should evaluate"),
        "true"
    );

    vm.eval(
        r#"
__messagePortOwnerFrame.contentWindow.postMessage(
  "bind-port",
  "*",
  [__messagePortOwnerTransferredPort]
);
"transferred"
"#,
    )
    .expect("MessagePort transfer should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('port-bound'))")
            .expect("port-bound state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance MessagePort transfer");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('port-bound'))")
            .expect("port-bound completion should evaluate"),
        "true"
    );

    let transferred_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(transferred_owners.len(), 2);
    assert_eq!(
        transferred_owners
            .iter()
            .filter(|(_, owner, _)| *owner == main_owner)
            .count(),
        1
    );
    let child_owner = transferred_owners
        .iter()
        .find_map(|(_, owner, _)| (*owner != main_owner).then_some(*owner))
        .expect("transferred endpoint should be owned by the child LocalWindow");
    assert!(matches!(
        child_owner,
        crate::native_bridge::WindowExecutionContextOwner::Frame(_)
    ));

    vm.eval(
        r#"
__messagePortOwnerFrame.srcdoc =
  "<!doctype html><script>parent.postMessage('replacement-ready', '*');<\/script>";
"navigating"
"#,
    )
    .expect("child replacement should evaluate");
    for _ in 0..8 {
        if vm
            .eval("String(__messagePortOwnerEvents.includes('replacement-ready'))")
            .expect("replacement-ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance child replacement");
    }
    assert_eq!(
        vm.eval("String(__messagePortOwnerEvents.includes('replacement-ready'))")
            .expect("replacement-ready completion should evaluate"),
        "true"
    );

    let replacement_owners = vm
        ._context_host
        .borrow()
        .message_port_execution_context_owners_for_test();
    assert_eq!(replacement_owners.len(), 1);
    assert_eq!(replacement_owners[0].1, main_owner);
    assert!(
        replacement_owners
            .iter()
            .all(|(_, owner, _)| *owner != child_owner),
        "navigation must actively retire the old child LocalWindow endpoint"
    );
}

#[tokio::test]
async fn stale_child_message_port_does_not_dispatch_to_reused_iframe_generation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-child-generation.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__messagePortGenerationEvents = [];
  addEventListener("message", event => {
    __messagePortGenerationEvents.push("window:" + event.data + ":" + event.origin);
  });

  const frame = document.createElement("iframe");
  frame.srcdoc = `<!doctype html><script>
    const channel = new MessageChannel();
    channel.port2.onmessage = () => {
      parent.postMessage("stale-port-handler-ran", "*");
    };
    parent.__staleChildPort = channel.port1;
    parent.postMessage("first-ready", "*");
  <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__messagePortGenerationFrame = frame;
  return "queued";
})()
"#,
        )
        .expect("stale child MessagePort setup should evaluate");
    assert_eq!(setup, "queued");

    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    for _ in 0..4 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortGenerationEvents.includes(
  "window:first-ready:null"
))"#,
            )
            .expect("first child ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should observe first child ready");
    }

    vm.eval(
        r#"
globalThis.__messagePortGenerationEvents = [];
__messagePortGenerationFrame.srcdoc = "<!doctype html><script>parent.postMessage('second-ready', '*');<\/script>";
"#,
    )
    .expect("second child navigation should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    for _ in 0..4 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortGenerationEvents.includes(
  "window:second-ready:https://message-port-child-generation.test"
))"#,
            )
            .expect("second child ready state should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should observe second child ready");
    }

    vm.eval("__staleChildPort.postMessage('should-not-dispatch')")
        .expect("posting to stale child MessagePort should evaluate");
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should flush stale child MessagePort wake");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__messagePortGenerationEvents)")
            .expect("stale child MessagePort events should evaluate"),
        r#"["window:second-ready:https://message-port-child-generation.test"]"#
    );
}

#[tokio::test]
async fn child_storage_mutations_queue_events_to_parent_window() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://storage-child.test/page",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  sessionStorage.clear();
  globalThis.__storageEvents = [];
  addEventListener('storage', event => {
    __storageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      storageArea: event.storageArea === sessionStorage,
      instance: event instanceof StorageEvent,
      tag: Object.prototype.toString.call(event)
    });
  });
  const frame = document.createElement('iframe');
  frame.srcdoc = `<script>sessionStorage.setItem('child', '1');<\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return __storageEvents.length;
})()
"#,
    )
    .expect("child storage setup should evaluate");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;
    assert_eq!(
        vm.eval("__storageEvents.length")
            .expect("child storage event should still be queued"),
        "0"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance child-origin storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__storageEvents)")
            .expect("child storage event result should evaluate"),
        r#"[{"key":"child","oldValue":null,"newValue":"1","storageArea":true,"instance":true,"tag":"[object StorageEvent]"}]"#
    );
}

#[tokio::test]
async fn opaque_origin_frames_reject_web_storage_access() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://opaque-storage.test/page.html",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__opaqueStorageMessages = [];
  addEventListener("message", event => {
    __opaqueStorageMessages.push(String(event.data));
  });

  const createFrame = id => {
    const frame = document.createElement("iframe");
    frame.setAttribute("sandbox", "allow-scripts");
    frame.srcdoc = `<script>
      const outcome = callback => {
        try {
          callback();
          return "resolved";
        } catch (error) {
          return error && error.name;
        }
      };
      parent.postMessage([
        "${id}",
        outcome(() => localStorage),
        outcome(() => sessionStorage),
        location.origin
      ].join(":"), "*");
    <\/script>`;
    return frame;
  };

  const host = document.body || document.documentElement || document;
  host.appendChild(createFrame("left"));
  host.appendChild(createFrame("right"));
  return "queued";
})()
"#,
        )
        .expect("opaque WebStorage setup should evaluate");
    assert_eq!(setup, "queued");

    for _ in 0..8 {
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child setup should use the selected-task dispatcher");
        let message_count = vm
            .eval(r#"String(__opaqueStorageMessages.length)"#)
            .expect("opaque WebStorage message count should evaluate");
        if message_count == "2" {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque WebStorage child load should advance");
    }
    for _ in 0..4 {
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("opaque WebStorage storage event drain should advance");
    }

    let result = vm
        .eval("JSON.stringify(__opaqueStorageMessages.sort())")
        .expect("opaque WebStorage messages should evaluate");
    assert_eq!(
        result,
        r#"["left:SecurityError:SecurityError:null","right:SecurityError:SecurityError:null"]"#
    );
}

#[test]
fn top_web_storage_receiver_ignores_ambient_opaque_child_owner() {
    let mut vm = new_storage_test_vm("https://top-web-storage-owner.test/page.html");
    vm.eval(
        r#"
(() => {
  localStorage.clear();
  localStorage.setItem("top-key", "top-value");
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts");
  frame.srcdoc = "<!doctype html><script>void localStorage;<\/script>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("top WebStorage owner setup should evaluate");

    let child_handle = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_has_opaque_origin(child_handle),
        "sandboxed child should have an opaque storage origin"
    );

    let top_context_ptr = &vm.page_default_context as *const v8::Global<v8::Context>;
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous =
            crate::native_bridge::enter_active_child_window_scope(scope, Some(child_handle));
        Ok(())
    })
    .expect("ambient opaque child owner should install");

    let result = vm.eval("localStorage.getItem('top-key')");
    vm.with_context_scope_by_ptr(top_context_ptr, |scope, _host_ptr| {
        let _previous = crate::native_bridge::enter_active_child_window_scope(scope, None);
        Ok(())
    })
    .expect("ambient opaque child owner should clear");

    assert_eq!(
        result.expect("top localStorage should use the top Window receiver"),
        "top-value"
    );
}

#[tokio::test]
async fn third_party_web_storage_is_partitioned_by_top_level_site() {
    let (child_origin, server) = spawn_web_storage_partition_child_server(3).await;
    let child_url = format!("{child_origin}/partition-child.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let web_storage = crate::RendererWebStorageHandles::ephemeral();

    let first_a = run_web_storage_partition_probe(
        "http://top-a-webstorage-partition.test/page.html",
        &child_url,
        "a1",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        first_a,
        r#"{"label":"a1","beforeLocal":null,"beforeSession":null,"afterLocal":"a1","afterSession":"a1"}"#
    );

    let first_b = run_web_storage_partition_probe(
        "http://top-b-webstorage-partition.test/page.html",
        &child_url,
        "b1",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        first_b,
        r#"{"label":"b1","beforeLocal":null,"beforeSession":null,"afterLocal":"b1","afterSession":"b1"}"#
    );

    let second_a = run_web_storage_partition_probe(
        "http://top-a-webstorage-partition.test/second.html",
        &child_url,
        "a2",
        &loader,
        &web_storage,
    )
    .await;
    assert_eq!(
        second_a,
        r#"{"label":"a2","beforeLocal":"a1","beforeSession":"a1","afterLocal":"a2","afterSession":"a2"}"#
    );

    let requests = server.await.expect("partition child server should finish");
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("GET /partition-child.html?label=")),
        "unexpected child frame requests: {requests:?}"
    );
}

#[tokio::test]
async fn third_party_about_blank_popup_does_not_reuse_opener_or_first_party_storage_area() {
    let (child_origin, server) = spawn_about_blank_popup_storage_child_server().await;
    let child_url = format!("{child_origin}/popup-child.html");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://top-about-blank-popup-partition.test/page.html",
        &loader,
    );

    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    vm.eval(&format!(
        r#"
(() => {{
  localStorage.clear();
  localStorage.setItem("popup-scope", "top");
  globalThis.__aboutBlankPopupStorageMessage = null;
  addEventListener("message", event => {{
    globalThis.__aboutBlankPopupStorageMessage = String(event.data);
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
    ))
    .expect("about:blank popup partition setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__aboutBlankPopupStorageMessage !== null)",
        "true",
        "about:blank popup partition result",
    )
    .await;

    assert_eq!(
        vm.eval("globalThis.__aboutBlankPopupStorageMessage || 'missing'")
            .expect("about:blank popup message should evaluate"),
        r#"{"popupBefore":null,"popupAfter":"popup-first-party","childAfter":"child-partition","opener":true}"#
    );
    assert_eq!(
        vm.eval("localStorage.getItem('popup-scope')")
            .expect("top localStorage should evaluate"),
        "top"
    );

    let mut child_first_party_vm =
        new_storage_test_vm_with_loader(&format!("{child_origin}/first-party.html"), &loader);
    assert_eq!(
        child_first_party_vm
            .eval("localStorage.getItem('popup-scope')")
            .expect("child first-party localStorage should evaluate"),
        "null"
    );

    let requests = server.await.expect("popup child server should finish");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /popup-child.html "),
        "unexpected popup child request: {:?}",
        requests[0]
    );
}

async fn run_web_storage_partition_probe(
    top_url: &str,
    child_url: &str,
    label: &str,
    loader: &ResourceRequestClient,
    web_storage: &crate::RendererWebStorageHandles,
) -> String {
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url, loader);
    vm.set_web_storage_handles(web_storage);
    let child_url = format!("{child_url}?label={label}");
    let child_url_literal = serde_json::to_string(&child_url).expect("child url should serialize");
    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__partitionMessage = null;
  addEventListener("message", event => {{
    globalThis.__partitionMessage = String(event.data);
  }});
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  return "queued";
}})()
"#
    ))
    .expect("partition probe setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        loader,
        "String(globalThis.__partitionMessage !== null)",
        "true",
        "third-party WebStorage partition result",
    )
    .await;

    let result = vm
        .eval("globalThis.__partitionMessage || 'missing'")
        .expect("partition message should evaluate");
    assert_ne!(
        result, "missing",
        "third-party child frame did not post its WebStorage result"
    );
    result
}

async fn spawn_web_storage_partition_child_server(
    expected_requests: usize,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WebStorage partition child server");
    let addr = listener
        .local_addr()
        .expect("WebStorage partition child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..expected_requests {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept WebStorage partition child request");
            let request = read_web_storage_partition_request_head(&mut stream)
                .await
                .expect("read WebStorage partition child request");
            let status = if request.starts_with("GET /partition-child.html?") {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
const label = new URL(location.href).searchParams.get("label");
const beforeLocal = localStorage.getItem("partitioned-local");
const beforeSession = sessionStorage.getItem("partitioned-session");
localStorage.setItem("partitioned-local", label);
sessionStorage.setItem("partitioned-session", label);
parent.postMessage(JSON.stringify({
  label,
  beforeLocal,
  beforeSession,
  afterLocal: localStorage.getItem("partitioned-local"),
  afterSession: sessionStorage.getItem("partitioned-session")
}), "*");
</script>
"#;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write WebStorage partition child response");
            requests.push(request);
        }
        requests
    });
    (format!("http://{addr}"), server)
}

async fn spawn_wpt_style_web_storage_message_child_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind WPT-style WebStorage message child server");
    let addr = listener
        .local_addr()
        .expect("WPT-style WebStorage message child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept WPT-style WebStorage message child request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read WPT-style WebStorage message child request");
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
window.addEventListener("message", event => {
  if (event.data.command === "create ID") {
    localStorage.setItem(event.data.key, "created");
    event.source.postMessage({
      message: "ID created",
      userID: localStorage.getItem("userID"),
    }, event.source.origin);
  }
});
</script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write WPT-style WebStorage message child response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_child_response_csp_sandbox_document_domain_server()
-> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind child response CSP sandbox document.domain server");
    let addr = listener
        .local_addr()
        .expect("child response CSP sandbox document.domain server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept child response CSP sandbox document.domain request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read child response CSP sandbox document.domain request");
        let body = "<!doctype html><title>child</title>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: sandbox allow-scripts allow-same-origin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write child response CSP sandbox document.domain response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_pending_child_message_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pending child message server");
    let addr = listener
        .local_addr()
        .expect("pending child message server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept pending child message request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read pending child message request");
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
addEventListener("message", event => {
  parent.postMessage("child:" + typeof event.data + ":" + (event.source === parent), "*");
});
</script>
"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write pending child message response");
        vec![request]
    });
    (format!("http://{addr}/child.html"), server)
}

async fn spawn_nested_same_origin_window_access_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind nested same-origin Window server");
    let addr = listener
        .local_addr()
        .expect("nested same-origin Window server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept nested same-origin Window request");
            let request = read_web_storage_partition_request_head(&mut stream)
                .await
                .expect("read nested same-origin Window request");
            let body = if request.starts_with("GET /root.html ") {
                r#"<!doctype html><body><script>
const nested = document.createElement("iframe");
nested.src = "/nested.html";
nested.addEventListener("load", () => {
  let topDenied = false;
  try {
    void top.document;
  } catch (error) {
    topDenied = error && error.name === "SecurityError";
  }
  let data;
  try {
    const module = new WebAssembly.Module(
      new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
    );
    data = {
      marker: nested.contentWindow.document.body.dataset.marker,
      parentIsRoot: nested.contentWindow.parent === globalThis,
      topDenied,
      wasmModule: module instanceof WebAssembly.Module
    };
  } catch (error) {
    data = { error: error && error.name, topDenied };
  }
  top.postMessage(data, "*");
});
document.body.appendChild(nested);
</script></body>"#
            } else {
                r#"<!doctype html><body data-marker="nested"></body>"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write nested same-origin Window response");
            requests.push(request);
        }
        requests
    });
    (format!("http://{addr}/root.html"), server)
}

async fn spawn_about_blank_popup_storage_child_server() -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind about:blank popup storage child server");
    let addr = listener
        .local_addr()
        .expect("about:blank popup storage child server addr");
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept about:blank popup storage child request");
        let request = read_web_storage_partition_request_head(&mut stream)
            .await
            .expect("read about:blank popup storage child request");
        let status = if request.starts_with("GET /popup-child.html ") {
            "200 OK"
        } else {
            "404 Not Found"
        };
        let body = r#"<!doctype html>
<meta charset="utf-8">
<script>
localStorage.setItem("popup-scope", "child-partition");
const popup = window.open("about:blank");
const popupBefore = popup.localStorage.getItem("popup-scope");
popup.localStorage.setItem("popup-scope", "popup-first-party");
parent.postMessage(JSON.stringify({
  popupBefore,
  popupAfter: popup.localStorage.getItem("popup-scope"),
  childAfter: localStorage.getItem("popup-scope"),
  opener: popup.opener === window
}), "*");
</script>
"#;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write about:blank popup storage child response");
        requests.push(request);
        requests
    });
    (format!("http://{addr}"), server)
}

async fn read_web_storage_partition_request_head(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn document_domain_exact_self_assignment_keeps_storage_event_delivery() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://www.example.com/document-domain-page",
        &loader,
    );

    let initial = vm
        .eval(
            r#"
(() => {
  localStorage.clear();
  globalThis.__documentDomainStorageEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "document-domain-storage-child";
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.contentWindow.location.href;
})()
"#,
        )
        .expect("document.domain storage event iframe setup should evaluate");
    assert_eq!(initial, "about:blank");
    drain_pending_page_child_frame_work_for_test(&mut vm).await;

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById("document-domain-storage-child");
  frame.contentWindow.addEventListener("storage", event => {
    __documentDomainStorageEvents.push({
      key: event.key,
      oldValue: event.oldValue,
      newValue: event.newValue,
      storageArea: event.storageArea === frame.contentWindow.localStorage,
      childDomain: frame.contentDocument.domain
    });
  });
  let childDomain;
  try {
    frame.contentDocument.domain = document.domain;
    childDomain = frame.contentDocument.domain;
  } catch (error) {
    childDomain = `${error.name}:${error.code}`;
  }
  localStorage.setItem("test", "test");
  return JSON.stringify({
    parentDomain: document.domain,
    childDomain,
    eventCount: __documentDomainStorageEvents.length
  });
})()
"#,
        )
        .expect("document.domain storage event mutation should evaluate");
    assert_eq!(
        setup,
        r#"{"parentDomain":"www.example.com","childDomain":"www.example.com","eventCount":0}"#,
        "document.domain exact-self assignment must succeed before the queued event fires"
    );

    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::StorageEvent,
            &loader,
        )
        .await
        .expect("selected dispatcher should advance document.domain storage event")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__documentDomainStorageEvents)")
            .expect("document.domain storage event result should evaluate"),
        r#"[{"key":"test","oldValue":null,"newValue":"test","storageArea":true,"childDomain":"www.example.com"}]"#
    );
}

#[tokio::test]
async fn child_window_post_message_dispatches_a_trusted_message_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://window-message-trust.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__windowMessageTrust = [];
  const frame = document.createElement("iframe");
  addEventListener("message", event => {
    __windowMessageTrust.push({
      data: event.data,
      trusted: event.isTrusted,
      sourceIsChild: event.source !== null && event.source === frame.contentWindow
    });
  });
  dispatchEvent(new MessageEvent("message", { data: "synthetic" }));
  frame.srcdoc = `<script>parent.postMessage("child", "*");<\/script>`;
  (document.body || document.documentElement).appendChild(frame);
  return String(__windowMessageTrust.length);
})()
"#,
        )
        .expect("window message trust setup should evaluate");
    assert_eq!(setup, "1", "synthetic dispatch should remain synchronous");

    for _ in 0..128 {
        if vm
            .eval("String(__windowMessageTrust.length)")
            .expect("window message trust length should evaluate")
            == "2"
        {
            break;
        }
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child postMessage setup should advance")
        {
            break;
        }
    }

    assert_eq!(
        vm.eval("JSON.stringify(__windowMessageTrust)")
            .expect("window message trust result should evaluate"),
        r#"[{"data":"synthetic","trusted":false,"sourceIsChild":false},{"data":"child","trusted":true,"sourceIsChild":true}]"#
    );
}

#[tokio::test]
async fn window_post_message_validates_and_normalizes_target_origin() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-target.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  onmessage = event => {
    __messageEvents.push({
      data: event.data,
      origin: event.origin,
      ports: event.ports.length
    });
  };
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error.code}`;
    }
  };
  const badHost = probe(() => postMessage("", "http://foo bar", []));
  const relative = probe(() => postMessage("", "example.org", []));
  postMessage(["ok"], location.protocol + "//" + location.host + "/", []);
  postMessage({fromOptions: true}, {targetOrigin: location.origin});
  return `${badHost}|${relative}|${__messageEvents.length}`;
})()
"#,
        )
        .expect("window postMessage targetOrigin setup should evaluate");

    assert_eq!(result, "SyntaxError:12|SyntaxError:12|0");
    for _ in 0..6 {
        if vm
            .eval("__messageEvents.length")
            .expect("queued window message count should evaluate")
            == "2"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should drain queued window message");
    }
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("queued window message should evaluate"),
        r#"[{"data":["ok"],"origin":"https://message-target.test","ports":0},{"data":{"fromOptions":true},"origin":"https://message-target.test","ports":0}]"#
    );
}

#[tokio::test]
async fn window_post_message_allows_promise_continuations_between_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-order.test/path",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageOrder = [];
  let waitingFor = "initial";
  addEventListener("message", event => {
    if (!waitingFor) {
      __messageOrder.push("unexpected:" + event.data);
      return;
    }
    __messageOrder.push("message:" + event.data + ":" + waitingFor);
    waitingFor = "";
    Promise.resolve().then(() => {
      __messageOrder.push("microtask-after:" + event.data);
      if (event.data !== "third") {
        waitingFor = "after-" + event.data;
      }
    });
  });
  postMessage("first", "*");
  postMessage("second", "*");
  postMessage("third", "*");
  return String(__messageOrder.length);
})()
"#,
        )
        .expect("window postMessage ordering setup should evaluate");

    assert_eq!(result, "0");
    for _ in 0..10 {
        let order = vm
            .eval("JSON.stringify(__messageOrder)")
            .expect("window postMessage order should evaluate while waiting");
        if order
            == r#"["message:first:initial","microtask-after:first","message:second:after-first","microtask-after:second","message:third:after-second","microtask-after:third"]"#
        {
            break;
        }
        let _ = vm
            .run_one_window_message_executor_turn(&loader)
            .await
            .expect("wait driver should drain ordered window messages");
    }
    assert_eq!(
        vm.eval("JSON.stringify(__messageOrder)")
            .expect("window postMessage order should evaluate"),
        r#"["message:first:initial","microtask-after:first","message:second:after-first","microtask-after:second","message:third:after-second","microtask-after:third"]"#
    );
}

#[tokio::test]
async fn window_post_message_uses_an_independent_one_message_per_turn_task_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-task-source.test/path",
        &loader,
    );

    assert_eq!(
        vm.eval(
            r#"
globalThis.__postedMessageTaskEvents = [];
addEventListener("message", event => {
  __postedMessageTaskEvents.push("message:" + event.data);
  Promise.resolve().then(() => {
    __postedMessageTaskEvents.push("microtask:" + event.data);
  });
});
postMessage("first", "*");
postMessage("second", "*");
JSON.stringify(__postedMessageTaskEvents)
"#,
        )
        .expect("posted-message task-source setup should evaluate"),
        "[]"
    );
    assert!(vm.has_ready_window_message_task());
    assert!(
        !vm.has_ready_timeout(),
        "postMessage must not create a synthetic timer task"
    );

    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("first posted-message task should run")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__postedMessageTaskEvents)")
            .expect("first posted-message checkpoint result should evaluate"),
        r#"["message:first","microtask:first"]"#
    );
    assert!(
        vm.has_ready_window_message_task(),
        "the remaining message must publish exactly one continuation"
    );
    assert!(!vm.has_ready_timeout());

    assert!(
        vm.run_one_window_message_executor_turn(&loader)
            .await
            .expect("second posted-message task should run")
    );
    assert_eq!(
        vm.eval("JSON.stringify(__postedMessageTaskEvents)")
            .expect("second posted-message result should evaluate"),
        r#"["message:first","microtask:first","message:second","microtask:second"]"#
    );
    assert!(!vm.has_ready_window_message_task());
    assert!(!vm.has_ready_timeout());
}

#[tokio::test]
async fn window_post_message_array_second_argument_uses_options_defaults_without_transfer() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-array-options.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      data: event.data,
      origin: event.origin,
      ports: event.ports.length
    });
  };
  try {
    postMessage({fromRawArray: true}, [channel.port2]);
  } catch (error) {
    return `${error.name}:${error.code}`;
  }
  return String(__messageEvents.length);
})()
"#,
        )
        .expect("window postMessage raw-array options setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain raw-array window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("raw-array window message should evaluate"),
        r#"[{"data":{"fromRawArray":true},"origin":"https://message-array-options.test","ports":0}]"#
    );
}

#[test]
fn document_domain_setter_records_explicit_parent_domain() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  document.domain = "example.com";
  const relaxed = document.domain;
  let invalid;
  try {
    document.domain = "other.example";
    invalid = "no-throw";
  } catch (error) {
    invalid = `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
  return `${initial}|${relaxed}|${invalid}`;
})()
"#,
        )
        .expect("document.domain setter probe should evaluate");

    assert_eq!(result, "www.example.com|example.com|SecurityError:true:18");
}

#[test]
fn csp_sandbox_disallows_top_document_domain_setter() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");
    vm.set_response_content_security_policies(&["sandbox allow-scripts".to_owned()]);

    let result = vm
        .eval(
            r#"
(() => {
  try {
    document.domain = document.domain;
    return "no-throw";
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("CSP sandbox document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:true:18");
}

#[test]
fn csp_sandbox_allow_same_origin_disallows_top_document_domain_setter() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");
    vm.set_response_content_security_policies(&[
        "sandbox allow-scripts allow-same-origin".to_owned()
    ]);

    let result = vm
        .eval(
            r#"
(() => {
  try {
    const initial = document.domain;
    document.domain = document.domain;
    return `${initial}|${document.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("CSP allow-same-origin document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:true:18");
}

#[tokio::test]
async fn child_response_csp_sandbox_disallows_document_domain_setter() {
    let (child_url, server) = spawn_child_response_csp_sandbox_document_domain_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let child_url_parsed = Url::parse(&child_url).expect("child url");
    let document_url = format!(
        "http://127.0.0.1:{}/parent.html",
        child_url_parsed
            .port()
            .expect("child url should carry a port")
    );
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("serialize child url");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  const frame = document.createElement("iframe");
  frame.src = {child_url_literal};
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__responseCspSandboxFrame = frame;
  return "queued";
}})()
"#
        ))
        .expect("child response CSP sandbox setup should evaluate");
    assert_eq!(setup, "queued");
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 256)
        .await
        .expect("child response CSP setup should use the selected-task dispatcher");

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "child response CSP sandbox completion",
    )
    .await;
    vm.drain_ready_child_frame_task_executor_turns_for_setup(&loader, 256)
        .await
        .expect("child response CSP lifecycle should use the selected-task dispatcher");
    let requests = server
        .await
        .expect("child response CSP sandbox server should finish");
    assert_eq!(requests.len(), 1);

    let result = vm
        .eval(
            r#"
(() => {
  const frame = globalThis.__responseCspSandboxFrame;
  const ChildDOMException = frame.contentWindow.DOMException;
  const document = frame.contentDocument;
  try {
    const initial = document.domain;
    document.domain = document.domain;
    return `${initial}|${document.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error instanceof ChildDOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("child response CSP sandbox document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:false:true:18");
}

#[test]
fn iframe_sandbox_allow_same_origin_disallows_document_domain_after_document_open() {
    let mut vm = new_storage_test_vm("https://www.example.com/path");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", "allow-scripts allow-same-origin");
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.open();
  frame.contentDocument.write("<!doctype html><title>child</title>");
  frame.contentDocument.close();
  const ChildDOMException = frame.contentWindow.DOMException;
  try {
    const initial = frame.contentDocument.domain;
    frame.contentDocument.domain = frame.contentDocument.domain;
    return `${initial}|${frame.contentDocument.domain}`;
  } catch (error) {
    return `${error.name}:${error instanceof DOMException}:${error instanceof ChildDOMException}:${error.code}`;
  }
})()
"#,
        )
        .expect("sandbox allow-same-origin document.domain probe should evaluate");

    assert_eq!(result, "SecurityError:false:true:18");
}

#[test]
fn document_domain_setter_rejects_ip_suffix_relaxation() {
    let mut vm = new_storage_test_vm("http://127.0.0.1/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  document.domain = "127.0.0.1";
  const exact = document.domain;
  let suffix;
  try {
    document.domain = "0.0.1";
    suffix = "no-throw";
  } catch (error) {
    suffix = `${error.name}:${error instanceof DOMException}:${error.code}:${document.domain}`;
  }
  return `${initial}|${exact}|${suffix}`;
})()
"#,
        )
        .expect("document.domain IP setter probe should evaluate");

    assert_eq!(
        result,
        "127.0.0.1|127.0.0.1|SecurityError:true:18:127.0.0.1"
    );
}
#[test]
fn document_domain_setter_rejects_public_suffix_relaxation() {
    let mut vm = new_storage_test_vm("https://www.co.uk/path");

    let result = vm
        .eval(
            r#"
(() => {
  const initial = document.domain;
  let suffix;
  try {
    document.domain = "co.uk";
    suffix = "no-throw";
  } catch (error) {
    suffix = `${error.name}:${error instanceof DOMException}:${error.code}:${document.domain}`;
  }
  document.domain = "www.co.uk";
  const exact = document.domain;
  return `${initial}|${suffix}|${exact}`;
})()
"#,
        )
        .expect("document.domain public suffix setter probe should evaluate");

    assert_eq!(
        result,
        "www.co.uk|SecurityError:true:18:www.co.uk|www.co.uk"
    );
}

#[test]
fn message_port_is_not_constructible_and_channel_ports_keep_declared_state() {
    let mut vm = new_storage_test_vm("https://message-port-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(":");
  };
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      descriptor?.enumerable,
      typeof descriptor?.set,
      descriptor?.configurable
    ].join(":");
  };
  let constructorResult;
  try {
    new MessagePort();
    constructorResult = "no-throw";
  } catch (error) {
    constructorResult = `${error.name}:${error instanceof TypeError}`;
  }
  const standalone = new MessageChannel().port1;
  const channel = new MessageChannel();
  const originalPort1 = channel.port1;
  const originalPort2 = channel.port2;
  const messagePortOwnSlots = Object.getOwnPropertyNames(standalone)
    .filter(name => name.startsWith("__lmMessagePort") ||
      name.startsWith("__moliMessagePort"))
    .sort();
  const messageChannelOwnSlots = Object.getOwnPropertyNames(channel)
    .filter(name => name.startsWith("__moliMessageChannel"))
    .sort();
  Object.defineProperties(MessagePort.prototype, {
    __lmMessagePortOnmessageHandler: { value: () => "proto-message", configurable: true },
    __lmMessagePortOnmessageerrorHandler: { value: () => "proto-error", configurable: true },
    __lmMessagePortOncloseHandler: { value: () => "proto-close", configurable: true },
    __moliMessagePortListeners: { value: [], configurable: true }
  });
  Object.defineProperties(standalone, {
    __lmMessagePortOnmessageHandler: { value: () => "own-message", configurable: true },
    __lmMessagePortOnmessageerrorHandler: { value: () => "own-error", configurable: true },
    __lmMessagePortOncloseHandler: { value: () => "own-close", configurable: true },
    __moliMessagePortListeners: { value: [], configurable: true }
  });
  Object.defineProperties(MessageChannel.prototype, {
    __moliMessageChannelPort1: { value: "proto-port1", configurable: true },
    __moliMessageChannelPort2: { value: "proto-port2", configurable: true }
  });
  Object.defineProperties(channel, {
    __moliMessageChannelPort1: { value: "own-port1", configurable: true },
    __moliMessageChannelPort2: { value: "own-port2", configurable: true }
  });
  standalone.onmessage = () => "real-message";
  standalone.onmessageerror = () => "real-error";
  standalone.onclose = () => "real-close";
  return JSON.stringify({
    constructorResult,
    standaloneTag: Object.prototype.toString.call(standalone),
    standaloneCtor: standalone.constructor && standalone.constructor.name,
    standaloneProtoCtor: Object.getPrototypeOf(standalone)?.constructor?.name ?? null,
    standaloneKeys: Object.keys(standalone).join(","),
    messagePortMethods: [
      methodDescriptor(MessagePort.prototype, "postMessage"),
      methodDescriptor(MessagePort.prototype, "start"),
      methodDescriptor(MessagePort.prototype, "close"),
      methodDescriptor(MessagePort.prototype, "addEventListener"),
      methodDescriptor(MessagePort.prototype, "removeEventListener")
    ],
    messagePortAccessors: [
      accessorDescriptor(MessagePort.prototype, "onmessage"),
      accessorDescriptor(MessagePort.prototype, "onmessageerror"),
      accessorDescriptor(MessagePort.prototype, "onclose")
    ],
    messageChannelAccessors: [
      accessorDescriptor(MessageChannel.prototype, "port1"),
      accessorDescriptor(MessageChannel.prototype, "port2")
    ],
    messagePortOwnSlots,
    messageChannelOwnSlots,
    standaloneOnmessage: typeof standalone.onmessage,
    standaloneOnmessageIsSpoof: standalone.onmessage === standalone.__lmMessagePortOnmessageHandler,
    standaloneOnmessageerror: typeof standalone.onmessageerror,
    standaloneOnmessageerrorIsSpoof: standalone.onmessageerror === standalone.__lmMessagePortOnmessageerrorHandler,
    standaloneOnclose: typeof standalone.onclose,
    standaloneOncloseIsSpoof: standalone.onclose === standalone.__lmMessagePortOncloseHandler,
    portTag: Object.prototype.toString.call(channel.port1),
    portCtor: channel.port1.constructor && channel.port1.constructor.name,
    portKeys: Object.keys(channel.port1).join(","),
    portOnmessage: channel.port1.onmessage,
    channelKeys: Object.keys(channel).join(","),
    stablePortAccessor: channel.port1 === originalPort1 && channel.port2 === originalPort2,
    channelPortSpoofed: channel.port1 === channel.__moliMessageChannelPort1 ||
      channel.port2 === channel.__moliMessageChannelPort2
  });
})()
"#,
        )
        .expect("MessagePort surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructorResult":"TypeError:true","standaloneTag":"[object MessagePort]","standaloneCtor":"MessagePort","standaloneProtoCtor":"MessagePort","standaloneKeys":"","messagePortMethods":["postMessage:function:postMessage:1:true:true:true","start:function:start:0:true:true:true","close:function:close:0:true:true:true","addEventListener:function:addEventListener:2:true:true:true","removeEventListener:function:removeEventListener:2:true:true:true"],"messagePortAccessors":["onmessage:function:get onmessage:0:true:function:true","onmessageerror:function:get onmessageerror:0:true:function:true","onclose:function:get onclose:0:true:function:true"],"messageChannelAccessors":["port1:function:get port1:0:true:undefined:true","port2:function:get port2:0:true:undefined:true"],"messagePortOwnSlots":[],"messageChannelOwnSlots":[],"standaloneOnmessage":"function","standaloneOnmessageIsSpoof":false,"standaloneOnmessageerror":"function","standaloneOnmessageerrorIsSpoof":false,"standaloneOnclose":"function","standaloneOncloseIsSpoof":false,"portTag":"[object MessagePort]","portCtor":"MessagePort","portKeys":"","portOnmessage":null,"channelKeys":"","stablePortAccessor":true,"channelPortSpoofed":false}"#
    );
}

#[tokio::test]
async fn window_post_message_structured_clones_data_and_ports() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://message-clone.test/", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const original = [];
  const payload = [original, original];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      sharedReference: event.data[0] === event.data[1],
      notOriginal: event.data[0] !== original,
      portCount: event.ports.length,
      portType: Object.prototype.toString.call(event.ports[0])
    });
  };
  postMessage(payload, "*", [channel.port1, channel.port2]);
  return __messageEvents.length;
})()
"#,
        )
        .expect("window postMessage clone setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain cloned window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("cloned window message should evaluate"),
        r#"[{"sharedReference":true,"notOriginal":true,"portCount":2,"portType":"[object MessagePort]"}]"#
    );
}

#[tokio::test]
async fn window_post_message_options_transfer_list_transfers_ports() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-options-transfer.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageEvents = [];
  const original = [];
  const payload = [original, original];
  const channel = new MessageChannel();
  onmessage = event => {
    __messageEvents.push({
      sharedReference: event.data[0] === event.data[1],
      notOriginal: event.data[0] !== original,
      portCount: event.ports.length,
      portType: Object.prototype.toString.call(event.ports[0])
    });
  };
  postMessage(payload, {targetOrigin: "*", transfer: [channel.port1, channel.port2]});
  return String(__messageEvents.length);
})()
"#,
        )
        .expect("window postMessage options transfer setup should evaluate");

    assert_eq!(result, "0");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await
        .expect("wait driver should drain options-transfer window message");
    assert_eq!(
        vm.eval("JSON.stringify(__messageEvents)")
            .expect("options-transfer window message should evaluate"),
        r#"[{"sharedReference":true,"notOriginal":true,"portCount":2,"portType":"[object MessagePort]"}]"#
    );
}

#[tokio::test]
async fn message_port_dispatch_uses_lightweight_popup_owner_scope() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://message-port-popup-owner.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__messagePortPopupOwnerMessages = [];
  onmessage = event => {
    __messagePortPopupOwnerMessages.push("window:" + event.data + ":" + event.origin);
  };

  const popup = open("https://message-port-popup-child.test/page.html");
  popup.onmessage = event => {
    if (event.data !== "setup") {
      return;
    }
    const channel = new MessageChannel();
    channel.port2.onmessage = () => {
      event.source.postMessage("popup-port-handler-ran", event.origin);
    };
    channel.port1.postMessage("start");
  };
  popup.postMessage("setup", "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup-owned MessagePort setup should evaluate");
    assert_eq!(setup, "scheduled");

    for _ in 0..12 {
        if vm
            .eval(
                r#"String(globalThis.__messagePortPopupOwnerMessages.some(
  message => message.startsWith("window:popup-port-handler-ran:")
))"#,
            )
            .expect("popup-owned MessagePort completion should evaluate")
            == "true"
        {
            break;
        }
        let _ = vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance popup-owned MessagePort");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__messagePortPopupOwnerMessages)")
            .expect("popup-owned MessagePort messages should evaluate"),
        r#"["window:popup-port-handler-ran:https://message-port-popup-child.test"]"#
    );
}

#[tokio::test]
async fn window_message_handler_broadcast_channel_stays_in_lightweight_popup_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_broadcast_channel_page_test_vm_with_loader(
        "https://window-message-popup-broadcast-channel-owner.test/",
        &loader,
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__windowMessagePopupBroadcastChannelMessages = [];
  const topChannel = new BroadcastChannel("window-message-popup-broadcast-channel-owner");
  topChannel.onmessage = event => {
    __windowMessagePopupBroadcastChannelMessages.push("top-bc:" + event.data + ":" + event.origin);
  };
  onmessage = event => {
    __windowMessagePopupBroadcastChannelMessages.push("window:" + event.data + ":" + event.origin);
  };

  const popup = open("https://window-message-popup-broadcast-channel-child.test/page.html");
  popup.onmessage = event => {
    if (event.data !== "probe") {
      return;
    }
    const popupChannel = new BroadcastChannel("window-message-popup-broadcast-channel-owner");
    popupChannel.postMessage("from-popup-window-message");
    event.source.postMessage("done", event.origin);
  };
  popup.postMessage("probe", "*");
  return "scheduled";
})()
"#,
        )
        .expect("popup window-message BroadcastChannel owner workflow should schedule");
    assert_eq!(setup, "scheduled");

    for _ in 0..12 {
        if vm
            .eval(
                r#"String(globalThis.__windowMessagePopupBroadcastChannelMessages.some(
  message => message.startsWith("window:")
))"#,
            )
            .expect("popup window-message BroadcastChannel completion should evaluate")
            == "true"
        {
            break;
        }
        let outcome = vm
            .run_one_window_message_executor_turn(&loader)
            .await
            .expect("typed popup Window.postMessage turn should apply");
        assert!(
            outcome,
            "popup window-message workflow should retain a scheduler-visible task"
        );
    }

    vm.apply_pending_broadcast_channel_delivery_tasks(&loader, 4)
        .await
        .expect("any admitted BroadcastChannel executor tasks should apply");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__windowMessagePopupBroadcastChannelMessages)")
            .expect("popup window-message BroadcastChannel messages should evaluate"),
        r#"["window:done:https://window-message-popup-broadcast-channel-child.test"]"#
    );
}

#[test]
fn window_post_message_requires_message_argument() {
    let mut vm = new_storage_test_vm("https://window-post-message-required-argument.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = args => {
    try {
      postMessage(...args);
      return "ok";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  return [outcome([]), outcome([undefined])].join("|");
})()
"#,
        )
        .expect("Window.postMessage required argument conversion should evaluate");

    assert_eq!(result, "TypeError:true|ok");
}

#[test]
fn body_onmessageerror_content_attribute_reflects_window_handler() {
    let mut vm = new_storage_test_vm("https://body-window-messageerror-handler.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__messageErrorRuns = 0;
  const html = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || html.appendChild(document.createElement("body"));
  const initial = [body.onmessageerror, window.onmessageerror];
  body.setAttribute(
    "onmessageerror",
    "globalThis.__messageErrorRuns += 1;"
  );
  const compiled = body.onmessageerror;
  compiled();
  const reflected = window.onmessageerror === compiled;

  body.removeAttribute("onmessageerror");
  const removed = body.onmessageerror === null && window.onmessageerror === null;
  body.setAttribute(
    "onmessageerror",
    "globalThis.__messageErrorRuns += 10;"
  );
  window.dispatchEvent(new Event("messageerror"));

  return JSON.stringify({
    initial: initial.map(value => value === null),
    compiledType: typeof compiled,
    reflected,
    removed,
    runs: globalThis.__messageErrorRuns
  });
})()
"#,
        )
        .expect("body WindowEventHandlers onmessageerror probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[true,true],"compiledType":"function","reflected":true,"removed":true,"runs":11}"#
    );
}

#[test]
fn window_post_message_legacy_transfer_argument_must_be_iterable() {
    let mut vm = new_storage_test_vm("https://message-transfer-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = value => {
    try {
      postMessage("", "*", value);
      return "no-throw";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  const channel = new MessageChannel();
  channel[0] = channel.port1;
  channel[1] = channel.port2;
  channel.length = 2;
  return [
    probe(null),
    probe(undefined),
    probe(1),
    probe({length: 1}),
    probe(channel)
  ].join("|");
})()
"#,
        )
        .expect("window postMessage transfer validation should evaluate");

    assert_eq!(
        result,
        "TypeError:true|no-throw|TypeError:true|TypeError:true|TypeError:true"
    );
}

#[test]
fn window_post_message_options_transfer_null_throws_type_error() {
    let mut vm = new_storage_test_vm("https://window-post-message-options.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const outcome = options => {
    try {
      postMessage("payload", options);
      return "ok";
    } catch (error) {
      return `${error.name}:${error instanceof TypeError}`;
    }
  };
  const arrayFrom = Array.from;
  Array.from = () => { throw new Error("postMessage transfer must not use Array.from"); };
  const iterableOutcome = outcome({
    transfer: {
      *[Symbol.iterator]() {}
    }
  });
  Array.from = arrayFrom;
  return [
    outcome({}),
    outcome({ transfer: undefined }),
    outcome({ transfer: null }),
    iterableOutcome
  ].join("|");
})()
"#,
        )
        .expect("Window.postMessage options transfer conversion should evaluate");

    assert_eq!(result, "ok|ok|TypeError:true|ok");
}

#[test]
fn window_post_message_wasm_memory_buffer_transfer_throws_type_error() {
    let mut vm = new_storage_test_vm("https://wasm-memory-transfer-validation.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const buffer = new WebAssembly.Memory({ initial: 1 }).buffer;
  try {
    postMessage("payload", "*", [buffer]);
    return "no-throw";
  } catch (error) {
    return `${error.name}:${error instanceof TypeError}:${error instanceof DOMException}`;
  }
})()
"#,
        )
        .expect("wasm memory buffer transfer validation should evaluate");

    assert_eq!(result, "TypeError:true:false");
}
#[tokio::test]
async fn queued_selectionchange_ignores_page_tampered_document_dispatch_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-dispatch-guard.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              globalThis.__selectionTamperedDispatch = "no";
              globalThis.__selectionChangeFired = "no";
              document.addEventListener("selectionchange", () => {
                globalThis.__selectionChangeFired = "yes";
              });
              document.dispatchEvent = () => {
                globalThis.__selectionTamperedDispatch = "yes";
                throw new Error("host must not call document.dispatchEvent");
              };
              getSelection().collapse(text, 1);
              return `${globalThis.__selectionTamperedDispatch}|${globalThis.__selectionChangeFired}|${getSelection().anchorOffset}`;
            })()
            "#,
        )
        .expect("queued selectionchange setup should evaluate");

    assert_eq!(result, "no|no|1");
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance queued selectionchange dispatch")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionTamperedDispatch}|${globalThis.__selectionChangeFired}|${getSelection().anchorOffset}`")
            .expect("queued selectionchange dispatch result should evaluate"),
        "no|yes|1"
    );
}
#[tokio::test]
async fn selectionchange_mutation_inside_listener_queues_a_later_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-reentrant-task.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              globalThis.__selectionReentrantCount = 0;
              globalThis.__selectionReentrantLog = [];
              document.addEventListener("selectionchange", () => {
                if (globalThis.__selectionReentrantCount === 0) {
                  selection.setPosition(text, 2);
                  selection.setPosition(text, 0);
                }
                globalThis.__selectionReentrantCount += 1;
                globalThis.__selectionReentrantLog.push(
                  `event:${globalThis.__selectionReentrantCount}:${selection.anchorOffset}`
                );
              });
              selection.setPosition(text, 1);
              return `${globalThis.__selectionReentrantLog.join("|")}|count:${globalThis.__selectionReentrantCount}`;
            })()
            "#,
        )
        .expect("reentrant selectionchange setup should evaluate");

    assert_eq!(result, "|count:0");
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance first selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionReentrantLog.join('|')}|count:${globalThis.__selectionReentrantCount}`")
            .expect("first reentrant selectionchange result should evaluate"),
        "event:1:0|count:1"
    );
    assert!(
        vm.run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance reentrant selectionchange task")
    );
    assert_eq!(
        vm.eval("`${globalThis.__selectionReentrantLog.join('|')}|count:${globalThis.__selectionReentrantCount}`")
            .expect("second reentrant selectionchange result should evaluate"),
        "event:1:0|event:2:0|count:2"
    );
}
#[tokio::test]
async fn selectionchange_without_document_listener_does_not_schedule_host_task() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://selectionchange-no-listener.test/",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              getSelection().setBaseAndExtent(text, 0, text, 2);
              return `${getSelection().anchorOffset}|${getSelection().focusOffset}`;
            })()
            "#,
        )
        .expect("selection mutation without listeners should evaluate");

    assert_eq!(result, "0|2");
    assert!(
        !vm.run_one_user_interaction_executor_turn(&loader)
            .await
            .expect("exact UserInteraction source should remain empty"),
        "a selection mutation without a Document listener must not enqueue a selectionchange task"
    );
}
#[test]
fn selection_range_methods_require_actual_range_objects() {
    let mut vm = new_storage_test_vm("https://selection-range-brand.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);
              const selection = getSelection();
              const range = document.createRange();
              range.setStart(text, 1);
              range.setEnd(text, 3);
              const equivalentRange = document.createRange();
              equivalentRange.setStart(text, 1);
              equivalentRange.setEnd(text, 3);
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return error && error.name;
                }
              };
              const addSelection = probe(() => selection.addRange(selection));
              const addPlainObject = probe(() => selection.addRange({}));
              selection.addRange(range);
              const removeSelection = probe(() => selection.removeRange(selection));
              const removeEquivalent = probe(() => selection.removeRange(equivalentRange));
              const rangeStillSelected = selection.rangeCount;
              selection.removeRange(range);
              return [
                addSelection,
                addPlainObject,
                removeSelection,
                removeEquivalent,
                rangeStillSelected,
                selection.rangeCount,
                selection.anchorNode === null,
                selection.focusNode === null
              ].join("|");
            })()
            "#,
        )
        .expect("Selection Range argument brand checks should evaluate");

    assert_eq!(
        result,
        "TypeError|TypeError|TypeError|NotFoundError|1|0|true|true"
    );
}
#[test]
fn selection_empty_state_operations_throw_invalid_state_error() {
    let mut vm = new_storage_test_vm("https://selection-empty-state.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const selection = getSelection();
              selection.removeAllRanges();
              const div = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(div);
              const probe = callback => {
                try {
                  callback();
                  return "no-throw";
                } catch (error) {
                  return `${error && error.name}:${error && error.code}:${error instanceof DOMException}`;
                }
              };
              return [
                probe(() => selection.collapseToStart()),
                probe(() => selection.collapseToEnd()),
                probe(() => selection.extend(div))
              ].join("|");
            })()
            "#,
        )
        .expect("empty Selection operation checks should evaluate");

    assert_eq!(
        result,
        "InvalidStateError:11:true|InvalidStateError:11:true|InvalidStateError:11:true"
    );
}
#[test]
fn selection_add_range_ignores_detached_and_foreign_ranges() {
    let mut vm = new_storage_test_vm("https://selection-add-range-root.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const selection = getSelection();
              selection.removeAllRanges();

              const host = document.createElement("div");
              const text = document.createTextNode("abcd");
              host.appendChild(text);
              (document.body || document.documentElement || document).appendChild(host);

              const detachedText = document.createTextNode("detached");
              const detachedRange = document.createRange();
              detachedRange.setStart(detachedText, 1);
              detachedRange.setEnd(detachedText, 3);
              selection.addRange(detachedRange);
              const detachedState = [
                selection.rangeCount,
                selection.anchorNode === null,
                detachedRange.startContainer === detachedText,
                detachedRange.startOffset,
                detachedRange.endContainer === detachedText,
                detachedRange.endOffset
              ].join(":");

              const foreignDocument = document.implementation.createHTMLDocument("");
              const foreignText = foreignDocument.createTextNode("foreign");
              foreignDocument.body.appendChild(foreignText);
              const foreignRange = foreignDocument.createRange();
              foreignRange.setStart(foreignText, 1);
              foreignRange.setEnd(foreignText, 4);
              selection.addRange(foreignRange);
              const foreignState = [
                selection.rangeCount,
                selection.anchorNode === null,
                foreignRange.startContainer === foreignText,
                foreignRange.startOffset,
                foreignRange.endContainer === foreignText,
                foreignRange.endOffset
              ].join(":");

              const selectedRange = document.createRange();
              selectedRange.setStart(text, 1);
              selectedRange.setEnd(text, 3);
              selection.addRange(selectedRange);
              const selectedBefore = selection.getRangeAt(0);
              selection.addRange(detachedRange);
              const secondDetachedState = [
                selection.rangeCount,
                selection.getRangeAt(0) === selectedBefore,
                selection.anchorNode === text,
                selection.anchorOffset,
                selection.focusNode === text,
                selection.focusOffset
              ].join(":");

              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childSelection = iframe.contentWindow.getSelection();
              const childOriginalSelection = childSelection;
              const childText = iframe.contentDocument.createTextNode("child");
              iframe.contentDocument.body.appendChild(childText);
              const childRange = iframe.contentDocument.createRange();
              childRange.selectNodeContents(iframe.contentDocument.body);
              childSelection.removeAllRanges();
              childSelection.addRange(childRange);
              const childSelectedRange = (() => {
                try {
                  return childSelection.getRangeAt(0);
                } catch (error) {
                  return error && error.name;
                }
              })();
              const childState = [
                iframe.contentWindow.getSelection() === childOriginalSelection,
                childSelection.rangeCount,
                childSelectedRange === childRange ? "same" : String(childSelectedRange),
                childSelection.anchorNode === iframe.contentDocument.body,
                childSelection.anchorOffset,
                childSelection.focusNode === iframe.contentDocument.body,
                childSelection.focusOffset
              ].join(":");

              return `${detachedState}|${foreignState}|${secondDetachedState}|${childState}`;
            })()
            "#,
        )
        .expect("Selection.addRange root checks should evaluate");

    assert_eq!(
        result,
        "0:true:true:1:true:3|0:true:true:1:true:4|1:true:true:1:true:3|true:1:same:true:0:true:1"
    );
}
#[test]
fn window_selection_rejects_child_document_shadow_ranges() {
    let mut vm = new_storage_test_vm("https://selection-child-shadow-range.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const childDocument = frame.contentWindow.document;
              const parentSelection = window.getSelection();
              parentSelection.removeAllRanges();

              const host = childDocument.createElement("div");
              childDocument.body.appendChild(host);
              const shadow = host.attachShadow({ mode: "open" });
              const span = childDocument.createElement("span");
              span.textContent = "Some text";
              shadow.appendChild(span);
              const shadowRange = childDocument.createRange();
              shadowRange.setStart(span.firstChild, 0);
              shadowRange.setEnd(span.firstChild, 3);
              parentSelection.addRange(shadowRange);
              const shadowState = [
                parentSelection.rangeCount,
                parentSelection.toString()
              ].join(":");

              const slottedHost = childDocument.createElement("div");
              childDocument.body.appendChild(slottedHost);
              const slottedSpan = childDocument.createElement("span");
              slottedSpan.textContent = "More text";
              slottedSpan.slot = "span";
              slottedHost.appendChild(slottedSpan);
              const slottedShadow = slottedHost.attachShadow({ mode: "open" });
              slottedShadow.innerHTML = '<slot name="span"></slot>';
              const slottedRange = childDocument.createRange();
              slottedRange.setStart(slottedSpan.firstChild, 0);
              slottedRange.setEnd(slottedSpan.firstChild, 4);
              parentSelection.addRange(slottedRange);
              const slottedState = [
                parentSelection.rangeCount,
                parentSelection.toString()
              ].join(":");

              const childSelection = frame.contentWindow.getSelection();
              childSelection.removeAllRanges();
              childSelection.addRange(shadowRange);
              const childState = [
                childSelection.rangeCount,
                childSelection.toString()
              ].join(":");

              return `${shadowState}|${slottedState}|${childState}`;
            })()
            "#,
        )
        .expect("window Selection should reject child document shadow ranges");

    assert_eq!(result, "0:|0:|1:Som");
}

#[test]
fn selection_get_composed_ranges_rescopes_shadow_boundaries() {
    let mut vm = new_storage_test_vm("https://selection-composed-ranges.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const start = document.createElement("span");
              start.id = "start";
              start.textContent = "Start";
              const host = document.createElement("div");
              const end = document.createElement("span");
              end.id = "end";
              end.textContent = "End";
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);
              container.appendChild(end);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner1">Inner1</span><span id="inner2">Inner2</span>';
              const inner2 = root.getElementById("inner2");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 3, inner2.firstChild, 3);

              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const rescoped = selection.getComposedRanges()[0];
              return [
                typeof selection.getComposedRanges,
                selection.rangeCount,
                selection.isCollapsed,
                exposed instanceof StaticRange,
                exposed.startContainer === start.firstChild,
                exposed.startOffset,
                exposed.endContainer === inner2.firstChild,
                exposed.endOffset,
                rescoped.startContainer === start.firstChild,
                rescoped.startOffset,
                rescoped.endContainer === host.parentNode,
                rescoped.endOffset,
                Array.isArray(selection.getComposedRanges())
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges shadow rescope checks should evaluate");

    assert_eq!(
        result,
        "function|1|true|true|true|3|true|3|true|3|true|2|true"
    );
}

#[test]
fn selection_get_composed_ranges_static_range_init_ignores_prototype_setters() {
    let mut vm = new_storage_test_vm("https://selection-composed-ranges-init.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const setterHits = [];
              for (const name of ["startContainer", "startOffset", "endContainer", "endOffset"]) {
                Object.defineProperty(Object.prototype, name, {
                  configurable: true,
                  get() { return undefined; },
                  set(value) {
                    const receiverKind = this instanceof StaticRange ? "range" : "plain";
                    setterHits.push(`${receiverKind}:${name}`);
                    Object.defineProperty(this, name, {
                      configurable: true,
                      enumerable: true,
                      writable: true,
                      value
                    });
                  }
                });
              }

              const start = document.createElement("span");
              start.textContent = "Start";
              const host = document.createElement("div");
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);
              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner">Inner</span>';
              const inner = root.getElementById("inner");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 2, inner.firstChild, 4);

              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const rescoped = selection.getComposedRanges()[0];
              return JSON.stringify({
                exposed: [
                  exposed instanceof StaticRange,
                  exposed.startContainer === start.firstChild,
                  exposed.startOffset,
                  exposed.endContainer === inner.firstChild,
                  exposed.endOffset
                ],
                rescoped: [
                  rescoped instanceof StaticRange,
                  rescoped.startContainer === start.firstChild,
                  rescoped.startOffset,
                  rescoped.endContainer === host.parentNode,
                  rescoped.endOffset
                ],
                plainSetterHits: setterHits.filter(hit => hit.startsWith("plain:"))
              });
            })()
            "#,
        )
        .expect("Selection.getComposedRanges StaticRange init setter probe should evaluate");

    assert_eq!(
        result,
        r#"{"exposed":[true,true,2,true,4],"rescoped":[true,true,2,true,2],"plainSetterHits":[]}"#
    );
}

#[test]
fn selection_get_range_at_returns_shadow_collapsed_range_for_cross_root_selection() {
    let mut vm = new_storage_test_vm("https://selection-cross-root-range-at.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const start = document.createElement("span");
              start.textContent = "Start";
              const host = document.createElement("div");
              const container = document.body || document.documentElement || document;
              container.appendChild(start);
              container.appendChild(host);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = '<span id="inner1">Inner1</span><span id="inner2">Inner2</span>';
              const inner1 = root.getElementById("inner1");
              const inner2 = root.getElementById("inner2");
              const selection = getSelection();
              selection.removeAllRanges();
              selection.setBaseAndExtent(start.firstChild, 3, inner2.firstChild, 3);

              const composed = selection.getComposedRanges({ shadowRoots: [root] })[0];
              const range = selection.getRangeAt(0);
              return [
                selection.isCollapsed,
                selection.anchorNode === inner2.firstChild,
                selection.anchorOffset,
                composed.startContainer === start.firstChild,
                composed.startOffset,
                composed.endContainer === inner2.firstChild,
                composed.endOffset,
                range.collapsed,
                range.startContainer === inner2.firstChild,
                range.startOffset,
                range.endContainer === inner2.firstChild,
                range.endOffset,
                range.isPointInRange(inner1, 0),
                range.comparePoint(inner1, 0)
              ].join("|");
            })()
            "#,
        )
        .expect("cross-root Selection.getRangeAt probe should evaluate");

    assert_eq!(
        result,
        "true|true|3|true|3|true|3|true|true|3|true|3|false|-1"
    );
}

#[test]
fn selection_cross_root_set_base_and_extent_collapses_legacy_to_focus() {
    let mut vm = new_storage_test_vm("https://selection-cross-root-collapse.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.createElement("div");
              const host = document.createElement("div");
              const outText = document.createElement("p");
              outText.textContent = "Outside shadow tree.";
              const host2 = document.createElement("div");
              container.appendChild(host);
              container.appendChild(outText);
              container.appendChild(host2);
              (document.body || document.documentElement || document).appendChild(container);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = "<p>Inside shadow tree 1.</p>";
              const root2 = host2.attachShadow({ mode: "open" });
              root2.innerHTML = "<p>Inside shadow tree 2.</p>";
              const inText = root.querySelector("p").firstChild;
              const inText2 = root2.querySelector("p").firstChild;
              const out = outText.firstChild;
              const selection = getSelection();
              const roots = { shadowRoots: [root, root2] };
              const records = [];
              const label = node =>
                node === inText ? "in1" :
                node === inText2 ? "in2" :
                node === out ? "out" :
                node === null ? "null" : "other";
              const state = name => {
                const composed = selection.getComposedRanges(roots)[0];
                records.push([
                  name,
                  selection.isCollapsed,
                  label(selection.anchorNode),
                  selection.anchorOffset,
                  label(selection.focusNode),
                  selection.focusOffset,
                  composed.collapsed,
                  label(composed.startContainer),
                  composed.startOffset,
                  label(composed.endContainer),
                  composed.endOffset
                ].join(":"));
              };

              selection.setBaseAndExtent(inText, 0, out, 1);
              state("shadow-to-light");
              selection.setBaseAndExtent(inText, 0, inText2, 1);
              state("shadow-to-shadow");
              selection.setBaseAndExtent(out, 1, inText, 0);
              state("light-to-shadow-backward");
              return records.join("|");
            })()
            "#,
        )
        .expect("cross-root Selection.setBaseAndExtent collapse probe should evaluate");

    assert_eq!(
        result,
        "shadow-to-light:true:out:1:out:1:false:in1:0:out:1|shadow-to-shadow:true:in2:1:in2:1:false:in1:0:in2:1|light-to-shadow-backward:true:out:1:out:1:false:in1:0:out:1"
    );
}

#[test]
fn selection_set_base_and_extent_to_earlier_shadow_root_collapses_to_anchor() {
    let mut vm = new_storage_test_vm("https://selection-cross-root-backward-collapse.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const host = document.createElement("div");
              const anchor = document.createElement("div");
              const parent = document.body || document.documentElement || document;
              parent.appendChild(host);
              parent.appendChild(anchor);
              const root = host.attachShadow({ mode: "open" });
              root.textContent = "A";

              const selection = getSelection();
              selection.setBaseAndExtent(anchor, 0, root, 0);

              return [
                selection.anchorNode === anchor,
                selection.anchorOffset,
                selection.focusNode === anchor,
                selection.focusOffset,
                selection.isCollapsed,
                selection.getRangeAt(0).startContainer === anchor,
                selection.getRangeAt(0).startOffset,
                selection.getRangeAt(0).endContainer === anchor,
                selection.getRangeAt(0).endOffset
              ].join(":");
            })()
            "#,
        )
        .expect("cross-root backward Selection.setBaseAndExtent probe should evaluate");

    assert_eq!(result, "true:0:true:0:true:true:0:true:0");
}

#[test]
fn selection_set_base_and_extent_orders_descendant_boundary_before_ancestor_after_child() {
    let mut vm = new_storage_test_vm("https://selection-ancestor-boundary-order.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = document.createElement("div");
              parent.innerHTML = "<span><b></b></span>";
              (document.body || document.documentElement || document).appendChild(parent);
              const span = parent.firstChild;
              const child = span.firstChild;
              const selection = getSelection();
              const record = label => {
                const range = selection.getRangeAt(0);
                return [
                  label,
                  range.startContainer === child,
                  range.startOffset,
                  range.endContainer === span,
                  range.endOffset,
                ].join(":");
              };
              selection.setBaseAndExtent(child, 0, span, 1);
              const forward = record("forward");
              selection.setBaseAndExtent(span, 1, child, 0);
              const backward = record("backward");
              return `${forward}|${backward}`;
            })()
            "#,
        )
        .expect("Selection.setBaseAndExtent descendant/ancestor order probe should evaluate");

    assert_eq!(result, "forward:true:0:true:1|backward:true:0:true:1");
}

#[test]
fn selection_get_composed_ranges_orders_slotted_boundary_like_chromium() {
    let mut vm = new_storage_test_vm("https://selection-composed-ranges-slot.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.createElement("div");
              const host = document.createElement("div");
              host.textContent = "Second";
              container.appendChild(host);
              (document.body || document.documentElement || document).appendChild(container);

              const root = host.attachShadow({ mode: "open" });
              root.innerHTML = 'First <slot></slot> Third';
              const second = host.firstChild;
              const third = root.querySelector("slot").nextSibling;
              const selection = getSelection();

              selection.removeAllRanges();
              selection.setBaseAndExtent(second, 3, third, 4);
              const rescoped = selection.getComposedRanges()[0];
              const exposed = selection.getComposedRanges({ shadowRoots: [root] })[0];

              selection.setBaseAndExtent(third, 4, second, 3);
              const reversed = selection.getComposedRanges({ shadowRoots: [root] })[0];

              return [
                selection.isCollapsed,
                selection.anchorNode === second,
                selection.anchorOffset,
                selection.focusNode === second,
                selection.focusOffset,
                rescoped.startContainer === container,
                rescoped.startOffset,
                rescoped.endContainer === second,
                rescoped.endOffset,
                exposed.startContainer === third,
                exposed.startOffset,
                exposed.endContainer === second,
                exposed.endOffset,
                reversed.startContainer === third,
                reversed.startOffset,
                reversed.endContainer === second,
                reversed.endOffset
              ].join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges slot ordering checks should evaluate");

    assert_eq!(
        result,
        "true|true|3|true|3|true|0|true|3|true|4|true|3|true|4|true|3"
    );
}

#[test]
fn selection_get_composed_ranges_tracks_associated_range_updates() {
    let mut vm = new_storage_test_vm("https://selection-composed-range-update.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const container = document.body || document.documentElement || document;
              const light = document.createElement("div");
              light.textContent = "Start outside shadow DOM";
              const outerHost = document.createElement("div");
              outerHost.textContent = "outerHost";
              const lightEnd = document.createElement("div");
              lightEnd.textContent = "End outside shadow DOM";
              container.appendChild(light);
              container.appendChild(outerHost);
              container.appendChild(lightEnd);

              const outerRoot = outerHost.attachShadow({ mode: "open" });
              outerRoot.appendChild(document.createElement("slot"));
              const innerHost = document.createElement("div");
              innerHost.textContent = "innerHost";
              outerRoot.appendChild(innerHost);
              const innerRoot = innerHost.attachShadow({ mode: "open" });
              innerRoot.appendChild(document.createElement("slot"));

              const selection = getSelection();
              const lightText = light.firstChild;
              const lightEndText = lightEnd.firstChild;
              const innerText = innerHost.firstChild;
              const roots = { shadowRoots: [outerRoot, innerRoot] };
              const records = [];
              const thrown = (fn) => {
                try { fn(); return false; } catch (_) { return true; }
              };
              const state = (label, liveRange) => {
                const ranges = selection.getComposedRanges(roots);
                const composed = ranges[0];
                records.push([
                  label,
                  liveRange.collapsed,
                  liveRange.startContainer === innerText ? "inner" :
                    liveRange.startContainer === lightText ? "light" :
                    liveRange.startContainer === lightEndText ? "end" :
                    liveRange.startContainer === outerRoot ? "outerRoot" : "other",
                  liveRange.startOffset,
                  selection.isCollapsed,
                  selection.anchorNode === innerText ? "inner" :
                    selection.anchorNode === lightText ? "light" :
                    selection.anchorNode === lightEndText ? "end" :
                    selection.anchorNode === outerRoot ? "outerRoot" :
                    selection.anchorNode === null ? "null" : "other",
                  selection.anchorOffset,
                  ranges.length,
                  composed ? (
                    (composed.startContainer === innerText ? "inner" :
                     composed.startContainer === lightText ? "light" :
                     composed.startContainer === lightEndText ? "end" :
                     composed.startContainer === outerRoot ? "outerRoot" :
                     composed.startContainer === document ? "document" : "other") +
                    ":" + composed.startOffset + ">" +
                    (composed.endContainer === innerText ? "inner" :
                     composed.endContainer === lightText ? "light" :
                     composed.endContainer === lightEndText ? "end" :
                     composed.endContainer === outerRoot ? "outerRoot" :
                     composed.endContainer === document ? "document" : "other") +
                    ":" + composed.endOffset
                  ) : "none"
                ].join(":"));
              };

              selection.setBaseAndExtent(lightText, 10, innerText, 5);
              records.push("cross-getRangeAt:" + (() => {
                const range = selection.getRangeAt(0);
                return [
                  range.collapsed,
                  range.startContainer === innerText ? "inner" :
                    range.startContainer === lightText ? "light" :
                    range.startContainer === lightEndText ? "end" :
                    range.startContainer === outerRoot ? "outerRoot" : "other",
                  range.startOffset,
                  range.endContainer === innerText ? "inner" :
                    range.endContainer === lightText ? "light" :
                    range.endContainer === lightEndText ? "end" :
                    range.endContainer === outerRoot ? "outerRoot" : "other",
                  range.endOffset
                ].join(":");
              })());

              selection.setBaseAndExtent(lightText, 10, lightText, 20);
              let liveRange = selection.getRangeAt(0);
              liveRange.setEnd(innerText, 5);
              state("setEnd-cross-keeps-composed-start", liveRange);

              selection.setBaseAndExtent(lightEndText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.setStart(innerText, 5);
              state("setStart-cross-keeps-composed-end", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.setStart(innerText, 5);
              state("setStart-cross-collapses-composed", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.selectNode(innerHost);
              state("selectNode-syncs-all", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              liveRange.collapse();
              state("collapse-syncs-all", liveRange);

              selection.removeAllRanges();
              liveRange = document.createRange();
              selection.addRange(liveRange);
              liveRange.setEnd(innerText, 5);
              state("addRange-before-setEnd", liveRange);
              liveRange.setStart(lightText, 10);
              state("setStart-after-addRange-setEnd", liveRange);

              selection.setBaseAndExtent(lightText, 10, lightEndText, 20);
              liveRange = selection.getRangeAt(0);
              const detached = document.createElement("span");
              liveRange.setStart(detached, 0);
              records.push([
                "detached-clears-selection",
                liveRange.collapsed,
                selection.rangeCount,
                selection.anchorNode === null,
                selection.getComposedRanges(roots).length,
                thrown(() => selection.getRangeAt(0))
              ].join(":"));

              return records.join("|");
            })()
            "#,
        )
        .expect("Selection.getComposedRanges associated Range updates should evaluate");

    assert_eq!(
        result,
        "cross-getRangeAt:true:inner:5:inner:5|setEnd-cross-keeps-composed-start:true:inner:5:true:inner:5:1:light:10>inner:5|setStart-cross-keeps-composed-end:true:inner:5:true:inner:5:1:inner:5>end:20|setStart-cross-collapses-composed:true:inner:5:true:inner:5:1:inner:5>inner:5|selectNode-syncs-all:false:outerRoot:1:false:outerRoot:1:1:outerRoot:1>outerRoot:2|collapse-syncs-all:true:end:20:true:end:20:1:end:20>end:20|addRange-before-setEnd:true:inner:5:true:inner:5:1:document:0>inner:5|setStart-after-addRange-setEnd:true:light:10:true:light:10:1:light:10>inner:5|detached-clears-selection:true:0:true:0:true"
    );
}

#[test]
fn selection_get_composed_ranges_rescopes_after_dom_removals() {
    let mut vm = new_storage_test_vm("https://selection-composed-dom-removal.test/");

    let result = vm
        .eval(
            r##"
            (() => {
              const sel = getSelection();
              const container = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(container);
              const failures = [];
              const expectBoundary = (name, range, side, node, offset) => {
                const actualNode = range[`${side}Container`];
                const actualOffset = range[`${side}Offset`];
                if (actualNode !== node || actualOffset !== offset) {
                  failures.push(`${name}:${side}:${actualNode?.nodeName}:${actualOffset}`);
                }
              };
              const composed = (...roots) =>
                sel.getComposedRanges({ shadowRoots: roots })[0];
              const reset = (html) => {
                sel.removeAllRanges();
                container.innerHTML = html;
              };

              for (const mode of ["open", "closed"]) {
                reset('a<div id="host"></div>b');
                let host = container.querySelector("#host");
                let root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 7, container, 2);
                host.remove();
                let range = composed(root);
                expectBoundary(`${mode}:host-remove`, range, "start", container, 1);
                expectBoundary(`${mode}:host-remove`, range, "end", container, 1);

                reset('<div id="wrapper">a<div id="host"></div>b</div>');
                const wrapper = container.querySelector("#wrapper");
                host = container.querySelector("#host");
                root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 4, root.firstChild, 7);
                wrapper.remove();
                range = composed(root);
                expectBoundary(`${mode}:wrapper-remove`, range, "start", container, 0);
                expectBoundary(`${mode}:wrapper-remove`, range, "end", container, 0);

                reset('<div id="hello">Hello,</div><div id="world"> World</div>');
                const hello = container.querySelector("#hello");
                const world = container.querySelector("#world");
                sel.setBaseAndExtent(hello.firstChild, 1, world.firstChild, 3);
                hello.firstChild.remove();
                range = sel.getComposedRanges()[0];
                expectBoundary(`${mode}:light-text-remove`, range, "start", hello, 0);
                expectBoundary(`${mode}:light-text-remove`, range, "end", world.firstChild, 3);

                reset('a<div id="host"></div>b');
                host = container.querySelector("#host");
                root = host.attachShadow({ mode });
                root.innerHTML = "hello, world";
                sel.setBaseAndExtent(root.firstChild, 7, container, 2);
                root.innerHTML = "";
                range = composed(root);
                expectBoundary(`${mode}:shadow-content-clear`, range, "start", root, 0);
                expectBoundary(`${mode}:shadow-content-clear`, range, "end", container, 2);

                reset('a<div id="outerhost"></div>b');
                const outerHost = container.querySelector("#outerhost");
                const outerRoot = outerHost.attachShadow({ mode });
                outerRoot.innerHTML = 'c<div id="innerHost"></div>d';
                const innerHost = outerRoot.querySelector("#innerHost");
                const innerRoot = innerHost.attachShadow({ mode });
                innerRoot.innerHTML = "hello, world";
                sel.setBaseAndExtent(container.firstChild, 0, innerRoot.firstChild, 4);
                outerHost.remove();
                range = composed(innerRoot, outerRoot);
                expectBoundary(`${mode}:outer-host-remove`, range, "start", container.firstChild, 0);
                expectBoundary(`${mode}:outer-host-remove`, range, "end", container, 1);
              }

              reset([
                '<div id=host>',
                '<div id=div1 slot=slot2>slotted content 1</div>',
                '<div id=div2 slot=slot1>slotted content 2</div>',
                '</div>'
              ].join(""));
              const host = container.querySelector("#host");
              const div1 = container.querySelector("#div1");
              const div2 = container.querySelector("#div2");
              const shadowRoot = host.attachShadow({ mode: "open" });
              shadowRoot.innerHTML = [
                '<span>before</span>',
                '<slot name=slot1></slot>',
                '<span>between</span>',
                '<slot name=slot2></slot>',
                '<span>after</span>',
              ].join("");
              sel.setBaseAndExtent(div1.firstChild, 2, div2.firstChild, 2);
              div1.remove();
              const range = composed(shadowRoot);
              expectBoundary("slot-start-remove", range, "start", host, 0);
              expectBoundary("slot-start-remove", range, "end", div2.firstChild, 2);

              return failures.join("|") || "ok";
            })()
            "##,
        )
        .expect("Selection.getComposedRanges removal rescope checks should evaluate");

    assert_eq!(result, "ok");
}

#[test]
fn document_get_selection_uses_associated_window_selection() {
    let mut vm = new_storage_test_vm("https://document-get-selection.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const topWindowSelection = window.getSelection();
              const topDocumentSelection = document.getSelection();
              const internalSlotName = "__moliWindowSelection";
              const topWindowInternalBefore = Object.getOwnPropertyNames(window)
                .includes(internalSlotName);
              window[internalSlotName] = "top-spoof";

              const htmlDocument = document.implementation.createHTMLDocument("");
              const xmlDocument = document.implementation.createDocument(null, "", null);

              const iframe = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(iframe);
              const childWindowSelection = iframe.contentWindow.getSelection();
              const childDocumentSelection = iframe.contentDocument.getSelection();
              const childWindowInternalBefore = Object.getOwnPropertyNames(iframe.contentWindow)
                .includes(internalSlotName);
              iframe.contentWindow[internalSlotName] = "child-spoof";

              return [
                topDocumentSelection === topWindowSelection,
                topWindowSelection === window.getSelection(),
                window[internalSlotName] === "top-spoof",
                topWindowInternalBefore,
                topDocumentSelection instanceof Selection,
                Object.prototype.toString.call(topDocumentSelection),
                Object.keys(topDocumentSelection).join(","),
                Object.getOwnPropertyNames(topDocumentSelection)
                  .filter((name) => name.startsWith("__moli")).length,
                htmlDocument.defaultView === null,
                htmlDocument.getSelection() === null,
                "getSelection" in xmlDocument,
                xmlDocument.defaultView === null,
                xmlDocument.getSelection() === null,
                childWindowSelection === iframe.contentWindow.getSelection(),
                childDocumentSelection === childWindowSelection,
                iframe.contentWindow[internalSlotName] === "child-spoof",
                childWindowInternalBefore,
                childWindowSelection !== topWindowSelection,
                childWindowSelection instanceof iframe.contentWindow.Selection,
                Object.getOwnPropertyNames(childWindowSelection)
                  .filter((name) => name.startsWith("__moli")).length
              ].join("|");
            })()
            "#,
        )
        .expect("Document.getSelection checks should evaluate");

    assert_eq!(
        result,
        "true|true|true|false|true|[object Selection]||0|true|true|true|true|true|true|true|true|false|true|true|0"
    );
}
