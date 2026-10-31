use super::*;

#[test]
fn geometry_point_transforms_preserve_2d_coordinates_and_use_full_3d_arithmetic() {
    let mut vm = new_storage_page_task_executor_test_vm("https://geometry-point-transform.test/");
    vm.eval("document.body.innerHTML = '<iframe id=child></iframe>'")
        .unwrap();
    let source = format!(
        r#"(() => {{
            const probe = {};
            globalThis.pointTransformChecks = [
                probe(globalThis),
                probe(document.getElementById('child').contentWindow)
            ];
            return pointTransformChecks.map(result => result.total).join(',');
        }})()"#,
        include_str!("geometry_point_transform.js"),
    );
    assert_eq!(vm.eval(&source).unwrap(), "88,88");
    assert_eq!(
        vm.eval("JSON.stringify(pointTransformChecks.flatMap(result => result.failures))")
            .unwrap(),
        "[]"
    );
}
