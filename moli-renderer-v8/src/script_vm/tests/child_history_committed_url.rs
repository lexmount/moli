use super::*;

#[tokio::test]
async fn child_history_commits_document_url_and_base_before_entry_events() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><base href='/assets/'><p>child</p>".to_owned(),
    ])
    .await;
    let base = server.base_url().origin().ascii_serialization();
    let loader = static_http_loader([]);
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(&format!("{base}/top"), &loader);
    vm.eval(
        "globalThis.frame = document.createElement('iframe'); \
         frame.onload = () => { globalThis.frameLoaded = true; }; \
         frame.src = '/original'; document.body.append(frame);",
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.frameLoaded)",
        "true",
        "child document load",
    )
    .await;
    let result = vm
        .eval(
            r#"(() => {
                const child = frame.contentWindow;
                let observed;
                child.navigation.addEventListener('currententrychange', () => {
                    observed = [child.document.URL, child.document.baseURI].join('|');
                }, {once: true});
                child.history.replaceState(null, '', '/committed?q=1');
                return observed;
            })()"#,
        )
        .unwrap();
    assert_eq!(result, format!("{base}/committed?q=1|{base}/assets/"));
    let host = vm._context_host.borrow();
    let child = host.child_browsing_context_handles_in_document_order()[0];
    assert_eq!(
        host.child_browsing_context_current_url(child)
            .unwrap()
            .as_str(),
        format!("{base}/committed?q=1")
    );
}
