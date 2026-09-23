use super::*;

#[tokio::test]
async fn history_traversal_canceled_precommit_callback_cannot_complete_successor() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);
    vm.eval(r#"
        history.replaceState(0, ''); history.pushState(1, '');
        globalThis.gates = []; globalThis.log = [];
        navigation.onnavigate = event => {
            if (event.navigationType === 'traverse') {
                event.intercept({precommitHandler: () => new Promise(resolve => gates.push(resolve))});
            }
        };
        const first = navigation.back();
        first.committed.catch(error => log.push('oldCommitted:' + error.name));
        first.finished.catch(error => log.push('oldFinished:' + error.name));
    "#).unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        1
    );
    vm.eval(
        r#"
        stop();
        const second = navigation.back();
        second.committed.then(() => log.push('newCommitted'));
        second.finished.then(() => log.push('newFinished'));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        0
    );
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        1
    );
    vm.eval("gates[0]();").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([history.state, log])").unwrap(),
        r#"[1,["oldCommitted:AbortError","oldFinished:AbortError"]]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        1
    );
    vm.eval("gates[1]();").unwrap();
    assert_eq!(
        vm.eval("JSON.stringify([history.state, log])").unwrap(),
        r#"[0,["oldCommitted:AbortError","oldFinished:AbortError","newCommitted","newFinished"]]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        0
    );
}

#[tokio::test]
async fn history_traversal_detach_releases_pending_admission_and_ignores_late_callback() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader("https://example.com/base", &loader);
    vm.eval(
        r#"
        globalThis.frame = document.createElement('iframe');
        (document.body || document.documentElement || document).appendChild(frame);
        globalThis.child = frame.contentWindow;
    "#,
    )
    .unwrap();
    assert_eq!(vm.eval("child.document.readyState").unwrap(), "complete");
    vm.eval(
        r#"
        globalThis.log = [];
        child.history.replaceState(0, ''); history.replaceState(0, '');
        history.pushState(1, ''); child.history.pushState(1, '');
        navigation.onnavigate = event => event.intercept({
            precommitHandler: () => new Promise(resolve => globalThis.release = resolve)
        });
        const result = navigation.back();
        result.committed.catch(error => log.push('committed:' + error.name));
        result.finished.catch(error => log.push('finished:' + error.name));
    "#,
    )
    .unwrap();
    assert!(
        vm.run_one_history_traversal_executor_turn(&loader)
            .await
            .unwrap()
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        1
    );
    vm.eval("frame.remove();").unwrap();
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        0
    );
    assert_eq!(
        vm.eval("JSON.stringify(log)").unwrap(),
        r#"["committed:AbortError","finished:AbortError"]"#
    );
    vm.eval("release();").unwrap();
    assert_eq!(vm.eval("history.state").unwrap(), "1");
    assert_eq!(
        vm.eval("JSON.stringify(log)").unwrap(),
        r#"["committed:AbortError","finished:AbortError"]"#
    );
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_history_traversal_admissions
            .len(),
        0
    );
}
