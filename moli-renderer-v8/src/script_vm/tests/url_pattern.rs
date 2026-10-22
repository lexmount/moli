use super::*;

#[test]
fn url_pattern_preserves_native_matching_conversion_and_realm_boundaries() {
    let mut vm = new_parsed_test_vm(
        "https://url-pattern.test/",
        "<!doctype html><html><body></body></html>",
    );
    let fixture = include_str!("../../../tests/fixtures/urlpattern-native.js");
    assert_eq!(
        vm.eval(&format!(
            "(() => {{ const child = document.body.appendChild(document.createElement('iframe')).contentWindow; const check = ({fixture}); check(globalThis); check(child); return 'ok'; }})()"
        ))
        .expect("URLPattern native semantics in Window and child realms"),
        "ok"
    );
}
