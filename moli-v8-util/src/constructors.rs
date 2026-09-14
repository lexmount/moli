use std::rc::Rc;

use v8::{Local, Object, PinScope, PropertyAttribute};

use crate::properties::new_null_prototype_object;
use crate::strings::{v8_string, v8str};
use crate::symbols::{get_private_object, set_private_value};

const ECMASCRIPT_CONSTRUCTORS_SLOT: &str = "__moliEcmascriptConstructors";
const ECMASCRIPT_PROTOTYPES_SLOT: &str = "__moliEcmascriptPrototypes";

#[derive(Clone, Copy)]
pub enum WebApiIntrinsicKind {
    Constructor,
    Prototype,
    /// Allocate from the embedder's original instance template, without Call.
    Instance,
}

pub enum WebApiIntrinsicLookup<'s> {
    /// This name is outside the embedder's Web API registry.
    Unmanaged,
    /// A managed name must never fall back to an author-controlled global,
    /// including when materialization failed or the interface is unavailable.
    Managed(Option<Local<'s, Object>>),
}

pub type WebApiIntrinsicResolver =
    for<'s, 'i> fn(&mut PinScope<'s, 'i>, &str, WebApiIntrinsicKind) -> WebApiIntrinsicLookup<'s>;

struct RealmWebApiIntrinsicResolver(WebApiIntrinsicResolver);

/// Connects shared binding helpers to the embedder's canonical realm cache.
/// Install after eager bootstrap; the resolver stores no intrinsic identities.
pub fn install_web_api_intrinsic_resolver(
    scope: &mut PinScope<'_, '_>,
    resolver: WebApiIntrinsicResolver,
) {
    scope
        .get_current_context()
        .set_slot(Rc::new(RealmWebApiIntrinsicResolver(resolver)));
}

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

/// Ensures that a realm global owns its native-only ECMAScript intrinsic maps.
///
/// The maps are stored under V8 private symbols, so author code cannot observe
/// or replace them through JavaScript reflection. They intentionally contain
/// realm-local V8 objects rather than Rust `Global` handles, allowing V8 to
/// reclaim the whole realm graph when its context becomes unreachable.
pub fn initialize_ecmascript_intrinsic_registry<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
) {
    let _ = intrinsic_registry_object(scope, global, ECMASCRIPT_CONSTRUCTORS_SLOT);
    let _ = intrinsic_registry_object(scope, global, ECMASCRIPT_PROTOTYPES_SLOT);
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

/// Records a trusted ECMAScript constructor and prototype. Web API identities
/// belong to the embedder's realm cache, accessed through its resolver.
///
/// Registrations are immutable. Returning `false` means allocation failed or
/// an entry with the same name was already finalized; callers should treat
/// that as a bootstrap/materialization error rather than silently replacing an
/// intrinsic identity.
pub fn register_ecmascript_intrinsic<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
    constructor: Local<'s, Object>,
    prototype: Local<'s, Object>,
) -> bool {
    let constructors = intrinsic_registry_object(scope, global, ECMASCRIPT_CONSTRUCTORS_SLOT);
    let prototypes = intrinsic_registry_object(scope, global, ECMASCRIPT_PROTOTYPES_SLOT);

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

pub fn registered_ecmascript_constructor<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    registered_intrinsic(scope, global, ECMASCRIPT_CONSTRUCTORS_SLOT, name)
}

pub fn registered_ecmascript_prototype<'s>(
    scope: &mut PinScope<'s, '_>,
    global: Local<'s, Object>,
    name: &str,
) -> Option<Local<'s, Object>> {
    registered_intrinsic(scope, global, ECMASCRIPT_PROTOTYPES_SLOT, name)
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

/// Allocates a native instance without invoking an author-visible constructor.
/// Managed realms use their original instance template. A standalone embedder
/// can register immutable intrinsic prototypes and initialize native state itself.
pub fn new_intrinsic_interface_instance<'s>(
    scope: &mut PinScope<'s, '_>,
    name: &str,
) -> Option<Local<'s, Object>> {
    if let Some(resolver) = scope
        .get_current_context()
        .get_slot::<RealmWebApiIntrinsicResolver>()
        && let WebApiIntrinsicLookup::Managed(instance) =
            (resolver.0)(scope, name, WebApiIntrinsicKind::Instance)
    {
        return instance;
    }
    let prototype = global_constructor_prototype(scope, name)?;
    let object = Object::new(scope);
    (object.set_prototype(scope, prototype.into()) == Some(true)).then_some(object)
}

pub fn global_constructor_object<'s>(
    scope: &mut PinScope<'s, '_>,
    name: &str,
) -> Option<Local<'s, Object>> {
    if let Some(resolver) = scope
        .get_current_context()
        .get_slot::<RealmWebApiIntrinsicResolver>()
        && let WebApiIntrinsicLookup::Managed(constructor) =
            (resolver.0)(scope, name, WebApiIntrinsicKind::Constructor)
    {
        return constructor;
    }
    let global = scope.get_current_context().global(scope);
    registered_ecmascript_constructor(scope, global, name)
        .or_else(|| constructor_object(scope, global, name))
}

pub fn global_constructor_prototype<'s>(
    scope: &mut PinScope<'s, '_>,
    name: &str,
) -> Option<Local<'s, Object>> {
    if let Some(resolver) = scope
        .get_current_context()
        .get_slot::<RealmWebApiIntrinsicResolver>()
        && let WebApiIntrinsicLookup::Managed(prototype) =
            (resolver.0)(scope, name, WebApiIntrinsicKind::Prototype)
    {
        return prototype;
    }
    let global = scope.get_current_context().global(scope);
    registered_ecmascript_prototype(scope, global, name)
        .or_else(|| constructor_prototype(scope, global, name))
}

pub fn callable_relevant_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut callable: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Context>> {
    // GetFunctionRealm follows both bound targets and Proxy targets. A bound
    // Proxy's creation context need not be the underlying function's realm.
    // Read internal targets without invoking author-defined property traps.
    loop {
        if let Ok(proxy) = v8::Local::<v8::Proxy>::try_from(callable) {
            if proxy.is_revoked() {
                throw_type_error(scope, "Cannot determine the realm of a revoked Proxy");
                return None;
            }
            callable = proxy.get_target(scope);
            continue;
        }
        if let Ok(function) = v8::Local::<v8::Function>::try_from(callable) {
            let target = function.get_bound_function(scope);
            if !target.is_undefined() {
                callable = target;
                continue;
            }
        }
        return v8::Local::<v8::Object>::try_from(callable)
            .ok()?
            .get_creation_context(scope);
    }
}

pub fn new_target_realm_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    new_target: v8::Local<'s, v8::Value>,
    constructor_name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let context = callable_relevant_context(scope, new_target)?;
    let prototype = {
        let context_scope = &mut v8::ContextScope::new(scope, context);
        let prototype = global_constructor_prototype(context_scope, constructor_name)?;
        v8::Global::new(context_scope, prototype)
    };
    Some(v8::Local::new(scope, &prototype))
}

pub fn receiver_uses_new_target_realm_object_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    new_target: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(receiver_prototype) = receiver.get_prototype(scope) else {
        return false;
    };
    let Some(object_prototype) =
        new_target_realm_constructor_prototype(scope, new_target, "Object")
    else {
        return false;
    };
    receiver_prototype.strict_equals(object_prototype.into())
}

pub fn apply_webidl_constructor_prototype_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    new_target: v8::Local<'s, v8::Value>,
    default_constructor_name: &str,
) {
    // V8 has already read `NewTarget.prototype` while preparing the native
    // constructor receiver. Do not read it again: a Proxy getter must run only
    // once. V8 represents a non-object result with NewTarget's realm-local
    // Object prototype, which lets us replace precisely that fallback with
    // the WebIDL interface's default prototype from the same realm.
    if !receiver_uses_new_target_realm_object_fallback(scope, receiver, new_target) {
        return;
    }
    let Some(prototype) =
        new_target_realm_constructor_prototype(scope, new_target, default_constructor_name)
    else {
        return;
    };
    let _ = receiver.set_prototype(scope, prototype.into());
}

/// Materializes the unexposed prototype described by a WebIDL iterator
/// FunctionTemplate. The temporary constructor is an implementation detail, so
/// its back-reference must not leak onto the iterator prototype.
pub fn materialize_hidden_function_template_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = template.get_function(scope)?;
    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let constructor_key = v8str(scope, "constructor");
    let _ = prototype.delete(scope, constructor_key.into());
    Some(prototype)
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use moli_v8_test_util::ensure_v8;

    use super::{
        ECMASCRIPT_CONSTRUCTORS_SLOT, ECMASCRIPT_PROTOTYPES_SLOT, WebApiIntrinsicKind,
        WebApiIntrinsicLookup, global_constructor_object, global_constructor_prototype,
        initialize_ecmascript_intrinsic_registry, install_web_api_intrinsic_resolver,
        new_intrinsic_interface_instance, register_ecmascript_intrinsic,
        registered_ecmascript_constructor, registered_ecmascript_prototype,
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

        initialize_ecmascript_intrinsic_registry(scope, global);
        let intrinsic_constructor = v8::Object::new(scope);
        let intrinsic_prototype = v8::Object::new(scope);
        assert!(register_ecmascript_intrinsic(
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
            registered_ecmascript_constructor(scope, global, "Sample")
                .is_some_and(|value| value.strict_equals(intrinsic_constructor.into()))
        );
        assert!(
            registered_ecmascript_prototype(scope, global, "Sample")
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
        let instance = new_intrinsic_interface_instance(scope, "Sample").unwrap();
        assert!(
            instance
                .get_prototype(scope)
                .unwrap()
                .strict_equals(intrinsic_prototype.into())
        );
    }

    #[test]
    fn failed_managed_native_allocation_never_falls_back_to_public_globals() {
        fn unavailable<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            name: &str,
            kind: WebApiIntrinsicKind,
        ) -> WebApiIntrinsicLookup<'s> {
            let _ = (scope, name);
            assert!(matches!(kind, WebApiIntrinsicKind::Instance));
            WebApiIntrinsicLookup::Managed(None)
        }

        ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        // Object is present and callable, so an accidental fallback would succeed.
        install_web_api_intrinsic_resolver(scope, unavailable);
        assert!(new_intrinsic_interface_instance(scope, "Object").is_none());
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

        initialize_ecmascript_intrinsic_registry(scope, global);
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
        assert!(register_ecmascript_intrinsic(
            scope,
            global,
            "Sample",
            constructor,
            prototype,
        ));
        let replacement_constructor = v8::Object::new(scope);
        let replacement_prototype = v8::Object::new(scope);
        assert!(!register_ecmascript_intrinsic(
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
            assert_ne!(name.as_deref(), Some(ECMASCRIPT_CONSTRUCTORS_SLOT));
            assert_ne!(name.as_deref(), Some(ECMASCRIPT_PROTOTYPES_SLOT));
        }
    }
}
