use super::*;

#[tokio::test]
async fn document_open_aborts_requests_when_cancelling_child_navigation() {
    for navigation in [
        "w.location.href='/replacement'",
        "const link=d.body.appendChild(d.createElement('a'));link.href='/replacement';link.click()",
        "f.src='/replacement'",
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = new_parsed_page_task_executor_test_vm(
            "https://document-open-abort.test/",
            "<!doctype html><body>parent",
            &loader,
        );
        let source = format!(
            r#"
globalThis.__documentAbortLog = [];
const f = document.body.appendChild(document.createElement('iframe'));
const w = f.contentWindow, d = w.document;
for (const keepalive of [false, true]) {{
  w.fetch('/pending-' + keepalive, {{keepalive}}).then(
    () => __documentAbortLog.push(['fulfilled', keepalive]),
    error => __documentAbortLog.push(['fetch', keepalive, error.name, error instanceof w.TypeError]));
}}
const xhr = new w.XMLHttpRequest();
xhr.open('GET', '/pending-xhr');
xhr.onabort = () => __documentAbortLog.push(['xhr', xhr.readyState, xhr.status]);
xhr.onload = () => __documentAbortLog.push(['load']);
xhr.send();
{navigation};
d.open();
JSON.stringify([__documentAbortLog, xhr.readyState, d === w.document, d.childNodes.length]);
"#
        );
        assert_eq!(vm.eval(&source).unwrap(), "[[],1,true,0]", "{navigation}");
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__documentAbortLog.length)",
            "3",
            "document.open navigation request cancellation",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(__documentAbortLog.sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b))))").unwrap(),
            r#"[["fetch",false,"TypeError",true],["fetch",true,"TypeError",true],["xhr",4,0]]"#,
            "{navigation}"
        );
        assert_eq!(vm.eval("d === f.contentDocument").unwrap(), "true");
    }
}

#[tokio::test]
async fn document_open_without_navigation_preserves_child_requests() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://document-open-preserve.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.eval(
        r#"
const f = document.body.appendChild(document.createElement('iframe'));
const w = f.contentWindow;
w.fetch('/pending-fetch');
w.fetch('/pending-keepalive', {keepalive:true});
const xhr = new w.XMLHttpRequest();
xhr.open('GET', '/pending-xhr');xhr.send();
"#,
    )
    .unwrap();
    let (fetches, xhrs) = {
        let host = vm._context_host.borrow();
        (
            host.active_window_fetch_contexts_for_test(),
            host.pending_window_xhr_execution_contexts_for_test(),
        )
    };
    assert_eq!(fetches.len(), 2);
    assert_eq!(xhrs.len(), 1);
    vm.eval("w.document.open()").unwrap();
    let host = vm._context_host.borrow();
    assert_eq!(host.active_window_fetch_contexts_for_test(), fetches);
    assert_eq!(host.pending_window_xhr_execution_contexts_for_test(), xhrs);
    for id in fetches
        .iter()
        .map(|(id, ..)| *id)
        .chain(xhrs.iter().map(|(id, ..)| *id))
    {
        assert!(host.async_subresource_fetch_event_target_is_current(
            crate::types::AsyncSubresourceFetchEventTarget::Completion { internal_id: id }
        ));
        assert!(!host.async_subresource_fetch_event_target_is_current(
            crate::types::AsyncSubresourceFetchEventTarget::DocumentAbort { internal_id: id }
        ));
    }
}

#[tokio::test]
async fn document_open_navigation_abort_does_not_touch_a_reopened_xhr() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://document-open-reused-xhr.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.eval(
        r#"
globalThis.__documentAbortReuse = [];
const f = document.body.appendChild(document.createElement('iframe'));
const w = f.contentWindow, d = w.document;
w.fetch('/pending-fetch').catch(() => __documentAbortReuse.push('fetch'));
const xhr = new w.XMLHttpRequest();
xhr.open('GET', '/pending-xhr');
xhr.onabort = () => __documentAbortReuse.push('old xhr');
xhr.send();
w.location.href='/replacement';
d.open();
xhr.open('GET', '/new-xhr');
"#,
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "JSON.stringify(__documentAbortReuse)",
        r#"["fetch"]"#,
        "reopened XHR ignores queued document abort",
    )
    .await;
    assert_eq!(vm.eval("String(xhr.readyState)").unwrap(), "1");
}

#[tokio::test]
async fn child_document_open_remains_disabled_after_window_stop_aborts_its_parser() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://document-open-stopped-parser.test/",
        "<!doctype html><body>parent",
        &loader,
    );
    vm.eval(r#"
globalThis.__stoppedParserResult = '';
const f = document.createElement('iframe');
f.srcdoc = '<!doctype html><p>kept</p><script>stop();setTimeout(()=>{document.open();document.write("replacement");parent.__stoppedParserResult=document.querySelector("p")?.textContent||"lost";},0)</scr'+'ipt><p>unparsed</p>';
document.body.appendChild(f);
"#).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "__stoppedParserResult",
        "kept",
        "stopped parser keeps document.open disabled in a later task",
    )
    .await;
    assert_eq!(
        vm.eval("String(f.contentDocument.querySelectorAll('p').length)")
            .unwrap(),
        "1"
    );
}
