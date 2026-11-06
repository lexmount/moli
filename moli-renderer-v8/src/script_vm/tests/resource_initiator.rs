use super::*;
use crate::script_provenance::CompiledStringProvenance;

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
