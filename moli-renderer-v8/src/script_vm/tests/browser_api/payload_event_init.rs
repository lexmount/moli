use super::*;

#[test]
fn payload_events_derive_required_and_optional_native_payloads_before_state() {
    let mut vm = new_storage_html_test_vm("https://payload-event-dictionary.test/");
    vm.eval(&format!("globalThis.payloadFixtureResult='pending'; ({}).then(value=>payloadFixtureResult=value,error=>payloadFixtureResult=String(error.stack||error));", include_str!("../../../../tests/fixtures/payload-event-dictionary.js"))).unwrap();
    vm.eval("0").unwrap();
    assert_eq!(vm.eval("String(payloadFixtureResult)").unwrap(), "true");
}
