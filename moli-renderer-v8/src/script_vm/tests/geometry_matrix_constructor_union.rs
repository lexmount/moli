use super::*;

#[test]
fn geometry_matrix_constructors_convert_string_or_sequence_in_the_callee_realm() {
    let mut vm = new_storage_page_task_executor_test_vm("https://geometry-matrix-union.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
            const probe = {};
            globalThis.matrixConstructorChecks = [
                probe(globalThis, false),
                probe(document.getElementById('child').contentWindow, false)
            ];
            return matrixConstructorChecks.map(result => result.total).join(',');
        }})()"#,
        include_str!("geometry_matrix_constructor_union.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "64,64");
    assert_eq!(
        vm.eval("JSON.stringify(matrixConstructorChecks.flatMap(result => result.failures))")
            .unwrap(),
        "[]"
    );
}
