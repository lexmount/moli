use super::*;

#[test]
fn cssom_writes_validate_receivers_then_convert_before_mutating() {
    let mut vm = new_storage_page_task_executor_test_vm("https://cssom-write-conversion.test/");
    vm.eval(include_str!("cssom_write_conversion.js"))
        .expect("CSSOM write conversion matrix should evaluate");
    assert_eq!(
        vm.eval("JSON.stringify(__cssomWriteConversionResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__cssomWriteConversionResults.complete && __cssomWriteConversionResults.total === 2085 && __cssomWriteConversionResults.passed === 2085")
            .unwrap(),
        "true"
    );
}
