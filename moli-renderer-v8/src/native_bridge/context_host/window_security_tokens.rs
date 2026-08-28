use super::{
    JsContextHost, OwnerDispatchScope, RuntimeObservableContextToken,
    WindowExecutionContextIdentity, WindowExecutionContextOwner,
};
use crate::{browsing_context_model::BrowsingContextAccessOrigin, document_runtime::DomHandle};
use moli_storage_key::OpaqueOriginNonce;
use std::rc::Rc;

const WINDOW_SECURITY_TOKEN_PREFIX: &str = "moli-window-origin-v1:";
const WINDOW_ISOLATED_WORLD_SECURITY_TOKEN_PREFIX: &str = "moli-window-isolated-origin-v1:";

/// Immutable security state retained by a published V8 realm after its
/// active execution-context registration is retired.
///
/// This slot deliberately contains no V8 handles and grants no scheduling or
/// WebAPI authority. Its only consumer is V8's synchronous WindowProxy access
/// callback, where an old author-retained closure must keep the origin of the
/// Document that created it rather than inheriting the replacement frame's
/// current origin.
#[derive(Clone, Debug, Eq, PartialEq)]
struct WindowAccessCheckPrincipal {
    identity: WindowExecutionContextIdentity,
    origin: WindowAccessOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowAccessCheckPrincipalState {
    Current,
    DetachedDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolvedWindowAccessCheckPrincipal {
    principal: WindowAccessCheckPrincipal,
    state: WindowAccessCheckPrincipalState,
}

impl JsContextHost {
    /// Publishes the passive access-check principal for one concrete realm.
    ///
    /// The active registry remains authoritative while the realm is current.
    /// The Context slot becomes authoritative only after that registry entry
    /// has been retired and the Context has entered detached-Document state.
    pub(crate) fn install_window_access_check_principal_for_context(
        &self,
        context: v8::Local<'_, v8::Context>,
        identity: WindowExecutionContextIdentity,
    ) -> bool {
        let Some(context_token) = context
            .get_slot::<RuntimeObservableContextToken>()
            .as_deref()
            .copied()
        else {
            return false;
        };
        if context_token != identity.realm_token()
            || !self.window_execution_context_identity_is_current(identity)
        {
            return false;
        }
        let Some(origin) = self.window_access_origin(identity) else {
            return false;
        };
        if context
            .get_slot::<WindowAccessCheckPrincipal>()
            .is_some_and(|previous| previous.identity != identity)
        {
            tracing::warn!(
                ?identity,
                "refused to rebind a V8 realm's immutable Window access-check identity"
            );
            return false;
        }
        let _previous = context.set_slot(Rc::new(WindowAccessCheckPrincipal { identity, origin }));
        true
    }

    /// Resolves security-only realm state without invoking a V8 property API.
    /// V8 calls this while `MayAccess` is already active, so looking through a
    /// WindowProxy or private property here would recursively enter the access
    /// callback.
    fn resolve_window_access_check_principal(
        &self,
        context: v8::Local<'_, v8::Context>,
    ) -> Option<ResolvedWindowAccessCheckPrincipal> {
        let realm_token = context
            .get_slot::<RuntimeObservableContextToken>()
            .as_deref()
            .copied()?;
        if let Some(registered) = self
            .window_execution_context_realms
            .concrete_registration(realm_token)
        {
            let registration = registered.registration;
            let identity = WindowExecutionContextIdentity::new(
                registration.owner,
                registered.dispatch_scope,
                realm_token,
                registration.access_policy,
            );
            if !self.window_execution_context_identity_is_current(identity) {
                return None;
            }
            return Some(ResolvedWindowAccessCheckPrincipal {
                principal: WindowAccessCheckPrincipal {
                    identity,
                    origin: self.window_access_origin(identity)?,
                },
                state: WindowAccessCheckPrincipalState::Current,
            });
        }
        if !crate::util::page_context_is_detached_document(context) {
            return None;
        }
        let principal = context
            .get_slot::<WindowAccessCheckPrincipal>()
            .as_deref()
            .cloned()?;
        (principal.identity.realm_token() == realm_token).then_some(
            ResolvedWindowAccessCheckPrincipal {
                principal,
                state: WindowAccessCheckPrincipalState::DetachedDocument,
            },
        )
    }

    /// Applies the WindowProxy security check between two concrete V8 realms.
    /// A detached realm may be the observer, but the target must still be a
    /// current LocalWindow realm. This keeps passive origin compatibility
    /// separate from every operation-admission/currentness path.
    pub(crate) fn window_realms_can_access(
        &self,
        accessing_context: v8::Local<'_, v8::Context>,
        accessed_host: &Self,
        accessed_context: v8::Local<'_, v8::Context>,
    ) -> bool {
        let Some(accessing) = self.resolve_window_access_check_principal(accessing_context) else {
            return false;
        };
        let Some(accessed) = accessed_host.resolve_window_access_check_principal(accessed_context)
        else {
            return false;
        };
        if accessed.state != WindowAccessCheckPrincipalState::Current {
            return false;
        }
        if !std::ptr::eq(self, accessed_host) && !self.shares_page_script_agent_with(accessed_host)
        {
            return false;
        }
        if accessing.principal.identity.grants_universal_access() {
            return true;
        }
        related_page_window_origins_can_access(
            &accessing.principal.origin,
            &accessed.principal.origin,
        )
    }

    pub(crate) fn main_default_world_security_token_key(&self) -> Option<String> {
        let origin = moli_url::origin_ascii_serialization(self.document_url());
        if self.document_domain_override.is_some() {
            return None;
        }
        window_security_token_key(origin)
    }

    pub(crate) fn child_default_world_security_token_key(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        if self.child_browsing_context_has_opaque_origin(handle) {
            return None;
        }
        let origin = self.child_browsing_context_window_origin(handle)?;
        if self
            .child_effective_origin_document_domain_override(handle)?
            .is_some()
        {
            return None;
        }
        window_security_token_key(origin)
    }

    pub(crate) fn main_isolated_world_security_token_key(&self) -> Option<String> {
        window_isolated_world_security_token_key(
            moli_url::origin_ascii_serialization(self.document_url()),
            self.document_domain_override.is_some(),
        )
    }

    pub(crate) fn child_isolated_world_security_token_key(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        if self.child_browsing_context_has_opaque_origin(handle) {
            return None;
        }
        let origin = self.child_browsing_context_window_origin(handle)?;
        window_isolated_world_security_token_key(
            origin,
            self.child_browsing_context_document_domain_override(handle)
                .is_some(),
        )
    }

    pub(crate) fn refresh_security_tokens_after_document_domain_mutation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document_handle: DomHandle,
    ) -> usize {
        let target_child = self.child_browsing_context_handle_for_stored_document(document_handle);
        let target_is_main = document_handle == self.document_handle();
        if !target_is_main && target_child.is_none() {
            return 0;
        }

        let mut dispatch_scopes = vec![OwnerDispatchScope::Top];
        dispatch_scopes.extend(
            self.child_browsing_context_handles_in_document_order()
                .into_iter()
                .map(OwnerDispatchScope::Child),
        );

        let mut updated = 0;
        for dispatch_scope in dispatch_scopes {
            let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
                continue;
            };
            let Some((_, context)) = self.window_execution_context(scope, owner, dispatch_scope)
            else {
                continue;
            };
            let key = match dispatch_scope {
                OwnerDispatchScope::Top => self.main_default_world_security_token_key(),
                OwnerDispatchScope::Child(handle) => {
                    self.child_default_world_security_token_key(handle)
                }
            };
            if set_window_security_token(scope, context, key.as_deref()) {
                let principal_updated = self
                    .current_registered_window_execution_context_identity(dispatch_scope)
                    .is_some_and(|identity| {
                        self.install_window_access_check_principal_for_context(context, identity)
                    });
                debug_assert!(
                    principal_updated,
                    "a current default Window realm must refresh its access-check principal"
                );
                updated += 1;
            }
        }
        self.refresh_child_window_access_surfaces_after_origin_mutation(scope);
        updated
    }

    pub(crate) fn refresh_child_default_world_security_token(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) -> bool {
        let dispatch_scope = OwnerDispatchScope::Child(handle);
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return false;
        };
        let Some((_, context)) = self.window_execution_context(scope, owner, dispatch_scope) else {
            return false;
        };
        if !set_window_security_token(
            scope,
            context,
            self.child_default_world_security_token_key(handle)
                .as_deref(),
        ) {
            return false;
        }
        self.current_registered_window_execution_context_identity(dispatch_scope)
            .is_some_and(|identity| {
                self.install_window_access_check_principal_for_context(context, identity)
            })
    }

    fn child_effective_origin_document_domain_override(
        &self,
        handle: DomHandle,
    ) -> Option<Option<String>> {
        self.child_browsing_contexts.get(&handle)?;
        Some(self.child_browsing_context_document_domain_override(handle))
    }

    pub(in crate::native_bridge::context_host) fn top_window_can_access_child(
        &self,
        handle: DomHandle,
    ) -> bool {
        let Some(top) = self.main_window_access_origin() else {
            return false;
        };
        let Some(child) = self.child_window_access_origin(handle) else {
            return false;
        };
        top.can_access(&child)
    }

    /// Checks an observer Realm against a target scope in another related
    /// Page before that target Realm necessarily exists.
    pub(crate) fn window_execution_context_can_access_related_page_dispatch_scope(
        &self,
        accessing: WindowExecutionContextIdentity,
        accessed_host: &Self,
        accessed_scope: OwnerDispatchScope,
    ) -> bool {
        if !self.window_execution_context_identity_is_current(accessing)
            || !self.shares_related_page_script_agent_with(accessed_host)
        {
            return false;
        }
        if accessing.grants_universal_access() {
            return true;
        }
        let Some(accessing_origin) = self.window_access_origin(accessing) else {
            return false;
        };
        let Some(accessed_origin) =
            accessed_host.window_access_origin_for_dispatch_scope(accessed_scope)
        else {
            return false;
        };
        related_page_window_origins_can_access(&accessing_origin, &accessed_origin)
    }

    /// Checks Window access before the target realm is entered or materialized.
    ///
    /// WebIDL operations such as a borrowed `fetch()` must authorize the
    /// receiver while still in the accessing realm. Resolving only the target
    /// V8 context first would let cross-origin callers bypass the WindowProxy
    /// boundary by entering that context directly.
    pub(crate) fn window_execution_context_can_access_dispatch_scope(
        &self,
        accessing: WindowExecutionContextIdentity,
        accessed_scope: OwnerDispatchScope,
    ) -> bool {
        if !self.window_execution_context_identity_is_current(accessing) {
            return false;
        }
        let Some(accessed_owner) = self.current_window_execution_context_owner(accessed_scope)
        else {
            return false;
        };
        if accessing.grants_universal_access() {
            return true;
        }
        if accessing.owner() == accessed_owner {
            return true;
        }
        let Some(accessing_origin) = self.window_access_origin(accessing) else {
            return false;
        };
        let Some(accessed_origin) = self.window_access_origin_for_dispatch_scope(accessed_scope)
        else {
            return false;
        };
        accessing_origin.can_access(&accessed_origin)
    }

    fn window_access_origin(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<WindowAccessOrigin> {
        match (identity.owner(), identity.dispatch_scope()) {
            (WindowExecutionContextOwner::Frame(_), OwnerDispatchScope::Top) => {
                self.main_window_access_origin()
            }
            (WindowExecutionContextOwner::Frame(_), OwnerDispatchScope::Child(child_handle)) => {
                self.child_window_access_origin(child_handle)
            }
        }
    }

    pub(in crate::native_bridge::context_host) fn window_access_origin_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowAccessOrigin> {
        match dispatch_scope {
            OwnerDispatchScope::Top => self.main_window_access_origin(),
            OwnerDispatchScope::Child(handle) => self.child_window_access_origin(handle),
        }
    }

    fn main_window_access_origin(&self) -> Option<WindowAccessOrigin> {
        if self.main_document_serialized_origin == "null" {
            return Some(WindowAccessOrigin::Opaque {
                identity: self.top_level_opaque_origin_nonce,
            });
        }
        WindowAccessOrigin::from_serialized_origin(
            self.main_document_serialized_origin.clone(),
            self.document_domain_override.clone(),
        )
    }

    pub(in crate::native_bridge::context_host) fn child_window_access_origin(
        &self,
        handle: DomHandle,
    ) -> Option<WindowAccessOrigin> {
        let serialized_origin = self.child_browsing_context_window_origin(handle)?;
        self.child_window_access_origin_with_serialized_origin(
            handle,
            serialized_origin,
            self.child_effective_origin_document_domain_override(handle)?,
            self.child_own_opaque_origin_nonce(handle),
        )
    }

    pub(in crate::native_bridge::context_host) fn prospective_child_window_access_origin(
        &self,
        handle: DomHandle,
        serialized_origin: &str,
    ) -> Option<WindowAccessOrigin> {
        // A new tuple origin has not set document.domain. A newly created
        // opaque origin has a fresh nonce and therefore no identity in common
        // with the current LocalWindow; inherited opaque origins are resolved
        // to their creator below.
        self.child_window_access_origin_with_serialized_origin(
            handle,
            serialized_origin.to_owned(),
            None,
            None,
        )
    }

    fn child_window_access_origin_with_serialized_origin(
        &self,
        handle: DomHandle,
        serialized_origin: String,
        document_domain: Option<String>,
        own_opaque_identity: Option<OpaqueOriginNonce>,
    ) -> Option<WindowAccessOrigin> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        if serialized_origin != "null" {
            return WindowAccessOrigin::from_serialized_origin(serialized_origin, document_domain);
        }
        if entry.security_origin_inherited() && !entry.document_sandbox_forces_opaque_origin() {
            return self
                .child_browsing_context_parent_handle(handle)
                .map_or_else(
                    || self.main_window_access_origin(),
                    |parent| self.child_window_access_origin(parent),
                );
        }
        Some(WindowAccessOrigin::Opaque {
            identity: own_opaque_identity,
        })
    }

    pub(in crate::native_bridge::context_host) fn main_window_opaque_origin_nonce(
        &self,
    ) -> Option<OpaqueOriginNonce> {
        match self.main_window_access_origin()? {
            WindowAccessOrigin::Opaque { identity } => identity,
            WindowAccessOrigin::Tuple { .. } => None,
        }
    }
}

pub(in crate::native_bridge::context_host) type WindowAccessOrigin =
    BrowsingContextAccessOrigin<OpaqueOriginNonce>;

fn related_page_window_origins_can_access(
    accessing: &WindowAccessOrigin,
    accessed: &WindowAccessOrigin,
) -> bool {
    accessing.can_access(accessed)
}

pub(crate) fn set_window_security_token(
    scope: &mut v8::PinScope<'_, '_, ()>,
    context: v8::Local<'_, v8::Context>,
    key: Option<&str>,
) -> bool {
    let Some(key) = key else {
        context.use_default_security_token();
        return true;
    };
    let Some(token) =
        v8::String::new_from_utf8(scope, key.as_bytes(), v8::NewStringType::Internalized)
    else {
        context.use_default_security_token();
        return false;
    };
    context.set_security_token(token.into());
    true
}

fn window_security_token_key(origin: String) -> Option<String> {
    (origin != "null").then(|| format!("{WINDOW_SECURITY_TOKEN_PREFIX}{origin}"))
}

fn window_isolated_world_security_token_key(
    frame_origin: String,
    frame_document_domain_was_set: bool,
) -> Option<String> {
    if frame_document_domain_was_set || frame_origin == "null" {
        return None;
    }
    // Blink concatenates the frame origin token with an isolated copy of that
    // origin. Keep the same separation from the default world while allowing
    // the isolated context to access its own WindowProxy.
    Some(format!(
        "{WINDOW_ISOLATED_WORLD_SECURITY_TOKEN_PREFIX}{frame_origin}|{frame_origin}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        WindowAccessOrigin, related_page_window_origins_can_access,
        window_isolated_world_security_token_key, window_security_token_key,
    };
    use moli_storage_key::OpaqueOriginNonce;

    #[test]
    fn opaque_origin_does_not_receive_a_shared_security_token() {
        assert_eq!(window_security_token_key("null".to_owned()), None);
    }

    #[test]
    fn opaque_origin_access_requires_the_same_non_serialized_identity() {
        let inherited_identity = OpaqueOriginNonce::new(7);
        let distinct_identity = OpaqueOriginNonce::new(8);
        let inherited = WindowAccessOrigin::opaque(inherited_identity);

        assert!(inherited.can_access(&WindowAccessOrigin::opaque(inherited_identity)));
        assert!(!inherited.can_access(&WindowAccessOrigin::opaque(distinct_identity)));
        assert!(!inherited.can_access(&WindowAccessOrigin::Opaque { identity: None }));
    }

    #[test]
    fn related_pages_use_the_shared_browser_context_opaque_nonce() {
        let inherited_identity = OpaqueOriginNonce::new(11);
        let inherited = WindowAccessOrigin::opaque(inherited_identity);
        let related_inherited = WindowAccessOrigin::opaque(inherited_identity);
        let distinct = WindowAccessOrigin::opaque(OpaqueOriginNonce::new(12));

        assert!(related_page_window_origins_can_access(
            &inherited,
            &related_inherited
        ));
        assert!(!related_page_window_origins_can_access(
            &inherited, &distinct
        ));
    }

    #[test]
    fn tuple_origin_receives_a_stable_namespaced_security_token() {
        assert_eq!(
            window_security_token_key("https://example.test".to_owned()).as_deref(),
            Some("moli-window-origin-v1:https://example.test")
        );
    }

    #[test]
    fn document_domain_access_requires_both_documents_and_ignores_port() {
        let accessing = WindowAccessOrigin::from_serialized_origin(
            "https://www.example.test:8443".to_owned(),
            Some("example.test".to_owned()),
        )
        .expect("accessing origin");
        let target = WindowAccessOrigin::from_serialized_origin(
            "https://sub.example.test:9443".to_owned(),
            Some("example.test".to_owned()),
        )
        .expect("target origin");
        let target_without_domain = WindowAccessOrigin::from_serialized_origin(
            "https://sub.example.test:9443".to_owned(),
            None,
        )
        .expect("target origin without domain");

        assert!(accessing.can_access(&target));
        assert!(!accessing.can_access(&target_without_domain));
    }

    #[test]
    fn isolated_world_uses_a_distinct_composite_origin_token() {
        let origin = "https://example.test".to_owned();
        let isolated = window_isolated_world_security_token_key(origin.clone(), false);

        assert_eq!(
            isolated.as_deref(),
            Some("moli-window-isolated-origin-v1:https://example.test|https://example.test")
        );
        assert_ne!(isolated, window_security_token_key(origin));
    }

    #[test]
    fn isolated_world_uses_full_access_check_for_opaque_or_domain_mutated_frame() {
        assert_eq!(
            window_isolated_world_security_token_key("null".to_owned(), false),
            None
        );
        assert_eq!(
            window_isolated_world_security_token_key("https://example.test".to_owned(), true,),
            None
        );
    }
}
