use v8::{Local, Object, PinScope, PropertyAttribute};

use crate::properties::new_null_prototype_object;
use crate::strings::{v8_string, v8str};
use crate::symbols::{get_private_object, set_private_value};

const INTRINSIC_CONSTRUCTORS_SLOT: &str = "__moliIntrinsicConstructors";
const INTRINSIC_PROTOTYPES_SLOT: &str = "__moliIntrinsicPrototypes";

fn intrinsic_registry_object<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    slot: &str,
) -> Local<'s, Object> {
    if let Some(registry) = get_private_object(scope, global, slot) {
        return registry;
    }

    let registry = new_null_prototype_object(scope);
    set_private_value(scope, global, slot, registry.into());
    registry
}

/// Ensures that a realm global owns its native-only intrinsic interface maps.
///
/// The maps are stored under V8 private symbols, so author code cannot observe
/// or replace them through JavaScript reflection. They intentionally contain
/// realm-local V8 objects rather than Rust `Global` handles, allowing V8 to
/// reclaim the whole realm graph when its context becomes unreachable.
pub fn initialize_intrinsic_interface_registry<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
) {
    let _ = intrinsic_registry_object(scope, global, INTRINSIC_CONSTRUCTORS_SLOT);
    let _ = intrinsic_registry_object(scope, global, INTRINSIC_PROTOTYPES_SLOT);
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

fn registered_intrinsic<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    slot: &str,
    name: &str,
) -> Option<Local<'s, Object>> {
    let registry = get_private_object(scope, global, slot)?;
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
        registered_intrinsic_prototype,
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
}
