use super::*;
use crate::script_provenance::CompiledStringProvenance;

mod event_handler_getter;
mod realm_lifetime;

fn execute_with_source(vm: &mut ScriptVm, source: &str, url: &Url, base: &Url) {
    vm.exec_in_enclosing_script_turn_with_provenance(
        source,
        &CompiledStringProvenance::new(url.clone(), base.clone()),
        0,
        None,
        true,
    )
    .map_err(|error| error.into_anyhow())
    .expect("classic script should execute");
}

#[tokio::test]
async fn classic_xhr_initiator_uses_outer_source_and_survives_promise_and_lazy_publication() {
    let server = StaticHttpServer::spawn_with_responder(
        3,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
        globalThis.finished = 0;
        globalThis.requestFromLibrary = path => {
          const xhr = new XMLHttpRequest();
          xhr.onloadend = () => finished++;
          xhr.open('GET', path);
          xhr.send();
        };
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let caller = base.join("caller.js?version=2").unwrap();
    execute_with_source(
        &mut vm,
        r#"
        requestFromLibrary('direct');
        Promise.resolve().then(() => requestFromLibrary('promise'));
        queueMicrotask(() => requestFromLibrary('microtask'));
    "#,
        &caller,
        &base,
    );
    execute_with_source(
        &mut vm,
        "globalThis.otherScriptRan = true;",
        &base.join("later.js").unwrap(),
        &base,
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(finished)",
        "3",
        "classic XHR initiator completion",
    )
    .await;
    assert_eq!(vm.eval("String(finished)").unwrap(), "3");
    assert_eq!(vm.eval(r#"JSON.stringify(performance.getEntriesByType('resource').map(entry => [entry.initiatorUrl, entry.toJSON().initiatorUrl]))"#).unwrap(),
        serde_json::json!([[caller.as_str(),caller.as_str()],[caller.as_str(),caller.as_str()],[caller.as_str(),caller.as_str()]]).to_string());
    assert_eq!(server.finish().await.len(), 3);
}

#[tokio::test]
async fn classic_xhr_initiator_redacts_cross_origin_and_opaque_script_sources() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    vm.eval("globalThis.redactedDone = 0").unwrap();
    for (index, source) in ["https://other.invalid/script.js", "data:text/javascript,0"]
        .into_iter()
        .enumerate()
    {
        execute_with_source(
            &mut vm,
            &format!(
                "const x{index} = new XMLHttpRequest(); x{index}.onloadend = () => redactedDone++; x{index}.open('GET', 'probe-{index}'); x{index}.send();"
            ),
            &Url::parse(source).unwrap(),
            &base,
        );
    }
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(redactedDone)",
        "2",
        "redacted classic XHR completion",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => entry.initiatorUrl))").unwrap(), r#"["",""]"#);
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn window_timer_resource_initiator_is_per_registration_for_function_string_interval_and_promise()
 {
    let server = StaticHttpServer::spawn_with_responder(
        5,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
      globalThis.request = path => {
        const xhr = new XMLHttpRequest(); xhr.open('GET', path); xhr.send();
      };
      globalThis.sharedTimer = path => request(path);
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let first = base.join("first.js").unwrap();
    let second = base.join("second.js").unwrap();
    execute_with_source(
        &mut vm,
        r#"
      setTimeout(sharedTimer, 0, 'first');
      setTimeout("request('string')", 0);
      const interval = setInterval(() => { clearInterval(interval); request('interval'); }, 0);
      setTimeout(() => Promise.resolve().then(() => request('promise')), 0);
    "#,
        &first,
        &base,
    );
    execute_with_source(
        &mut vm,
        "setTimeout(sharedTimer, 0, 'second');",
        &second,
        &base,
    );
    execute_with_source(
        &mut vm,
        "globalThis.unrelated = true;",
        &base.join("later.js").unwrap(),
        &base,
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "5",
        "callback initiator completion",
    )
    .await;
    let actual = vm.eval(r#"JSON.stringify(performance.getEntriesByType('resource').map(entry => [new URL(entry.name).pathname, entry.initiatorUrl]).sort())"#).unwrap();
    assert_eq!(
        actual,
        serde_json::json!([
            ["/first", first.as_str()],
            ["/interval", first.as_str()],
            ["/promise", first.as_str()],
            ["/second", second.as_str()],
            ["/string", first.as_str()]
        ])
        .to_string()
    );
    assert_eq!(server.finish().await.len(), 5);
}

#[tokio::test]
async fn window_timer_resource_initiator_keeps_unknown_registration_unknown() {
    let server = StaticHttpServer::spawn_with_responder(
        1,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        "setTimeout(() => { const x = new XMLHttpRequest(); x.open('GET', 'unknown'); x.send(); }, 0);",
        &Url::parse("https://other.invalid/registration.js").unwrap(),
        &base,
    );
    execute_with_source(
        &mut vm,
        "globalThis.unrelated = true;",
        &base.join("later.js").unwrap(),
        &base,
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "1",
        "callback initiator completion",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => entry.initiatorUrl))").unwrap(), r#"[""]"#);
    assert_eq!(server.finish().await.len(), 1);
}

fn dispatch_native_document_probe(vm: &mut ScriptVm, event_type: &str) {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host| {
        unsafe { &mut *host }
            .dispatch_document_event(scope, host, event_type)
            .map_err(anyhow::Error::msg)
    })
    .unwrap();
}

#[tokio::test]
async fn dom_listener_resource_initiator_covers_handle_event_lookup_and_duplicate_registration() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
      globalThis.request = path => { const x = new XMLHttpRequest(); x.open('GET', path); x.send(); };
      globalThis.listener = { get handleEvent() { request('lookup'); return () => request('body'); } };
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let first = base.join("first.js").unwrap();
    execute_with_source(
        &mut vm,
        "document.addEventListener('probe', listener);",
        &first,
        &base,
    );
    execute_with_source(
        &mut vm,
        "document.addEventListener('probe', listener);",
        &base.join("duplicate.js").unwrap(),
        &base,
    );
    dispatch_native_document_probe(&mut vm, "probe");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "2",
        "callback initiator completion",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => entry.initiatorUrl))").unwrap(), serde_json::json!([first.as_str(), first.as_str()]).to_string());
    assert_eq!(server.finish().await.len(), 2);
}

#[tokio::test]
async fn dom_listener_resource_initiator_preserves_each_registration_and_synchronous_caller() {
    let server = StaticHttpServer::spawn_with_responder(
        4,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
      globalThis.listener = event => { const x = new XMLHttpRequest(); x.open('GET', event.type); x.send(); };
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let first = base.join("first.js").unwrap();
    let second = base.join("second.js").unwrap();
    let caller = base.join("caller.js").unwrap();
    execute_with_source(
        &mut vm,
        "document.addEventListener('first', listener);",
        &first,
        &base,
    );
    execute_with_source(
        &mut vm,
        "document.addEventListener('second', listener);",
        &second,
        &base,
    );
    dispatch_native_document_probe(&mut vm, "first");
    dispatch_native_document_probe(&mut vm, "second");
    execute_with_source(
        &mut vm,
        "document.dispatchEvent(new Event('first'));",
        &caller,
        &base,
    );
    execute_with_source(
        &mut vm,
        "document.dispatchEvent(new Event('first'));",
        &Url::parse("https://other.invalid/caller.js").unwrap(),
        &base,
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "4",
        "callback initiator completion",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => [new URL(entry.name).pathname, entry.initiatorUrl]).sort())").unwrap(), serde_json::json!([
        ["/first", ""], ["/first", caller.as_str()], ["/first", first.as_str()], ["/second", second.as_str()]
    ]).to_string());
    assert_eq!(server.finish().await.len(), 4);
}

#[tokio::test]
async fn event_handler_resource_initiator_updates_on_reassignment_and_keeps_caller_precedence() {
    let server = StaticHttpServer::spawn_with_responder(
        4,
        "Content-Type: text/plain\r\nCache-Control: no-store\r\n",
        |_, _| "ok".to_owned(),
    )
    .await;
    let base = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(base.as_str(), &loader);
    execute_with_source(
        &mut vm,
        r#"
      globalThis.sharedHandler = () => { const x = new XMLHttpRequest(); x.open('GET', 'handler'); x.send(); };
    "#,
        &base.join("library.js").unwrap(),
        &base,
    );
    let first = base.join("first.js").unwrap();
    let second = base.join("second.js").unwrap();
    let caller = base.join("caller.js").unwrap();
    execute_with_source(&mut vm, "document.onload = sharedHandler;", &first, &base);
    dispatch_native_document_probe(&mut vm, "load");
    execute_with_source(&mut vm, "document.onload = sharedHandler;", &second, &base);
    dispatch_native_document_probe(&mut vm, "load");
    execute_with_source(
        &mut vm,
        "document.dispatchEvent(new Event('load'));",
        &caller,
        &base,
    );
    execute_with_source(
        &mut vm,
        "document.onload = sharedHandler;",
        &Url::parse("https://other.invalid/setter.js").unwrap(),
        &base,
    );
    dispatch_native_document_probe(&mut vm, "load");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(performance.getEntriesByType('resource').length)",
        "4",
        "callback initiator completion",
    )
    .await;
    assert_eq!(vm.eval("JSON.stringify(performance.getEntriesByType('resource').map(entry => entry.initiatorUrl).sort())").unwrap(), serde_json::json!(["", caller.as_str(), first.as_str(), second.as_str()]).to_string());
    assert_eq!(server.finish().await.len(), 4);
}
