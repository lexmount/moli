use super::*;

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_child_navigation_does_not_dispatch_a_destination_navigate_event() {
    let server = StaticHttpServer::spawn_with_responder(
        12,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| {
            r#"<!doctype html><script>
navigation.onnavigate = event => {
  event.preventDefault();
  top.postMessage('UNEXPECTED NAVIGATE', '*');
};
onload = () => {
  if (location.search === '?next') top.postMessage('DONE', '*');
};
</script>"#
                .to_owned()
        },
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.cross-navigation.test"),
        server.resolve_entry("child.cross-navigation.test"),
    ]);
    let parent = server.url_for_host("parent.cross-navigation.test", "/parent");
    let child = server.url_for_host("child.cross-navigation.test", "/child");
    let destination = server.url_for_host("child.cross-navigation.test", "/child?next");
    for action in ["href", "window", "click", "open"] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
        vm.eval(&format!(
            r#"
globalThis.loads = 0;
globalThis.messages = [];
globalThis.sourceNavigations = 0;
navigation.onnavigate = () => ++sourceNavigations;
addEventListener('message', event => messages.push(event.data));
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.name = 'target';
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
            child = child.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "cross-origin child should finish its initial load",
        )
        .await;
        let url = format!("{:?}", destination.as_str());
        let script = match action {
            "href" => format!("frame.contentWindow.location.href = {url}; 'queued'"),
            "window" => format!("frame.contentWindow.location = {url}; 'queued'"),
            "open" => format!("window.open({url}, 'target'); 'queued'"),
            "click" => format!(
                "const link = document.createElement('a'); link.href = {url}; \
                 link.target = 'target'; root.appendChild(link); link.click(); 'queued'"
            ),
            _ => unreachable!(),
        };
        vm.eval(&script).unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(messages.includes('DONE'))",
            "true",
            "foreign navigation must commit without firing its canceling navigate handler",
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify([messages, sourceNavigations, loads])")
                .unwrap(),
            r#"[["DONE"],0,2]"#,
            "{action}",
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cross_origin_location_navigation_preserves_push_and_filters_previous_origin_entries() {
    let server = StaticHttpServer::spawn_with_responder(
        4,
        "Content-Type: text/html; charset=utf-8\r\n",
        |_, _| "<!doctype html><p>ready</p>".to_owned(),
    )
    .await;
    let loader = static_http_loader([
        server.resolve_entry("parent.cross-history.test"),
        server.resolve_entry("child.cross-history.test"),
    ]);
    let parent = server.url_for_host("parent.cross-history.test", "/parent");
    let child = server.url_for_host("child.cross-history.test", "/child");
    let destination = server.url_for_host("parent.cross-history.test", "/returned");
    for target in [destination.as_str(), "about:blank"] {
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(parent.as_str(), &loader);
        vm.eval(&format!(
            r#"
globalThis.loads = 0;
const root = document.documentElement || document.appendChild(document.createElement('html'));
const frame = document.createElement('iframe');
frame.onload = () => ++loads;
frame.src = {child:?};
root.appendChild(frame);
"#,
            child = child.as_str(),
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "1",
            "foreign child should commit before navigation back to its parent's origin",
        )
        .await;
        vm.eval(&format!(
            "frame.contentWindow.location = {target:?}; 'queued'"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(loads)",
            "2",
            "new child document should commit",
        )
        .await;
        assert_eq!(
            vm.eval("String(frame.contentWindow.navigation.entries().length)")
                .unwrap(),
            "1",
            "entries from the previous origin must not leak into {target}",
        );
        if target != "about:blank" {
            assert_eq!(
                vm.eval("frame.contentWindow.navigation.activation.navigationType")
                    .unwrap(),
                "push",
                "a loaded foreign document is not an initial empty document",
            );
        }
    }
}
