use super::*;

#[test]
fn retained_child_window_survives_context_retirement() {
    for materialized in [false, true] {
        for reinserted in [false, true] {
            let mut vm = new_storage_test_vm("https://retained-child-window.test/");
            vm.eval(
                r#"
document.appendChild(document.createElement('html'));
document.documentElement.appendChild(document.createElement('body'));
globalThis.frame = document.createElement('iframe');
document.body.append(frame);
globalThis.heldWindow = frame.contentWindow;
globalThis.heldDocument = heldWindow.document;
globalThis.heldEvent = heldWindow.Event;
heldWindow.marker = {retained: true};
globalThis.heldMarker = heldWindow.marker;
"#,
            )
            .unwrap();
            assert_eq!(vm.prebootstrapped_child_default_contexts.borrow().len(), 1);
            if materialized {
                materialize_single_child_default_realm_for_test(&mut vm, "retained Window");
                assert_eq!(vm.child_frame_realm_store.len(), 1);
            }

            vm.eval(if reinserted {
                "frame.remove(); document.body.append(frame); globalThis.replacement = frame.contentWindow;"
            } else {
                "frame.remove();"
            })
            .unwrap();
            vm.prune_stale_child_default_execution_contexts();
            assert_eq!(vm.child_frame_realm_store.len(), 0);
            assert_eq!(
                vm._context_host
                    .borrow()
                    .window_execution_context_registry_counts_for_test(),
                if reinserted { (2, 2) } else { (1, 1) },
                "retirement must release the old realm registration"
            );
            let observed = vm.eval(
                r#"JSON.stringify([
heldWindow.document === heldDocument,
heldWindow.Event === heldEvent,
heldWindow.marker === heldMarker,
heldWindow.self === heldWindow,
typeof heldWindow.addEventListener === 'function',
heldDocument.open() === heldDocument,
heldWindow.document === heldDocument,
typeof replacement === 'undefined' || replacement !== heldWindow,
typeof replacement === 'undefined' || replacement.document !== heldDocument
])"#,
            );
            assert_eq!(
                observed.unwrap_or_else(|error| panic!(
                    "materialized={materialized}, reinserted={reinserted}: {error}"
                )),
                "[true,true,true,true,true,true,true,true,true]"
            );
        }
    }
}
