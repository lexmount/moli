use super::*;

async fn load_siblings(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) {
    vm.eval(
        r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__removed = document.createElement('div');
document.body.append(__removed);
globalThis.__frames = [document.createElement('iframe'), document.createElement('iframe')];
globalThis.__loaded = 0;
for (const [index, frame] of __frames.entries()) {
  frame.onload = () => ++__loaded;
  frame.src = '/child?index=' + index;
  __removed.append(frame);
}
"#,
    )
    .expect("siblings should start real child navigation");
    advance_page_task_executor_until_eval_equals(
        vm,
        loader,
        "String(__loaded)",
        "2",
        "loaded removal siblings",
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn child_removal_unload_reentry_dismisses_prepared_siblings_once() {
    for reinsert in [false, true] {
        let server = StaticHttpServer::spawn(if reinsert { 3 } else { 2 }).await;
        let loader = static_http_loader(std::iter::empty::<String>());
        let mut vm =
            new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
        load_siblings(&mut vm, &loader).await;
        vm.eval(&format!(
            r#"
globalThis.__oldWindows = __frames.map(frame => frame.contentWindow);
globalThis.__dismissalEvents = [];
for (const [index, win] of __oldWindows.entries()) {{
  win.addEventListener('beforeunload', () => __dismissalEvents.push(index + ':beforeunload'));
  win.addEventListener('pagehide', () => __dismissalEvents.push(index + ':pagehide'));
  win.addEventListener('unload', () => {{
    __dismissalEvents.push(index + ':unload');
    if (index === 0) {{
      __frames[1].remove();
      if ({reinsert}) document.body.append(__frames[1]);
    }}
  }});
}}
__removed.remove();
"#
        ))
        .expect("removal reentry must not revive or skip a retiring sibling");
        assert_eq!(
            vm.eval("JSON.stringify(__dismissalEvents)").unwrap(),
            r#"["0:pagehide","0:unload","1:pagehide","1:unload"]"#,
            "sibling dismissal with reinsert={reinsert}"
        );
        assert_eq!(
            vm.eval("String(__oldWindows.every(win => win.closed))")
                .unwrap(),
            "true"
        );
        if reinsert {
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(__loaded)",
                "3",
                "fresh sibling after unload reentry",
            )
            .await;
            assert_eq!(vm.eval("String(__frames[1].isConnected && __frames[1].contentWindow !== __oldWindows[1] && !__frames[1].contentWindow.closed)").unwrap(), "true");
        }
        let mut requests = server.finish_targets().await;
        requests.sort_unstable();
        assert_eq!(
            requests,
            if reinsert {
                vec!["/child?index=0", "/child?index=1", "/child?index=1"]
            } else {
                vec!["/child?index=0", "/child?index=1"]
            }
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn child_removal_unload_self_removal_does_not_repeat_dismissal() {
    let server = StaticHttpServer::spawn(2).await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    load_siblings(&mut vm, &loader).await;
    vm.eval(
        r#"
globalThis.__dismissalEvents = [];
for (const [index, frame] of __frames.entries()) {
  frame.contentWindow.addEventListener('unload', () => {
    __dismissalEvents.push(index);
    frame.remove();
  });
}
__removed.remove();
"#,
    )
    .expect("self removal during unload should settle once");
    assert_eq!(
        vm.eval("JSON.stringify(__dismissalEvents)").unwrap(),
        "[0,1]"
    );
    let mut requests = server.finish_targets().await;
    requests.sort_unstable();
    assert_eq!(requests, ["/child?index=0", "/child?index=1"]);
}

#[tokio::test(flavor = "current_thread")]
async fn child_removal_unload_starts_keepalive_in_retiring_realm_and_posts_to_parent() {
    let server = StaticHttpServer::spawn_with_responder(
        2,
        "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n",
        |index, _| {
            if index == 0 {
                r#"<!doctype html><body><script>
addEventListener('beforeunload', () => ++parent.__unexpectedBeforeUnload);
addEventListener('unload', () => {
  fetch('/kept', {keepalive:true, method:'POST', body:'retiring child'})
    .then(() => ++parent.__retiredPromiseCallbacks, () => ++parent.__retiredPromiseCallbacks);
  parent.postMessage('removed-child', '*');
});
</script>"#
                    .to_owned()
            } else {
                "accepted".to_owned()
            }
        },
    )
    .await;
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm =
        new_storage_page_task_executor_test_vm_with_loader(server.base_url().as_str(), &loader);
    vm.eval(
        r#"
if (!document.documentElement) document.appendChild(document.createElement('html'));
if (!document.body) document.documentElement.appendChild(document.createElement('body'));
globalThis.__unexpectedBeforeUnload = 0;
globalThis.__retiredPromiseCallbacks = 0;
globalThis.__loaded = false;
globalThis.__removedChildMessage = '';
addEventListener('message', event => { __removedChildMessage = event.data; });
globalThis.__frame = document.createElement('iframe');
__frame.onload = () => { __loaded = true; };
__frame.src = '/child';
document.body.append(__frame);
"#,
    )
    .expect("the child must install its own realm's unload handler");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__loaded)",
        "true",
        "child unload Fetch setup",
    )
    .await;
    vm.eval("__frame.remove()")
        .expect("removal should run native dismissal");
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(__removedChildMessage)",
        "removed-child",
        "unload message to live parent",
    )
    .await;
    let requests = tokio::time::timeout(std::time::Duration::from_secs(8), server.finish())
        .await
        .expect("the retiring child must send its keepalive to the server");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].target, "/kept");
    assert_eq!(requests[1].body, b"retiring child");
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while loader
            .resource_runtime_diagnostics()
            .detached_keepalive_loads
            .active_load_count
            != 0
        {
            if !vm
                .run_one_oldest_ready_page_task_executor_turn(&loader)
                .await
                .unwrap()
            {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }
    })
    .await
    .expect("detached network-only keepalive must release its native lease");
    assert_eq!(
        vm.eval("JSON.stringify([__unexpectedBeforeUnload,__retiredPromiseCallbacks])")
            .unwrap(),
        "[0,0]"
    );
}
