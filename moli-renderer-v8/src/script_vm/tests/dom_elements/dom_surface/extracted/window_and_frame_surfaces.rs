use super::*;

#[test]
fn detached_native_xml_text_content_queues_child_list_record() {
    let mut vm = new_storage_test_vm("https://detached-xml-text-content-mutation.test/");

    let result = vm
        .eval(
            r#"
            (() => {
              const xml = new DOMParser().parseFromString('<root></root>', 'text/xml');
              const element = xml.createElement('sample');
              element.appendChild(xml.createCDATASection('foo'));
              const observer = new MutationObserver(() => {});
              observer.observe(element, { childList: true });
              element.textContent = 'foo';
              const record = observer.takeRecords()[0];
              return [
                record && record.removedNodes[0].nodeType,
                record && record.removedNodes[0].data,
                record && record.addedNodes[0].nodeType,
                record && record.addedNodes[0].data
              ].join('|');
            })()
            "#,
        )
        .expect("detached native XML textContent should queue childList mutation");

    assert_eq!(result, "4|foo|3|foo");
}
#[tokio::test]
async fn child_parser_css_preserves_document_and_result_realms() {
    for head in [
        "<style>div { color: green; }</style>",
        "<link rel=author href=/author>",
    ] {
        let mut vm = new_storage_test_vm("https://child-parser-css-realm.test/");
        let markup = serde_json::to_string(&format!(
            "<!doctype html><head>{head}</head><body><div id=target>child</div>"
        ))
        .unwrap();
        vm.exec(
            &format!(
                r#"
globalThis.realmFrame = document.createElement('iframe');
realmFrame.srcdoc = {markup};
(document.body || document.documentElement || document).appendChild(realmFrame);
"#
            ),
            None,
        )
        .expect("child CSS document should be queued");
        run_child_navigation_commit_and_host_load_for_test(&mut vm, "child CSS realm document")
            .await;

        let result = vm
            .eval(
                r#"
(() => {
  const w = realmFrame.contentWindow, d = realmFrame.contentDocument;
  const element = d.getElementById('target');
  const list = d.querySelectorAll('div'), range = d.createRange();
  let typeError, syntaxError;
  try { element.matches(); } catch (error) { typeError = error; }
  try { element.matches('['); } catch (error) { syntaxError = error; }
  return JSON.stringify({
    document: d === w.document && d instanceof w.Document && !(d instanceof Document),
    element: element instanceof w.Element && !(element instanceof Element),
    list: list instanceof w.NodeList && !(list instanceof NodeList),
    range: range instanceof w.Range && !(range instanceof Range),
    typeError: typeError instanceof w.TypeError && !(typeError instanceof TypeError),
    syntaxError: syntaxError instanceof w.DOMException && !(syntaxError instanceof DOMException),
    sheets: d.styleSheets instanceof w.StyleSheetList &&
      Array.from(d.styleSheets).every(sheet => sheet instanceof w.CSSStyleSheet &&
        sheet.cssRules[0] instanceof w.CSSStyleRule),
    sheetCount: d.styleSheets.length,
    fonts: d.fonts instanceof w.FontFaceSet
  });
})()
"#,
            )
            .expect("child CSS wrappers and DOM results should remain in the child realm");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::json!({
                "document": true,
                "element": true,
                "list": true,
                "range": true,
                "typeError": true,
                "syntaxError": true,
                "sheets": true,
                "sheetCount": usize::from(head.starts_with("<style>")),
                "fonts": true,
            }),
            "child markup: {head}"
        );
    }
}
#[tokio::test]
async fn child_parser_css_updates_held_collections_in_their_realm() {
    let mut vm = new_storage_test_vm("https://child-parser-css-held-realm.test/");
    vm.exec(
        r#"
globalThis.realmFrame = document.createElement('iframe');
realmFrame.srcdoc = `<!doctype html><head><script>
  globalThis.heldSheets = document.styleSheets;
  globalThis.heldFonts = document.fonts;
</` + `script><style>
  @font-face { font-family: ChildParser; src: local('ChildParser'); }
  div { color: green; }
</style></head><body><div>child</div>`;
(document.body || document.documentElement || document).appendChild(realmFrame);
"#,
        None,
    )
    .expect("child CSS collection setup should be queued");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child should commit before the script captures CSS collections",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child script should capture collections before parsing style",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(&mut vm, "child CSS collection update")
        .await;

    let result = vm
        .eval(
            r#"
(() => {
  const w = realmFrame.contentWindow;
  // Inspect held objects before any Document getter can initialize or resync them.
  const sheets = w.heldSheets, fonts = w.heldFonts;
  const faces = Array.from(fonts);
  const result = {
    sheetCount: sheets.length,
    ruleCount: sheets[0].cssRules.length,
    sheetRealm: sheets[0] instanceof w.CSSStyleSheet &&
      sheets[0].cssRules[0] instanceof w.CSSFontFaceRule,
    fontCount: faces.length,
    fontRealm: faces.every(face => face instanceof w.FontFace && !(face instanceof FontFace)),
    sheetListRealm: sheets instanceof w.StyleSheetList,
    fontSetRealm: fonts instanceof w.FontFaceSet
  };
  result.identity = sheets === w.document.styleSheets && fonts === w.document.fonts;
  return JSON.stringify(result);
})()
"#,
        )
        .expect("parser should update already exposed child CSS collections");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result).unwrap(),
        serde_json::json!({
            "sheetCount": 1,
            "ruleCount": 2,
            "sheetRealm": true,
            "fontCount": 1,
            "fontRealm": true,
            "sheetListRealm": true,
            "fontSetRealm": true,
            "identity": true,
        })
    );
}
#[tokio::test]
async fn child_content_document_getter_does_not_enumerate_script_wrappers_after_load() {
    let mut vm = new_storage_test_vm("https://child-content-document-getter-script-state.test/");
    vm.eval(
        r#"
(() => {
  globalThis.__childGetterCalledPageMethod = 0;
  const frame = document.createElement("iframe");
  frame.srcdoc = `<body><script>
    document.getElementsByTagName = function() {
      top.__childGetterCalledPageMethod += 1;
      throw new Error("contentDocument getter should not call page method");
    };
    top.__childScriptRan = true;
  </` + `script></body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child contentDocument getter setup should evaluate");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child srcdoc should commit before its parser script",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child srcdoc parser script should run on DocumentScriptReady",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(
        &mut vm,
        "child contentDocument getter srcdoc",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector("iframe");
  const doc = frame.contentDocument;
  return [
    String(globalThis.__childScriptRan),
    String(doc === frame.contentWindow.document),
    String(globalThis.__childGetterCalledPageMethod)
  ].join("|");
})()
"#,
        )
        .expect("child contentDocument getter should not call child document methods");

    assert_eq!(result, "true|true|0");
}
#[test]
fn window_named_properties_use_the_chromium_prototype_layer() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

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
                const named = document.createElement("div");
                named.id = "namedProbe";
                document.body.appendChild(named);
                const namedProperties = Object.getPrototypeOf(Window.prototype);
                return [
                    Object.getPrototypeOf(globalThis) === Window.prototype,
                    Object.getPrototypeOf(namedProperties) === EventTarget.prototype,
                    namedProperties.constructor === EventTarget,
                    Object.getOwnPropertyNames(namedProperties).length,
                    window.namedProbe === named,
                    Object.prototype.hasOwnProperty.call(window, "namedProbe"),
                    Object.getOwnPropertyDescriptor(window, "namedProbe") === undefined,
                    Object.prototype.hasOwnProperty.call(namedProperties, "namedProbe"),
                    Object.getOwnPropertyDescriptor(namedProperties, "namedProbe").value === named,
                    document === globalThis.document
                ].join("|");
            })()
            "#,
        )
        .expect("Window named properties prototype probe should evaluate");

    assert_eq!(result, "true|true|true|0|true|false|true|true|true|true");
}
#[test]
fn window_named_properties_exposes_webidl_class_string() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const namedProperties = Object.getPrototypeOf(Window.prototype);
                const descriptor = Object.getOwnPropertyDescriptor(
                    namedProperties,
                    Symbol.toStringTag
                );
                return JSON.stringify({
                    own: Object.hasOwn(namedProperties, Symbol.toStringTag),
                    value: descriptor && descriptor.value,
                    configurable: descriptor && descriptor.configurable,
                    enumerable: descriptor && descriptor.enumerable,
                    writable: descriptor && descriptor.writable,
                    classString: Object.prototype.toString.call(namedProperties)
                });
            })()
            "#,
        )
        .expect("WindowProperties class string probe should evaluate");

    assert_eq!(
        result,
        r#"{"own":true,"value":"WindowProperties","configurable":true,"enumerable":false,"writable":false,"classString":"[object WindowProperties]"}"#
    );
}
#[test]
fn window_named_properties_respect_later_prototype_properties_and_descriptor_flags() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

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
                for (const id of ["visibleNamed", "eventTargetShadow", "objectShadow"]) {
                    const element = document.createElement("span");
                    element.id = id;
                    document.body.appendChild(element);
                }
                EventTarget.prototype.eventTargetShadow = "event-target";
                Object.prototype.objectShadow = "object";

                const namedProperties = Object.getPrototypeOf(Window.prototype);
                const descriptor = Object.getOwnPropertyDescriptor(
                    namedProperties,
                    "visibleNamed"
                );
                return JSON.stringify({
                    visibleValue: namedProperties.visibleNamed.id,
                    visibleOwn: Object.hasOwn(namedProperties, "visibleNamed"),
                    descriptorWritable: descriptor.writable,
                    descriptorEnumerable: descriptor.enumerable,
                    descriptorConfigurable: descriptor.configurable,
                    eventTargetValue: window.eventTargetShadow,
                    eventTargetOwn: Object.hasOwn(namedProperties, "eventTargetShadow"),
                    objectValue: window.objectShadow,
                    objectOwn: Object.hasOwn(namedProperties, "objectShadow")
                });
            })()
            "#,
        )
        .expect("Window named property prototype visibility probe should evaluate");

    assert_eq!(
        result,
        r#"{"visibleValue":"visibleNamed","visibleOwn":true,"descriptorWritable":true,"descriptorEnumerable":false,"descriptorConfigurable":true,"eventTargetValue":"event-target","eventTargetOwn":false,"objectValue":"object","objectOwn":false}"#
    );
}
#[test]
fn main_and_child_window_proxies_have_immutable_prototypes() {
    let mut vm = new_storage_test_vm("https://window-proxy-prototype.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                "use strict";
                const frame = document.createElement("iframe");
                (document.body || document.documentElement || document).appendChild(frame);

                const probe = target => {
                    const original = Object.getPrototypeOf(target);
                    const replacement = {};
                    const outcome = callback => {
                        try {
                            callback();
                            return "returned";
                        } catch (error) {
                            return error && error.name;
                        }
                    };

                    const objectDifferent = outcome(() => {
                        Object.setPrototypeOf(target, replacement);
                    });
                    const dunderDifferent = outcome(() => {
                        target.__proto__ = replacement;
                    });
                    const reflectDifferent = Reflect.setPrototypeOf(target, replacement);
                    const unchanged = Object.getPrototypeOf(target) === original;
                    const objectSame = Object.setPrototypeOf(target, original) === target;
                    const dunderSame = outcome(() => {
                        target.__proto__ = original;
                    });
                    const reflectSame = Reflect.setPrototypeOf(target, original);

                    return {
                        objectDifferent,
                        dunderDifferent,
                        reflectDifferent,
                        unchanged,
                        objectSame,
                        dunderSame,
                        reflectSame,
                    };
                };

                const observations = {
                    main: probe(window),
                    child: probe(frame.contentWindow),
                };
                frame.remove();
                return JSON.stringify(observations);
            })()
            "#,
        )
        .expect("WindowProxy immutable prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"main":{"objectDifferent":"TypeError","dunderDifferent":"TypeError","reflectDifferent":false,"unchanged":true,"objectSame":true,"dunderSame":"returned","reflectSame":true},"child":{"objectDifferent":"TypeError","dunderDifferent":"TypeError","reflectDifferent":false,"unchanged":true,"objectSame":true,"dunderSame":"returned","reflectSame":true}}"#
    );
}
#[test]
fn window_internal_child_context_identity_is_not_read_from_web_properties() {
    let mut vm = new_storage_test_vm("https://window-private-identity.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const original = document;
                Object.defineProperty(window, "__moliChildBrowsingContextHandle", {
                    configurable: true,
                    value: 123456
                });
                return document === original && window.__moliChildBrowsingContextHandle === 123456;
            })()
            "#,
        )
        .expect("private child context identity probe should evaluate");

    assert_eq!(result, "true");
}
#[test]
fn iframe_id_named_window_property_returns_element_not_child_window() {
    let mut vm = new_storage_test_vm("https://iframe-id-named-property.test/");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.id = 'i';
(document.body || document.documentElement || document).appendChild(frame);
[
  Object.prototype.toString.call(i),
  i instanceof HTMLIFrameElement,
  typeof i.contentWindow,
  typeof i.contentWindow.navigation
].join('|')
"#,
        )
        .expect("iframe id named property should expose the element wrapper");

    assert_eq!(result, "[object HTMLIFrameElement]|true|object|object");
}
#[test]
fn iframe_name_named_window_property_returns_child_window() {
    let mut vm = new_storage_test_vm("https://iframe-name-named-property.test/");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.id = 'frame';
frame.name = 'target';
(document.body || document.documentElement || document).appendChild(frame);
[
  Object.prototype.toString.call(target),
  target === document.getElementById('frame').contentWindow,
  typeof target.navigation
].join('|')
"#,
        )
        .expect("iframe name named property should expose the child window");

    assert_eq!(result, "[object Window]|true|object");
}
#[test]
fn iframe_in_shadow_tree_is_not_a_named_window_property() {
    let mut vm = new_storage_test_vm("https://shadow-iframe-named-property.test/");

    let result = vm
        .eval(
            r#"
const host = document.createElement('div');
(document.body || document.documentElement || document).appendChild(host);
const shadow = host.attachShadow({ mode: 'open' });
const shadowFrame = document.createElement('iframe');
shadowFrame.name = 'shadowTarget';
shadow.appendChild(shadowFrame);
const lightFrame = document.createElement('iframe');
lightFrame.name = 'lightTarget';
(document.body || document.documentElement || document).appendChild(lightFrame);
[
  'shadowTarget' in window,
  window.shadowTarget === undefined,
  shadowFrame.contentWindow !== null,
  'lightTarget' in window,
  window.lightTarget === lightFrame.contentWindow
].join('|')
"#,
        )
        .expect("shadow iframe named property probe should evaluate");

    assert_eq!(result, "false|true|true|true|true");
}
#[test]
fn child_webassembly_native_values_use_public_intrinsic_prototypes() {
    let mut vm = new_storage_test_vm("https://child-wasm-intrinsics.test/");
    assert_eq!(
        vm.eval(
            r#"
(() => {
  const names = ['Module', 'Instance', 'Memory', 'Table', 'Global',
    'CompileError', 'LinkError', 'RuntimeError'];
  const parentConstructors = names.map(name => WebAssembly[name]);
  const frame = document.createElement('iframe');
  const sibling = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  (document.body || document.documentElement || document).appendChild(sibling);
  const child = frame.contentWindow;
  const wasm = child.WebAssembly;
  const other = sibling.contentWindow.WebAssembly;
  const results = {};
  results.isolated = names.every((name, index) =>
    wasm[name].prototype !== other[name].prototype &&
    wasm[name].prototype !== parentConstructors[index].prototype &&
    wasm[name].prototype.constructor === wasm[name] &&
    other[name].prototype.constructor === other[name] &&
    parentConstructors[index].prototype.constructor === parentConstructors[index]);
  results.errorParent = ['CompileError', 'LinkError', 'RuntimeError'].every(name =>
    Object.getPrototypeOf(wasm[name].prototype) === child.Error.prototype);
  results.descriptors = names.every(name => {
    const descriptor = Object.getOwnPropertyDescriptor(wasm[name], 'prototype');
    return !descriptor.writable && !descriptor.enumerable && !descriptor.configurable;
  });
  function thrownInChild(name, operation) {
    try { operation(); return false; }
    catch (error) {
      return error.constructor === wasm[name] && error instanceof wasm[name] &&
        Object.getPrototypeOf(error) === wasm[name].prototype &&
        !(error instanceof WebAssembly[name]);
    }
  }
  results.compileError = thrownInChild('CompileError', () =>
    new wasm.Module(new Uint8Array([0])));
  const importing = new wasm.Module(new Uint8Array([
    0,97,115,109,1,0,0,0,1,4,1,96,0,0,2,7,1,1,109,1,102,0,0
  ]));
  results.linkError = thrownInChild('LinkError', () =>
    new wasm.Instance(importing, {m: {f: 1}}));
  const trap = new wasm.Module(new Uint8Array([
    0,97,115,109,1,0,0,0,1,4,1,96,0,0,3,2,1,0,7,5,1,1,102,0,0,
    10,5,1,3,0,0,11
  ]));
  results.runtimeError = thrownInChild('RuntimeError', () =>
    new wasm.Instance(trap).exports.f());
  const sentinel = new RangeError('author getter');
  try { new wasm.Memory({get initial() { throw sentinel; }}); }
  catch (error) { results.authorException = error === sentinel; }
  return JSON.stringify(results);
})()
    "#
        )
        .unwrap(),
        r#"{"isolated":true,"errorParent":true,"descriptors":true,"compileError":true,"linkError":true,"runtimeError":true,"authorException":true}"#
    );
}
#[test]
fn child_webassembly_constructors_use_newtarget_child_realm_default_prototype() {
    let mut vm = new_storage_test_vm("https://child-wasm-newtarget.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constructorFrame = document.createElement("iframe");
  const newTargetFrame = document.createElement("iframe");
  const otherFrame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(constructorFrame);
  (document.body || document.documentElement || document).appendChild(newTargetFrame);
  (document.body || document.documentElement || document).appendChild(otherFrame);
  const constructorRealm = constructorFrame.contentWindow;
  const newTargetRealm = newTargetFrame.contentWindow;
  const otherRealm = otherFrame.contentWindow;
  const bytes = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);
  const bindOther = otherRealm.Function.prototype.bind;
  const ProxyOther = otherRealm.Proxy;

  function freshNewTarget(prototype) {
    const newTarget = new newTargetRealm.Function();
    newTarget.prototype = prototype;
    return newTarget;
  }

  function usesNewTargetDefaultPrototype(interfaceName, argument, newTarget) {
    const Constructor = constructorRealm.WebAssembly[interfaceName];
    const object = Reflect.construct(Constructor, [argument], newTarget);
    const ExpectedConstructor = newTargetRealm.WebAssembly[interfaceName];
    return (
      Object.getPrototypeOf(object) === ExpectedConstructor.prototype &&
      object instanceof ExpectedConstructor
    );
  }

  function usesTopRealmDefaultPrototype() {
    function TopNewTarget() {}
    TopNewTarget.prototype = undefined;
    const object = Reflect.construct(
      constructorRealm.WebAssembly.Module,
      [bytes],
      TopNewTarget
    );
    return (
      Object.getPrototypeOf(object) === WebAssembly.Module.prototype &&
      object instanceof WebAssembly.Module
    );
  }

  function ignoresReplacedRealmNamespace() {
    const newTarget = freshNewTarget(undefined);
    const namespace = newTargetRealm.WebAssembly;
    const intrinsicPrototype = namespace.Module.prototype;
    newTargetRealm.WebAssembly = { Module: { prototype: {} } };
    try {
      const object = Reflect.construct(
        constructorRealm.WebAssembly.Module,
        [bytes],
        newTarget
      );
      return Object.getPrototypeOf(object) === intrinsicPrototype;
    } finally {
      newTargetRealm.WebAssembly = namespace;
    }
  }

  return JSON.stringify({
    namespaceIsSeparate:
      constructorRealm.WebAssembly !== WebAssembly &&
      newTargetRealm.WebAssembly !== WebAssembly,
    constructorFunctionPrototype:
      Object.getPrototypeOf(constructorRealm.WebAssembly.Module) ===
      constructorRealm.Function.prototype,
    moduleDirect: usesNewTargetDefaultPrototype("Module", bytes, freshNewTarget(undefined)),
    moduleBound: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      bindOther.call(freshNewTarget(null))
    ),
    moduleProxy: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      new ProxyOther(freshNewTarget(true), {})
    ),
    moduleProxyBound: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      new ProxyOther(bindOther.call(freshNewTarget(NaN)), {})
    ),
    memoryProxy: usesNewTargetDefaultPrototype(
      "Memory",
      { initial: 0 },
      new ProxyOther(freshNewTarget(false), {})
    ),
    compileErrorDirect: usesNewTargetDefaultPrototype(
      "CompileError",
      undefined,
      freshNewTarget(undefined)
    ),
    moduleTopRealm: usesTopRealmDefaultPrototype(),
    moduleIntrinsicSurvivesNamespaceReplacement: ignoresReplacedRealmNamespace()
  });
})()
"#,
        )
        .expect("child WebAssembly NewTarget realm regression should evaluate");

    assert_eq!(
        result,
        r#"{"namespaceIsSeparate":true,"constructorFunctionPrototype":true,"moduleDirect":true,"moduleBound":true,"moduleProxy":true,"moduleProxyBound":true,"memoryProxy":true,"compileErrorDirect":true,"moduleTopRealm":true,"moduleIntrinsicSurvivesNamespaceReplacement":true}"#
    );
}
#[test]
fn targeted_anchor_click_reports_same_document_hash_change_for_child_window() {
    let mut vm = new_storage_test_vm("https://targeted-child-hash.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.name = 'target';
const root = document.body || document.documentElement || document;
root.appendChild(frame);
frame.contentWindow.history.pushState(null, '', '/child.html');
const link = document.createElement('a');
link.href = '/child.html#next';
link.target = 'target';
root.appendChild(link);
let seen = [];
frame.contentWindow.navigation.onnavigate = e => {
  seen.push([
    e.hashChange,
    e.destination.sameDocument,
    e.destination.url,
    e.destination.index
  ].join(','));
};
link.click();
seen.join('|')
"#,
        )
        .expect("targeted same-document anchor click should dispatch child navigate");

    assert_eq!(
        result,
        "true,true,https://targeted-child-hash.test/child.html#next,-1"
    );
}
#[test]
fn window_load_uses_original_event_after_global_constructors_are_deleted() {
    let mut vm = new_storage_test_vm("https://window-load-original-event.test/");

    vm.eval(
        r#"
(() => {
  const OriginalEvent = Event;
  const OriginalPageTransitionEvent = PageTransitionEvent;
  globalThis.__windowLifecycleEvents = [];
  addEventListener('load', event => {
    __windowLifecycleEvents.push(
      `load:${event instanceof OriginalEvent}:${event.target === document}:${event.currentTarget === window}`
    );
  });
  addEventListener('pageshow', event => {
    __windowLifecycleEvents.push(
      `pageshow:${event instanceof OriginalPageTransitionEvent}:${event.persisted}`
    );
  });
  delete globalThis.Event;
  delete globalThis.PageTransitionEvent;
})()
"#,
    )
    .expect("window lifecycle constructor deletion setup should evaluate");

    vm.dispatch_window_load_event()
        .expect("window load should use saved original constructors");

    assert_eq!(
        vm.eval("__windowLifecycleEvents.join('|')")
            .expect("window lifecycle results should evaluate"),
        "load:true:true:true|pageshow:true:false"
    );
}
#[tokio::test]
async fn main_window_indexed_child_descriptor_matches_window_semantics() {
    let mut vm = new_storage_test_vm("https://window-indexed-descriptor.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'queued';
})()
"#,
    )
    .expect("same-origin child navigation should queue");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "srcdoc child should complete before indexed Window reflection",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("srcdoc child should have a default execution context");
    vm.eval_in_child_default_context(child_context_id, "document.write('replacement')")
        .expect("post-parse child document.write should replace the child Document");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const descriptor = Object.getOwnPropertyDescriptor(window, '0');
  let strictAssignmentThrew = false;
  try {
    (() => {
      'use strict';
      window[0] = null;
    })();
  } catch (error) {
    strictAssignmentThrew = error instanceof TypeError;
  }
  return JSON.stringify({
    valueIsChild: descriptor.value === frame.contentWindow,
    writable: descriptor.writable,
    enumerable: descriptor.enumerable,
    configurable: descriptor.configurable,
    listed: Object.getOwnPropertyNames(window).includes('0'),
    strictAssignmentThrew,
    assignmentPreservedChild: window[0] === frame.contentWindow
  });
})()
"#,
        )
        .expect("main Window indexed child descriptor should evaluate");

    assert_eq!(
        result,
        r#"{"valueIsChild":true,"writable":false,"enumerable":true,"configurable":true,"listed":true,"strictAssignmentThrew":true,"assignmentPreservedChild":true}"#
    );
}
#[test]
fn same_origin_child_window_migration_to_cross_origin_installs_denied_surface() {
    let mut vm = new_storage_test_vm("https://child-cross-origin-migration.test/");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<body>same-origin</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__migratedFrame = frame;
globalThis.__migratedWindow = frame.contentWindow;
globalThis.__migratedWindow.localStorage.setItem("before", "same-origin");
frame.src = "data:text/html,<body>cross-origin</body>";
"#,
        None,
    )
    .expect("same-origin child migration setup should run");

    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const win = globalThis.__migratedWindow;
  const probe = callback => {
    try {
      const value = callback();
      return value === null ? "null" : `${typeof value}:${String(value)}`;
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  return [
    __migratedFrame.contentDocument === null,
    win === __migratedFrame.contentWindow,
    Object.getPrototypeOf(win) === null,
    Object.prototype.toString.call(win),
    probe(() => win.document),
    probe(() => win.localStorage),
    probe(() => win.sessionStorage),
    probe(() => win.trustedTypes),
    probe(() => win.location.href)
  ].join("|");
})()
"#,
        )
        .expect("migrated cross-origin window surface should evaluate");

    assert_eq!(
        result,
        "true|true|true|[object Window]|SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true"
    );
}
#[test]
fn child_window_proxy_identity_survives_cross_origin_round_trip() {
    let mut vm = new_storage_test_vm("https://child-window-proxy-round-trip.test/");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
frame.srcdoc = "<body>initial same-origin</body>";
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__roundTripFrame = frame;
globalThis.__roundTripWindow = frame.contentWindow;
"#,
        None,
    )
    .expect("same-origin child WindowProxy setup should run");
    vm.drain_pending_child_frame_work_for_test();
    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("initial same-origin child realm should materialize");
    vm.eval_in_child_default_context(
        initial_context_id,
        "globalThis.__retiredInnerWindowMarker = 41",
    )
    .expect("initial child inner global marker should evaluate");

    vm.exec(
        r#"
__roundTripFrame.src = "data:text/html,<body>cross-origin</body>";
"#,
        None,
    )
    .expect("cross-origin child navigation should start");
    vm.drain_pending_child_frame_work_for_test();

    let cross_origin_identity = vm
        .eval("__roundTripWindow === __roundTripFrame.contentWindow")
        .expect("cross-origin WindowProxy identity should evaluate");
    assert_eq!(cross_origin_identity, "true");

    vm.exec(
        r#"
__roundTripFrame.src = "about:blank";
"#,
        None,
    )
    .expect("same-origin child navigation should start");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
[
  __roundTripWindow === __roundTripFrame.contentWindow,
  __roundTripFrame.contentDocument !== null,
  __roundTripWindow.document === __roundTripFrame.contentDocument
].join("|")
"#,
        )
        .expect("same-origin WindowProxy restoration should evaluate");
    assert_eq!(result, "true|true|true");

    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement same-origin child realm should materialize");
    assert_ne!(replacement_context_id, initial_context_id);
    let replacement_marker = vm
        .eval_in_child_default_context(
            replacement_context_id,
            "typeof globalThis.__retiredInnerWindowMarker",
        )
        .expect("replacement child inner global marker should evaluate");
    assert_eq!(replacement_marker, "undefined");
}
#[tokio::test(flavor = "current_thread")]
async fn isolated_world_universal_access_is_enforced_by_the_central_window_access_policy() {
    const PARENT_HOST: &str = "web-platform.test";
    const CHILD_HOST: &str = "www1.web-platform.test";

    let server = StaticHttpServer::spawn(1).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let child_url = server.url_for_host(CHILD_HOST, "/child.html");
    let child_origin = child_url.origin().ascii_serialization();
    let loader = static_http_loader([server.resolve_entry(CHILD_HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__universalAccessChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize universal-access child URL")
    ))
    .expect("universal-access child URL should install");

    vm.exec(
        r#"
const tupleFrame = document.createElement("iframe");
tupleFrame.id = "tuple-frame";
globalThis.__universalAccessTupleLoaded = false;
tupleFrame.onload = () => { globalThis.__universalAccessTupleLoaded = true; };
tupleFrame.src = globalThis.__universalAccessChildUrl;
(document.body || document.documentElement || document).appendChild(tupleFrame);

const opaqueFrame = document.createElement("iframe");
opaqueFrame.id = "opaque-frame";
opaqueFrame.sandbox = "allow-scripts";
opaqueFrame.srcdoc = "<p id='opaque-secret'>opaque child</p>";
(document.body || document.documentElement || document).appendChild(opaqueFrame);
"#,
        None,
    )
    .expect("universal-access child setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__universalAccessTupleLoaded)",
        "true",
        "tuple-origin child realm should materialize",
    )
    .await;

    let denied_context_id = vm
        .create_isolated_world("origin-enforced-utility", false)
        .expect("origin-enforced isolated world should be created");
    assert_eq!(
        vm.eval_in_isolated_context(
            denied_context_id,
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  };
  return [
    probe(() => document.getElementById("tuple-frame").contentWindow.document.URL),
    probe(() => document.getElementById("opaque-frame").contentWindow.document.body.textContent)
  ].join("|");
})()
"#,
        )
        .expect("origin-enforced isolated-world probe should evaluate"),
        "SecurityError:true|SecurityError:true"
    );

    let universal_context_id = vm
        .create_isolated_world("universal-utility", true)
        .expect("universal isolated world should be created");
    let universal_result = vm
        .eval_in_isolated_context(
            universal_context_id,
            r#"
[
  document.getElementById("tuple-frame").contentWindow.document.URL,
  document.getElementById("opaque-frame").contentWindow.document.getElementById("opaque-secret").textContent
].join("|")
"#,
        )
        .expect("universal isolated-world probe should evaluate");
    assert_eq!(universal_result, format!("{child_url}|opaque child"),);

    let tuple_child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.origin == child_origin)
        .map(|realm| realm.context_id)
        .expect("tuple-origin child default realm should exist");
    assert_eq!(
        vm.eval_in_child_default_context(
            tuple_child_context_id,
            r#"
(() => {
  try {
    return parent.document.URL;
  } catch (error) {
    return `${error && error.name}:${error instanceof DOMException}`;
  }
})()
"#,
        )
        .expect("child-to-parent access probe should evaluate"),
        "SecurityError:true",
        "universal access must belong only to the requesting isolated realm"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}
#[tokio::test(flavor = "current_thread")]
async fn child_cross_origin_window_denials_use_the_child_dom_exception_realm() {
    const PARENT_HOST: &str = "web-platform.test";
    const CHILD_HOST: &str = "www1.web-platform.test";

    let server = StaticHttpServer::spawn(1).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let child_url = server.url_for_host(CHILD_HOST, "/child.html");
    let loader = static_http_loader([server.resolve_entry(CHILD_HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__crossOriginChildUrl = {};",
        serde_json::to_string(child_url.as_str()).expect("serialize cross-origin child URL")
    ))
    .expect("cross-origin child URL should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__crossOriginChildLoaded = false;
frame.onload = () => { globalThis.__crossOriginChildLoaded = true; };
frame.src = globalThis.__crossOriginChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
"#,
        None,
    )
    .expect("cross-origin child SecurityError realm setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__crossOriginChildLoaded)",
        "true",
        "cross-origin child realm should materialize",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("cross-origin child realm should materialize");

    let result = vm
        .eval_in_child_default_context(
            child_context_id,
            r#"
(() => {
  "use strict";
  const probe = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return [
        error && error.name,
        error instanceof DOMException,
        error && error.constructor === DOMException,
        error && error.code
      ].join(":");
    }
  };
  return [
    probe(() => parent.document),
    probe(() => top.localStorage),
    probe(() => parent.location.href),
    probe(() => { parent.document = null; })
  ].join("|");
})()
"#,
        )
        .expect("child cross-origin SecurityError realm probe should evaluate");

    assert_eq!(
        result,
        "SecurityError:true:true:18|SecurityError:true:true:18|SecurityError:true:true:18|SecurityError:true:true:18"
    );
    assert_eq!(server.finish_targets().await, vec!["/child.html"]);
}
#[test]
fn window_event_target_methods_have_browser_lengths() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            [
                window.addEventListener.length,
                window.removeEventListener.length,
                window.dispatchEvent.length,
                addEventListener.length,
                removeEventListener.length,
                dispatchEvent.length
            ].join("|")
            "#,
        )
        .expect("window EventTarget method lengths should evaluate");

    assert_eq!(result, "2|2|1|2|2|1");
}
#[test]
fn child_browsing_context_lookup_tolerates_document_handle_cycle() {
    let mut vm = new_storage_test_vm("https://child-lookup-cycle.test/");

    vm.eval(
        r#"
(() => {
  const iframe = document.createElement("iframe");
  iframe.name = "targetFrame";
  (document.body || document.documentElement || document).appendChild(iframe);
  return "ready";
})()
"#,
    )
    .expect("cycle lookup fixture should evaluate");

    let (document, handle) = {
        let host = vm._context_host.borrow();
        (
            host.document_handle(),
            host.child_browsing_context_handle_by_index(0)
                .expect("test iframe should have a child browsing context"),
        )
    };
    vm._context_host
        .borrow_mut()
        .set_child_browsing_context_document_handle_for_test(handle, document);

    let handles = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order();
    assert_eq!(handles, vec![handle]);
    assert_eq!(
        vm._context_host
            .borrow()
            .child_browsing_context_handle_by_name("missingFrame"),
        None
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .child_browsing_context_handle_by_name("targetFrame"),
        Some(handle)
    );
}
