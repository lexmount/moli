use super::*;
use crate::{document_runtime::DomHandle, util::v8str};

fn test_constructor<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
}

#[test]
fn registry_data_shares_definitions_and_promises_without_sharing_native_handles() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    let mut source = CustomElementStore::default();
    let target = CustomElementStore::for_registry(source.registry_data());
    let unrelated = CustomElementStore::default();
    let registry = source.registry_data();
    let registry_wrapper = v8::Object::new(scope);
    registry.bind_wrapper(scope, registry_wrapper);
    let source_promise = registry.when_defined(scope, "x-shared").unwrap();
    let target_promise = target
        .registry_data()
        .when_defined(scope, "x-shared")
        .unwrap();
    assert_eq!(source_promise, target_promise);

    let constructor = v8::Function::new(scope, test_constructor).unwrap();
    let definition = build_custom_element_definition(scope, constructor, None)
        .expect("build a real registry definition");
    registry.install_definition(scope, "x-shared", definition);
    assert_eq!(source_promise.state(), v8::PromiseState::Fulfilled);
    assert!(
        source_promise
            .result(scope)
            .strict_equals(constructor.into())
    );
    assert_eq!(source.get(scope, "x-shared"), Some(constructor));
    assert_eq!(target.get(scope, "x-shared"), Some(constructor));
    assert!(unrelated.get(scope, "x-shared").is_none());

    let handle = DomHandle::new(7);
    source.mark_upgraded_handle(handle, "x-shared");
    source.set_form_association_state(handle, Some(DomHandle::new(8)));
    source.set_form_disabled_state(handle, true);
    assert!(source.owns_custom_element_handle(handle));
    assert!(!target.owns_custom_element_handle(handle));
    assert!(!target.is_upgraded_handle(handle));
    assert_eq!(target.form_association_state(handle), None);
    assert_eq!(target.form_disabled_state(handle), None);

    let pending_handle = DomHandle::new(9);
    let wrapper = v8::Object::new(scope);
    source.begin_construction(
        scope,
        crate::native_bridge::NativeDomHostId::default(),
        constructor,
        wrapper,
        pending_handle,
    );
    assert!(source.has_pending_wrapper_for(scope, constructor));
    assert!(!target.has_pending_wrapper_for(scope, constructor));
    source.mark_failed_construction_handle(pending_handle);
    assert!(!target.is_failed_construction_handle(pending_handle));

    drop(source);
    drop(registry);
    assert_eq!(target.get(scope, "x-shared"), Some(constructor));
    assert!(!target.is_subtree_lifecycle_quiescent());
    assert!(!target.owns_custom_element_handle(handle));
}

#[test]
fn retired_registry_traces_captured_values_without_rooting_an_unreachable_registry() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let data = Rc::new(CustomElementRegistryData::default());
    let (registry, weak_registry, constructor, callback, promise, resolver) = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let registry = v8::Object::new(scope);
        data.bind_wrapper(scope, registry);
        let script = v8str(
            scope,
            "(class { get connectedCallback() { return function captured() {}; } })",
        );
        let script = v8::Script::compile(scope, script, None).unwrap();
        let value = crate::script_execution::execute_compiled_script(scope, script).unwrap();
        let constructor = v8::Local::<v8::Function>::try_from(value).unwrap();
        let definition = build_custom_element_definition(scope, constructor, None).unwrap();
        let captured = definition.callbacks.connected.as_ref().unwrap().clone();
        data.install_definition(scope, "x-traced", definition);
        data.when_defined(scope, "x-pending").unwrap();
        let pending = data.pending_when_defined.borrow();
        let pending = pending.get("x-pending").unwrap();
        (
            v8::Global::new(scope, registry),
            v8::Weak::new(scope, registry),
            v8::Weak::new(scope, constructor),
            captured,
            pending.promise.clone(),
            pending.resolver.clone(),
        )
    };

    data.retire();
    isolate.low_memory_notification();
    assert!(!weak_registry.is_empty());
    assert!(!constructor.is_empty());
    assert!(!callback.is_empty(), "captured getter result stays alive");
    assert!(!promise.is_empty());
    assert!(!resolver.is_empty(), "pending resolution stays possible");

    drop(registry);
    isolate.low_memory_notification();
    assert!(weak_registry.is_empty());
    assert!(constructor.is_empty());
    assert!(callback.is_empty());
    assert!(promise.is_empty());
    assert!(resolver.is_empty());
    assert_eq!(data.definitions.borrow().len(), 1);
    assert_eq!(data.pending_when_defined.borrow().len(), 1);
}

#[test]
fn empty_registry_metadata_does_not_root_its_creation_context() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let data = CustomElementRegistryData::default();
    let context = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let registry = v8::Object::new(scope);
        data.bind_wrapper(scope, registry);
        v8::Weak::new(scope, context)
    };
    isolate.low_memory_notification();
    assert!(
        context.is_empty(),
        "an empty registry has no execution-owned values"
    );
}
