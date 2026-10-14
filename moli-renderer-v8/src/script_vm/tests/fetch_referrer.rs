use super::*;

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
