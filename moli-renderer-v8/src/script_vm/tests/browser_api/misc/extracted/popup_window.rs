use super::*;

#[test]
fn window_open_empty_url_reuses_lightweight_document_without_navigation() {
    let mut vm = new_storage_test_vm("https://example.com/");
    assert_eq!(
        vm.eval(
            r#"(() => {
              const popup = open("about:blank#report", "report");
              popup.marker = 73;
              popup.document.body.textContent = "keep this document";
              const document = popup.document;
              const historyLength = popup.history.length;
              const reused = open("", "report");
              return reused === popup && reused.document === document &&
                reused.marker === 73 && reused.document.body.textContent === "keep this document" &&
                reused.location.href === "about:blank#report" && reused.history.length === historyLength;
            })()"#,
        )
        .unwrap(),
        "true"
    );
    let actions = vm.take_pending_popup_activations();
    assert_eq!(actions.len(), 2);
    assert!(actions[0].navigation_requested());
    assert!(!actions[1].navigation_requested());
    assert_eq!(actions[0].popup_id(), actions[1].popup_id());
}

#[test]
fn window_open_empty_url_preserves_special_target_document() {
    let mut vm = new_storage_test_vm("https://example.com/");
    assert_eq!(
        vm.eval(
            r#"["_self", "_parent", "_top"].every(target => {
              const before = document;
              return open("", target) === window && document === before && location.href === "https://example.com/";
            })"#,
        )
        .unwrap(),
        "true"
    );
    assert!(vm.take_pending_popup_activations().is_empty());
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
}

#[tokio::test]
async fn window_open_lightweight_named_reuse_updates_dom_opener_synchronously() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_page_task_executor_test_vm_with_loader("https://example.com/", &loader);
    vm.eval(
        r#"window.child=open('about:blank','child');
           const html='<script>opener.__siblingImmediate=open("", "child").opener===window;<\/script>';
           window.sibling=open(URL.createObjectURL(new Blob([html], {type:'text/html'})), 'sibling');"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(window.__siblingImmediate)",
        "true",
        "sibling script should observe its updated DOM opener immediately",
    )
    .await;
    assert_eq!(
        vm.eval("child.opener === sibling && child.opener !== window")
            .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn retained_popup_storage_managers_use_bound_popup_owner() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>localStorage.setItem('popup-local', 'value'); \
         opener.postMessage('storage-ready', '*');</script>"
            .to_owned(),
    ])
    .await;
    let popup_url = server.url_for_host("127.0.0.1", "/popup");
    let opener_url = server.url_for_host("localhost", "/opener");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(opener_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__popupStorageReady = false; \
         onmessage = e => __popupStorageReady = e.data === 'storage-ready'; \
         globalThis.__popupStorageOwnerPopup = open({:?}); true",
        popup_url.as_str(),
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupStorageReady)",
        "true",
        "popup storage owner response",
    )
    .await;
    assert_eq!(
        vm.eval("try { __popupStorageOwnerPopup.navigator; 'accessible'; } catch (e) { e.name; }")
            .unwrap(),
        "SecurityError"
    );

    // Capture the actual native popup's managers under its owner. Page script
    // cannot obtain these objects through a cross-origin Window, but this test
    // exercises retained receiver ownership independently of Window access.
    vm.with_default_context_scope(|scope, host_ptr| {
        let host = unsafe { &*host_ptr };
        let popup_id = host.open_lightweight_popup_ids()[0];
        let owner = crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id);
        let previous = owner.enter(scope);
        let popup = host.lightweight_popup_window(scope, popup_id).unwrap();
        let navigator = popup
            .get(scope, crate::util::v8str(scope, "navigator").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .unwrap();
        let storage = navigator
            .get(scope, crate::util::v8str(scope, "storage").into())
            .unwrap();
        let buckets = navigator
            .get(scope, crate::util::v8str(scope, "storageBuckets").into())
            .unwrap();
        owner.restore(scope, previous);
        let global = scope.get_current_context().global(scope);
        assert_eq!(
            global.set(
                scope,
                crate::util::v8str(scope, "__retainedPopupStorage").into(),
                storage
            ),
            Some(true)
        );
        assert_eq!(
            global.set(
                scope,
                crate::util::v8str(scope, "__retainedPopupBuckets").into(),
                buckets
            ),
            Some(true)
        );
        Ok(())
    })
    .unwrap();

    vm.exec(
        r#"
globalThis.__popupStorageOwnerProbe = "pending";
(async () => {
  const storage = __retainedPopupStorage;
  const buckets = __retainedPopupBuckets;
  const bucket = await buckets.open("popup-retained", { quota: 2048 });
  const popupEstimate = await storage.estimate();
  const topEstimate = await navigator.storage.estimate();
  return {
    storageIsTopStorage: storage === navigator.storage,
    bucketsIsTopBuckets: buckets === navigator.storageBuckets,
    popupUsagePositive: popupEstimate.usage > 0,
    topUsage: topEstimate.usage,
    bucketName: bucket.name,
    popupKeys: await buckets.keys(),
    topKeys: await navigator.storageBuckets.keys()
  };
})().then(
  value => { globalThis.__popupStorageOwnerProbe = JSON.stringify(value); },
  error => { globalThis.__popupStorageOwnerProbe = `error:${error && error.name}:${error && error.message}`; }
);
"#,
        None,
    )
    .expect("popup storage owner probe should schedule");

    let result = vm
        .eval("String(globalThis.__popupStorageOwnerProbe)")
        .expect("popup storage owner probe should settle");
    assert_eq!(
        result,
        r#"{"storageIsTopStorage":false,"bucketsIsTopBuckets":false,"popupUsagePositive":true,"topUsage":0,"bucketName":"popup-retained","popupKeys":["popup-retained"],"topKeys":[]}"#
    );
    assert_eq!(
        vm.storage_bucket_keys_for_test(
            &moli_storage_key::MoliStorageKey::first_party_from_url(&popup_url, None,)
                .serialized_storage_key()
        ),
        vec!["popup-retained"]
    );
    assert_eq!(
        vm.storage_bucket_keys_for_test(
            &moli_storage_key::MoliStorageKey::first_party_from_url(&opener_url, None,)
                .serialized_storage_key()
        ),
        Vec::<String>::new()
    );
    assert_eq!(server.finish_targets().await.len(), 1);
}
#[test]
fn window_open_rejects_invalid_urls_before_selecting_a_target() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const probe = target => {
                try {
                  open("https://example.com\0mozilla.org", target);
                  return "allowed";
                } catch (error) {
                  return [
                    error.name,
                    error instanceof DOMException,
                    error.code
                  ].join(":");
                }
              };
              return [probe(), probe("_self")].join("|");
            })()
            "#,
        )
        .expect("invalid window.open URL probe should evaluate");

    assert_eq!(result, "SyntaxError:true:12|SyntaxError:true:12");
}
#[test]
fn window_open_resolves_relative_urls_against_the_entry_function_realm() {
    let mut vm = new_storage_html_test_vm("https://entry-realm.test/top/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
              const parent = document.body || document.documentElement || document;
              const makeFrame = base => {
                const frame = document.createElement("iframe");
                parent.appendChild(frame);
                frame.contentDocument.open();
                frame.contentDocument.write(
                  `<!doctype html><base href="https://entry-realm.test/${base}/">`
                );
                frame.contentDocument.close();
                return frame.contentWindow;
              };
              const current = makeFrame("current");
              const relevant = makeFrame("relevant");
              const entry = makeFrame("function");
              entry.openArgs = { open: current.open, relevant };

              const makeEntryFunction = target => entry.Function(
                `window.openArgs.open.call(
                   window.openArgs.relevant,
                   "resources/window-to-open.html",
                   "${target}"
                 );`
              );
              Promise.resolve().then(makeEntryFunction("promise-entry"));

              const startFunction = makeEntryFunction("wasm-entry");
              const startModule = new WebAssembly.Module(new Uint8Array([
                0, 97, 115, 109, 1, 0, 0, 0,
                1, 4, 1, 96, 0, 0,
                2, 19, 1, 6, 109, 111, 100, 117, 108, 101,
                8, 105, 109, 112, 111, 114, 116, 101, 100, 0, 0,
                8, 1, 0
              ]));
              new WebAssembly.Instance(startModule, {
                module: { imported: startFunction }
              });
              return "queued";
            })()
            "#,
        )
        .expect("cross-realm entry window.open calls should evaluate");
    assert_eq!(result, "queued");

    let mut activations = vm.take_pending_popup_activations();
    activations.sort_by(|left, right| left.target_name().cmp(right.target_name()));
    assert_eq!(activations.len(), 2);
    assert_eq!(activations[0].target_name(), "promise-entry");
    assert_eq!(activations[1].target_name(), "wasm-entry");
    assert!(activations.iter().all(|activation| {
        activation.url() == "https://entry-realm.test/function/resources/window-to-open.html"
    }));
}
#[test]
fn window_open_about_blank_returns_lightweight_popup_window() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const popup = open("about:blank#1");
              const closeDescriptor = Object.getOwnPropertyDescriptor(popup, "close");
              const closeValue = closeDescriptor && closeDescriptor.value;
              const closeShape = [
                typeof closeValue,
                closeValue && closeValue.name,
                closeValue && closeValue.length,
                closeDescriptor && closeDescriptor.enumerable,
                closeDescriptor && closeDescriptor.configurable,
                closeDescriptor && closeDescriptor.writable,
                /\[native code\]/.test(String(closeValue))
              ].join(":");
              popup.navigation.oncurrententrychange = () => {
                popup.__unexpectedCurrentEntryChange = true;
              };
              const before = [
                String(popup),
                popup.opener === window,
                popup.location.href,
                popup.navigation.currentEntry === null,
                closeShape,
                String(popup.close())
              ].join("|");
              popup.location.href = "about:blank#2";
              return [
                before,
                popup.location.href,
                popup.__unexpectedCurrentEntryChange === true
              ].join(";");
            })()
            "##,
        )
        .expect("about:blank popup probe should evaluate");

    assert_eq!(
        result,
        "[object Window]|true|about:blank#1|true|function:close:0:false:true:true:true|undefined;about:blank#2;false"
    );
}
#[test]
fn window_open_blank_targets_expose_mutable_empty_browsing_context_names() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const implicit = open("about:blank#implicit");
              const empty = open("about:blank#empty", "");
              const blank = open("about:blank#blank", "_blank");
              const mixedBlank = open("about:blank#mixed", "_BlAnK");
              const initialNames = [implicit, empty, blank, mixedBlank]
                .map(popup => popup.name);
              const initialTypes = [implicit, empty, blank, mixedBlank]
                .map(popup => typeof popup.name);
              let conversions = 0;
              empty.name = {
                toString() {
                  conversions++;
                  return "renamed-popup";
                }
              };
              blank.name = 42;
              const reused = open("about:blank#reused", "renamed-popup");
              return JSON.stringify({
                initialNames,
                initialTypes,
                conversions,
                emptyName: empty.name,
                blankName: blank.name,
                reused: reused === empty,
                reusedName: reused.name
              });
            })()
            "##,
        )
        .expect("blank-target popup Window.name probe should evaluate");

    assert_eq!(
        result,
        r#"{"initialNames":["","","",""],"initialTypes":["string","string","string","string"],"conversions":1,"emptyName":"renamed-popup","blankName":"42","reused":true,"reusedName":"renamed-popup"}"#
    );
}
#[test]
fn window_open_existing_context_special_targets_never_enter_the_popup_carrier() {
    for target in ["_self", "_parent", "_top", "_SeLf", "_PaReNt", "_TOP"] {
        let mut vm = new_storage_test_vm("https://example.com/source");
        let result = vm
            .eval(&format!(
                "String(window.open('https://example.com/{target}', '{target}'))"
            ))
            .expect("special-target window.open should evaluate");
        assert_eq!(result, "[object Window]");
        assert!(
            vm.take_pending_popup_activations().is_empty(),
            "{target} must use existing-context navigation authority"
        );
        let navigation = vm
            .take_pending_location_navigation_with_seed()
            .unwrap_or_else(|| panic!("{target} should record existing-context navigation"));
        assert_eq!(
            navigation.url.as_str(),
            format!("https://example.com/{target}")
        );
    }
}
#[test]
fn window_open_special_target_parsing_is_case_insensitive_but_does_not_trim() {
    let mut blank_vm = new_storage_test_vm("https://example.com/source");
    assert_eq!(
        blank_vm
            .eval("String(window.open('about:blank#mixed-blank', '_BlAnK'))")
            .expect("mixed-case _blank should evaluate"),
        "[object Window]"
    );
    let blank_activations = blank_vm.take_pending_popup_activations();
    assert_eq!(blank_activations.len(), 1);
    assert_eq!(blank_activations[0].target_name(), "_BlAnK");
    assert!(
        blank_vm
            .take_pending_location_navigation_with_seed()
            .is_none(),
        "_blank must not enter existing-context navigation"
    );

    let mut named_vm = new_storage_test_vm("https://example.com/source");
    assert_eq!(
        named_vm
            .eval(
                "(() => {\
                    const spaced = window.open('about:blank#spaced-name', ' _self ');\
                    return JSON.stringify({\
                        spacedString: String(spaced),\
                        spacedName: spaced.name,\
                    });\
                })()",
            )
            .expect("whitespace-padded target name should evaluate"),
        r#"{"spacedString":"[object Window]","spacedName":" _self "}"#
    );
    let named_activations = named_vm.take_pending_popup_activations();
    assert_eq!(named_activations.len(), 1);
    assert_eq!(named_activations[0].target_name(), " _self ");
    assert!(
        named_vm
            .take_pending_location_navigation_with_seed()
            .is_none(),
        "Chromium passes the raw target to FrameTree; whitespace must not turn a name into _self"
    );
}
#[test]
fn window_open_lightweight_popup_location_assignment_uses_window_setter() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const popup = open("about:blank#initial");
              const descriptor = Object.getOwnPropertyDescriptor(popup, "location");
              popup.location = "about:blank#assigned";
              return [
                typeof descriptor.get,
                descriptor.get.name,
                typeof descriptor.set,
                descriptor.set.name,
                descriptor.enumerable,
                descriptor.configurable,
                popup.location.href
              ].join("|");
            })()
            "##,
        )
        .expect("popup Window.location assignment probe should evaluate");

    assert_eq!(
        result,
        "function|get location|function|set location|true|false|about:blank#assigned"
    );
}
#[test]
fn lightweight_popup_location_uses_its_exposed_dom_exception_constructor() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const popup = open("about:blank");
              const originalHref = popup.location.href;
              const descriptor = Object.getOwnPropertyDescriptor(popup, "DOMException");
              let thrown;
              try {
                popup.location.href = "https://example.com:notaport/common/blank.html";
              } catch (error) {
                thrown = [
                  error.name,
                  error.code,
                  error.constructor === popup.DOMException,
                  error instanceof popup.DOMException
                ].join(":");
              }
              return [
                typeof popup.DOMException,
                descriptor.enumerable,
                descriptor.configurable,
                descriptor.writable,
                thrown,
                popup.location.href === originalHref
              ].join("|");
            })()
            "##,
        )
        .expect("popup invalid Location URL probe should evaluate");

    assert_eq!(
        result,
        "function|false|true|true|SyntaxError:12:true:true|true"
    );
    assert!(
        vm.take_pending_location_navigation_with_seed().is_none(),
        "invalid popup Location navigation must not escape to the opener"
    );
}
#[tokio::test]
async fn lightweight_popup_fragment_navigation_dispatches_popstate_and_hashchange() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/", &loader);

    let after_set = vm
        .eval(
            r##"
(() => {
  const popup = open("about:blank#one");
  globalThis.__popupFragmentEvents = [];
  popup.addEventListener("popstate", event => {
    __popupFragmentEvents.push([
      "popstate",
      event.target === popup,
      event.currentTarget === popup,
      event.state === null,
      popup.location.href
    ].join(":"));
  });
  popup.addEventListener("hashchange", event => {
    __popupFragmentEvents.push([
      "hashchange",
      event.oldURL,
      event.newURL,
      event.target === popup,
      event.currentTarget === popup,
      popup.location.href
    ].join(":"));
  });
  popup.location.href = "about:blank#two";
  return [
    popup.location.href,
    __popupFragmentEvents.join("|")
  ].join("|");
})()
"##,
        )
        .expect("popup fragment navigation setup should evaluate");

    assert_eq!(
        after_set,
        "about:blank#two|popstate:true:true:true:about:blank#two"
    );

    assert!(
        !vm.has_ready_timeout(),
        "popup hashchange must not acquire a PageTimer descriptor"
    );
    assert!(
        vm.run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::HashChange,
            &loader,
        )
        .await
        .expect("popup fragment hashchange task should run")
    );
    assert_eq!(
        vm.eval("__popupFragmentEvents.join('|')")
            .expect("popup fragment events should evaluate"),
        "popstate:true:true:true:about:blank#two|hashchange:about:blank#one:about:blank#two:true:true:about:blank#two"
    );
}
#[test]
fn window_open_noopener_false_preserves_opener() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
(() => {
  const popup = open("about:blank#keeps-opener", "", "noopener=false");
  const zeroPopup = open("about:blank#zero-opener", "", "noopener=0");
  return [
    popup !== null,
    popup.opener === window,
    zeroPopup !== null,
    zeroPopup.opener === window
  ].join("|");
})()
"#,
        )
        .expect("noopener=false popup probe should evaluate");

    assert_eq!(result, "true|true|true|true");
}
#[test]
fn window_open_lightweight_popup_clones_and_isolates_session_storage() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              sessionStorage.clear();
              sessionStorage.setItem("FOO", "BAR");
              const popup = open("about:blank#session");
              const initial = popup.sessionStorage.getItem("FOO");
              sessionStorage.setItem("BAZ", "QUX");
              popup.sessionStorage.setItem("FOO", "BAR-POPUP");
              return JSON.stringify({
                distinctArea: popup.sessionStorage !== sessionStorage,
                stableArea: popup.sessionStorage === popup.sessionStorage,
                initial,
                popupFoo: popup.sessionStorage.getItem("FOO"),
                openerFoo: sessionStorage.getItem("FOO"),
                popupBaz: popup.sessionStorage.getItem("BAZ"),
                openerBaz: sessionStorage.getItem("BAZ")
              });
            })()
            "##,
        )
        .expect("popup sessionStorage clone probe should evaluate");

    assert_eq!(
        result,
        r#"{"distinctArea":true,"stableArea":true,"initial":"BAR","popupFoo":"BAR-POPUP","openerFoo":"BAR","popupBaz":null,"openerBaz":"QUX"}"#
    );
}
#[tokio::test]
async fn storage_manager_estimate_uses_lightweight_popup_session_storage_owner() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader("https://example.com/", &loader);

    let result = vm
        .eval(
            r##"
(() => {
  globalThis.__popupStorageEstimateMessages = [];
  addEventListener("message", event => {
    __popupStorageEstimateMessages.push(String(event.data));
  });
  localStorage.clear();
  sessionStorage.clear();
  sessionStorage.setItem("top", "AAAAA");
  const html = `
    <!doctype html>
    <script>
      sessionStorage.setItem("popup", "BBBBBBBBBBB");
      navigator.storage.estimate().then(
        estimate => opener.postMessage(JSON.stringify({
          usage: estimate.usage,
          indexedDB: estimate.usageDetails.indexedDB ?? null,
          topValue: sessionStorage.getItem("top"),
          popupValue: sessionStorage.getItem("popup")
        }), "*"),
        error => opener.postMessage("error:" + (error && error.name), "*")
      );
    <\/script>
  `;
  const popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  sessionStorage.setItem("top", "T");
  return String(__popupStorageEstimateMessages.length) + "|" + (popup !== null);
})()
"##,
        )
        .expect("popup StorageManager estimate setup should evaluate");
    assert_eq!(result, "0|true");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupStorageEstimateMessages.length)",
        "1",
        "popup StorageManager estimate",
    )
    .await;

    assert_eq!(
        vm.eval("__popupStorageEstimateMessages[0]")
            .expect("popup StorageManager estimate message should evaluate"),
        r#"{"usage":16,"indexedDB":null,"topValue":"AAAAA","popupValue":"BBBBBBBBBBB"}"#
    );
}
#[test]
fn window_open_named_lightweight_popup_reuses_without_recloning_session_storage() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              sessionStorage.clear();
              const popup = open("", "sessionStorageTestWindow");
              const initial = popup.sessionStorage.getItem("FOO");
              sessionStorage.setItem("FOO", "BAR");
              const reopened = open("", "sessionStorageTestWindow");
              return JSON.stringify({
                sameWindow: popup === reopened,
                popupName: popup.name,
                initial,
                openerFoo: sessionStorage.getItem("FOO"),
                popupFoo: popup.sessionStorage.getItem("FOO")
              });
            })()
            "##,
        )
        .expect("named popup sessionStorage reopen probe should evaluate");

    assert_eq!(
        result,
        r#"{"sameWindow":true,"popupName":"sessionStorageTestWindow","initial":null,"openerFoo":"BAR","popupFoo":null}"#
    );
}
#[tokio::test]
async fn window_open_named_lightweight_popup_reuse_pushes_history_and_back_traverses() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__namedPopupHistoryEvents = [];
  const firstHtml = `<!doctype html><script>
    opener.__namedPopupHistoryEvents.push("first:" + location.href);
  <\/script>`;
  const secondHtml = `<!doctype html><script>
    opener.__namedPopupHistoryEvents.push("second:" + history.length + ":" + location.href);
  <\/script>`;
  globalThis.__namedPopupFirstUrl = URL.createObjectURL(new Blob([firstHtml], { type: "text/html" }));
  globalThis.__namedPopupSecondUrl = URL.createObjectURL(new Blob([secondHtml], { type: "text/html" }));
  globalThis.__namedPopup = open(__namedPopupFirstUrl, "namedPopupHistoryWindow");
  return [
    __namedPopup.location.href === "about:blank",
    __namedPopup.history.length,
    __namedPopupHistoryEvents.length
  ].join("|");
})()
"#,
        )
        .expect("named popup history setup should evaluate");
    assert_eq!(setup, "true|1|0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__namedPopupHistoryEvents.length)",
        "1",
        "first named popup document should load",
    )
    .await;

    assert_eq!(
        vm.eval("__namedPopup.location.href === __namedPopupFirstUrl")
            .expect("first popup response URL should commit"),
        "true"
    );
    let reopened = vm
        .eval(
            r#"
(() => {
  const reopened = open(__namedPopupSecondUrl, "namedPopupHistoryWindow");
  return JSON.stringify({
    sameWindow: reopened === __namedPopup,
    hrefIsSecond: reopened.location.href === __namedPopupSecondUrl,
    historyLength: reopened.history.length,
    eventCount: __namedPopupHistoryEvents.length
  });
})()
"#,
        )
        .expect("named popup reopen should evaluate");
    assert_eq!(
        reopened,
        r#"{"sameWindow":true,"hrefIsSecond":false,"historyLength":1,"eventCount":1}"#
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__namedPopupHistoryEvents.length)",
        "2",
        "second named popup document should load",
    )
    .await;
    assert_eq!(
        vm.eval("[__namedPopup.history.length, __namedPopup.location.href === __namedPopupSecondUrl].join('|')")
            .expect("named popup history should commit with its response"),
        "2|true"
    );

    vm.eval("__namedPopup.history.back()")
        .expect("named popup history.back should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__namedPopupHistoryEvents.length)",
        "3",
        "named popup back traversal should load previous document",
    )
    .await;

    let result = vm
        .eval(
            r#"
JSON.stringify({
  hrefIsFirst: __namedPopup.location.href === __namedPopupFirstUrl,
  events: __namedPopupHistoryEvents.map(value => value.split(":")[0])
})
"#,
        )
        .expect("named popup back traversal result should evaluate");
    assert_eq!(
        result,
        r#"{"hrefIsFirst":true,"events":["first","second","first"]}"#
    );
}
#[tokio::test]
async fn window_open_popup_history_forward_from_restored_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/page.html", &loader);
    vm.eval(
        r#"
        globalThis.popupHistoryLoads = [];
        globalThis.popupHistoryRestoring = false;
        globalThis.popupFirstUrl = URL.createObjectURL(new Blob([`<!doctype html><script>
            onload = () => {
                opener.popupHistoryLoads.push('first');
                if (opener.popupHistoryRestoring) history.forward();
            };
        <\/script>`], {type:'text/html'}));
        globalThis.popupSecondUrl = URL.createObjectURL(new Blob([`<!doctype html><script>
            onload = () => opener.popupHistoryLoads.push('second');
        <\/script>`], {type:'text/html'}));
        globalThis.historyPopup = open(popupFirstUrl, 'historyForwardPopup');
        "#,
    )
    .expect("popup setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "popupHistoryLoads.join(',')",
        "first",
        "first popup load",
    )
    .await;
    vm.eval("open(popupSecondUrl, 'historyForwardPopup')")
        .expect("second popup navigation should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "popupHistoryLoads.join(',')",
        "first,second",
        "second popup load",
    )
    .await;
    vm.eval("popupHistoryRestoring = true; historyPopup.history.back()")
        .expect("popup back traversal should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "popupHistoryLoads.join(',')",
        "first,second,first,second",
        "restored popup load should be able to traverse forward",
    )
    .await;
    assert_eq!(
        vm.eval(
            "[historyPopup.location.href === popupSecondUrl, historyPopup.history.length].join('|')"
        )
        .expect("restored popup history should evaluate"),
        "true|2"
    );
}
#[tokio::test]
async fn popup_history_back_from_departing_document_keeps_first_traversal() {
    assert_popup_consecutive_history_back(true).await;
}
#[tokio::test]
async fn popup_history_back_from_opener_keeps_first_traversal() {
    assert_popup_consecutive_history_back(false).await;
}
#[tokio::test]
async fn lightweight_popup_cross_document_navigation_clears_old_onload_handler() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__popupOnloadEvents = [];
  globalThis.__popupSecondLoadDone = false;
  const firstHtml = `<!doctype html><script>
    opener.__popupOnloadEvents.push("first-script");
    window.onload = () => opener.__popupOnloadEvents.push("first-load");
    window.addEventListener("load", () => opener.__popupOnloadEvents.push("first-listener"));
  <\/script>`;
  const secondHtml = `<!doctype html><script>
    opener.__popupOnloadEvents.push("second-script:" + (window.onload === null));
    window.addEventListener("load", () => {
      opener.__popupOnloadEvents.push("second-load");
      opener.__popupSecondLoadDone = true;
    });
  <\/script>`;
  globalThis.__popupOnloadFirstUrl = URL.createObjectURL(new Blob([firstHtml], { type: "text/html" }));
  globalThis.__popupOnloadSecondUrl = URL.createObjectURL(new Blob([secondHtml], { type: "text/html" }));
  globalThis.__popupOnloadWindow = open(__popupOnloadFirstUrl, "popupOnloadClearWindow");
  return String(__popupOnloadEvents.length);
})()
"#,
        )
        .expect("popup onload clear setup should evaluate");
    assert_eq!(setup, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupOnloadEvents.length)",
        "3",
        "first popup document script and load listeners should run",
    )
    .await;

    let reopened = vm
        .eval(
            r#"
(() => {
  const reopened = open(__popupOnloadSecondUrl, "popupOnloadClearWindow");
  return [
    reopened === __popupOnloadWindow,
    reopened.history.length,
    __popupOnloadEvents.join("|")
  ].join("|");
})()
"#,
        )
        .expect("popup onload clear reopen should evaluate");
    assert_eq!(reopened, "true|1|first-script|first-load|first-listener");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupSecondLoadDone)",
        "true",
        "second popup document load listener should run",
    )
    .await;
    assert_eq!(
        vm.eval("String(__popupOnloadWindow.history.length)")
            .expect("popup response should commit its history entry"),
        "2"
    );

    let result = vm
        .eval("globalThis.__popupOnloadEvents.join('|')")
        .expect("popup onload clear events should evaluate");
    assert_eq!(
        result,
        "first-script|first-load|first-listener|second-script:true|second-load"
    );
}
#[test]
fn window_open_closed_named_lightweight_popup_creates_new_window() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              const first = open("about:blank#first", "closedNamedPopup");
              first.close();
              const second = open("about:blank#second", "closedNamedPopup");
              return JSON.stringify({
                sameWindow: first === second,
                firstClosed: first.closed,
                secondClosed: second.closed,
                secondName: second.name,
                secondHref: second.location.href
              });
            })()
            "##,
        )
        .expect("closed named popup reopen probe should evaluate");

    assert_eq!(
        result,
        r##"{"sameWindow":false,"firstClosed":true,"secondClosed":false,"secondName":"closedNamedPopup","secondHref":"about:blank#second"}"##
    );
}
#[tokio::test]
async fn window_open_noopener_lightweight_popup_uses_fresh_session_storage() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_broadcast_channel_page_test_vm_with_loader("https://example.com/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  sessionStorage.clear();
  sessionStorage.setItem("FOO", "BAR");
  globalThis.__noopenerPopupMessages = [];
  const channel = new BroadcastChannel("storage-session-window-noopener-reduced");
  channel.onmessage = event => __noopenerPopupMessages.push(event.data);
  const html = `
    <!doctype html>
    <script>
      const assertions = {
        openerIsNull: opener === null && window.opener === null,
        initial: sessionStorage.getItem("FOO")
      };
      sessionStorage.setItem("FOO", "BAR-NEWWINDOW");
      assertions.afterSet = sessionStorage.getItem("FOO");
      new BroadcastChannel("storage-session-window-noopener-reduced").postMessage(assertions);
      window.close();
    <\/script>
  `;
  const url = URL.createObjectURL(new Blob([html], { type: "text/html" }));
  const popup = open(url, "_blank", "noopener");
  return JSON.stringify({
    returnedNull: popup === null,
    openerFoo: sessionStorage.getItem("FOO"),
    messages: __noopenerPopupMessages.length
  });
})()
"#,
        )
        .expect("noopener popup setup should evaluate");

    assert_eq!(
        result,
        r#"{"returnedNull":true,"openerFoo":"BAR","messages":0}"#
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__noopenerPopupMessages.length)",
        "1",
        "noopener popup should publish its session-storage result",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
              popup: __noopenerPopupMessages[0],
              openerFoo: sessionStorage.getItem("FOO")
            })"#,
        )
        .expect("noopener popup message should evaluate"),
        r#"{"popup":{"openerIsNull":true,"initial":null,"afterSet":"BAR-NEWWINDOW"},"openerFoo":"BAR"}"#
    );
    let popups = vm.take_pending_popup_activations();
    assert_eq!(popups.len(), 1);
    assert!(matches!(
        popups[0].source(),
        crate::RendererPopupActivationSource::Window {
            window: crate::RendererWindowDocumentSource::RootFrame,
            exposes_opener: false,
            ..
        }
    ));
}
#[tokio::test]
async fn lightweight_popup_session_storage_events_do_not_fire_on_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/page.html",
        &loader,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              sessionStorage.clear();
              globalThis.__popupStorageEvents = [];
              addEventListener("storage", event => {
                __popupStorageEvents.push(`${event.key}:${event.newValue}`);
              });
              const popup = open("about:blank#session-event");
              popup.sessionStorage.setItem("k", "v");
              return __popupStorageEvents.length;
            })()
            "##,
        )
        .expect("popup sessionStorage event setup should evaluate");

    assert_eq!(result, "0");
    for _ in 0..4 {
        let _ = vm
            .run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::StorageEvent,
                &loader,
            )
            .await
            .expect("selected dispatcher should drain popup sessionStorage tasks");
    }
    assert_eq!(
        vm.eval("__popupStorageEvents.join('|')")
            .expect("popup sessionStorage event log should evaluate"),
        ""
    );
}
#[tokio::test]
async fn lightweight_popup_local_storage_events_fire_on_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/page.html",
        &loader,
    );

    let result = vm
        .eval(
            r##"
            (() => {
              localStorage.clear();
              globalThis.__popupLocalStorageEvents = [];
              addEventListener("storage", event => {
                __popupLocalStorageEvents.push({
                  key: event.key,
                  oldValue: event.oldValue,
                  newValue: event.newValue,
                  url: event.url,
                  storageArea: event.storageArea === localStorage
                });
              });
              const popup = open("about:blank#local-event");
              popup.localStorage.setItem("k", "v");
              return __popupLocalStorageEvents.length;
            })()
            "##,
        )
        .expect("popup localStorage event setup should evaluate");

    assert_eq!(result, "0");
    for _ in 0..4 {
        if vm
            .eval("__popupLocalStorageEvents.length")
            .expect("popup localStorage event length should evaluate")
            == "1"
        {
            break;
        }
        let _ = vm
            .run_one_dom_manipulation_task_executor_turn(
                PageDomManipulationTestFamily::StorageEvent,
                &loader,
            )
            .await
            .expect("selected dispatcher should drain popup localStorage tasks");
    }
    assert_eq!(
        vm.eval("JSON.stringify(__popupLocalStorageEvents)")
            .expect("popup localStorage event log should evaluate"),
        r##"[{"key":"k","oldValue":null,"newValue":"v","url":"about:blank#local-event","storageArea":true}]"##
    );
}
#[test]
fn window_open_lightweight_popup_inherits_opener_viewport_surface() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r##"
            (() => {
              for (const [name, value] of Object.entries({
                innerWidth: 320,
                innerHeight: 240,
                outerWidth: 321,
                outerHeight: 241,
                devicePixelRatio: 2.5
              })) {
                Object.defineProperty(globalThis, name, {
                  configurable: true,
                  get: () => value
                });
              }
              const popup = open("about:blank#surface");
              return JSON.stringify({
                width: popup.innerWidth,
                height: popup.innerHeight,
                outerWidth: popup.outerWidth,
                outerHeight: popup.outerHeight,
                dpr: popup.devicePixelRatio
              });
            })()
            "##,
        )
        .expect("popup viewport surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"width":320,"height":240,"outerWidth":321,"outerHeight":241,"dpr":2.5}"#
    );
}
#[tokio::test]
async fn window_open_non_about_returns_lightweight_popup_and_dispatches_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/base/page.html",
        &loader,
    );

    let result = vm
        .eval(
            r#"
            (() => {
              globalThis.__popupLoadEvents = [];
              const url = URL.createObjectURL(new Blob(["<!doctype html><title>popup</title>"], { type: "text/html" }));
              const popup = open(url);
              const initialLocation = popup.location;
              const initialNavigator = popup.navigator;
              const initialDocument = popup.document;
              popup.onload = () => {
                __popupLoadEvents.push([
                  popup.location.href.startsWith("blob:https://example.com/"),
                  popup.opener === window,
                  typeof popup.close,
                  popup.location === initialLocation,
                  popup.navigator === initialNavigator,
                  popup.document === initialDocument
                ].join("|"));
              };
              return [
                String(popup),
                popup.location.href === "about:blank",
                popup.opener === window,
                typeof popup.close
              ].join("|");
            })()
            "#,
        )
        .expect("non-about popup should be returned synchronously");

    assert_eq!(result, "[object Window]|true|true|function");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupLoadEvents.length)",
        "1",
        "non-about popup load event",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__popupLoadEvents.join(',')")
            .expect("popup load event log should evaluate"),
        "true|true|function|true|true|false"
    );
}
#[tokio::test]
async fn lightweight_popup_load_keeps_live_callback_realms_and_rejects_retired_ones() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://popup-callback-realm.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
globalThis.__popupCallbackRealmEvents = [];
const makeFrame = () => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  return frame;
};
globalThis.__popupSourceWindow = makeFrame().contentWindow;
globalThis.__popupLiveCallbackWindow = makeFrame().contentWindow;
globalThis.__popupRetiredCallbackFrame = makeFrame();
globalThis.__popupRetiredCallbackWindow = __popupRetiredCallbackFrame.contentWindow;
globalThis.__popupRetiredCallback = __popupRetiredCallbackWindow.Function(
  "parent.__popupCallbackRealmEvents.push('retired');"
);
__popupRetiredCallbackFrame.remove();
"removed"
"#,
    )
    .expect("popup callback realms should be created before retirement");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("removed callback Window should retire through Page tasks");

    assert_eq!(
        vm.eval(
            r#"
(document.body || document.documentElement || document).appendChild(__popupRetiredCallbackFrame);
String(__popupRetiredCallbackWindow !== __popupRetiredCallbackFrame.contentWindow)
"#,
        )
        .expect("reinserted iframe should expose a new Window"),
        "true"
    );

    vm.eval(
        r#"
const popupURL = URL.createObjectURL(new Blob([
  "<!doctype html><title>cross-realm callback</title>"
], { type: "text/html" }));
globalThis.__popupCallbackRealmWindow = __popupSourceWindow.open(popupURL);
__popupCallbackRealmWindow.onload = function(event) {
  __popupCallbackRealmEvents.push([
    "parent", window === top, this === __popupCallbackRealmWindow, event.target === this
  ].join(":"));
};
__popupCallbackRealmWindow.addEventListener("load", __popupRetiredCallback);
__popupCallbackRealmWindow.addEventListener("load", __popupLiveCallbackWindow.Function(
  "event",
  `parent.__popupCallbackRealmEvents.push([
    "child", window === parent.__popupLiveCallbackWindow,
    this === parent.__popupCallbackRealmWindow, event.target === this
  ].join(":"));`
));
"queued"
"#,
    )
    .expect("popup load callbacks should register across Window realms");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupCallbackRealmEvents.includes('child:true:true:true'))",
        "true",
        "popup load callback from another live Window realm",
    )
    .await;
    assert_eq!(
        vm.eval("__popupCallbackRealmEvents.join('|')")
            .expect("popup callback realm events should evaluate"),
        "parent:true:true:true|child:true:true:true",
        "live callback realms must run and reinsertion must not revive a retired callback"
    );
}
#[tokio::test]
async fn lightweight_popup_document_write_during_load_replaces_existing_body() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
            (() => {
              globalThis.__popupWriteDuringLoad = "pending";
              const html = `
                <!doctype html>
                <script>
                  onload = () => {
                    document.write("<body>Filler Text<div id='log'></div>");
                    opener.__popupWriteDuringLoad = document.body.textContent;
                  };
                <\/script>
                <body>FAIL`;
              const url = URL.createObjectURL(new Blob([html], { type: "text/html" }));
              const popup = open(url);
              return popup !== null && __popupWriteDuringLoad;
            })()
            "#,
        )
        .expect("popup document.write during load setup should evaluate");

    assert_eq!(result, "pending");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupWriteDuringLoad)",
        "Filler Text",
        "popup document.write during load",
    )
    .await;

    assert_eq!(
        vm.eval("__popupWriteDuringLoad")
            .expect("popup document.write during load result should evaluate"),
        "Filler Text"
    );
}
#[tokio::test]
async fn lightweight_popup_promise_handshake_survives_source_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://popup-message-handshake.test/page.html",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupHandshakeResult = "pending";
  let resolveReady;
  const ready = new Promise(resolve => { resolveReady = resolve; });
  let popup;
  addEventListener("message", event => {
    if (event.data === "ready") {
      resolveReady({ source: event.source, origin: event.origin });
      return;
    }
    if (event.data && event.data.kind === "response") {
      __popupHandshakeResult = JSON.stringify({
        responseOrigin: event.origin,
        sourceIsPopup: event.source === popup,
        popupClosed: popup.closed,
        sourceWasOpener: event.data.sourceWasOpener,
        requestOrigin: event.data.requestOrigin
      });
    }
  });
  const html = `<!doctype html><script>
    addEventListener("message", event => {
      event.source.postMessage({
        kind: "response",
        sourceWasOpener: event.source === opener,
        requestOrigin: event.origin
      }, event.origin);
      close();
    });
    opener.postMessage("ready", "*");
  <\/script>`;
  popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  ready.then(({ source, origin }) => {
    source.postMessage("request", origin);
  });
  return __popupHandshakeResult;
})()
"#,
        )
        .expect("popup message handshake should schedule");

    assert_eq!(result, "pending");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__popupHandshakeResult === 'pending')",
        "false",
        "popup ready/request/response handshake",
    )
    .await;

    assert_eq!(
        vm.eval("String(globalThis.__popupHandshakeResult)")
            .expect("popup message handshake result should evaluate"),
        r#"{"responseOrigin":"https://popup-message-handshake.test","sourceIsPopup":true,"popupClosed":true,"sourceWasOpener":true,"requestOrigin":"https://popup-message-handshake.test"}"#
    );
}
#[tokio::test]
async fn lightweight_popup_parent_is_self_and_parent_post_message_stays_in_popup() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupParentProbe = [];
  globalThis.__popupParentMessages = [];
  addEventListener("message", event => {
    __popupParentMessages.push(String(event.data));
  });
  const html = `
    <!doctype html>
    <script>
      opener.__popupParentProbe.push({
        parentEqSelf: parent === self,
        topEqSelf: top === self,
        openerEqWindowOpener: opener === window.opener
      });
      const send = data => {
        if (window.parent !== null) {
          window.parent.postMessage("parent:" + data, "*");
        }
        if (window.opener !== null) {
          window.opener.postMessage("opener:" + data, "*");
        }
      };
      send("one");
      send("two");
      send("three");
    <\/script>
  `;
  open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  return __popupParentMessages.length;
})()
"#,
        )
        .expect("popup parent postMessage probe should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupParentMessages.length)",
        "3",
        "popup parent messages",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify({probe: __popupParentProbe, messages: __popupParentMessages})")
            .expect("popup parent postMessage result should evaluate"),
        r#"{"probe":[{"parentEqSelf":true,"topEqSelf":true,"openerEqWindowOpener":true}],"messages":["opener:one","opener:two","opener:three"]}"#
    );
}
#[tokio::test]
async fn lightweight_popup_child_frame_can_message_popup_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupChildFrameMessages = [];
  addEventListener("message", event => {
    __popupChildFrameMessages.push(String(event.data));
  });
  const html = `
    <!doctype html>
    <script>
      opener.__popupChildFrameMessages.push("popup-start:" + Boolean(document.documentElement));
      const frame = document.createElement("iframe");
      frame.srcdoc = \`
        <script>
          const payload = [
            parent !== self,
            top === parent,
            parent.opener !== null,
            typeof parent.opener.postMessage
          ].join("|");
          parent.opener.postMessage("child:" + payload, "*");
        <\\/script>
      \`;
      (document.body || document.documentElement || document).appendChild(frame);
    <\/script>
  `;
  open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  return __popupChildFrameMessages.length;
})()
"#,
        )
        .expect("popup child frame opener setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupChildFrameMessages.length",
        "2",
        "popup child frame message",
    )
    .await;

    assert_eq!(
        vm.eval("__popupChildFrameMessages.join('|')")
            .expect("popup child frame message should evaluate"),
        "popup-start:true|child:true|true|true|function"
    );
}
#[tokio::test]
async fn lightweight_popup_external_child_frame_can_message_popup_opener() {
    let (child_url, server) = spawn_popup_external_child_frame_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child URL should serialize");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupExternalChildFrameMessages = [];
  addEventListener("message", event => {{
    __popupExternalChildFrameMessages.push(String(event.data));
  }});
    const html = `
      <!doctype html>
      <iframe src={child_url_literal}></iframe>
      <script>opener.__popupExternalChildFrameMessages.push("popup-start");<\/script>
    `;
  open(URL.createObjectURL(new Blob([html], {{ type: "text/html" }})));
  return __popupExternalChildFrameMessages.length;
}})()
"#
        ))
        .expect("popup external child frame setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupExternalChildFrameMessages.length",
        "2",
        "popup external child frame message",
    )
    .await;

    assert_eq!(
        vm.eval("__popupExternalChildFrameMessages.join('|')")
            .expect("popup external child frame message should evaluate"),
        "popup-start|external:true|true|function"
    );
    server
        .await
        .expect("popup external child frame server should finish");
}
#[tokio::test]
async fn lightweight_popup_load_waits_for_external_child_frame_and_focus_handlers() {
    let (child_url, server) = spawn_popup_external_child_frame_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);
    let child_url_literal = serde_json::to_string(&child_url).expect("child URL should serialize");

    let result = vm
        .eval(&format!(
            r##"
(() => {{
  globalThis.__popupChildLoadFocusEvents = [];
  addEventListener("message", event => {{
    __popupChildLoadFocusEvents.push(String(event.data));
  }});
  const html = `
    <!doctype html>
    <input id="popup-input">
    <iframe src={child_url_literal}></iframe>
    <script>
      const frame = document.querySelector("iframe");
      frame.addEventListener("load", () => {{
        opener.__popupChildLoadFocusEvents.push("iframe-load");
      }});
      window.addEventListener("load", () => {{
        opener.__popupChildLoadFocusEvents.push("popup-load");
        frame.focus();
        document.querySelector("#popup-input").focus();
      }});
    <\/script>
  `;
  open(URL.createObjectURL(new Blob([html], {{ type: "text/html" }})));
  return __popupChildLoadFocusEvents.length;
}})()
"##
        ))
        .expect("popup child load/focus setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupChildLoadFocusEvents.length)",
        "5",
        "popup child load/focus events",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  iframeBeforePopup: __popupChildLoadFocusEvents.indexOf("iframe-load") < __popupChildLoadFocusEvents.indexOf("popup-load"),
  popupBeforeFocus: __popupChildLoadFocusEvents.indexOf("popup-load") < __popupChildLoadFocusEvents.indexOf("child-window-focus"),
  focusBeforeBlur: __popupChildLoadFocusEvents.indexOf("child-window-focus") < __popupChildLoadFocusEvents.indexOf("child-window-blur"),
  externalLoadObserved: __popupChildLoadFocusEvents.some(value => value.startsWith("external:"))
})"#
        )
        .expect("popup child load/focus ordering should evaluate"),
        r#"{"iframeBeforePopup":true,"popupBeforeFocus":true,"focusBeforeBlur":true,"externalLoadObserved":true}"#
    );
    server
        .await
        .expect("popup external child frame server should finish");
}
#[tokio::test]
async fn lightweight_popup_post_message_interleaves_opener_promise_waiters() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupWatcherLog = [];
  let waiting = "first";
  addEventListener("message", event => {
    if (!waiting) {
      __popupWatcherLog.push("unexpected:" + event.data);
      return;
    }
    __popupWatcherLog.push("message:" + event.data + ":" + waiting);
    waiting = "";
    Promise.resolve().then(() => {
      __popupWatcherLog.push("microtask-after:" + event.data);
      if (__popupWatcherLog.filter(item => item.startsWith("message:")).length < 3) {
        waiting = "next";
      }
    });
  });
  const html = `
    <!doctype html>
    <script>
      function post_message(data) {
        if (window.parent !== null) {
          window.parent.postMessage(data, { targetOrigin: "*" });
        }
        if (window.opener !== null) {
          window.opener.postMessage(data, { targetOrigin: "*" });
        }
      }
      Promise.reject(new DOMException("", "SecurityError"))
        .catch(error => post_message("open:" + error.name));
      Promise.reject(new DOMException("", "SecurityError"))
        .catch(error => post_message("keys:" + error.name));
      Promise.reject(new DOMException("", "SecurityError"))
        .catch(error => post_message("delete:" + error.name));
    <\/script>
  `;
  open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  return __popupWatcherLog.length;
})()
"#,
        )
        .expect("popup EventWatcher-like postMessage setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupWatcherLog.filter(item => item.startsWith('message:')).length)",
        "3",
        "popup watcher messages",
    )
    .await;

    assert_eq!(
        vm.eval("JSON.stringify(__popupWatcherLog)")
            .expect("popup watcher log should evaluate"),
        r#"["message:open:SecurityError:first","microtask-after:open:SecurityError","message:keys:SecurityError:next","microtask-after:keys:SecurityError","message:delete:SecurityError:next","microtask-after:delete:SecurityError"]"#
    );
}
#[tokio::test]
async fn csp_sandbox_popup_storage_bucket_messages_reach_opener_once() {
    let (popup_url, server) = spawn_csp_sandbox_storage_bucket_popup_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__cspSandboxPopupMessages = [];
  let waiting = "first";
  addEventListener("message", event => {{
    if (!waiting) {{
      __cspSandboxPopupMessages.push("unexpected:" + event.data);
      return;
    }}
    __cspSandboxPopupMessages.push(String(event.data) + ":" + waiting);
    waiting = "";
    Promise.resolve().then(() => {{
      if (__cspSandboxPopupMessages.filter(item => !item.startsWith("unexpected:")).length < 3) {{
        waiting = "next";
      }}
    }});
  }});
  const popup = open({popup_url_literal});
  return String(__cspSandboxPopupMessages.length) + "|" + (popup !== null);
}})()
"#
        ))
        .expect("CSP sandbox popup storage bucket setup should evaluate");

    assert_eq!(result, "0|true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__cspSandboxPopupMessages.length",
        "3",
        "CSP sandbox popup messages",
    )
    .await;
    server
        .await
        .expect("CSP sandbox popup server should finish");

    assert_eq!(
        vm.eval("JSON.stringify(__cspSandboxPopupMessages)")
            .expect("CSP sandbox popup messages should evaluate"),
        r#"["navigator.storageBuckets.open(): REJECTED: SecurityError:first","navigator.storageBuckets.keys(): REJECTED: SecurityError:next","navigator.storageBuckets.delete(): REJECTED: SecurityError:next"]"#
    );
}
#[tokio::test]
async fn sandbox_child_about_blank_popup_reloads_self_without_escape_and_messages_top() {
    assert_sandbox_child_about_blank_popup_reloads_self_and_messages_top(
        "allow-scripts allow-popups",
        "null",
    )
    .await;
}
#[tokio::test]
async fn sandbox_child_about_blank_popup_reloads_self_with_escape_and_messages_top() {
    assert_sandbox_child_about_blank_popup_reloads_self_and_messages_top(
        "allow-scripts allow-popups allow-popups-to-escape-sandbox",
        "http://127.0.0.1",
    )
    .await;
}
#[tokio::test]
async fn sandbox_child_inside_popup_cannot_navigate_popup_top() {
    let (popup_url, server) = spawn_sandbox_child_top_navigation_popup_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__sandboxPopupTopNavigationMessages = [];
  addEventListener("message", event => {{
    __sandboxPopupTopNavigationMessages.push(String(event.data));
    event.source.close();
  }});
  window.open({popup_url_literal});
  return __sandboxPopupTopNavigationMessages.length;
}})()
"#
        ))
        .expect("sandbox popup top navigation setup should evaluate");
    assert_eq!(setup, "0");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__sandboxPopupTopNavigationMessages.length",
        "1",
        "sandbox popup top navigation message",
    )
    .await;
    server
        .await
        .expect("sandbox popup top navigation server should finish");

    assert_eq!(
        vm.eval("__sandboxPopupTopNavigationMessages.join('|')")
            .expect("sandbox popup top navigation message should evaluate"),
        "cannot navigate"
    );
}
#[tokio::test]
async fn window_open_204_popup_ignores_navigation_and_preserves_initial_empty_history() {
    let (popup_url, loaded_url, server) = spawn_lightweight_popup_204_then_loaded_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popup204Events = [];
  const popup = open({popup_url_literal});
  popup.onload = () => __popup204Events.push("load");
  globalThis.__popup204 = popup;
  return [
    popup.location.href,
    popup.history.length
  ].join("|");
}})()
"#
        ))
        .expect("204 popup setup should evaluate");
    assert_eq!(setup, "about:blank|1");

    let result = vm
        .eval(
            r#"
(() => {
  __popup204.location.href = "about:blank#foo";
  return [
    __popup204.location.href,
    __popup204.location.hash,
    __popup204.history.length,
    __popup204Events.join("|")
  ].join("|");
})()
"#,
        )
        .expect("204 popup fragment navigation result should evaluate");

    assert_eq!(result, "about:blank#foo|#foo|1|");

    wait_for_one_page_resource_completion_executor_test_turn(&mut vm, "popup 204 completion").await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("post-fragment popup work should use the selected-task dispatcher");
    assert_eq!(
        vm.eval("[__popup204.location.href, __popup204.location.hash, __popup204.history.length, __popup204Events.join('|')].join('|')")
            .expect("post-204 ignored popup fragment state should evaluate"),
        "about:blank#foo|#foo|1|"
    );

    let after_navigation = vm
        .eval(
            r#"
(() => {
  globalThis.__popup204Messages = [];
  addEventListener("message", event => __popup204Messages.push(event.data));
  const code = `
    window.onload = () => {
      window.opener.postMessage("loaded", "*")
    }
  `;
  const target = "resources/code-injector.html?2&pipe=sub(none)&code=" + encodeURIComponent(code);
  globalThis.__popup204RequestedUrl = new URL(target, document.URL).href;
  __popup204.location.href = target;
  return [
    __popup204.location.href,
    __popup204.history.length,
    __popup204Messages.length
  ].join("|");
})()
"#,
        )
        .expect("204 popup follow-up navigation should evaluate");
    let expected_loaded_url_prefix =
        loaded_url.trim_end_matches("loaded.html").to_owned() + "resources/code-injector.html";
    let after_navigation_parts = after_navigation.split('|').collect::<Vec<_>>();
    assert_eq!(after_navigation_parts.len(), 3);
    assert_eq!(after_navigation_parts[0], "about:blank#foo");
    assert_eq!(after_navigation_parts[1], "1");
    assert_eq!(after_navigation_parts[2], "0");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popup204Messages.length",
        "1",
        "popup 204 follow-up load message",
    )
    .await;
    server.await.expect("popup 204 server should finish");
    let requested_url = vm
        .eval("__popup204RequestedUrl")
        .expect("popup requested URL should evaluate");
    assert!(requested_url.starts_with(&expected_loaded_url_prefix));
    assert_eq!(
        vm.eval(
            "[__popup204.location.href, __popup204.history.length, __popup204Messages.join('|')].join('|')"
        )
        .expect("popup 204 loaded result should evaluate"),
        format!("{requested_url}|1|loaded")
    );
}
#[tokio::test]
async fn window_open_without_url_replaces_initial_empty_history_on_first_navigation() {
    let (origin, server) = spawn_lightweight_popup_relative_navigation_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&origin)
        .expect("popup server origin")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__initialEmptyPopupMessages = [];
  addEventListener("message", event => __initialEmptyPopupMessages.push(event.data));
  const popup = open();
  globalThis.__initialEmptyPopup = popup;
  const initial = [popup.location.href, popup.history.length].join("|");
  popup.location.href = "resources/popup.html?1";
  return [initial, __initialEmptyPopupMessages.length].join("|");
})()
"#,
        )
        .expect("initial empty popup navigation setup should evaluate");
    assert_eq!(setup, "about:blank|1|0");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__initialEmptyPopupMessages.length",
        "1",
        "initial empty popup replacement load message",
    )
    .await;
    assert_eq!(
        vm.eval(
            "[__initialEmptyPopup.location.href, __initialEmptyPopup.history.length, __initialEmptyPopupMessages.join('|')].join('|')"
        )
        .expect("initial empty popup replacement result should evaluate"),
        format!("{origin}/base/resources/popup.html?1|1|loaded")
    );

    vm.eval("__initialEmptyPopup.location.href = 'resources/popup.html?2'")
        .expect("second opener-relative popup navigation should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__initialEmptyPopupMessages.length",
        "2",
        "second opener-relative popup load message",
    )
    .await;
    assert_eq!(
        vm.eval(
            "[__initialEmptyPopup.location.href, __initialEmptyPopup.history.length, __initialEmptyPopupMessages.join('|')].join('|')"
        )
        .expect("second opener-relative popup result should evaluate"),
        format!("{origin}/base/resources/popup.html?2|2|loaded|loaded")
    );
    let request_paths = server
        .await
        .expect("popup relative navigation server should finish");
    assert_eq!(
        request_paths,
        vec![
            "/base/resources/popup.html?1".to_owned(),
            "/base/resources/popup.html?2".to_owned(),
        ]
    );
}
#[tokio::test]
async fn window_open_document_with_initial_iframe_keeps_one_joint_history_entry() {
    let (popup_url, server) = spawn_lightweight_popup_response_html_server(
        "popup initial iframe history test server",
        "popup initial iframe history",
        "",
        r#"<!doctype html>
<script>
  opener.postMessage("script:" + history.length, "*");
  onload = () => opener.postMessage("load:" + history.length, "*");
</script>
<iframe srcdoc="<!doctype html><p>child</p>"></iframe>"#,
    )
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&popup_url, &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let setup = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupInitialIframeHistory = [];
  addEventListener("message", event => {{
    __popupInitialIframeHistory.push(String(event.data));
  }});
  window.open({popup_url_literal});
  return __popupInitialIframeHistory.length;
}})()
"#
        ))
        .expect("popup initial iframe history setup should evaluate");
    assert_eq!(setup, "0");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupInitialIframeHistory.length",
        "2",
        "popup initial iframe history messages",
    )
    .await;
    server
        .await
        .expect("popup initial iframe history server should finish");
    assert_eq!(
        vm.eval("__popupInitialIframeHistory.join('|')")
            .expect("popup initial iframe history result should evaluate"),
        "script:1|load:1"
    );
}
#[tokio::test]
async fn lightweight_popup_javascript_url_navigation_runs_async_with_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__topMarker = 7;
  globalThis.__popupHrefEvents = [];
  const popup = open();
  popup.location.href = "javascript:window.opener.__popupHrefEvents.push('href:' + (window.opener.__topMarker === 7) + ':' + (document.defaultView === window));";
  __popupHrefEvents.push("after-setter");
  return [
    __popupHrefEvents.join("|"),
    popup.location.href,
    popup.opener === window
  ].join("|");
})()
"#,
    )
    .expect("popup javascript location setup should evaluate");

    assert_eq!(setup, "after-setter|about:blank|true");
    vm.drain_pending_child_frame_work_for_test();
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("popup javascript location timer should run"),
        "javascript: location navigation must enter through its exact timer turn"
    );
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("__popupHrefEvents.join('|')")
            .expect("popup href javascript URL events should evaluate"),
        "after-setter|href:true:true"
    );

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__popupOpenEvents = [];
  const popup = open("javascript:window.opener.__popupOpenEvents.push('open:' + (window.opener.__topMarker === 7) + ':' + (document.defaultView === window));");
  __popupOpenEvents.push("after-open");
  return [
    __popupOpenEvents.join("|"),
    popup.location.href,
    popup.opener === window
  ].join("|");
})()
"#,
    )
    .expect("popup javascript open setup should evaluate");

    assert_eq!(setup, "after-open|about:blank|true");
    vm.drain_pending_child_frame_work_for_test();
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("popup javascript open timer should run"),
        "javascript: open navigation must enter through its exact timer turn"
    );
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("__popupOpenEvents.join('|')")
            .expect("popup open javascript URL events should evaluate"),
        "after-open|open:true:true"
    );
}
#[tokio::test]
async fn lightweight_popup_javascript_url_string_completion_replaces_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let setup = vm
        .eval(
            r#"
(() => {
  globalThis.__stringCompletionPopup = open();
  __stringCompletionPopup.location.href =
    "javascript:'<!doctype html><main id=javascript-url-result>replaced</main>'";
  return `${__stringCompletionPopup.location.href}|${__stringCompletionPopup.document.body.textContent}`;
})()
"#,
        )
        .expect("popup javascript string completion setup should evaluate");

    assert_eq!(setup, "about:blank|");
    vm.drain_pending_child_frame_work_for_test();
    assert!(
        vm.run_next_due_timer_callback_for_test(&loader)
            .await
            .expect("popup javascript URL timer should run")
    );
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r##"JSON.stringify([
  __stringCompletionPopup.location.href,
  __stringCompletionPopup.document.querySelector("#javascript-url-result").textContent,
  __stringCompletionPopup.document.defaultView === __stringCompletionPopup,
  __stringCompletionPopup.opener === window
])"##,
        )
        .expect("popup javascript string completion should replace the document"),
        r#"["about:blank","replaced",true,true]"#
    );
}
#[tokio::test]
async fn loaded_lightweight_popup_can_replace_itself_from_javascript_url_string_completion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const html = `<!doctype html><script>
    open("javascript:'<main id=javascript-url-self-result>self replaced</main>'", "_self");
  <\/script>`;
  globalThis.__selfReplacingPopup =
    open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  return __selfReplacingPopup.document.body.textContent;
})()
"#,
        )
        .expect("self-replacing popup setup should evaluate"),
        ""
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(Boolean(__selfReplacingPopup.document.querySelector('#javascript-url-self-result')))",
        "true",
        "loaded popup javascript URL string completion",
    )
    .await;
    assert_eq!(
        vm.eval(
            "__selfReplacingPopup.document.querySelector('#javascript-url-self-result').textContent",
        )
        .expect("self-replaced popup document should remain observable"),
        "self replaced"
    );
}
#[tokio::test]
async fn popup_javascript_url_string_document_keeps_inherited_frame_src_policy() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://popup-javascript-url-frame-csp.test/",
        &loader,
    );
    vm.set_response_content_security_policies(&["frame-src 'none'".to_owned()]);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__popupJavascriptUrlFrameCspEvents = [];
  addEventListener("message", event => {
    __popupJavascriptUrlFrameCspEvents.push(String(event.data));
  });
  globalThis.__javascriptUrlCspPopup = open();
  __javascriptUrlCspPopup.addEventListener("securitypolicyviolation", event => {
    __javascriptUrlCspPopup.opener.postMessage(
      `${event.violatedDirective}:${event.blockedURI}`, "*");
  });
  const markup = '<iframe src="https://blocked-frame.test/fail.html"></iframe>';
  __javascriptUrlCspPopup.location.href = "javascript:" + JSON.stringify(markup);
  return __popupJavascriptUrlFrameCspEvents.length;
})()
"#,
        )
        .expect("popup javascript URL inherited CSP setup should evaluate"),
        "0"
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .lightweight_popup_policy_container(1)
            .expect("popup policy container")
            .response_content_security_policies,
        ["frame-src 'none'".to_owned()]
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(Boolean(__javascriptUrlCspPopup.document.querySelector('iframe')))",
        "true",
        "popup javascript URL replacement iframe",
    )
    .await;
    assert_eq!(
        vm.eval(
            r#"(() => {
  const frame = __javascriptUrlCspPopup.document.querySelector('iframe');
  return `${frame.contentWindow !== null}|${frame.contentDocument !== null}`;
})()"#,
        )
        .expect("popup javascript URL nested frame projection should be observable"),
        "true|true"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupJavascriptUrlFrameCspEvents.length)",
        "1",
        "popup javascript URL inherited frame-src violation",
    )
    .await;
    assert_eq!(
        vm.eval("__popupJavascriptUrlFrameCspEvents.join('|')")
            .expect("popup javascript URL inherited CSP result should evaluate"),
        "frame-src:https://blocked-frame.test/fail.html"
    );
    assert_eq!(
        vm.eval("__javascriptUrlCspPopup.location.href")
            .expect("popup javascript URL replacement should retain its prior URL"),
        "about:blank"
    );
}
#[tokio::test]
async fn lightweight_popup_javascript_url_uses_inline_navigation_csp_not_eval_csp() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut allowed = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/base/page.html",
        &loader,
    );
    allowed.set_response_content_security_policies(&[
        "script-src 'unsafe-hashes' 'sha256-IIiAJ8UuliU8o1qAv6CV4P3R8DeTf/v3MrsCwXW171Y='"
            .to_owned(),
    ]);

    assert_eq!(
        allowed
            .eval(
                r#"
(() => {
  globalThis.__javascriptUrlMessages = [];
  globalThis.__javascriptUrlViolations = [];
  onmessage = event => __javascriptUrlMessages.push(event.data);
  document.addEventListener("securitypolicyviolation", event => {
    __javascriptUrlViolations.push(`${event.effectiveDirective}:${event.blockedURI}`);
  });
  return open("javascript:opener.postMessage('pass', '*')") !== null;
})()
"#,
            )
            .expect("allowed javascript URL setup should evaluate"),
        "true"
    );
    for _ in 0..4 {
        if allowed
            .eval("__javascriptUrlMessages.length")
            .expect("javascript URL message count should evaluate")
            == "1"
        {
            break;
        }
        let _ = allowed
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("wait driver should advance javascript URL task");
    }
    assert_eq!(
        allowed
            .eval("JSON.stringify([__javascriptUrlMessages, __javascriptUrlViolations])")
            .expect("javascript URL CSP result should evaluate"),
        r#"[["pass"],[]]"#
    );

    let mut blocked = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/base/page.html",
        &loader,
    );
    blocked.set_response_content_security_policies(&["script-src 'none'".to_owned()]);
    assert_eq!(
        blocked
            .eval(
                r#"
(() => {
  const violations = [];
  document.addEventListener("securitypolicyviolation", event => {
    violations.push(`${event.effectiveDirective}:${event.blockedURI}`);
  });
  globalThis.__blockedJavascriptUrlViolations = violations;
  const popup = open("javascript:opener.postMessage('blocked', '*')");
  return `${popup === null}|${violations.join(',')}`;
})()
"#,
            )
            .expect("blocked javascript URL setup should evaluate"),
        "true|"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut blocked),
        1
    );
    assert_eq!(
        blocked
            .eval("globalThis.__blockedJavascriptUrlViolations.join(',')")
            .expect("queued javascript URL CSP violation should be observable"),
        "script-src-elem:inline"
    );
}
#[tokio::test]
async fn lightweight_popup_javascript_url_eval_permit_is_one_shot() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://popup-javascript-url-eval.test/",
        &loader,
    );
    vm.set_response_content_security_policies(&["script-src 'unsafe-inline'".to_owned()]);

    vm.eval(
        r#"
globalThis.__popupJavascriptUrlEvalResults = [];
onmessage = event => __popupJavascriptUrlEvalResults.push(String(event.data));
const popup = open();
popup.location.href = `javascript:
  try {
    eval("globalThis.__nestedEvalRan = true");
    opener.postMessage("nested:allowed", "*");
  } catch (error) {
    opener.postMessage("nested:" + error.name, "*");
  }
`;
"queued"
"#,
    )
    .expect("popup javascript URL nested eval should queue");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupJavascriptUrlEvalResults.length)",
        "1",
        "popup javascript URL nested eval policy result",
    )
    .await;
    assert_eq!(
        vm.eval("__popupJavascriptUrlEvalResults.sort().join('|')")
            .expect("popup javascript URL nested eval result should evaluate"),
        "nested:EvalError"
    );
}
#[tokio::test]
async fn lightweight_popup_nested_eval_uses_popup_response_csp() {
    let result = lightweight_popup_nested_eval_result(
        "script-src 'unsafe-inline' 'unsafe-eval'",
        "Content-Security-Policy: script-src 'unsafe-inline'",
        "EvalError",
    )
    .await;
    assert_eq!(result, "EvalError");
}
#[tokio::test]
async fn lightweight_popup_nested_eval_uses_popup_response_trusted_types() {
    let result = lightweight_popup_nested_eval_result(
        "script-src 'unsafe-inline' 'unsafe-eval'",
        "Content-Security-Policy: script-src 'unsafe-inline' 'unsafe-eval'; require-trusted-types-for 'script'",
        "EvalError",
    )
    .await;
    assert_eq!(result, "EvalError");
}
#[tokio::test]
async fn lightweight_popup_post_message_round_trips_with_wasm_module() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://example.com/base/page.html",
        &loader,
    );

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupMessageEvents = [];
  const module = new WebAssembly.Module(
    new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])
  );
  globalThis.__popupSentModule = module;
  const html = `<!doctype html><script>
    onmessage = event => {
      const instance = new WebAssembly.Instance(event.data.module, {});
      opener.__popupMessageEvents.push({
        target: "popup",
        module: event.data.module instanceof WebAssembly.Module,
        clone: event.data.module !== opener.__popupSentModule,
        sourceIsTop: event.source === opener,
        origin: event.origin,
        exports: Object.keys(instance.exports).length
      });
      opener.postMessage({ message: "reply", module: event.data.module }, "*");
    };
    opener.postMessage("ready", "*");
  <\/script>`;
  const popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  onmessage = event => {
    if (event.data === "ready") {
      popup.postMessage({ message: "send module", module }, "*");
      return;
    }
    const instance = new WebAssembly.Instance(event.data.module, {});
    __popupMessageEvents.push({
      target: "top",
      module: event.data.module instanceof WebAssembly.Module,
      sourceIsPopup: event.source === popup,
      origin: event.origin,
      exports: Object.keys(instance.exports).length
    });
  };
  return __popupMessageEvents.length;
})()
"#,
        )
        .expect("lightweight popup postMessage setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupMessageEvents.length",
        "2",
        "typed popup Window.postMessage roundtrip",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__popupMessageEvents)")
            .expect("popup message events should evaluate"),
        r#"[{"target":"popup","module":true,"clone":true,"sourceIsTop":true,"origin":"https://example.com","exports":0},{"target":"top","module":true,"sourceIsPopup":true,"origin":"https://example.com","exports":0}]"#
    );
}
#[tokio::test]
async fn lightweight_popup_blob_document_executes_before_load_and_handles_wasm_message() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupEvents = [];
  const html = `
    <!doctype html>
    <script>
      opener.__popupEvents.push("script:" + (document.defaultView === self) + ":" + (opener === window.opener));
      self.onmessage = ({ data }) => {
        const instance = new WebAssembly.Instance(data.module, {});
        opener.__popupEvents.push("popup:" + (data.module instanceof WebAssembly.Module) + ":" + Object.keys(instance.exports).length);
        opener.postMessage({ message: "module received", module: data.module }, "*");
      };
    <\/script>
  `;
  const popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  const module = new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
  onmessage = event => {
    const instance = new WebAssembly.Instance(event.data.module, {});
    __popupEvents.push("top:" + (event.source === popup) + ":" + (event.data.module instanceof WebAssembly.Module) + ":" + Object.keys(instance.exports).length);
  };
  popup.onload = () => {
    __popupEvents.push("load:" + (popup.document.defaultView === popup));
    popup.postMessage({ message: "send module", module }, "*");
  };
  return __popupEvents.length;
})()
"#,
        )
        .expect("popup blob document setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupEvents.length)",
        "4",
        "popup blob document and message round trip",
    )
    .await;
    assert_eq!(
        vm.eval("__popupEvents.join('|')")
            .expect("popup event log should evaluate"),
        "script:true:true|load:true|popup:true:0|top:true:true:0"
    );
}
#[tokio::test]
async fn lightweight_popup_document_script_scan_uses_native_dom_after_page_method_tamper() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupNativeScanEvents = [];
  const html = `
    <!doctype html>
    <script>
      opener.__popupNativeScanEvents.push("first");
      document.getElementsByTagName = () => {
        opener.__popupNativeScanEvents.push("getElementsByTagName");
        throw new Error("popup script runner should not call page document methods");
      };
      Element.prototype.getAttribute = function() {
        opener.__popupNativeScanEvents.push("getAttribute");
        throw new Error("popup script runner should not call page element methods");
      };
      Object.defineProperty(Node.prototype, "textContent", {
        configurable: true,
        get() {
          opener.__popupNativeScanEvents.push("textContent");
          throw new Error("popup script runner should not read page textContent");
        }
      });
    <\/script>
    <script>
      opener.__popupNativeScanEvents.push("second:" + document.currentScript.tagName);
    <\/script>
  `;
  const popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  popup.onload = () => {
    __popupNativeScanEvents.push("load");
  };
  return __popupNativeScanEvents.length;
})()
"#,
        )
        .expect("popup native scan setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupNativeScanEvents.length)",
        "3",
        "popup native script scan",
    )
    .await;
    assert_eq!(
        vm.eval("__popupNativeScanEvents.join('|')")
            .expect("popup native scan event log should evaluate"),
        "first|second:SCRIPT|load"
    );
}
#[tokio::test]
async fn lightweight_popup_external_classic_script_does_not_block_page_owner() {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind delayed popup script server");
    let addr = listener.local_addr().expect("delayed popup script address");
    let (script_requested_tx, script_requested_rx) = std::sync::mpsc::sync_channel(1);
    let (release_script_tx, release_script_rx) = std::sync::mpsc::sync_channel(1);
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};

        let mut document_stream = accept_popup_redirect_test_connection(&listener)
            .expect("accept delayed popup document request");
        let mut buffer = [0; 1024];
        let _ = document_stream
            .read(&mut buffer)
            .expect("read delayed popup document request");
        let body = r#"<!doctype html>
<script>opener.__popupAsyncScriptEvents.push("inline-before");</script>
<script src="/slow.js"></script>
<script>opener.__popupAsyncScriptEvents.push("inline-after");</script>"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        document_stream
            .write_all(response.as_bytes())
            .expect("write delayed popup document response");

        let mut script_stream = accept_popup_redirect_test_connection(&listener)
            .expect("accept delayed popup script request");
        let _ = script_stream
            .read(&mut buffer)
            .expect("read delayed popup script request");
        script_requested_tx
            .send(())
            .expect("publish delayed popup script request");
        let released_by_page_test = release_script_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .is_ok();
        let body = r#"opener.__popupAsyncScriptEvents.push("external");"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        script_stream
            .write_all(response.as_bytes())
            .expect("write delayed popup script response");
        released_by_page_test
    });

    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = format!("http://{addr}/owner.html");
    let popup_url = format!("http://{addr}/popup.html");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&document_url, &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");
    assert_eq!(
        vm.eval(&format!(
            r#"
(() => {{
  globalThis.__popupAsyncScriptEvents = [];
  const popup = open({popup_url_literal});
  popup.onload = () => __popupAsyncScriptEvents.push("load");
  return __popupAsyncScriptEvents.length;
}})()
"#
        ))
        .expect("open delayed-script popup"),
        "0"
    );

    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "delayed popup document completion",
    )
    .await;
    script_requested_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("popup external script request should start asynchronously");
    assert_eq!(
        vm.eval("JSON.stringify([1 + 1, __popupAsyncScriptEvents])")
            .expect("Page owner should remain script-responsive"),
        r#"[2,["inline-before"]]"#
    );

    assert!(
        release_script_tx.send(()).is_ok(),
        "popup document completion returned only after the script gate watchdog fired"
    );
    assert!(
        server
            .join()
            .expect("delayed popup script server should finish"),
        "popup script response was released by the watchdog, not by a responsive Page owner"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupAsyncScriptEvents.length)",
        "4",
        "delayed popup script continuation and load",
    )
    .await;
    assert_eq!(
        vm.eval("__popupAsyncScriptEvents.join('|')")
            .expect("read delayed popup script ordering"),
        "inline-before|external|inline-after|load"
    );
}
#[tokio::test]
async fn lightweight_popup_document_csp_blocks_inline_scripts() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupCspEvents = [];
  const html = `
    <!doctype html>
    <meta http-equiv="Content-Security-Policy" content="script-src 'none'">
    <script>opener.__popupCspEvents.push("script");<\/script>
  `;
  const popup = open(URL.createObjectURL(new Blob([html], { type: "text/html" })));
  popup.onload = () => {
    __popupCspEvents.push("load:" + (popup.document.defaultView === popup));
  };
  return __popupCspEvents.length;
})()
"#,
        )
        .expect("popup CSP setup should evaluate");

    assert_eq!(result, "0");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupCspEvents.length)",
        "1",
        "popup CSP load dispatch",
    )
    .await;
    assert_eq!(
        vm.eval("__popupCspEvents.join('|')")
            .expect("popup CSP event log should evaluate"),
        "load:true"
    );
}
#[tokio::test]
async fn lightweight_popup_document_response_csp_blocks_inline_scripts() {
    let (popup_url, server) = spawn_lightweight_popup_response_csp_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupResponseCspEvents = [];
  const popup = open({popup_url_literal});
  popup.onload = () => {{
    __popupResponseCspEvents.push("load:" + (popup.document.defaultView === popup));
  }};
  return __popupResponseCspEvents.length;
}})()
"#
        ))
        .expect("popup response CSP setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup response CSP completion",
    )
    .await;
    server
        .await
        .expect("popup response CSP server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupResponseCspEvents.length)",
        "1",
        "popup response CSP load event",
    )
    .await;
    assert_eq!(
        vm.eval("__popupResponseCspEvents.join('|')")
            .expect("popup response CSP event log should evaluate"),
        "load:true"
    );
}
#[tokio::test]
async fn lightweight_popup_response_csp_sandbox_without_allow_scripts_blocks_inline_scripts() {
    let (popup_url, server) = spawn_lightweight_popup_response_html_server(
        "response CSP sandbox popup test server",
        "response CSP sandbox popup",
        "Content-Security-Policy: sandbox allow-same-origin",
        r#"<!doctype html><script>opener.__popupResponseSandboxScriptEvents.push("script");</script><noscript><span id="popup-response-sandbox-fallback"></span></noscript>"#,
    )
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupResponseSandboxScriptEvents = [];
  const popup = open({popup_url_literal});
  popup.onload = () => {{
    const fallback = popup.document.createElement("div");
    fallback.innerHTML =
      "<noscript><span id=popup-sandbox-fallback></span></noscript>";
    (popup.document.body || popup.document.documentElement).appendChild(fallback);
    __popupResponseSandboxScriptEvents.push([
      "load",
      popup.document.defaultView === popup,
      popup.document.getElementById("popup-response-sandbox-fallback") !== null,
      popup.document.getElementById("popup-sandbox-fallback") !== null
    ].join(":"));
  }};
  return __popupResponseSandboxScriptEvents.length;
}})()
"#
        ))
        .expect("popup response CSP sandbox script setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup response CSP sandbox completion",
    )
    .await;
    server
        .await
        .expect("popup response CSP sandbox server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupResponseSandboxScriptEvents.length)",
        "1",
        "popup response CSP sandbox load event",
    )
    .await;
    assert_eq!(
        vm.eval("__popupResponseSandboxScriptEvents.join('|')")
            .expect("popup response CSP sandbox event log should evaluate"),
        "load:true:true:true"
    );
}
#[tokio::test]
async fn lightweight_popup_document_domain_self_assignment_uses_popup_owner_state() {
    let (popup_url, server) = spawn_lightweight_popup_document_domain_server(None).await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupDomainProbe = [];
  const popup = open({popup_url_literal});
  popup.onload = () => {{
    try {{
      const retainedDocument = popup.document;
      const initial = retainedDocument.domain;
      retainedDocument.domain = retainedDocument.domain;
      __popupDomainProbe.push(`${{initial}}|${{retainedDocument.domain}}`);
      try {{
        popup.document;
        __popupDomainProbe.push("unexpected-access");
      }} catch (error) {{
        __popupDomainProbe.push(error.name);
      }}
    }} catch (error) {{
      __popupDomainProbe.push(`${{error.name}}:${{error instanceof DOMException}}:${{error.code}}`);
    }}
  }};
  return __popupDomainProbe.length;
}})()
"#
        ))
        .expect("popup document.domain setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup document-domain completion",
    )
    .await;
    server
        .await
        .expect("popup document-domain server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupDomainProbe.length)",
        "2",
        "popup document-domain load event",
    )
    .await;

    assert_eq!(
        vm.eval("__popupDomainProbe.join('|')")
            .expect("popup document-domain result should evaluate"),
        "127.0.0.1|127.0.0.1|SecurityError"
    );
}
#[tokio::test]
async fn lightweight_popup_response_csp_sandbox_disallows_document_domain_setter() {
    let (popup_url, server) = spawn_lightweight_popup_document_domain_server(Some(
        "Content-Security-Policy: sandbox allow-scripts allow-same-origin",
    ))
    .await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupResponseSandboxDomainProbe = [];
  const popup = open({popup_url_literal});
  popup.onload = () => {{
    try {{
      const initial = popup.document.domain;
      popup.document.domain = popup.document.domain;
      __popupResponseSandboxDomainProbe.push(`${{initial}}|${{popup.document.domain}}`);
    }} catch (error) {{
      __popupResponseSandboxDomainProbe.push(`${{error.name}}:${{error instanceof DOMException}}:${{error.code}}`);
    }}
  }};
  return __popupResponseSandboxDomainProbe.length;
}})()
"#
        ))
        .expect("popup response CSP sandbox document.domain setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup response CSP sandbox document-domain completion",
    )
    .await;
    server
        .await
        .expect("popup response CSP sandbox server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupResponseSandboxDomainProbe.length)",
        "1",
        "popup response CSP sandbox document-domain load event",
    )
    .await;

    assert_eq!(
        vm.eval("__popupResponseSandboxDomainProbe.join('|')")
            .expect("popup response CSP sandbox document-domain result should evaluate"),
        "SecurityError:true:18"
    );
}
#[tokio::test]
async fn lightweight_popup_document_response_csp_blocks_external_scripts() {
    let (popup_url, server) = spawn_lightweight_popup_response_csp_external_script_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupResponseCspExternalEvents = [];
  const popup = open({popup_url_literal});
  popup.onload = () => {{
    __popupResponseCspExternalEvents.push("load:" + (popup.document.defaultView === popup));
  }};
  return __popupResponseCspExternalEvents.length;
}})()
"#
        ))
        .expect("popup response external CSP setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup response external CSP completion",
    )
    .await;
    let external_script_requested = server
        .await
        .expect("popup response external CSP server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupResponseCspExternalEvents.length)",
        "1",
        "popup response external CSP load event",
    )
    .await;

    assert!(
        !external_script_requested,
        "response CSP should block external popup scripts before fetch"
    );
    assert_eq!(
        vm.eval("__popupResponseCspExternalEvents.join('|')")
            .expect("popup response external CSP event log should evaluate"),
        "load:true"
    );
}
#[tokio::test]
async fn lightweight_popup_external_script_redirect_final_url_obeys_csp() {
    let (popup_url, final_script_url, source_server, target_server) =
        spawn_lightweight_popup_response_csp_redirect_external_script_servers().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let document_url = Url::parse(&popup_url)
        .expect("popup url")
        .join("/base/page.html")
        .expect("document url");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(document_url.as_str(), &loader);
    let popup_url_literal = serde_json::to_string(&popup_url).expect("serialize popup url");

    let result = vm
        .eval(&format!(
            r#"
(() => {{
  globalThis.__popupRedirectCspEvents = [];
  const popup = open({popup_url_literal});
  popup.addEventListener("securitypolicyviolation", event => {{
    __popupRedirectCspEvents.push([
      event.blockedURI,
      event.effectiveDirective,
      event.disposition,
      event instanceof SecurityPolicyViolationEvent
    ].join("|"));
  }});
  popup.onload = () => {{
    __popupRedirectCspEvents.push("load:" + (popup.document.defaultView === popup));
  }};
  return __popupRedirectCspEvents.length;
}})()
"#
        ))
        .expect("popup redirect final URL CSP setup should evaluate");

    assert_eq!(result, "0");
    wait_for_one_page_resource_completion_selected_task_executor_test_turn(
        &mut vm,
        &loader,
        "popup redirect CSP completion",
    )
    .await;
    source_server
        .join()
        .expect("popup redirect CSP source server should finish");
    target_server
        .join()
        .expect("popup redirect CSP target server should finish");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupRedirectCspEvents.length)",
        "2",
        "popup redirect CSP load event",
    )
    .await;

    assert_eq!(
        vm.eval("__popupRedirectCspEvents.join('||')")
            .expect("popup redirect final URL CSP event log should evaluate"),
        format!("{final_script_url}|script-src-elem|enforce|true||load:true")
    );
}

#[tokio::test]
async fn lightweight_popup_window_child_queries_stay_in_the_popup_document() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__popupFrameScopeProbe = "pending";
  const topFrame = document.createElement("iframe");
  topFrame.name = "top-child";
  document.body.appendChild(topFrame);
  topFrame.contentDocument.body.innerHTML = '<p id="top-only"></p>';

  const popupMarkup = `
    <!doctype html>
    <iframe name="popup-child" srcdoc="<p id='popup-only'></p>"></iframe>
    <script>
      addEventListener("load", () => {
        const childDocument = frames[0].document;
        opener.__popupFrameScopeProbe = JSON.stringify({
          length: window.length,
          name: frames[0].name,
          popupNode: childDocument.getElementById("popup-only") !== null,
          topNode: childDocument.getElementById("top-only") !== null
        });
      });
    <\/script>
  `;
  open(URL.createObjectURL(new Blob([popupMarkup], { type: "text/html" })));
  return __popupFrameScopeProbe;
})()
"#,
        )
        .expect("popup child-query scope setup should evaluate");

    assert_eq!(result, "pending");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupFrameScopeProbe",
        r#"{"length":1,"name":"popup-child","popupNode":true,"topNode":false}"#,
        "popup child-query document scope",
    )
    .await;
}

#[tokio::test]
async fn noopener_hyperlink_reuses_an_existing_named_popup_and_preserves_its_opener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  const popupName = "moli-noopener-named-target";
  const popup = open("about:blank", popupName);
  const targetMarkup = '<!doctype html><p id="named-target"></p>';
  const targetUrl = URL.createObjectURL(new Blob([targetMarkup], { type: "text/html" }));
  const link = document.createElement("a");
  link.rel = "noopener";
  link.target = popupName;
  link.href = targetUrl;
  document.body.appendChild(link);
  globalThis.__namedNoopenerPopup = popup;
  globalThis.__namedNoopenerTargetUrl = targetUrl;
  link.click();
  return String(popup.location.href === 'about:blank' && popup.opener === window);
})()
"#,
        )
        .expect("named noopener popup setup should evaluate");

    assert_eq!(result, "true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__namedNoopenerPopup.document.getElementById('named-target') !== null)",
        "true",
        "named noopener target navigation",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  openerPreserved: __namedNoopenerPopup.opener === window,
  namePreserved: __namedNoopenerPopup.name === "moli-noopener-named-target",
  targetCommitted: __namedNoopenerPopup.location.href === __namedNoopenerTargetUrl
})"#,
        )
        .expect("named noopener popup result should evaluate"),
        r#"{"openerPreserved":true,"namePreserved":true,"targetCommitted":true}"#
    );
}

#[test]
fn window_open_existing_named_child_sets_and_can_disown_its_opener() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.name = "named-child";
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__namedOpenerFrame = frame;
  return "created";
})()
"#,
    )
    .expect("named child opener setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const child = __namedOpenerFrame.contentWindow;
  const before = child.opener;
  const opened = open("about:blank#opened", "named-child");
  const descriptor = Object.getOwnPropertyDescriptor(child, "opener");
  const openerGet = descriptor.get;
  const openerSet = descriptor.set;
  const selected = {
    initiallyNull: before === null,
    returnedExisting: opened === child,
    openerIsTop: child.opener === window,
    directGetterIsTop: openerGet() === window,
    descriptorShape: [
      typeof openerGet,
      typeof openerSet,
      descriptor.enumerable,
      descriptor.configurable
    ].join(":")
  };

  child.opener = null;
  const disowned = {
    openerIsNull: child.opener === null,
    directGetterIsNull: openerGet() === null,
    descriptorPreserved:
      Object.getOwnPropertyDescriptor(child, "opener").get === openerGet
  };

  child.opener = "immaterial";
  const replacement = Object.getOwnPropertyDescriptor(child, "opener");
  return JSON.stringify({
    selected,
    disowned,
    replaced: {
      value: child.opener,
      directGetterIsNull: openerGet() === null,
      writable: replacement.writable,
      enumerable: replacement.enumerable,
      configurable: replacement.configurable
    }
  });
})()
"#,
        )
        .expect("named child opener semantics should evaluate");

    assert_eq!(
        result,
        r#"{"selected":{"initiallyNull":true,"returnedExisting":true,"openerIsTop":true,"directGetterIsTop":true,"descriptorShape":"function:function:true:true"},"disowned":{"openerIsNull":true,"directGetterIsNull":true,"descriptorPreserved":true},"replaced":{"value":"immaterial","directGetterIsNull":true,"writable":true,"enumerable":true,"configurable":true}}"#
    );
}

#[tokio::test]
async fn lightweight_popup_opener_accessor_preserves_and_disowns_the_underlying_relation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_page_task_executor_test_vm_with_loader("https://example.com/", &loader);

    let result = vm
        .eval(
            r#"
(() => {
  const topOpenerGet = Object.getOwnPropertyDescriptor(window, "opener").get;
  const replaced = open();
  const replacedDescriptor = Object.getOwnPropertyDescriptor(replaced, "opener");
  const replacedGet = replacedDescriptor.get;
  replaced.opener = "replacement";
  const replacementDescriptor = Object.getOwnPropertyDescriptor(replaced, "opener");

  const disowned = open();
  const disownedDescriptor = Object.getOwnPropertyDescriptor(disowned, "opener");
  const disownedGet = disownedDescriptor.get;
  disowned.opener = null;
  const descriptorAfterNull = Object.getOwnPropertyDescriptor(disowned, "opener");

  const closed = open();
  const closedGet = Object.getOwnPropertyDescriptor(closed, "opener").get;
  closed.close();
  closed.opener = "closed replacement";
  globalThis.__closedPopupOpenerGet = closedGet;
  globalThis.__closedPopupForOpenerTest = closed;

  return JSON.stringify({
    replaced: {
      accessorShape: [
        typeof replacedGet,
        typeof replacedDescriptor.set,
        replacedDescriptor.writable,
        replacedDescriptor.enumerable,
        replacedDescriptor.configurable
      ].join(":"),
      value: replaced.opener,
      boundGetterKeepsRelation: replacedGet() === window,
      borrowedGetterKeepsRelation: topOpenerGet.call(replaced) === window,
      dataShape: [
        replacementDescriptor.writable,
        replacementDescriptor.enumerable,
        replacementDescriptor.configurable
      ].join(":")
    },
    disowned: {
      valueIsNull: disowned.opener === null,
      boundGetterIsNull: disownedGet() === null,
      accessorPreserved: descriptorAfterNull.get === disownedGet
    },
    closing: {
      closedFlag: closed.closed,
      boundGetterKeepsRelation: closedGet() === window,
      value: closed.opener
    }
  });
})()
"#,
        )
        .expect("lightweight popup opener accessor semantics should evaluate");

    assert_eq!(
        result,
        r#"{"replaced":{"accessorShape":"function:function::true:true","value":"replacement","boundGetterKeepsRelation":true,"borrowedGetterKeepsRelation":true,"dataShape":"true:true:true"},"disowned":{"valueIsNull":true,"boundGetterIsNull":true,"accessorPreserved":true},"closing":{"closedFlag":true,"boundGetterKeepsRelation":true,"value":"closed replacement"}}"#
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__closedPopupOpenerGet() === null)",
        "true",
        "queued popup definite close clears the opener relation",
    )
    .await;
    assert_eq!(
        vm.eval("String(__closedPopupForOpenerTest.opener)")
            .expect("closed popup replacement opener should evaluate"),
        "closed replacement"
    );
}

#[tokio::test]
async fn top_realm_popup_load_handler_can_queue_a_timer_after_window_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  globalThis.__popupCloseHandlerState = "pending";
  globalThis.__popupCloseHandlerEvents = [];
  const targetUrl = URL.createObjectURL(new Blob([
    "<!doctype html><title>close target</title>"
  ], { type: "text/html" }));
  const popup = open(targetUrl);
  popup.onload = () => {
    popup.close();
    const timerId = setTimeout(() => {
      __popupCloseHandlerEvents.push([
        popup.closed,
        popup.opener === null
      ].join(":"));
    }, 0);
    __popupCloseHandlerState = [
      popup.closed,
      popup.opener === window,
      timerId !== 0
    ].join(":");
  };
  globalThis.__popupCloseHandlerPopup = popup;
  return __popupCloseHandlerState;
})()
"#,
        )
        .expect("popup close handler setup should evaluate"),
        "pending"
    );

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupCloseHandlerState",
        "true:true:true",
        "popup close handler synchronous state",
    )
    .await;
    assert_eq!(
        vm.eval("__popupCloseHandlerEvents.length")
            .expect("popup close handler timer count should evaluate"),
        "0",
        "the close task must remain ahead of the later timer"
    );

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__popupCloseHandlerEvents.join('|')",
        "true:true",
        "top-realm timer queued after popup close",
    )
    .await;
}

#[tokio::test]
async fn named_popup_broadcast_precedes_its_queued_close() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_page_task_executor_test_vm_with_loader("https://example.com/base/page.html", &loader);

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const popupName = "moli-noopener-close-order";
  globalThis.__namedPopupCloseOrder = "pending";
  const channel = new BroadcastChannel(popupName);
  const popup = open("about:blank", popupName);
  channel.onmessage = event => {
    __namedPopupCloseOrder = JSON.stringify({
      payload: event.data,
      openerPreserved: popup.opener === window,
      alreadyClosing: popup.closed
    });
  };
  const targetMarkup = `<!doctype html><script>
    const channel = new BroadcastChannel(window.name);
    channel.postMessage("sent-before-close");
    window.close();
  <\/script>`;
  const link = document.createElement("a");
  link.rel = "noopener";
  link.target = popupName;
  link.href = URL.createObjectURL(new Blob([targetMarkup], { type: "text/html" }));
  document.body.appendChild(link);
  globalThis.__namedPopupCloseOrderPopup = popup;
  link.click();
  return __namedPopupCloseOrder;
})()
"#,
        )
        .expect("named popup close ordering setup should evaluate"),
        "pending"
    );

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__namedPopupCloseOrder",
        r#"{"payload":"sent-before-close","openerPreserved":true,"alreadyClosing":true}"#,
        "BroadcastChannel delivery before named popup close",
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__namedPopupCloseOrderPopup.opener === null)",
        "true",
        "named popup definite close",
    )
    .await;
}



#[test]
fn window_open_initial_empty_history_push_is_a_replacement() {
    let mut vm = new_storage_test_vm("https://initial-popup-history.test/page.html");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const popup = open();
  popup.history.pushState({ step: 1 }, '', 'about:blank#pushed');
  popup.history.replaceState({ step: 2 }, '', 'about:blank#replaced');
  const result = [
    popup.location.href,
    popup.history.state.step,
    popup.history.length,
  ].join('|');
  popup.close();
  return result;
})()
"#,
        )
        .expect("initial empty popup history mutation should evaluate"),
        "about:blank#replaced|2|1"
    );
}

#[test]
fn window_open_named_reuse_before_first_commit_keeps_history_pending() {
    let mut vm = new_storage_test_vm("https://example.com/base/page.html");

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const firstUrl = URL.createObjectURL(
    new Blob(['<!doctype html><p>first</p>'], { type: 'text/html' })
  );
  const secondUrl = URL.createObjectURL(
    new Blob(['<!doctype html><p>second</p>'], { type: 'text/html' })
  );
  const popup = open(firstUrl, 'pendingNamedPopup');
  const reopened = open(secondUrl, 'pendingNamedPopup');
  const result = [
    popup === reopened,
    reopened.location.href === secondUrl,
    reopened.history.length,
  ].join('|');
  popup.close();
  URL.revokeObjectURL(firstUrl);
  URL.revokeObjectURL(secondUrl);
  return result;
})()
"#,
        )
        .expect("pending named popup reuse should evaluate"),
        "true|false|1"
    );
}

#[test]
fn response_csp_sandbox_separates_main_and_initial_popup_origins() {
    let document_url = Url::parse("https://sandboxed-popup.test/page.html").expect("document URL");
    let mut vm = new_storage_test_vm(document_url.as_str());
    assert_eq!(
        vm.eval("origin").expect("initial Window origin"),
        "https://sandboxed-popup.test"
    );

    vm.set_main_navigation_policy_container(
        crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
            &[(
                "Content-Security-Policy".to_owned(),
                b"sandbox allow-scripts allow-popups allow-popups-to-escape-sandbox".to_vec(),
            )],
            &document_url,
        ),
    );
    assert_eq!(vm.eval("origin").expect("sandboxed Window origin"), "null");
    assert_eq!(
        vm.eval("location.href").expect("sandboxed Window location"),
        "https://sandboxed-popup.test/page.html"
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
  const popup = open("about:blank");
  try {
    return popup.origin;
  } catch (error) {
    return error.name;
  }
})()"#,
        )
        .expect("initial popup origin access"),
        "SecurityError"
    );
}

#[tokio::test]
async fn sandbox_child_hyperlink_popup_inherits_sandbox() {
    assert_sandbox_child_hyperlink_popup_origin("allow-scripts allow-popups", false).await;
}

#[tokio::test]
async fn sandbox_child_hyperlink_popup_escapes_when_allowed() {
    assert_sandbox_child_hyperlink_popup_origin(
        "allow-scripts allow-popups allow-popups-to-escape-sandbox",
        true,
    )
    .await;
}
