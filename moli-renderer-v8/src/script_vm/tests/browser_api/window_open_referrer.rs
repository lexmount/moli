use super::*;

#[tokio::test]
async fn window_open_referrer_uses_entry_document_and_policy_across_realms() {
    for entry_kind in ["child", "popup"] {
        let entry = r#"<!doctype html><base href="/entry/base/">
            <script>
            window.scheduleReferrerCases = callback => Promise.resolve().then(() => callback());
            </script>
            <body onload="(opener || top).__entryLoaded(window)">
            <iframe srcdoc="<base href='/incumbent/'>"></iframe>
            <iframe srcdoc="<base href='/relevant/'>"></iframe>"#;
        let popup = r#"<!doctype html><script>
            new BroadcastChannel('window-open-referrer').postMessage({
                label: new URL(location).searchParams.get('label'),
                href: location.href,
                referrer: document.referrer,
                hasOpener: opener !== null
            });
            window.close();
            </script>"#;
        let mut bodies = vec![entry.to_owned()];
        bodies.extend(std::iter::repeat_n(popup.to_owned(), 12));
        let server = StaticHttpServer::spawn_with_bodies(bodies).await;
        let base = server.base_url().origin().ascii_serialization();
        let loader = static_http_loader([]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            &format!("{base}/top/page.html"),
            "<!doctype html><body>top",
            &loader,
        );
        // The top-level policy must not override the entry Document's policy.
        vm.set_response_referrer_policy(Some("no-referrer".to_owned()));
        vm.eval(include_str!(
            "../../../../tests/fixtures/window-open-referrer.js"
        ))
        .unwrap();
        vm.eval(&format!(
            "startWindowOpenReferrerTest({base:?}, {entry_kind:?})"
        ))
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(windowOpenReferrerResults.size)",
            "12",
            &format!("cross-realm window.open referrers from {entry_kind}"),
        )
        .await;
        assert_eq!(
            vm.eval("JSON.stringify(windowOpenReferrerFailures())")
                .unwrap(),
            "[]",
            "{entry_kind} entry"
        );
        assert_eq!(server.finish().await.len(), 13);
        vm.eval("windowOpenReferrerCleanup()").unwrap();
    }
}

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

#[tokio::test]
async fn window_open_referrer_filters_cross_origin_destinations() {
    const SOURCE: &str = "http://window-open-referrer.test/page?query#fragment";
    const FULL: &str = "http://window-open-referrer.test/page?query";
    const ORIGIN: &str = "http://window-open-referrer.test/";
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>opener.postMessage(document.referrer, '*'); close()</script>"
            .to_owned();
        5
    ])
    .await;
    let loader = static_http_loader([]);
    let mut vm =
        new_parsed_page_task_executor_test_vm(SOURCE, "<!doctype html><body>source", &loader);
    vm.eval("globalThis.referrers = []; onmessage = e => referrers.push(e.data)")
        .unwrap();
    for (index, (policy, expected)) in [
        (None, ORIGIN),
        (Some("unsafe-url"), FULL),
        (Some("no-referrer"), ""),
        (Some("same-origin"), ""),
        (Some("origin"), ORIGIN),
    ]
    .into_iter()
    .enumerate()
    {
        vm.set_response_referrer_policy(policy.map(str::to_owned));
        vm.eval(&format!("open({:?})", server.base_url().as_str()))
            .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(referrers.length)",
            &(index + 1).to_string(),
            "cross-origin popup referrer",
        )
        .await;
        assert_eq!(vm.eval("referrers.at(-1)").unwrap(), expected, "{policy:?}");
    }
    assert_eq!(server.finish().await.len(), 5);
}

#[tokio::test]
async fn window_open_referrer_follows_srcdoc_containers_but_keeps_entry_policy() {
    for depth in [1, 2] {
        for policy in ["unsafe-url", "no-referrer"] {
            let server = StaticHttpServer::spawn_with_bodies(vec![
                "<!doctype html><script>opener.top.postMessage(document.referrer, '*'); close()</script>"
                    .to_owned(),
            ])
            .await;
            let loader = static_http_loader([]);
            let mut vm = new_parsed_page_task_executor_test_vm(
                "http://srcdoc-referrer.test/source?query#fragment",
                "<!doctype html><base href='http://different-base.test/'><body>source",
                &loader,
            );
            vm.set_response_referrer_policy(Some("origin".to_owned()));
            let popup_url = server.base_url().to_string();
            let mut markup = format!(
                "<!doctype html><meta name='referrer' content='{policy}'><script>open({popup_url:?})</script>"
            );
            if depth == 2 {
                let child_markup = serde_json::to_string(&markup)
                    .unwrap()
                    .replace('<', "\\u003c");
                markup = format!(
                    "<!doctype html><body><script>const f = document.createElement('iframe'); \
                     f.srcdoc = {child_markup}; document.body.append(f)</script>"
                );
            }
            vm.eval(&format!(
                "globalThis.referrers = []; onmessage = e => referrers.push(e.data); \
                 const frame = document.createElement('iframe'); frame.srcdoc = {}; \
                 document.body.append(frame);",
                serde_json::to_string(&markup).unwrap()
            ))
            .unwrap();
            advance_page_task_executor_until_eval_equals(
                &mut vm,
                &loader,
                "String(referrers.length)",
                "1",
                "srcdoc popup referrer",
            )
            .await;
            assert_eq!(
                vm.eval("referrers[0]").unwrap(),
                if policy == "no-referrer" {
                    ""
                } else {
                    "http://srcdoc-referrer.test/source?query"
                },
                "depth={depth}, policy={policy}"
            );
            assert_eq!(server.finish().await.len(), 1);
        }
    }
}

#[tokio::test]
async fn window_open_referrer_is_empty_for_an_opaque_entry_document() {
    let server = StaticHttpServer::spawn_with_bodies(vec![
        "<!doctype html><script>open('/popup')</script>".to_owned(),
        "<!doctype html><script>opener.top.postMessage(document.referrer, '*'); close()</script>"
            .to_owned(),
    ])
    .await;
    let loader = static_http_loader([]);
    let mut vm = new_parsed_page_task_executor_test_vm(
        server.base_url().as_str(),
        "<!doctype html><body>source",
        &loader,
    );
    vm.eval(
        "globalThis.referrers = []; onmessage = e => referrers.push(e.data); \
         const frame = document.createElement('iframe'); \
         frame.sandbox = 'allow-scripts allow-popups allow-popups-to-escape-sandbox'; \
         frame.src = '/opaque'; document.body.append(frame);",
    )
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(referrers.length)",
        "1",
        "opaque entry popup referrer",
    )
    .await;
    assert_eq!(vm.eval("referrers[0]").unwrap(), "");
    assert_eq!(server.finish_targets().await, ["/opaque", "/popup"]);
}
