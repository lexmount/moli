use super::*;

#[tokio::test]
async fn popup_navigation_parser_preserves_nested_writes_and_the_unparsed_tail() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-navigation-parser.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(r#"
globalThis.__popupParseTrace = [];
const nested = URL.createObjectURL(new Blob([
  'opener.__popupParseTrace.push(["nested",!!document.getElementById("tail")]);document.write("<b id=nested>nested</b>");'
], {type:'text/javascript'}));
const written = '<span id=written>one</span><script src="'+nested+'"></script><span id=written-tail>two</span>';
const blocking = URL.createObjectURL(new Blob([
  'const doc=document, before=document.getElementById("before");' +
  'doc.addEventListener("readystatechange",()=>{if(doc.readyState==="interactive")opener.__popupParseTrace.push(["interactive"])});' +
  'opener.__popupParseTrace.push(["blocking",document.currentScript.tagName,!!document.getElementById("tail")]);' +
  'document.open();document.close();document.write('+JSON.stringify(written)+');' +
  'opener.__popupParseTrace.push(["write",doc===document,before===document.getElementById("before"),!!document.getElementById("written")]);'
], {type:'text/javascript'}));
const deferred = URL.createObjectURL(new Blob([
  'opener.__popupParseTrace.push(["deferred",!!document.getElementById("tail"),document.readyState]);document.write("destructive");'
], {type:'text/javascript'}));
const html = '<!doctype html><body><div id=before>before</div><script src="'+blocking+'"></script>' +
  '<script defer src="'+deferred+'"></script><p id=tail>tail</p>' +
  '<script>opener.__popupParseTrace.push(["tail",document.readyState]);Promise.resolve().then(()=>opener.__popupParseTrace.push(["tail-reaction"]));</script>';
globalThis.__parsedPopup = open(URL.createObjectURL(new Blob([html], {type:'text/html'})));
__parsedPopup.onload = () => __popupParseTrace.push(['load', __parsedPopup.document.readyState]);
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupParseTrace.length)",
        "8",
        "popup navigation nested parser scripts and load",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__popupParseTrace)").unwrap(),
        r#"[["blocking","SCRIPT",false],["write",true,true,true],["nested",false],["tail","loading"],["tail-reaction"],["interactive"],["deferred",true,"interactive"],["load","complete"]]"#
    );
    assert_eq!(
        vm.eval("JSON.stringify(Array.from(__parsedPopup.document.querySelectorAll('[id]'),n=>[n.id,n.textContent]))").unwrap(),
        r#"[["before","before"],["written","one"],["nested","nested"],["written-tail","two"],["tail","tail"]]"#
    );
    vm.eval("__parsedPopup.close()").unwrap();
}

#[tokio::test]
async fn popup_navigation_parser_applies_meta_csp_only_to_subsequent_scripts() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-navigation-meta.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(
        r#"
globalThis.__popupParsePolicy = [];
const html = '<!doctype html><head><script>opener.__popupParsePolicy.push("before")</script>' +
  '<meta http-equiv="Content-Security-Policy" content="script-src &apos;nonce-ok&apos;">' +
  '<script>opener.__popupParsePolicy.push("blocked")</script>' +
  '<script nonce=ok>opener.__popupParsePolicy.push("allowed")</script>';
globalThis.__policyPopup = open(URL.createObjectURL(new Blob([html], {type:'text/html'})));
__policyPopup.onload = () => __popupParsePolicy.push('load');
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(__popupParsePolicy)",
        r#"["before","allowed","load"]"#,
        "popup navigation parser policy delivery",
    )
    .await;
    vm.eval("__policyPopup.close()").unwrap();
}

#[tokio::test]
async fn popup_navigation_parser_uses_the_live_base_for_relative_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-navigation-base.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(
        r#"
globalThis.__popupParseBase = [];
const html = '<!doctype html><head><base href="https://popup-navigation-base.test/resources/">' +
  '<script>opener.__popupParseBase.push(document.baseURI,new Request("asset").url);</script>';
globalThis.__basePopup = open(URL.createObjectURL(new Blob([html], {type:'text/html'})));
__basePopup.onload = () => __popupParseBase.push('load');
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(__popupParseBase)",
        r#"["https://popup-navigation-base.test/resources/","https://popup-navigation-base.test/resources/asset","load"]"#,
        "popup navigation parser base URL",
    )
    .await;
    vm.eval("__basePopup.close()").unwrap();
}

#[test]
fn popup_live_document_open_replaces_the_tree_and_close_finishes_the_parser() {
    let mut vm = new_storage_test_vm("https://popup-live-open.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document, root = d.documentElement, old = [...d.childNodes];
  try {
    const initial = [d.compatMode, d.doctype, old.map(n => n.nodeName)];
    const observer = new MutationObserver(() => {});
    observer.observe(d, {childList:true});
    const same = d.open() === d;
    const removals = observer.takeRecords();
    const opened = [same, d === w.document, d.childNodes.length, d.documentElement,
      d.head, d.body, root.isConnected, d.readyState, d.compatMode,
      removals.length, removals[0].removedNodes.length === old.length];
    d.close();
    const closed = [d.readyState, d.body.localName, d.body.textContent];
    return JSON.stringify({initial, opened, closed});
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"initial":["BackCompat",null,["HTML"]],"opened":[true,true,0,null,null,null,false,"loading","CSS1Compat",1,true],"closed":["complete","body",""]}"#
    );
}

#[test]
fn popup_blank_navigation_restores_quirks_mode_without_a_doctype() {
    let mut vm = new_storage_test_vm("https://popup-blank-mode.test/");
    assert_eq!(
        vm.eval(
            r#"
document.open();
document.write('<!doctype html><body>standards opener');
document.close();
globalThis.modePopup = open('about:blank', 'mode-popup');
globalThis.oldModeDocument = modePopup.document;
oldModeDocument.open();
oldModeDocument.write('<!doctype html><body>standards popup');
oldModeDocument.close();
oldModeDocument.compatMode;
"#,
        )
        .unwrap(),
        "CSS1Compat"
    );
    vm.eval("open('about:blank?replacement', 'mode-popup')")
        .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([modePopup.document !== oldModeDocument, modePopup.document.URL, modePopup.document.compatMode, modePopup.document.doctype, Array.from(modePopup.document.childNodes, n => n.nodeName), modePopup.document.body.textContent, document.compatMode])")
            .unwrap(),
        r#"[true,"about:blank?replacement","BackCompat",null,["HTML"],"","CSS1Compat"]"#
    );
    vm.eval("modePopup.close()").unwrap();
}

#[test]
fn popup_live_document_write_preserves_split_tokens_and_native_node_identity() {
    let mut vm = new_storage_test_vm("https://popup-live-token.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document;
  try {
    d.open();
    d.write('<!doctype html><title>Stream');
    d.write(' title</title><p id="fir');
    d.write('st">one');
    const first = d.getElementById('first');
    let events = 0;
    first.addEventListener('click', () => ++events);
    d.write(' two</p><!-- com');
    d.writeln('ment --><p>three</p>');
    d.close();
    first.dispatchEvent(new Event('click'));
    return JSON.stringify([d.title, first === d.getElementById('first'), first.textContent,
      d.body.innerHTML, events, d.readyState]);
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"["Stream title",true,"one two","<p id=\"first\">one two</p><!-- comment --><p>three</p>\n",1,"complete"]"#
    );
}

#[test]
fn popup_live_document_inline_scripts_keep_insertion_order_and_ignore_nested_open() {
    let mut vm = new_storage_test_vm("https://popup-live-scripts.test/");
    let result = vm.eval(r#"
(() => {
  const w = open(), d = w.document;
  globalThis.__popupStreamOrder = [];
  try {
    d.open();
    d.write('<!doctype html><div id="before"></div><scr');
    d.write('ipt>opener.__popupStreamOrder.push([document===window.document, document.currentScript.tagName, !!document.getElementById("after")]);document.open();document.write("<span id=written>nested</span>");opener.__popupStreamOrder.push(!!document.getElementById("written"));document.close();</scr');
    d.write('ipt><p id="after">after</p>');
    const order = __popupStreamOrder.slice();
    return JSON.stringify({order, nodes:Array.from(d.body.children,n=>n.id||n.tagName),
      ready:d.readyState, current:d.currentScript});
  } finally { w.close(); }
})()
"#).unwrap();
    assert_eq!(
        result,
        r#"{"order":[[true,"SCRIPT",false],true],"nodes":["before","SCRIPT","written","after"],"ready":"complete","current":null}"#
    );
}

#[test]
fn popup_live_document_parser_delivers_meta_csp_before_inline_script_preparation() {
    let mut vm = new_storage_test_vm("https://popup-live-csp.test/");
    let result = vm.eval(r#"
(() => {
  const w = open(), d = w.document;
  globalThis.__popupStreamCsp = [];
  try {
    d.open();
    d.write('<!doctype html><meta http-equiv="Content-Security-Policy" content="script-src &apos;nonce-ok&apos;"><body><script>opener.__popupStreamCsp.push("blocked")</script><script nonce="ok">opener.__popupStreamCsp.push("allowed")</script>');
    d.close();
    return JSON.stringify(__popupStreamCsp);
  } finally { w.close(); }
})()
"#).unwrap();
    assert_eq!(result, r#"["allowed"]"#);
}

#[tokio::test]
async fn popup_live_document_csp_blocked_script_cannot_execute_after_adoption() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-live-blocked-adoption.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    let result = vm.eval(r#"
(() => {
  const w = open(), d = w.document;
  globalThis.__popupBlockedStream = 0;
  try {
    d.open();
    d.write('<!doctype html><meta http-equiv="Content-Security-Policy" content="script-src &apos;none&apos;"><body><script id=s>window.__popupBlockedStream++</script>');
    d.close();
    document.body.appendChild(d.getElementById('s'));
    return String(__popupBlockedStream);
  } finally { w.close(); }
})()
"#).unwrap();
    assert_eq!(result, "0");
}

#[test]
fn popup_live_document_custom_element_callback_writes_at_the_active_insertion_point() {
    let mut vm = new_storage_test_vm("https://popup-live-ce-write.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document, calls = [];
  try {
    const registry = new CustomElementRegistry();
    registry.initialize(d);
    registry.define('popup-stream-writer', class extends HTMLElement {
      connectedCallback() {
        calls.push('connected');
        d.write('<b id=nested>nested</b>');
      }
    });
    d.open();
    d.write('<!doctype html><body><popup-stream-writer></popup-stream-writer><p id=tail>tail</p>');
    d.close();
    return JSON.stringify({calls, body:d.body.innerHTML});
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"calls":["connected"],"body":"<popup-stream-writer></popup-stream-writer><p id=\"tail\">tail</p><b id=\"nested\">nested</b>"}"#
    );
}

#[test]
fn popup_live_document_custom_element_callback_can_replace_the_active_parser() {
    let mut vm = new_storage_test_vm("https://popup-live-ce-open.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document;
  try {
    const registry = new CustomElementRegistry();
    registry.initialize(d);
    registry.define('popup-stream-opener', class extends HTMLElement {
      connectedCallback() {
        d.open();
        d.write('<!doctype html><p>replacement</p>');
        d.close();
      }
    });
    d.open();
    d.write('<!doctype html><body><popup-stream-opener></popup-stream-opener><p>stale tail</p>');
    return JSON.stringify([d.body.textContent, d.readyState]);
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(result, r#"["replacement","complete"]"#);
}

#[test]
fn popup_live_document_readiness_callback_can_replace_a_finishing_stream() {
    let mut vm = new_storage_test_vm("https://popup-live-readiness-reentry.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const w = open(), d = w.document;
  try {
    d.open();
    d.addEventListener('readystatechange', function once() {
      if (d.readyState === 'interactive') {
        d.removeEventListener('readystatechange', once);
        d.open();
        d.write('<!doctype html><p>replacement</p>');
      }
    });
    d.write('<!doctype html><p>old</p>');
    d.close();
    return JSON.stringify([d.body.textContent, d.readyState]);
  } finally { w.close(); }
})()
"#,
        )
        .unwrap();
    assert_eq!(result, r#"["replacement","loading"]"#);
}

#[tokio::test]
async fn popup_live_document_external_script_resumes_before_close_and_deferred_work() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-live-external.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(r#"
globalThis.__popupStreamExternal = [];
globalThis.__popupStreamWindow = open();
const d = __popupStreamWindow.document;
d.open();
const blocking = URL.createObjectURL(new Blob([
  'opener.__popupStreamExternal.push(["blocking",!!document.getElementById("after")]);document.write("<b id=nested>nested</b>");'
], {type:'text/javascript'}));
const deferred = URL.createObjectURL(new Blob([
  'opener.__popupStreamExternal.push(["deferred",!!document.getElementById("after"),document.readyState]);'
], {type:'text/javascript'}));
d.write('<!doctype html><script src="'+blocking+'"></script><p id=after>after</p><script defer src="'+deferred+'"></script>');
globalThis.__popupStreamPaused = !d.getElementById('after') && d.readyState === 'loading';
d.close();
__popupStreamWindow.addEventListener('load', () => __popupStreamExternal.push(['load', d.readyState]));
"#).unwrap();
    assert_eq!(vm.eval("String(__popupStreamPaused)").unwrap(), "true");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupStreamExternal.length)",
        "3",
        "popup stream external and deferred scripts",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__popupStreamExternal)").unwrap(),
        r#"[["blocking",false],["deferred",true,"interactive"],["load","complete"]]"#
    );
    assert_eq!(
        vm.eval("__popupStreamWindow.document.getElementById('nested').textContent")
            .unwrap(),
        "nested"
    );
    vm.eval("__popupStreamWindow.close()").unwrap();
}

#[tokio::test]
async fn popup_live_document_replacement_discards_old_external_script_completion() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-live-replacement.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(r#"
globalThis.__popupStreamRetired = [];
globalThis.__popupStreamReplacement = open();
const d = __popupStreamReplacement.document;
d.open();
const source = URL.createObjectURL(new Blob(['opener.__popupStreamRetired.push("old")'], {type:'text/javascript'}));
d.write('<!doctype html><script src="'+source+'"></script><p>old tail</p>');
d.close();
d.open();
d.write('<!doctype html><p>replacement</p>');
__popupStreamReplacement.addEventListener('load', () => __popupStreamRetired.push('new load'));
d.close();
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(__popupStreamRetired)",
        r#"["new load"]"#,
        "replacement popup stream load",
    )
    .await;
    assert_eq!(
        vm.eval("__popupStreamReplacement.document.body.textContent")
            .unwrap(),
        "replacement"
    );
    vm.eval("__popupStreamReplacement.close()").unwrap();
}

#[tokio::test]
async fn popup_live_document_async_script_keeps_parsing_and_delays_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://popup-live-async.test/",
        "<!doctype html><body>opener",
        &loader,
    );
    vm.eval(r#"
globalThis.__popupStreamAsync = [];
globalThis.__popupStreamAsyncWindow = open();
const d = __popupStreamAsyncWindow.document;
d.open();
const source = URL.createObjectURL(new Blob([
  'opener.__popupStreamAsync.push(["async",!!document.getElementById("after")]);document.write("destructive");'
], {type:'text/javascript'}));
d.write('<!doctype html><script async src="'+source+'"></script><p id=after>after</p>');
globalThis.__popupStreamNotBlocked = !!d.getElementById('after') && __popupStreamAsync.length === 0;
d.close();
globalThis.__popupStreamAwaitingAsync = d.readyState;
__popupStreamAsyncWindow.addEventListener('load', () => __popupStreamAsync.push(['load', d.body.textContent]));
"#).unwrap();
    assert_eq!(vm.eval("String(__popupStreamNotBlocked)").unwrap(), "true");
    assert_eq!(vm.eval("__popupStreamAwaitingAsync").unwrap(), "complete");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__popupStreamAsync.length)",
        "2",
        "popup stream async script and load",
    )
    .await;
    assert_eq!(
        vm.eval("JSON.stringify(__popupStreamAsync)").unwrap(),
        r#"[["async",true],["load","after"]]"#
    );
    vm.eval("__popupStreamAsyncWindow.close()").unwrap();
}
