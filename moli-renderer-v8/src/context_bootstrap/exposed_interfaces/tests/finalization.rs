use std::pin::pin;
use std::rc::Rc;

use super::super::materialize::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
    materialize_interface,
};
use super::super::metadata::{InterfaceId, RealmKind, TemplateBuildProfile};
use super::super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceEntry};
use super::super::template_registry::ExposedInterfaceTemplateRegistry;
use super::{counting_lazy_getter, lazy_getter_calls, reset_lazy_getter_calls};
use crate::util::{
    constructor_prototype_object, get_private_value, registered_ecmascript_constructor,
    registered_ecmascript_prototype, v8str,
};

struct UnpublishedInterface<'s> {
    registry: Rc<ExposedInterfaceTemplateRegistry>,
    realm: Rc<IntrinsicInterfaceRegistry>,
    id: InterfaceId,
    name: &'static str,
    constructor: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
}

impl UnpublishedInterface<'_> {
    fn assert_failed(&self, scope: &mut v8::PinScope<'_, '_>, cause: &str) {
        let error = materialize_interface(scope, self.id)
            .expect_err("failed materialization must return an error without panicking");
        assert!(format!("{error:#}").contains(cause), "{error:#}");
        assert!(matches!(
            &*self.realm.entry(self.id).unwrap(),
            RealmInterfaceEntry::Failed
        ));
        assert!(self.realm.constructor(scope, self.id).is_none());
        assert!(self.realm.prototype(scope, self.id).is_none());
        assert!(self.realm.public_interface(scope, self.id).is_none());
        assert!(ensure_intrinsic_interface_constructor(scope, self.name).is_err());
        assert!(ensure_intrinsic_interface_prototype(scope, self.name).is_err());
        assert_eq!(self.registry.materialization_count(self.id), 0);
        assert_eq!(lazy_getter_calls(), 0);

        assert!(crate::util::global_constructor_object(scope, self.name).is_none());
        assert!(crate::util::global_constructor_prototype(scope, self.name).is_none());
        // No intrinsic identity survives in a second registry or public lookup.
        let global = scope.get_current_context().global(scope);
        assert!(registered_ecmascript_constructor(scope, global, self.name).is_none());
        assert!(registered_ecmascript_prototype(scope, global, self.name).is_none());
        assert_eq!(
            lazy_getter_calls(),
            0,
            "failed lookups must not read public bindings"
        );
        super::super::install::capture_eager_intrinsic_interfaces(scope, global, RealmKind::Window)
            .expect_err("eager capture cannot revive a failed interface");
        assert!(matches!(
            &*self.realm.entry(self.id).unwrap(),
            RealmInterfaceEntry::Failed
        ));
    }
}

fn with_unpublished_interface(
    name: &'static str,
    test: impl for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>, UnpublishedInterface<'s>),
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
    moli_v8_util::install_web_api_intrinsic_resolver(
        scope,
        super::super::materialize::resolve_web_api_intrinsic,
    );
    assert_eq!(
        global.set_lazy_data_property(scope, v8str(scope, name).into(), counting_lazy_getter),
        Some(true)
    );
    reset_lazy_getter_calls();
    test(
        scope,
        UnpublishedInterface {
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
    with_unpublished_interface("Document", |scope, interface| {
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
    with_unpublished_interface("DocumentFragment", |scope, interface| {
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
    with_unpublished_interface("Notification", |scope, interface| {
        // Freeze after constructor inheritance is linked so the failure still
        // exercises the permission finalizer, rather than an earlier step.
        let parent = ensure_intrinsic_interface_constructor(scope, "EventTarget")
            .expect("Notification parent constructor");
        assert_eq!(
            interface.constructor.set_prototype(scope, parent.into()),
            Some(true)
        );
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
    with_unpublished_interface("PerformanceObserver", |scope, interface| {
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
    with_unpublished_interface("HTMLDivElement", |scope, interface| {
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
fn shared_binding_helpers_preserve_intrinsics_after_author_overrides() {
    with_unpublished_interface("DOMStringMap", |scope, interface| {
        let public = materialize_interface(scope, interface.id).expect("completed materialization");
        let global = scope.get_current_context().global(scope);
        let replacement = v8::Object::new(scope);
        let replacement_prototype = v8::Object::new(scope);
        assert_eq!(
            replacement.set(
                scope,
                v8str(scope, "prototype").into(),
                replacement_prototype.into(),
            ),
            Some(true)
        );
        assert_eq!(
            global.set(
                scope,
                v8str(scope, interface.name).into(),
                replacement.into()
            ),
            Some(true)
        );

        assert!(
            crate::util::global_constructor_object(scope, interface.name)
                .unwrap()
                .strict_equals(interface.constructor.into())
        );
        assert!(
            crate::util::global_constructor_prototype(scope, interface.name)
                .unwrap()
                .strict_equals(interface.prototype.into())
        );
        let instance = v8::Object::new(scope);
        moli_webapi_declare::set_required_interface_prototype(scope, instance, interface.name)
            .expect("shared declarations must use the realm intrinsic");
        assert!(
            instance
                .get_prototype(scope)
                .unwrap()
                .strict_equals(interface.prototype.into())
        );
        assert!(
            materialize_interface(scope, interface.id)
                .unwrap()
                .strict_equals(public)
        );
        assert_eq!(interface.registry.materialization_count(interface.id), 1);
        assert_eq!(
            global.delete(scope, v8str(scope, interface.name).into()),
            Some(true)
        );
        scope.get_current_context().detach_global();
        assert!(
            crate::util::global_constructor_object(scope, interface.name)
                .unwrap()
                .strict_equals(interface.constructor.into())
        );
        assert!(
            crate::util::global_constructor_prototype(scope, interface.name)
                .unwrap()
                .strict_equals(interface.prototype.into())
        );
    });
}

#[test]
fn materialized_xhr_event_target_initializes_its_private_state() {
    with_unpublished_interface("XMLHttpRequestEventTarget", |scope, interface| {
        assert!(
            ensure_intrinsic_interface_prototype(scope, interface.name)
                .expect("event target materialization")
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
        assert!(matches!(
            &*interface.realm.entry(interface.id).unwrap(),
            RealmInterfaceEntry::Ready(_)
        ));
        assert_eq!(lazy_getter_calls(), 0);
    });
}

#[test]
fn crypto_finalizes_its_secure_surface_before_publication() {
    assert_crypto_materialization(false);
}

#[test]
fn crypto_finalization_preserves_existing_secure_surface() {
    assert_crypto_materialization(true);
}

fn assert_crypto_materialization(already_finalized: bool) {
    with_unpublished_interface("Crypto", |scope, interface| {
        let UnpublishedInterface {
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
        assert!(matches!(
            &*realm.entry(id).unwrap(),
            RealmInterfaceEntry::Uninitialized
        ));

        let intrinsic = ensure_intrinsic_interface_prototype(scope, "Crypto")
            .expect("materialization must finalize the secure surface");
        assert!(intrinsic.strict_equals(prototype.into()));
        assert!(matches!(
            &*realm.entry(id).unwrap(),
            RealmInterfaceEntry::Ready(_)
        ));
        assert!(
            ensure_intrinsic_interface_constructor(scope, "Crypto")
                .expect("trusted Crypto constructor")
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
