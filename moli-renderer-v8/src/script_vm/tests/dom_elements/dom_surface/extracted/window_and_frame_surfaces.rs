use super::*;

#[test]
fn subframe_loading_resumes_after_nested_and_canceled_document_retirement() {
    let mut vm = new_storage_html_test_vm("https://scoped-frame-retirement.test/");
    vm.exec(
        r#"
        globalThis.tryNewFrame = () => {
          const frame = document.createElement('iframe');
          document.body.appendChild(frame);
          const loaded = frame.contentWindow !== null;
          frame.remove();
          return loaded;
        };
        "#,
        None,
    )
    .unwrap();
    let outer = {
        let host = vm._context_host.borrow();
        host.disable_subframe_loading_for_document_subtree(host.document_handle())
    };
    assert_eq!(vm.eval("tryNewFrame()").unwrap(), "false");
    let canceled: anyhow::Result<()> = vm.with_default_context_scope(|scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        let _inner = host.disable_subframe_loading_for_document_subtree(host.document_handle());
        let source = crate::util::v8str(scope, "tryNewFrame()");
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(
            crate::script_execution::execute_compiled_script(scope, script)
                .unwrap()
                .is_false()
        );
        Err(anyhow::anyhow!(
            "retirement canceled before replacing the Document"
        ))
    });
    assert!(canceled.is_err());
    assert_eq!(vm.eval("tryNewFrame()").unwrap(), "false");
    drop(outer);
    assert_eq!(vm.eval("tryNewFrame()").unwrap(), "true");
    vm.exec(
        "document.open();document.write('<body>new document</body>');document.close();",
        None,
    )
    .unwrap();
    assert_eq!(vm.eval("tryNewFrame()").unwrap(), "true");
}

#[test]
fn document_getter_checks_the_actual_access_context_and_preserves_function_realms() {
    let mut vm = new_storage_html_test_vm("https://document-access-context.test/");
    vm.exec("globalThis.readOwnDocument = () => document;", None)
        .unwrap();
    vm.with_default_context_scope(|scope, _| {
        let own_context = scope.get_current_context();
        let window = own_context.global(scope);
        let document_key = crate::util::v8str(scope, "document");
        let descriptor = window
            .get_own_property_descriptor(scope, document_key.into())
            .unwrap();
        let descriptor = v8::Local::<v8::Object>::try_from(descriptor).unwrap();
        let getter = descriptor
            .get(scope, crate::util::v8str(scope, "get").into())
            .unwrap();
        let document = window.get(scope, document_key.into()).unwrap();
        let read_own = window
            .get(scope, crate::util::v8str(scope, "readOwnDocument").into())
            .unwrap();
        let caller = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, caller);
        let backup = std::pin::pin!(v8::BackupIncumbentScope::new(caller));
        let _backup = backup.init();
        let global = caller.global(scope);
        for (name, value) in [
            ("nativeGetter", getter),
            ("receiver", window.into()),
            ("savedDocument", document),
            ("readOwnDocument", read_own),
        ] {
            assert_eq!(
                global.set(scope, crate::util::v8str(scope, name).into(), value),
                Some(true)
            );
        }
        let source = crate::util::v8str(
            scope,
            r#"JSON.stringify([
              nativeGetter.call(receiver) === savedDocument,
              readOwnDocument() === savedDocument
            ])"#,
        );
        let script = v8::Script::compile(scope, source, None).unwrap();
        let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        assert_eq!(value.to_rust_string_lossy(scope), r#"[true,true]"#);
        Ok(())
    })
    .unwrap();
}

#[test]
fn retired_window_named_and_indexed_access_cannot_follow_a_reinserted_iframe() {
    for cleanup_before_reinsertion in [false, true] {
        let mut vm = new_storage_html_test_vm("https://retained-window-properties.test/");
        vm.exec(
            r#"
globalThis.parentMarker = document.createElement('div');
parentMarker.id = '__retainedParentMarker';
parentMarker.textContent = 'PARENT';
document.body.appendChild(parentMarker);
globalThis.frame = document.createElement('iframe');
document.body.appendChild(frame);
globalThis.oldWindow = frame.contentWindow;
globalThis.oldDocument = oldWindow.document;
oldDocument.body.innerHTML = '<div id="__retainedOldMarker">ORIGINAL</div><iframe name="__retainedOldChild"></iframe>';
globalThis.parentLengthGetter = Object.getOwnPropertyDescriptor(window, 'length').get;
globalThis.childLengthGetter = Object.getOwnPropertyDescriptor(oldWindow, 'length').get;
globalThis.retiredProperties = function(w) {
  return JSON.stringify([
    w.__retainedParentMarker === undefined,
    w.__retainedOldMarker === undefined,
    w.__retainedNewMarker === undefined,
    w.__retainedNewChild === undefined,
    !('__retainedParentMarker' in w),
    !('__retainedNewMarker' in w),
    !('__retainedNewChild' in w),
    w.length === 0,
    w[0] === undefined,
    !('0' in w),
    !Object.prototype.hasOwnProperty.call(w, '0'),
    !Reflect.ownKeys(w).includes('0'),
    Object.getOwnPropertyDescriptor(w, '0') === undefined
  ]);
};
globalThis.readRetiredProperties = oldWindow.Function('return (' + retiredProperties.toString() + ')(window)');
"#,
            None,
        )
        .expect("live Window properties and a function in its own realm should be retained");
        assert_eq!(
            vm.eval(
                r#"JSON.stringify([
oldWindow.__retainedOldMarker === oldDocument.getElementById('__retainedOldMarker'),
oldWindow.__retainedOldChild === oldWindow[0],
oldWindow.length === 1,
parentLengthGetter.call(oldWindow) === 1,
childLengthGetter.call(window) === 1
])"#,
            )
            .expect("live Window named access and borrowed length getters should work"),
            "[true,true,true,true,true]"
        );
        vm.exec("frame.remove();", None)
            .expect("the original browsing context should be removed");

        let assert_retired_properties = |vm: &mut ScriptVm, stage: &str| {
            let expected = "[true,true,true,true,true,true,true,true,true,true,true,true,true]";
            assert_eq!(
                vm.eval("retiredProperties(oldWindow)")
                    .expect("retired Window properties should be accessible from the parent realm"),
                expected,
                "{stage}, cleanup before reinsertion: {cleanup_before_reinsertion}"
            );
            assert_eq!(
                vm.eval("readRetiredProperties()")
                    .expect("retired Window properties should be accessible from its own realm"),
                expected,
                "{stage}, cleanup before reinsertion: {cleanup_before_reinsertion}"
            );
            assert_eq!(
                vm.eval(
                    "parentLengthGetter.call(oldWindow) + '|' + childLengthGetter.call(oldWindow)"
                )
                .expect("borrowed length getters should use the receiver's execution identity"),
                "0|0",
                "{stage}, cleanup before reinsertion: {cleanup_before_reinsertion}"
            );
        };
        if cleanup_before_reinsertion {
            assert!(vm.live_child_default_runtime_realm_inventory().is_empty());
            assert_retired_properties(&mut vm, "after cleanup");
        }
        vm.exec(
            r#"
document.body.appendChild(frame);
globalThis.newWindow = frame.contentWindow;
globalThis.newDocument = frame.contentDocument;
newDocument.body.innerHTML = '<div id="__retainedNewMarker">REPLACEMENT</div><iframe name="__retainedNewChild"></iframe>';
globalThis.newChild = newWindow[0];
newChild.document.body.textContent = 'NEW CHILD';
"#,
            None,
        )
        .expect("the reinserted iframe should acquire a new document and child frame");
        vm.live_child_default_runtime_realm_inventory();
        assert_retired_properties(&mut vm, "after reinsertion");
        assert_eq!(
            vm.eval(
                r#"
(() => {
  if (oldWindow.__retainedParentMarker)
    oldWindow.__retainedParentMarker.textContent = 'WRITTEN VIA OLD WINDOW';
  if (oldWindow[0])
    oldWindow[0].document.body.textContent = 'WRITTEN VIA OLD WINDOW';
  return JSON.stringify([
    oldWindow !== newWindow,
    oldWindow.document === oldDocument,
    oldDocument.body.textContent,
    parentMarker.textContent,
    newDocument.body.textContent,
    newChild.document.body.textContent,
    newWindow.__retainedNewMarker === newDocument.getElementById('__retainedNewMarker'),
    newWindow.__retainedNewChild === newChild,
    newWindow.length === 1,
    newWindow[0] === newChild,
    parentLengthGetter.call(newWindow) === 1,
    childLengthGetter.call(newWindow) === 1,
    Object.getOwnPropertyDescriptor(newWindow, '0').value === newChild
  ]);
})()
"#,
            )
            .expect("only the live Window should expose its document names and child frames"),
            r#"[true,true,"ORIGINAL","PARENT","REPLACEMENT","NEW CHILD",true,true,true,true,true,true,true]"#,
            "cleanup before reinsertion: {cleanup_before_reinsertion}"
        );
    }
}

#[test]
fn window_document_retains_original_document_after_iframe_removal_and_reinsertion() {
    for cleanup_before_reinsertion in [false, true] {
        let mut vm = new_storage_test_vm("https://retained-window-document.test/");
        vm.exec(
            r#"
globalThis.documentFrame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(documentFrame);
globalThis.originalWindow = documentFrame.contentWindow;
globalThis.originalDocument = originalWindow.document;
globalThis.parentDocumentGetter = Object.getOwnPropertyDescriptor(window, 'document').get;
globalThis.childDocumentGetter = Object.getOwnPropertyDescriptor(originalWindow, 'document').get;
originalDocument.body.textContent = 'ORIGINAL';
documentFrame.remove();
"#,
            None,
        )
        .expect("iframe Window and Document should be retained before removal");

        let assert_original_document = |vm: &mut ScriptVm, stage: &str| {
            assert_eq!(
                vm.eval(
                    r#"JSON.stringify([
originalWindow.document === originalDocument,
parentDocumentGetter.call(originalWindow) === originalDocument,
childDocumentGetter.call(originalWindow) === originalDocument,
childDocumentGetter.call(window) === document
])"#,
                )
                .expect("retained and borrowed Document getters should evaluate"),
                "[true,true,true,true]",
                "{stage}, cleanup before reinsertion: {cleanup_before_reinsertion}"
            );
        };
        assert_original_document(&mut vm, "after removal");
        if cleanup_before_reinsertion {
            assert!(vm.live_child_default_runtime_realm_inventory().is_empty());
            assert_original_document(&mut vm, "after cleanup");
        }

        vm.exec(
            r#"
(document.body || document.documentElement || document).appendChild(documentFrame);
globalThis.replacementWindow = documentFrame.contentWindow;
globalThis.replacementDocument = documentFrame.contentDocument;
replacementDocument.body.textContent = 'REPLACEMENT';
"#,
            None,
        )
        .expect("the same iframe element should acquire a new Window and Document");
        vm.live_child_default_runtime_realm_inventory();
        assert_original_document(&mut vm, "after reinsertion");
        assert_eq!(
            vm.eval(
                r#"
(() => {
  const replacementGetter = Object.getOwnPropertyDescriptor(replacementWindow, 'document').get;
  originalWindow.document.body.textContent = 'WRITTEN VIA OLD WINDOW';
  return JSON.stringify([
    originalWindow !== replacementWindow,
    originalDocument !== replacementDocument,
    replacementWindow.document === replacementDocument,
    childDocumentGetter.call(replacementWindow) === replacementDocument,
    replacementGetter.call(originalWindow) === originalDocument,
    originalDocument.body.textContent,
    replacementDocument.body.textContent
  ]);
})()
"#,
            )
            .expect("writing through the retained Window should affect only its own Document"),
            r#"[true,true,true,true,true,"WRITTEN VIA OLD WINDOW","REPLACEMENT"]"#,
            "cleanup before reinsertion: {cleanup_before_reinsertion}"
        );
    }
}

#[test]
fn window_document_getter_follows_the_receiver_window_proxy_across_navigation() {
    let mut vm = new_storage_test_vm("https://navigated-window-document.test/");
    vm.exec(
        r#"
globalThis.documentFrame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(documentFrame);
globalThis.retainedWindow = documentFrame.contentWindow;
globalThis.originalDocument = retainedWindow.document;
globalThis.parentDocumentGetter = Object.getOwnPropertyDescriptor(window, 'document').get;
globalThis.childDocumentGetter = Object.getOwnPropertyDescriptor(retainedWindow, 'document').get;
"#,
        None,
    )
    .expect("the initial WindowProxy and document getters should be retained");
    vm.drain_pending_child_frame_work_for_test();
    vm.exec("documentFrame.srcdoc = '<body><div id=destinationMarker>DESTINATION</div><iframe name=destinationChild></iframe></body>';", None)
        .expect("same-origin cross-document navigation should start");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
retainedWindow === documentFrame.contentWindow,
originalDocument !== documentFrame.contentDocument,
retainedWindow.document === documentFrame.contentDocument,
parentDocumentGetter.call(retainedWindow) === documentFrame.contentDocument,
childDocumentGetter.call(retainedWindow) === documentFrame.contentDocument,
childDocumentGetter.call(window) === document,
retainedWindow.document.body.textContent,
retainedWindow.destinationMarker === documentFrame.contentDocument.getElementById('destinationMarker'),
retainedWindow.destinationChild === retainedWindow[0],
retainedWindow.length === 1,
'0' in retainedWindow,
Object.getOwnPropertyDescriptor(retainedWindow, '0').value === retainedWindow[0]
])"#,
        )
        .expect("retained WindowProxy and borrowed getters should read the destination Document"),
        r#"[true,true,true,true,true,true,"DESTINATION",true,true,true,true,true]"#
    );
}

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
  globalThis.IntrinsicFontFace = FontFace;
  Object.defineProperty(globalThis, 'FontFace', { get() {
    throw new Error('parser must not read the public FontFace constructor');
  }, configurable: true });
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
    fontRealm: faces.every(face => face instanceof w.IntrinsicFontFace && !(face instanceof FontFace)),
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
    boundProxyInterfaces: [
      ["Module", bytes],
      ["Instance", new WebAssembly.Module(bytes)],
      ["Memory", { initial: 0 }],
      ["Table", { element: "anyfunc", initial: 0 }],
      ["Global", { value: "i32" }],
      ["CompileError"], ["LinkError"], ["RuntimeError"]
    ].every(([name, argument]) => usesNewTargetDefaultPrototype(
      name, argument, bindOther.call(new ProxyOther(freshNewTarget(false), {}))
    )),
    moduleMixedBoundProxy: usesNewTargetDefaultPrototype(
      "Module",
      bytes,
      bindOther.call(new ProxyOther(
        bindOther.call(new ProxyOther(freshNewTarget(null), {})), {}
      ))
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
        r#"{"namespaceIsSeparate":true,"constructorFunctionPrototype":true,"moduleDirect":true,"moduleBound":true,"moduleProxy":true,"moduleProxyBound":true,"boundProxyInterfaces":true,"moduleMixedBoundProxy":true,"memoryProxy":true,"compileErrorDirect":true,"moduleTopRealm":true,"moduleIntrinsicSurvivesNamespaceReplacement":true}"#
    );
}
#[test]
fn targeted_anchor_click_reports_same_document_hash_change_for_child_window() {
    let mut vm = new_storage_html_test_vm("https://targeted-child-hash.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.name = 'target';
const root = document.body || document.documentElement || document;
root.appendChild(frame);
frame.contentWindow.history.replaceState(null, '', 'about:blank#child');
const link = document.createElement('a');
link.href = 'about:blank#next';
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

    assert_eq!(result, "true,true,about:blank#next,-1");
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
      `load:${event instanceof OriginalEvent}:${event.target === document}:${event.currentTarget === window}:${event.isTrusted}:${event.bubbles}:${event.cancelable}`
    );
  });
  addEventListener('pageshow', event => {
    __windowLifecycleEvents.push(
      `pageshow:${event instanceof OriginalPageTransitionEvent}:${event.persisted}:${event.isTrusted}:${event.bubbles}:${event.cancelable}`
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
        "load:true:true:true:true:false:false|pageshow:true:false:true:true:true"
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

#[tokio::test]
async fn child_navigation_performance_name_updates_after_iframe_src_change() {
    let mut vm = new_storage_test_vm("https://child-navigation-performance.test/page.html");
    vm.eval(
        r#"
(() => {
  globalThis.__childNavigationLoadCount = 0;
  const frame = document.createElement("iframe");
  globalThis.__childNavigationFrame = frame;
  frame.onload = () => {
    globalThis.__childNavigationLoadCount++;
  };
  frame.src = "/src/browser/tests/navigation-timing/resources/blank_page_green.html";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child navigation performance setup should evaluate");
    let loads_before_initial_dispatch = vm
        .eval("String(globalThis.__childNavigationLoadCount)")
        .expect("initial child navigation load count should evaluate");
    assert_eq!(loads_before_initial_dispatch, "0");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "initial child navigation performance load",
    )
    .await;
    let first = vm
        .eval(
            r#"
(() => {
  const frame = __childNavigationFrame;
  const entry = frame.contentWindow.performance.getEntriesByType("navigation")[0];
  const timing = frame.contentWindow.performance.timing;
  return [
    entry.name === frame.contentWindow.location.href,
    entry.name.endsWith("/blank_page_green.html"),
    globalThis.__childNavigationLoadCount,
    timing.domInteractive > timing.navigationStart,
    timing.loadEventStart >= timing.domInteractive,
    timing.loadEventEnd >= timing.loadEventStart,
    frame.contentWindow.performance.now() >=
      timing.loadEventEnd - timing.navigationStart
  ].join(":");
})()
"#,
        )
        .expect("first child navigation performance probe should evaluate");
    assert_eq!(first, "true:true:1:true:true:true:true");

    vm.eval(
        r#"__childNavigationFrame.src = "/src/browser/tests/navigation-timing/resources/blank_page_yellow.html";"#,
    )
    .expect("second child navigation should queue");
    let loads_before_second_dispatch = vm
        .eval("String(globalThis.__childNavigationLoadCount)")
        .expect("second child navigation pre-HostLoad load count should evaluate");
    assert_eq!(loads_before_second_dispatch, "1");
    run_child_navigation_commit_and_host_load_for_test(
        &mut vm,
        "second child navigation performance load",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = __childNavigationFrame;
  const entry = frame.contentWindow.performance.getEntriesByType("navigation")[0];
  return [
    entry.name === frame.contentWindow.location.href,
    entry.name.endsWith("/blank_page_yellow.html"),
    globalThis.__childNavigationLoadCount
  ].join(":");
})()
"#,
        )
        .expect("second child navigation performance probe should evaluate");

    assert_eq!(result, "true:true:2");
}

#[tokio::test]
async fn child_parser_eof_syncs_selectedcontent_for_navigation_and_document_write() {
    let mut vm = new_storage_test_vm("https://child-selectedcontent-parser.test/");
    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.srcdoc = "<select><button><selectedcontent></button><option>X";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child selectedcontent navigation setup should evaluate");
    run_child_navigation_commit_and_host_load_for_test(&mut vm, "child selectedcontent navigation")
        .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const doc = document.querySelector("iframe").contentDocument;
  const selectedcontent = doc.querySelector("selectedcontent");
  const source = doc.querySelector("option");
  return [selectedcontent.textContent, selectedcontent.firstChild !== source.firstChild].join("|");
})()
"#,
        )
        .expect("child selectedcontent navigation state should evaluate"),
        "X|true"
    );

    vm.eval(
        r#"
(() => {
  const doc = document.querySelector("iframe").contentDocument;
  doc.open();
  doc.write("<select><button><selectedcontent></button><option>x<i>i<b>ib</i>b");
  doc.close();
})()
"#,
    )
    .expect("child selectedcontent document.write setup should evaluate");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const doc = document.querySelector("iframe").contentDocument;
  const selectedcontent = doc.querySelector("selectedcontent");
  const source = doc.querySelector("option");
  return [
    selectedcontent.textContent,
    selectedcontent.innerHTML === source.innerHTML,
    selectedcontent.firstChild !== source.firstChild,
    selectedcontent.querySelector("i") !== source.querySelector("i"),
    selectedcontent.querySelectorAll("b").length
  ].join("|");
})()
"#,
        )
        .expect("child selectedcontent document.write state should evaluate"),
        "xiibb|true|true|true|2"
    );
}

#[test]
fn window_named_properties_implement_webidl_exotic_object_operations() {
    let mut vm = new_storage_test_vm("https://window-named-properties.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                "use strict";
                if (!document.documentElement) {
                    document.appendChild(document.createElement("html"));
                }
                if (!document.body) {
                    document.documentElement.appendChild(document.createElement("body"));
                }

                const frame = document.createElement("iframe");
                document.body.appendChild(frame);
                const w = frame.contentWindow;
                const wp = Object.getPrototypeOf(w.Window.prototype);
                const originalPrototype = Object.getPrototypeOf(wp);

                const named = w.document.createElement("div");
                named.id = "a";
                w.document.body.appendChild(named);
                const indexed = w.document.createElement("div");
                indexed.id = "0";
                w.document.body.appendChild(indexed);

                let differentPrototypeThrows = false;
                let childPrototypeSetterThrows = false;
                let preventExtensionsThrows = false;
                let directSetThrows = false;
                try {
                    Object.setPrototypeOf(wp, {});
                } catch (error) {
                    differentPrototypeThrows = error instanceof TypeError;
                }
                try {
                    wp.__proto__ = null;
                } catch (error) {
                    childPrototypeSetterThrows = error instanceof w.TypeError;
                }
                try {
                    Object.preventExtensions(wp);
                } catch (error) {
                    preventExtensionsThrows = error instanceof TypeError;
                }
                try {
                    wp.a = 1;
                } catch (error) {
                    directSetThrows = error instanceof TypeError;
                }

                let setterThis;
                let directSetterThis;
                Object.defineProperty(w.Object.prototype, "setterProbe", {
                    configurable: true,
                    set() { setterThis = this; }
                });
                Object.defineProperty(w.Object.prototype, "directSetterProbe", {
                    configurable: true,
                    set() { directSetterThis = this; }
                });
                Object.defineProperty(w.EventTarget.prototype, "blockedProbe", {
                    configurable: true,
                    value: 1,
                    writable: false
                });
                const receiver = Object.create(wp);
                const namedDescriptor = Object.getOwnPropertyDescriptor(wp, "a");
                const indexedDescriptor = Reflect.getOwnPropertyDescriptor(wp, 0);

                const observations = {
                    prototype: [
                        Reflect.setPrototypeOf(wp, originalPrototype),
                        !Reflect.setPrototypeOf(wp, w.Object.prototype),
                        Object.getPrototypeOf(wp) === originalPrototype,
                        differentPrototypeThrows,
                        childPrototypeSetterThrows
                    ],
                    extensibility: [
                        !Reflect.preventExtensions(wp),
                        Object.isExtensible(wp),
                        preventExtensionsThrows
                    ],
                    properties: [
                        wp.a === named,
                        wp[0] === indexed,
                        "a" in wp,
                        Reflect.has(wp, 0),
                        namedDescriptor.value === named,
                        namedDescriptor.writable && !namedDescriptor.enumerable &&
                            namedDescriptor.configurable,
                        indexedDescriptor.value === indexed,
                        indexedDescriptor.writable && !indexedDescriptor.enumerable &&
                            indexedDescriptor.configurable
                    ],
                    directMutation: [
                        !Reflect.defineProperty(wp, "a", {}),
                        !Reflect.defineProperty(wp, Symbol(), {}),
                        !Reflect.set(wp, "a", 1),
                        !Reflect.set(wp, "missing", 1),
                        !Reflect.set(wp, Symbol(), 1),
                        directSetThrows,
                        !Reflect.deleteProperty(wp, "a"),
                        !Reflect.deleteProperty(wp, "missing"),
                        !Reflect.deleteProperty(wp, Symbol.toStringTag),
                        Reflect.set(wp, "directSetterProbe", 50),
                        directSetterThis === wp
                    ],
                    receiverSet: [
                        Reflect.set(wp, "a", 10, receiver),
                        Reflect.set(wp, 0, 20, receiver),
                        Reflect.set(wp, "setterProbe", 30, receiver),
                        !Reflect.set(wp, "blockedProbe", 40, receiver),
                        receiver.a === 10,
                        receiver[0] === 20,
                        setterThis === receiver,
                        !Object.hasOwn(receiver, "setterProbe"),
                        !Object.hasOwn(receiver, "blockedProbe")
                    ],
                    keys: [
                        Object.getOwnPropertyNames(wp).length === 0,
                        Reflect.ownKeys(wp).length === 1,
                        Reflect.ownKeys(wp)[0] === Symbol.toStringTag
                    ]
                };

                delete w.Object.prototype.setterProbe;
                delete w.Object.prototype.directSetterProbe;
                delete w.EventTarget.prototype.blockedProbe;
                frame.remove();
                return JSON.stringify(observations);
            })()
            "#,
        )
        .expect("WindowProperties exotic object operations should evaluate");

    assert_eq!(
        result,
        r#"{"prototype":[true,true,true,true,true],"extensibility":[true,true,true],"properties":[true,true,true,true,true,true,true,true],"directMutation":[true,true,true,true,true,true,true,true,true,true,true],"receiverSet":[true,true,true,true,true,true,true,true,true],"keys":[true,true,true]}"#
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_window_indexed_child_exposes_live_same_origin_grandchild() {
    const TOP_HOST: &str = "top.window-indexed.test";
    const MIDDLE_HOST: &str = "middle.window-indexed.test";

    let server = StaticHttpServer::spawn(2).await;
    let top_url = server.url_for_host(TOP_HOST, "/page.html");
    let middle_url = server.url_for_host(MIDDLE_HOST, "/middle.html");
    let grandchild_url = server.url_for_host(TOP_HOST, "/grandchild.html");
    let loader = static_http_loader([
        server.resolve_entry(TOP_HOST),
        server.resolve_entry(MIDDLE_HOST),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url.as_str(), &loader);

    vm.eval(&format!(
        r#"
globalThis.__middleWindowUrl = {};
globalThis.__grandchildWindowUrl = {};
globalThis.__middleWindowLoaded = false;
globalThis.__nestedWindowLoaded = false;
addEventListener("message", event => {{
  if (event.data === "nested-window-loaded") {{
    globalThis.__nestedWindowLoaded = true;
  }}
}});
"#,
        serde_json::to_string(middle_url.as_str()).expect("serialize middle frame URL"),
        serde_json::to_string(grandchild_url.as_str()).expect("serialize grandchild frame URL")
    ))
    .expect("nested cross-origin Window URLs should install");
    vm.eval(
        r#"
(() => {
  const middle = document.createElement("iframe");
  middle.src = globalThis.__middleWindowUrl;
  middle.onload = () => { globalThis.__middleWindowLoaded = true; };
  (document.body || document.documentElement || document).appendChild(middle);
})()
"#,
    )
    .expect("cross-origin middle frame should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__middleWindowLoaded)",
        "true",
        "cross-origin middle frame should load",
    )
    .await;

    let middle_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("cross-origin middle frame realm should materialize");
    vm.eval_in_child_default_context(
        middle_context_id,
        &format!(
            r#"
(() => {{
  const nested = document.createElement("iframe");
  nested.name = "liveNested";
  nested.src = {};
  nested.onload = () => top.postMessage("nested-window-loaded", "*");
  document.body.appendChild(nested);
}})()
"#,
            serde_json::to_string(grandchild_url.as_str()).expect("serialize grandchild frame URL")
        ),
    )
    .expect("same-origin-with-top grandchild should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__nestedWindowLoaded)",
        "true",
        "same-origin-with-top grandchild should load",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const middle = frames[0];
  const grandchild = middle[0];
  const adopted = grandchild.document.adoptNode(document.createElement("button"));
  return JSON.stringify({
    middleLength: middle.length,
    grandchildIsWindow: grandchild.window === grandchild,
    grandchildTopIsTop: grandchild.top === window,
    adoptedIntoGrandchild: adopted.ownerDocument === grandchild.document,
    namedVisible: "liveNested" in middle,
    namedOwn: Object.prototype.hasOwnProperty.call(middle, "liveNested"),
    namedMatchesIndexed: middle.liveNested === grandchild,
    namedDescriptorMatches:
      Object.getOwnPropertyDescriptor(middle, "liveNested").value === grandchild
  });
})()
"#,
        )
        .expect("top should traverse the cross-origin middle Window index");
    assert_eq!(
        result,
        r#"{"middleLength":1,"grandchildIsWindow":true,"grandchildTopIsTop":true,"adoptedIntoGrandchild":true,"namedVisible":true,"namedOwn":true,"namedMatchesIndexed":true,"namedDescriptorMatches":true}"#
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/middle.html", "/grandchild.html"]
    );
}

#[test]
fn location_ancestor_origins_is_a_stable_document_list_with_a_detached_empty_list() {
    let mut vm = new_storage_test_vm("https://ancestor-origins.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const location = frame.contentWindow.location;
  const ChildDOMStringList = frame.contentWindow.DOMStringList;
  const active = location.ancestorOrigins;
  const activeAgain = location.ancestorOrigins;
  const activeSnapshot = {
    values: Array.from(active),
    sameObject: active === activeAgain,
    brand: active instanceof ChildDOMStringList,
    topRealmBrand: active instanceof DOMStringList,
    isArray: Array.isArray(active),
    length: active.length,
    item0: active.item(0),
    item1IsNull: active.item(1) === null,
    containsParent: active.contains(window.origin)
  };
  frame.remove();
  const detached = location.ancestorOrigins;

  const unreadFrame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(unreadFrame);
  const unreadLocation = unreadFrame.contentWindow.location;
  unreadFrame.remove();
  const unreadDetached = unreadLocation.ancestorOrigins;
  return JSON.stringify({
    active: activeSnapshot,
    detachedValues: Array.from(detached),
    detachedIsDifferent: detached !== active,
    detachedIsStable: detached === location.ancestorOrigins,
    detachedBrand: detached instanceof ChildDOMStringList,
    unreadDetachedValues: Array.from(unreadDetached),
    unreadDetachedIsArray: Array.isArray(unreadDetached),
    unreadDetachedConstructor: unreadDetached.constructor.name
  });
})()
"#,
        )
        .expect("Location ancestor origins lifetime probe should evaluate");

    assert_eq!(
        result,
        r#"{"active":{"values":["https://ancestor-origins.test"],"sameObject":true,"brand":true,"topRealmBrand":false,"isArray":false,"length":1,"item0":"https://ancestor-origins.test","item1IsNull":true,"containsParent":true},"detachedValues":[],"detachedIsDifferent":true,"detachedIsStable":true,"detachedBrand":true,"unreadDetachedValues":[],"unreadDetachedIsArray":false,"unreadDetachedConstructor":"DOMStringList"}"#
    );
}

#[test]
fn location_ancestor_origins_snapshots_frame_referrer_policy_per_navigation() {
    let mut vm = new_storage_test_vm("https://ancestor-policy.test/page.html");

    let initial = vm
        .eval(
            r#"
(() => {
  const mount = document.body || document.documentElement || document;

  const snapshotted = document.createElement("iframe");
  snapshotted.srcdoc = "<!doctype html><p>snapshotted</p>";
  snapshotted.referrerPolicy = "no-referrer";
  mount.appendChild(snapshotted);
  const snapshottedLocation = snapshotted.contentWindow.location;
  const snapshottedInitialList = snapshottedLocation.ancestorOrigins;
  snapshotted.referrerPolicy = "";

  const futureNavigation = document.createElement("iframe");
  futureNavigation.referrerPolicy = "no-referrer";
  mount.appendChild(futureNavigation);
  const futureLocation = futureNavigation.contentWindow.location;
  const futureInitialList = futureLocation.ancestorOrigins;
  futureNavigation.referrerPolicy = "";
  futureNavigation.srcdoc = "<!doctype html><p>future navigation</p>";

  Object.assign(globalThis, {
    __ancestorPolicySnapshottedLocation: snapshottedLocation,
    __ancestorPolicySnapshottedInitialList: snapshottedInitialList,
    __ancestorPolicyFutureLocation: futureLocation,
    __ancestorPolicyFutureInitialList: futureInitialList
  });
  return JSON.stringify({
    snapshotted: Array.from(snapshottedInitialList),
    future: Array.from(futureInitialList)
  });
})()
"#,
        )
        .expect("initial Location ancestor policy probe should evaluate");
    assert_eq!(initial, r#"{"snapshotted":["null"],"future":["null"]}"#);

    vm.drain_pending_child_frame_work_for_test();

    let committed = vm
        .eval(
            r#"
(() => {
  const snapshotted = __ancestorPolicySnapshottedLocation.ancestorOrigins;
  const future = __ancestorPolicyFutureLocation.ancestorOrigins;
  return JSON.stringify({
    snapshotted: Array.from(snapshotted),
    snapshottedNewDocumentList:
      snapshotted !== __ancestorPolicySnapshottedInitialList,
    snapshottedStable:
      snapshotted === __ancestorPolicySnapshottedLocation.ancestorOrigins,
    future: Array.from(future),
    futureNewDocumentList: future !== __ancestorPolicyFutureInitialList,
    futureStable: future === __ancestorPolicyFutureLocation.ancestorOrigins
  });
})()
"#,
        )
        .expect("committed Location ancestor policy probe should evaluate");
    assert_eq!(
        committed,
        r#"{"snapshotted":["null"],"snapshottedNewDocumentList":true,"snapshottedStable":true,"future":["https://ancestor-policy.test"],"futureNewDocumentList":true,"futureStable":true}"#
    );
}

#[test]
fn window_named_properties_numeric_names_respect_prototype_shadowing() {
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
                const frame = document.createElement("iframe");
                (document.body || document.documentElement || document).appendChild(frame);
                const probe = w => {
                    const wp = Object.getPrototypeOf(w.Window.prototype);
                    const named = w.document.createElement("div");
                    named.id = "0";
                    w.document.body.appendChild(named);
                    const checks = [];
                    const check = (value, own) => {
                        const descriptor = Object.getOwnPropertyDescriptor(wp, 0);
                        checks.push(wp[0] === value, Object.hasOwn(wp, 0) === own,
                            own ? descriptor.value === value : descriptor === undefined);
                    };
                    try {
                        check(named, true);
                        w.Object.prototype[0] = 20;
                        check(20, false);
                        w.EventTarget.prototype[0] = 30;
                        check(30, false);
                        delete w.EventTarget.prototype[0];
                        check(20, false);
                        delete w.Object.prototype[0];
                        check(named, true);

                        let calls = 0;
                        let getterThis;
                        Object.defineProperty(w.EventTarget.prototype, 0, {
                            configurable: true,
                            get() { calls++; getterThis = this; return 40; }
                        });
                        checks.push(Object.getOwnPropertyDescriptor(wp, 0) === undefined,
                            !Object.hasOwn(wp, 0), Reflect.has(wp, 0), calls === 0);
                        const receiver = Object.create(null);
                        checks.push(Reflect.get(wp, 0, receiver) === 40,
                            calls === 1, getterThis === receiver);
                    } finally {
                        delete w.Object.prototype[0];
                        delete w.EventTarget.prototype[0];
                        named.remove();
                    }
                    return checks;
                };
                const result = [probe(window), probe(frame.contentWindow)];
                frame.remove();
                return JSON.stringify(result);
            })()
            "#,
        )
        .expect("numeric Window named properties should respect prototype visibility");

    let checks: Vec<Vec<bool>> = serde_json::from_str(&result).expect("visibility probe results");
    assert_eq!(checks, vec![vec![true; 22]; 2]);
}

#[test]
fn window_named_properties_ignore_inherited_proxy_traps() {
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
                const frame = document.createElement("iframe");
                (document.body || document.documentElement || document).appendChild(frame);
                const probe = w => {
                    const wp = Object.getPrototypeOf(w.Window.prototype);
                    const prototype = w.Object.prototype;
                    const constructor = w.HTMLDivElement;
                    const elementPrototype = constructor.prototype;
                    const named = w.document.createElement("div");
                    named.id = "namedProbe";
                    w.document.body.appendChild(named);
                    const operations = [
                        ["get", () => wp.namedProbe === named &&
                            constructor.prototype === elementPrototype],
                        ["has", () => "namedProbe" in wp],
                        ["getOwnPropertyDescriptor", () =>
                            Object.getOwnPropertyDescriptor(wp, "namedProbe").value === named],
                        ["ownKeys", () => {
                            const keys = Reflect.ownKeys(wp);
                            return keys.length === 1 && keys[0] === Symbol.toStringTag;
                        }],
                        ["getPrototypeOf", () => Object.getPrototypeOf(wp) === w.EventTarget.prototype],
                        ["isExtensible", () => Reflect.isExtensible(wp)],
                        ["preventExtensions", () => !Reflect.preventExtensions(wp)]
                    ];
                    const checks = [];
                    for (const [trap, operation] of operations) {
                        for (const accessor of [false, true]) {
                            let calls = 0;
                            const poison = () => { calls++; throw new Error("inherited " + trap); };
                            const descriptor = Object.create(null);
                            descriptor.configurable = true;
                            descriptor[accessor ? "get" : "value"] = poison;
                            Object.defineProperty(prototype, trap, descriptor);
                            let passed = false;
                            try {
                                passed = operation();
                            } catch (_) {
                                // Preserve the result so every trap and both realms are checked.
                            } finally {
                                delete prototype[trap];
                            }
                            checks.push(passed && calls === 0);
                        }
                    }
                    named.remove();
                    return checks;
                };
                const result = [probe(window), probe(frame.contentWindow)];
                frame.remove();
                return JSON.stringify(result);
            })()
            "#,
        )
        .expect("WindowProperties should not observe inherited proxy traps");

    let checks: Vec<Vec<bool>> = serde_json::from_str(&result).expect("proxy trap probe results");
    assert_eq!(checks, vec![vec![true; 14]; 2]);
}
