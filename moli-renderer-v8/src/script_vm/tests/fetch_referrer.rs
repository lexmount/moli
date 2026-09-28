use super::*;

#[tokio::test]
async fn request_referrer_validation_uses_the_relevant_realm_origin() {
    let loader = static_http_loader([]);
    let mut vm = new_page_task_executor_test_vm_with_loader(
        "https://request-origin.test/page.html",
        &loader,
    );
    assert_eq!(
        vm.eval(
            r#"(() => {
                const own = 'https://request-origin.test/referrer';
                const foreign = 'https://base-origin.test/referrer';
                const frame = document.createElement('iframe'); document.body.append(frame);
                const child = frame.contentWindow;
                for (const realm of [window, child]) {
                    const base = realm.document.createElement('base');
                    base.href = 'https://base-origin.test/path/'; realm.document.head.append(base);
                    for (const [referrer, expected] of [
                        [own, own], [foreign, 'about:client'], ['relative', 'about:client'],
                        ['', ''], ['about:client', 'about:client']
                    ]) {
                        const actual = new realm.Request(own, {referrer}).referrer;
                        if (actual !== expected) throw new Error(referrer + ': ' + actual);
                    }
                }
                const RequestFromRetiredRealm = child.Request;
                frame.remove(); document.body.append(frame);
                if (new RequestFromRetiredRealm(own, {referrer:own}).referrer !== own)
                    throw new Error('retired constructor lost its settings origin');
                if (new RequestFromRetiredRealm(own, {referrer:foreign}).referrer !== 'about:client')
                    throw new Error('retired constructor accepted a foreign origin');
                return 'pass';
            })()"#,
        )
        .unwrap(),
        "pass"
    );
}

#[tokio::test]
async fn window_fetch_referrer_inputs_reach_the_network() {
    let server = StaticHttpServer::spawn_with_bodies(vec!["ok".to_owned(); 14]).await;
    let base = server.base_url().origin().ascii_serialization();
    let page_url = format!("{base}/context/page.html?base=1#fragment");
    let loader = static_http_loader([]);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(&page_url, &loader);
    vm.eval(&format!(
        "{}\nglobalThis.referrerResult = 'pending'; fetchReferrerInputs('{base}').then(value => {{ referrerResult = value; }}, error => {{ referrerResult = String(error); }});",
        include_str!("../../../tests/fixtures/fetch-referrer-inputs.js"),
    )).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(referrerResult !== 'pending')",
        "true",
        "fetch referrer requests should finish",
    )
    .await;
    assert_eq!(vm.eval("referrerResult").unwrap(), "pass");
    let requests = server.finish().await;
    for (index, (request, expected)) in requests
        .iter()
        .zip([
            Some("/selected?q=1"),
            Some("/request"),
            Some("/clone"),
            Some("/override"),
            None,
            None,
            Some("/context/page.html?base=1"),
            Some("/context/page.html?base=1"),
            Some("/context/relative?q=1"),
            Some("/"),
            Some("/"),
            None,
            Some("/context/page.html?base=1"),
            Some("/context/page.html?base=1"),
        ])
        .enumerate()
    {
        assert_eq!(request.target, format!("/echo?case={index}"));
        assert_eq!(
            request.header_value("referer"),
            expected.map(|path| format!("{base}{path}")).as_deref(),
            "case {index}"
        );
        assert_eq!(request.header_value("sec-fetch-site"), Some("same-origin"));
    }
}
