use super::*;

#[test]
fn processing_instruction_attributes_preserve_data_mutations_and_native_receivers() {
    let mut vm = new_storage_page_task_executor_test_vm("https://processing-instruction.test/");
    assert_eq!(
        vm.eval(include_str!("processing_instruction_attributes.js"))
            .unwrap(),
        "true"
    );
}
