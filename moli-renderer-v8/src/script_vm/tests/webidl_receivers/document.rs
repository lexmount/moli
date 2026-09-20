use super::*;

#[test]
fn document_members_validate_receivers_before_conversion_in_the_callee_realm() {
    run_document_member_probe("documentMemberReceiverProbe");
}

#[test]
fn document_members_preserve_genuine_receivers_and_collection_contents() {
    run_document_member_probe("documentGenuineReceiverProbe");
}

fn run_document_member_probe(probe: &str) {
    let mut vm = new_parsed_test_vm(
        "https://document-member-receivers.test/",
        "<!doctype html><body><iframe id='document-member-child'></iframe></body>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "Document member receiver realm");
    let fixture = include_str!("../../../../tests/fixtures/document-member-receivers.js");
    vm.eval(&format!(
        "{fixture}\n{probe}().then(result => {{ globalThis.documentMemberResult = result; }}, error => {{ globalThis.documentMemberResult = String(error.stack || error); }});"
    )).expect("Document member receiver probe should run");
    assert_eq!(
        vm.eval("JSON.stringify(documentMemberResult)").unwrap(),
        "[]"
    );
}
