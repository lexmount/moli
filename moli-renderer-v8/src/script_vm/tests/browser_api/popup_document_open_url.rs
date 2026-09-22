use super::*;

#[test]
fn popup_document_open_updates_url_history_and_resource_context_from_entry() {
    let mut vm = new_storage_test_vm("https://popup-open-url.test/entry/page?query#fragment");
    let main_owner = vm.current_main_document_task_owner();
    vm.eval(
        r#"
const root = document.appendChild(document.createElement('html'));
const head = root.appendChild(document.createElement('head'));
root.appendChild(document.createElement('body'));
const base = head.appendChild(document.createElement('base'));
base.href = '/different-base/';
globalThis.__urlPopup = open();
"#,
    )
    .unwrap();
    let popup_id = vm.take_pending_popup_activations()[0].popup_id().unwrap();
    let loader = vm
        .with_default_context_scope(|_, host_ptr| {
            let host = unsafe { &*host_ptr };
            let owner = host
                .current_lightweight_popup_document_owner(popup_id)
                .unwrap();
            Ok(host
                .document_resource_loader_for_window_owner(
                    crate::native_bridge::WindowDocumentOwner::LightweightPopup(owner),
                )
                .unwrap()
                .clone())
        })
        .unwrap();
    let captured = loader.fetch_context();
    let result = vm
        .eval(
            r#"
(() => {
  const w = __urlPopup, d = w.document;
  w.history.replaceState({map:new Map([['kept',42]])}, '');
  const state = w.history.state, length = w.history.length;
  const same = d.open() === d;
  const a = d.createElement('a'); a.href = 'relative';
  const after = [same,w.document === d,d.URL,d.documentURI,d.baseURI,w.location.href,
    w.history.length === length,w.history.state === state,state.map.get('kept'),a.href];
  w.history.pushState(null, '', '?next');
  after.push(w.history.length === length + 1);
  d.close();
  return JSON.stringify(after);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"[true,true,"https://popup-open-url.test/entry/page?query","https://popup-open-url.test/entry/page?query","https://popup-open-url.test/entry/page?query","https://popup-open-url.test/entry/page?query",true,true,42,"https://popup-open-url.test/entry/relative",true]"#
    );
    let current = loader.fetch_context();
    assert_eq!(captured.document_url().as_str(), "about:blank");
    assert_eq!(
        captured.base_url().as_str(),
        "https://popup-open-url.test/different-base/"
    );
    assert_eq!(
        current.document_url().as_str(),
        "https://popup-open-url.test/entry/page?next"
    );
    assert_eq!(current.base_url(), current.document_url());
    assert_eq!(current.owner(), captured.owner());
    assert_eq!(current.origin(), captured.origin());
    assert_eq!(current.origin(), "https://popup-open-url.test");
    assert_eq!(vm.current_main_document_task_owner(), main_owner);
    vm.eval("__urlPopup.close()").unwrap();
}

#[tokio::test]
async fn popup_document_open_and_implicit_write_update_loaded_popup_urls() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-open-loaded.test/entry?query#fragment",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(
        r#"
window.__loadedUrlPopups = [];
window.__loadedUrlResults = [];
window.__loadedUrlReady = () => {
  for (const [mode,w] of __loadedUrlPopups) {
    const d = w.document, expected = document.URL.split('#')[0];
    w.history.replaceState({kept:42}, '');
    const state = w.history.state, length = w.history.length;
    let same = true;
    if (mode === 'borrowed') same = Document.prototype.open.call(d) === d;
    else if (mode === 'write' || mode === 'writeln') d[mode]('replacement');
    else same = d.open() === d;
    __loadedUrlResults.push([mode,same,d === w.document,d.URL === expected,
      d.documentURI === expected,d.baseURI === expected,w.location.href === expected,
      w.history.state === state,w.history.length === length,
      d.body && d.body.textContent]);
    d.close();
  }
};
window.__loadedUrlCount = 0;
const source = URL.createObjectURL(new Blob(['<!doctype html><body>original'], {type:'text/html'}));
for (const mode of ['open','borrowed','write','writeln']) {
  const w = open(source);
  __loadedUrlPopups.push([mode,w]);
  w.addEventListener('load', () => {
    ++__loadedUrlCount;
  }, {once:true});
}
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__loadedUrlCount)",
        "4",
        "loaded popup document.open URLs",
    )
    .await;
    vm.eval("__loadedUrlReady()").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(__loadedUrlResults)").unwrap(),
        r#"[["open",true,true,true,true,true,true,true,true,null],["borrowed",true,true,true,true,true,true,true,true,null],["write",true,true,true,true,true,true,true,true,"replacement"],["writeln",true,true,true,true,true,true,true,true,"replacement\n"]]"#
    );
    vm.eval("__loadedUrlPopups.forEach(([,w]) => w.close())")
        .unwrap();
}

#[tokio::test]
async fn popup_document_open_uses_popup_entry_and_preserves_self_fragment() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-open-entry.test/root#root-fragment",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(
        r#"
window.__entryTarget = open();
window.__entryUrlResults = [];
window.__entryUrlPopups = ['self','other'].map(mode => {
  const html = `<!doctype html><script>
    onload = () => {
      const selfTarget = ${JSON.stringify(mode)} === 'self';
      const target = selfTarget ? document : opener.__entryTarget.document;
      const expected = selfTarget ? document.URL : document.URL.split('#')[0];
      const same = target.open() === target;
      opener.__entryUrlResults.push([${JSON.stringify(mode)},same,target.URL === expected,
        target.documentURI === expected,target.baseURI === expected,
        target.defaultView.location.href === expected]);
      target.close();
    };
  </script><body>original`;
  return open(URL.createObjectURL(new Blob([html],{type:'text/html'})) + '#popup-fragment');
});
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__entryUrlResults.length)",
        "2",
        "popup entry document URL",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__entryUrlResults.sort())").unwrap(),
        r#"[["other",true,true,true,true,true],["self",true,true,true,true,true]]"#
    );
    vm.eval("__entryUrlPopups.forEach(w=>w.close()); __entryTarget.close();")
        .unwrap();
}

#[tokio::test]
async fn popup_document_open_keeps_reentrant_history_update_last() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-open-reentrant.test/entry#fragment",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(r#"
window.__reentrantLoaded = false;
window.__reentrantPopup = open(URL.createObjectURL(new Blob(['<!doctype html>'],{type:'text/html'})));
__reentrantPopup.addEventListener('load',()=>{__reentrantLoaded=true},{once:true});
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__reentrantLoaded)",
        "true",
        "loaded popup history callback",
    )
    .await;
    let result = vm
        .eval(
            r#"
(() => {
  const w = __reentrantPopup, d = w.document;
  let count = 0;
  try {
    w.navigation.addEventListener('currententrychange', () => {
      ++count;
      w.history.replaceState({nested:true}, '', '/nested#final');
    }, {once:true});
    d.open();
    d.close();
    return JSON.stringify([count,d.URL,d.documentURI,d.baseURI,w.location.href,w.history.state]);
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"[1,"https://popup-open-reentrant.test/nested#final","https://popup-open-reentrant.test/nested#final","https://popup-open-reentrant.test/nested#final","https://popup-open-reentrant.test/nested#final",{"nested":true}]"#
    );
}

#[test]
fn popup_document_open_updates_initial_document_before_queued_close() {
    let mut vm = new_storage_test_vm("https://popup-open-closing.test/entry#fragment");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document;
  let changes = 0;
  w.navigation.addEventListener('currententrychange',()=>++changes);
  w.close();
  const same = d.open() === d;
  d.close();
  return JSON.stringify([same,d.URL,d.documentURI,d.baseURI,changes]);
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"[true,"https://popup-open-closing.test/entry","https://popup-open-closing.test/entry","https://popup-open-closing.test/entry",0]"#
    );
}
