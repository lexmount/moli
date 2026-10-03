use std::pin::pin;
use std::rc::Rc;

use super::super::materialize::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
    materialize_interface,
};
use super::super::metadata::{InterfaceId, RealmKind, TemplateBuildProfile};
use super::super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceState};
use super::super::template_registry::ExposedInterfaceTemplateRegistry;
use super::{counting_lazy_getter, lazy_getter_calls, reset_lazy_getter_calls};
use crate::util::{
    constructor_prototype_object, get_private_value, register_intrinsic_interface,
    registered_intrinsic_constructor, registered_intrinsic_prototype, v8str,
};

struct RegisteredInterface<'s> {
    registry: Rc<ExposedInterfaceTemplateRegistry>,
    realm: Rc<IntrinsicInterfaceRegistry>,
    id: InterfaceId,
    name: &'static str,
    constructor: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
}

impl RegisteredInterface<'_> {
    fn assert_failed(&self, scope: &mut v8::PinScope<'_, '_>, cause: &str) {
        let error = materialize_interface(scope, self.id)
            .expect_err("failed recovery must return an error without panicking");
        assert!(format!("{error:#}").contains(cause), "{error:#}");
        assert_eq!(self.realm.state(self.id), Some(RealmInterfaceState::Failed));
        assert!(self.realm.constructor(scope, self.id).is_none());
        assert!(self.realm.prototype(scope, self.id).is_none());
        assert!(self.realm.public_interface(scope, self.id).is_none());
        assert!(ensure_intrinsic_interface_constructor(scope, self.name).is_err());
        assert!(ensure_intrinsic_interface_prototype(scope, self.name).is_err());
        assert_eq!(self.registry.materialization_count(self.id), 0);
        assert_eq!(lazy_getter_calls(), 0);

        // Immutable private entries survive, but Failed prevents their reuse.
        let global = scope.get_current_context().global(scope);
        assert!(
            registered_intrinsic_constructor(scope, global, self.name)
                .is_some_and(|value| value.strict_equals(self.constructor.into()))
        );
        assert!(
            registered_intrinsic_prototype(scope, global, self.name)
                .is_some_and(|value| value.strict_equals(self.prototype.into()))
        );
    }
}

fn with_registered_interface(
    name: &'static str,
    test: impl for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>, RegisteredInterface<'s>),
) {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let registry = ExposedInterfaceTemplateRegistry::install(
        scope,
        crate::context_bootstrap::specs::constructor_specs(),
        TemplateBuildProfile::Window,
    )
    .expect("template registry");
    let realm = IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        RealmKind::Window,
    )
    .expect("realm registry");
    let id = registry.id_by_name(name).expect("interface metadata");
    let constructor = registry
        .get_or_build_template(scope, id)
        .expect("interface template")
        .get_function(scope)
        .expect("interface constructor");
    let prototype =
        constructor_prototype_object(scope, constructor.into()).expect("interface prototype");
    let global = context.global(scope);
    assert!(register_intrinsic_interface(
        scope,
        global,
        name,
        constructor.into(),
        prototype,
    ));
    assert_eq!(
        global.set_lazy_data_property(scope, v8str(scope, name).into(), counting_lazy_getter),
        Some(true)
    );
    reset_lazy_getter_calls();
    test(
        scope,
        RegisteredInterface {
            registry,
            realm,
            id,
            name,
            constructor,
            prototype,
        },
    );
}

#[test]
fn failed_unscopables_finalization_is_terminal() {
    with_registered_interface("Document", |scope, interface| {
        let unscopables = interface
            .prototype
            .get(scope, v8::Symbol::get_unscopables(scope).into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .expect("template-defined unscopables object");
        assert!(!unscopables.get_prototype(scope).unwrap().is_null());
        assert_eq!(
            unscopables.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        interface.assert_failed(scope, "@@unscopables null prototype");
    });
}

#[test]
fn missing_unscopables_finalization_is_terminal() {
    with_registered_interface("DocumentFragment", |scope, interface| {
        assert_eq!(
            interface
                .prototype
                .delete(scope, v8::Symbol::get_unscopables(scope).into()),
            Some(true)
        );
        interface.assert_failed(scope, "@@unscopables object is unavailable");
    });
}

#[test]
fn failed_notification_finalization_is_terminal() {
    with_registered_interface("Notification", |scope, interface| {
        assert_eq!(
            interface
                .constructor
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        interface.assert_failed(scope, "failed to define `permission` accessor");
    });
}

#[test]
fn failed_performance_observer_finalization_is_terminal() {
    with_registered_interface("PerformanceObserver", |scope, interface| {
        assert_eq!(
            interface
                .constructor
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        interface.assert_failed(scope, "failed to define `supportedEntryTypes` value");
    });
}

#[test]
fn failed_html_constructor_link_is_terminal() {
    with_registered_interface("HTMLDivElement", |scope, interface| {
        assert_eq!(
            interface
                .prototype
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen),
            Some(true)
        );
        interface.assert_failed(scope, "intrinsic prototype constructor");
    });
}

#[test]
fn published_intrinsics_stay_hidden_until_the_caller_exposes_them() {
    with_registered_interface("DOMStringMap", |scope, interface| {
        let global = scope.get_current_context().global(scope);
        interface
            .realm
            .register_intrinsic_objects(
                scope,
                global,
                interface.id,
                interface.name,
                interface.constructor.into(),
                interface.prototype,
                interface.constructor.into(),
            )
            .expect("intrinsic publication");
        assert_eq!(
            interface.realm.state(interface.id),
            Some(RealmInterfaceState::Materializing)
        );
        assert!(interface.realm.constructor(scope, interface.id).is_none());
        assert!(interface.realm.prototype(scope, interface.id).is_none());
        assert!(
            interface
                .realm
                .public_interface(scope, interface.id)
                .is_none()
        );
        interface
            .realm
            .set_state(interface.id, RealmInterfaceState::Finalizing)
            .expect("caller starts finalization");
        assert!(interface.realm.constructor(scope, interface.id).is_some());
        assert!(interface.realm.prototype(scope, interface.id).is_some());
        assert!(
            interface
                .realm
                .public_interface(scope, interface.id)
                .is_some()
        );
    });
}

#[test]
fn recovered_xhr_event_target_initializes_its_private_state() {
    with_registered_interface("XMLHttpRequestEventTarget", |scope, interface| {
        assert!(
            ensure_intrinsic_interface_prototype(scope, interface.name)
                .expect("event target recovery")
                .strict_equals(interface.prototype.into())
        );
        let listener_slot = get_private_value(
            scope,
            interface.prototype,
            crate::context_bootstrap::SIMPLE_EVENT_TARGET_SLOT,
        )
        .expect("listener slot name")
        .to_string(scope)
        .expect("listener slot string")
        .to_rust_string_lossy(scope);
        assert_eq!(listener_slot, "__moliXhrEventTargetListeners");
        for slot in [
            crate::context_bootstrap::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT,
            "__moliXhrUsesSimpleEventTarget",
        ] {
            assert!(
                get_private_value(scope, interface.prototype, slot)
                    .expect("event target marker")
                    .is_true()
            );
        }
        assert_eq!(
            interface.realm.state(interface.id),
            Some(RealmInterfaceState::Ready)
        );
        assert_eq!(lazy_getter_calls(), 0);
    });
}

#[test]
fn uninitialized_crypto_adopts_registered_intrinsics_and_finalizes_secure_surface() {
    assert_registered_crypto_recovery(false);
}

#[test]
fn uninitialized_crypto_adopts_registered_intrinsics_and_refinalizes_secure_surface() {
    assert_registered_crypto_recovery(true);
}

fn assert_registered_crypto_recovery(already_finalized: bool) {
    with_registered_interface("Crypto", |scope, interface| {
        let RegisteredInterface {
            registry,
            realm,
            id,
            constructor,
            prototype,
            ..
        } = interface;
        let global = scope.get_current_context().global(scope);
        for name in ["subtle", "randomUUID"] {
            assert!(
                prototype
                    .get_own_property_descriptor(scope, v8str(scope, name).into())
                    .expect("the raw template has no secure-context surface")
                    .is_undefined()
            );
        }
        crate::context_bootstrap::crypto::install_window_crypto_runtime_state(scope, global, true)
            .expect("secure-context Crypto state");
        if already_finalized {
            crate::context_bootstrap::crypto::finalize_crypto_realm_bindings(scope, prototype)
                .expect("the trusted pair already has a finalized secure surface");
        }
        let get_random_values = prototype
            .get(scope, v8str(scope, "getRandomValues").into())
            .expect("template-defined getRandomValues method");
        assert_eq!(realm.state(id), Some(RealmInterfaceState::Uninitialized));

        let recovered = ensure_intrinsic_interface_prototype(scope, "Crypto")
            .expect("adoption must allow secure-context finalization to run again");
        assert!(recovered.strict_equals(prototype.into()));
        assert_eq!(realm.state(id), Some(RealmInterfaceState::Ready));
        assert!(
            ensure_intrinsic_interface_constructor(scope, "Crypto")
                .expect("adopted Crypto constructor")
                .strict_equals(constructor.into())
        );
        assert!(
            prototype
                .get(scope, v8str(scope, "getRandomValues").into())
                .expect("retained template method")
                .strict_equals(get_random_values)
        );
        let subtle_descriptor = prototype
            .get_own_property_descriptor(scope, v8str(scope, "subtle").into())
            .expect("secure-context subtle descriptor")
            .to_object(scope)
            .expect("subtle descriptor object");
        let subtle_getter = subtle_descriptor
            .get(scope, v8str(scope, "get").into())
            .expect("subtle getter");
        assert!(subtle_getter.is_function());
        assert!(
            subtle_descriptor
                .get(scope, v8str(scope, "set").into())
                .expect("subtle setter field")
                .is_undefined()
        );
        let uuid_descriptor = prototype
            .get_own_property_descriptor(scope, v8str(scope, "randomUUID").into())
            .expect("secure-context randomUUID descriptor")
            .to_object(scope)
            .expect("randomUUID descriptor object");
        let random_uuid = uuid_descriptor
            .get(scope, v8str(scope, "value").into())
            .expect("randomUUID method");
        assert!(random_uuid.is_function());
        for descriptor in [subtle_descriptor, uuid_descriptor] {
            for name in ["enumerable", "configurable"] {
                assert!(
                    descriptor
                        .get(scope, v8str(scope, name).into())
                        .expect("Crypto descriptor attribute")
                        .is_true()
                );
            }
        }
        assert!(
            uuid_descriptor
                .get(scope, v8str(scope, "writable").into())
                .expect("randomUUID writable attribute")
                .is_true()
        );

        assert!(
            materialize_interface(scope, id)
                .expect("repeated Crypto materialization")
                .strict_equals(constructor.into())
        );
        assert!(
            ensure_intrinsic_interface_prototype(scope, "Crypto")
                .expect("repeated Crypto prototype lookup")
                .strict_equals(prototype.into())
        );
        let repeated_subtle_descriptor = prototype
            .get_own_property_descriptor(scope, v8str(scope, "subtle").into())
            .expect("retained subtle descriptor")
            .to_object(scope)
            .expect("subtle descriptor object");
        assert!(
            repeated_subtle_descriptor
                .get(scope, v8str(scope, "get").into())
                .expect("retained subtle getter")
                .strict_equals(subtle_getter)
        );
        assert!(
            prototype
                .get(scope, v8str(scope, "randomUUID").into())
                .expect("retained randomUUID method")
                .strict_equals(random_uuid)
        );
        assert_eq!(registry.build_count(id), 1);
        assert_eq!(registry.materialization_count(id), 1);
        assert_eq!(lazy_getter_calls(), 0);
    });
}
