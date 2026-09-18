use super::*;

fn inactive_document_test_vm(loaded_srcdoc: bool) -> StandaloneScriptVmHarness {
    let mut vm = new_storage_test_vm("https://inactive-document-registry.test/");
    vm.eval(
        r#"
        const root = document.documentElement || document.appendChild(document.createElement('html'));
        const body = document.body || root.appendChild(document.createElement('body'));
        globalThis.registryFrame = document.createElement('iframe');
        body.appendChild(registryFrame);
        "#,
    )
    .unwrap();
    if loaded_srcdoc {
        vm.eval("registryFrame.srcdoc = '<!doctype html><html><head></head><body></body></html>'")
            .unwrap();
        vm.drain_pending_child_frame_work_for_test();
    }
    vm
}

#[test]
fn inactive_child_document_accepts_its_registry_without_reviving_script_execution() {
    for loaded_srcdoc in [false, true] {
        let mut vm = inactive_document_test_vm(loaded_srcdoc);
        let result = vm
            .eval(
                r#"
                (() => {
                  globalThis.retainedDocument = registryFrame.contentDocument;
                  globalThis.retainedRegistry = registryFrame.contentWindow.customElements;
                  globalThis.inactiveScriptRuns = 0;
                  registryFrame.remove();
                  const failures = [];
                  const check = (name, operation) => {
                    try { if (!operation()) failures.push(name); }
                    catch (error) { failures.push(name + ':' + error.name); }
                  };
                  for (const name of ['div', 'script', 'unregistered-element']) {
                    for (const explicit of [false, true]) {
                      const options = explicit ? {customElementRegistry: retainedRegistry} : undefined;
                      check(name + ':create:' + explicit, () =>
                        retainedDocument.createElement(name, options).ownerDocument === retainedDocument);
                      check(name + ':createNS:' + explicit, () =>
                        retainedDocument.createElementNS('http://www.w3.org/1999/xhtml', name, options)
                          .ownerDocument === retainedDocument);
                    }
                  }
                  const script = retainedDocument.createElement('script');
                  script.textContent = 'parent.inactiveScriptRuns++';
                  retainedDocument.body.appendChild(script);
                  check('script stayed inactive', () => inactiveScriptRuns === 0);
                  check('frame stayed detached', () => registryFrame.contentDocument === null);
                  return JSON.stringify(failures);
                })()
                "#,
            )
            .unwrap();
        assert_eq!(result, "[]", "loaded_srcdoc={loaded_srcdoc}");
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(vm.eval("inactiveScriptRuns").unwrap(), "0");
        assert_eq!(
            vm.eval("retainedDocument.createElement('span').ownerDocument === retainedDocument")
                .unwrap(),
            "true",
            "the retained Document must still own later-created elements"
        );
        assert_eq!(
            vm.eval("retainedDocument.createElement('span', {customElementRegistry: retainedRegistry}).ownerDocument === retainedDocument")
                .unwrap(),
            "true",
            "later creation must still accept the Document's original registry"
        );
    }
}

#[test]
fn inactive_child_document_registry_getter_preserves_later_registry_validation() {
    let mut vm = inactive_document_test_vm(false);
    let result = vm
        .eval(
            r#"
            (() => {
              const doc = registryFrame.contentDocument;
              const registry = registryFrame.contentWindow.customElements;
              const scoped = new CustomElementRegistry();
              registryFrame.remove();
              const failures = [];
              const check = (name, operation) => {
                try { if (!operation()) failures.push(name); }
                catch (error) { failures.push(name + ':' + error.name); }
              };
              check('registry getter', () => doc.customElementRegistry === registry);
              check('implicit registry', () => doc.createElement('div').ownerDocument === doc);
              check('explicit registry', () =>
                doc.createElement('div', {customElementRegistry: registry}).ownerDocument === doc);
              check('explicit null', () =>
                doc.createElement('div', {customElementRegistry: null}).customElementRegistry === null);
              check('scoped registry', () =>
                doc.createElement('div', {customElementRegistry: scoped}).customElementRegistry === scoped);
              check('importNode', () =>
                doc.importNode(document.createElement('span'), {customElementRegistry: registry})
                  .ownerDocument === doc);
              check('attachShadow', () =>
                doc.createElement('div').attachShadow({mode: 'open', customElementRegistry: registry})
                  .customElementRegistry === registry);
              const rejectsForeign = operation => {
                try { operation(); return false; }
                catch (error) { return error.name === 'NotSupportedError'; }
              };
              check('reject parent registry', () => rejectsForeign(() =>
                doc.createElement('div', {customElementRegistry: customElements})));
              check('reject child registry in parent', () => rejectsForeign(() =>
                document.createElement('div', {customElementRegistry: registry})));
              check('reject foreign registry in createElementNS', () => rejectsForeign(() =>
                doc.createElementNS('http://www.w3.org/1999/xhtml', 'div',
                                    {customElementRegistry: customElements})));
              check('getter remains stable', () => doc.customElementRegistry === registry);
              return JSON.stringify(failures);
            })()
            "#,
        )
        .unwrap();
    assert_eq!(result, "[]");
}

#[test]
fn inactive_child_document_keeps_reachable_wrappers_without_retaining_unreachable_nodes() {
    for loaded_srcdoc in [false, true] {
        let mut vm = inactive_document_test_vm(loaded_srcdoc);
        vm.eval(
            r#"
            globalThis.retainedDocument = registryFrame.contentDocument;
            globalThis.retainedBody = retainedDocument.body;
            globalThis.retainedChildren = retainedBody.children;
            globalThis.retainedNode = retainedDocument.createElement('div');
            retainedNode.marker = {};
            globalThis.retainedMarker = retainedNode.marker;
            retainedBody.appendChild(retainedNode);
            globalThis.weakBeforeRemoval = new WeakRef(retainedDocument.createElement('aside'));
            registryFrame.remove();
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
                  beforeCollected: weakBeforeRemoval.deref() === undefined,
                  afterCollected: weakAfterRemoval.deref() === undefined
                })"#,
            )
            .unwrap(),
            r#"{"document":true,"body":true,"children":true,"node":true,"expando":true,"beforeCollected":true,"afterCollected":true}"#,
            "loaded_srcdoc={loaded_srcdoc}"
        );
    }
}

#[test]
fn inactive_child_document_and_nodes_keep_registry_associations_after_teardown() {
    for loaded_srcdoc in [false, true] {
        let mut vm = inactive_document_test_vm(loaded_srcdoc);
        vm.eval(
            r#"
            globalThis.retainedDocument = registryFrame.contentDocument;
            globalThis.retainedRegistry = registryFrame.contentWindow.customElements;
            globalThis.scopedRegistry = new CustomElementRegistry();
            globalThis.retainedElements = [
              retainedDocument.createElement('div'),
              retainedDocument.createElement('div', {customElementRegistry: retainedRegistry}),
              retainedDocument.createElement('div', {customElementRegistry: scopedRegistry}),
              retainedDocument.createElement('div', {customElementRegistry: null})
            ];
            registryFrame.remove();
            "#,
        )
        .unwrap();
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval(
                r#"JSON.stringify([
                  retainedDocument.customElementRegistry === retainedRegistry,
                  retainedElements[0].customElementRegistry === retainedRegistry,
                  retainedElements[1].customElementRegistry === retainedRegistry,
                  retainedElements[2].customElementRegistry === scopedRegistry,
                  retainedElements[3].customElementRegistry === null,
                  retainedDocument.createElement('span', {customElementRegistry: retainedRegistry})
                    .customElementRegistry === retainedRegistry,
                  retainedDocument.importNode(document.createElement('span'),
                    {customElementRegistry: retainedRegistry}).ownerDocument === retainedDocument,
                  retainedDocument.createElement('div').attachShadow({mode: 'open',
                    customElementRegistry: retainedRegistry}).customElementRegistry === retainedRegistry
                ])"#,
            )
            .unwrap(),
            "[true,true,true,true,true,true,true,true]",
            "loaded_srcdoc={loaded_srcdoc}"
        );
    }
}

#[test]
fn inactive_child_document_can_materialize_its_intrinsic_registry_after_teardown() {
    for materialized_constructor in [false, true] {
        let mut vm = inactive_document_test_vm(false);
        vm.eval(&format!(
            "globalThis.materializedConstructor = {materialized_constructor}"
        ))
        .unwrap();
        vm.eval(
            r#"
            globalThis.retainedDocument = registryFrame.contentDocument;
            globalThis.registryPrototype = materializedConstructor
              ? registryFrame.contentWindow.CustomElementRegistry.prototype : null;
            globalThis.registryPropertyReads = 0;
            for (const name of ['customElements', 'CustomElementRegistry']) {
              Object.defineProperty(registryFrame.contentWindow, name, {configurable: true, get() {
                registryPropertyReads++;
                throw new Error('public property must not supply the native registry');
              }});
            }
            registryFrame.remove();
            "#,
        )
        .unwrap();
        vm.drain_pending_child_frame_work_for_test();
        assert_eq!(
            vm.eval(
                r#"(() => {
                  const registry = retainedDocument.customElementRegistry;
                  globalThis.weakRegistry = new WeakRef(registry);
                  return JSON.stringify([
                    !materializedConstructor || Object.getPrototypeOf(registry) === registryPrototype,
                    Object.prototype.toString.call(registry) === '[object CustomElementRegistry]',
                    retainedDocument.customElementRegistry === registry,
                    retainedDocument.createElement('div').customElementRegistry === registry,
                    registryPropertyReads === 0
                  ]);
                })()"#,
            )
            .unwrap(),
            "[true,true,true,true,true]",
            "materialized_constructor={materialized_constructor}"
        );
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
        assert_eq!(
            vm.eval("weakRegistry.deref() !== undefined && retainedDocument.customElementRegistry === weakRegistry.deref()")
                .unwrap(),
            "true",
            "a retained Document must keep its lazily created registry alive"
        );
    }
}
