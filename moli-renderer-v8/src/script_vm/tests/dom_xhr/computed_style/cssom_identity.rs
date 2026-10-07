use super::*;

#[test]
fn computed_style_declarations_have_independent_objects_and_live_native_state() {
    let mut vm = new_storage_page_task_executor_test_vm("https://computed-style-identity.test/");
    vm.eval(include_str!("cssom_identity.js"))
        .expect("computed-style identity checks should evaluate");
    assert_eq!(
        vm.eval("__computedStyleIdentityResults.complete").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("JSON.stringify(__computedStyleIdentityResults.checks.filter(row => !row.passed))")
            .unwrap(),
        "[]"
    );
    assert_eq!(
        vm.eval("__computedStyleIdentityResults.total === 618 && __computedStyleIdentityResults.passed === 618")
            .unwrap(),
        "true"
    );
}

#[test]
fn computed_style_declarations_are_collectible_and_reuse_native_descriptors() {
    let mut vm = new_storage_page_task_executor_test_vm("https://computed-style-identity-gc.test/");
    vm.eval(
        r#"
globalThis.identityGcTarget = document.createElement('div');
identityGcTarget.style.color = 'rgb(1, 2, 3)';
document.body.appendChild(identityGcTarget);
globalThis.identityGcHeld = getComputedStyle(identityGcTarget);
globalThis.identityGcHeldRef = new WeakRef(identityGcHeld);
globalThis.identityGcTemporary = Array.from({length: 128}, () => new WeakRef(getComputedStyle(identityGcTarget)));
globalThis.identityDescriptorProbes = Array.from({length: 128}, () => getComputedStyle(identityGcTarget));
globalThis.identityInline = identityGcTarget.style;
"#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("new Set(identityDescriptorProbes).size === 128")
            .unwrap(),
        "true"
    );
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let global = scope.get_current_context().global(scope);
        let name = v8::String::new(scope, "identityDescriptorProbes").unwrap();
        let probes =
            v8::Local::<v8::Array>::try_from(global.get(scope, name.into()).unwrap()).unwrap();
        let mut reflector = None;
        for index in 0..probes.length() {
            let wrapper =
                v8::Local::<v8::Object>::try_from(probes.get_index(scope, index).unwrap()).unwrap();
            let id = wrapper.get_internal_field(scope, 0).unwrap();
            let id = v8::Local::<v8::Number>::try_from(id).unwrap().value() as u64;
            assert_eq!(
                *reflector.get_or_insert(id),
                id,
                "new declarations should reuse their native descriptor"
            );
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(vm.eval("identityGcHeld.color").unwrap(), "rgb(1, 2, 3)");
    let document = vm.document_handle_for_test();
    let cached = vm.computed_style_cache_entry_count_for_document_for_test(document);
    vm.eval(
        "identityDescriptorProbes = null; for (let i = 0; i < 512; i++) getComputedStyle(identityGcTarget).color;"
    )
    .unwrap();
    assert_eq!(
        vm.computed_style_cache_entry_count_for_document_for_test(document),
        cached,
        "new declarations must preserve the shared native style cache"
    );
    let collect = |vm: &mut crate::runtime::PageVmTaskExecutorTestHarness| {
        vm.renderer_document_isolate
            .clone()
            .with_entered_renderer_document_isolate(|isolate| {
                isolate.clear_kept_objects();
                isolate.low_memory_notification();
                Ok(())
            })
            .unwrap();
    };
    collect(&mut vm);
    assert_eq!(
        vm.eval("identityGcTemporary.every(ref => ref.deref() === undefined) && identityGcHeldRef.deref() === identityGcHeld && identityGcTarget.style === identityInline")
            .unwrap(),
        "true"
    );
    vm.eval("identityGcTarget.style.color = 'rgb(7, 8, 9)';")
        .unwrap();
    assert_eq!(vm.eval("identityGcHeld.color").unwrap(), "rgb(7, 8, 9)");
    vm.eval("identityGcHeld = null").unwrap();
    collect(&mut vm);
    assert_eq!(
        vm.eval("identityGcHeldRef.deref() === undefined").unwrap(),
        "true"
    );
}

#[test]
fn detached_iframe_computed_style_declarations_are_fresh_objects() {
    let mut vm = new_storage_page_task_executor_test_vm("https://detached-style-identity.test/");
    assert_eq!(
        vm.eval(
            r#"(() => {
const detached = document.implementation.createHTMLDocument('');
const frame = detached.createElement('iframe');
detached.body.appendChild(frame);
const child = frame.contentWindow;
const target = child.document.createElement('div');
child.document.body.appendChild(target);
const first = child.getComputedStyle(target), second = child.getComputedStyle(target);
first.__identityTag = 'first';
target.style.color = 'rgb(1, 2, 3)';
return first !== second && !Object.hasOwn(second, '__identityTag') &&
  first.color === 'rgb(1, 2, 3)' && second.color === 'rgb(1, 2, 3)';
})()"#,
        )
        .unwrap(),
        "true"
    );
}
