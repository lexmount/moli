use super::*;

#[test]
fn named_popup_unload_reentry_does_not_publish_a_rejected_activation() {
    for event in ["beforeunload", "pagehide"] {
        let mut vm = new_storage_test_vm("https://popup-unload.test/opener");
        vm.eval("globalThis.reentryPopup = open('about:blank', 'unloading-popup')")
            .unwrap();
        assert_eq!(vm.take_pending_popup_activations().len(), 1);
        vm.eval(&format!(
            r#"
globalThis.reentryCalls = 0;
reentryPopup.addEventListener({event:?}, () => {{
  ++reentryCalls;
  globalThis.reentryReturned = open('https://popup-unload.test/forbidden', 'unloading-popup');
}});
reentryPopup.location.href = 'about:blank?replacement';
"#
        ))
        .unwrap();
        assert_eq!(vm.eval("String(reentryCalls)").unwrap(), "1", "{event}");
        assert_eq!(
            vm.eval("String(reentryReturned === reentryPopup)").unwrap(),
            "true"
        );
        assert_eq!(
            vm.eval("reentryPopup.location.href").unwrap(),
            "about:blank?replacement"
        );
        assert!(vm.take_pending_popup_activations().is_empty(), "{event}");
    }
}

#[test]
fn window_open_empty_url_does_not_queue_special_target_navigation() {
    for target in ["_self", "_parent", "_top", "_SeLf", "_PaReNt", "_TOP"] {
        let mut vm = new_storage_test_vm("https://empty-open.test/current");
        let result = vm
            .eval(&format!(
                r#"(() => {{
                  const before = document;
                  const calls = ['', undefined, {{toString() {{return ''}}}}];
                  return String(calls.every(url =>
                    open(url, '{target}') === window && document === before));
                }})()"#
            ))
            .expect("empty URL should select the existing target");
        assert_eq!(result, "true", "{target}");
        assert!(vm.take_pending_popup_activations().is_empty());
        assert!(vm.take_pending_location_navigation_with_seed().is_none());

        vm.eval(&format!("open('about:blank', '{target}')"))
            .expect("explicit about:blank should navigate");
        assert_eq!(
            vm.take_pending_location_navigation_with_seed()
                .expect("explicit URL navigation")
                .url
                .as_str(),
            "about:blank"
        );
    }
}

#[tokio::test]
async fn window_open_empty_url_preserves_loaded_named_documents_and_history() {
    for kind in ["iframe", "popup"] {
        let loader = static_http_loader([]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://empty-open.test/current",
            "<!doctype html><body>opener",
            &loader,
        );
        vm.eval(&format!(
            r#"
globalThis.__emptyOpenKind = '{kind}';
globalThis.__emptyOpenReady = false;
const source = URL.createObjectURL(new Blob([
  '<!doctype html><body>retained document'
], {{type:'text/html'}}));
if (__emptyOpenKind === 'iframe') {{
  const frame = document.createElement('iframe');
  frame.name = 'empty-open-target';
  frame.src = source;
  frame.onload = () => {{__emptyOpenReady = true}};
  document.body.append(frame);
  globalThis.__emptyOpenFrame = frame;
  globalThis.__emptyOpenWindow = frame.contentWindow;
}} else {{
  const popup = open(source, 'empty-open-target');
  popup.onload = () => {{__emptyOpenReady = true}};
  globalThis.__emptyOpenWindow = popup;
}}
"#
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__emptyOpenReady)",
            "true",
            "initial named document load",
        )
        .await;
        vm.take_pending_popup_activations();
        vm.eval(
            r#"
globalThis.__emptyOpenDone = false;
globalThis.__emptyOpenRows = [];
const w = __emptyOpenWindow, before = w.document, href = w.location.href;
w.history.replaceState({kept:42}, '');
const state = w.history.state, length = w.history.length;
const marker = before.body.appendChild(before.createElement('p'));
marker.textContent = 'marker';
let loads = 0;
if (__emptyOpenKind === 'iframe') __emptyOpenFrame.onload = () => ++loads;
else { w.onload = () => ++loads; w.opener = null; }
for (const url of ['', undefined, {toString() {return ''}}]) {
  const returned = open(url, 'empty-open-target');
  __emptyOpenRows.push([returned === w, w.document === before,
    w.location.href === href, w.history.state === state, w.history.length === length]);
}
setTimeout(() => {
  __emptyOpenRows.push([w.document === before, w.location.href === href,
    w.history.state === state, w.history.length === length,
    before.body.contains(marker), loads === 0,
    w.opener === (__emptyOpenKind === 'iframe' ? window : null)]);
  __emptyOpenDone = true;
}, 100);
"#,
        )
        .unwrap();
        assert!(vm.take_pending_popup_activations().is_empty(), "{kind}");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__emptyOpenDone)",
            "true",
            "empty URL must not queue another document load",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__emptyOpenRows)").unwrap(),
            "[[true,true,true,true,true],[true,true,true,true,true],[true,true,true,true,true],[true,true,true,true,true,true,true]]",
            "{kind}"
        );
    }
}

#[tokio::test]
async fn window_open_empty_url_preserves_pending_named_navigation() {
    for kind in ["iframe", "popup"] {
        let loader = static_http_loader([]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://empty-open.test/current",
            "<!doctype html><body>opener",
            &loader,
        );
        vm.eval(&format!(
            r#"
globalThis.__pendingEmptyReady = false;
globalThis.__pendingEmptySource = URL.createObjectURL(new Blob([
  '<!doctype html><body>pending completed'
], {{type:'text/html'}}));
if ('{kind}' === 'iframe') {{
  const frame = document.createElement('iframe');
  frame.name = 'pending-empty-target';
  frame.src = __pendingEmptySource;
  frame.onload = () => {{__pendingEmptyReady = true}};
  document.body.append(frame);
  globalThis.__pendingEmptyWindow = frame.contentWindow;
}} else {{
  const popup = open(__pendingEmptySource, 'pending-empty-target');
  popup.onload = () => {{__pendingEmptyReady = true}};
  globalThis.__pendingEmptyWindow = popup;
}}
globalThis.__pendingEmptyReturned = open('', 'pending-empty-target');
"#
        ))
        .unwrap();
        assert_eq!(
            vm.take_pending_popup_activations().len(),
            usize::from(kind == "popup"),
            "empty URL must not create a second popup activation"
        );
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__pendingEmptyReady)",
            "true",
            "original navigation must finish",
        )
        .await;
        assert_eq!(
            vm.eval(
                "JSON.stringify([__pendingEmptyReturned === __pendingEmptyWindow, \
                 __pendingEmptyWindow.location.href === __pendingEmptySource, \
                 __pendingEmptyWindow.document.body.textContent])"
            )
            .unwrap(),
            r#"[true,true,"pending completed"]"#,
            "{kind}"
        );
    }
}

#[tokio::test]
async fn window_open_empty_url_selects_special_targets_from_child_scripts() {
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://empty-open.test/current",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(
        r#"
globalThis.__emptyChildResults = [];
const frame = document.createElement('iframe');
frame.srcdoc = `<script>
  for (const url of ['', undefined, {toString() {return ''}}]) {
    for (const target of ['_self','_parent','_top']) {
      const expected = target === '_self' ? window : parent;
      const before = expected.document, href = expected.location.href;
      parent.__emptyChildResults.push([open(url, target) === expected,
        expected.document === before, expected.location.href === href]);
    }
  }
<\/script>`;
document.body.append(frame);
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__emptyChildResults.length)",
        "9",
        "empty URL child special targets",
    )
    .await;
    assert_eq!(
        vm.eval("String(__emptyChildResults.every(row => row.every(Boolean)))")
            .unwrap(),
        "true"
    );
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
    assert!(vm.take_pending_popup_activations().is_empty());
}
