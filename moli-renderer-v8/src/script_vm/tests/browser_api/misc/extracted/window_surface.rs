use super::*;

#[test]
fn window_identifier_in_domcontentloaded_listener_still_resolves_to_global_this() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        if (!document.documentElement) {
            const html = document.createElement("html");
            document.appendChild(html);
        }
        if (!document.body) {
            const body = document.createElement("body");
            document.documentElement.appendChild(body);
        }
        for (const id of ["window", "self", "top", "parent", "frames"]) {
            const node = document.createElement("div");
            node.id = id;
            document.body.appendChild(node);
        }
        document.addEventListener("DOMContentLoaded", function onReady() {
            window.__domcontentloadedWindowProbe = [
                window === globalThis,
                self === globalThis,
                top === globalThis,
                parent === globalThis,
                frames === globalThis,
                typeof window.addEventListener,
                typeof window.removeEventListener,
                typeof window.dispatchEvent
            ].join("|");
            window.removeEventListener("error", onReady);
        });
        "#,
        None,
    )
    .expect("listener registration should succeed");

    vm.dispatch_document_lifecycle_event("DOMContentLoaded")
        .expect("DOMContentLoaded dispatch should succeed");

    let result = vm
        .eval("window.__domcontentloadedWindowProbe")
        .expect("DOMContentLoaded listener probe should be readable");

    assert_eq!(
        result,
        "true|true|true|true|true|function|function|function"
    );
}
#[test]
fn assigning_window_does_not_replace_legacy_unforgeable_window_alias() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        window = { hacked: true };
        "#,
        None,
    )
    .expect("assigning window should not throw");

    let result = vm
        .eval(
            r##"
            [
                window === globalThis,
                typeof window.addEventListener,
                typeof window.removeEventListener,
                window.hacked === true
            ].join("|")
            "##,
        )
        .expect("window alias probe should evaluate");

    assert_eq!(result, "true|function|function|false");
}
#[test]
fn assigning_replaceable_window_aliases_shadows_self_parent_and_frames_only() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        self = { hacked: true };
        parent = { hacked: true };
        frames = { hacked: true };
        "#,
        None,
    )
    .expect("assigning replaceable aliases should not throw");

    let result = vm
        .eval(
            r#"
            [
                self === globalThis,
                parent === globalThis,
                frames === globalThis,
                self.hacked === true,
                parent.hacked === true,
                frames.hacked === true,
                Object.getOwnPropertyDescriptor(globalThis, "self")?.configurable === true,
                Object.getOwnPropertyDescriptor(globalThis, "parent")?.configurable === true,
                Object.getOwnPropertyDescriptor(globalThis, "frames")?.configurable === true
            ].join("|")
            "#,
        )
        .expect("replaceable alias reassignment probe should evaluate");

    assert_eq!(result, "false|false|false|true|true|true|true|true|true");
}
#[test]
fn zhihu_window_own_surface_descriptors_match_browser_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const summarize = (name) => {
                const d = Object.getOwnPropertyDescriptor(window, name);
                return {
                  present: !!d,
                  enumerable: !!d?.enumerable,
                  configurable: !!d?.configurable,
                  hasGetter: typeof d?.get === "function",
                  hasSetter: typeof d?.set === "function"
                };
              };
              return JSON.stringify({
                location: summarize("location"),
                history: summarize("history"),
                navigation: summarize("navigation"),
                navigator: summarize("navigator"),
                screen: summarize("screen"),
                customElements: summarize("customElements"),
                crypto: summarize("crypto"),
                performance: summarize("performance"),
                visualViewport: summarize("visualViewport"),
                localStorage: summarize("localStorage"),
                sessionStorage: summarize("sessionStorage"),
                indexedDB: summarize("indexedDB"),
                name: summarize("name"),
                keysInclude: [
                  Object.keys(window).includes("location"),
                  Object.keys(window).includes("history"),
                  Object.keys(window).includes("navigator"),
                  Object.keys(window).includes("screen"),
                  Object.keys(window).includes("customElements"),
                  Object.keys(window).includes("crypto"),
                  Object.keys(window).includes("name")
                ],
                helperLeak: [
                  Object.keys(window).includes("__moliHostWrite"),
                  Object.keys(window).includes("__moliNativeBridge"),
                  Object.keys(window).includes("__moliModuleEvaluationSettled"),
                  Object.keys(window).includes("__moliDOMImplementationSingleton"),
                  Object.keys(window).includes("XPathResult"),
                  Object.keys(window).includes("NodeFilter")
                ]
              });
            })()
            "#,
        )
        .expect("window own surface descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"location":{"present":true,"enumerable":true,"configurable":false,"hasGetter":true,"hasSetter":true},"history":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"navigation":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"navigator":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"screen":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"customElements":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"crypto":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"performance":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"visualViewport":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"localStorage":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"sessionStorage":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"indexedDB":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":false},"name":{"present":true,"enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true},"keysInclude":[true,true,true,true,true,true,true],"helperLeak":[false,false,false,false,false,false]}"#
    );
}
#[test]
fn window_navigator_screen_viewport_caches_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://window-subobject-private-cache.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const slots = [
                "__moliWindowNavigator",
                "__moliWindowScreen",
                "__moliWindowCrypto",
                "__moliWindowVisualViewport",
                "__moliWindowScrollX",
                "__moliWindowScrollY"
              ];
              const internalNames = () => Object.getOwnPropertyNames(window)
                .filter(name => slots.includes(name))
                .sort()
                .join(",");
              const beforeAccess = internalNames();
              const firstNavigator = window.navigator;
              const firstScreen = window.screen;
              const firstCrypto = window.crypto;
              const firstViewport = window.visualViewport;
              window.scrollTo(0, 30);
              const afterAccess = internalNames();
              Object.defineProperty(window, "__moliWindowNavigator", {
                value: { spoof: "navigator" },
                configurable: true,
                writable: true
              });
              Object.defineProperty(window, "__moliWindowScreen", {
                value: { spoof: "screen" },
                configurable: true,
                writable: true
              });
              Object.defineProperty(window, "__moliWindowCrypto", {
                value: { spoof: "crypto" },
                configurable: true,
                writable: true
              });
              Object.defineProperty(window, "__moliWindowVisualViewport", {
                value: { spoof: "viewport" },
                configurable: true,
                writable: true
              });
              Object.defineProperty(window, "__moliWindowScrollX", {
                value: 999,
                configurable: true,
                writable: true
              });
              Object.defineProperty(window, "__moliWindowScrollY", {
                value: 999,
                configurable: true,
                writable: true
              });
              try {
                const afterSpoof = internalNames();
                const secondNavigator = window.navigator;
                const secondScreen = window.screen;
                const secondCrypto = window.crypto;
                const secondViewport = window.visualViewport;
                return JSON.stringify({
                  beforeAccess,
                  afterAccess,
                  afterSpoof,
                  publicSpoof: [
                    window.__moliWindowNavigator.spoof,
                    window.__moliWindowScreen.spoof,
                    window.__moliWindowCrypto.spoof,
                    window.__moliWindowVisualViewport.spoof,
                    window.__moliWindowScrollX,
                    window.__moliWindowScrollY
                  ].join(","),
                  sameObjects: [
                    firstNavigator === secondNavigator,
                    firstScreen === secondScreen,
                    firstCrypto === secondCrypto,
                    firstViewport === secondViewport
                  ].join(","),
                  values: [
                    typeof secondNavigator.userAgent,
                    secondScreen.width,
                    typeof secondCrypto.getRandomValues,
                    secondViewport.width,
                    window.scrollX,
                    window.pageYOffset
                  ].join(",")
                });
              } finally {
                delete window.__moliWindowNavigator;
                delete window.__moliWindowScreen;
                delete window.__moliWindowCrypto;
                delete window.__moliWindowVisualViewport;
                delete window.__moliWindowScrollX;
                delete window.__moliWindowScrollY;
              }
            })()
            "#,
        )
        .expect("window subobject private cache probe should evaluate");

    assert_eq!(
        result,
        r#"{"beforeAccess":"","afterAccess":"","afterSpoof":"__moliWindowCrypto,__moliWindowNavigator,__moliWindowScreen,__moliWindowScrollX,__moliWindowScrollY,__moliWindowVisualViewport","publicSpoof":"navigator,screen,crypto,viewport,999,999","sameObjects":"true,true,true,true","values":"string,1920,function,1920,0,30"}"#
    );
}
#[test]
fn window_global_event_handler_accessors_match_declared_surface() {
    let mut vm = new_storage_test_vm("https://window-event-handler-accessors.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const describe = name => {
                const descriptor = Object.getOwnPropertyDescriptor(window, name);
                return [
                  name,
                  Object.hasOwn(window, name),
                  typeof descriptor?.get,
                  descriptor?.get?.name,
                  descriptor?.get?.length,
                  typeof descriptor?.set,
                  descriptor?.set?.name,
                  descriptor?.set?.length,
                  descriptor?.enumerable,
                  descriptor?.configurable
                ].join(":");
              };
              function onerrorHandler() {}
              function unhandledHandler() {}
              function rejectionHandler() {}
              window.onerror = onerrorHandler;
              window.onunhandledrejection = unhandledHandler;
              window.onrejectionhandled = rejectionHandler;
              const assigned = [
                window.onerror === onerrorHandler,
                window.onunhandledrejection === unhandledHandler,
                window.onrejectionhandled === rejectionHandler
              ].join(":");
              window.onerror = {};
              window.onunhandledrejection = 1;
              window.onrejectionhandled = undefined;
              const cleared = [
                window.onerror,
                window.onunhandledrejection,
                window.onrejectionhandled
              ].map(value => value === null ? "null" : typeof value).join(":");
              return JSON.stringify({
                descriptors: [
                  describe("onerror"),
                  describe("onunhandledrejection"),
                  describe("onrejectionhandled")
                ],
                keys: [
                  Object.keys(window).includes("onerror"),
                  Object.keys(window).includes("onunhandledrejection"),
                  Object.keys(window).includes("onrejectionhandled")
                ],
                assigned,
                cleared
              });
            })()
            "#,
        )
        .expect("window global event handler accessor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["onerror:true:function:get onerror:0:function:set onerror:1:true:true","onunhandledrejection:true:function:get onunhandledrejection:0:function:set onunhandledrejection:1:true:true","onrejectionhandled:true:function:get onrejectionhandled:0:function:set onrejectionhandled:1:true:true"],"keys":[true,true,true],"assigned":"true:true:true","cleared":"null:null:null"}"#
    );
}
#[test]
fn window_viewport_surface_exposes_outer_dimensions() {
    let mut vm = new_storage_test_vm("https://window-viewport-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => JSON.stringify({
              innerWidth,
              innerHeight,
              outerWidth,
              outerHeight,
              screenX,
              screenY,
              hasOuterWidth: "outerWidth" in window,
              hasOuterHeight: "outerHeight" in window,
              outerWidthType: typeof outerWidth,
              outerHeightType: typeof outerHeight
            }))()
            "#,
        )
        .expect("window viewport surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"innerWidth":1920,"innerHeight":1080,"outerWidth":1920,"outerHeight":1080,"screenX":0,"screenY":0,"hasOuterWidth":true,"hasOuterHeight":true,"outerWidthType":"number","outerHeightType":"number"}"#
    );
}
#[test]
fn window_and_visual_viewport_follow_renderer_viewport_surface_changes() {
    let mut vm = new_storage_test_vm("https://window-live-viewport-surface.test/");
    let surface = |inner_width, inner_height| crate::protocol_types::ViewportSurface {
        inner_width,
        inner_height,
        outer_width: inner_width,
        outer_height: inner_height,
        device_pixel_ratio: 1.0,
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,

        ..Default::default()
    };

    vm.set_viewport_surface(Some(surface(800, 600)))
        .expect("initial viewport surface should update");
    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__cachedVisualViewport = window.visualViewport;
              return JSON.stringify({
                innerWidth,
                innerHeight,
                visualWidth: __cachedVisualViewport.width,
                visualHeight: __cachedVisualViewport.height
              });
            })()
            "#,
        )
        .expect("initial viewport dimensions should evaluate");
    assert_eq!(
        initial,
        r#"{"innerWidth":800,"innerHeight":600,"visualWidth":800,"visualHeight":600}"#
    );

    vm.set_viewport_surface(Some(surface(1024, 768)))
        .expect("replacement viewport surface should update");
    let updated = vm
        .eval(
            r#"
            JSON.stringify({
              innerWidth,
              innerHeight,
              sameVisualViewport: visualViewport === __cachedVisualViewport,
              visualWidth: visualViewport.width,
              visualHeight: visualViewport.height
            })
            "#,
        )
        .expect("updated viewport dimensions should evaluate");
    assert_eq!(
        updated,
        r#"{"innerWidth":1024,"innerHeight":768,"sameVisualViewport":true,"visualWidth":1024,"visualHeight":768}"#
    );

    vm.set_viewport_surface(None)
        .expect("cleared viewport surface should update");
    let restored = vm
        .eval(
            r#"
            JSON.stringify({
              innerWidth,
              innerHeight,
              sameVisualViewport: visualViewport === __cachedVisualViewport,
              visualWidth: visualViewport.width,
              visualHeight: visualViewport.height
            })
            "#,
        )
        .expect("restored viewport dimensions should evaluate");
    assert_eq!(
        restored,
        r#"{"innerWidth":1920,"innerHeight":1080,"sameVisualViewport":true,"visualWidth":1920,"visualHeight":1080}"#
    );
}
#[test]
fn visual_viewport_private_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://visual-viewport-private-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const viewport = window.visualViewport;
              const internalNames = object => Object.getOwnPropertyNames(object)
                .filter(name => name.startsWith("__moliVisualViewport"))
                .sort()
                .join(",");

              const initialOwnSlots = internalNames(viewport);
              const descriptorSummary = name => {
                const descriptor = Object.getOwnPropertyDescriptor(
                  VisualViewport.prototype,
                  name
                );
                return `${name}:${typeof descriptor?.get}:${descriptor?.get?.name}:${descriptor?.get?.length}:${typeof descriptor?.set}:${descriptor?.enumerable}:${descriptor?.configurable}:${Object.hasOwn(viewport, name)}`;
              };
              const attributeNames = [
                "offsetLeft",
                "offsetTop",
                "pageLeft",
                "pageTop",
                "width",
                "height",
                "scale"
              ];
              const slots = [
                "__moliVisualViewportOffsetLeft",
                "__moliVisualViewportOffsetTop",
                "__moliVisualViewportPageLeft",
                "__moliVisualViewportPageTop",
                "__moliVisualViewportWidth",
                "__moliVisualViewportHeight",
                "__moliVisualViewportScale"
              ];
              for (const name of slots) {
                VisualViewport.prototype[name] = -100;
                viewport[name] = -200;
              }
              viewport.__moliVisualViewportBrand = true;

              const values = [
                viewport.offsetLeft,
                viewport.offsetTop,
                viewport.pageLeft,
                viewport.pageTop,
                viewport.width,
                viewport.height,
                viewport.scale
              ].join(",");

              const widthGetter = Object.getOwnPropertyDescriptor(
                VisualViewport.prototype,
                "width"
              ).get;
              const fake = {};
              fake.__moliVisualViewportBrand = true;
              const fakeWidth = (() => {
                try {
                  return String(widthGetter.call(fake));
                } catch (error) {
                  return `throw:${error && error.name}`;
                }
              })();

              return [
                initialOwnSlots,
                values,
                attributeNames.map(descriptorSummary).join(","),
                fakeWidth,
                internalNames(fake),
                viewport instanceof VisualViewport,
                String(viewport)
              ].join("|");
            })()
            "#,
        )
        .expect("VisualViewport backing slots should resist reflection and spoofing");

    assert_eq!(
        result,
        "|0,0,0,0,1920,1080,1|offsetLeft:function:get offsetLeft:0:undefined:true:true:false,offsetTop:function:get offsetTop:0:undefined:true:true:false,pageLeft:function:get pageLeft:0:undefined:true:true:false,pageTop:function:get pageTop:0:undefined:true:true:false,width:function:get width:0:undefined:true:true:false,height:function:get height:0:undefined:true:true:false,scale:function:get scale:0:undefined:true:true:false|throw:TypeError|__moliVisualViewportBrand|true|[object VisualViewport]"
    );
}
#[test]
fn window_agent_rejects_blocking_atomics_wait() {
    let mut vm = new_storage_test_vm("https://window-atomics-wait.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              try {
                Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1);
                return "unexpected-success";
              } catch (error) {
                return `${error.name}:${error.message}`;
              }
            })()
            "#,
        )
        .expect("Window Atomics.wait probe should evaluate");

    assert_eq!(
        result,
        "TypeError:Atomics.wait cannot be called in this context"
    );
}
#[test]
fn window_name_default_and_assignment_match_browser_expectation() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const before = window.name;
              window.name = 42;
              const after = window.name;
              const descriptor = Object.getOwnPropertyDescriptor(window, "name");
              return JSON.stringify({
                before,
                after,
                enumerable: !!descriptor?.enumerable,
                configurable: !!descriptor?.configurable,
                hasGetter: typeof descriptor?.get === "function",
                hasSetter: typeof descriptor?.set === "function"
              });
            })()
            "#,
        )
        .expect("window.name probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":"","after":"42","enumerable":true,"configurable":true,"hasGetter":true,"hasSetter":true}"#
    );
}
#[test]
fn zhihu_probe_window_capabilities_exist_on_global_scope() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            JSON.stringify({
              stopType: typeof stop,
              printType: typeof print,
              openType: typeof open,
              mediaSourceType: typeof MediaSource,
              mediaSourceProbe: MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"'),
              mediaSourceNope: MediaSource.isTypeSupported('video/webm; codecs="vp09.00.10.08"'),
              availLeft: screen.availLeft,
              availTop: screen.availTop
            })
            "#,
        )
        .expect("zhihu window capability probe should evaluate");

    assert_eq!(
        result,
        r#"{"stopType":"function","printType":"function","openType":"function","mediaSourceType":"function","mediaSourceProbe":true,"mediaSourceNope":false,"availLeft":0,"availTop":0}"#
    );
}
#[test]
fn window_is_secure_context_reflects_document_trustworthiness() {
    for (url, expected) in [
        ("http://insecure-context.test/", "boolean|false"),
        ("https://secure-context.test/", "boolean|true"),
        ("http://localhost/", "boolean|true"),
    ] {
        let mut vm = new_storage_test_vm(url);
        assert_eq!(
            vm.eval("`${typeof isSecureContext}|${isSecureContext}`")
                .expect("isSecureContext probe should evaluate"),
            expected,
            "unexpected secure-context signal for {url}"
        );
    }
}
#[test]
fn window_event_source_constructor_exposes_connecting_instance() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
            (() => {
              const describe = (name, construct) => {
                let errorName = "";
                let errorCode = 0;
                let errorMessage = "";
                try {
                  construct();
                } catch (error) {
                  errorName = error && error.name;
                  errorCode = error && error.code;
                  errorMessage = error && error.message;
                }
                const ctor = globalThis[name];
                return [
                  typeof ctor,
                  ctor && ctor.name,
                  ctor && ctor.length,
                  Object.getPrototypeOf(ctor.prototype) === EventTarget.prototype,
                  errorName,
                  errorCode,
                  errorMessage
                ].join("|");
              };
              const description = describe(
                "EventSource",
                () => new EventSource("/events")
              );
              const source = new EventSource("/events", { withCredentials: true });
              const handlerObject = { handleEvent() {} };
              source.onerror = handlerObject;
              const state = [
                source.url,
                source.withCredentials,
                source.readyState,
                source.onerror === handlerObject,
                Object.hasOwn(source, "addEventListener"),
                source.addEventListener.length,
                source.removeEventListener.length,
                source.dispatchEvent.length
              ];
              source.close();
              globalThis.__eventSourceConstructor = [
                description,
                ...state,
                source.readyState,
              ].join("|");
            })()
            "#,
    )
    .expect("EventSource constructor probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__eventSourceConstructor)")
        .expect("EventSource constructor probe should serialize");

    assert_eq!(
        result,
        r#""function|EventSource|1|true||0||https://example.com/events|true|0|true|false|2|2|1|2""#
    );
}
#[test]
fn window_get_computed_style_is_an_own_global_method_only() {
    let mut vm = new_storage_test_vm("https://computed-style-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const summarize = descriptor => [
                typeof descriptor.value,
                descriptor.value.name,
                descriptor.value.length,
                descriptor.enumerable,
                descriptor.configurable,
                descriptor.writable,
                /^function getComputedStyle\(/.test(String(descriptor.value)),
                /\[native code\]/.test(String(descriptor.value))
              ].join(":");

              const globalDescriptor =
                Object.getOwnPropertyDescriptor(window, "getComputedStyle");
              const prototypeDescriptor =
                Object.getOwnPropertyDescriptor(Window.prototype, "getComputedStyle");
              const target = document.createElement("div");
              (document.body || document.documentElement || document).appendChild(target);
              const style = getComputedStyle(target);

              return [
                summarize(globalDescriptor),
                prototypeDescriptor === undefined,
                typeof style.getPropertyValue
              ].join("|");
            })()
            "#,
        )
        .expect("declared getComputedStyle descriptors should evaluate");

    assert_eq!(
        result,
        "function:getComputedStyle:1:true:true:true:true:true|true|function"
    );
}
#[test]
fn zhihu_probe_window_blur_and_find_match_browser_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const shape = name => {
                const descriptor = Object.getOwnPropertyDescriptor(window, name);
                const value = descriptor && descriptor.value;
                return [
                  typeof value,
                  value && value.name,
                  value && value.length,
                  descriptor && descriptor.enumerable,
                  descriptor && descriptor.configurable,
                  descriptor && descriptor.writable,
                  /\[native code\]/.test(String(value))
                ].join(":");
              };
              return JSON.stringify({
                blur: shape("blur"),
                find: shape("find"),
                stop: shape("stop"),
                print: shape("print"),
                open: shape("open"),
                bareBlurType: typeof blur,
                bareFindType: typeof find,
                blurResult: String(window.blur()),
                findResult: window.find("needle"),
                bareFindResult: find("needle")
              });
            })()
            "#,
        )
        .expect("window blur/find probe should evaluate");

    assert_eq!(
        result,
        r#"{"blur":"function:blur:0:true:true:true:true","find":"function:find:0:true:true:true:true","stop":"function:stop:0:true:true:true:true","print":"function:print:0:true:true:true:true","open":"function:open:0:true:true:true:true","bareBlurType":"function","bareFindType":"function","blurResult":"undefined","findResult":false,"bareFindResult":false}"#
    );
}
#[test]
fn window_dialog_and_open_arguments_use_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = (callback) => {
                try {
                  const value = callback();
                  return value === null ? "null" : String(value);
                } catch (error) {
                  return error && error.name;
                }
              };
              return [
                probe(() => alert(undefined)),
                probe(() => alert(Symbol("message"))),
                probe(() => confirm()),
                probe(() => confirm({
                  toString() {
                    throw new RangeError("confirm");
                  }
                })),
                probe(() => prompt(undefined, undefined)),
                probe(() => prompt("message", Symbol("default"))),
                probe(() => open()),
                probe(() => open(Symbol("url"))),
                probe(() => open("/popup", Symbol("target"))),
                probe(() => open("/popup", "_blank", {
                  toString() {
                    throw new RangeError("features");
                  }
                }))
              ].join("|");
            })()
            "#,
        )
        .expect("window dialog/open WebIDL argument probe should evaluate");

    assert_eq!(
        result,
        "undefined|TypeError|false|RangeError|null|TypeError|[object Window]|TypeError|TypeError|RangeError"
    );
}
#[test]
fn window_name_accessors_reject_non_window_receivers_without_mutating_the_window() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const descriptor = Object.getOwnPropertyDescriptor(window, "name");
              const outcome = callback => {
                try { callback(); return "ok"; }
                catch (error) { return error.name; }
              };
              const before = window.name;
              return JSON.stringify({
                setter: outcome(() => descriptor.set.call({}, "forged")),
                getter: outcome(() => descriptor.get.call({})),
                before,
                after: window.name
              });
            })()
            "#,
        )
        .expect("window.name illegal receiver probe should evaluate");

    assert_eq!(
        result,
        r#"{"setter":"TypeError","getter":"TypeError","before":"","after":""}"#
    );
}

#[test]
fn window_name_accessors_follow_the_receiver_realm_and_preserve_state_on_conversion_errors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const frame = document.createElement("iframe");
              (document.body || document.documentElement || document).appendChild(frame);
              const child = frame.contentWindow;
              const parentDescriptor = Object.getOwnPropertyDescriptor(window, "name");
              const childDescriptor = Object.getOwnPropertyDescriptor(child, "name");

              parentDescriptor.set.call(child, "child-from-parent-realm");
              childDescriptor.set.call(window, "parent-from-child-realm");
              const borrowedParent = childDescriptor.get.call(window);
              const borrowedChild = parentDescriptor.get.call(child);

              const popup = window.open("about:blank", "popup-owner");
              const borrowedPopupBefore = parentDescriptor.get.call(popup);
              parentDescriptor.set.call(popup, "renamed-popup-owner");
              const borrowedPopupAfter = parentDescriptor.get.call(popup);
              const openerAfterPopupRename = window.name;

              const iterator = document.createNodeIterator(document);
              let invalidReceiverGetter;
              let invalidReceiverSetter;
              let invalidReceiverConverted = false;
              try {
                parentDescriptor.get.call(iterator);
              } catch (error) {
                invalidReceiverGetter = error.name;
              }
              try {
                parentDescriptor.set.call(iterator, {
                  toString() {
                    invalidReceiverConverted = true;
                    return "forged";
                  }
                });
              } catch (error) {
                invalidReceiverSetter = error.name;
              }

              let conversionError;
              try {
                childDescriptor.set.call(window, {
                  toString() { throw new RangeError("window-name-conversion"); }
                });
              } catch (error) {
                conversionError = `${error.name}:${error.message}`;
              }

              return JSON.stringify({
                borrowedParent,
                borrowedChild,
                borrowedPopupBefore,
                borrowedPopupAfter,
                openerAfterPopupRename,
                invalidReceiverGetter,
                invalidReceiverSetter,
                invalidReceiverConverted,
                parentAfterError: window.name,
                childAfterError: child.name,
                conversionError
              });
            })()
            "#,
        )
        .expect("borrowed Window.name accessors should preserve their receiver realm");

    assert_eq!(
        result,
        r#"{"borrowedParent":"parent-from-child-realm","borrowedChild":"child-from-parent-realm","borrowedPopupBefore":"popup-owner","borrowedPopupAfter":"renamed-popup-owner","openerAfterPopupRename":"parent-from-child-realm","invalidReceiverGetter":"TypeError","invalidReceiverSetter":"TypeError","invalidReceiverConverted":false,"parentAfterError":"parent-from-child-realm","childAfterError":"child-from-parent-realm","conversionError":"RangeError:window-name-conversion"}"#
    );
}
