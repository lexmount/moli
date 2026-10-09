use super::*;
use crate::native_bridge::{
    ComputedStyleDescriptor, ComputedStylePseudoKey, ComputedStyleTargetKey,
    node_runtime_and_handle_from_object,
};

#[test]
fn native_node_identity_does_not_resolve_an_unowned_reflector_in_the_callers_bridge() {
    let mut vm = new_storage_test_vm("https://native-bridge-owner.test/");
    vm.eval("globalThis.nativeNode = document.createElement('div')")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeNode").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let native = v8::Local::<v8::Object>::try_from(value).unwrap();
        let (owner, handle) = node_runtime_and_handle_from_object(scope, native).unwrap();
        assert_eq!(owner, host_ptr);
        let reflector = native.get_internal_field(scope, 0).unwrap();

        // This V8-only fixture has a real, colliding reflector ID but no host.
        // Looking up that ID in the caller's bridge would return nativeNode.
        let context = v8::Context::new(scope, Default::default());
        let unowned = {
            let scope = &mut v8::ContextScope::new(scope, context);
            let template = v8::ObjectTemplate::new(scope);
            template.set_internal_field_count(1);
            let object = template.new_instance(scope).unwrap();
            assert!(object.set_internal_field(0, reflector));
            v8::Global::new(scope, object)
        };
        let unowned = v8::Local::new(scope, &unowned);
        assert!(
            node_runtime_and_handle_from_object(scope, unowned).is_err(),
            "a reflector without its owning host must not borrow the caller's identity"
        );
        assert_eq!(
            node_runtime_and_handle_from_object(scope, native).unwrap(),
            (owner, handle),
            "the genuine wrapper still resolves to its native identity"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn native_node_wrapper_materialization_requires_a_host_for_the_current_context() {
    let mut vm = new_storage_test_vm("https://native-bridge-context.test/");
    vm.eval("globalThis.nativeNode = document.createElement('div')")
        .unwrap();
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "nativeNode").unwrap();
        let value = global.get(scope, key.into()).unwrap();
        let native = v8::Local::<v8::Object>::try_from(value).unwrap();
        let (_, handle) = node_runtime_and_handle_from_object(scope, native).unwrap();
        let descriptor = ComputedStyleDescriptor::new(
            ComputedStylePseudoKey::Originating,
            ComputedStyleTargetKey::Dynamic,
        );

        let context = v8::Context::new(scope, Default::default());
        {
            let scope = &mut v8::ContextScope::new(scope, context);
            assert!(
                unsafe { &mut *host_ptr }
                    .native_bridge_mut()
                    .wrap_handle(scope, host_ptr, handle)
                    .is_none(),
                "an unowned context must not receive a cached or newly created native wrapper"
            );
            assert!(
                unsafe { &mut *host_ptr }
                    .native_bridge_mut()
                    .create_computed_style(scope, host_ptr, handle, descriptor.clone())
                    .is_none(),
                "NewObject declarations also require an owning host"
            );
        }
        let wrapper = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, handle)
            .unwrap();
        assert_eq!(wrapper, native, "the owning context keeps wrapper identity");
        let first = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .create_computed_style(scope, host_ptr, handle, descriptor.clone())
            .unwrap();
        let second = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .create_computed_style(scope, host_ptr, handle, descriptor)
            .unwrap();
        assert_ne!(
            first, second,
            "computed style declarations remain NewObject"
        );
        Ok(())
    })
    .unwrap();
}
