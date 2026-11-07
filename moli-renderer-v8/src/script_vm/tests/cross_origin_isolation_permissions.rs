use super::*;

#[test]
fn window_isolation_applies_response_permissions_without_changing_network_policy() {
    let url = Url::parse("https://isolation-permission.test/").unwrap();
    for (permission, dip, expected) in [
        ("cross-origin-isolated=()", false, false),
        ("cross-origin-isolated=(self)", false, true),
        (
            "cross-origin-isolated=(\"https://other.test\")",
            false,
            false,
        ),
        ("cross-origin-isolated=*", false, true),
        ("cross-origin-isolated=()", true, true),
    ] {
        let mut vm = new_storage_test_vm(url.as_str());
        let mut headers = vec![(
            "Permissions-Policy".to_owned(),
            permission.as_bytes().to_vec(),
        )];
        if dip {
            headers.push((
                "Document-Isolation-Policy".to_owned(),
                b"isolate-and-require-corp".to_vec(),
            ));
        } else {
            headers.push((
                "Cross-Origin-Opener-Policy".to_owned(),
                b"same-origin".to_vec(),
            ));
            headers.push((
                "Cross-Origin-Embedder-Policy".to_owned(),
                b"require-corp".to_vec(),
            ));
        }
        let policy =
            crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
                &headers, &url,
            );
        assert!(policy.cross_origin_isolated);
        vm.set_main_navigation_policy_container(policy);
        assert_eq!(
            vm.eval("String(crossOriginIsolated)").unwrap(),
            expected.to_string(),
            "{permission}, DIP={dip}"
        );
    }
}
