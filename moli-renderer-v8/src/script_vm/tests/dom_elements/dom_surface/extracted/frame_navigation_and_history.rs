use super::*;

#[test]
fn document_own_enumerable_surface_matches_browser_location_shape() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const d = Object.getOwnPropertyDescriptor(document, "location");
              return JSON.stringify({
                keys: Object.keys(document),
                internalOwnNames: Object.getOwnPropertyNames(document)
                  .filter(name => name.startsWith("__moliWindowLocation")),
                ownLocation: Object.prototype.hasOwnProperty.call(document, "location"),
                locationEnumerable: !!d?.enumerable,
                locationConfigurable: !!d?.configurable,
                locationIdentity: document.location === window.location
              });
            })()
            "#,
        )
        .expect("document own enumerable surface probe should evaluate");

    assert_eq!(
        result,
        r#"{"keys":["location"],"internalOwnNames":[],"ownLocation":true,"locationEnumerable":true,"locationConfigurable":false,"locationIdentity":true}"#
    );
}
#[test]
fn top_level_lexical_bindings_can_shadow_replaceable_window_alias_names() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.exec(
        r#"
        const self = "self-shadow";
        const parent = "parent-shadow";
        const frames = "frames-shadow";
        globalThis.__aliasShadowProbe = [self, parent, frames].join("|");
        "#,
        None,
    )
    .expect("top-level lexical declarations should not conflict with replaceable aliases");

    let result = vm
        .eval("globalThis.__aliasShadowProbe")
        .expect("shadow probe should evaluate");

    assert_eq!(result, "self-shadow|parent-shadow|frames-shadow");
}
#[tokio::test]
async fn child_window_name_assignment_updates_parent_named_access() {
    let mut vm = new_storage_test_vm("https://dynamic-frame-name.test/");

    let initial = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'frame';
  frame.name = 'bar';
  (document.body || document.documentElement || document).appendChild(frame);
  return [
    'bar' in window,
    window.bar === frame.contentWindow,
    frame.contentWindow.name
  ].join('|');
})()
"#,
        )
        .expect("initial iframe browsing-context name should evaluate");
    assert_eq!(initial, "true|true|bar");

    vm.eval(
        r#"
document.getElementById('frame').srcdoc =
  "<script>window.name = 'foo'<\/script>";
"#,
    )
    .expect("child window.name assignment should queue through srcdoc navigation");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child window.name assignment srcdoc should commit before parser work",
    )
    .await;
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "child window.name assignment should run as parser script",
    )
    .await;
    run_child_document_lifecycle_and_host_load_for_test(
        &mut vm,
        "child window.name assignment srcdoc",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.getElementById('frame');
  return [
    'foo' in window,
    window.foo === frame.contentWindow,
    'bar' in window,
    window.bar === undefined,
    frame.name,
    frame.contentWindow.name
  ].join('|');
})()
"#,
        )
        .expect("dynamic iframe browsing-context name should evaluate");

    assert_eq!(result, "true|true|false|true|bar|foo");
}
#[test]
fn detached_iframe_navigation_entry_properties_are_invalidated() {
    let mut vm = new_storage_test_vm("https://detached-navigation-entry.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const entry = frame.contentWindow.navigation.currentEntry;
const before = [
  entry.sameDocument,
  entry.url !== null,
  entry.key !== '',
  entry.id !== '',
  entry.index
].join(',');
frame.remove();
const after = [
  entry.sameDocument,
  entry.url === null,
  entry.key,
  entry.id,
  entry.index
].join(',');
`${before}|${after}`
"#,
        )
        .expect("detached iframe navigation entry should evaluate");

    assert_eq!(result, "true,true,true,true,0|false,true,,,-1");
}
#[test]
fn navigation_runtime_state_ignores_proto_pollution_slots() {
    let mut vm = new_storage_test_vm("https://navigation-slot-pollution.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const accessorDescriptor = (prototype, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    const setter = descriptor?.set;
    return [
      name,
      typeof descriptor?.get,
      descriptor?.get?.name,
      descriptor?.get?.length,
      typeof setter,
      setter ? setter.name : "none",
      setter ? setter.length : "none",
      descriptor?.enumerable,
      descriptor?.configurable
    ].join(":");
  };
  const historyDescriptors = [
    accessorDescriptor(History.prototype, "length"),
    accessorDescriptor(History.prototype, "state"),
    accessorDescriptor(History.prototype, "scrollRestoration")
  ];
  const navigationDescriptors = [
    accessorDescriptor(Navigation.prototype, "canGoBack"),
    accessorDescriptor(Navigation.prototype, "canGoForward"),
    accessorDescriptor(Navigation.prototype, "currentEntry"),
    accessorDescriptor(Navigation.prototype, "activation"),
    accessorDescriptor(Navigation.prototype, "transition")
  ];
  History.prototype.__lmHistoryLength = 99;
  History.prototype.__lmHistoryState = { polluted: true };
  History.prototype.__lmHistoryScrollRestoration = "manual";
  Navigation.prototype.__lmNavigationCurrentEntry = null;
  NavigationHistoryEntry.prototype.__lmNavigationEntryInitialIndex = 41;
  NavigationHistoryEntry.prototype.__lmNavigationEntryUrl = "https://proto.invalid/#bad";
  NavigationHistoryEntry.prototype.__lmNavigationEntryStateSnapshot = { value: "proto-state" };
  const internalHistoryNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmHistory"))
    .sort();
  const internalNavigationNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmNavigation"))
    .sort();
  const internalEntryNames = entry => Object.getOwnPropertyNames(entry)
    .filter(name => name === "state" ||
      name === "__moliWindowRuntimeOwner" ||
      name.startsWith("__lmNavigationEntry") ||
      name.startsWith("__lmHistoryEntry"))
    .sort();
  const historyOwnSlots = internalHistoryNames(history);
  const navigationOwnSlots = internalNavigationNames(navigation);
  const windowHolderOwnSlots = Object.getOwnPropertyNames(window)
    .filter(name => name === "__moliWindowHistory" ||
      name === "__moliWindowNavigation")
    .sort();
  Object.defineProperties(window, {
    __moliWindowHistory: {
      value: { length: 99 },
      configurable: true
    },
    __moliWindowNavigation: {
      value: { currentEntry: null },
      configurable: true
    }
  });
  Object.defineProperties(history, {
    __lmHistoryLength: { value: 99, configurable: true },
    __lmHistoryState: { value: { polluted: true }, configurable: true },
    __lmHistoryScrollRestoration: { value: "manual", configurable: true },
    __lmHistoryEntries: { value: [], configurable: true },
    __lmHistoryIndex: { value: 41, configurable: true },
    __moliWindowRuntimeOwner: { value: null, configurable: true }
  });
  Object.defineProperties(navigation, {
    __lmNavigationCurrentEntry: { value: null, configurable: true },
    __lmNavigationActivation: { value: { polluted: true }, configurable: true },
    __lmNavigationTransition: { value: { polluted: true }, configurable: true },
    __moliWindowRuntimeOwner: { value: null, configurable: true }
  });
  const first = navigation.currentEntry;
  navigation.updateCurrentEntry({ state: { value: "real-before" } });
  const firstOwnSlots = internalEntryNames(first);
  Object.defineProperties(first, {
    __lmNavigationEntryInitialIndex: { value: 41, configurable: true },
    __lmNavigationEntryUrl: { value: "https://own.invalid/#bad", configurable: true },
    __lmNavigationEntryId: { value: "own-id", configurable: true },
    __lmNavigationEntryKey: { value: "own-key", configurable: true },
    __lmNavigationEntryStateSnapshot: { value: { value: "own-state" }, configurable: true },
    __lmHistoryEntryStateSnapshot: { value: { step: 99 }, configurable: true },
    state: { value: { value: "own-exposed-state" }, configurable: true }
  });
  const before = {
    length: history.length,
    windowHistoryStable: window.history === history,
    windowNavigationStable: window.navigation === navigation,
    windowHolderSpoofed: Object.getOwnPropertyNames(window)
      .filter(name => name === "__moliWindowHistory" ||
        name === "__moliWindowNavigation")
      .sort()
      .join(","),
    historyStateNull: history.state === null,
    scrollRestoration: history.scrollRestoration,
    currentEntryPresent: navigation.currentEntry !== null,
    transitionNull: navigation.transition === null,
    index: first.index,
    hash: new URL(first.url).hash,
    keySpoofed: first.key === "own-key",
    idSpoofed: first.id === "own-id",
    state: first.getState().value
  };
  history.pushState({ step: 1 }, "", "#one");
  const current = navigation.currentEntry;
  navigation.updateCurrentEntry({ state: { value: "real-after" } });
  const currentOwnSlots = internalEntryNames(current);
  Object.defineProperties(current, {
    __lmNavigationEntryInitialIndex: { value: 41, configurable: true },
    __lmNavigationEntryUrl: { value: "https://own.invalid/#bad", configurable: true },
    __lmNavigationEntryId: { value: "own-current-id", configurable: true },
    __lmNavigationEntryKey: { value: "own-current-key", configurable: true },
    __lmNavigationEntryStateSnapshot: { value: { value: "own-current-state" }, configurable: true },
    state: { value: { value: "own-current-exposed-state" }, configurable: true }
  });
  const after = {
    length: history.length,
    historyStateStep: history.state.step,
    scrollRestoration: history.scrollRestoration,
    currentEntryPresent: navigation.currentEntry !== null,
    transitionNull: navigation.transition === null,
    index: current.index,
    indexes: navigation.entries().map(entry => entry.index).join(","),
    hash: new URL(current.url).hash,
    keySpoofed: current.key === "own-current-key",
    idSpoofed: current.id === "own-current-id",
    state: current.getState().value
  };
  return JSON.stringify({
    historyDescriptors,
    navigationDescriptors,
    historyOwnSlots,
    navigationOwnSlots,
    windowHolderOwnSlots,
    firstOwnSlots,
    currentOwnSlots,
    before,
    after
  });
})()
"##,
        )
        .expect("navigation runtime state should ignore prototype slots");

    assert_eq!(
        result,
        r##"{"historyDescriptors":["length:function:get length:0:undefined:none:none:true:true","state:function:get state:0:undefined:none:none:true:true","scrollRestoration:function:get scrollRestoration:0:function:set scrollRestoration:1:true:true"],"navigationDescriptors":["canGoBack:function:get canGoBack:0:undefined:none:none:true:true","canGoForward:function:get canGoForward:0:undefined:none:none:true:true","currentEntry:function:get currentEntry:0:undefined:none:none:true:true","activation:function:get activation:0:undefined:none:none:true:true","transition:function:get transition:0:undefined:none:none:true:true"],"historyOwnSlots":[],"navigationOwnSlots":[],"windowHolderOwnSlots":[],"firstOwnSlots":[],"currentOwnSlots":[],"before":{"length":1,"windowHistoryStable":true,"windowNavigationStable":true,"windowHolderSpoofed":"__moliWindowHistory,__moliWindowNavigation","historyStateNull":true,"scrollRestoration":"auto","currentEntryPresent":true,"transitionNull":true,"index":0,"hash":"","keySpoofed":false,"idSpoofed":false,"state":"real-before"},"after":{"length":2,"historyStateStep":1,"scrollRestoration":"auto","currentEntryPresent":true,"transitionNull":true,"index":1,"indexes":"0,1","hash":"#one","keySpoofed":false,"idSpoofed":false,"state":"real-after"}}"##
    );
}
#[test]
fn targeted_anchor_click_dispatches_navigate_on_named_child_window() {
    let mut vm = new_storage_html_test_vm("https://targeted-child-navigate.test/page.html");

    let result = vm
        .eval(
            r#"
const frame = document.createElement('iframe');
frame.name = 'target';
const root = document.body || document.documentElement || document;
root.appendChild(frame);
const link = document.createElement('a');
link.href = '/next.html';
link.target = 'target';
root.appendChild(link);
let seen = [];
frame.contentWindow.navigation.onnavigate = e => {
  seen.push([
    e.navigationType,
    e.cancelable,
    e.canIntercept,
    e.userInitiated,
    e.hashChange,
    e.formData === null,
    e.destination.url,
    e.destination.sameDocument,
    e.destination.key,
    e.destination.id,
    e.destination.index,
    e.sourceElement === link
  ].join(','));
};
link.click();
seen.join('|')
"#,
        )
        .expect("targeted anchor click should dispatch child navigate");

    assert_eq!(
        result,
        "push,true,true,false,false,true,https://targeted-child-navigate.test/next.html,false,,,-1,true"
    );
}
#[test]
fn same_document_anchor_click_updates_navigation_current_entry() {
    let mut vm = new_storage_test_vm("https://same-document-anchor.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const link = document.createElement('a');
  link.href = '#section';
  (document.body || document.documentElement || document).appendChild(link);
  const startIndex = navigation.currentEntry.index;
  let seen = null;
  navigation.oncurrententrychange = e => {
    seen = [
      e.navigationType,
      e.from === navigation.entries()[startIndex],
      e.from.index,
      navigation.currentEntry.index,
      location.hash
    ].join('|');
  };
  link.click();
  return seen;
})()
"##,
        )
        .expect("same-document anchor click should update navigation current entry");

    assert_eq!(result, "push|true|0|1|#section");
}
#[test]
fn replace_state_detaches_previous_navigation_entry_index() {
    let mut vm = new_storage_test_vm("https://replace-entry-index.test/page.html");

    let result = vm
        .eval(
            r##"
(() => {
  const original = navigation.currentEntry;
  let seen = null;
  navigation.oncurrententrychange = e => {
    seen = [
      e.from === original,
      e.from.index,
      navigation.currentEntry.index,
      navigation.entries().includes(original)
    ].join('|');
  };
  history.replaceState(null, '', '#replaced');
  return seen;
})()
"##,
        )
        .expect("replaceState should detach the previous navigation entry index");

    assert_eq!(result, "true|-1|0|false");
}
#[test]
fn document_open_with_three_arguments_uses_associated_window() {
    let mut vm = new_storage_test_vm("https://document-open-window.test/");

    let result = vm
        .eval(
            r#"
(() => {
  window.open = function() { throw new Error('shadowed open should not run'); };
  const live = document.open('/popup', '', '');
  const detached = new DOMParser().parseFromString('', 'text/html');
  let detachedError = '';
  try {
    detached.open('/popup', '', '');
  } catch (error) {
    detachedError = error.name + ':' + error.code;
  }
  return [
    live === window,
    live instanceof live.Window,
    detached.defaultView === null,
    detachedError
  ].join('|');
})()
"#,
        )
        .expect("three-argument document.open should evaluate");

    assert_eq!(result, "true|true|true|InvalidAccessError:15");
}
#[tokio::test(flavor = "current_thread")]
async fn iframe_javascript_url_replacement_preserves_later_fragment_for_reload() {
    const HOST: &str = "iframe-javascript-url-reload.test";

    let server = StaticHttpServer::spawn(2).await;
    let top_url = server.url_for_host(HOST, "/page.html");
    let child_url = server.url_for_host(HOST, "/blank.html");
    let child_fragment_url = server.url_for_host(HOST, "/blank.html#foo");
    let loader = static_http_loader([server.resolve_entry(HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url.as_str(), &loader);

    vm.eval(&format!(
        r#"
(() => {{
  globalThis.__javascriptUrlReloadLoadCount = 0;
  globalThis.__javascriptUrlReloadChildUrl = {};
  globalThis.__javascriptUrlReloadFragmentUrl = {};
  const frame = document.createElement('iframe');
  frame.src = __javascriptUrlReloadChildUrl;
  frame.onload = () => {{
    globalThis.__javascriptUrlReloadLoadCount++;
  }};
  (document.body || document.documentElement || document).appendChild(frame);
}})()
"#,
        serde_json::to_string(child_url.as_str()).expect("serialize child URL"),
        serde_json::to_string(child_fragment_url.as_str()).expect("serialize fragment URL")
    ))
    .expect("javascript URL reload child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__javascriptUrlReloadLoadCount)",
        "1",
        "initial child document should load",
    )
    .await;

    vm.eval(
        r#"
(() => {
  const frame = document.querySelector('iframe');
  frame.contentWindow.location =
    "javascript:'<html>javascript generated page</html>'";
  frame.contentWindow.location = __javascriptUrlReloadFragmentUrl;
})()
"#,
    )
    .expect("javascript URL followed by fragment navigation should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__javascriptUrlReloadLoadCount)",
        "2",
        "javascript URL replacement should dispatch load",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    frame.contentDocument.URL,
    frame.contentDocument.body.textContent
  ].join('|');
})()
"#,
        )
        .expect("javascript URL replacement state should evaluate"),
        format!("{}|javascript generated page", child_fragment_url.as_str())
    );

    vm.eval("document.querySelector('iframe').contentWindow.location.reload()")
        .expect("replacement document reload should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__javascriptUrlReloadLoadCount)",
        "3",
        "replacement document should reload the network resource",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    frame.contentDocument.URL,
    frame.contentDocument.body.textContent
  ].join('|');
})()
"#,
        )
        .expect("reloaded child document state should evaluate"),
        format!("{}|child fixture", child_fragment_url.as_str())
    );
    assert_eq!(
        server.finish_targets().await,
        vec!["/blank.html", "/blank.html"]
    );
}
#[tokio::test]
async fn queued_iframe_javascript_url_does_not_execute_after_scripting_is_disabled() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-disabled.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'disabled-javascript-url-frame';
  frame.src = "javascript:document.documentElement.setAttribute('data-javascript-url-ran', 'yes')";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("disabled iframe javascript URL setup should evaluate as automation");
    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("disabled-javascript-url-frame")
        .expect("disabled javascript URL iframe owner");

    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "NavigationCommit must publish the javascript URL work before scripting is disabled"
    );
    vm.set_script_execution_disabled(true);
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "the already-queued javascript URL work must be consumed as disabled",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(document.getElementById('disabled-javascript-url-frame').contentDocument.documentElement.getAttribute('data-javascript-url-ran'))"
        )
        .expect("disabled child document should remain observable to automation"),
        "null"
    );
    assert!(
        vm._context_host
            .borrow()
            .child_browsing_context_pending_live_navigation_for_test(child_handle)
            .is_none(),
        "disabled javascript URL work must settle its exact pending navigation"
    );
}
#[tokio::test]
async fn stale_iframe_javascript_url_work_cannot_finish_newer_navigation() {
    let mut vm = new_storage_test_vm("https://iframe-javascript-url-stale.test/");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.id = 'stale-javascript-url-frame';
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("stale javascript URL setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "stale javascript URL setup should initialize the child document",
    )
    .await;
    let child_context_id = materialize_single_child_default_realm_for_test(
        &mut vm,
        "stale javascript URL initial child document",
    );
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;

    vm.eval_in_child_default_context(
        child_context_id,
        "location.href = 'javascript:globalThis.__staleJavascriptUrlRan = true'",
    )
    .expect("first javascript URL navigation should queue");
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "first NavigationCommit should enqueue javascript URL work"
    );

    vm.eval_in_child_default_context(
        child_context_id,
        "location.href = 'javascript:globalThis.__freshJavascriptUrlRan = true'",
    )
    .expect("replacement javascript URL navigation should queue");
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "the stale javascript URL work should be consumed as a typed stale task"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "String(globalThis.__staleJavascriptUrlRan)",
        )
        .expect("stale javascript URL execution state should be readable"),
        "undefined",
        "stale javascript URL work must not execute in the still-current realm"
    );
    let pending = vm
        ._context_host
        .borrow()
        .child_browsing_context_pending_live_navigation_for_test(child_handle)
        .expect("replacement navigation must remain pending after stale work is dropped");
    assert!(
        matches!(
            pending,
            crate::native_bridge::ChildBrowsingContextBootstrap::Url(ref url)
                if url.as_str().contains("__freshJavascriptUrlRan")
        ),
        "stale work must not clear the replacement navigation"
    );

    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "replacement NavigationCommit should remain runnable"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "replacement javascript URL should execute on its own ready turn"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            child_context_id,
            "JSON.stringify([globalThis.__staleJavascriptUrlRan, globalThis.__freshJavascriptUrlRan])",
        )
        .expect("replacement javascript URL execution state should be readable"),
        "[null,true]"
    );
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "a subsequent non-string javascript URL navigation must not synthesize iframe load"
    );
}
#[tokio::test]
async fn no_src_iframe_initial_about_blank_has_no_navigation_activation_after_load() {
    let mut vm = new_storage_test_vm("https://iframe-initial-activation.test/");

    vm.eval(
        r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
"#,
    )
    .expect("no-src iframe setup should evaluate");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "no-src iframe initial about:blank should finish synchronously"
    );

    let result = vm
        .eval(
            r#"
(() => {
const frame = document.querySelector('iframe');
return [
  frame.contentWindow.location.href,
  frame.contentWindow.navigation.entries().length,
  frame.contentWindow.navigation.currentEntry.url,
  frame.contentWindow.navigation.activation === null
].join('|')
})()
"#,
        )
        .expect("no-src iframe navigation activation should evaluate");

    assert_eq!(result, "about:blank|1|about:blank|true");
}
#[test]
fn no_src_iframe_initial_about_blank_load_is_synchronous_at_connection() {
    let mut vm = new_storage_test_vm("https://iframe-initial-load-timing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  globalThis.initialBlankLoadEvents = events;
  queueMicrotask(() => events.push("microtask"));
  delete globalThis.Event;
  const frame = document.createElement("iframe");
  document.addEventListener("load", event => {
    if (event.target === frame) {
      events.push("capture");
    }
  }, true);
  frame.onload = () => events.push("handler");
  events.push("before");
  (document.body || document.documentElement || document).appendChild(frame);
  events.push("after");

  const late = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(late);
  late.onload = () => events.push("late");
  return events.join("|");
})()
"#,
        )
        .expect("initial about:blank load timing should evaluate");

    assert_eq!(result, "before|capture|handler|after");
    let settled = vm
        .eval("globalThis.initialBlankLoadEvents.join('|')")
        .expect("initial about:blank microtask order should evaluate");
    assert_eq!(settled, "before|capture|handler|after|microtask");
    assert!(
        !vm.has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad),
        "synchronous initial about:blank delivery must not leave HostLoad work"
    );
}
#[tokio::test]
async fn joint_history_branch_shrinks_after_traversal() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://joint-history.test/", &loader);
    vm.eval("history.replaceState('A', ''); for (const state of ['B', 'C', 'D']) history.pushState(state, ''); history.go(-2); 'queued'")
        .expect("queue traversal to B");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("traverse");
    assert_eq!(
        vm.eval("[history.state, history.length].join('|')")
            .unwrap(),
        "B|4"
    );
    assert_eq!(vm.eval("history.pushState('E', ''); [history.state, history.length, navigation.entries().length, navigation.canGoForward].join('|')").unwrap(), "E|3|3|false");
}
#[tokio::test]
async fn joint_history_siblings_share_steps_and_forward_pruning() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://joint-history.test/", &loader);
    vm.eval(
        r#"
      for (const id of ['a', 'b']) {
        const frame = document.createElement('iframe'); frame.id = id;
        frame.srcdoc = '<p>child</p>';
        (document.body || document.documentElement || document).appendChild(frame);
      }
      'ready'
    "#,
    )
    .expect("create siblings");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("load siblings");
    assert_eq!(vm.eval(r#"
      globalThis.a = document.getElementById('a').contentWindow;
      globalThis.b = document.getElementById('b').contentWindow;
      globalThis.held = [history, a.history, b.history];
      globalThis.snapshot = () => held.map(h => h.length).concat(held.map(h => h.state)).join('|');
      history.replaceState('top', ''); a.history.replaceState('a0', ''); b.history.replaceState('b0', '');
      a.history.pushState('a1', ''); b.history.pushState('b1', ''); a.history.pushState('a2', '');
      snapshot()
    "#).unwrap(), "4|4|4|top|a2|b1");
    vm.eval("history.back(); 'queued'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("back targets a, not last iframe b");
    assert_eq!(vm.eval("snapshot()").unwrap(), "4|4|4|top|a1|b1");
    vm.eval("a.history.back(); 'queued'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child caller traverses joint history to b0");
    assert_eq!(vm.eval("snapshot()").unwrap(), "4|4|4|top|a1|b0");
    assert_eq!(
        vm.eval("b.history.pushState('b2', ''); snapshot()")
            .unwrap(),
        "3|3|3|top|a1|b2"
    );
    assert_eq!(vm.eval("[a.navigation.entries().length, b.navigation.entries().length, a.navigation.canGoForward, b.navigation.canGoForward].join('|')").unwrap(), "2|2|false|false");
}
#[tokio::test]
async fn joint_history_navigation_traverse_uses_nearest_shared_step() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://joint-history.test/", &loader);
    vm.eval(
        r#"
      const frame = document.createElement('iframe'); frame.srcdoc = '<p>child</p>';
      (document.body || document.documentElement || document).appendChild(frame); 'ready'
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    vm.eval(
        r#"
      globalThis.child = document.querySelector('iframe').contentWindow;
      history.replaceState('top0', ''); child.history.replaceState('child0', '');
      child.history.pushState('child1', ''); history.pushState('top1', '');
      child.history.pushState('child2', ''); child.navigation.back(); 'queued'
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("[history.state, child.history.state, history.length].join('|')")
            .unwrap(),
        "top1|child1|4"
    );
    assert_eq!(
        vm.eval("history.pushState('top2', ''); [history.length, child.history.length].join('|')")
            .unwrap(),
        "4|4"
    );
    // A History traversal can change more than one Document's entry; all live
    // views must be updated before observers see the committed state.
    vm.eval("history.go(-2); 'queued'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("[history.state, child.history.state, history.length].join('|')")
            .unwrap(),
        "top0|child1|4"
    );
    vm.eval(r#"
      history.pushState('top3', '');
      globalThis.jointTraversalResult = 'pending';
      child.navigation.traverseTo(child.navigation.entries()[0].key).finished.then(entry => {
        jointTraversalResult = entry === child.navigation.currentEntry ? 'child-entry' : 'wrong-entry';
      }); 'queued'
    "#).unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("[history.state, child.history.state, jointTraversalResult].join('|')")
            .unwrap(),
        "top0|child0|child-entry"
    );
}
#[tokio::test]
async fn joint_history_pending_cursor_is_shared_and_detached_steps_remain_traversable() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://joint-history.test/", &loader);
    vm.eval(
        r#"
        for (const id of ['a', 'b']) {
            const f = document.createElement('iframe'); f.id = id; f.srcdoc = '<p>child</p>';
            (document.body || document.documentElement || document).append(f);
        }
        'ready'
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    vm.eval(
        r#"
        globalThis.a = document.getElementById('a').contentWindow;
        globalThis.b = document.getElementById('b').contentWindow;
        a.history.replaceState('a0', ''); b.history.replaceState('b0', '');
        a.history.pushState('a1', ''); b.history.pushState('b1', ''); a.history.pushState('a2', '');
        globalThis.childDocuments = [a.document, b.document];
        history.replaceState('replaced', '', '#replaced');
        history.back(); a.history.back(); 'queued'
    "#,
    )
    .unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("[a.history.state, b.history.state, history.length, a.document === childDocuments[0], b.document === childDocuments[1]].join('|')")
            .unwrap(),
        "a1|b0|4|true|true"
    );
    vm.eval("document.getElementById('a').remove(); document.getElementById('b').remove(); globalThis.pops=0; onpopstate=()=>pops++; history.back(); 'queued'").unwrap();
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    assert_eq!(
        vm.eval("history.pushState('branch', ''); [history.length, pops].join('|')")
            .unwrap(),
        "2|0"
    );
}
#[tokio::test]
async fn child_history_push_preserves_existing_top_history_length() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    for parent_steps in [0, 1, 3] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(
            "https://joint-child-length.test/page.html",
            &loader,
        );
        for _ in 0..parent_steps {
            vm.eval("history.pushState(null, ''); 'pushed'")
                .expect("parent history should advance");
        }
        vm.eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
        )
        .expect("child history frame setup should evaluate");
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child should finish loading");

        let result = vm
            .eval(
                r#"
(() => {
  const child = document.querySelector('iframe').contentWindow;
  const lengths = [];
  const snapshot = () => lengths.push(history.length, child.history.length);
  snapshot();
  child.history.replaceState({ step: 0 }, '');
  snapshot();
  child.history.pushState({ step: 1 }, '');
  snapshot();
  child.history.pushState({ step: 2 }, '');
  snapshot();
  child.history.replaceState({ step: 3 }, '');
  snapshot();
  return lengths.concat(child.history.state.step).join('|');
})()
"#,
            )
            .expect("child history mutations should evaluate");
        let initial_length = parent_steps + 1;
        let first_push_length = initial_length + 1;
        let second_push_length = initial_length + 2;
        assert_eq!(
            result,
            format!(
                "{initial_length}|{initial_length}|{initial_length}|{initial_length}|\
                 {first_push_length}|{first_push_length}|{second_push_length}|{second_push_length}|\
                 {second_push_length}|{second_push_length}|3"
            ),
            "parent history steps: {parent_steps}"
        );

        vm.eval("document.querySelector('iframe').contentWindow.history.back(); 'queued'")
            .expect("child history back should queue traversal");
        vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .expect("child history traversal should finish");
        assert_eq!(
            vm.eval("document.querySelector('iframe').contentWindow.history.state.step")
                .expect("back should restore the first pushed state"),
            "1"
        );
        assert_eq!(
            vm.eval(
                r#"
(() => {
  const child = document.querySelector('iframe').contentWindow;
  child.history.pushState({ step: 4 }, '');
  return [history.length, child.history.length, child.history.state.step].join('|');
})()
"#,
            )
            .expect("push after back should replace the forward history step"),
            format!("{second_push_length}|{second_push_length}|4"),
            "parent history steps: {parent_steps}"
        );
    }
}
#[tokio::test]
async fn top_history_back_routes_to_child_joint_history_entry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://joint-child-back.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child joint-history frame setup should evaluate");
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await;
    vm.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .expect("child setup should use the selected-task dispatcher");

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  frame.contentWindow.history.pushState({ child: true }, '', '#child');
  return [
    location.href,
    history.length,
    frame.contentWindow.history.length,
    frame.contentWindow.location.href
  ].join('|');
})()
"#,
        )
        .expect("child joint-history setup should evaluate");

    assert_eq!(
        setup,
        "https://joint-child-back.test/page.html|2|2|https://joint-child-back.test/page.html#child"
    );
    let _ = vm
        .run_one_oldest_ready_page_task_executor_turn(&loader)
        .await;

    vm.eval("history.back(); 'queued'")
        .expect("top history back should queue traversal");

    let mut result = String::new();
    for _ in 0..4 {
        assert!(
            vm.run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .expect("wait driver should advance joint-history traversal")
        );
        result = vm
            .eval(
                r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    location.href,
    frame.contentWindow.location.href,
    frame.contentWindow.navigation.currentEntry.url,
    frame.contentWindow.navigation.currentEntry.index
  ].join('|');
})()
"#,
            )
            .expect("joint-history traversal result should evaluate");
        if result == "https://joint-child-back.test/page.html|about:srcdoc|about:srcdoc|0" {
            break;
        }
    }

    assert_eq!(
        result,
        "https://joint-child-back.test/page.html|about:srcdoc|about:srcdoc|0"
    );
}
#[tokio::test]
async fn top_history_back_ignores_removed_child_joint_history_entry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://removed-child-joint-back.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("removed child joint-history frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  frame.contentWindow.history.pushState({ child: true }, '', '#child');
  return [
    location.href,
    navigation.entries().length,
    navigation.currentEntry.index,
    history.length,
    frame.contentWindow.history.length,
    frame.contentWindow.location.href
  ].join('|');
})()
"#,
        )
        .expect("removed child joint-history setup should evaluate");

    assert_eq!(
        setup,
        "https://removed-child-joint-back.test/page.html|1|0|2|2|https://removed-child-joint-back.test/page.html#child"
    );

    vm.eval("document.querySelector('iframe').remove(); history.back(); 'queued'")
        .expect("top history back after child removal should queue no traversal");

    let result = vm
        .eval(
            r#"
(() => [
  location.href,
  navigation.entries().length,
  navigation.currentEntry.index,
  history.length,
  document.querySelector('iframe') === null
].join('|'))()
"#,
        )
        .expect("top history back after child removal should remain a no-op");

    assert_eq!(
        result,
        "https://removed-child-joint-back.test/page.html|1|0|2|true"
    );
}
#[tokio::test]
async fn child_cross_document_pending_navigation_exposes_back_availability() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_test_vm_with_loader("https://child-cross-doc-back.test/page.html", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("frame setup");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval("document.querySelector('iframe').contentWindow.navigation.navigate('?foo'); 'queued'")
        .expect("child navigate");
    vm.drain_pending_child_frame_work_for_test();

    let state = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  const n = frame.contentWindow.navigation;
  return [
    frame.contentWindow.location.href,
    frame.contentWindow.history.length,
    n.entries().map(e => `${e.index}:${e.url}`).join(','),
    n.currentEntry && `${n.currentEntry.index}:${n.currentEntry.url}`,
    n.canGoBack,
    n.canGoForward
  ].join('|');
})()
"#,
        )
        .expect("child cross-document navigation state should evaluate");

    assert_eq!(
        state,
        "about:srcdoc|1|0:about:srcdoc|0:about:srcdoc|true|false"
    );
}
#[tokio::test]
async fn detached_child_navigation_error_exposes_committed_entry_during_dispatch() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://child-detach-navigation-error.test/page.html",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child navigation-error frame setup should evaluate");
    for _ in 0..128 {
        if !vm
            .run_one_oldest_ready_page_task_executor_turn(&loader)
            .await
            .expect("child navigation-error setup should use the selected-task dispatcher")
        {
            break;
        }
    }

    let initial = vm
        .eval(
            r##"
(() => {
  globalThis.__lmDetachedChildNavigateErrorLog = [];
  const child = document.querySelector('iframe').contentWindow;
  child.history.pushState({ child: true }, "", "#one");
  return [
    child.navigation.entries().length,
    child.navigation.currentEntry.index,
    child.location.href
  ].join("|");
})()
"##,
        )
        .expect("child initial same-document navigation should evaluate");
    assert_eq!(
        initial,
        "2|1|https://child-detach-navigation-error.test/page.html#one"
    );

    let setup = vm
        .eval(
            r##"
(() => {
  const frame = document.querySelector('iframe');
  const child = frame.contentWindow;
  globalThis.__lmDetachedChildNavigateErrorLog.length = 0;
  const target = child.navigation.entries()[0];
  child.navigation.onnavigate = event => {
    event.intercept({
      handler() {
        setTimeout(() => frame.remove(), 0);
        return new Promise(resolve => setTimeout(resolve, 1));
      }
    });
  };
  child.navigation.onnavigatesuccess = () => {
    globalThis.__lmDetachedChildNavigateErrorLog.push("success");
  };
  child.navigation.onnavigateerror = event => {
    const current = child.navigation.currentEntry;
    globalThis.__lmDetachedChildNavigateErrorLog.push([
      "error",
      current === target,
      current instanceof child.NavigationHistoryEntry,
      event.error && event.error.name
    ].join(":"));
  };
  const result = child.navigation.traverseTo(target.key);
  result.committed.then(entry => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`committed:${entry === target}`);
  }, error => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`committed-rejected:${error.name}`);
  });
  result.finished.then(() => {
    globalThis.__lmDetachedChildNavigateErrorLog.push("finished");
  }, error => {
    globalThis.__lmDetachedChildNavigateErrorLog.push(`finished-rejected:${error.name}`);
  });
  return "queued";
})()
"##,
        )
        .expect("child detach navigation-error setup should evaluate");
    assert_eq!(setup, "queued");

    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .expect("child traversal should run through the production history source")
    );
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child detach navigation-error timeouts should drain");
    let settled = vm
        .eval(
            r#"
(() => {
  return [
    globalThis.__lmDetachedChildNavigateErrorLog.join("|"),
    document.querySelector("iframe") === null
  ].join("||");
})()
"#,
        )
        .expect("child detach navigation-error log should evaluate");

    assert_eq!(
        settled,
        "committed:true|error:true:true:AbortError|finished-rejected:AbortError||true"
    );
}
#[tokio::test]
async fn child_meta_refresh_timer_is_canceled_when_frame_reloads() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__lmChildMetaRefreshLoads = [];
  const frame = document.createElement('iframe');
  frame.onload = () => {
    __lmChildMetaRefreshLoads.push(frame.contentDocument.body.textContent.trim());
  };
  frame.srcdoc = '<meta http-equiv="refresh" content="0"><p>first</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child meta refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let first_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child first meta refresh load log should evaluate");
    assert_eq!(first_loads, "first");

    vm.eval(
        r#"
(() => {
  document.querySelector('iframe').srcdoc = '<p>second</p>';
  return 'queued';
})()
"#,
    )
    .expect("child replacement srcdoc should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let replaced_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child replacement load log should evaluate");
    assert_eq!(replaced_loads, "first|second");

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("stale child meta refresh timer should drain without firing");
    let final_loads = vm
        .eval("globalThis.__lmChildMetaRefreshLoads.join('|')")
        .expect("child meta refresh final load log should evaluate");
    assert_eq!(final_loads, "first|second");
}
#[tokio::test]
async fn child_meta_refresh_navigate_event_cancellation_prevents_reload() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__lmChildMetaRefreshLoadCount = 0;
  globalThis.__lmChildMetaRefreshNavigateEvents = [];
  const frame = document.createElement('iframe');
  frame.onload = () => {
    __lmChildMetaRefreshLoadCount += 1;
    frame.contentWindow.navigation.onnavigate = event => {
      __lmChildMetaRefreshNavigateEvents.push([
        event.navigationType,
        event.cancelable,
        event.canIntercept,
        event.destination.sameDocument
      ].join(':'));
      event.preventDefault();
    };
  };
  frame.srcdoc = '<meta http-equiv="refresh" content="0"><p>first</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child meta refresh cancellation setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("canceled child meta refresh timer should settle");
    vm.drain_pending_child_frame_work_for_test();

    let state = vm
        .eval(
            r#"
(() => {
  const frame = document.querySelector('iframe');
  return [
    __lmChildMetaRefreshLoadCount,
    __lmChildMetaRefreshNavigateEvents.join('|'),
    frame.contentDocument.body.textContent.trim(),
    frame.contentWindow.location.href
  ].join('||');
})()
"#,
        )
        .expect("child meta refresh cancellation state should evaluate");
    assert_eq!(state, "1||reload:true:true:false||first||about:srcdoc");
}
#[tokio::test]
async fn child_meta_refresh_rejects_javascript_urls() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = `<script>globalThis.x = 1<\/script>
    <meta http-equiv="refresh" content="0;url=javascript:globalThis.x=2">`;
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("child javascript refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child timer lane should settle");

    assert_eq!(
        vm.eval("String(document.querySelector('iframe').contentWindow.x)")
            .expect("child script state should remain observable"),
        "1",
        "declarative refresh must never execute a javascript: URL"
    );
}
#[tokio::test]
async fn child_sandbox_blocks_meta_refresh_when_it_is_created() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-same-origin');
  frame.srcdoc = '<meta http-equiv="refresh" content="0;url=#blocked"><p>source</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("sandboxed child refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.eval("document.querySelector('iframe').removeAttribute('sandbox'); 'removed'")
        .expect("sandbox removal should evaluate");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("child timer lane should settle");

    assert_eq!(
        vm.eval("document.querySelector('iframe').contentWindow.location.href")
            .expect("child URL should remain observable"),
        "about:srcdoc",
        "removing the sandbox later must not revive a refresh rejected at creation time"
    );
}
#[tokio::test]
async fn child_meta_refresh_remains_scheduled_when_sandbox_is_added_later() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-meta-refresh.test/", &loader);

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  frame.srcdoc = '<meta http-equiv="refresh" content="0;url=#allowed"><p>source</p>';
  (document.body || document.documentElement || document).appendChild(frame);
  return 'ready';
})()
"#,
    )
    .expect("unsandboxed child refresh setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    vm.eval(
        "document.querySelector('iframe').setAttribute('sandbox', 'allow-same-origin'); 'added'",
    )
    .expect("sandbox addition should evaluate");
    vm.advance_timers_until_deadline_for_test(&loader)
        .await
        .expect("scheduled child refresh should run");

    assert_eq!(
        vm.eval("document.querySelector('iframe').contentWindow.location.href")
            .expect("child URL should remain observable"),
        "https://child-meta-refresh.test/#allowed",
        "the sandbox policy is checked when the refresh is created, not again when it becomes due"
    );
}
#[tokio::test]
async fn child_window_load_replacement_stops_old_delivery_before_owner_output() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_test_vm_with_loader("https://child-load-phases.test/", &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__childLoadPhaseEvents = [];
  globalThis.__childLoadPhaseReplacement = `
    <script>
      addEventListener("load", () => parent.__childLoadPhaseEvents.push("new-window"));
      addEventListener("pageshow", () => parent.__childLoadPhaseEvents.push("new-pageshow"));
      addEventListener("pagehide", () => parent.__childLoadPhaseEvents.push("new-pagehide"));
      addEventListener("unload", () => parent.__childLoadPhaseEvents.push("new-unload"));
    <\/script>
    <body data-version="new">new</body>`;
  const frame = document.createElement("iframe");
  frame.id = "load-phase-frame";
  frame.name = "load-phase-frame-client";
  frame.onload = () => {
    __childLoadPhaseEvents.push(`frame:${frame.contentDocument.body.dataset.version}`);
  };
  frame.srcdoc = `
    <script>
      addEventListener("load", () => {
        window.__oldWindowLoadInvoked = true;
        parent.__childLoadPhaseEvents.push("old-window");
        parent.document.getElementById("load-phase-frame").srcdoc =
          parent.__childLoadPhaseReplacement;
      });
      addEventListener("pagehide", () => parent.__childLoadPhaseEvents.push("old-pagehide"));
      addEventListener("unload", () => parent.__childLoadPhaseEvents.push("old-unload"));
      addEventListener("pageshow", () => parent.__childLoadPhaseEvents.push("old-pageshow"));
    <\/script>
    <body data-version="old">old</body>`;
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child load-phase replacement setup should evaluate");

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "old child load-phase srcdoc should commit before parser work",
    )
    .await;
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "old child load handler should install",
    )
    .await;
    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("load-phase-frame")
        .expect("load-phase iframe should remain in the main document");
    {
        let context_host = vm._context_host.borrow();
        let load_callback_identities =
            context_host.child_window_event_callback_identities_for_test(child_handle, "load");
        assert_eq!(
            load_callback_identities.len(),
            1,
            "old Window load listener must be registered against the reused LocalWindow"
        );
        assert!(
            load_callback_identities
                .iter()
                .filter_map(|(relevant, _)| *relevant)
                .all(|identity| context_host
                    .window_execution_context_identity_is_current(identity)),
            "old Window load listener must retain a current callback relevant realm"
        );
    }
    for context in ["old interactive", "old DOMContentLoaded", "old complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "old Window load should begin exact-owner delivery",
    )
    .await;
    assert_eq!(
        vm.eval(
            "String(document.getElementById('load-phase-frame').contentWindow.__oldWindowLoadInvoked === true)"
        )
        .expect("old child Window load invocation marker should evaluate"),
        "true",
        "HostLoad must invoke the old Window listener before it observes replacement"
    );
    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("old child load-phase trace should evaluate"),
        "old-window",
        "replacement in Window load must suppress old iframe load and pageshow"
    );
    assert_eq!(
        vm.completed_child_frame_navigation_load_count(),
        0,
        "stale old delivery must not publish frame/protocol completion"
    );

    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "Window-load replacement should commit on its navigation source",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        "Window-load replacement realm materialization must survive stale context retirement",
    )
    .await;
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "replacement child load handlers should install",
    )
    .await;
    for context in [
        "replacement interactive",
        "replacement DOMContentLoaded",
        "replacement complete",
    ] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            context,
        )
        .await;
    }
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "replacement load delivery should finish independently",
    )
    .await;

    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("replacement child load-phase trace should evaluate"),
        "old-window|old-pagehide|old-unload|new-window|frame:new|new-pageshow",
        "started old load must unload once in pagehide-before-unload order before replacement"
    );
    let snapshots = vm.take_completed_child_frame_navigation_loads();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].name.as_deref(),
        Some("load-phase-frame-client")
    );
    assert_eq!(
        snapshots[0].url, "about:srcdoc",
        "srcdoc load snapshots should expose the committed document URL, not its inherited base URL"
    );
    let replacement_loader_id = snapshots[0]
        .loader_id
        .as_deref()
        .expect("completed child navigation should expose its DocumentLoader identity")
        .to_owned();
    let frame_tree = vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(frame_tree.len(), 1);
    assert_eq!(frame_tree[0].loader_id, replacement_loader_id);
    assert_eq!(
        frame_tree[0].name.as_deref(),
        Some("load-phase-frame-client")
    );
    assert_eq!(
        frame_tree[0].owner_element_id.as_deref(),
        Some("load-phase-frame")
    );

    vm.eval(
        r#"document.getElementById("load-phase-frame").srcdoc = "<body data-version='third'>third</body>""#,
    )
    .expect("second replacement navigation should queue");
    expect_child_frame_task_source_after_realm_prerequisite(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "second replacement should retire the second exact document owner",
    )
    .await;
    assert_eq!(
        vm.eval("__childLoadPhaseEvents.join('|')")
            .expect("second replacement unload trace should evaluate"),
        "old-window|old-pagehide|old-unload|new-window|frame:new|new-pageshow|new-pagehide|new-unload",
        "each replacement document must own a fresh exactly-once unload lifecycle"
    );
    let second_replacement_tree = vm.child_browsing_context_frame_tree_snapshot_for_protocol();
    assert_eq!(second_replacement_tree.len(), 1);
    assert_ne!(
        second_replacement_tree[0].loader_id, replacement_loader_id,
        "a cross-document child navigation must replace its DocumentLoader identity"
    );
}
#[test]
fn no_src_iframe_initial_about_blank_fragment_location_does_not_change_current_entry() {
    let mut vm = new_storage_test_vm("https://iframe-initial-currententry.test/");

    let result = vm
        .eval(
            r##"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const child = frame.contentWindow;
const events = [];
child.navigation.oncurrententrychange = event => {
  events.push(`${event.navigationType}:${child.location.href}:${child.navigation.currentEntry.url}`);
};
child.location.href = "about:blank#1";
child.location.href = "about:blank#2";
[
  child.location.href,
  child.navigation.entries().length,
  child.navigation.currentEntry.url,
  events.join(",")
].join("|")
"##,
        )
        .expect("initial about:blank fragment navigation should evaluate");

    assert_eq!(result, "about:blank#2|1|about:blank|");
}
#[test]
fn initial_about_blank_iframe_assign_replaces_history_after_same_document_update() {
    for update_fragment in [false, true] {
        let mut vm = new_storage_test_vm("https://initial-blank-history.test/");
        vm.exec(
            r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
const initialDocument = frame.contentDocument;
"#,
            None,
        )
        .expect("initial about:blank frame should be created");
        if update_fragment {
            vm.exec("frame.contentWindow.location.hash = 'updated';", None)
                .expect("initial document fragment should update");
        }
        assert_eq!(
            vm.eval(
                r#"[
  frame.contentDocument === initialDocument,
  frame.contentDocument.readyState,
  history.length,
  frame.contentWindow.history.length
].join('|')"#,
            )
            .expect("initial document state should evaluate"),
            "true|complete|1|1"
        );

        vm.exec(
            "frame.contentWindow.location.assign('about:blank?next');",
            None,
        )
        .expect("initial document navigation should queue");
        vm.drain_pending_child_frame_work_for_test();

        assert_eq!(
            vm.eval(
                r#"[
  frame.contentWindow.location.href,
  frame.contentDocument.readyState,
  history.length,
  frame.contentWindow.history.length,
  frame.contentDocument === initialDocument
].join('|')"#,
            )
            .expect("initial document replacement should evaluate"),
            "about:blank?next|complete|1|1|false",
            "same-document fragment update: {update_fragment}"
        );
    }
}
#[test]
fn initial_about_blank_navigation_activation_uses_child_realm() {
    let mut vm = new_storage_test_vm("https://activation-realm.test/");
    assert_eq!(
        vm.eval(
            r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
String(frame.contentWindow.navigation.activation === null)
"#,
        )
        .expect("initial child should have no activation"),
        "true"
    );
    vm.exec(
        "frame.contentWindow.navigation.navigate('about:blank?next');",
        None,
    )
    .expect("parent should navigate the initial child");
    vm.drain_pending_child_frame_work_for_test();

    assert_eq!(
        vm.eval(
            r#"
(() => {
  const child = frame.contentWindow;
  const activation = child.navigation.activation;
  return JSON.stringify([
    activation.entry === child.navigation.currentEntry,
    Object.getPrototypeOf(activation) === child.NavigationActivation.prototype,
    Object.getPrototypeOf(activation.entry) === child.NavigationHistoryEntry.prototype,
    Object.getPrototypeOf(activation.from) === child.NavigationHistoryEntry.prototype,
    activation.from.url,
    activation.from.index,
    activation.navigationType
  ]);
})()
"#,
        )
        .expect("activation and its entries should belong to the child realm"),
        r#"[true,true,true,true,"about:blank",-1,"replace"]"#
    );
}
#[test]
fn non_initial_about_blank_iframe_assign_appends_history_after_replace() {
    for initial_srcdoc in [false, true] {
        let mut vm = new_storage_test_vm("https://non-initial-blank-history.test/");
        vm.exec(
            r#"
const frame = document.createElement('iframe');
const historySnapshot = () => [
  frame.contentWindow.location.href,
  frame.contentDocument.readyState,
  history.length,
  frame.contentWindow.history.length
].join('|');
"#,
            None,
        )
        .expect("frame should be created");
        if initial_srcdoc {
            vm.exec("frame.srcdoc = '<!doctype html><body>loaded';", None)
                .expect("srcdoc should be set before connecting the frame");
        }
        vm.exec(
            "(document.body || document.documentElement || document).appendChild(frame);",
            None,
        )
        .expect("frame should connect");
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval("historySnapshot()")
                .expect("loaded frame state should evaluate"),
            if initial_srcdoc {
                "about:srcdoc|complete|1|1"
            } else {
                "about:blank|complete|1|1"
            }
        );

        vm.exec(
            r#"
const documentBeforeReplace = frame.contentDocument;
frame.contentWindow.location.replace('about:blank');
"#,
            None,
        )
        .expect("document replacement should queue");
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval(
                "[historySnapshot(), frame.contentDocument === documentBeforeReplace].join('|')"
            )
            .expect("loaded non-initial about:blank state should evaluate"),
            "about:blank|complete|1|1|false"
        );

        let (handle, previous_entry_id) = {
            let host = vm._context_host.borrow();
            let handle = host.child_browsing_context_handles_in_document_order()[0];
            let seed = host
                .child_browsing_context_navigation_seed_snapshot(handle)
                .expect("loaded child should have a navigation seed")
                .committed_navigation_entry_seed;
            let current_entry = seed
                .entries
                .iter()
                .find(|entry| entry.history_index == seed.current_index)
                .expect("loaded child should have a current history entry");
            (handle, current_entry.id.clone())
        };
        vm.exec(
            "frame.contentWindow.location.assign('about:blank?next');",
            None,
        )
        .expect("non-initial document navigation should queue");
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval("historySnapshot()")
                .expect("non-initial document history should evaluate"),
            "about:blank?next|complete|2|2",
            "initial srcdoc: {initial_srcdoc}"
        );
        let seed = vm
            ._context_host
            .borrow()
            .child_browsing_context_navigation_seed_snapshot(handle)
            .expect("navigated child should have a navigation seed")
            .committed_navigation_entry_seed;
        assert!(
            seed.entries.iter().any(|entry| {
                entry.id == previous_entry_id && entry.history_index < seed.current_index
            }),
            "assign must retain the previous history step; initial srcdoc: {initial_srcdoc}"
        );
    }
}
#[tokio::test]
async fn child_script_document_open_after_location_navigation_is_noop() {
    let mut vm = new_storage_test_vm("https://child-script-location-open.test/");

    let result = vm
        .eval(
            r#"
(() => {
  globalThis.__childTextAfterNavigation = 'unset';
  globalThis.__childLoadHandlerStage = 'unset';
  globalThis.__childLoadListenerStage = 'unset';
  const frame = document.createElement('iframe');
  frame.__childLoadHandlerStage = 'unset';
  frame.__childLoadListenerStage = 'unset';
  frame.srcdoc = `
    <script>
      const blob = new Blob(['PASS'], { type: 'text/html' });
      location.href = URL.createObjectURL(blob);
      frameElement.onload = () => {
        frameElement.__childLoadHandlerStage = 'entered';
        parent.__childTextAfterNavigation = frameElement.contentDocument.body.textContent;
        frameElement.__childLoadHandlerStage = 'completed';
      };
      frameElement.addEventListener('load', () => {
        frameElement.__childLoadListenerStage = 'entered';
        parent.__childLoadListenerStage = 'entered';
      });
      document.open();
      document.write('FAIL');
      document.close();
    <\/script>`;
  (document.body || document.documentElement || document).appendChild(frame);
  return String(frame.contentDocument.body.textContent).includes('FAIL');
})()
"#,
        )
        .expect("child script document.open after location navigation should evaluate");

    assert_eq!(result, "false");
    assert!(
        !vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "initial srcdoc navigation must not dispatch load before commit"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "initial srcdoc must commit on its own navigation turn"
    );
    for label in [
        "initial-empty realm retirement before srcdoc script",
        "srcdoc realm materialization before parser script",
    ] {
        expect_one_child_frame_task_source(
            &mut vm,
            ChildFrameSemanticTurnKind::RealmMaterialization,
            label,
        )
        .await;
    }
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
        "srcdoc parser script must execute on DocumentScriptReady",
    )
    .await;
    assert_eq!(
        vm.eval("globalThis.__childTextAfterNavigation")
            .expect("child load handler precondition should evaluate"),
        "unset",
        "navigation and script turns must not dispatch iframe load inline"
    );
    assert_eq!(
        vm.eval("typeof document.querySelector('iframe').onload")
            .expect("child load handler registration should evaluate"),
        "function",
        "the child callback must be registered on the parent-owned frame element"
    );
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::NavigationCommit)
            .await,
        "the blob URL assigned by the child script must commit on a later navigation turn"
    );
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        "the queued srcdoc lifecycle task must stale-discard before blob lifecycle work",
    )
    .await;
    for transition in ["interactive", "DOMContentLoaded", "complete"] {
        expect_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            &format!("blob replacement should run {transition} before HostLoad"),
        )
        .await;
    }
    assert_eq!(
        vm.eval("document.querySelector('iframe').onload === null")
            .expect("retired child callback projection should evaluate"),
        "true",
        "LocalWindow retirement must tombstone handler properties instead of rediscovering the old callback from its wrapper"
    );
    expect_one_child_frame_task_source(
        &mut vm,
        ChildFrameSemanticTurnKind::HostLoad,
        "the committed blob document should settle load from a later HostLoad turn",
    )
    .await;
    assert_eq!(
        vm.eval("document.querySelector('iframe').__childLoadHandlerStage")
            .expect("deferred load delivery stage should evaluate"),
        "unset",
        "the later HostLoad turn must not invoke a callback owned by the retired child LocalWindow"
    );
    assert_eq!(
        vm.eval("document.querySelector('iframe').__childLoadListenerStage")
            .expect("deferred load listener stage should evaluate"),
        "unset",
        "the later HostLoad turn must not invoke a listener owned by the retired child LocalWindow"
    );

    assert_eq!(
        vm.eval(
            r#"
[
  document.querySelector('iframe').__childLoadHandlerStage,
  document.querySelector('iframe').__childLoadListenerStage,
  globalThis.__childTextAfterNavigation,
  globalThis.__childLoadListenerStage,
  document.querySelector('iframe').contentDocument.URL.startsWith('blob:'),
  document.querySelector('iframe').contentDocument.body.textContent,
  frames[0].document.body.textContent,
  String(document.querySelector('iframe').contentDocument.body.textContent).includes('FAIL')
].join('|')
"#
        )
        .expect("child navigation result should evaluate"),
        "unset|unset|unset|unset|true|PASS|PASS|false"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn document_domain_full_access_requires_both_documents_and_resets_on_navigation() {
    const INITIAL_CHILD_HOST: &str = "sub.example.test";
    const REPLACEMENT_CHILD_HOST: &str = "other.example.test";

    let initial_server = StaticHttpServer::spawn(1).await;
    let replacement_server = StaticHttpServer::spawn(1).await;
    let initial_child_url = initial_server.url_for_host(INITIAL_CHILD_HOST, "/child.html");
    let replacement_child_url =
        replacement_server.url_for_host(REPLACEMENT_CHILD_HOST, "/replacement.html");
    let loader = static_http_loader([
        initial_server.resolve_entry(INITIAL_CHILD_HOST),
        replacement_server.resolve_entry(REPLACEMENT_CHILD_HOST),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "http://www.example.test:8443/page.html",
        &loader,
    );
    vm.eval(&format!(
        "globalThis.__initialDomainChildUrl = {}; globalThis.__replacementDomainChildUrl = {};",
        serde_json::to_string(initial_child_url.as_str())
            .expect("serialize initial document.domain child URL"),
        serde_json::to_string(replacement_child_url.as_str())
            .expect("serialize replacement document.domain child URL")
    ))
    .expect("document.domain child URLs should install");

    vm.exec(
        r#"
const frame = document.createElement("iframe");
globalThis.__domainAccessLoadCount = 0;
frame.onload = () => { globalThis.__domainAccessLoadCount += 1; };
frame.src = globalThis.__initialDomainChildUrl;
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__domainAccessFrame = frame;
globalThis.__probeDomainChildDocument = () => {
  try {
    return frame.contentWindow.document.domain;
  } catch (error) {
    return `${error && error.name}:${error instanceof DOMException}`;
  }
};
"#,
        None,
    )
    .expect("cross-origin document.domain frame setup should run");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__domainAccessLoadCount)",
        "1",
        "initial document.domain child should load",
    )
    .await;

    let initial_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("cross-origin child realm should materialize");
    assert_eq!(
        vm.eval("__probeDomainChildDocument()")
            .expect("initial cross-origin access probe should evaluate"),
        "SecurityError:true"
    );

    assert_eq!(
        vm.eval("document.domain = 'example.test'; __probeDomainChildDocument()")
            .expect("one-sided parent document.domain probe should evaluate"),
        "SecurityError:true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            initial_context_id,
            r#"
(() => {
  const before = (() => {
    try {
      return parent.document.domain;
    } catch (error) {
      return `${error && error.name}:${error instanceof DOMException}`;
    }
  })();
  document.domain = "example.test";
  return [before, parent.document.domain, top.document.domain].join("|");
})()
"#,
        )
        .expect("child document.domain access probe should evaluate"),
        "SecurityError:true|example.test|example.test"
    );
    assert_eq!(
        vm.eval(
            "[__probeDomainChildDocument(), __domainAccessFrame.contentDocument.domain].join('|')",
        )
        .expect("two-sided document.domain access probe should evaluate"),
        "example.test|example.test"
    );

    vm.exec(
        r#"
__domainAccessFrame.src = globalThis.__replacementDomainChildUrl;
"#,
        None,
    )
    .expect("replacement cross-origin child navigation should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__domainAccessLoadCount)",
        "2",
        "replacement document.domain child should load",
    )
    .await;

    let replacement_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("replacement cross-origin child realm should materialize");
    assert_ne!(replacement_context_id, initial_context_id);
    assert_eq!(
        vm.eval("__probeDomainChildDocument()")
            .expect("replacement one-sided document.domain probe should evaluate"),
        "SecurityError:true"
    );
    assert_eq!(
        vm.eval_in_child_default_context(
            replacement_context_id,
            "document.domain = 'example.test'; parent.document.domain",
        )
        .expect("replacement child document.domain access probe should evaluate"),
        "example.test"
    );
    assert_eq!(initial_server.finish_targets().await, vec!["/child.html"]);
    assert_eq!(
        replacement_server.finish_targets().await,
        vec!["/replacement.html"]
    );
}

#[test]
fn initial_empty_iframe_reload_methods_replace_document_and_dispatch_load() {
    for reload in ["location.reload()", "history.go(0)", "navigation.reload()"] {
        let mut vm = new_storage_test_vm("https://initial-empty-reload.test/page.html");

        vm.exec(
            &format!(
                r#"
const frame = document.createElement('iframe');
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__initialReloadFrame = frame;
globalThis.__initialReloadDocument = frame.contentDocument;
globalThis.__initialReloadLoads = 0;
frame.onload = () => ++__initialReloadLoads;
frame.contentWindow.{reload};
"#
            ),
            None,
        )
        .expect("initial-empty iframe reload should evaluate");
        assert!(
            vm.has_pending_child_navigation_commit_for_test(),
            "reloading an ordinary initial about:blank iframe must queue a navigation"
        );

        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval(
                r#"[
  __initialReloadFrame.contentDocument !== __initialReloadDocument,
  __initialReloadLoads,
  __initialReloadFrame.contentWindow.location.href
].join('|')"#,
            )
            .expect("initial-empty iframe reload result should evaluate"),
            "true|1|about:blank"
        );
    }
}

#[test]
fn initial_empty_iframe_reload_preserves_pending_attribute_navigation_result_shape() {
    let mut vm = new_storage_test_vm("https://initial-empty-reload.test/page.html");

    let setup = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  frame.loading = 'lazy';
  frame.hidden = true;
  frame.src = '/pending-attribute-navigation.html';
  (document.body || document.documentElement || document).appendChild(frame);
  const child = frame.contentWindow;
  const log = [];
  const optionReads = [];
  child.navigation.onnavigate = () => log.push('navigate');
  child.navigation.onnavigatesuccess = () => log.push('navigatesuccess');
  child.navigation.onnavigateerror = () => log.push('navigateerror');

  const options = {};
  Object.defineProperties(options, {
    info: {
      get() {
        optionReads.push('info');
        return 'initial-empty';
      }
    },
    state: {
      get() {
        optionReads.push('state');
        return { initialEmpty: true };
      }
    }
  });
  const result = child.navigation.reload(options);
  result.committed.then(
    () => log.push('committed:fulfilled'),
    () => log.push('committed:rejected')
  );
  result.finished.then(
    () => log.push('finished:fulfilled'),
    () => log.push('finished:rejected')
  );
  child.location.reload();
  child.history.go(0);
  Promise.resolve().then(() => log.push('checkpoint'));
  globalThis.__initialEmptyReloadFrame = frame;
  globalThis.__initialEmptyReloadLog = log;

  return JSON.stringify({
    href: child.location.href,
    resultRealm: Object.getPrototypeOf(result) === child.Object.prototype,
    keys: Reflect.ownKeys(result),
    committedPromise: result.committed instanceof child.Promise,
    finishedPromise: result.finished instanceof child.Promise,
    distinctPromises: result.committed !== result.finished,
    optionReads,
    log
  });
})()
"#,
        )
        .expect("initial-empty reload setup should evaluate");

    assert_eq!(
        setup,
        r#"{"href":"about:blank","resultRealm":true,"keys":["committed","finished"],"committedPromise":true,"finishedPromise":true,"distinctPromises":true,"optionReads":["info","state"],"log":[]}"#
    );
    assert_eq!(
        vm.eval("globalThis.__initialEmptyReloadLog.join('|')")
            .expect("initial-empty reload microtask log should evaluate"),
        "checkpoint"
    );
    assert_eq!(
        vm.eval("globalThis.__initialEmptyReloadFrame.contentWindow.location.href")
            .expect("initial-empty reload location should evaluate"),
        "about:blank"
    );
}

#[test]
fn non_initial_about_blank_iframe_remains_reloadable() {
    let mut vm = new_storage_test_vm("https://non-initial-blank-reload.test/page.html");

    vm.exec(
        r#"
const frame = document.createElement('iframe');
frame.srcdoc = '<p>first committed document</p>';
(document.body || document.documentElement || document).appendChild(frame);
globalThis.__nonInitialBlankReloadFrame = frame;
"#,
        None,
    )
    .expect("committed child setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.exec(
        r#"
__nonInitialBlankReloadFrame.src = 'about:blank';
"#,
        None,
    )
    .expect("non-initial about:blank navigation should evaluate");
    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval("__nonInitialBlankReloadFrame.contentWindow.location.href")
            .expect("non-initial about:blank location should evaluate"),
        "about:blank"
    );

    vm.exec(
        r#"
globalThis.__nonInitialBlankReloadLoads = 0;
__nonInitialBlankReloadFrame.onload = () => {
  globalThis.__nonInitialBlankReloadLoads += 1;
};
globalThis.__nonInitialBlankNavigationResult =
  __nonInitialBlankReloadFrame.contentWindow.navigation.reload();
"#,
        None,
    )
    .expect("non-initial about:blank reload should evaluate");
    assert!(
        vm.has_pending_child_navigation_commit_for_test(),
        "a committed about:blank Document must be admitted to the reload pipeline"
    );

    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"[
  __nonInitialBlankReloadFrame.contentWindow.location.href,
  __nonInitialBlankReloadLoads
].join('|')"#,
        )
        .expect("non-initial about:blank reload result should evaluate"),
        "about:blank|1"
    );
}
#[tokio::test(flavor = "current_thread")]
async fn initial_empty_lazy_iframe_reload_preserves_pending_attribute_navigation() {
    const PARENT_HOST: &str = "lazy-iframe-reload.test";

    let server = StaticHttpServer::spawn(3).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let location_url = server.url_for_host(PARENT_HOST, "/location-child.html");
    let navigation_url = server.url_for_host(PARENT_HOST, "/navigation-child.html");
    let history_url = server.url_for_host(PARENT_HOST, "/history-child.html");
    let loader = static_http_loader([server.resolve_entry(PARENT_HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);

    vm.exec(
        r#"
globalThis.__lazyReloadLoads = { location: 0, navigation: 0, history: 0 };
globalThis.__lazyNavigationReloadSettled = 'pending';
globalThis.__lazyNavigationReloadOptionReads = [];

const locationFrame = document.createElement('iframe');
locationFrame.loading = 'lazy';
locationFrame.hidden = true;
locationFrame.src = '/location-child.html';
locationFrame.onload = () => { globalThis.__lazyReloadLoads.location += 1; };
(document.body || document.documentElement || document).appendChild(locationFrame);
locationFrame.contentWindow.location.reload();
locationFrame.hidden = false;
globalThis.__lazyLocationReloadFrame = locationFrame;

const navigationFrame = document.createElement('iframe');
navigationFrame.loading = 'lazy';
navigationFrame.hidden = true;
navigationFrame.src = '/navigation-child.html';
navigationFrame.onload = () => { globalThis.__lazyReloadLoads.navigation += 1; };
(document.body || document.documentElement || document).appendChild(navigationFrame);
const reloadOptions = {};
Object.defineProperties(reloadOptions, {
  state: {
    get() {
      globalThis.__lazyNavigationReloadOptionReads.push('state');
      return { retained: true };
    }
  },
  info: {
    get() {
      globalThis.__lazyNavigationReloadOptionReads.push('info');
      return 'retained';
    }
  }
});
const reloadResult = navigationFrame.contentWindow.navigation.reload(reloadOptions);
Promise.all([reloadResult.committed, reloadResult.finished]).then(
  () => { globalThis.__lazyNavigationReloadSettled = 'fulfilled'; },
  () => { globalThis.__lazyNavigationReloadSettled = 'rejected'; }
);
navigationFrame.hidden = false;
globalThis.__lazyNavigationReloadFrame = navigationFrame;

const historyFrame = document.createElement('iframe');
historyFrame.loading = 'lazy';
historyFrame.hidden = true;
historyFrame.src = '/history-child.html';
historyFrame.onload = () => { globalThis.__lazyReloadLoads.history += 1; };
(document.body || document.documentElement || document).appendChild(historyFrame);
historyFrame.contentWindow.history.go(0);
historyFrame.hidden = false;
globalThis.__lazyHistoryReloadFrame = historyFrame;
"#,
        None,
    )
    .expect("lazy iframe reload setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__lazyReloadLoads.location === 1 && __lazyReloadLoads.navigation === 1 && __lazyReloadLoads.history === 1)",
        "true",
        "all reload entry points should preserve the pending attribute navigation",
    )
    .await;

    assert_eq!(
        vm.eval(
            r#"[
  __lazyLocationReloadFrame.contentWindow.location.href,
  __lazyNavigationReloadFrame.contentWindow.location.href,
  __lazyHistoryReloadFrame.contentWindow.location.href,
  __lazyReloadLoads.location,
  __lazyReloadLoads.navigation,
  __lazyReloadLoads.history,
  __lazyNavigationReloadSettled,
  __lazyNavigationReloadOptionReads.join(',')
].join('|')"#,
        )
        .expect("lazy iframe reload result should evaluate"),
        format!("{location_url}|{navigation_url}|{history_url}|1|1|1|pending|info,state")
    );
    let mut request_targets = server.finish_targets().await;
    request_targets.sort();
    assert_eq!(
        request_targets,
        vec![
            "/history-child.html",
            "/location-child.html",
            "/navigation-child.html"
        ]
    );
}
#[tokio::test(flavor = "current_thread")]
async fn lazy_iframe_location_replace_cancels_pending_attribute_navigation() {
    const PARENT_HOST: &str = "lazy-iframe-replace.test";
    const CROSS_ORIGIN_HOST: &str = "cross-origin.test";

    let server = StaticHttpServer::spawn(2).await;
    let parent_url = server.url_for_host(PARENT_HOST, "/page.html");
    let same_origin_navigation_url = server.url_for_host(PARENT_HOST, "/same-nav.html");
    let cross_origin_navigation_url = server.url_for_host(CROSS_ORIGIN_HOST, "/nav.html");
    let loader = static_http_loader([
        server.resolve_entry(PARENT_HOST),
        server.resolve_entry(CROSS_ORIGIN_HOST),
    ]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent_url.as_str(), &loader);
    vm.eval(&format!(
        "globalThis.__lazyCrossOriginNavigationUrl = {};",
        serde_json::to_string(cross_origin_navigation_url.as_str())
            .expect("serialize lazy cross-origin navigation URL")
    ))
    .expect("lazy cross-origin navigation URL should install");

    vm.exec(
        r#"
const frame = document.createElement('iframe');
frame.loading = 'lazy';
frame.hidden = true;
frame.src = '/src.html';
(document.body || document.documentElement || document).appendChild(frame);
frame.contentWindow.location.replace('data:text/html,<body>navigated</body>');
frame.hidden = false;
globalThis.__lazyReplaceFrame = frame;

const sameOriginFrame = document.createElement('iframe');
sameOriginFrame.loading = 'lazy';
sameOriginFrame.hidden = true;
sameOriginFrame.src = '/same-src.html';
globalThis.__lazySameOriginLoaded = false;
sameOriginFrame.onload = () => { globalThis.__lazySameOriginLoaded = true; };
(document.body || document.documentElement || document).appendChild(sameOriginFrame);
sameOriginFrame.contentWindow.location.replace('/same-nav.html');
sameOriginFrame.hidden = false;
globalThis.__lazySameOriginReplaceFrame = sameOriginFrame;

const crossOriginFrame = document.createElement('iframe');
crossOriginFrame.loading = 'lazy';
crossOriginFrame.hidden = true;
crossOriginFrame.src = '/cross-src.html';
globalThis.__lazyCrossOriginLoaded = false;
crossOriginFrame.onload = () => { globalThis.__lazyCrossOriginLoaded = true; };
(document.body || document.documentElement || document).appendChild(crossOriginFrame);
crossOriginFrame.contentWindow.location.replace(globalThis.__lazyCrossOriginNavigationUrl);
crossOriginFrame.hidden = false;
globalThis.__lazyCrossOriginReplaceFrame = crossOriginFrame;
"#,
        None,
    )
    .expect("lazy iframe replace setup should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__lazySameOriginLoaded && globalThis.__lazyCrossOriginLoaded)",
        "true",
        "replacement child navigations should complete",
    )
    .await;

    let result = vm
        .eval(
            r#"
(() => {
  const probeHref = () => {
    try {
      __lazyReplaceFrame.contentWindow.location.href;
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  const probeCrossOriginHref = () => {
    try {
      __lazyCrossOriginReplaceFrame.contentWindow.location.href;
      return "no-throw";
    } catch (error) {
      return error && error.name;
    }
  };
  return [
    __lazyReplaceFrame.contentDocument === null,
    probeHref(),
    __lazySameOriginReplaceFrame.contentWindow.location.href,
    __lazyCrossOriginReplaceFrame.contentDocument === null,
    probeCrossOriginHref()
  ].join('|');
})()
"#,
        )
        .expect("lazy iframe replace result should evaluate");

    assert_eq!(
        result,
        format!("true|SecurityError|{same_origin_navigation_url}|true|SecurityError")
    );
    let mut request_targets = server.finish_targets().await;
    request_targets.sort();
    assert_eq!(request_targets, vec!["/nav.html", "/same-nav.html"]);
}
#[test]
fn window_named_access_uses_its_realm_during_child_history_callbacks() {
    let mut vm = new_storage_test_vm("https://joint-history.test/parent");
    vm.eval(
        r#"
      const frame = document.createElement('iframe');
      frame.id = 'topFrame';
      frame.srcdoc = '<p id="childOnly">child</p>';
      (document.body || document.documentElement || document).appendChild(frame);
    "#,
    )
    .unwrap();
    vm.drain_pending_child_frame_work_for_test();
    let child = vm
        ._context_host
        .borrow()
        .child_browsing_context_handles_in_document_order()[0];
    let context = &vm.page_default_runtime.context as *const v8::Global<v8::Context>;
    vm.with_context_scope_by_ptr(context, |scope, _| {
        crate::native_bridge::enter_active_child_window_scope(scope, Some(child));
        Ok(())
    })
    .unwrap();
    let result =
        vm.eval("[topFrame.id, typeof childOnly, frame.contentWindow.childOnly.id].join('|')");
    vm.with_context_scope_by_ptr(context, |scope, _| {
        crate::native_bridge::enter_active_child_window_scope(scope, None);
        Ok(())
    })
    .unwrap();
    assert_eq!(result.unwrap(), "topFrame|undefined|childOnly");
}
#[test]
fn dynamic_about_blank_iframe_navigation_uses_origin_referrer() {
    let mut vm = new_storage_test_vm(
        "http://dynamic-about-blank-referrer.test/source/parent.html?query#fragment",
    );

    vm.eval(
        r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  globalThis.__dynamicBlankFrame = frame;
})()
"#,
    )
    .expect("initial about:blank child should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    vm.eval(
        r#"
(() => {
  globalThis.__dynamicBlankLoadCount = 0;
  __dynamicBlankFrame.onload = () => globalThis.__dynamicBlankLoadCount++;
  __dynamicBlankFrame.src = 'about:blank';
})()
"#,
    )
    .expect("dynamic about:blank navigation should evaluate");

    vm.drain_pending_child_frame_work_for_test();
    assert_eq!(
        vm.eval(
            r#"(() => {
  const frame = document.querySelector('iframe');
  return JSON.stringify({
    referrer: frame.contentDocument.referrer,
    loads: __dynamicBlankLoadCount,
    href: frame.contentWindow.location.href,
    historyLength: frame.contentWindow.history.length
  });
})()"#,
        )
        .expect("dynamic about:blank result should evaluate"),
        r#"{"referrer":"http://dynamic-about-blank-referrer.test/","loads":1,"href":"about:blank","historyLength":1}"#
    );
}

#[test]
fn iframe_src_navigation_uses_owner_document_encoding_for_query() {
    let mut vm = new_storage_test_vm("https://iframe-src-encoding.test/page.html");
    vm.document_runtime
        .set_document_character_set("windows-1252");

    let reflected_src = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "encoded-src-frame";
  frame.src = "resources/frame.html?\u00DF";
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.src;
})()
"#,
        )
        .expect("legacy-encoded iframe src setup should evaluate");
    assert_eq!(
        reflected_src,
        "https://iframe-src-encoding.test/resources/frame.html?%DF"
    );

    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("encoded-src-frame")
        .expect("encoded src iframe owner");
    let attribute_bootstrap = vm
        ._context_host
        .borrow()
        .child_browsing_context_attribute_bootstrap_for_test(child_handle)
        .expect("encoded src attribute bootstrap should exist");
    assert!(
        matches!(
            attribute_bootstrap,
            crate::native_bridge::ChildBrowsingContextBootstrap::Url(ref url)
                if url.as_str()
                    == "https://iframe-src-encoding.test/resources/frame.html?%DF"
        ),
        "iframe navigation must use the same owner-document encoding as its reflected src: {attribute_bootstrap:?}"
    );
}

#[test]
fn about_document_fragment_updates_preserve_relative_element_urls() {
    for kind in ["srcdoc", "popup"] {
        let mut vm = new_storage_test_vm("https://inherited-base.test/path/page.html");
        vm.exec(
            &format!(
                r#"
let child;
if ('{kind}' === 'popup') {{
  child = open();
}} else {{
  const frame = document.createElement('iframe');
  frame.srcdoc = '<p>child</p>';
  (document.body || document.documentElement || document).append(frame);
  child = frame.contentWindow;
}}
"#
            ),
            None,
        )
        .expect("about Document should be created");
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval(
                r#"
(() => {
  const saved = child.document;
  const href = child.location.href + '#updated';
  child.location.href = href;
  const anchor = saved.createElement('a'); anchor.href = 'next.html';
  const image = saved.createElement('img'); image.src = 'next.html';
  const form = saved.createElement('form'); form.action = 'next.html';
  const expected = 'https://inherited-base.test/path/next.html';
  return [saved === child.document, saved.URL === href,
    saved.baseURI === 'https://inherited-base.test/path/page.html',
    anchor.href === expected, image.src === expected, form.action === expected].join('|');
})()
"#
            )
            .expect("relative URL resolution after fragment update should evaluate"),
            "true|true|true|true|true|true",
            "{kind}"
        );
    }
}

#[test]
fn document_named_item_does_not_shadow_legacy_unforgeable_location() {
    let mut vm = new_storage_test_vm("https://example.com/current/path");

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

              const originalLocation = document.location;
              const originalPathname = originalLocation.pathname;
              const locationForm = document.createElement("form");
              locationForm.name = "location";
              const ordinaryForm = document.createElement("form");
              ordinaryForm.name = "namedProbe";
              document.body.append(locationForm, ordinaryForm);

              const whileConnected = [
                document.location === originalLocation,
                document.location.pathname === originalPathname,
                document.namedProbe === ordinaryForm,
              ];
              locationForm.remove();

              return JSON.stringify({
                whileConnected,
                afterRemoval: document.location === originalLocation,
              });
            })()
            "#,
        )
        .expect("legacy-unforgeable document location probe should evaluate");

    assert_eq!(
        result,
        r#"{"whileConnected":[true,true,true],"afterRemoval":true}"#
    );
}

#[test]
fn constructed_documents_share_legacy_unforgeable_location_accessors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
              const first = new Document();
              const second = new Document();
              const firstDescriptor = Object.getOwnPropertyDescriptor(first, "location");
              const secondDescriptor = Object.getOwnPropertyDescriptor(second, "location");
              const throwsName = callback => {
                try {
                  callback();
                  return "returned";
                } catch (error) {
                  return error && error.name;
                }
              };

              return JSON.stringify({
                firstValue: first.location,
                secondValue: second.location,
                own: Object.prototype.hasOwnProperty.call(first, "location"),
                getType: typeof firstDescriptor.get,
                setType: typeof firstDescriptor.set,
                getName: firstDescriptor.get.name,
                getLength: firstDescriptor.get.length,
                setName: firstDescriptor.set.name,
                setLength: firstDescriptor.set.length,
                getSame: firstDescriptor.get === secondDescriptor.get,
                setSame: firstDescriptor.set === secondDescriptor.set,
                enumerable: firstDescriptor.enumerable,
                configurable: firstDescriptor.configurable,
                assign: throwsName(() => {
                  "use strict";
                  first.location = "https://example.org/";
                }),
                badGet: throwsName(() => firstDescriptor.get.call({})),
                badSet: throwsName(() => firstDescriptor.set.call({}, "x")),
              });
            })()
            "#,
        )
        .expect("constructed Document location accessor probe should evaluate");

    assert_eq!(
        result,
        r#"{"firstValue":null,"secondValue":null,"own":true,"getType":"function","setType":"function","getName":"get location","getLength":0,"setName":"set location","setLength":1,"getSame":true,"setSame":true,"enumerable":true,"configurable":false,"assign":"TypeError","badGet":"TypeError","badSet":"TypeError"}"#
    );
}

#[tokio::test]
async fn embed_and_object_javascript_attributes_use_resource_fetch_not_script_navigation() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        "https://embedded-javascript-attribute.test/",
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__embeddedJavascriptAttributeEvents = [];
  const root = document.body || document.documentElement || document;
  for (const [tag, attribute] of [["embed", "src"], ["object", "data"]]) {
    const element = document.createElement(tag);
    element[attribute] =
      `javascript:parent.__embeddedJavascriptAttributeEvents.push("executed-${tag}")`;
    element.onload = () => {
      __embeddedJavascriptAttributeEvents.push(`load-${tag}`);
    };
    if (tag === "object") {
      element.onerror = event => {
        __embeddedJavascriptAttributeEvents.push(
          `error-object:${event.isTrusted}:${element.contentDocument === null}`
        );
      };
    }
    root.appendChild(element);
  }
})()
"#,
    )
    .expect("embedded javascript attribute setup should evaluate");

    for element in ["embed", "object"] {
        expect_page_child_frame_task_source_after_realm_prerequisite(
            &mut vm,
            &loader,
            ChildFrameSemanticTurnKind::NavigationCommit,
            &format!("{element} javascript attribute should enter its navigation commit turn"),
        )
        .await;
    }
    assert!(
        vm._context_host.borrow().has_pending_child_document_loads(),
        "embed and object javascript attributes should start resource fetches"
    );
    assert!(
        !vm.has_pending_child_frame_realm_materialization(),
        "resource attributes must not schedule javascript execution in child realms"
    );
    for element in ["embed", "object"] {
        wait_for_one_page_resource_completion_selected_task_executor_test_turn(
            &mut vm,
            &loader,
            &format!("{element} javascript attribute fetch failure"),
        )
        .await;
    }
    assert!(
        !vm._context_host.borrow().has_pending_child_document_loads(),
        "both embedded resource fetch failures should settle"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__embeddedJavascriptAttributeEvents)")
            .expect("embedded javascript attribute events should evaluate"),
        r#"["error-object:true:true"]"#,
        "resource failures must not execute javascript or load, and object must enter fallback"
    );
}

#[test]
fn iframe_in_shadow_tree_is_not_a_window_child_property() {
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
  window.length,
  window.frames.length,
  window[0] === lightFrame.contentWindow,
  window[1] === undefined,
  'shadowTarget' in window,
  window.shadowTarget === undefined,
  shadowFrame.contentWindow !== null,
  'lightTarget' in window,
  window.lightTarget === lightFrame.contentWindow
].join('|')
"#,
        )
        .expect("shadow iframe named property probe should evaluate");

    assert_eq!(result, "1|1|true|true|false|true|true|true|true");
}

#[tokio::test(flavor = "current_thread")]
async fn base_target_navigation_exposes_replacement_document_before_iframe_load() {
    const HOST: &str = "anchor-base-target.test";

    let server = StaticHttpServer::spawn(3).await;
    let top_url = server.url_for_host(HOST, "/path/page.html");
    let replacement_url = server.url_for_host(HOST, "/replacement.html");
    let loader = static_http_loader([server.resolve_entry(HOST)]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(top_url.as_str(), &loader);

    vm.eval(
        r#"
(() => {
  globalThis.__targetLoadCount = 0;
  globalThis.__targetLoadAccess = "pending";
  globalThis.__frameLoadCount = 0;
  globalThis.__baseTargetOwnerRealm = "pending";

  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  document.addEventListener('load', () => { __frameLoadCount += 1; }, true);
  body.innerHTML = `
    <iframe id="base-target-source" name="sourceFrame" src="/source.html"></iframe>
    <iframe id="base-target-target" name="targetFrame" src="/target.html"></iframe>
  `;
})()
"#,
    )
    .expect("base-target network child setup should evaluate");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__frameLoadCount)",
        "2",
        "initial parser-created base-target child documents should load",
    )
    .await;

    vm.eval(
        r#"
(() => {
  const source = document.getElementById('base-target-source');
  const target = document.getElementById('base-target-target');
  __baseTargetOwnerRealm = [
    source instanceof HTMLIFrameElement,
    target instanceof HTMLIFrameElement,
    target instanceof target.contentWindow.HTMLIFrameElement,
    target.contentWindow.frameElement === target
  ].join('|');

  const doc = source.contentDocument;
  const firstBase = doc.createElement('base');
  firstBase.target = 'targetFrame';
  const secondBase = doc.createElement('base');
  secondBase.target = '_self';
  doc.head.append(firstBase, secondBase);

  const link = doc.createElement('a');
  link.href = '/replacement.html';
  link.setAttribute('target', '');
  doc.body.appendChild(link);

  target.addEventListener('load', () => {
    __targetLoadCount += 1;
    try {
      __targetLoadAccess = target.contentDocument.location.href;
    } catch (error) {
      __targetLoadAccess = `${error && error.name}:${error && error.message}`;
    }
  }, true);
  link.click();
})()
"#,
    )
    .expect("base-target replacement navigation should start");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__targetLoadCount)",
        "1",
        "base-target replacement document should load",
    )
    .await;

    assert_eq!(
        vm.eval("__baseTargetOwnerRealm")
            .expect("base-target frame owner realm result should evaluate"),
        "true|true|false|true",
        "parser-created frame elements and window.frameElement must use the owner Document realm"
    );
    assert_eq!(
        vm.eval("__targetLoadAccess")
            .expect("target load callback access result should evaluate"),
        replacement_url.as_str(),
        "the replacement Document must be same-origin accessible during the iframe load callback"
    );

    let mut targets = server.finish_targets().await;
    targets.sort();
    assert_eq!(
        targets,
        ["/replacement.html", "/source.html", "/target.html"]
    );
}

#[test]
fn no_src_iframe_initial_about_blank_has_a_quirks_empty_document() {
    let mut vm = new_storage_test_vm("https://iframe-initial-document.test/page.html");
    vm.document_runtime
        .set_document_character_set("windows-1252");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(frame);
  const doc = frame.contentDocument;
  return JSON.stringify({
    compatMode: doc.compatMode,
    contentType: doc.contentType,
    readyState: doc.readyState,
    documentURI: doc.documentURI,
    url: doc.URL,
    doctypeIsNull: doc.doctype === null,
    characterSet: doc.characterSet,
    documentChildCount: doc.childNodes.length,
    documentElement: doc.documentElement.tagName,
    documentElementChildCount: doc.documentElement.childNodes.length,
    head: doc.documentElement.firstChild.tagName,
    headChildCount: doc.head.childNodes.length,
    body: doc.documentElement.lastChild.tagName,
    bodyChildCount: doc.body.childNodes.length
  });
})()
"#,
        )
        .expect("initial about:blank document shape should evaluate");

    assert_eq!(
        result,
        r#"{"compatMode":"BackCompat","contentType":"text/html","readyState":"complete","documentURI":"about:blank","url":"about:blank","doctypeIsNull":true,"characterSet":"UTF-8","documentChildCount":1,"documentElement":"HTML","documentElementChildCount":2,"head":"HEAD","headChildCount":0,"body":"BODY","bodyChildCount":0}"#
    );
}

#[test]
fn removing_iframe_discards_retained_window_relations_synchronously() {
    let mut vm = new_storage_test_vm("https://iframe-discard-window-relations.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const child = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(child);
  const childWindow = child.contentWindow;
  const childDocument = childWindow.document;

  const grandchild = childDocument.createElement("iframe");
  childDocument.body.appendChild(grandchild);
  const grandchildWindow = grandchild.contentWindow;
  const grandchildDocument = grandchildWindow.document;

  const before = {
    childContentWindowIsStable: child.contentWindow === childWindow,
    childParentIsTop: childWindow.parent === window,
    childTopIsTop: childWindow.top === window,
    childFrameElementIsOwner: childWindow.frameElement === child,
    grandchildContentWindowIsStable: grandchild.contentWindow === grandchildWindow,
    grandchildParentIsChild: grandchildWindow.parent === childWindow,
    grandchildTopIsTop: grandchildWindow.top === window,
    grandchildFrameElementIsOwner: grandchildWindow.frameElement === grandchild
  };

  child.parentNode.removeChild(child);
  const after = {
    childContentWindowIsNull: child.contentWindow === null,
    childParentIsNull: childWindow.parent === null,
    childTopIsNull: childWindow.top === null,
    childFrameElementIsNull: childWindow.frameElement === null,
    childDocumentIsRetained: childWindow.document === childDocument,
    grandchildContentWindowIsNull: grandchild.contentWindow === null,
    grandchildParentIsNull: grandchildWindow.parent === null,
    grandchildTopIsNull: grandchildWindow.top === null,
    grandchildFrameElementIsNull: grandchildWindow.frameElement === null,
    grandchildDocumentIsRetained: grandchildWindow.document === grandchildDocument
  };
  return JSON.stringify({ before, after });
})()
"#,
        )
        .expect("iframe discard relation probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":{"childContentWindowIsStable":true,"childParentIsTop":true,"childTopIsTop":true,"childFrameElementIsOwner":true,"grandchildContentWindowIsStable":true,"grandchildParentIsChild":true,"grandchildTopIsTop":true,"grandchildFrameElementIsOwner":true},"after":{"childContentWindowIsNull":true,"childParentIsNull":true,"childTopIsNull":true,"childFrameElementIsNull":true,"childDocumentIsRetained":true,"grandchildContentWindowIsNull":true,"grandchildParentIsNull":true,"grandchildTopIsNull":true,"grandchildFrameElementIsNull":true,"grandchildDocumentIsRetained":true}}"#
    );
}

#[test]
fn moving_iframe_into_own_child_document_discards_retained_window_relations() {
    let mut vm = new_storage_test_vm("https://iframe-own-child-document.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const iframe = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(iframe);
  const childWindow = iframe.contentWindow;
  const childDocument = childWindow.document;

  const before = {
    parentIsTop: childWindow.parent === window,
    topIsTop: childWindow.top === window,
    frameElementIsOwner: childWindow.frameElement === iframe,
    contentWindowIsStable: iframe.contentWindow === childWindow
  };

  childDocument.body.appendChild(iframe);
  const after = {
    parentIsNull: childWindow.parent === null,
    topIsNull: childWindow.top === null,
    frameElementIsNull: childWindow.frameElement === null,
    contentWindowIsNull: iframe.contentWindow === null,
    documentIsRetained: childWindow.document === childDocument,
    movedIntoChildDocument: iframe.ownerDocument === childDocument,
    remainsInserted: childDocument.body.firstChild === iframe
  };
  return JSON.stringify({ before, after });
})()
"#,
        )
        .expect("self-descendant iframe move probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":{"parentIsTop":true,"topIsTop":true,"frameElementIsOwner":true,"contentWindowIsStable":true},"after":{"parentIsNull":true,"topIsNull":true,"frameElementIsNull":true,"contentWindowIsNull":true,"documentIsRetained":true,"movedIntoChildDocument":true,"remainsInserted":true}}"#
    );
}

#[test]
fn iframe_src_navigation_uses_owner_document_encoding_for_query() {
    let mut vm = new_storage_test_vm("https://iframe-src-encoding.test/page.html");
    vm.document_runtime
        .set_document_character_set("windows-1252");

    let reflected_src = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "encoded-src-frame";
  frame.src = "resources/frame.html?\u00DF";
  (document.body || document.documentElement || document).appendChild(frame);
  return frame.src;
})()
"#,
        )
        .expect("legacy-encoded iframe src setup should evaluate");
    assert_eq!(
        reflected_src,
        "https://iframe-src-encoding.test/resources/frame.html?%DF"
    );

    let child_handle = vm
        .document_runtime
        .dom_host()
        .element_handle_by_id("encoded-src-frame")
        .expect("encoded src iframe owner");
    let attribute_bootstrap = vm
        ._context_host
        .borrow()
        .child_browsing_context_attribute_bootstrap_for_test(child_handle)
        .expect("encoded src attribute bootstrap should exist");
    assert!(
        matches!(
            attribute_bootstrap,
            crate::native_bridge::ChildBrowsingContextBootstrap::Url(ref url)
                if url.as_str()
                    == "https://iframe-src-encoding.test/resources/frame.html?%DF"
        ),
        "iframe navigation must use the same owner-document encoding as its reflected src: {attribute_bootstrap:?}"
    );
}

#[test]
fn reattaching_iframe_does_not_resurrect_retired_window_relations() {
    let mut vm = new_storage_test_vm("https://loaded-iframe-discard-relations.test/");

    vm.eval(
        r#"
(() => {
  const child = document.createElement("iframe");
  (document.body || document.documentElement || document).appendChild(child);
  const grandchild = child.contentDocument.createElement("iframe");
  grandchild.srcdoc = "<!doctype html><p>loaded grandchild</p>";
  child.contentDocument.body.appendChild(grandchild);
  globalThis.__loadedDiscardChild = child;
  globalThis.__loadedDiscardGrandchild = grandchild;
  return "scheduled";
})()
"#,
    )
    .expect("loaded descendant frame setup should evaluate");
    vm.drain_pending_child_frame_work_for_test();

    let result = vm
        .eval(
            r#"
(() => {
  const child = globalThis.__loadedDiscardChild;
  const grandchild = globalThis.__loadedDiscardGrandchild;
  const childWindow = child.contentWindow;
  const grandchildWindow = grandchild.contentWindow;
  const before = {
    grandchildLoaded: grandchildWindow.document.body.textContent === "loaded grandchild",
    childParentIsTop: childWindow.parent === window,
    childTopIsTop: childWindow.top === window,
    grandchildParentIsChild: grandchildWindow.parent === childWindow,
    grandchildTopIsTop: grandchildWindow.top === window
  };

  child.remove();
  const detached = {
    childParentIsNull: childWindow.parent === null,
    childTopIsNull: childWindow.top === null,
    grandchildParentIsNull: grandchildWindow.parent === null,
    grandchildTopIsNull: grandchildWindow.top === null
  };

  (document.body || document.documentElement || document).appendChild(child);
  const reattached = {
    newChildWindow: child.contentWindow !== childWindow,
    oldChildParentIsNull: childWindow.parent === null,
    oldChildTopIsNull: childWindow.top === null,
    oldGrandchildParentIsNull: grandchildWindow.parent === null,
    oldGrandchildTopIsNull: grandchildWindow.top === null,
    oldGrandchildRemoved: child.contentDocument.querySelector("iframe") === null
  };
  return JSON.stringify({ before, detached, reattached });
})()
"#,
        )
        .expect("loaded descendant frame discard probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":{"grandchildLoaded":true,"childParentIsTop":true,"childTopIsTop":true,"grandchildParentIsChild":true,"grandchildTopIsTop":true},"detached":{"childParentIsNull":true,"childTopIsNull":true,"grandchildParentIsNull":true,"grandchildTopIsNull":true},"reattached":{"newChildWindow":true,"oldChildParentIsNull":true,"oldChildTopIsNull":true,"oldGrandchildParentIsNull":true,"oldGrandchildTopIsNull":true,"oldGrandchildRemoved":true}}"#
    );
}
