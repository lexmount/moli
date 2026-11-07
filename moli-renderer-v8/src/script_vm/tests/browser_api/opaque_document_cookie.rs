use super::*;

#[test]
fn network_document_cookie_rejects_opaque_origin_after_value_conversion() {
    for opaque in [false, true] {
        let mut vm = new_storage_test_vm("https://opaque-cookie.test/document");
        if opaque {
            vm.set_main_navigation_policy_container(
                crate::document_runtime::DocumentPolicyContainer {
                    response_content_security_policies: vec!["sandbox allow-scripts".into()],
                    ..Default::default()
                },
            );
        }
        let result = vm
            .eval(
                r#"JSON.stringify((() => {
            const probe = operation => {
                try { operation(); return ['allowed', true]; }
                catch (error) { return [error.name, error instanceof DOMException]; }
            };
            let converted = 0;
            const read = probe(() => document.cookie);
            const write = probe(() => {
                document.cookie = {toString() { ++converted; return 'opaque_cookie_probe=1'; }};
            });
            return {origin, read, write, converted};
        })())"#,
            )
            .unwrap();
        let outcome = if opaque { "SecurityError" } else { "allowed" };
        let expected = serde_json::json!({
            "origin":if opaque {"null"} else {"https://opaque-cookie.test"},
            "read":[outcome,true], "write":[outcome,true], "converted":1,
        });
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            expected
        );
    }
}

#[test]
fn cookie_averse_documents_do_not_throw_for_an_opaque_origin() {
    for url in ["about:blank", "data:text/html,opaque"] {
        let mut vm = new_storage_test_vm(url);
        let result = vm
            .eval(
                r#"JSON.stringify((() => {
            const detached = new DOMParser().parseFromString('<p>Detached</p>', 'text/html');
            const snapshot = document => {
                let converted = 0;
                document.cookie = {toString() { ++converted; return 'averse_cookie_probe=1'; }};
                return {value:document.cookie, converted};
            };
            return [snapshot(document), snapshot(detached)];
        })())"#,
            )
            .unwrap();
        assert_eq!(
            result,
            r#"[{"value":"","converted":1},{"value":"","converted":1}]"#
        );
    }
}

#[test]
fn inherited_blank_document_cookies_stop_after_context_removal() {
    let mut vm = new_storage_test_vm("https://inherited-cookie.test/document");
    let result = vm
        .eval(
            r#"JSON.stringify((() => {
            document.cookie = 'inherited_cookie_probe=parent; Path=/';
            const frame = document.createElement('iframe');
            (document.body || document.documentElement || document).appendChild(frame);
            const saved = frame.contentDocument;
            const inheritedRead = saved.cookie.includes('inherited_cookie_probe=parent');
            saved.cookie = 'inherited_cookie_probe=child; Path=/';
            const inheritedWrite = document.cookie.includes('inherited_cookie_probe=child');
            frame.remove();
            const detachedRead = saved.cookie;
            saved.cookie = 'inherited_cookie_probe=detached; Path=/';
            return {inheritedRead, inheritedWrite, detachedRead,
                detachedWriteIgnored:document.cookie.includes('inherited_cookie_probe=child')};
        })())"#,
        )
        .unwrap();
    assert_eq!(
        result,
        r#"{"inheritedRead":true,"inheritedWrite":true,"detachedRead":"","detachedWriteIgnored":true}"#
    );
}
