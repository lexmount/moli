use super::*;

#[test]
fn document_domain_isolation_preserves_validation_without_mutating_origin() {
    for (headers, exposed, ignores_mutation) in [
        (vec![], false, false),
        (
            vec![("Cross-Origin-Embedder-Policy", "require-corp")],
            false,
            false,
        ),
        (
            vec![
                ("Cross-Origin-Opener-Policy", "same-origin"),
                ("Cross-Origin-Embedder-Policy", "require-corp"),
            ],
            true,
            true,
        ),
        (
            vec![
                ("Cross-Origin-Opener-Policy", "same-origin"),
                ("Cross-Origin-Embedder-Policy", "credentialless"),
                ("Permissions-Policy", "cross-origin-isolated=()"),
            ],
            false,
            true,
        ),
        (
            vec![("Document-Isolation-Policy", "isolate-and-require-corp")],
            true,
            true,
        ),
        (
            vec![("Document-Isolation-Policy", "isolate-and-credentialless")],
            true,
            true,
        ),
    ] {
        let document_url = Url::parse("https://www1.web-platform.test/page.html").unwrap();
        let mut vm = new_storage_test_vm(document_url.as_str());
        let headers = headers
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        vm.set_main_navigation_policy_container(
            crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                &headers,
                &document_url,
            ),
        );
        assert_eq!(
            vm.eval("String(crossOriginIsolated)").unwrap(),
            exposed.to_string(),
            "{headers:?}"
        );
        assert_eq!(
            vm.eval(
                r#"(() => {
  const failures = [];
  for (const value of ['', 'not-web-platform.test', 'test', '.web-platform.test']) {
    try { document.domain = value; failures.push('accepted'); }
    catch (error) { failures.push(error.name); }
  }
  const marker = {};
  try { document.domain = {toString() { throw marker; }}; }
  catch (error) { failures.push(error === marker ? 'marker' : error.name); }
  for (const target of [new Document(), document.implementation.createHTMLDocument('')]) {
    try { target.domain = 'web-platform.test'; failures.push('accepted'); }
    catch (error) { failures.push(error.name); }
  }
  return failures.join('|');
})()"#,
            )
            .unwrap(),
            "SecurityError|SecurityError|SecurityError|SecurityError|marker|SecurityError|SecurityError",
            "{headers:?}"
        );
        // A same-host assignment must also be ignored: recording it would mark
        // the origin as domain-relaxed even though the getter remains unchanged.
        vm.eval("document.domain = location.hostname").unwrap();
        assert_eq!(
            vm._context_host
                .borrow()
                .inherited_document_domain(None)
                .borrow()
                .is_none(),
            ignores_mutation,
            "{headers:?}"
        );
        assert_eq!(
            vm.eval(
                r#"(() => {
  Object.defineProperty(globalThis, 'crossOriginIsolated', {value: !crossOriginIsolated});
  let conversions = 0;
  document.domain = {toString() { conversions++; return 'web-platform.test'; }};
  return [document.domain, conversions].join('|');
})()"#,
            )
            .unwrap(),
            if ignores_mutation {
                "www1.web-platform.test|1"
            } else {
                "web-platform.test|1"
            },
            "{headers:?}"
        );
    }
}
