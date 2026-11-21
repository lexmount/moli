use std::rc::Rc;

use anyhow::{Result, anyhow};
use moli_v8_util::install_web_api_intrinsic_resolver;

use super::materialize::{exposed_interface_lazy_getter, resolve_web_api_intrinsic};
#[cfg(test)]
use super::metadata::STORAGE_INTERFACE_NAMES;
use super::metadata::{GlobalInstallation, RealmKind, TemplateBuildProfile};
use super::realm_registry::{IntrinsicInterfaceRegistry, RealmInterfaceEntry};
use super::template_registry::ExposedInterfaceTemplateRegistry;
use crate::context_bootstrap::specs::ConstructorSpec;
use crate::util::{
    constructor_object, constructor_prototype_object, initialize_ecmascript_intrinsic_registry,
    register_ecmascript_intrinsic, registered_ecmascript_constructor,
    registered_ecmascript_prototype, v8str,
};

const LEGACY_WINDOW_INTERFACE_ALIASES: &[(&str, &str)] = &[
    ("webkitURL", "URL"),
    ("SVGPoint", "DOMPoint"),
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
    let registry = worker_interface_template_registry(scope, realm_kind, specs)?;
    IntrinsicInterfaceRegistry::initialize_for_current_context(scope, registry.len(), realm_kind)?;
    // Worker eager objects must exist before the intrinsic resolver is enabled
    // by capture_eager_intrinsic_interfaces. Event constructors can run as soon
    // as they are exposed, including in embedders without that capture step.
    install_event_runtime(scope);

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

pub(in crate::context_bootstrap) fn prepare_worker_event_target_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    realm_kind: RealmKind,
    specs: Vec<ConstructorSpec>,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    let registry = worker_interface_template_registry(scope, realm_kind, specs)?;
    let event_target_id = registry
        .id_by_name("EventTarget")
        .ok_or_else(|| anyhow!("worker interface registry is missing EventTarget"))?;
    registry.get_or_build_template(scope, event_target_id)
}

fn worker_interface_template_registry<C>(
    scope: &mut v8::PinScope<'_, '_, C>,
    realm_kind: RealmKind,
    specs: Vec<ConstructorSpec>,
) -> Result<Rc<ExposedInterfaceTemplateRegistry>> {
    let expected_profile = TemplateBuildProfile::for_realm(realm_kind);
    if let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) {
        if registry.profile() != expected_profile {
            return Err(anyhow!(
                "worker interface registry profile mismatch: expected {expected_profile:?}, got {:?}",
                registry.profile()
            ));
        }
        return Ok(registry);
    }
    ExposedInterfaceTemplateRegistry::install(scope, specs, expected_profile)
}

pub(crate) fn filter_window_exposed_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    secure_context: bool,
) -> Result<()> {
    super::super::window_events::install_secure_window_event_handler_accessors(
        scope,
        global,
        secure_context,
    );
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
    crate::context_bootstrap::web_mcp::filter_exposure(scope, secure_context)?;
    Ok(())
}

/// Exposure is determined by the realm's native metadata, independently of
/// author changes to the corresponding global property.
pub(in crate::context_bootstrap) fn is_realm_interface_exposed(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> bool {
    let Some(registry) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return false;
    };
    let Ok(realm) = IntrinsicInterfaceRegistry::for_current_context(scope, registry.len()) else {
        return false;
    };
    let realm_kind = realm.realm_kind();
    let secure = match realm_kind {
        RealmKind::Window => {
            let global = scope.get_current_context().global(scope);
            crate::context_bootstrap::runtime_state::window_realm_secure_context_available(
                scope, global,
            )
        }
        RealmKind::DedicatedWorker | RealmKind::SharedWorker | RealmKind::ServiceWorker => {
            crate::worker::worker_realm_secure_context_available(scope)
        }
    };
    registry
        .id_by_name(name)
        .and_then(|id| registry.metadata(id))
        .is_some_and(|metadata| metadata.is_exposed(realm_kind, secure))
}

pub(crate) fn initialize_realm_interface_registry(
    scope: &mut v8::PinScope<'_, '_>,
    realm_kind: RealmKind,
) -> Result<()> {
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    IntrinsicInterfaceRegistry::initialize_for_current_context(scope, registry.len(), realm_kind)?;
    // Native values can be built during Window bootstrap, before eager
    // capture. Resolve through the realm cache from the start so an exception
    // never re-enters the Window's public lazy property or access checks.
    install_web_api_intrinsic_resolver(scope, resolve_web_api_intrinsic);
    install_event_runtime(scope);
    Ok(())
}

fn install_event_runtime(scope: &mut v8::PinScope<'_, '_>) {
    moli_webapi::event_target::install_runtime(
        scope,
        moli_webapi::event_target::EventTargetRuntime {
            initialize: crate::context_bootstrap::media_queries::initialize_simple_event_target,
            set_handler: crate::context_bootstrap::media_queries::simple_object_event_set_handler,
        },
    );
    crate::context_bootstrap::web_audio_runtime::install_audio_runtime(scope);
    moli_webapi::events::install_runtime(
        scope,
        moli_webapi::events::EventRuntime {
            timestamp: |scope| {
                crate::context_bootstrap::dom_time_since_origin_millis(
                    crate::context_bootstrap::current_performance_time_origin(scope),
                )
            },
            is_window_receiver: crate::context_bootstrap::is_window_receiver,
            initialize_wrapper: crate::context_bootstrap::events::initialize_event_wrapper,
        },
    );
}

/// Captures trusted eager constructor/prototype identities after realm
/// bootstrap and before author script can mutate the public global.
pub(crate) fn capture_eager_intrinsic_interfaces<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    realm_kind: RealmKind,
) -> Result<()> {
    install_event_runtime(scope);
    let registry = ExposedInterfaceTemplateRegistry::current(scope)
        .ok_or_else(|| anyhow!("exposed interface template registry is unavailable"))?;
    initialize_ecmascript_intrinsic_registry(scope, global);
    capture_ecmascript_intrinsic(scope, global, "Error")?;
    capture_ecmascript_intrinsic(scope, global, "Promise")?;
    let realm = IntrinsicInterfaceRegistry::initialize_for_current_context(
        scope,
        registry.len(),
        realm_kind,
    )?;

    for metadata in registry.metadata_entries() {
        match &*realm
            .entry(metadata.id)
            .ok_or_else(|| anyhow!("interface id is out of range"))?
        {
            RealmInterfaceEntry::Ready => continue,
            RealmInterfaceEntry::Failed | RealmInterfaceEntry::Materializing => {
                return Err(anyhow!(
                    "eager intrinsic `{}` is not ready for capture",
                    metadata.name
                ));
            }
            RealmInterfaceEntry::Uninitialized => {}
        }
        // Reading a public lazy property here would defeat lazy
        // materialization. Lazy callbacks publish their completed realm entry.
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
        realm.publish_ready(scope, metadata.id, constructor, prototype, constructor)?;
    }
    install_web_api_intrinsic_resolver(scope, resolve_web_api_intrinsic);
    moli_webapi::install_realm_callbacks(
        scope,
        moli_webapi::RealmCallbacks {
            point_mutated: crate::context_bootstrap::svg_runtime::reflect_svg_point_mutation,
            media_query_matches: crate::context_bootstrap::media_queries::read_media_query_matches,
            runtime: Some(&moli_webapi::RealmRuntime {
                abort_signal_owner: crate::abort_signal_route::resolve_abort_signal_owner,
                structured_clone_value: crate::context_bootstrap::structured_clone_value,
                owner_terminating: crate::worker::worker_termination_requested,
                trace_callback: |scope, callback| {
                    crate::v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction::new(
                        scope, callback,
                    )
                    .into_object()
                },
                invoke_traced_callback: crate::webidl_callback_execution::invoke_traced_callback,
                blob: crate::blob::RUNTIME,
                form_data: super::super::form_data_runtime::RUNTIME,
            }),
        },
    );
    Ok(())
}

pub(super) fn capture_ecmascript_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<()> {
    match (
        registered_ecmascript_constructor(scope, global, name),
        registered_ecmascript_prototype(scope, global, name),
    ) {
        (Some(_), Some(_)) => return Ok(()),
        (None, None) => {}
        _ => {
            return Err(anyhow!(
                "ECMAScript intrinsic `{name}` has partial registry state"
            ));
        }
    }
    let constructor = constructor_object(scope, global, name)
        .ok_or_else(|| anyhow!("missing ECMAScript intrinsic constructor `{name}`"))?;
    let prototype = constructor_prototype_object(scope, constructor)
        .ok_or_else(|| anyhow!("ECMAScript intrinsic `{name}` has no object prototype"))?;
    if !register_ecmascript_intrinsic(scope, global, name, constructor, prototype) {
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
pub(crate) fn realm_interface_is_materialized(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
) -> bool {
    let Some(templates) = ExposedInterfaceTemplateRegistry::current(scope) else {
        return false;
    };
    let Some(id) = templates.id_by_name(name) else {
        return false;
    };
    let Ok(realm) = IntrinsicInterfaceRegistry::for_current_context(scope, templates.len()) else {
        return false;
    };
    realm
        .entry(id)
        .is_some_and(|entry| matches!(*entry, RealmInterfaceEntry::Ready))
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
