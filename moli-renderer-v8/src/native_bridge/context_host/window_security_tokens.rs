use super::{
    JsContextHost, OwnerDispatchScope, WindowExecutionContextIdentity, WindowExecutionContextOwner,
    child_frames::{ChildAncestorOriginsReferrerPolicy, ChildBrowsingContextEntry},
};
use crate::document_runtime::DomHandle;

const WINDOW_SECURITY_TOKEN_PREFIX: &str = "moli-window-origin-v1:";
const WINDOW_ISOLATED_WORLD_SECURITY_TOKEN_PREFIX: &str = "moli-window-isolated-origin-v1:";

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn child_ancestor_origins_referrer_policy_from_owner_attribute(
        &self,
        handle: DomHandle,
    ) -> ChildAncestorOriginsReferrerPolicy {
        match self
            .dom_host()
            .get_attribute(handle, "referrerpolicy")
            .as_deref()
            .and_then(crate::referrer_policy::normalize_referrer_policy)
            .as_deref()
        {
            Some("no-referrer") => ChildAncestorOriginsReferrerPolicy::NoReferrer,
            Some("same-origin") => ChildAncestorOriginsReferrerPolicy::SameOrigin,
            Some(_) | None => ChildAncestorOriginsReferrerPolicy::Default,
        }
    }

    pub(in crate::native_bridge::context_host) fn refresh_current_child_document_ancestor_origins(
        &mut self,
        handle: DomHandle,
    ) -> bool {
        let Some(child_origin) = self.child_window_access_origin(handle) else {
            return false;
        };
        let Some(parent_scope) = self.owner_dispatch_scope_for_node(handle) else {
            return false;
        };
        let Some(parent_origin) = self.window_access_origin_for_dispatch_scope(parent_scope) else {
            return false;
        };
        let parent_ancestor_origins = match parent_scope {
            OwnerDispatchScope::Child(parent) => self
                .child_browsing_contexts
                .get(&parent)
                .map(ChildBrowsingContextEntry::document_internal_ancestor_origins)
                .unwrap_or_default(),
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => Vec::new(),
        };
        let Some(policy) = self
            .child_browsing_contexts
            .get(&handle)
            .map(ChildBrowsingContextEntry::ancestor_origins_referrer_policy_snapshot)
        else {
            return false;
        };
        let origins = build_child_document_internal_ancestor_origins(
            parent_origin,
            &parent_ancestor_origins,
            &child_origin,
            policy,
        );
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return false;
        };
        entry.set_document_internal_ancestor_origins(origins);
        true
    }

    pub(crate) fn child_browsing_context_ancestor_origins(
        &self,
        handle: DomHandle,
    ) -> Option<Vec<String>> {
        self.current_child_document_task_owner(handle)?;
        Some(
            self.child_browsing_contexts
                .get(&handle)?
                .document_internal_ancestor_origins()
                .into_iter()
                .map(|origin| origin.serialized_origin())
                .collect(),
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
            // Lightweight popups still share the opener V8 context. Until they have a concrete
            // LocalWindow realm, changing that shared context token would also mutate the opener.
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
                OwnerDispatchScope::LightweightPopup(_) => None,
            };
            if set_window_security_token(scope, context, key.as_deref()) {
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
        set_window_security_token(
            scope,
            context,
            self.child_default_world_security_token_key(handle)
                .as_deref(),
        )
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

    /// Compares the Window's origin with its top-level origin without applying
    /// `document.domain` relaxation. Algorithms that require "same origin"
    /// use this rather than the broader Window access check.
    pub(crate) fn child_window_has_same_origin_with_its_top_level_origin(
        &self,
        handle: DomHandle,
    ) -> bool {
        let mut top_child = handle;
        for _ in 0..=self.child_browsing_contexts.len() {
            let Some(parent) = self.child_browsing_context_parent_handle(top_child) else {
                break;
            };
            top_child = parent;
        }
        let top = match self.child_browsing_context_popup_owner_id(top_child) {
            Some(popup_id) => self.lightweight_popup_window_access_origin(popup_id),
            None => self.main_window_access_origin(),
        };
        let Some(top) = top else {
            return false;
        };
        let Some(child) = self.child_window_access_origin(handle) else {
            return false;
        };
        top.has_same_origin(&child)
    }

    pub(in crate::native_bridge::context_host) fn child_window_can_access_lightweight_popup(
        &self,
        handle: DomHandle,
        popup_id: u64,
    ) -> bool {
        let Some(child) = self.child_window_access_origin(handle) else {
            return false;
        };
        let Some(popup) = self.lightweight_popup_window_access_origin(popup_id) else {
            return false;
        };
        child.can_access(&popup)
    }

    pub(crate) fn window_execution_context_can_access(
        &self,
        accessing: WindowExecutionContextIdentity,
        accessed: WindowExecutionContextIdentity,
    ) -> bool {
        if !self.window_execution_context_identity_is_current(accessed) {
            return false;
        }
        self.window_execution_context_can_access_dispatch_scope(
            accessing,
            accessed.dispatch_scope(),
        )
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
            (
                WindowExecutionContextOwner::LightweightPopup { popup_id, .. },
                OwnerDispatchScope::LightweightPopup(dispatch_popup_id),
            ) if popup_id == dispatch_popup_id => {
                self.lightweight_popup_window_access_origin(popup_id)
            }
            _ => None,
        }
    }

    fn lightweight_popup_window_access_origin(&self, popup_id: u64) -> Option<WindowAccessOrigin> {
        self.lightweight_popup_access_origin(popup_id)
    }

    pub(in crate::native_bridge::context_host) fn window_access_origin_for_dispatch_scope(
        &self,
        dispatch_scope: OwnerDispatchScope,
    ) -> Option<WindowAccessOrigin> {
        match dispatch_scope {
            OwnerDispatchScope::Top => self.main_window_access_origin(),
            OwnerDispatchScope::Child(handle) => self.child_window_access_origin(handle),
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                self.lightweight_popup_window_access_origin(popup_id)
            }
        }
    }

    fn main_window_access_origin(&self) -> Option<WindowAccessOrigin> {
        let serialized_origin = moli_url::origin_ascii_serialization(self.document_url());
        if serialized_origin == "null" {
            return Some(WindowAccessOrigin::opaque(
                self.current_window_execution_context_owner(OwnerDispatchScope::Top)?,
            ));
        }
        WindowAccessOrigin::from_serialized_origin(
            serialized_origin,
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
            self.current_window_execution_context_owner(OwnerDispatchScope::Child(handle)),
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
        own_opaque_identity: Option<WindowExecutionContextOwner>,
    ) -> Option<WindowAccessOrigin> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        if serialized_origin != "null" {
            return WindowAccessOrigin::from_serialized_origin(serialized_origin, document_domain);
        }
        if entry.security_origin_inherited() && !entry.document_sandbox_forces_opaque_origin() {
            if let Some(popup_id) = self.child_browsing_context_popup_owner_id(handle) {
                return self.lightweight_popup_window_access_origin(popup_id);
            }
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::native_bridge::context_host) enum WindowAccessOrigin {
    Opaque {
        identity: Option<WindowExecutionContextOwner>,
    },
    Tuple {
        serialized_origin: String,
        scheme: String,
        document_domain: Option<String>,
    },
}

impl WindowAccessOrigin {
    pub(in crate::native_bridge::context_host) fn opaque(
        identity: WindowExecutionContextOwner,
    ) -> Self {
        Self::Opaque {
            identity: Some(identity),
        }
    }

    fn opaque_without_identity() -> Self {
        Self::Opaque { identity: None }
    }

    pub(in crate::native_bridge::context_host) fn from_serialized_origin(
        serialized_origin: String,
        document_domain: Option<String>,
    ) -> Option<Self> {
        if serialized_origin == "null" {
            return Some(Self::Opaque { identity: None });
        }
        let scheme = url::Url::parse(&serialized_origin)
            .ok()?
            .scheme()
            .to_owned();
        Some(Self::Tuple {
            serialized_origin,
            scheme,
            document_domain,
        })
    }

    pub(in crate::native_bridge::context_host) fn can_access(&self, target: &Self) -> bool {
        if let (
            Self::Opaque {
                identity: Some(accessing_identity),
            },
            Self::Opaque {
                identity: Some(target_identity),
            },
        ) = (self, target)
        {
            return accessing_identity == target_identity;
        }
        let (
            Self::Tuple {
                serialized_origin: accessing_origin,
                scheme: accessing_scheme,
                document_domain: accessing_domain,
            },
            Self::Tuple {
                serialized_origin: target_origin,
                scheme: target_scheme,
                document_domain: target_domain,
            },
        ) = (self, target)
        else {
            return false;
        };
        match (accessing_domain, target_domain) {
            (None, None) => accessing_origin == target_origin,
            (Some(accessing_domain), Some(target_domain)) => {
                accessing_scheme == target_scheme && accessing_domain == target_domain
            }
            _ => false,
        }
    }

    pub(in crate::native_bridge::context_host) fn has_same_origin(&self, target: &Self) -> bool {
        match (self, target) {
            (
                Self::Opaque {
                    identity: Some(origin),
                },
                Self::Opaque {
                    identity: Some(target_origin),
                },
            ) => origin == target_origin,
            (
                Self::Tuple {
                    serialized_origin: origin,
                    ..
                },
                Self::Tuple {
                    serialized_origin: target_origin,
                    ..
                },
            ) => origin == target_origin,
            _ => false,
        }
    }

    pub(in crate::native_bridge::context_host) fn serialized_origin(&self) -> String {
        match self {
            Self::Opaque { .. } => "null".to_owned(),
            Self::Tuple {
                serialized_origin, ..
            } => serialized_origin.clone(),
        }
    }
}

fn build_child_document_internal_ancestor_origins(
    parent_origin: WindowAccessOrigin,
    parent_ancestor_origins: &[WindowAccessOrigin],
    child_origin: &WindowAccessOrigin,
    policy: ChildAncestorOriginsReferrerPolicy,
) -> Vec<WindowAccessOrigin> {
    let mut masked = match policy {
        ChildAncestorOriginsReferrerPolicy::NoReferrer => true,
        ChildAncestorOriginsReferrerPolicy::SameOrigin => {
            !parent_origin.has_same_origin(child_origin)
        }
        ChildAncestorOriginsReferrerPolicy::Default => false,
    };
    let mut origins = Vec::with_capacity(1 + parent_ancestor_origins.len());
    origins.push(if masked {
        WindowAccessOrigin::opaque_without_identity()
    } else {
        parent_origin.clone()
    });
    for ancestor_origin in parent_ancestor_origins {
        if masked && ancestor_origin.has_same_origin(&parent_origin) {
            origins.push(WindowAccessOrigin::opaque_without_identity());
        } else {
            masked = false;
            origins.push(ancestor_origin.clone());
        }
    }
    origins
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
        ChildAncestorOriginsReferrerPolicy, WindowAccessOrigin,
        build_child_document_internal_ancestor_origins, window_isolated_world_security_token_key,
        window_security_token_key,
    };
    use crate::{frame_owner_model::LocalWindowId, native_bridge::WindowExecutionContextOwner};

    #[test]
    fn opaque_origin_does_not_receive_a_shared_security_token() {
        assert_eq!(window_security_token_key("null".to_owned()), None);
    }

    #[test]
    fn opaque_origin_access_requires_the_same_non_serialized_identity() {
        let inherited_identity = WindowExecutionContextOwner::Frame(LocalWindowId(7));
        let distinct_identity = WindowExecutionContextOwner::Frame(LocalWindowId(8));
        let inherited = WindowAccessOrigin::opaque(inherited_identity);

        assert!(inherited.can_access(&WindowAccessOrigin::opaque(inherited_identity)));
        assert!(!inherited.can_access(&WindowAccessOrigin::opaque(distinct_identity)));
        assert!(!inherited.can_access(&WindowAccessOrigin::Opaque { identity: None }));
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
    fn same_origin_check_does_not_use_document_domain_relaxation() {
        let original = WindowAccessOrigin::from_serialized_origin(
            "https://www.example.test:8443".to_owned(),
            None,
        )
        .expect("original origin");
        let original_with_domain = WindowAccessOrigin::from_serialized_origin(
            "https://www.example.test:8443".to_owned(),
            Some("example.test".to_owned()),
        )
        .expect("original origin with document.domain");
        let relaxed_peer = WindowAccessOrigin::from_serialized_origin(
            "https://sub.example.test:9443".to_owned(),
            Some("example.test".to_owned()),
        )
        .expect("relaxed peer origin");

        assert!(original.has_same_origin(&original_with_domain));
        assert!(!original_with_domain.has_same_origin(&relaxed_peer));
        assert!(original_with_domain.can_access(&relaxed_peer));
    }

    #[test]
    fn ancestor_origins_mask_the_same_origin_parent_prefix_only() {
        let origin_a =
            WindowAccessOrigin::from_serialized_origin("https://a.example.test".to_owned(), None)
                .expect("origin A");
        let origin_b =
            WindowAccessOrigin::from_serialized_origin("https://b.example.test".to_owned(), None)
                .expect("origin B");

        let no_referrer = build_child_document_internal_ancestor_origins(
            origin_b.clone(),
            &[origin_b.clone(), origin_a.clone()],
            &origin_a,
            ChildAncestorOriginsReferrerPolicy::NoReferrer,
        );
        assert_eq!(
            no_referrer
                .iter()
                .map(WindowAccessOrigin::serialized_origin)
                .collect::<Vec<_>>(),
            vec!["null", "null", "https://a.example.test"]
        );

        let same_origin = build_child_document_internal_ancestor_origins(
            origin_b.clone(),
            std::slice::from_ref(&origin_a),
            &origin_a,
            ChildAncestorOriginsReferrerPolicy::SameOrigin,
        );
        assert_eq!(
            same_origin
                .iter()
                .map(WindowAccessOrigin::serialized_origin)
                .collect::<Vec<_>>(),
            vec!["null", "https://a.example.test"]
        );

        let default = build_child_document_internal_ancestor_origins(
            origin_b,
            &[origin_a],
            &WindowAccessOrigin::from_serialized_origin(
                "https://child.example.test".to_owned(),
                None,
            )
            .expect("child origin"),
            ChildAncestorOriginsReferrerPolicy::Default,
        );
        assert_eq!(
            default
                .iter()
                .map(WindowAccessOrigin::serialized_origin)
                .collect::<Vec<_>>(),
            vec!["https://b.example.test", "https://a.example.test"]
        );
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
