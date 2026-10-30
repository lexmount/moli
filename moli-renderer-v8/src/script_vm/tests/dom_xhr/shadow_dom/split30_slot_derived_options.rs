use super::*;
fn check_slot_assigned_options(expression: &str) {
    let mut vm = new_parsed_test_vm(
        "https://slot-assigned-options.test/",
        "<!doctype html><body><iframe></iframe></body>",
    );
    materialize_single_child_default_realm_for_test(&mut vm, "slot options callee realm");
    let source = format!(
        "{}\n{expression}",
        include_str!("../slot_assigned_options.js")
    );
    assert_eq!(
        vm.eval(&source).expect("slot options probe should pass"),
        "ok"
    );
}

#[test]
fn slot_assigned_options_dictionary_defaults_and_values() {
    check_slot_assigned_options("slotAssignedOptionsProbe('dictionary')");
}

#[test]
fn slot_assigned_options_preserve_exception_and_receiver_order() {
    check_slot_assigned_options("slotAssignedOptionsProbe('exceptions')");
}

#[test]
fn slot_assigned_options_observe_reentrant_assignment_changes() {
    check_slot_assigned_options("slotAssignedOptionsProbe('mutation')");
}
