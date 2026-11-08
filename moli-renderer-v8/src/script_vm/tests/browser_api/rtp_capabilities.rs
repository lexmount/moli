use super::*;

#[test]
fn rtp_capabilities_use_webidl_conversion_and_fresh_callee_realm_dictionaries() {
    let mut vm = new_parsed_test_vm(
        "https://rtp-capabilities.test/",
        "<!doctype html><iframe id=child></iframe>",
    );
    let result = vm.eval(include_str!("rtp_capabilities.js")).expect(
        "RTP capabilities should preserve WebIDL conversion, realms and dictionary isolation",
    );
    assert_eq!(result, "true");
}
