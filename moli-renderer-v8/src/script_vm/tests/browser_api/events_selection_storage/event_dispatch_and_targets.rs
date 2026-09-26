use super::*;

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

              const outcome = callback => {
                try {
                  callback();
                  return "return";
                } catch (error) {
                  return error.name;
                }
              };

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
                prototypeError: outcome(() => HTMLElement.prototype.onsubmit),
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
        r#"{"clickGet":"function","clickSet":"function","clickConfigurable":true,"submitGet":"function","submitSet":"function","iframeSrcGet":"function","iframeSrcSet":"function","iframeSrcConfigurable":true,"divHandler":true,"otherNull":true,"formHandler":true,"prototypeError":"TypeError","iframePrototypeSrc":"","iframeSrcValue":"https://element-prototype-accessors.test/child.html","iframeSrcAttribute":"/child.html","copiedIframeSrcValue":"https://element-prototype-accessors.test/copied.html","copiedIframeSrcAttribute":"/copied.html"}"#
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
fn callable_proxy_event_listener_does_not_require_a_string_function_name() {
    let mut vm = new_storage_test_vm("https://callable-proxy-event-listener.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const target = document.createElement("div");
              const calls = [];
              const listener = new Proxy(function(event) {
                calls.push(event.type);
              }, {});

              target.addEventListener("probe", listener);
              target.dispatchEvent(new Event("probe"));
              target.removeEventListener("probe", listener);
              target.dispatchEvent(new Event("probe"));

              return `${typeof listener}:${calls.join(",")}`;
            })()
            "#,
        )
        .expect("callable Proxy listener should register and dispatch");

    assert_eq!(result, "function:probe");
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

              let fakePathThrowsTypeError = false;
              try {
                Event.prototype.composedPath.call({ __lmCp: ["fake"] });
              } catch (error) {
                fakePathThrowsTypeError = error instanceof TypeError;
              }
              return JSON.stringify({
                cleanBeforeOwn,
                cleanDuring,
                cleanAfterLength: clean.composedPath().length,
                spoofedBeforeLength: spoofedBefore.length,
                spoofedDuring,
                spoofedAfterLength: spoofed.composedPath().length,
                simpleDuring,
                simpleAfterLength: simple.composedPath().length,
                fakePathThrowsTypeError
              });
            })()
            "#,
        )
        .expect("event composed path private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"cleanBeforeOwn":false,"cleanDuring":{"ownNameVisible":false,"firstIsTarget":true,"hasPath":true},"cleanAfterLength":0,"spoofedBeforeLength":0,"spoofedDuring":{"ownValue":"spoofed-during","firstIsTarget":true,"containsSpoof":false},"spoofedAfterLength":0,"simpleDuring":{"ownValue":"spoofed-simple","firstIsSimpleTarget":true,"length":1,"containsSpoof":false},"simpleAfterLength":0,"fakePathThrowsTypeError":true}"#
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
              const rejectsReceiver = (getter, receiver) => {
                try { getter.call(receiver); }
                catch (error) { return error instanceof TypeError; }
                return false;
              };
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
                  rejectsReceiver(closeWasCleanGetter, fakeClose),
                  rejectsReceiver(closeCodeGetter, fakeClose),
                  rejectsReceiver(closeReasonGetter, fakeClose)
                ],
                submitOwnBefore,
                submitterDescriptor: getterDescriptor(SubmitEvent.prototype, "submitter"),
                submitterIsButton: submit.submitter === button,
                submitSpoofIsInput: submit.__moliSubmitEventSubmitter instanceof HTMLInputElement,
                fakeSubmitterThrowsTypeError: rejectsReceiver(submitterGetter, fakeSubmit),
                formDataOwnBefore,
                formDataDescriptor: getterDescriptor(FormDataEvent.prototype, "formData"),
                formDataIsReal: formDataEvent.formData === formData,
                formDataSpoofIsSpoofed: formDataEvent.__moliFormDataEventFormData === spoofedFormData,
                fakeFormDataThrowsTypeError: rejectsReceiver(formDataGetter, fakeFormDataEvent)
              });
            })()
            "#,
        )
        .expect("event subclass private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"closeDescriptors":["wasClean:function:get wasClean:0:undefined:true:true","code:function:get code:0:undefined:true:true","reason:function:get reason:0:undefined:true:true"],"closeOwnBefore":[],"closeValues":[true,1000,"done"],"closeSpoofValues":[false,4000,"spoofed"],"fakeClose":[true,true,true],"submitOwnBefore":[],"submitterDescriptor":"submitter:function:get submitter:0:undefined:true:true","submitterIsButton":true,"submitSpoofIsInput":true,"fakeSubmitterThrowsTypeError":true,"formDataOwnBefore":[],"formDataDescriptor":"formData:function:get formData:0:undefined:true:true","formDataIsReal":true,"formDataSpoofIsSpoofed":true,"fakeFormDataThrowsTypeError":true}"#
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
              handlerTarget.addEventListener("click", () => handlerCalls.push("capture"), { capture: true });
              handlerTarget.onclick = event => {
                handlerCalls.push("handler");
                event.stopPropagation();
              };
              handlerTarget.addEventListener("click", () => handlerCalls.push("bubble"));
              handlerTarget.dispatchEvent(new Event("click", { bubbles: true }));

              return `${nativeCalls.join(",")}|${handlerCalls.join(",")}`;
            })()
            "#,
        )
        .expect("target stopPropagation dispatch probe should evaluate");

    assert_eq!(result, "capture:2,capture2:2|capture,handler,bubble");
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
              outer.onclick = event => {
                stoppedCalls.push("handler");
                event.stopPropagation();
              };
              outer.addEventListener("click", () => stoppedCalls.push("listener"));
              document.body.addEventListener("click", () => stoppedCalls.push("body"));
              inner.dispatchEvent(new Event("click", { bubbles: true }));

              const immediateCalls = [];
              outer.ondblclick = event => {
                immediateCalls.push("handler");
                event.stopImmediatePropagation();
              };
              outer.addEventListener("dblclick", () => immediateCalls.push("listener"));
              inner.dispatchEvent(new Event("dblclick", { bubbles: true }));

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
fn relative_range_boundaries_share_selection_updates() {
    let mut vm = new_parsed_test_vm(
        "https://shared-binding-regression.test/",
        "<!doctype html><body></body>",
    );
    let result = vm
        .eval(
            r#"
      (() => {
        document.body.innerHTML = '<div><i>a</i><b>b</b><em>c</em></div>';
        const parent = document.body.firstChild;
        const selection = getSelection();
        const results = [];
        for (const method of ['setStartBefore', 'setStartAfter', 'setEndBefore', 'setEndAfter']) {
          const range = document.createRange();
          range.selectNodeContents(parent);
          selection.removeAllRanges();
          selection.addRange(range);
          range[method](parent.children[1]);
          results.push(selection.anchorNode === range.startContainer,
            selection.anchorOffset === range.startOffset,
            selection.focusNode === range.endContainer,
            selection.focusOffset === range.endOffset);
        }
        return results.every(Boolean);
      })()
    "#,
        )
        .unwrap();
    assert_eq!(result, "true");
}

#[test]
fn selection_endpoints_follow_associated_live_range_mutations() {
    let mut vm = new_storage_test_vm("https://selection-live-mutations.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/selection-live-range-mutations.js"
        ))
        .expect("Selection endpoints should follow their live Range"),
        ""
    );
}

#[test]
fn before_unload_event_uses_its_string_return_value_interface() {
    let mut vm = new_storage_test_vm("https://before-unload-event-interface.test/");

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
              const descriptor = Object.getOwnPropertyDescriptor(
                BeforeUnloadEvent.prototype,
                "returnValue"
              );
              const event = document.createEvent("BeforeUnloadEvent");
              const initial = [
                event instanceof BeforeUnloadEvent,
                event instanceof Event,
                Object.hasOwn(event, "returnValue"),
                event.returnValue
              ];
              event.initEvent("beforeunload", false, true);
              event.returnValue = null;
              const nullValue = event.returnValue;
              event.returnValue = undefined;
              const undefinedValue = event.returnValue;
              event.returnValue = { toString() { return "object value"; } };
              const objectValue = event.returnValue;
              const symbolError = errorName(() => {
                event.returnValue = Symbol("return value");
              });

              return JSON.stringify({
                constructor: [
                  typeof BeforeUnloadEvent,
                  Object.getPrototypeOf(BeforeUnloadEvent.prototype) === Event.prototype,
                  errorName(() => new BeforeUnloadEvent())
                ],
                initial,
                initialized: [event.type, event.cancelable],
                values: [nullValue, undefinedValue, objectValue, symbolError, event.returnValue],
                metadata: [
                  descriptor.get.name,
                  descriptor.get.length,
                  descriptor.set.name,
                  descriptor.set.length,
                  descriptor.enumerable,
                  descriptor.configurable
                ],
                receiverErrors: [
                  errorName(() => descriptor.get.call(new Event("plain"))),
                  errorName(() => descriptor.set.call({}, "value"))
                ]
              });
            })()
            "#,
        )
        .expect("BeforeUnloadEvent returnValue interface probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["function",true,"TypeError"],"initial":[true,true,false,""],"initialized":["beforeunload",true],"values":["null","undefined","object value","TypeError","object value"],"metadata":["get returnValue",0,"set returnValue",1,true,true],"receiverErrors":["TypeError","TypeError"]}"#
    );
}

#[test]
fn selection_range_membership_uses_native_document_relationships() {
    let mut vm = new_storage_test_vm("https://selection-native-membership.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/selection-native-membership.js"
        ))
        .expect("selection membership should ignore author relationship properties"),
        ""
    );
}

#[test]
fn selection_shadow_direction_and_composed_boundaries_follow_native_mutations() {
    let mut vm = new_storage_test_vm("https://selection-shadow-state.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/selection-shadow-state.js"
        ))
        .expect("shadow selections should retain their composed state"),
        ""
    );
}

#[test]
fn selection_tracks_associated_range_document_across_realms() {
    let mut vm = new_storage_test_vm("https://selection-range-owner.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/selection-range-ownership.js"
        ))
        .expect("associated Range mutations should update their owning Selection"),
        ""
    );
}

#[test]
fn text_control_selection_setters_restore_the_focused_document_selection() {
    let mut vm = new_storage_test_vm("https://text-control-selection-restoration.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/text-control-selection-restoration.js"
        ))
        .expect("text control selection updates should restore only their focused document"),
        ""
    );
}

#[test]
fn document_selection_tracks_visible_text_controls_independently_of_cached_offsets() {
    let mut vm = new_storage_test_vm("https://text-control-visible-selection.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/text-control-visible-selection.js"
        ))
        .expect("visible text control selections should follow their document and focus lifecycle"),
        ""
    );
}

#[test]
fn text_control_selections_follow_value_default_value_and_form_reset() {
    let mut vm = new_storage_test_vm("https://text-control-selection-values.test/");
    assert_eq!(
        vm.eval(include_str!(
            "../../../../../tests/fixtures/text-control-selection-values.js"
        ))
        .expect("value mutations should reconcile both cached and visible selections"),
        ""
    );
}

#[test]
fn sequential_focus_navigation_retains_dom_positions_across_mutations() {
    for preserve_selection in [false, true] {
        // Navigation positions must be tracked even when the page has never
        // created a Selection or Range. Use a fresh VM for each mode.
        let mut vm = new_storage_test_vm("https://focus-navigation-starting-point.test/");
        let count = vm
            .eval(include_str!(
                "../../../../../tests/fixtures/focus-navigation-starting-point.js"
            ))
            .expect("set up focus navigation scenarios")
            .parse::<usize>()
            .unwrap();
        for index in 0..count {
            for reverse in [false, true] {
                let scenario = vm
                    .eval(&format!(
                        "JSON.stringify(__focusNavigation.setup({index}, {reverse}, {preserve_selection}))"
                    ))
                    .expect("prepare a focus origin and mutate its tree");
                vm.dispatch_key_event(
                    "keydown",
                    "Tab",
                    "Tab",
                    "",
                    if reverse { 8 } else { 0 },
                    false,
                    false,
                )
                .expect("native Tab input should continue from the focus origin");
                let result = vm
                    .eval("JSON.stringify(__focusNavigation.snapshot())")
                    .unwrap();
                let result: serde_json::Value = serde_json::from_str(&result).unwrap();
                assert_eq!(
                    result["actual"], result["expected"],
                    "{scenario}, reverse={reverse}, preserve_selection={preserve_selection}"
                );
                assert_eq!(
                    result["selectionPreserved"], true,
                    "{scenario}, reverse={reverse}, preserve_selection={preserve_selection}"
                );
            }
        }
    }
}
