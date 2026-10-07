use super::*;

#[test]
fn computed_style_live_declarations_track_document_flat_tree_membership() {
    let mut vm = new_storage_page_task_executor_test_vm("https://computed-style-flat-tree.test/");
    vm.eval(include_str!("cssom_flat_tree.js"))
        .expect("document flat-tree transitions should evaluate");
    assert_eq!(vm.eval("__cssomFlatTreeResults.complete").unwrap(), "true");
    assert_eq!(
        vm.eval("JSON.stringify(__cssomFlatTreeResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__cssomFlatTreeResults.total === 864 && __cssomFlatTreeResults.passed === 864")
            .unwrap(),
        "true"
    );
}

#[test]
fn inspector_computed_style_tracks_child_flat_tree_membership() {
    let mut vm = new_storage_page_task_executor_test_vm("https://inspector-flat-tree.test/");
    vm.eval(
        r#"
const frame = document.createElement('iframe');
document.body.appendChild(frame);
const childDocument = frame.contentDocument;
globalThis.__inspectorFlatTarget = childDocument.createElement('div');
__inspectorFlatTarget.id = 'inspector-flat-target';
__inspectorFlatTarget.style.color = 'rgb(1, 2, 3)';
globalThis.__inspectorFlatHost = childDocument.createElement('section');
childDocument.body.appendChild(__inspectorFlatHost);
__inspectorFlatHost.attachShadow({mode: 'open'});
"#,
    )
    .expect("child inspector fixture should initialize");
    let target = element_handle_by_id(&vm, "inspector-flat-target");

    for (mutation, active) in [
        ("void 0", false),
        (
            "__inspectorFlatHost.ownerDocument.body.appendChild(__inspectorFlatTarget)",
            true,
        ),
        (
            "__inspectorFlatHost.appendChild(__inspectorFlatTarget)",
            false,
        ),
        (
            "__inspectorFlatHost.shadowRoot.appendChild(__inspectorFlatHost.ownerDocument.createElement('slot'))",
            true,
        ),
        ("__inspectorFlatTarget.remove()", false),
    ] {
        vm.eval(mutation)
            .expect("flat-tree mutation should succeed");
        let properties = vm
            .computed_style_properties_for_inspector_handle(target)
            .expect("an existing child element must keep its inspector identity");
        if active {
            assert!(
                properties
                    .iter()
                    .any(|(name, value)| name == "color" && value == "rgb(1, 2, 3)"),
                "computed values were missing after {mutation}"
            );
        } else {
            assert!(
                properties.is_empty(),
                "inactive element exposed computed values after {mutation}"
            );
        }
    }
}
