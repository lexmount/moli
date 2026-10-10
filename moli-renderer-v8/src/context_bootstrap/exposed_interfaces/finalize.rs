use anyhow::{Context, Result, anyhow};

use super::metadata::RealmKind;
use crate::context_bootstrap::{
    crypto::finalize_crypto_realm_bindings, events::finalize_pointer_event_realm_bindings,
    notification_runtime::finalize_notification_realm_bindings,
    performance_runtime::finalize_performance_observer_realm_bindings,
    web_audio_runtime::finalize_base_audio_context_realm_bindings,
};
use crate::network_host::finalize_xml_http_request_event_target_realm_bindings;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealmDependentFinalizer {
    DocumentRealmBindings,
    NodeMixinUnscopables,
    GlobalEventHandlersSecureContextSurface,
    CryptoSecureContextSurface,
    BaseAudioContextSecureContextSurface,
    NavigatorSecureContextSurface,
    MediaElementSecureContextSurface,
    XmlHttpRequestEventTargetState,
    NotificationPermission,
    PointerEventSecureContextSurface,
    PerformanceWindowAccessors,
    PerformanceObserverSupportedEntryTypes,
}

const REALM_DEPENDENT_FINALIZER_ALLOWLIST: &[(&str, RealmDependentFinalizer)] = &[
    // The template creates @@unscopables, but its object needs a realm-local
    // null prototype after materialization.
    ("Document", RealmDependentFinalizer::DocumentRealmBindings),
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
        "Navigator",
        RealmDependentFinalizer::NavigatorSecureContextSurface,
    ),
    (
        "HTMLMediaElement",
        RealmDependentFinalizer::MediaElementSecureContextSurface,
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
        "HTMLElement",
        RealmDependentFinalizer::GlobalEventHandlersSecureContextSurface,
    ),
    (
        "SVGElement",
        RealmDependentFinalizer::GlobalEventHandlersSecureContextSurface,
    ),
    (
        "MathMLElement",
        RealmDependentFinalizer::GlobalEventHandlersSecureContextSurface,
    ),
    (
        "Performance",
        RealmDependentFinalizer::PerformanceWindowAccessors,
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

/// Completes bindings on unpublished realm-local objects. Taking the objects
/// directly avoids reentrant lookups of the interface being initialized.
///
/// The dispatch is deliberately interface-local: it must never scan public
/// global constructor properties, because reading one of those properties is
/// itself the lazy-materialization trigger. Context-independent declarations
/// should continue moving to the reusable FunctionTemplate installer.
pub(super) fn finalize_materialized_interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    interface_name: &str,
    constructor: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
    realm_kind: RealmKind,
) -> Result<()> {
    let Some(finalizer) = realm_dependent_finalizer(interface_name) else {
        return Ok(());
    };
    match finalizer {
        RealmDependentFinalizer::DocumentRealmBindings => {
            finalize_node_mixin_unscopables(scope, prototype)?;
            crate::context_bootstrap::window_events::finalize_secure_global_event_handler_realm_bindings(
                scope, prototype,
            )
        }
        RealmDependentFinalizer::NodeMixinUnscopables => {
            finalize_node_mixin_unscopables(scope, prototype)
        }
        RealmDependentFinalizer::BaseAudioContextSecureContextSurface => {
            finalize_base_audio_context_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::GlobalEventHandlersSecureContextSurface => {
            crate::context_bootstrap::window_events::finalize_secure_global_event_handler_realm_bindings(
                scope, prototype,
            )
        }
        RealmDependentFinalizer::NavigatorSecureContextSurface => {
            crate::context_bootstrap::navigator_runtime::finalize_navigator_realm_bindings(
                scope, prototype,
            )
        }
        RealmDependentFinalizer::MediaElementSecureContextSurface => {
            crate::context_bootstrap::navigator_runtime::finalize_media_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::CryptoSecureContextSurface => {
            finalize_crypto_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::XmlHttpRequestEventTargetState => {
            finalize_xml_http_request_event_target_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::NotificationPermission => {
            finalize_notification_realm_bindings(scope, constructor.into())
        }
        RealmDependentFinalizer::PointerEventSecureContextSurface => {
            finalize_pointer_event_realm_bindings(scope, prototype)
        }
        RealmDependentFinalizer::PerformanceWindowAccessors => {
            if realm_kind == RealmKind::Window {
                crate::context_bootstrap::performance_runtime::finalize_window_performance_realm_bindings(
                    scope, prototype,
                )
            } else {
                Ok(())
            }
        }
        RealmDependentFinalizer::PerformanceObserverSupportedEntryTypes => {
            let supported_entry_types = if realm_kind == RealmKind::Window {
                crate::context_bootstrap::performance_runtime::WINDOW_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES
            } else {
                crate::context_bootstrap::performance_runtime::WORKER_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES
            };
            finalize_performance_observer_realm_bindings(scope, constructor.into(), supported_entry_types)
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
                "Navigator",
                "HTMLMediaElement",
                "Crypto",
                "XMLHttpRequestEventTarget",
                "Notification",
                "PointerEvent",
                "HTMLElement",
                "SVGElement",
                "MathMLElement",
                "Performance",
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
