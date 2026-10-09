use std::rc::Rc;

use anyhow::{Result, anyhow};
use moli_v8_util::{WebApiIntrinsicKind, WebApiIntrinsicLookup};

use super::finalize::finalize_materialized_interface;
use super::metadata::{InterfaceId, ResolvedPrototypeProperty};
use super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceEntry};
use super::template_registry::ExposedInterfaceTemplateRegistry;
use crate::context_bootstrap::constructors::html_element_constructor_with_early_sanity_trap;
use crate::context_bootstrap::runtime_state::set_interface_prototype_constructor;
use crate::context_bootstrap::shared::throw_error;
use crate::context_bootstrap::specs::ConstructorKind;
use crate::util::{constructor_prototype_object, v8str};

/// Adapter for shared declaration helpers. Recognized Web APIs always resolve
/// through the realm entry, even when a public binding was deleted or replaced.
pub(super) fn resolve_web_api_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    kind: WebApiIntrinsicKind,
) -> WebApiIntrinsicLookup<'s> {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return WebApiIntrinsicLookup::Unmanaged;
    };
    if registry.id_by_name(name).is_none() {
        return WebApiIntrinsicLookup::Unmanaged;
    }
    WebApiIntrinsicLookup::Managed(match kind {
        WebApiIntrinsicKind::Constructor => ensure_intrinsic_interface_constructor(scope, name)
            .ok()
            .map(Into::into),
        WebApiIntrinsicKind::Prototype => ensure_intrinsic_interface_prototype(scope, name).ok(),
    })
}

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

    match &*realm
        .entry(id)
        .ok_or_else(|| anyhow!("interface state is out of range"))?
    {
        RealmInterfaceEntry::Ready => {
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
        RealmInterfaceEntry::Materializing => {
            return Err(anyhow!("materialization cycle reached `{}`", metadata.name));
        }
        RealmInterfaceEntry::Failed => {
            return Err(anyhow!(
                "a previous materialization of `{}` failed",
                metadata.name
            ));
        }
        RealmInterfaceEntry::Uninitialized => {}
    }

    realm.begin_materialization(id)?;
    match materialize_uninitialized_interface(scope, &registry, &realm, id) {
        Ok(interface) => Ok(interface),
        Err(error) => {
            realm.fail(id)?;
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
        // Window's immutable chain includes its named properties object between
        // Window.prototype and EventTarget.prototype, installed by the templates.
        if metadata.name != "Window"
            && !constructor_prototype
                .set_prototype(scope, parent_prototype.into())
                .unwrap_or(false)
        {
            return Err(anyhow!(
                "failed to link `{}.prototype` inheritance",
                metadata.name
            ));
        }
    }

    finish_materialized_interface(
        scope,
        registry,
        realm,
        id,
        constructor,
        constructor_prototype,
    )
}

fn finish_materialized_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &Rc<ExposedInterfaceTemplateRegistry>,
    realm: &Rc<IntrinsicInterfaceRegistry>,
    id: InterfaceId,
    constructor: v8::Local<'s, v8::Function>,
    constructor_prototype: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Value>> {
    let metadata = registry
        .metadata(id)
        .ok_or_else(|| anyhow!("unknown exposed interface id {}", id.index()))?;
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
            set_interface_prototype_constructor(scope, constructor_prototype, proxy.into())?;
            v8::Local::<v8::Object>::from(proxy)
        }
        ConstructorKind::DomParser
        | ConstructorKind::VideoDecoder
        | ConstructorKind::VideoEncoder
        | ConstructorKind::VideoColorSpace
        | ConstructorKind::MediaMetadata
        | ConstructorKind::RtcIceCandidate
        | ConstructorKind::RtcSessionDescription
        | ConstructorKind::RtcError
        | ConstructorKind::RtcErrorEvent
        | ConstructorKind::RtcDataChannelEvent
        | ConstructorKind::RtcPeerConnectionIceEvent
        | ConstructorKind::RtcPeerConnectionIceErrorEvent
        | ConstructorKind::Image
        | ConstructorKind::Audio
        | ConstructorKind::Option => {
            let proxy = moli_webapi_declare::web_api_constructor_with_deferred_prototype(
                scope,
                constructor,
            )
            .ok_or_else(|| {
                anyhow!(
                    "failed to create WebIDL constructor entry for `{}`",
                    metadata.name
                )
            })?;
            if metadata.prototype_property == ResolvedPrototypeProperty::TemplateReadOnly {
                set_interface_prototype_constructor(scope, constructor_prototype, proxy.into())?;
            }
            proxy.into()
        }
        _ => constructor.into(),
    };
    // Like Blink's ConstructorForTypeSlowCase, finish every fallible binding
    // installation before publishing the interface in the per-context cache.
    finalize_materialized_interface(
        scope,
        metadata.name,
        constructor,
        constructor_prototype,
        realm.realm_kind(),
    )?;
    realm.publish_ready(
        scope,
        id,
        constructor.into(),
        constructor_prototype,
        public_interface,
    )?;
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
