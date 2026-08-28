use super::{
    JsContextHost, OwnerDispatchScope, WindowExecutionContextIdentity,
    window_security_tokens::WindowAccessOrigin,
};
use crate::{
    browsing_context_model::BrowsingContextAccessOrigin, document_runtime::DomHandle,
    util::context_host_ptr_from_window_object,
};
use url::Url;

/// Stable reason for refusing one local browsing-context navigation.
///
/// Callers use the reason only to preserve API-specific reporting: element
/// activation silently skips a candidate, while Location keeps its existing
/// synchronous sandbox `SecurityError` surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BrowsingContextNavigationDenial {
    JavascriptCrossOrigin,
    SandboxedAncestor,
    SandboxedPopup,
    SandboxedTopWithoutPermission,
    SandboxedTopWithoutTransientUserActivation,
    SandboxedTopWithoutStickyUserActivation,
    TopWithoutActivationOrDestinationRelation,
    UnrelatedContext,
    StaleContext,
}

impl BrowsingContextNavigationDenial {
    pub(crate) const fn is_sandbox_violation(self) -> bool {
        matches!(
            self,
            Self::SandboxedAncestor
                | Self::SandboxedPopup
                | Self::SandboxedTopWithoutPermission
                | Self::SandboxedTopWithoutTransientUserActivation
                | Self::SandboxedTopWithoutStickyUserActivation
        )
    }
}

impl JsContextHost {
    pub(crate) fn renderer_remote_javascript_url_source(
        &self,
        source_identity: WindowExecutionContextIdentity,
        suppress_referrer: bool,
    ) -> Option<crate::runtime::RendererRemoteJavaScriptUrlSource> {
        if !self.window_execution_context_identity_is_current(source_identity) {
            return None;
        }
        let navigation_source = self.renderer_top_level_navigation_source_for_dispatch_scope(
            source_identity.dispatch_scope(),
            suppress_referrer,
        )?;
        let window_source = self.remote_window_proxy_source_for_identity(source_identity)?;
        let source_origin =
            self.window_access_origin_for_dispatch_scope(source_identity.dispatch_scope())?;
        let (opaque_origin_nonce, document_domain) = match source_origin {
            BrowsingContextAccessOrigin::Opaque { identity } => {
                if window_source.serialized_origin() != "null" || identity.is_none() {
                    return None;
                }
                (identity, None)
            }
            BrowsingContextAccessOrigin::Tuple {
                serialized_origin,
                document_domain,
                ..
            } => {
                if serialized_origin != window_source.serialized_origin() {
                    return None;
                }
                (None, document_domain)
            }
        };
        let world = if self.window_execution_context_identity_is_default_world(source_identity) {
            crate::runtime::RendererRemoteJavaScriptUrlSourceWorld::Main
        } else {
            crate::runtime::RendererRemoteJavaScriptUrlSourceWorld::Isolated {
                grants_universal_access: source_identity.grants_universal_access(),
            }
        };
        Some(crate::runtime::RendererRemoteJavaScriptUrlSource::new(
            navigation_source,
            window_source,
            opaque_origin_nonce,
            document_domain,
            world,
        ))
    }

    pub(crate) fn remote_javascript_url_source_can_access_dispatch_scope(
        &self,
        target_scope: OwnerDispatchScope,
        source: &crate::runtime::RendererRemoteJavaScriptUrlSource,
    ) -> bool {
        if matches!(
            source.world(),
            crate::runtime::RendererRemoteJavaScriptUrlSourceWorld::Isolated {
                grants_universal_access: true
            }
        ) {
            return true;
        }
        let source_origin = if source.window_source().serialized_origin() == "null" {
            BrowsingContextAccessOrigin::Opaque {
                identity: source.opaque_origin_nonce(),
            }
        } else {
            let Some(origin) = BrowsingContextAccessOrigin::from_serialized_origin(
                source.window_source().serialized_origin().to_owned(),
                source.document_domain().map(ToOwned::to_owned),
            ) else {
                return false;
            };
            origin
        };
        let Some(target_origin) = self.window_access_origin_for_dispatch_scope(target_scope) else {
            return false;
        };
        source_origin.can_access(&target_origin)
    }

    /// Local renderer equivalent of Blink `LocalFrame::CanNavigate` for the
    /// browsing-context kinds Moli currently owns.
    ///
    /// RemoteFrame, fenced-frame-root and remote embedder decisions are not
    /// represented by this function: no such target kind exists in the local
    /// owner model yet. Every local child and related Page candidate must pass
    /// through this authority before target selection or direct Location
    /// navigation proceeds.
    pub(crate) fn can_navigate_browsing_context(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_identity: WindowExecutionContextIdentity,
        target_host: &JsContextHost,
        target_scope: OwnerDispatchScope,
        destination_url: &Url,
    ) -> Result<(), BrowsingContextNavigationDenial> {
        if !self.window_execution_context_identity_is_current(source_identity)
            || target_host
                .current_window_execution_context_owner(target_scope)
                .is_none()
        {
            return Err(BrowsingContextNavigationDenial::StaleContext);
        }
        let source_scope = source_identity.dispatch_scope();
        let same_page = std::ptr::eq(self, target_host);
        if same_page && source_scope == target_scope {
            return Ok(());
        }

        if destination_url.scheme() == "javascript"
            && !self.can_access_navigation_scope(source_identity, target_host, target_scope)
        {
            return Err(BrowsingContextNavigationDenial::JavascriptCrossOrigin);
        }

        let source_policy = self
            .document_policy_container_snapshot_for_owner(source_scope)
            .ok_or(BrowsingContextNavigationDenial::StaleContext)?;
        let target_is_outermost = target_scope_is_outermost(target_scope);
        let target_is_source_tree_top = same_page
            && matches!(source_scope, OwnerDispatchScope::Child(_))
            && target_scope == OwnerDispatchScope::Top;
        let target_is_source_descendant =
            same_page && self.navigation_target_is_descendant_of_source(source_scope, target_scope);

        if source_policy.sandbox.sandboxes_navigation {
            if !target_is_source_descendant && !target_is_outermost {
                return Err(BrowsingContextNavigationDenial::SandboxedAncestor);
            }

            if target_is_outermost && !target_is_source_tree_top {
                let sandbox = source_policy.sandbox;
                if !sandbox.allows_popups_to_escape
                    && (!sandbox.allows_popups
                        || !target_host.top_level_opener_matches_source(
                            scope,
                            target_scope,
                            self,
                            source_identity,
                        ))
                {
                    return Err(BrowsingContextNavigationDenial::SandboxedPopup);
                }
            }

            if target_is_source_tree_top {
                let sandbox = source_policy.sandbox;
                if !sandbox.allows_top_navigation
                    && !sandbox.allows_top_navigation_by_user_activation
                {
                    return Err(BrowsingContextNavigationDenial::SandboxedTopWithoutPermission);
                }
                if !sandbox.allows_top_navigation
                    && sandbox.allows_top_navigation_by_user_activation
                    && !self.transient_user_activation()
                {
                    return Err(
                        BrowsingContextNavigationDenial::SandboxedTopWithoutTransientUserActivation,
                    );
                }
                if sandbox.allows_top_navigation
                    && source_policy.top_navigation_without_user_gesture_is_restricted
                    && !self.sticky_user_activation()
                {
                    return Err(
                        BrowsingContextNavigationDenial::SandboxedTopWithoutStickyUserActivation,
                    );
                }
                return Ok(());
            }
        }

        if self.can_access_navigation_scope_or_ancestor(source_identity, target_host, target_scope)
        {
            return Ok(());
        }

        if target_is_outermost
            && (self.source_outermost_opener_matches_target(
                scope,
                source_identity,
                target_host,
                target_scope,
            ) || self.can_access_target_top_level_opener_ancestor(
                scope,
                source_identity,
                target_host,
                target_scope,
            ))
        {
            return Ok(());
        }

        if target_is_source_tree_top {
            if self.sticky_user_activation()
                || target_host
                    .navigation_scope_can_access_destination_origin(target_scope, destination_url)
                || target_host
                    .navigation_scope_shares_destination_site(target_scope, destination_url)
                || self.browser_context_runtime().popup_blocker_policy()
                    == crate::RendererPopupBlockerPolicy::AllowWithoutTransientActivation
            {
                return Ok(());
            }
            return Err(BrowsingContextNavigationDenial::TopWithoutActivationOrDestinationRelation);
        }

        Err(BrowsingContextNavigationDenial::UnrelatedContext)
    }

    /// Source-side `CanNavigate` admission for a related top-level target
    /// whose LocalWindow belongs to another script agent. Only replicated
    /// group facts are consulted here; the receiving renderer repeats exact
    /// endpoint/Page currentness before executing the accepted command.
    pub(crate) fn can_navigate_remote_top_level_browsing_context(
        &self,
        source_identity: WindowExecutionContextIdentity,
        target: &crate::script_vm::RendererRemoteTopLevelWindowProxyTarget,
        destination_url: &Url,
    ) -> Result<(), BrowsingContextNavigationDenial> {
        if !self.window_execution_context_identity_is_current(source_identity) {
            return Err(BrowsingContextNavigationDenial::StaleContext);
        }
        let source_scope = source_identity.dispatch_scope();
        let target_origin = if target.current_serialized_origin == "null" {
            BrowsingContextAccessOrigin::Opaque {
                identity: target.current_opaque_origin_nonce,
            }
        } else {
            BrowsingContextAccessOrigin::from_serialized_origin(
                target.current_serialized_origin.clone(),
                target.current_document_domain.clone(),
            )
            .ok_or(BrowsingContextNavigationDenial::StaleContext)?
        };
        let source_can_access_target = source_identity.grants_universal_access()
            || self
                .window_access_origin_for_dispatch_scope(source_scope)
                .is_some_and(|source_origin| source_origin.can_access(&target_origin));
        if destination_url.scheme() == "javascript" && !source_can_access_target {
            return Err(BrowsingContextNavigationDenial::JavascriptCrossOrigin);
        }
        let source_policy = self
            .document_policy_container_snapshot_for_owner(source_scope)
            .ok_or(BrowsingContextNavigationDenial::StaleContext)?;
        let source_endpoint = self
            .top_level_window_proxy_endpoint_id()
            .ok_or(BrowsingContextNavigationDenial::StaleContext)?;
        let source_opener = self
            .page_script_environment
            .as_ref()
            .and_then(crate::script_vm::RendererPageScriptEnvironment::top_level_opener_endpoint);
        let target_was_opened_by_source = target.opener_endpoint == Some(source_endpoint);
        let source_was_opened_by_target = source_opener == Some(target.endpoint);

        if source_policy.sandbox.sandboxes_navigation {
            let sandbox = source_policy.sandbox;
            if !sandbox.allows_popups_to_escape
                && (!sandbox.allows_popups || !target_was_opened_by_source)
            {
                return Err(BrowsingContextNavigationDenial::SandboxedPopup);
            }
        }
        if source_can_access_target || target_was_opened_by_source || source_was_opened_by_target {
            return Ok(());
        }
        Err(BrowsingContextNavigationDenial::UnrelatedContext)
    }

    /// Source-side `CanNavigate` admission for a nested context whose owner
    /// lives in another script agent. The replicated tree contains only the
    /// policy/origin facts needed for this decision; target currentness is
    /// checked again against the root-Document-qualified frame token when the
    /// command reaches the owning Page.
    pub(crate) fn can_navigate_remote_frame_browsing_context(
        &self,
        source_identity: WindowExecutionContextIdentity,
        target: &crate::script_vm::RendererRemoteFrameSnapshot,
        destination_url: &Url,
    ) -> Result<(), BrowsingContextNavigationDenial> {
        if !self.window_execution_context_identity_is_current(source_identity) {
            return Err(BrowsingContextNavigationDenial::StaleContext);
        }
        let Some(environment) = self.page_script_environment.as_ref() else {
            return Err(BrowsingContextNavigationDenial::StaleContext);
        };
        if environment.remote_frame_snapshot(target.token).as_ref() != Some(target) {
            return Err(BrowsingContextNavigationDenial::StaleContext);
        }
        let can_access_target_or_ancestor = self
            .source_can_access_remote_frame_or_ancestor(source_identity, target)
            .unwrap_or(false);
        if destination_url.scheme() == "javascript" && !can_access_target_or_ancestor {
            return Err(BrowsingContextNavigationDenial::JavascriptCrossOrigin);
        }
        let source_policy = self
            .document_policy_container_snapshot_for_owner(source_identity.dispatch_scope())
            .ok_or(BrowsingContextNavigationDenial::StaleContext)?;
        if source_policy.sandbox.sandboxes_navigation {
            // A related Page's frame is neither a descendant of the source nor
            // an outermost browsing context. This is Blink's sandboxed
            // ancestor refusal before opener/top-level exceptions apply.
            return Err(BrowsingContextNavigationDenial::SandboxedAncestor);
        }
        if can_access_target_or_ancestor {
            return Ok(());
        }
        Err(BrowsingContextNavigationDenial::UnrelatedContext)
    }

    fn source_can_access_remote_frame_or_ancestor(
        &self,
        source_identity: WindowExecutionContextIdentity,
        target: &crate::script_vm::RendererRemoteFrameSnapshot,
    ) -> Option<bool> {
        if source_identity.grants_universal_access() {
            return Some(true);
        }
        let source_origin =
            self.window_access_origin_for_dispatch_scope(source_identity.dispatch_scope())?;
        let environment = self.page_script_environment.as_ref()?;
        let tree = environment.remote_frame_tree_snapshot(target.token.endpoint)?;
        let mut candidate = Some(target.clone());
        while let Some(snapshot) = candidate {
            let target_origin = if snapshot.serialized_origin == "null" {
                BrowsingContextAccessOrigin::Opaque {
                    identity: snapshot.opaque_origin_nonce,
                }
            } else {
                BrowsingContextAccessOrigin::from_serialized_origin(
                    snapshot.serialized_origin,
                    snapshot.document_domain,
                )?
            };
            if source_origin.can_access(&target_origin) {
                return Some(true);
            }
            candidate = snapshot.parent_browsing_context_id.and_then(|parent_id| {
                tree.iter()
                    .find(|candidate| candidate.token.browsing_context_id == parent_id)
                    .cloned()
            });
        }
        let top = environment.remote_top_level_target_snapshot(target.token.endpoint)?;
        let top_origin = if top.current_serialized_origin == "null" {
            BrowsingContextAccessOrigin::Opaque {
                identity: top.current_opaque_origin_nonce,
            }
        } else {
            BrowsingContextAccessOrigin::from_serialized_origin(
                top.current_serialized_origin,
                top.current_document_domain,
            )?
        };
        Some(source_origin.can_access(&top_origin))
    }

    pub(crate) fn navigation_api_base_url_for_identity(
        &self,
        _scope: &mut v8::PinScope<'_, '_>,
        identity: WindowExecutionContextIdentity,
    ) -> Option<Url> {
        self.navigation_api_base_url_for_identity_without_scope(identity)
    }

    pub(crate) fn navigation_api_base_url_for_identity_without_scope(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<Url> {
        if !self.window_execution_context_identity_is_current(identity) {
            return None;
        }
        match identity.dispatch_scope() {
            OwnerDispatchScope::Top => self
                .dom_host()
                .document_base_url()
                .or_else(|| Some(self.document_url().clone())),
            OwnerDispatchScope::Child(handle) => self.child_browsing_context_base_url(handle),
        }
    }

    pub(crate) fn document_url_for_window_execution_context_identity(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<Url> {
        if !self.window_execution_context_identity_is_current(identity) {
            return None;
        }
        Some(match identity.dispatch_scope() {
            OwnerDispatchScope::Top => self
                .dom_host()
                .document_url()
                .cloned()
                .unwrap_or_else(|| self.document_url().clone()),
            OwnerDispatchScope::Child(handle) => self.document_url_for_child_context(handle),
        })
    }

    /// Freezes Chromium's policy-container
    /// `can_navigate_top_without_user_gesture` decision after a child Document
    /// commit. Dynamic iframe attribute mutation must not rewrite this bit for
    /// the already-committed Document.
    pub(in crate::native_bridge::context_host) fn refresh_child_top_navigation_policy_after_commit(
        &mut self,
        handle: DomHandle,
    ) {
        let inherited_restriction = self
            .child_browsing_context_parent_handle(handle)
            .and_then(|parent| self.child_browsing_context_policy_container_snapshot(parent))
            .map(|policy| policy.top_navigation_without_user_gesture_is_restricted)
            .unwrap_or(
                self.document_policy_container()
                    .top_navigation_without_user_gesture_is_restricted,
            );
        let Some(policy) = self.child_browsing_context_policy_container_snapshot(handle) else {
            return;
        };
        let restricted = if self.child_committed_origin_matches_top(handle) {
            false
        } else if policy.sandbox.frame_owner_explicitly_allows_top_navigation {
            inherited_restriction
        } else {
            true
        };
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.set_top_navigation_without_user_gesture_is_restricted(restricted);
        }
    }

    fn navigation_target_is_descendant_of_source(
        &self,
        source_scope: OwnerDispatchScope,
        target_scope: OwnerDispatchScope,
    ) -> bool {
        let OwnerDispatchScope::Child(target) = target_scope else {
            return false;
        };
        match source_scope {
            OwnerDispatchScope::Top => true,
            OwnerDispatchScope::Child(source) => {
                target != source && self.child_handle_is_descendant_of(target, source)
            }
        }
    }

    fn child_handle_is_descendant_of(&self, mut candidate: DomHandle, ancestor: DomHandle) -> bool {
        while let Some(parent) = self.child_browsing_context_parent_handle(candidate) {
            if parent == ancestor {
                return true;
            }
            candidate = parent;
        }
        false
    }

    fn can_access_navigation_scope(
        &self,
        source_identity: WindowExecutionContextIdentity,
        target_host: &JsContextHost,
        target_scope: OwnerDispatchScope,
    ) -> bool {
        if std::ptr::eq(self, target_host) {
            self.window_execution_context_can_access_dispatch_scope(source_identity, target_scope)
        } else {
            self.window_execution_context_can_access_related_page_dispatch_scope(
                source_identity,
                target_host,
                target_scope,
            )
        }
    }

    fn can_access_navigation_scope_or_ancestor(
        &self,
        source_identity: WindowExecutionContextIdentity,
        target_host: &JsContextHost,
        mut target_scope: OwnerDispatchScope,
    ) -> bool {
        loop {
            if self.can_access_navigation_scope(source_identity, target_host, target_scope) {
                return true;
            }
            target_scope = match target_scope {
                OwnerDispatchScope::Child(child) => target_host
                    .child_browsing_context_parent_handle(child)
                    .map(OwnerDispatchScope::Child)
                    .unwrap_or(OwnerDispatchScope::Top),
                OwnerDispatchScope::Top => return false,
            };
        }
    }

    fn top_level_opener_matches_source(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target_scope: OwnerDispatchScope,
        source_host: &JsContextHost,
        source_identity: WindowExecutionContextIdentity,
    ) -> bool {
        let Some((host_ptr, dispatch_scope)) =
            self.top_level_opener_endpoint_for_scope(scope, target_scope)
        else {
            return false;
        };
        std::ptr::eq(unsafe { &*host_ptr }, source_host)
            && dispatch_scope == source_identity.dispatch_scope()
    }

    fn source_outermost_opener_matches_target(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_identity: WindowExecutionContextIdentity,
        target_host: &JsContextHost,
        target_scope: OwnerDispatchScope,
    ) -> bool {
        let source_scope = source_identity.dispatch_scope();
        if !target_scope_is_outermost(source_scope) {
            return false;
        }
        let Some((host_ptr, dispatch_scope)) =
            self.top_level_opener_endpoint_for_scope(scope, source_scope)
        else {
            return false;
        };
        std::ptr::eq(unsafe { &*host_ptr }, target_host) && dispatch_scope == target_scope
    }

    fn can_access_target_top_level_opener_ancestor(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        source_identity: WindowExecutionContextIdentity,
        target_host: &JsContextHost,
        target_scope: OwnerDispatchScope,
    ) -> bool {
        let Some((opener_host_ptr, opener_scope)) =
            target_host.top_level_opener_endpoint_for_scope(scope, target_scope)
        else {
            return false;
        };
        let opener_host = unsafe { &*opener_host_ptr };
        self.can_access_navigation_scope_or_ancestor(source_identity, opener_host, opener_scope)
    }

    fn top_level_opener_endpoint_for_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target_scope: OwnerDispatchScope,
    ) -> Option<(*const JsContextHost, OwnerDispatchScope)> {
        if target_scope != OwnerDispatchScope::Top {
            return None;
        }
        let opener = self
            .top_level_opener_value(scope)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
        navigation_window_endpoint(scope, opener)
    }

    fn navigation_scope_can_access_destination_origin(
        &self,
        target_scope: OwnerDispatchScope,
        destination_url: &Url,
    ) -> bool {
        let Some(target_origin) = self.window_access_origin_for_dispatch_scope(target_scope) else {
            return false;
        };
        let Some(destination_origin) = WindowAccessOrigin::from_serialized_origin(
            moli_url::origin_ascii_serialization(destination_url),
            None,
        ) else {
            return false;
        };
        target_origin.can_access(&destination_origin)
    }

    fn child_committed_origin_matches_top(&self, handle: DomHandle) -> bool {
        let Some(top_origin) = self
            .window_access_origin_for_dispatch_scope(OwnerDispatchScope::Top)
            .map(origin_without_document_domain)
        else {
            return false;
        };
        let Some(child_origin) = self
            .window_access_origin_for_dispatch_scope(OwnerDispatchScope::Child(handle))
            .map(origin_without_document_domain)
        else {
            return false;
        };
        top_origin.can_access(&child_origin)
    }

    fn navigation_scope_shares_destination_site(
        &self,
        target_scope: OwnerDispatchScope,
        destination_url: &Url,
    ) -> bool {
        if target_scope != OwnerDispatchScope::Top {
            return false;
        }
        let Some(BrowsingContextAccessOrigin::Tuple {
            serialized_origin,
            scheme,
            document_domain,
        }) = self.window_access_origin_for_dispatch_scope(target_scope)
        else {
            return false;
        };
        if scheme != destination_url.scheme() {
            return false;
        }
        let target_host = document_domain.or_else(|| {
            Url::parse(&serialized_origin)
                .ok()?
                .host_str()
                .map(str::to_owned)
        });
        let Some(target_host) = target_host else {
            return false;
        };
        let Some(destination_host) = destination_url.host_str() else {
            return false;
        };
        let Some(target_domain) = chromium_domain_and_registry(&target_host) else {
            return false;
        };
        let Some(destination_domain) = chromium_domain_and_registry(destination_host) else {
            return false;
        };
        target_domain == destination_domain
    }
}

fn origin_without_document_domain<OpaqueIdentity>(
    origin: BrowsingContextAccessOrigin<OpaqueIdentity>,
) -> BrowsingContextAccessOrigin<OpaqueIdentity> {
    match origin {
        BrowsingContextAccessOrigin::Opaque { identity } => {
            BrowsingContextAccessOrigin::Opaque { identity }
        }
        BrowsingContextAccessOrigin::Tuple {
            serialized_origin,
            scheme,
            ..
        } => BrowsingContextAccessOrigin::Tuple {
            serialized_origin,
            scheme,
            document_domain: None,
        },
    }
}

fn chromium_domain_and_registry(host: &str) -> Option<&str> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.parse::<std::net::IpAddr>().is_ok()
        || !host.contains('.')
        || moli_site::host_is_public_suffix(host)
    {
        return None;
    }
    Some(moli_site::registrable_site_host(host))
}

fn navigation_window_endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<(*const JsContextHost, OwnerDispatchScope)> {
    let host_ptr = crate::native_bridge::cross_origin_window_target_host_ptr(scope, window)
        .or_else(|| context_host_ptr_from_window_object(scope, window))?;
    let dispatch_scope =
        if crate::native_bridge::is_cross_origin_related_top_window_proxy(scope, window) {
            OwnerDispatchScope::Top
        } else {
            crate::context_bootstrap::runtime_window_dispatch_scope(scope, window)?
        };
    Some((host_ptr.cast_const(), dispatch_scope))
}

fn target_scope_is_outermost(scope: OwnerDispatchScope) -> bool {
    matches!(scope, OwnerDispatchScope::Top)
}
