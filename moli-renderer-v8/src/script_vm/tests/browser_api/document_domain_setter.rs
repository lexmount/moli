use super::*;

#[tokio::test(flavor = "current_thread")]
async fn document_domain_setter_rejects_added_trailing_dots() {
    const HOST: &str = "www.sub.example.test";
    let mut failures = Vec::new();
    for kind in [
        "main",
        "blank",
        "srcdoc",
        "iframe",
        "nested-blank",
        "nested-srcdoc",
        "popup",
        "blank-popup",
    ] {
        let network = matches!(kind, "iframe" | "nested-blank" | "nested-srcdoc" | "popup");
        let child = "<!doctype html><body>child<script>\
            if (opener) opener.postMessage('domain-setter-ready', '*');</script>";
        let server =
            StaticHttpServer::spawn_with_bodies(vec![child.to_owned(); usize::from(network)]).await;
        let loader = static_http_loader([server.resolve_entry(HOST)]);
        let mut vm = new_parsed_page_task_executor_test_vm(
            server.url_for_host(HOST, "/entry.html").as_str(),
            "<!doctype html><body>parent",
            &loader,
        );
        let script = include_str!("../../../../tests/fixtures/document-domain-setter.js");
        vm.exec(
            &format!(
                r#"
globalThis.__domainSetterResult = null;
({script})({{kind:{kind:?},sourceURL:{source_url:?}}}).then(
  result => {{__domainSetterResult = result}},
  error => {{__domainSetterResult = {{error:String(error)}}}}
);
"#,
                source_url = server.url_for_host(HOST, "/child.html").as_str(),
            ),
            None,
        )
        .unwrap();
        advance_page_task_executor_until_eval_equals(
            &mut vm,
            &loader,
            "String(__domainSetterResult !== null)",
            "true",
            "document.domain setter checks should finish",
        )
        .await;
        let result: serde_json::Value =
            serde_json::from_str(&vm.eval("JSON.stringify(__domainSetterResult)").unwrap())
                .unwrap();
        let expected_checks = if network { 33 } else { 25 };
        if result["checks"] != expected_checks || result["failures"] != serde_json::json!([]) {
            failures.push((kind, result));
        }
        assert_eq!(
            server.finish_targets().await,
            if network { vec!["/child.html"] } else { vec![] },
            "{kind}"
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn document_domain_preserves_trailing_dots_in_url_hosts_and_assignments() {
    for (host, value, accepted, expected) in [
        (
            "www.example.test",
            "www.example.test.",
            false,
            "www.example.test",
        ),
        (
            "www.example.test.",
            "www.example.test",
            false,
            "www.example.test.",
        ),
        (
            "www.example.test.",
            "WWW.EXAMPLE.TEST.",
            true,
            "www.example.test.",
        ),
        ("www.example.test.", "example.test.", true, "example.test."),
        (
            "www.example.test.",
            "example.test",
            false,
            "www.example.test.",
        ),
        (
            "www.example.test.",
            "example.test%2e",
            true,
            "example.test.",
        ),
        (
            "www.example.test.",
            "example.test\u{3002}",
            true,
            "example.test.",
        ),
        ("www.example.test.", "test.", false, "www.example.test."),
        ("www.example.co.uk.", "co.uk.", false, "www.example.co.uk."),
        (
            "www.example.co.uk.",
            "example.co.uk.",
            true,
            "example.co.uk.",
        ),
        (
            "www.foo.kawasaki.jp.",
            "kawasaki.jp.",
            false,
            "www.foo.kawasaki.jp.",
        ),
        (
            "www.city.kawasaki.jp.",
            "city.kawasaki.jp.",
            true,
            "city.kawasaki.jp.",
        ),
    ] {
        let mut vm = new_storage_test_vm(&format!("https://{host}/page"));
        let value = serde_json::to_string(value).unwrap();
        let actual: serde_json::Value = serde_json::from_str(
            &vm.eval(&format!(
                r#"(() => {{
                    const before = document.domain;
                    let outcome = "accepted";
                    try {{ document.domain = {value}; }}
                    catch (error) {{
                        outcome = `${{error.name}}:${{error.code}}:${{error instanceof DOMException}}`;
                    }}
                    return JSON.stringify([before, outcome, document.domain]);
                }})()"#
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::json!([
                host,
                if accepted {
                    "accepted"
                } else {
                    "SecurityError:18:true"
                },
                expected,
            ]),
            "{host} assigning {value}"
        );
    }
}
