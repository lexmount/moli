use anyhow::{Context, Result, anyhow};

use super::materialize::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
};
use crate::context_bootstrap::{
    crypto::finalize_crypto_realm_bindings, events::finalize_pointer_event_realm_bindings,
    notification_runtime::finalize_notification_realm_bindings,
    performance_runtime::finalize_performance_observer_realm_bindings,
    web_audio_runtime::finalize_base_audio_context_realm_bindings,
};
use crate::network_host::finalize_xml_http_request_event_target_realm_bindings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealmDependentFinalizer {
    NodeMixinUnscopables,
    CryptoSecureContextSurface,
    BaseAudioContextSecureContextSurface,
    XmlHttpRequestEventTargetState,
    NotificationPermission,
    PointerEventSecureContextSurface,
    PerformanceObserverSupportedEntryTypes,
}

const REALM_DEPENDENT_FINALIZER_ALLOWLIST: &[(&str, RealmDependentFinalizer)] = &[
    // The template creates @@unscopables, but its object needs a realm-local
    // null prototype after materialization.
    ("Document", RealmDependentFinalizer::NodeMixinUnscopables),
    (
        "DocumentFragment",
        RealmDependentFinalizer::NodeMixinUnscopables,
    ),
    ("Element", RealmDependentFinalizer::NodeMixinUnscopables),
    (
        "DocumentType",
        RealmDependentFinalizer::NodeMixinUnscopables,
    ),
    (
        "CharacterData",
        RealmDependentFinalizer::NodeMixinUnscopables,
    ),
    // These entries install private realm state, context-conditional surface,
    // or constructor data whose JavaScript identity must be realm-local.
    (
        "BaseAudioContext",
        RealmDependentFinalizer::BaseAudioContextSecureContextSurface,
    ),
    (
        "Crypto",
        RealmDependentFinalizer::CryptoSecureContextSurface,
    ),
    (
        "XMLHttpRequestEventTarget",
        RealmDependentFinalizer::XmlHttpRequestEventTargetState,
    ),
    (
        "Notification",
        RealmDependentFinalizer::NotificationPermission,
    ),
    (
        "PointerEvent",
        RealmDependentFinalizer::PointerEventSecureContextSurface,
    ),
    (
        "PerformanceObserver",
        RealmDependentFinalizer::PerformanceObserverSupportedEntryTypes,
    ),
];

fn realm_dependent_finalizer(interface_name: &str) -> Option<RealmDependentFinalizer> {
    REALM_DEPENDENT_FINALIZER_ALLOWLIST
        .iter()
        .find_map(|(name, finalizer)| (*name == interface_name).then_some(*finalizer))
}

/// Completes bindings which still require a concrete realm-local prototype.
///
/// The dispatch is deliberately interface-local: it must never scan public
/// global constructor properties, because reading one of those properties is
/// itself the lazy-materialization trigger. Context-independent declarations
/// should continue moving to the reusable FunctionTemplate installer.
pub(super) fn finalize_materialized_interface(
    scope: &mut v8::PinScope<'_, '_>,
    interface_name: &str,
) -> Result<()> {
    let Some(finalizer) = realm_dependent_finalizer(interface_name) else {
        return Ok(());
    };
    let prototype = ensure_intrinsic_interface_prototype(scope, interface_name)?;
    match finalizer {
        RealmDependentFinalizer::NodeMixinUnscopables => {
            finalize_node_mixin_unscopables(scope, prototype)
        }
        RealmDependentFinalizer::BaseAudioContextSecureContextSurface => {
            finalize_base_audio_context_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::CryptoSecureContextSurface => {
            finalize_crypto_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::XmlHttpRequestEventTargetState => {
            finalize_xml_http_request_event_target_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::NotificationPermission => {
            let constructor = ensure_intrinsic_interface_constructor(scope, "Notification")?;
            finalize_notification_realm_bindings(scope, constructor.into())
        }
        RealmDependentFinalizer::PointerEventSecureContextSurface => {
            finalize_pointer_event_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::PerformanceObserverSupportedEntryTypes => {
            let constructor = ensure_intrinsic_interface_constructor(scope, "PerformanceObserver")?;
            finalize_performance_observer_realm_bindings(scope, constructor.into())
        }
    }
    .with_context(|| format!("failed to finalize intrinsic interface `{interface_name}`"))
}

fn finalize_node_mixin_unscopables(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
) -> Result<()> {
    let unscopables = prototype
        .get(scope, v8::Symbol::get_unscopables(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("intrinsic @@unscopables object is unavailable"))?;
    if unscopables.set_prototype(scope, v8::null(scope).into()) != Some(true) {
        return Err(anyhow!(
            "failed to set intrinsic @@unscopables null prototype"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_dependent_finalizer_allowlist_excludes_static_member_owners() {
        let names = REALM_DEPENDENT_FINALIZER_ALLOWLIST
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "Document",
                "DocumentFragment",
                "Element",
                "DocumentType",
                "CharacterData",
                "BaseAudioContext",
                "Crypto",
                "XMLHttpRequestEventTarget",
                "Notification",
                "PointerEvent",
                "PerformanceObserver",
            ]
        );
        for static_owner in [
            "EventTarget",
            "Node",
            "HTMLAnchorElement",
            "HTMLAreaElement",
            "ElementInternals",
            "NavigationTransition",
            "XMLHttpRequest",
        ] {
            assert_eq!(realm_dependent_finalizer(static_owner), None);
        }
    }
}
