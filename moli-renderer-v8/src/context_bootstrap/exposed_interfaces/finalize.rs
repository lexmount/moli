use anyhow::Result;

use super::materialize::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
};
use super::metadata::RealmKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealmDependentFinalizer {
    NodeMixinUnscopables,
    EventTargetBrand,
    DomExceptionPrototypeCache,
    CryptoSecureContextSurface,
    BlobPrototypeCache,
    XmlHttpRequestEventTargetState,
    NotificationPermission,
    PointerEventSecureContextSurface,
    PerformanceWindowAccessors,
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
    ("EventTarget", RealmDependentFinalizer::EventTargetBrand),
    (
        "DOMException",
        RealmDependentFinalizer::DomExceptionPrototypeCache,
    ),
    (
        "Crypto",
        RealmDependentFinalizer::CryptoSecureContextSurface,
    ),
    ("Blob", RealmDependentFinalizer::BlobPrototypeCache),
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

/// Completes bindings which still require a concrete realm-local prototype.
///
/// The dispatch is deliberately interface-local: it must never scan public
/// global constructor properties, because reading one of those properties is
/// itself the lazy-materialization trigger. Context-independent declarations
/// should continue moving to the reusable FunctionTemplate installer.
pub(super) fn finalize_materialized_interface(
    scope: &mut v8::PinScope<'_, '_>,
    interface_name: &str,
    realm_kind: RealmKind,
) -> Result<()> {
    let prototype = ensure_intrinsic_interface_prototype(scope, interface_name)?;

    let Some(finalizer) = realm_dependent_finalizer(interface_name) else {
        return Ok(());
    };
    match finalizer {
        RealmDependentFinalizer::NodeMixinUnscopables => {
            finalize_node_mixin_unscopables(scope, prototype);
        }
        RealmDependentFinalizer::EventTargetBrand => {
            crate::context_bootstrap::event_template::mark_event_target_interface_prototype(
                scope, prototype,
            );
        }
        RealmDependentFinalizer::DomExceptionPrototypeCache => {
            crate::context_bootstrap::constructors::finalize_dom_exception_realm_bindings(
                scope, prototype,
            );
        }
        RealmDependentFinalizer::CryptoSecureContextSurface => {
            crate::context_bootstrap::crypto::finalize_crypto_realm_bindings(scope, prototype)?;
        }
        RealmDependentFinalizer::BlobPrototypeCache => {
            crate::blob::finalize_blob_realm_bindings(scope, prototype);
        }
        RealmDependentFinalizer::XmlHttpRequestEventTargetState => {
            crate::network_host::finalize_xml_http_request_event_target_realm_bindings(
                scope, prototype,
            );
        }
        RealmDependentFinalizer::NotificationPermission => {
            crate::context_bootstrap::notification_runtime::install_notification_realm_bindings(
                scope,
            );
        }
        RealmDependentFinalizer::PointerEventSecureContextSurface => {
            crate::context_bootstrap::events::finalize_pointer_event_realm_bindings(
                scope, prototype,
            )?;
        }
        RealmDependentFinalizer::PerformanceWindowAccessors => {
            if realm_kind == RealmKind::Window {
                crate::context_bootstrap::performance_runtime::finalize_window_performance_realm_bindings(
                    scope, prototype,
                )?;
            }
        }
        RealmDependentFinalizer::PerformanceObserverSupportedEntryTypes => {
            let constructor = ensure_intrinsic_interface_constructor(scope, "PerformanceObserver")?;
            let supported_entry_types = if realm_kind == RealmKind::Window {
                crate::context_bootstrap::performance_runtime::WINDOW_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES
            } else {
                crate::context_bootstrap::performance_runtime::WORKER_PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES
            };
            crate::context_bootstrap::performance_runtime::finalize_performance_observer_realm_bindings(
                scope,
                constructor.into(),
                supported_entry_types,
            );
        }
    }
    Ok(())
}

fn finalize_node_mixin_unscopables(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
) {
    let Some(unscopables) = prototype
        .get(scope, v8::Symbol::get_unscopables(scope).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let _ = unscopables.set_prototype(scope, v8::null(scope).into());
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
                "EventTarget",
                "DOMException",
                "Crypto",
                "Blob",
                "XMLHttpRequestEventTarget",
                "Notification",
                "PointerEvent",
                "Performance",
                "PerformanceObserver",
            ]
        );
        for static_owner in [
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
