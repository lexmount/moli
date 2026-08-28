use v8::{Local, Object, PinScope, PropertyAttribute};

use crate::properties::new_null_prototype_object;
use crate::strings::{v8_string, v8str};

const INTRINSIC_CONSTRUCTORS_SLOT: &str = "__moliIntrinsicConstructors";
const INTRINSIC_PROTOTYPES_SLOT: &str = "__moliIntrinsicPrototypes";
const PUBLIC_INTERFACE_OBJECTS_SLOT: &str = "__moliPublicInterfaceObjects";
const INTRINSIC_INTERFACE_REGISTRY_EMBEDDER_DATA_SLOT: i32 = 0;

#[derive(Debug)]
struct IntrinsicInterfaceRegistryEmbedderDataMarker;

fn intrinsic_registry_owner<'s>(scope: &mut PinScope<'s, '_>) -> Local<'s, Object> {
    let context = scope.get_current_context();
    if context
        .get_slot::<IntrinsicInterfaceRegistryEmbedderDataMarker>()
        .is_some()
    {
        return context
            .get_embedder_data(scope, INTRINSIC_INTERFACE_REGISTRY_EMBEDDER_DATA_SLOT)
            .and_then(|value| Local::<Object>::try_from(value).ok())
            .expect("initialized intrinsic registry embedder data must be an Object");
    }

    let owner = new_null_prototype_object(scope);
    context.set_embedder_data(
        INTRINSIC_INTERFACE_REGISTRY_EMBEDDER_DATA_SLOT,
        owner.into(),
    );
    let previous = context.set_slot(std::rc::Rc::new(
        IntrinsicInterfaceRegistryEmbedderDataMarker,
    ));
    debug_assert!(previous.is_none());
    owner
}

fn intrinsic_registry_object<'s>(
    scope: &mut PinScope<'s, '_>,
    _global: Local<'s, Object>,
    slot: &'static str,
) -> Local<'s, Object> {
    let owner = intrinsic_registry_owner(scope);
    let key = v8str(scope, slot);
    if let Some(registry) = owner
        .get(scope, key.into())
        .and_then(|value| Local::<Object>::try_from(value).ok())
    {
        return registry;
    }

    let registry = new_null_prototype_object(scope);
    assert!(
        owner
            .define_own_property(
                scope,
                key.into(),
                registry.into(),
                PropertyAttribute::DONT_ENUM
                    | PropertyAttribute::READ_ONLY
                    | PropertyAttribute::DONT_DELETE,
            )
            .unwrap_or(false),
        "failed to install intrinsic interface registry map"
    );
    registry
}

/// Ensures that a V8 Context owns its native-only intrinsic interface maps.
///
/// The traceable owner lives in Context embedder data rather than on the
/// stable outer global proxy. The traceable owner itself is inaccessible to
/// author code, so its maps cannot be observed or replaced through JavaScript
/// reflection. They contain realm-local V8 objects rather than Rust `Global`
/// handles, allowing V8 to reclaim the whole realm graph when its Context
/// becomes unreachable.
pub fn initialize_intrinsic_interface_registry<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
) {
    let _ = intrinsic_registry_object(scope, global, INTRINSIC_CONSTRUCTORS_SLOT);
    let _ = intrinsic_registry_object(scope, global, INTRINSIC_PROTOTYPES_SLOT);
    let _ = intrinsic_registry_object(scope, global, PUBLIC_INTERFACE_OBJECTS_SLOT);
}

/// Replaces all native-only interface maps for a newly attached realm.
///
/// `Context::global()` is V8's stable outer global proxy. Navigation can
/// detach that proxy and attach it to a new Context, so private properties on
/// the proxy are not a per-Context owner boundary. Reusing an existing map
/// would make the new realm retain constructors, prototypes, and closures from
/// the detached realm. Call this exactly once at the start of each real realm
/// bootstrap, before any interface can be materialized.
pub fn reset_intrinsic_interface_registry<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
) {
    let context = scope.get_current_context();
    let owner = new_null_prototype_object(scope);
    context.set_embedder_data(
        INTRINSIC_INTERFACE_REGISTRY_EMBEDDER_DATA_SLOT,
        owner.into(),
    );
    if context
        .get_slot::<IntrinsicInterfaceRegistryEmbedderDataMarker>()
        .is_none()
    {
        let previous = context.set_slot(std::rc::Rc::new(
            IntrinsicInterfaceRegistryEmbedderDataMarker,
        ));
        debug_assert!(previous.is_none());
    }
    initialize_intrinsic_interface_registry(scope, global);
}

fn define_intrinsic<'s>(
    scope: &mut PinScope<'s, '_>,
    registry: Local<'s, Object>,
    name: &str,
    value: Local<'s, Object>,
) -> bool {
    let Some(key) = v8_string(scope, name) else {
        return false;
    };
    registry
        .define_own_property(
            scope,
            key.into(),
            value.into(),
            PropertyAttribute::DONT_ENUM
                | PropertyAttribute::READ_ONLY
                | PropertyAttribute::DONT_DELETE,
        )
        .unwrap_or(false)
}

/// Records the trusted constructor and prototype for one realm-local Web API.
///
/// Registrations are immutable. Returning `false` means allocation failed or
/// an entry with the same name was already finalized; callers should treat
/// that as a bootstrap/materialization error rather than silently replacing an
/// intrinsic identity.
pub fn register_intrinsic_interface<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
    constructor: Local<'s, Object>,
    prototype: Local<'s, Object>,
) -> bool {
    let constructors = intrinsic_registry_object(scope, global, INTRINSIC_CONSTRUCTORS_SLOT);
    let prototypes = intrinsic_registry_object(scope, global, INTRINSIC_PROTOTYPES_SLOT);

    let Some(key) = v8_string(scope, name) else {
        return false;
    };
    if constructors.has_own_property(scope, key.into()) != Some(false)
        || prototypes.has_own_property(scope, key.into()) != Some(false)
    {
        return false;
    }

    if !define_intrinsic(scope, constructors, name, constructor) {
        return false;
    }
    if !define_intrinsic(scope, prototypes, name, prototype) {
        // A failed second definition is only realistically possible after an
        // allocation failure. Leave the constructor entry intact so callers
        // cannot mistake a partially initialized interface for an ambient
        // public-global lookup.
        return false;
    }
    true
}

/// Records the value returned by the exposed-interface lazy property.
///
/// Most interfaces expose their intrinsic constructor directly. HTML element
/// interfaces use a callable Proxy around that constructor to enforce the
/// custom-element early-sanity check, so the public value is tracked
/// separately from the trusted constructor used for inheritance and wrapper
/// creation.
pub fn register_public_interface_object<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
    value: Local<'s, Object>,
) -> bool {
    let objects = intrinsic_registry_object(scope, global, PUBLIC_INTERFACE_OBJECTS_SLOT);
    let Some(key) = v8_string(scope, name) else {
        return false;
    };
    if objects.has_own_property(scope, key.into()) != Some(false) {
        return false;
    }
    define_intrinsic(scope, objects, name, value)
}

fn registered_intrinsic<'s>(
    scope: &mut PinScope<'s, '_>,
    _global: Local<'s, Object>,
    slot: &'static str,
    name: &str,
) -> Option<Local<'s, Object>> {
    let owner = intrinsic_registry_owner(scope);
    let registry = owner
        .get(scope, v8str(scope, slot).into())
        .and_then(|value| Local::<Object>::try_from(value).ok())?;
    let key = v8_string(scope, name)?;
    registry
        .get(scope, key.into())
        .and_then(|value| Local::<Object>::try_from(value).ok())
}

pub fn registered_intrinsic_constructor<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    registered_intrinsic(scope, global, INTRINSIC_CONSTRUCTORS_SLOT, name)
}

pub fn registered_intrinsic_prototype<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    registered_intrinsic(scope, global, INTRINSIC_PROTOTYPES_SLOT, name)
}

pub fn registered_public_interface_object<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    registered_intrinsic(scope, global, PUBLIC_INTERFACE_OBJECTS_SLOT, name)
}

pub fn constructor_object<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    let key = v8_string(scope, name)?;
    global
        .get_real_named_property(scope, key.into())
        .or_else(|| global.get(scope, key.into()))
        .and_then(|value| Local::<Object>::try_from(value).ok())
}

pub fn constructor_prototype_object<'s>(
    scope: &mut PinScope<'s, '_>,
    constructor: Local<'s, Object>,
) -> Option<Local<'s, Object>> {
    constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| Local::<Object>::try_from(value).ok())
}

pub fn constructor_prototype<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    constructor_object(scope, global, name)
        .and_then(|ctor| constructor_prototype_object(scope, ctor))
}

pub fn global_constructor_object<'s>(
    scope: &mut PinScope<'s, '_>,
    name: &str,
) -> Option<Local<'s, Object>> {
    let global = scope.get_current_context().global(scope);
    registered_intrinsic_constructor(scope, global, name)
        .or_else(|| constructor_object(scope, global, name))
}

pub fn global_constructor_prototype<'s>(
    scope: &mut PinScope<'s, '_>,
    name: &str,
) -> Option<Local<'s, Object>> {
    let global = scope.get_current_context().global(scope);
    registered_intrinsic_prototype(scope, global, name)
        .or_else(|| constructor_prototype(scope, global, name))
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use moli_v8_test_util::ensure_v8;

    use super::{
        INTRINSIC_CONSTRUCTORS_SLOT, INTRINSIC_PROTOTYPES_SLOT, global_constructor_object,
        global_constructor_prototype, initialize_intrinsic_interface_registry,
        register_intrinsic_interface, registered_intrinsic_constructor,
        registered_intrinsic_prototype, reset_intrinsic_interface_registry,
    };
    use crate::strings::v8str;

    #[test]
    fn registered_intrinsics_survive_public_global_overrides() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        initialize_intrinsic_interface_registry(scope, global);
        let intrinsic_constructor = v8::Object::new(scope);
        let intrinsic_prototype = v8::Object::new(scope);
        assert!(register_intrinsic_interface(
            scope,
            global,
            "Sample",
            intrinsic_constructor,
            intrinsic_prototype,
        ));

        let public_constructor = v8::Object::new(scope);
        let public_prototype = v8::Object::new(scope);
        assert_eq!(
            public_constructor.set(
                scope,
                v8str(scope, "prototype").into(),
                public_prototype.into(),
            ),
            Some(true),
        );
        assert_eq!(
            global.set(
                scope,
                v8str(scope, "Sample").into(),
                public_constructor.into(),
            ),
            Some(true),
        );

        assert!(
            registered_intrinsic_constructor(scope, global, "Sample")
                .is_some_and(|value| value.strict_equals(intrinsic_constructor.into()))
        );
        assert!(
            registered_intrinsic_prototype(scope, global, "Sample")
                .is_some_and(|value| value.strict_equals(intrinsic_prototype.into()))
        );
        assert!(
            global_constructor_object(scope, "Sample")
                .is_some_and(|value| value.strict_equals(intrinsic_constructor.into()))
        );
        assert!(
            global_constructor_prototype(scope, "Sample")
                .is_some_and(|value| value.strict_equals(intrinsic_prototype.into()))
        );
    }

    #[test]
    fn unregistered_constructors_still_fall_back_to_the_public_global() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        initialize_intrinsic_interface_registry(scope, global);
        let constructor = v8::Object::new(scope);
        let prototype = v8::Object::new(scope);
        assert_eq!(
            constructor.set(scope, v8str(scope, "prototype").into(), prototype.into()),
            Some(true),
        );
        assert_eq!(
            global.set(scope, v8str(scope, "Fallback").into(), constructor.into()),
            Some(true),
        );

        assert!(
            global_constructor_object(scope, "Fallback")
                .is_some_and(|value| value.strict_equals(constructor.into()))
        );
        assert!(
            global_constructor_prototype(scope, "Fallback")
                .is_some_and(|value| value.strict_equals(prototype.into()))
        );
    }

    #[test]
    fn intrinsic_registrations_are_immutable_and_private() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(scope);

        let constructor = v8::Object::new(scope);
        let prototype = v8::Object::new(scope);
        assert!(register_intrinsic_interface(
            scope,
            global,
            "Sample",
            constructor,
            prototype,
        ));
        let replacement_constructor = v8::Object::new(scope);
        let replacement_prototype = v8::Object::new(scope);
        assert!(!register_intrinsic_interface(
            scope,
            global,
            "Sample",
            replacement_constructor,
            replacement_prototype,
        ));

        let names = global
            .get_own_property_names(scope, Default::default())
            .expect("global own property names");
        for index in 0..names.length() {
            let name = names
                .get_index(scope, index)
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope));
            assert_ne!(name.as_deref(), Some(INTRINSIC_CONSTRUCTORS_SLOT));
            assert_ne!(name.as_deref(), Some(INTRINSIC_PROTOTYPES_SLOT));
        }
    }

    #[test]
    fn reused_global_proxy_keeps_intrinsic_maps_isolated_by_context() {
        ensure_v8();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let global_template = v8::ObjectTemplate::new(scope);
        let old_context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                ..Default::default()
            },
        );
        let stable_proxy;
        let old_constructor_global;
        {
            let old_scope = &mut v8::ContextScope::new(scope, old_context);
            stable_proxy = old_context.global(old_scope);
            reset_intrinsic_interface_registry(old_scope, stable_proxy);
            let old_constructor = v8::Object::new(old_scope);
            let old_prototype = v8::Object::new(old_scope);
            assert!(register_intrinsic_interface(
                old_scope,
                stable_proxy,
                "Sample",
                old_constructor,
                old_prototype,
            ));
            old_constructor_global = v8::Global::new(old_scope, old_constructor);
        }
        old_context.detach_global();

        let new_context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_template: Some(global_template),
                global_object: Some(stable_proxy.into()),
                ..Default::default()
            },
        );
        {
            let new_scope = &mut v8::ContextScope::new(scope, new_context);
            let rebound_proxy = new_context.global(new_scope);
            assert!(rebound_proxy.strict_equals(stable_proxy.into()));
            reset_intrinsic_interface_registry(new_scope, rebound_proxy);
            assert!(registered_intrinsic_constructor(new_scope, rebound_proxy, "Sample").is_none());
            let new_constructor = v8::Object::new(new_scope);
            let new_prototype = v8::Object::new(new_scope);
            assert!(register_intrinsic_interface(
                new_scope,
                rebound_proxy,
                "Sample",
                new_constructor,
                new_prototype,
            ));
            let registered = registered_intrinsic_constructor(new_scope, rebound_proxy, "Sample")
                .expect("new Context intrinsic registration should be readable");
            assert!(
                registered.strict_equals(new_constructor.into()),
                "new Context intrinsic registry returned a different constructor"
            );
        }

        {
            let old_scope = &mut v8::ContextScope::new(scope, old_context);
            let old_global = old_context.global(old_scope);
            let old_constructor = v8::Local::new(old_scope, &old_constructor_global);
            assert!(
                registered_intrinsic_constructor(old_scope, old_global, "Sample")
                    .is_some_and(|value| value.strict_equals(old_constructor.into()))
            );
        }
    }
}
