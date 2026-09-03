use std::rc::Rc;

use anyhow::{Result, anyhow};

use super::materialize::exposed_interface_lazy_getter;
#[cfg(test)]
use super::metadata::STORAGE_INTERFACE_NAMES;
use super::metadata::{GlobalInstallation, RealmKind, TemplateBuildProfile};
use super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceState};
use super::template_registry::ExposedInterfaceTemplateRegistry;
use crate::context_bootstrap::specs::ConstructorSpec;
use crate::util::{
    constructor_object, constructor_prototype_object, initialize_intrinsic_interface_registry,
    register_intrinsic_interface, register_public_interface_object,
    registered_intrinsic_constructor, registered_intrinsic_prototype,
    registered_public_interface_object, v8str,
};

const LEGACY_WINDOW_INTERFACE_ALIASES: &[(&str, &str)] = &[
    ("SVGPoint", "DOMPoint"),
    ("SVGRect", "DOMRect"),
    ("SVGMatrix", "DOMMatrix"),
    ("webkitAudioContext", "AudioContext"),
    ("WebKitCSSMatrix", "DOMMatrix"),
];

pub(in crate::context_bootstrap) fn install_window_exposed_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    global_template: v8::Local<'s, v8::ObjectTemplate>,
    registry: &Rc<ExposedInterfaceTemplateRegistry>,
) -> Result<()> {
    for metadata in registry.metadata_entries() {
        if metadata.installation != GlobalInstallation::Lazy
            || !metadata.exposure.contains(RealmKind::Window)
        {
            continue;
        }
        let data = v8::Integer::new_from_unsigned(scope, metadata.id.callback_data());
        global_template.set_lazy_data_property_with_configuration(
            v8str(scope, metadata.name).into(),
            v8::LazyDataPropertyConfiguration::new(exposed_interface_lazy_getter)
                .data(data.into())
                .property_attribute(v8::PropertyAttribute::DONT_ENUM)
                .getter_side_effect_type(v8::SideEffectType::HasNoSideEffect),
        );
    }
    for &(alias, interface) in LEGACY_WINDOW_INTERFACE_ALIASES {
        let Some(interface_id) = registry.id_by_name(interface) else {
            continue;
        };
        let data = v8::Integer::new_from_unsigned(scope, interface_id.callback_data());
        global_template.set_lazy_data_property_with_configuration(
            v8str(scope, alias).into(),
            v8::LazyDataPropertyConfiguration::new(exposed_interface_lazy_getter)
                .data(data.into())
                .property_attribute(v8::PropertyAttribute::DONT_ENUM)
                .getter_side_effect_type(v8::SideEffectType::HasNoSideEffect),
        );
    }
    Ok(())
}

pub(in crate::context_bootstrap) fn install_worker_exposed_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    realm_kind: RealmKind,
    secure_context: bool,
    specs: Vec<ConstructorSpec>,
) -> Result<()> {
    let registry = ExposedInterfaceTemplateRegistry::install(
        scope,
        specs,
        TemplateBuildProfile::for_realm(realm_kind),
    )?;
    IntrinsicInterfaceRegistry::initialize_for_current_context(scope, registry.len(), realm_kind)?;

    for metadata in registry.metadata_entries() {
        if metadata.installation != GlobalInstallation::Lazy
            || !metadata.is_exposed(realm_kind, secure_context)
            || !registry.supports_interface(metadata.id)
        {
            continue;
        }
        let data = v8::Integer::new_from_unsigned(scope, metadata.id.callback_data());
        global
            .set_lazy_data_property_with_configuration(
                scope,
                v8str(scope, metadata.name).into(),
                v8::LazyDataPropertyConfiguration::new(exposed_interface_lazy_getter)
                    .data(data.into())
                    .property_attribute(v8::PropertyAttribute::DONT_ENUM)
                    .getter_side_effect_type(v8::SideEffectType::HasNoSideEffect),
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| {
                anyhow!(
                    "failed to install lazy worker interface `{}`",
                    metadata.name
                )
            })?;
    }
    Ok(())
}

pub(crate) fn filter_window_exposed_interfaces(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    secure_context: bool,
) -> Result<()> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    for metadata in registry.metadata_entries() {
        if metadata.installation == GlobalInstallation::Lazy
            && !metadata.is_exposed(RealmKind::Window, secure_context)
        {
            global
                .delete(scope, v8str(scope, metadata.name).into())
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| {
                    anyhow!(
                        "failed to remove unexposed window interface `{}`",
                        metadata.name
                    )
                })?;
        }
    }
    Ok(())
}

pub(crate) fn initialize_realm_interface_registry(
    scope: &mut v8::PinScope<'_, '_>,
    realm_kind: RealmKind,
) -> Result<()> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    IntrinsicInterfaceRegistry::initialize_for_current_context(scope, registry.len(), realm_kind)?;
    Ok(())
}

/// Captures trusted eager constructor/prototype identities after realm
/// bootstrap and before author script can mutate the public global.
pub(crate) fn capture_eager_intrinsic_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    realm_kind: RealmKind,
) -> Result<()> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    initialize_intrinsic_interface_registry(scope, global);
    capture_ecmascript_intrinsic(scope, global, "Error")?;
    let realm = IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        realm_kind,
    )?;

    for metadata in registry.metadata_entries() {
        let captured_constructor = registered_intrinsic_constructor(scope, global, metadata.name);
        let captured_prototype = registered_intrinsic_prototype(scope, global, metadata.name);
        if captured_constructor.is_some() != captured_prototype.is_some() {
            return Err(anyhow!(
                "eager intrinsic `{}` has partial constructor/prototype state",
                metadata.name
            ));
        }
        if let Some(constructor) = captured_constructor {
            let prototype = captured_prototype.expect("captured intrinsic pair was validated");
            if registered_public_interface_object(scope, global, metadata.name).is_none()
                && !register_public_interface_object(scope, global, metadata.name, constructor)
            {
                return Err(anyhow!(
                    "failed to capture eager public interface `{}`",
                    metadata.name
                ));
            }
            let public_interface = registered_public_interface_object(scope, global, metadata.name)
                .ok_or_else(|| {
                    anyhow!("captured public interface `{}` is missing", metadata.name)
                })?;
            realm.register_objects(scope, metadata.id, constructor, prototype, public_interface)?;
            realm.set_state(metadata.id, RealmInterfaceState::Ready)?;
            continue;
        }
        // Reading a public lazy property here would defeat lazy
        // materialization. Lazy callbacks register their own intrinsic pair.
        //
        // Worker bootstrap still constructs interfaces outside the shared
        // registry in a few cohorts. Those entries have metadata, but no
        // registry template in this isolate, so their already-created public
        // constructors must be captured before author script can replace
        // them.
        if metadata.installation == GlobalInstallation::Lazy
            && registry.supports_interface(metadata.id)
        {
            continue;
        }
        let Some(constructor) = constructor_object(scope, global, metadata.name) else {
            continue;
        };
        let Some(prototype) = constructor_prototype_object(scope, constructor) else {
            continue;
        };
        if !register_intrinsic_interface(scope, global, metadata.name, constructor, prototype) {
            return Err(anyhow!(
                "failed to capture eager intrinsic `{}`",
                metadata.name
            ));
        }
        if !register_public_interface_object(scope, global, metadata.name, constructor) {
            return Err(anyhow!(
                "failed to capture eager public interface `{}`",
                metadata.name
            ));
        }
        realm.register_objects(scope, metadata.id, constructor, prototype, constructor)?;
        realm.set_state(metadata.id, RealmInterfaceState::Ready)?;
    }
    Ok(())
}

fn capture_ecmascript_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<()> {
    if registered_intrinsic_constructor(scope, global, name).is_some() {
        return Ok(());
    }
    let constructor = constructor_object(scope, global, name)
        .ok_or_else(|| anyhow!("missing ECMAScript intrinsic constructor `{name}`"))?;
    let prototype = constructor_prototype_object(scope, constructor)
        .ok_or_else(|| anyhow!("ECMAScript intrinsic `{name}` has no object prototype"))?;
    if !register_intrinsic_interface(scope, global, name, constructor, prototype) {
        return Err(anyhow!("failed to capture ECMAScript intrinsic `{name}`"));
    }
    Ok(())
}

pub(in crate::context_bootstrap) fn is_lazy_exposed_interface(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> bool {
    ExposedInterfaceTemplateRegistry::current(scope)
        .is_some_and(|registry| registry.is_lazy_name(name))
}

pub(in crate::context_bootstrap) fn install_interface_template_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &'static str,
) {
    moli_webapi_declare::mark_web_api_platform_object_template_instances(scope, template);
    if name == "WebSocket" {
        // WebSocket exposes a legacy dynamic @@toStringTag accessor which
        // distinguishes the prototype object from ordinary instances.
        return;
    }
    let prototype = template.prototype_template(scope);
    prototype.set_with_attr(
        v8::Symbol::get_to_string_tag(scope).into(),
        v8str(scope, name).into(),
        v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
    );
}

#[cfg(test)]
pub(crate) fn interface_materialization_count(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> usize {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return 0;
    };
    registry
        .id_by_name(name)
        .map_or(0, |id| registry.materialization_count(id))
}

#[cfg(test)]
pub(crate) fn interface_template_build_count(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> usize {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return 0;
    };
    registry
        .id_by_name(name)
        .map_or(0, |id| registry.build_count(id))
}

#[cfg(test)]
pub(crate) fn ready_interface_template_names(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<&'static str> {
    ExposedInterfaceTemplateRegistry::current(scope)
        .map_or_else(Vec::new, |registry| registry.ready_template_names())
}

#[cfg(test)]
pub(crate) fn storage_interface_materialization_count(scope: &mut v8::PinScope<'_, '_>) -> usize {
    STORAGE_INTERFACE_NAMES
        .iter()
        .map(|name| interface_materialization_count(scope, name))
        .sum()
}

#[cfg(test)]
pub(crate) fn materialized_interface_names(
    scope: &mut v8::PinScope<'_, '_>,
) -> Vec<(&'static str, usize)> {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return Vec::new();
    };
    registry
        .metadata_entries()
        .iter()
        .filter_map(|metadata| {
            let count = registry.materialization_count(metadata.id);
            (count != 0).then_some((metadata.name, count))
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn lazy_window_interface_names(scope: &mut v8::PinScope<'_, '_>) -> Vec<&'static str> {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return Vec::new();
    };
    registry
        .metadata_entries()
        .iter()
        .filter(|metadata| {
            metadata.installation == GlobalInstallation::Lazy
                && metadata.is_exposed(RealmKind::Window, true)
                && !LEGACY_WINDOW_INTERFACE_ALIASES
                    .iter()
                    .any(|(alias, _)| *alias == metadata.name)
        })
        .map(|metadata| metadata.name)
        .collect()
}
