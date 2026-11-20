use super::*;

#[tokio::test]
async fn window_open_referrer_applies_policy_when_reusing_a_named_popup() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>opener.postMessage(document.referrer, '*')</script>".to_owned();
        2
    ])
    .await;
    let base = server.base_url().origin().ascii_serialization();
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        &format!("{base}/source/page.html?query#fragment"),
        "<!doctype html><body>source",
        &loader,
    );
    vm.eval(
        "globalThis.referrers = []; onmessage = e => referrers.push(e.data); \
         globalThis.target = open('/first', 'referrer-target');",
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(referrers.length)",
        "1",
        "initial popup navigation",
    )
    .await;
    assert_eq!(
        vm.eval("referrers[0]").unwrap(),
        format!("{base}/source/page.html?query")
    );
    vm.set_response_referrer_policy(Some("origin".to_owned()));
    assert_eq!(
        vm.eval("String(open('/second', 'referrer-target') === target)")
            .unwrap(),
        "true"
    );
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(referrers.length)",
        "2",
        "named popup navigation with updated policy",
    )
    .await;
    assert_eq!(vm.eval("referrers[1]").unwrap(), format!("{base}/"));
    assert_eq!(server.finish_targets().await, ["/first", "/second"]);
    vm.eval("target.close()").unwrap();
}

#[test]
fn window_open_referrer_for_initial_blank_does_not_apply_navigation_policy() {
    let mut vm = new_storage_test_vm("https://blank-referrer.test/page?query#fragment");
    for policy in ["no-referrer", "origin", "unsafe-url"] {
        vm.set_response_referrer_policy(Some(policy.to_owned()));
        assert_eq!(
            vm.eval(
                r#"JSON.stringify(['', 'about:blank', 'about:blank#fragment'].map(url => {
                    const popup = open(url);
                    const result = popup.document.referrer;
                    popup.close();
                    return result;
                }))"#,
            )
            .unwrap(),
            r#"["https://blank-referrer.test/page?query#fragment","https://blank-referrer.test/page?query#fragment","https://blank-referrer.test/page?query#fragment"]"#,
            "{policy}"
        );
    }
}
