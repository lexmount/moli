use super::*;

fn inactive_document_test_vm(loaded_srcdoc: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://retired-iframe-wrapper.test/");
    vm.eval(
        r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        globalThis.testFrame = document.createElement('iframe');
        body.appendChild(testFrame);
        "#,
    )
    .unwrap();
    if loaded_srcdoc {
        vm.eval("testFrame.srcdoc = '<!doctype html><html><head></head><body></body></html>'")
            .unwrap();
        vm.drain_pending_child_frame_work_for_test();
    }
    vm
}

#[test]
fn wrappers_created_after_iframe_teardown_are_weakly_cached() {
    for loaded_srcdoc in [false, true] {
        let mut vm = inactive_document_test_vm(loaded_srcdoc);
        vm.eval(
            r#"
            globalThis.retainedDocument = testFrame.contentDocument;
            globalThis.retainedBody = retainedDocument.body;
            globalThis.retainedChildren = retainedBody.children;
            globalThis.retainedNode = retainedDocument.createElement('div');
            retainedNode.marker = {};
            globalThis.retainedMarker = retainedNode.marker;
            retainedBody.appendChild(retainedNode);
            testFrame.remove();
            "#,
        )
        .unwrap();
        vm.drain_pending_child_frame_work_for_test();
        vm.eval(
            "globalThis.weakAfterRemoval = new WeakRef(retainedDocument.createElement('section'));",
        )
        .unwrap();
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
        assert_eq!(
            vm.eval(
                r#"JSON.stringify({
                  document: retainedNode.ownerDocument === retainedDocument,
                  body: retainedDocument.body === retainedBody,
                  children: retainedBody.children === retainedChildren,
                  node: retainedChildren[0] === retainedNode,
                  expando: retainedChildren[0].marker === retainedMarker,
                  afterCollected: weakAfterRemoval.deref() === undefined
                })"#,
            )
            .unwrap(),
            r#"{"document":true,"body":true,"children":true,"node":true,"expando":true,"afterCollected":true}"#,
            "loaded_srcdoc={loaded_srcdoc}"
        );
    }
}
