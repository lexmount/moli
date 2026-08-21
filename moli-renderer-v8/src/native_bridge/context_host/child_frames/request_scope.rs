use super::*;
use crate::{
    context_bootstrap::web_storage_area_key_for_storage_key,
    native_bridge::{
        ACTIVE_CHILD_WINDOW_HANDLE_SLOT, OwnerDispatchScope, WindowExecutionContextIdentity,
    },
    util::{get_private_value, set_private_value},
};
use moli_storage_key::{MoliStorageKey, OpaqueOriginNonce, StoragePartitionRelation, site_for_url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageContextScope {
    origin: String,
    storage_key: MoliStorageKey,
}

impl StorageContextScope {
    fn new(origin: String, storage_key: MoliStorageKey) -> Self {
        Self {
            origin,
            storage_key,
        }
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn storage_key(&self) -> &MoliStorageKey {
        &self.storage_key
    }

    pub(crate) fn into_parts(self) -> (String, MoliStorageKey) {
        (self.origin, self.storage_key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebStorageScope {
    context: StorageContextScope,
    area_key: String,
}

impl WebStorageScope {
    fn new(origin: String, storage_key: MoliStorageKey) -> Self {
        Self::from_storage_context(StorageContextScope::new(origin, storage_key))
    }

    fn from_storage_context(context: StorageContextScope) -> Self {
        let area_key = web_storage_area_key_for_storage_key(context.storage_key());
        Self { context, area_key }
    }

    pub(crate) fn from_parts(
        origin: String,
        area_key: String,
        storage_key: MoliStorageKey,
    ) -> Self {
        Self {
            context: StorageContextScope::new(origin, storage_key),
            area_key,
        }
    }

    pub(crate) fn origin(&self) -> &str {
        self.context.origin()
    }

    pub(crate) fn area_key(&self) -> &str {
        &self.area_key
    }

    pub(crate) fn storage_key(&self) -> &MoliStorageKey {
        self.context.storage_key()
    }

    pub(crate) fn into_storage_context(self) -> StorageContextScope {
        self.context
    }

    pub(crate) fn into_parts(self) -> (String, String, MoliStorageKey) {
        let (origin, storage_key) = self.context.into_parts();
        (origin, self.area_key, storage_key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveStorageContext {
    scope: StorageContextScope,
}

impl ActiveStorageContext {
    fn new(scope: StorageContextScope) -> Self {
        Self { scope }
    }

    pub(crate) fn origin(&self) -> &str {
        self.scope.origin()
    }

    pub(crate) fn storage_key(&self) -> &MoliStorageKey {
        self.scope.storage_key()
    }

    pub(crate) fn web_storage_area_key(&self) -> String {
        web_storage_area_key_for_storage_key(self.storage_key())
    }

    pub(crate) fn into_origin_and_area_key(self) -> (String, String) {
        let area_key = web_storage_area_key_for_storage_key(self.scope.storage_key());
        let (origin, _) = self.scope.into_parts();
        (origin, area_key)
    }
}

impl JsContextHost {
    pub(crate) fn child_browsing_context_current_url(&self, handle: DomHandle) -> Option<Url> {
        self.frame_owner_store
            .current_child_owner_snapshot(handle)
            .map(|owner| owner.document_url)
    }

    pub(crate) fn child_browsing_context_secure_context_url(
        &self,
        handle: DomHandle,
    ) -> Option<Url> {
        let mut current = Some(handle);
        let mut depth = 0;
        while let Some(current_handle) = current {
            let entry = self.child_browsing_contexts.get(&current_handle)?;
            if !entry.security_origin_inherited() {
                return entry.current_document_url();
            }
            depth += 1;
            if depth > self.child_browsing_contexts.len() {
                return None;
            }
            current = self.child_browsing_context_parent_handle(current_handle);
        }
        // Initial about:blank and srcdoc retain the creator origin's potential
        // trustworthiness even when sandboxing derives an opaque origin.
        Some(self.document_url().clone())
    }

    pub(crate) fn child_browsing_context_request_scope(
        &self,
        handle: DomHandle,
    ) -> Option<(String, Url)> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        let document_url = self.child_browsing_context_base_url(handle)?;
        Some((entry.frame_id().to_owned(), document_url))
    }

    pub(crate) fn active_child_subresource_request_scope(
        &self,
    ) -> Option<(DomHandle, String, Url)> {
        let handle = *self.active_child_subresource_request_scopes.last()?;
        let (frame_id, document_url) = self.child_browsing_context_request_scope(handle)?;
        Some((handle, frame_id, document_url))
    }

    pub(crate) fn child_browsing_context_base_url(&self, handle: DomHandle) -> Option<Url> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        let document_url = self.child_browsing_context_current_url(handle)?;
        if entry.pending_attribute_bootstrap_commit()
            && let Some(snapshot) = self
                .materialize_local_child_snapshot_for_bootstrap(handle, entry.attribute_bootstrap())
        {
            if moli_url::is_about_blank(&snapshot.url)
                && entry.attribute_bootstrap().security_origin_inherited()
            {
                return Some(self.document_base_url_for_child_context(handle));
            }
            return Some(child_snapshot_base_url(&snapshot, snapshot.url.clone()));
        }
        let inherits_parent_about_blank = moli_url::is_about_blank(&document_url)
            && self.child_browsing_context_inherits_parent_origin(handle);
        let native_base_url = self
            .child_browsing_context_document_handle(handle)
            .and_then(|document_handle| {
                self.dom_host()
                    .node(document_handle)
                    .and_then(crate::dom::native::Node::as_document)
                    .map(|document| document.base_url().clone())
            });
        if let Some(native_base_url) = native_base_url
            && !(inherits_parent_about_blank && moli_url::is_about_blank(&native_base_url))
        {
            return Some(native_base_url);
        }
        if inherits_parent_about_blank {
            return Some(self.document_base_url_for_child_context(handle));
        }
        let Some(snapshot) = entry.cached_snapshot_ref() else {
            return Some(document_url);
        };
        Some(child_snapshot_base_url(snapshot, document_url))
    }

    pub(in crate::native_bridge::context_host) fn push_child_subresource_request_scope(
        &mut self,
        handle: DomHandle,
    ) {
        self.active_child_subresource_request_scopes.push(handle);
    }

    pub(in crate::native_bridge::context_host) fn pop_child_subresource_request_scope(&mut self) {
        let _ = self.active_child_subresource_request_scopes.pop();
    }

    /// Keep the request-attribution stack aligned with the active child
    /// Window marker until the selected resource task's microtasks have run.
    /// Only one selected task can be between body and checkpoint on this
    /// owner-local lane.
    pub(crate) fn defer_child_subresource_request_scope_pop_after_microtasks(&mut self) {
        assert!(
            !self.pending_child_subresource_request_scope_pop,
            "a second child subresource body cannot precede the selected task checkpoint"
        );
        self.pending_child_subresource_request_scope_pop = true;
    }

    /// Consume the deferred request-attribution scope after the task-end
    /// checkpoint. Returns whether a scope was restored.
    pub(crate) fn finish_deferred_child_subresource_request_scope_pop(&mut self) -> bool {
        if !std::mem::take(&mut self.pending_child_subresource_request_scope_pop) {
            return false;
        }
        self.pop_child_subresource_request_scope();
        true
    }

    pub(crate) fn enter_child_async_continuation_scope<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> v8::Local<'s, v8::Value> {
        self.push_child_subresource_request_scope(handle);
        let global = scope.get_current_context().global(scope);
        let previous = get_private_value(scope, global, ACTIVE_CHILD_WINDOW_HANDLE_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into());
        let handle_value = v8::BigInt::new_from_u64(scope, handle.index() as u64);
        set_private_value(
            scope,
            global,
            ACTIVE_CHILD_WINDOW_HANDLE_SLOT,
            handle_value.into(),
        );
        previous
    }

    pub(crate) fn child_browsing_context_target_origin(&self, handle: DomHandle) -> Option<String> {
        self.child_browsing_context_window_origin(handle)
    }

    pub(crate) fn child_browsing_context_web_storage_scope(
        &mut self,
        handle: DomHandle,
        top_origin: &str,
    ) -> Option<WebStorageScope> {
        let mut ancestry = Vec::new();
        let mut current = Some(handle);
        while let Some(handle) = current {
            if !self.child_browsing_contexts.contains_key(&handle) {
                return None;
            }
            ancestry.push(handle);
            if ancestry.len() > self.child_browsing_contexts.len() {
                return None;
            }
            current = self.child_browsing_context_parent_handle(handle);
        }

        let mut inherited_scope = self.web_storage_scope_for_top_document_origin(top_origin);
        for handle in ancestry.into_iter().rev() {
            inherited_scope =
                self.child_browsing_context_web_storage_scope_with_parent(handle, inherited_scope)?;
        }
        Some(inherited_scope)
    }

    pub(crate) fn child_browsing_context_network_partition_key(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        let current_url = self.child_browsing_context_current_url(handle)?;
        let nonce = entry.document_credentialless_storage_nonce().or_else(|| {
            self.child_web_storage_opaque_context_nonces
                .get(&handle)
                .copied()
        });
        let top_level_site = site_for_url(self.document_url());
        let origin = self.child_browsing_context_network_partition_origin(handle)?;
        let relation =
            StoragePartitionRelation::from_sites(&site_for_url(&current_url), &top_level_site);
        let mut key = MoliStorageKey::new(origin, top_level_site, nonce, relation);
        if self.child_browsing_context_ancestor_chain_is_cross_site(handle) {
            key = key.with_cross_site_ancestor();
        }
        Some(key.serialized_storage_key())
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_navigation_network_partition_key(
        &self,
        handle: DomHandle,
        bootstrap: &ChildBrowsingContextBootstrap,
        credentialless_storage_nonce: Option<OpaqueOriginNonce>,
    ) -> Option<String> {
        let top_level_site = site_for_url(self.document_url());
        let origin =
            self.child_browsing_context_navigation_network_partition_origin(handle, bootstrap)?;
        let target_url = Self::child_browsing_context_bootstrap_url(bootstrap)?;
        let relation =
            StoragePartitionRelation::from_sites(&site_for_url(&target_url), &top_level_site);
        let mut key = MoliStorageKey::new(
            origin,
            top_level_site,
            credentialless_storage_nonce,
            relation,
        );
        if self.child_browsing_context_ancestor_chain_is_cross_site(handle) {
            key = key.with_cross_site_ancestor();
        }
        Some(key.serialized_storage_key())
    }

    fn child_browsing_context_ancestor_chain_is_cross_site(&self, handle: DomHandle) -> bool {
        let top_level_site = site_for_url(self.document_url());
        let mut current = self.child_browsing_context_parent_handle(handle);
        let mut depth = 0;
        while let Some(parent) = current {
            let Some(parent_url) = self.child_browsing_context_current_url(parent) else {
                return true;
            };
            if site_for_url(&parent_url) != top_level_site {
                return true;
            }
            depth += 1;
            if depth > self.child_browsing_contexts.len() {
                return true;
            }
            current = self.child_browsing_context_parent_handle(parent);
        }
        false
    }

    fn child_browsing_context_navigation_network_partition_origin(
        &self,
        handle: DomHandle,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> Option<String> {
        let sandbox = self.child_browsing_context_sandbox_policy_from_owner(handle);
        if sandbox.forces_opaque_origin {
            return Some("null".to_owned());
        }
        if bootstrap.security_origin_inherited() {
            return Some(self.child_browsing_context_creator_network_partition_origin(handle));
        }
        let parent_origin = self.child_browsing_context_creator_network_partition_origin(handle);
        let url = Self::child_browsing_context_bootstrap_url(bootstrap)?;
        Some(
            moli_url::origin_ascii_serialization_with_about_blank_inheritance(&url, &parent_origin),
        )
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_network_partition_origin(
        &self,
        handle: DomHandle,
    ) -> Option<String> {
        let url = self.child_browsing_context_current_url(handle)?;
        self.child_browsing_context_document_origin_for_url(handle, &url)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_document_origin_for_url(
        &self,
        handle: DomHandle,
        url: &Url,
    ) -> Option<String> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        if entry.document_sandbox_forces_opaque_origin() {
            return Some("null".to_owned());
        }
        if entry.security_origin_inherited() {
            return Some(self.child_browsing_context_creator_network_partition_origin(handle));
        }
        let parent_origin = self.child_browsing_context_creator_network_partition_origin(handle);
        Some(moli_url::origin_ascii_serialization_with_about_blank_inheritance(url, &parent_origin))
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_creator_network_partition_origin(
        &self,
        handle: DomHandle,
    ) -> String {
        self.child_browsing_context_parent_handle(handle)
            .and_then(|parent| self.child_browsing_context_network_partition_origin(parent))
            .unwrap_or_else(|| moli_url::origin_ascii_serialization(self.document_url()))
    }

    pub(crate) fn active_storage_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        active_child_handle: Option<DomHandle>,
    ) -> ActiveStorageContext {
        self.ambient_storage_context(scope, active_child_handle)
    }

    pub(crate) fn storage_context_for_window_execution_context_identity(
        &mut self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<ActiveStorageContext> {
        if !self.window_execution_context_identity_is_current(identity) {
            return None;
        }
        match identity.dispatch_scope() {
            OwnerDispatchScope::Top => Some(self.top_document_storage_context()),
            OwnerDispatchScope::Child(handle) => {
                self.storage_context_for_child_browsing_context(handle)
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                self.storage_context_for_lightweight_popup(popup_id)
            }
        }
    }

    pub(crate) fn secure_context_url_for_window_execution_context_identity(
        &self,
        identity: WindowExecutionContextIdentity,
    ) -> Option<Url> {
        if !self.window_execution_context_identity_is_current(identity) {
            return None;
        }
        match identity.dispatch_scope() {
            OwnerDispatchScope::Top => Some(self.document_url().clone()),
            OwnerDispatchScope::Child(handle) => {
                self.child_browsing_context_secure_context_url(handle)
            }
            OwnerDispatchScope::LightweightPopup(popup_id) => {
                self.lightweight_popup_document_url(popup_id)
            }
        }
    }

    pub(crate) fn storage_context_for_child_browsing_context(
        &mut self,
        handle: DomHandle,
    ) -> Option<ActiveStorageContext> {
        let top_origin = moli_url::origin_ascii_serialization(self.document_url());
        self.child_browsing_context_web_storage_scope(handle, &top_origin)
            .map(|scope| ActiveStorageContext::new(scope.into_storage_context()))
    }

    pub(crate) fn storage_context_for_lightweight_popup(
        &mut self,
        popup_id: u64,
    ) -> Option<ActiveStorageContext> {
        self.lightweight_popup_bound_web_storage_scope(popup_id)
            .map(|scope| ActiveStorageContext::new(scope.into_storage_context()))
    }

    pub(crate) fn top_document_storage_context(&mut self) -> ActiveStorageContext {
        let top_origin = moli_url::origin_ascii_serialization(self.document_url());
        ActiveStorageContext::new(
            self.web_storage_scope_for_top_document_origin(&top_origin)
                .into_storage_context(),
        )
    }

    pub(crate) fn ambient_storage_context(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        active_child_handle: Option<DomHandle>,
    ) -> ActiveStorageContext {
        let active_child_handle = active_child_handle
            .or_else(|| self.active_child_subresource_request_scopes.last().copied());
        if let Some(handle) = active_child_handle
            && let Some(context) = self.storage_context_for_child_browsing_context(handle)
        {
            return context;
        }
        if let Some(popup_id) = crate::native_bridge::active_lightweight_popup_id(scope)
            && let Some(context) = self.storage_context_for_lightweight_popup(popup_id)
        {
            return context;
        }
        self.top_document_storage_context()
    }

    pub(crate) fn top_web_storage_scope(&mut self) -> WebStorageScope {
        let top_origin = moli_url::origin_ascii_serialization(self.document_url());
        self.web_storage_scope_for_top_document_origin(&top_origin)
    }

    pub(crate) fn web_storage_scope_for_url_as_first_party(
        &mut self,
        url: &Url,
    ) -> WebStorageScope {
        let origin = moli_url::origin_ascii_serialization(url);
        let opaque_nonce = moli_storage_key::url_needs_opaque_nonce(url).then(|| {
            self.browser_context_runtime
                .next_web_storage_opaque_context_nonce()
        });
        let storage_key = MoliStorageKey::first_party_from_url(url, opaque_nonce);
        WebStorageScope::new(origin, storage_key)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_web_storage_scope_with_parent(
        &mut self,
        handle: DomHandle,
        parent_scope: WebStorageScope,
    ) -> Option<WebStorageScope> {
        let parent_has_cross_site_ancestor = parent_scope.storage_key().has_cross_site_ancestor();
        if self.child_browsing_context_has_opaque_origin(handle) {
            let url = self.child_browsing_context_current_url(handle)?;
            let top_level_site = site_for_url(self.document_url());
            let relation = if moli_url::is_about_blank(&url) {
                StoragePartitionRelation::Unknown
            } else {
                StoragePartitionRelation::from_sites(&site_for_url(&url), &top_level_site)
            };
            let storage_key = MoliStorageKey::new(
                "null".to_owned(),
                top_level_site,
                Some(self.ensure_child_web_storage_opaque_context_nonce(handle)),
                relation,
            )
            .with_cross_site_ancestor();
            return Some(WebStorageScope::new("null".to_owned(), storage_key));
        }
        let (security_origin_inherited, credentialless_storage_nonce) = {
            let entry = self.child_browsing_contexts.get(&handle)?;
            (
                entry.security_origin_inherited(),
                entry.document_credentialless_storage_nonce(),
            )
        };
        if let Some(nonce) = credentialless_storage_nonce {
            let origin = if security_origin_inherited {
                parent_scope.origin().to_owned()
            } else {
                let url = self.child_browsing_context_current_url(handle)?;
                moli_url::origin_ascii_serialization_with_about_blank_inheritance(
                    &url,
                    parent_scope.origin(),
                )
            };
            let top_level_site = site_for_url(self.document_url());
            let mut storage_key = web_storage_key_for_origin_and_top_level_site_with_nonce(
                &origin,
                top_level_site,
                Some(nonce),
            );
            if parent_has_cross_site_ancestor {
                storage_key = storage_key.with_cross_site_ancestor();
            }
            return Some(WebStorageScope::new(origin, storage_key));
        }
        if security_origin_inherited {
            return Some(parent_scope);
        }
        let url = self.child_browsing_context_current_url(handle)?;
        let origin = moli_url::origin_ascii_serialization_with_about_blank_inheritance(
            &url,
            parent_scope.origin(),
        );
        let top_level_site = site_for_url(self.document_url());
        let mut storage_key =
            web_storage_key_for_origin_and_top_level_site_with_nonce(&origin, top_level_site, None);
        if parent_has_cross_site_ancestor {
            storage_key = storage_key.with_cross_site_ancestor();
        }
        Some(WebStorageScope::new(origin, storage_key))
    }

    pub(in crate::native_bridge::context_host) fn detached_child_frame_web_storage_scope(
        &mut self,
        document: &crate::dom::native::NativeDom,
        handle: DomHandle,
        url: &Url,
        security_origin_inherited: bool,
        parent_scope: WebStorageScope,
    ) -> WebStorageScope {
        if document
            .get_attribute(handle, "sandbox")
            .is_some_and(|sandbox| sandbox_attribute_forces_opaque_origin(&sandbox))
            || url.scheme() == "data"
        {
            let top_level_site = site_for_url(self.document_url());
            let relation = if moli_url::is_about_blank(url) {
                StoragePartitionRelation::Unknown
            } else {
                StoragePartitionRelation::from_sites(&site_for_url(url), &top_level_site)
            };
            let storage_key = MoliStorageKey::new(
                "null".to_owned(),
                top_level_site,
                Some(
                    self.browser_context_runtime
                        .next_web_storage_opaque_context_nonce(),
                ),
                relation,
            );
            return WebStorageScope::new("null".to_owned(), storage_key);
        }
        if security_origin_inherited {
            return parent_scope;
        }
        let origin = moli_url::origin_ascii_serialization_with_about_blank_inheritance(
            url,
            parent_scope.origin(),
        );
        let top_level_site = site_for_url(self.document_url());
        WebStorageScope::new(
            origin.clone(),
            web_storage_key_for_origin_and_top_level_site_with_nonce(&origin, top_level_site, None),
        )
    }

    fn web_storage_scope_for_top_document_origin(&mut self, origin: &str) -> WebStorageScope {
        if let Some(storage_key) = self.top_level_storage_key.as_ref() {
            return WebStorageScope::new(storage_key.origin().to_owned(), storage_key.clone());
        }
        let opaque_nonce = moli_storage_key::url_needs_opaque_nonce(self.document_url())
            .then(|| self.ensure_web_storage_opaque_context_nonce());
        let storage_key = MoliStorageKey::from_url_and_top_level_site(
            self.document_url(),
            site_for_url(self.document_url()),
            opaque_nonce,
        );
        WebStorageScope::new(origin.to_owned(), storage_key)
    }

    fn ensure_web_storage_opaque_context_nonce(&mut self) -> OpaqueOriginNonce {
        if let Some(nonce) = self.web_storage_opaque_context_nonce {
            return nonce;
        }
        let nonce = self
            .browser_context_runtime
            .next_web_storage_opaque_context_nonce();
        self.web_storage_opaque_context_nonce = Some(nonce);
        nonce
    }

    fn ensure_child_web_storage_opaque_context_nonce(
        &mut self,
        handle: DomHandle,
    ) -> OpaqueOriginNonce {
        if let Some(nonce) = self
            .child_web_storage_opaque_context_nonces
            .get(&handle)
            .copied()
        {
            return nonce;
        }
        let nonce = self
            .browser_context_runtime
            .next_web_storage_opaque_context_nonce();
        self.child_web_storage_opaque_context_nonces
            .insert(handle, nonce);
        nonce
    }

    pub(in crate::native_bridge::context_host) fn ensure_top_document_credentialless_storage_nonce(
        &mut self,
    ) -> OpaqueOriginNonce {
        unsafe { &mut *self.runtime }
            .ensure_credentialless_storage_nonce(&self.browser_context_runtime)
    }

    pub(in crate::native_bridge::context_host) fn child_document_credentialless_storage_nonce(
        &mut self,
        credentialless: bool,
    ) -> Option<OpaqueOriginNonce> {
        credentialless.then(|| self.ensure_top_document_credentialless_storage_nonce())
    }

    pub(crate) fn child_browsing_context_is_same_origin_with_top(&self, handle: DomHandle) -> bool {
        self.top_window_can_access_child(handle)
    }

    pub(crate) fn child_browsing_context_has_opaque_origin(&self, handle: DomHandle) -> bool {
        if self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.document_sandbox_forces_opaque_origin())
        {
            return true;
        }
        if self
            .child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.security_origin_inherited())
        {
            return false;
        }
        self.child_browsing_context_current_url(handle)
            .is_some_and(|url| url.scheme() == "data")
    }

    pub(crate) fn child_browsing_context_scripting_enabled(&self, handle: DomHandle) -> bool {
        !self.script_execution_disabled()
            && self
                .child_browsing_contexts
                .get(&handle)
                .is_none_or(|entry| entry.document_sandbox_allows_scripts())
    }

    pub(crate) fn document_scripting_enabled(&self, document_handle: DomHandle) -> bool {
        self.child_browsing_context_host_for_document_handle(document_handle)
            .map(|handle| self.child_browsing_context_scripting_enabled(handle))
            .unwrap_or(true)
    }

    pub(crate) fn child_browsing_context_inherits_parent_origin(&self, handle: DomHandle) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.security_origin_inherited())
            && !self.child_browsing_context_has_opaque_origin(handle)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_sandbox_policy_from_owner(
        &self,
        handle: DomHandle,
    ) -> crate::document_runtime::DocumentSandboxPolicy {
        document_sandbox_policy_from_attribute(
            self.dom_host().get_attribute(handle, "sandbox").as_deref(),
        )
    }

    pub(crate) fn child_browsing_context_document_credentialless(&self, handle: DomHandle) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.document_credentialless())
    }

    pub(crate) fn child_browsing_context_document_credentialless_for_owner(
        &self,
        handle: DomHandle,
        owner_credentialless: bool,
    ) -> bool {
        owner_credentialless
            || self
                .child_browsing_context_parent_handle(handle)
                .is_some_and(|parent| self.child_browsing_context_document_credentialless(parent))
    }

    pub(crate) fn child_browsing_context_popup_opener_sandbox_policy(
        &self,
        handle: DomHandle,
    ) -> Option<crate::document_runtime::DocumentSandboxPolicy> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        let policy = entry.document_sandbox_policy();
        (policy.sandboxes_document_domain && !policy.allows_popups_to_escape).then_some(policy)
    }

    pub(crate) fn child_browsing_context_allows_top_navigation(&self, handle: DomHandle) -> bool {
        self.dom_host()
            .get_attribute(handle, "sandbox")
            .is_none_or(|sandbox| sandbox_attribute_allows_top_navigation(&sandbox))
    }
}

fn web_storage_key_for_origin_and_top_level_site_with_nonce(
    origin: &str,
    top_level_site: String,
    opaque_nonce: Option<OpaqueOriginNonce>,
) -> MoliStorageKey {
    let relation = site_for_serialized_storage_origin(origin)
        .map(|current_site| StoragePartitionRelation::from_sites(&current_site, &top_level_site))
        .unwrap_or(StoragePartitionRelation::Unknown);
    MoliStorageKey::new(origin.to_owned(), top_level_site, opaque_nonce, relation)
}

fn child_snapshot_base_url(snapshot: &ChildBrowsingContextSnapshot, fallback: Url) -> Url {
    let document = crate::parser::HtmlParser.parse(snapshot.url.clone(), snapshot.markup.clone());
    document
        .document()
        .map(|doc| doc.base_url().clone())
        .unwrap_or(fallback)
}

#[cfg(test)]
fn web_storage_area_key_for_origin_and_top_level_site(
    origin: &str,
    top_level_site: &str,
    opaque_nonce: Option<OpaqueOriginNonce>,
) -> String {
    let relation = site_for_serialized_storage_origin(origin)
        .map(|current_site| StoragePartitionRelation::from_sites(&current_site, top_level_site))
        .unwrap_or(StoragePartitionRelation::Unknown);
    let storage_key = MoliStorageKey::new(
        origin.to_owned(),
        top_level_site.to_owned(),
        opaque_nonce,
        relation,
    );
    web_storage_area_key_for_storage_key(&storage_key)
}

fn site_for_serialized_storage_origin(origin: &str) -> Option<String> {
    if origin == "null" {
        return None;
    }
    if origin == "file://" {
        return Some("file://".to_owned());
    }
    Url::parse(origin).ok().map(|url| site_for_url(&url))
}

pub(in crate::native_bridge::context_host) fn sandbox_attribute_forces_opaque_origin(
    value: &str,
) -> bool {
    !sandbox_attribute_allows_same_origin(value)
}

pub(in crate::native_bridge::context_host) fn document_sandbox_policy_from_attribute(
    value: Option<&str>,
) -> crate::document_runtime::DocumentSandboxPolicy {
    let Some(value) = value else {
        return crate::document_runtime::DocumentSandboxPolicy::default();
    };
    crate::document_runtime::DocumentSandboxPolicy {
        forces_opaque_origin: sandbox_attribute_forces_opaque_origin(value),
        allows_scripts: sandbox_attribute_allows_scripts(value),
        allows_popups_to_escape: sandbox_attribute_allows_popups_to_escape(value),
        sandboxes_document_domain: sandbox_attribute_sets_document_domain_flag(value),
    }
}

fn sandbox_attribute_sets_document_domain_flag(_value: &str) -> bool {
    // allow-same-origin affects origin opacity, but a sandbox attribute still
    // carries the sandboxed document.domain flag.
    true
}

fn sandbox_attribute_allows_same_origin(value: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("allow-same-origin"))
}

fn sandbox_attribute_allows_scripts(value: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("allow-scripts"))
}

fn sandbox_attribute_allows_top_navigation(value: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("allow-top-navigation"))
}

fn sandbox_attribute_allows_popups_to_escape(value: &str) -> bool {
    value
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case("allow-popups-to-escape-sandbox"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_storage_area_key_uses_explicit_first_party_storage_key() {
        assert_eq!(
            web_storage_area_key_for_origin_and_top_level_site(
                "https://app.example.test",
                "https://example.test",
                None
            ),
            "storage-key:v1;origin=https://app.example.test;top-level-site=https://example.test"
        );
    }

    #[test]
    fn web_storage_area_key_partitions_third_party_origin_by_top_level_site() {
        assert_eq!(
            web_storage_area_key_for_origin_and_top_level_site(
                "https://cdn.example.test",
                "https://other.test",
                None
            ),
            "storage-key:v1;origin=https://cdn.example.test;top-level-site=https://other.test"
        );
    }

    #[test]
    fn web_storage_area_key_includes_opaque_nonce() {
        assert_eq!(
            web_storage_area_key_for_origin_and_top_level_site(
                "null",
                "https://example.test",
                Some(OpaqueOriginNonce::new(7))
            ),
            "storage-key:v1;origin=null;top-level-site=https://example.test;opaque-nonce=7"
        );
    }

    #[test]
    fn sandbox_attribute_sets_document_domain_flag_even_with_allow_same_origin() {
        assert!(sandbox_attribute_sets_document_domain_flag(""));
        assert!(sandbox_attribute_sets_document_domain_flag("allow-scripts"));
        assert!(sandbox_attribute_sets_document_domain_flag(
            "allow-scripts allow-same-origin"
        ));
        assert!(sandbox_attribute_sets_document_domain_flag(
            "allow-scripts ALLOW-SAME-ORIGIN"
        ));
    }
}
