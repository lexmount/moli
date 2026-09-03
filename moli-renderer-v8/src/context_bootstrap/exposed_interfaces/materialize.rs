use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::finalize::finalize_materialized_interface;
use super::metadata::{InterfaceId, ResolvedPrototypeProperty};
use super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceState};
use super::template_registry::ExposedInterfaceTemplateRegistry;
use crate::context_bootstrap::constructors::html_element_constructor_with_early_sanity_trap;
use crate::context_bootstrap::runtime_state::set_interface_prototype_constructor;
use crate::context_bootstrap::shared::throw_error;
use crate::context_bootstrap::specs::ConstructorKind;
use crate::util::{
    constructor_prototype_object, register_intrinsic_interface, register_public_interface_object,
    registered_intrinsic_constructor, registered_intrinsic_prototype,
    registered_public_interface_object, v8str,
};

pub(super) fn exposed_interface_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(raw_id) = args.data().uint32_value(scope) else {
        throw_error(
            scope,
            "Exposed interface lazy property has invalid callback data.",
        );
        return;
    };
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        // Falling back to the caller's current context would create the
        // constructor in the wrong realm for parent -> iframe property reads.
        throw_error(
            scope,
            "Exposed interface lazy property holder has no creation context.",
        );
        return;
    };

    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    match materialize_interface(scope, InterfaceId::from_callback_data(raw_id)) {
        Ok(interface_object) => rv.set(interface_object),
        Err(error) => throw_error(scope, &format!("Failed to materialize Web API: {error}")),
    }
}

pub(super) fn materialize_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: InterfaceId,
) -> Result<v8::Local<'s, v8::Value>> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    let metadata = registry
        .metadata(id)
        .ok_or_else(|| anyhow!("unknown exposed interface id {}", id.index()))?;
    let realm = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len())?;
    let global = scope.get_current_context().global(scope);

    match realm
        .state(id)
        .ok_or_else(|| anyhow!("interface state is out of range"))?
    {
        RealmInterfaceState::Ready => {
            return realm
                .public_interface(scope, id)
                .map(Into::into)
                .ok_or_else(|| {
                    anyhow!(
                        "ready realm-owned public interface object `{}` is missing",
                        metadata.name
                    )
                });
        }
        RealmInterfaceState::Materializing => {
            return Err(anyhow!("materialization cycle reached `{}`", metadata.name));
        }
        RealmInterfaceState::Finalizing => {
            return realm
                .public_interface(scope, id)
                .map(Into::into)
                .ok_or_else(|| {
                    anyhow!(
                        "finalizing realm-owned public interface object `{}` is missing",
                        metadata.name
                    )
                });
        }
        RealmInterfaceState::Failed => {
            return Err(anyhow!(
                "a previous materialization of `{}` failed",
                metadata.name
            ));
        }
        RealmInterfaceState::Uninitialized => {}
    }

    if registered_intrinsic_constructor(scope, global, metadata.name).is_some()
        || registered_intrinsic_prototype(scope, global, metadata.name).is_some()
        || registered_public_interface_object(scope, global, metadata.name).is_some()
    {
        return Err(anyhow!(
            "uninitialized interface `{}` has partial registry state",
            metadata.name
        ));
    }

    realm.set_state(id, RealmInterfaceState::Materializing)?;
    match materialize_uninitialized_interface(scope, &registry, &realm, id) {
        Ok(interface) => Ok(interface),
        Err(error) => {
            realm.set_state(id, RealmInterfaceState::Failed)?;
            Err(error)
        }
    }
}

pub(crate) fn ensure_intrinsic_interface_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Function>> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    let id = registry
        .id_by_name(name)
        .ok_or_else(|| anyhow!("unknown exposed interface `{name}`"))?;
    let realm = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len())?;
    if let Some(constructor) = realm.constructor(scope, id) {
        return v8::Local::<v8::Function>::try_from(constructor)
            .map_err(|_| anyhow!("intrinsic constructor `{name}` is not a Function"));
    }
    let _ = materialize_interface(scope, id)?;
    let constructor = realm.constructor(scope, id).ok_or_else(|| {
        anyhow!("intrinsic constructor `{name}` is missing after materialization")
    })?;
    v8::Local::<v8::Function>::try_from(constructor)
        .map_err(|_| anyhow!("intrinsic constructor `{name}` is not a Function"))
}

pub(crate) fn build_intrinsic_interface_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Object>> {
    let _ = ensure_intrinsic_interface_constructor(scope, name)?;
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    let id = registry
        .id_by_name(name)
        .ok_or_else(|| anyhow!("unknown exposed interface `{name}`"))?;
    let template = registry
        .ready_template(scope, id)
        .ok_or_else(|| anyhow!("missing FunctionTemplate for `{name}`"))?;
    template
        .instance_template(scope)
        .new_instance(scope)
        .ok_or_else(|| anyhow!("failed to instantiate `{name}` instance template"))
}

pub(crate) fn ensure_intrinsic_interface_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Object>> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    let id = registry
        .id_by_name(name)
        .ok_or_else(|| anyhow!("unknown exposed interface `{name}`"))?;
    let realm = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len())?;
    if let Some(prototype) = realm.prototype(scope, id) {
        return Ok(prototype);
    }
    let _ = ensure_intrinsic_interface_constructor(scope, name)?;
    realm
        .prototype(scope, id)
        .ok_or_else(|| anyhow!("intrinsic prototype `{name}` is missing after materialization"))
}

pub(crate) fn object_is_intrinsic_interface_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return false;
    };
    let Some(id) = registry.id_by_name(name) else {
        return false;
    };
    registry
        .ready_template(scope, id)
        .is_some_and(|template| template.has_instance(object.into()))
}

fn materialize_uninitialized_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &Rc<ExposedInterfaceTemplateRegistry>,
    realm: &Rc<IntrinsicInterfaceRegistry>,
    id: InterfaceId,
) -> Result<v8::Local<'s, v8::Value>> {
    let metadata = registry
        .metadata(id)
        .ok_or_else(|| anyhow!("unknown exposed interface id {}", id.index()))?;
    let parent = metadata
        .parent
        .map(|parent_id| intrinsic_parent(scope, registry, parent_id))
        .transpose()?;
    let runtime_installed_prototype = match metadata.prototype_property {
        ResolvedPrototypeProperty::TemplateReadOnly => None,
        ResolvedPrototypeProperty::RuntimeInstalled { prototype } => {
            Some(intrinsic_prototype(scope, registry, prototype)?)
        }
    };
    let template = registry.get_or_build_template(scope, id).map_err(|error| {
        anyhow!(
            "failed to build FunctionTemplate for `{}`: {error}",
            metadata.name
        )
    })?;
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("V8 failed to create constructor `{}`", metadata.name))?;
    let constructor_prototype = if let Some(prototype) = runtime_installed_prototype {
        let mut descriptor =
            v8::PropertyDescriptor::new_from_value_writable(prototype.into(), false);
        descriptor.set_configurable(false);
        descriptor.set_enumerable(false);
        if !constructor
            .define_property(scope, v8str(scope, "prototype").into(), &descriptor)
            .unwrap_or(false)
        {
            return Err(anyhow!(
                "failed to install legacy factory `{}.prototype`",
                metadata.name
            ));
        }
        prototype
    } else {
        constructor_prototype_object(scope, constructor.into())
            .ok_or_else(|| anyhow!("constructor `{}` has no object prototype", metadata.name))?
    };

    if let Some((parent_constructor, parent_prototype)) = parent {
        if !constructor
            .set_prototype(scope, parent_constructor.into())
            .unwrap_or(false)
        {
            return Err(anyhow!(
                "failed to link `{}` constructor inheritance",
                metadata.name
            ));
        }
        if !constructor_prototype
            .set_prototype(scope, parent_prototype.into())
            .unwrap_or(false)
        {
            return Err(anyhow!(
                "failed to link `{}.prototype` inheritance",
                metadata.name
            ));
        }
    }

    let global = scope.get_current_context().global(scope);
    if !register_intrinsic_interface(
        scope,
        global,
        metadata.name,
        constructor.into(),
        constructor_prototype,
    ) {
        return Err(anyhow!(
            "failed to register intrinsic interface `{}`",
            metadata.name
        ));
    }
    let public_interface = match metadata.kind {
        ConstructorKind::HtmlElement => {
            let proxy =
                html_element_constructor_with_early_sanity_trap(scope, constructor, metadata.name)
                    .ok_or_else(|| {
                        anyhow!(
                            "failed to create HTML constructor sanity proxy for `{}`",
                            metadata.name
                        )
                    })?;
            set_interface_prototype_constructor(scope, constructor, proxy.into());
            v8::Local::<v8::Object>::from(proxy)
        }
        _ => constructor.into(),
    };
    if !register_public_interface_object(scope, global, metadata.name, public_interface) {
        return Err(anyhow!(
            "failed to register public interface object `{}`",
            metadata.name
        ));
    }
    realm.register_objects(
        scope,
        id,
        constructor.into(),
        constructor_prototype,
        public_interface,
    )?;
    realm.set_state(id, RealmInterfaceState::Finalizing)?;
    finalize_materialized_interface(scope, metadata.name, realm.realm_kind())?;
    realm.set_state(id, RealmInterfaceState::Ready)?;
    registry.record_materialization(id);
    Ok(public_interface.into())
}

fn intrinsic_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &Rc<ExposedInterfaceTemplateRegistry>,
    id: InterfaceId,
) -> Result<v8::Local<'s, v8::Object>> {
    let metadata = registry
        .metadata(id)
        .ok_or_else(|| anyhow!("unknown intrinsic prototype interface id {}", id.index()))?;
    let realm = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len())?;
    if realm.prototype(scope, id).is_none() {
        let _ = materialize_interface(scope, id)?;
    }
    realm
        .prototype(scope, id)
        .ok_or_else(|| anyhow!("missing intrinsic prototype `{}`", metadata.name))
}

fn intrinsic_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &Rc<ExposedInterfaceTemplateRegistry>,
    parent_id: InterfaceId,
) -> Result<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let parent = registry
        .metadata(parent_id)
        .ok_or_else(|| anyhow!("unknown parent interface id {}", parent_id.index()))?;
    let realm = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len())?;
    if realm.constructor(scope, parent_id).is_none() {
        if !registry.supports_interface(parent_id) {
            return Err(anyhow!(
                "eager parent intrinsic `{}` was not captured before lazy materialization",
                parent.name
            ));
        }
        let _ = materialize_interface(scope, parent_id)?;
    }

    let constructor = realm
        .constructor(scope, parent_id)
        .ok_or_else(|| anyhow!("missing intrinsic parent constructor `{}`", parent.name))?;
    let prototype = realm
        .prototype(scope, parent_id)
        .ok_or_else(|| anyhow!("missing intrinsic parent prototype `{}`", parent.name))?;
    Ok((constructor, prototype))
}
