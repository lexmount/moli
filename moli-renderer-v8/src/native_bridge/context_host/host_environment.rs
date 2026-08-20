use std::{collections::HashSet, rc::Rc};

use super::{
    JsContextHost,
    permissions::{permission_names_match, permission_override_name},
};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::{
        DocumentReadyState, DomStylesheetOwnerChange, DomStylesheetOwnerChangeKind, Node,
    },
    native_bridge::element::label_control_handle,
    network::context::DocumentResourceLoader,
    style_engine::{
        AdoptedStyleSheetInstallation, CssCustomPropertyRegistration,
        CssCustomPropertyRegistrationError, OwnedStyleSourceDocumentContext, StyleAttributeImpact,
        StyleMutationEffect, StyleViewport, StyloComputedStyleInputs,
        StyloDocumentComputedStyleInputCacheKey, StyloPreparedComputedStyleInputs,
        StyloStylesheetSource,
    },
    {
        RendererWebStorageHandles,
        context_bootstrap::{
            SharedStorageBucketStore, SharedWebStorageStore, WeakIndexedDbManager,
        },
        types::SubresourceResourceType,
    },
};
use dom::ElementState as StyloElementState;
use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;

fn push_unique_handle(handles: &mut Vec<DomHandle>, handle: DomHandle) {
    if !handles.contains(&handle) {
        handles.push(handle);
    }
}

fn hover_chain_for_input_hit(runtime: &JsContextHost, hit: Option<DomHandle>) -> Vec<DomHandle> {
    let mut hovered = Vec::new();
    let mut visited = HashSet::new();
    let mut current = hit;
    while let Some(handle) = current {
        if !visited.insert(handle) {
            break;
        }
        let Some(node) = runtime.dom_host().node(handle) else {
            break;
        };
        if node.is_element() {
            push_unique_handle(&mut hovered, handle);
            if let Some(control) = label_control_handle(runtime, handle) {
                push_unique_handle(&mut hovered, control);
            }
        }
        if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
            current = Some(slot);
            continue;
        }
        if let Some(parent) = runtime.dom_host().parent_node(handle) {
            current = if runtime.dom_host().is_shadow_root(parent) {
                runtime.dom_host().shadow_root_host(parent)
            } else {
                Some(parent)
            };
            continue;
        }
        current = node
            .is_document()
            .then(|| runtime.child_browsing_context_host_for_document_handle(handle))
            .flatten();
    }
    hovered
}

impl JsContextHost {
    pub(crate) fn documents_with_adopted_style_sheets(&self) -> Vec<DomHandle> {
        self.style_engine.documents_with_adopted_style_sheets()
    }

    pub(crate) fn create_live_stylesheet(
        &self,
        document: DomHandle,
        css_text: &str,
        base_url: url::Url,
        allow_import_rules: style::stylesheets::AllowImportRules,
    ) -> crate::live_stylesheet::LiveStylesheetRef {
        let quirks_mode = self
            .dom_host()
            .node(document)
            .and_then(crate::dom::native::Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(style::context::QuirksMode::NoQuirks);
        self.live_stylesheets.create(
            css_text,
            base_url,
            quirks_mode,
            allow_import_rules,
            self.style_engine.author_shared_lock(),
        )
    }

    fn create_inline_live_stylesheet_with_shared_initial_contents(
        &self,
        document: DomHandle,
        css_text: &str,
        base_url: url::Url,
    ) -> crate::live_stylesheet::LiveStylesheetRef {
        let quirks_mode = self
            .dom_host()
            .node(document)
            .and_then(crate::dom::native::Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(style::context::QuirksMode::NoQuirks);
        self.live_stylesheets
            .create_inline_with_shared_initial_contents(
                css_text,
                base_url,
                quirks_mode,
                self.style_engine.author_shared_lock(),
            )
    }

    fn create_linked_live_stylesheet_template(
        &self,
        document: DomHandle,
        css_text: &str,
        base_url: url::Url,
    ) -> crate::live_stylesheet::LiveStylesheetRef {
        let quirks_mode = self
            .dom_host()
            .node(document)
            .and_then(crate::dom::native::Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(style::context::QuirksMode::NoQuirks);
        self.live_stylesheets
            .create_linked_resource_template_with_shared_initial_contents(
                css_text,
                base_url,
                quirks_mode,
                self.style_engine.author_shared_lock(),
            )
    }

    fn create_live_stylesheet_from_shared_initial_contents(
        &self,
        shared_contents: std::sync::Arc<crate::live_stylesheet::SharedStylesheetContents>,
    ) -> crate::live_stylesheet::LiveStylesheetRef {
        self.live_stylesheets.create_from_shared_initial_contents(
            shared_contents,
            self.style_engine.author_shared_lock(),
        )
    }

    fn create_live_stylesheet_from_parsed_source(
        &self,
        document: DomHandle,
        source: &crate::style_engine::StyloStylesheetSource,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        let parsed_stylesheet = source.parsed_stylesheet()?;
        let quirks_mode = self
            .dom_host()
            .node(document)
            .and_then(crate::dom::native::Node::as_document)
            .map(|document| document.quirks_mode())
            .unwrap_or(style::context::QuirksMode::NoQuirks);
        Some(self.live_stylesheets.create_from_parsed_stylesheet(
            &parsed_stylesheet,
            source.base_url().clone(),
            quirks_mode,
            style::stylesheets::AllowImportRules::Yes,
        ))
    }

    pub(crate) fn prepare_linked_stylesheet_resource(
        &self,
        owner: DomHandle,
        css_text: &str,
        base_url: url::Url,
        sheet_url: url::Url,
        origin_clean: bool,
    ) -> Option<crate::document_runtime::PreparedLinkedStylesheetResource> {
        let document = self.dom_host().owner_document_handle(owner)?;
        let template =
            self.create_linked_live_stylesheet_template(document, css_text, base_url.clone());
        let (has_import_rules, import_urls) = template.top_level_import_state();
        if !has_import_rules {
            template.share_initial_contents();
        }
        let source = crate::style_engine::StyloStylesheetSource::from_live_stylesheet(&template)
            .with_sheet_url(sheet_url)
            .with_origin_clean(origin_clean);
        Some(
            crate::document_runtime::PreparedLinkedStylesheetResource::new(
                source,
                import_urls,
                has_import_rules,
            ),
        )
    }

    pub(crate) fn live_stylesheet(
        &self,
        id: crate::live_stylesheet::StylesheetId,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        self.live_stylesheets.get(id)
    }

    pub(crate) fn create_live_stylesheet_wrapper_lease(
        &self,
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
    ) -> (
        crate::live_stylesheet::StylesheetWrapperLeaseId,
        crate::live_stylesheet::StylesheetWrapperLease,
    ) {
        self.live_stylesheets.create_wrapper_lease(stylesheet)
    }

    pub(crate) fn replace_live_stylesheet_wrapper_lease(
        &self,
        id: crate::live_stylesheet::StylesheetWrapperLeaseId,
        stylesheet: Option<crate::live_stylesheet::LiveStylesheetRef>,
    ) -> bool {
        self.live_stylesheets.replace_wrapper_lease(id, stylesheet)
    }

    pub(crate) fn bind_live_stylesheet_rule_wrapper(
        &self,
        existing_id: Option<crate::live_stylesheet::StylesheetRuleWrapperLeaseId>,
        stylesheet: &crate::live_stylesheet::LiveStylesheetRef,
        path: Vec<usize>,
    ) -> Option<(
        crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
        Option<crate::live_stylesheet::StylesheetRuleWrapperLease>,
    )> {
        self.live_stylesheets
            .bind_rule_wrapper(existing_id, stylesheet, path)
    }

    pub(crate) fn release_live_stylesheet_rule_wrapper(
        &self,
        id: crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
    ) -> bool {
        self.live_stylesheets.release_rule_wrapper(id)
    }

    pub(crate) fn retained_live_stylesheet_rule_wrapper_snapshot_for_detach(
        &self,
        id: crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
    ) -> Option<moli_css_parse::CssRuleSnapshot> {
        self.live_stylesheets
            .retained_rule_wrapper_snapshot_for_detach(id)
    }

    pub(crate) fn with_attached_live_stylesheet_rule_wrapper<R>(
        &self,
        id: crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: crate::live_stylesheet::StylesheetId,
        read: impl FnOnce(&crate::live_stylesheet::StylesheetRuleWrapperBinding) -> R,
    ) -> Option<R> {
        self.live_stylesheets
            .with_attached_rule_wrapper(id, expected_stylesheet_id, read)
    }

    pub(crate) fn attached_live_stylesheet_rule_wrapper_path(
        &self,
        id: crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> Option<Vec<usize>> {
        self.live_stylesheets
            .attached_rule_wrapper_path(id, expected_stylesheet_id)
    }

    pub(crate) fn imported_live_stylesheet_for_rule_wrapper(
        &self,
        id: crate::live_stylesheet::StylesheetRuleWrapperLeaseId,
        expected_stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        self.live_stylesheets
            .imported_stylesheet_for_rule_wrapper(id, expected_stylesheet_id)
    }

    pub(crate) fn install_live_stylesheet_import_graph(
        &self,
        root: crate::document_runtime::ConnectedStyleImportRoot,
        responses: &[crate::live_stylesheet::LiveStylesheetImportResponse],
    ) -> Option<bool> {
        self.live_stylesheets.install_import_graph(
            root.stylesheet_id,
            root.contents_revision,
            root.import_generation,
            responses,
            root.root_resource_url.as_ref(),
        )
    }

    pub(crate) fn refresh_live_stylesheet_after_import_graph(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        let refreshed = self.style_engine.refresh_owner_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        ) || self.style_engine.refresh_linked_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        );
        if refreshed {
            self.clear_layout_rect_cache();
            self.queue_stylesheet_source_css_projection(owner);
        }
        refreshed
    }

    fn install_owner_live_stylesheet(&mut self, owner: DomHandle) {
        let dom_host: *const crate::dom::native::DomHost = self.dom_host();
        if self
            .style_engine
            .owner_live_stylesheet_with_host(unsafe { &*dom_host }, owner)
            .is_some()
        {
            return;
        }
        let Some(source) = self.style_engine.owner_style_sheet_processing_source(owner) else {
            return;
        };
        let Some(document) = unsafe { &*dom_host }.owner_document_handle(owner) else {
            return;
        };
        let stylesheet = self.create_inline_live_stylesheet_with_shared_initial_contents(
            document,
            source.css_text(),
            source.parser_base().clone(),
        );
        let media = unsafe { &*dom_host }
            .get_attribute(owner, "media")
            .unwrap_or_default();
        stylesheet.set_media_text(&media);
        self.style_engine.install_owner_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet,
        );
    }

    fn refresh_owner_live_stylesheet_attributes(&mut self, owner: DomHandle) {
        let Some(stylesheet) = self.owner_live_stylesheet(owner) else {
            return;
        };
        let dom_host: *const crate::dom::native::DomHost = self.dom_host();
        let media = unsafe { &*dom_host }
            .get_attribute(owner, "media")
            .unwrap_or_default();
        stylesheet.set_media_text(&media);
        self.style_engine.refresh_owner_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet.id(),
        );
    }

    fn refresh_linked_live_stylesheet_attributes(&mut self, owner: DomHandle) {
        let Some(stylesheet) = self.linked_live_stylesheet(owner) else {
            return;
        };
        let media = self
            .dom_host()
            .get_attribute(owner, "media")
            .unwrap_or_default();
        let media_changed = stylesheet.set_media_text(&media).affects_cascade();
        let disabled_changed = stylesheet
            .set_disabled(self.dom_host().get_attribute(owner, "disabled").is_some())
            .affects_cascade();
        if !(media_changed || disabled_changed) {
            return;
        }
        let dom_host = self.dom_host() as *const _;
        self.style_engine.refresh_linked_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet.id(),
        );
        self.clear_layout_rect_cache();
    }

    fn apply_page_network_policy_to_document_loader(&self, loader: &DocumentResourceLoader) {
        loader
            .request_client()
            .set_extra_http_headers(&self.extra_http_headers);
        loader
            .request_client()
            .set_network_offline(self.network_offline);
        loader
            .request_client()
            .set_blocked_url_patterns(&self.blocked_url_patterns);
    }

    /// Registers the exact resource authority of the initial committed main
    /// Document.
    pub(crate) fn register_main_document_resource_loader(
        &mut self,
        loader: &DocumentResourceLoader,
    ) {
        let loader = loader.clone();
        self.apply_page_network_policy_to_document_loader(&loader);
        let owner = self
            .current_main_document_task_owner()
            .expect("main Document must have an owner before its resource loader is installed");
        let owner = crate::native_bridge::WindowDocumentOwner::Frame(owner);
        self.document_resource_loaders.register(owner, loader);
        unsafe { &mut *self.runtime }
            .bind_document_resource_loader(self.document_resource_loaders.clone(), owner);
    }

    /// Replaces only the transport view of the already-registered main
    /// Document authority.
    pub(crate) fn replace_main_document_resource_transport(
        &mut self,
        loader: &DocumentResourceLoader,
    ) {
        let loader = loader.clone();
        self.apply_page_network_policy_to_document_loader(&loader);
        let owner = self
            .current_main_document_task_owner()
            .expect("main Document must retain an owner while replacing its resource transport");
        self.document_resource_loaders.replace_transport_view(
            crate::native_bridge::WindowDocumentOwner::Frame(owner),
            loader,
        );
    }

    /// Installs the storage-partition and browsing-context Web Storage stores.
    ///
    /// These handles deliberately do not travel through `ResourceRequestClient`: network
    /// backend replacement must leave a live Document's storage identity
    /// untouched.
    pub(crate) fn set_web_storage_handles(&mut self, handles: &RendererWebStorageHandles) {
        self.web_storage_store = handles.local_storage();
        self.session_storage_store = handles.session_storage();
    }

    pub(crate) fn set_stored_document_start_scripts(
        &mut self,
        scripts: &[crate::DocumentStartScript],
    ) {
        self.stored_document_start_scripts = scripts.to_vec();
    }

    pub(crate) fn stored_default_document_start_scripts(&self) -> Vec<crate::DocumentStartScript> {
        self.stored_document_start_scripts
            .iter()
            .filter(|script| script.world_name.is_none())
            .cloned()
            .collect()
    }

    pub(crate) fn stored_document_start_scripts(&self) -> Vec<crate::DocumentStartScript> {
        self.stored_document_start_scripts.clone()
    }

    pub(crate) fn stored_default_runtime_binding_names(&self) -> Vec<String> {
        self.stored_runtime_bindings
            .iter()
            .filter(|binding| binding.execution_context_name.is_none())
            .map(|binding| binding.name.clone())
            .collect()
    }

    pub(crate) fn browser_cookie_enabled(&self) -> bool {
        unsafe { (&*self.runtime).host_document().browser_cookie_enabled() }
    }

    pub(crate) fn web_storage_store(&self) -> SharedWebStorageStore {
        self.web_storage_store.clone()
    }

    pub(crate) fn session_storage_store(&self) -> SharedWebStorageStore {
        self.session_storage_store.clone()
    }

    pub(crate) fn set_indexed_db_manager(&mut self, manager: Option<WeakIndexedDbManager>) {
        self.indexed_db_manager = manager;
    }

    pub(crate) fn indexed_db_manager(&self) -> Option<WeakIndexedDbManager> {
        self.indexed_db_manager.clone()
    }

    pub(crate) fn set_storage_bucket_store(&mut self, store: SharedStorageBucketStore) {
        self.storage_bucket_store = store;
    }

    pub(crate) fn storage_bucket_store(&self) -> SharedStorageBucketStore {
        self.storage_bucket_store.clone()
    }

    pub(crate) fn document_resource_loader_for_owner(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<DocumentResourceLoader> {
        self.document_resource_loaders
            .get(crate::native_bridge::WindowDocumentOwner::Frame(owner))
    }

    pub(crate) fn current_main_document_resource_loader(&self) -> Option<DocumentResourceLoader> {
        self.document_resource_loader_for_owner(self.current_main_document_task_owner()?)
    }

    pub(crate) fn document_resource_loader_for_window_owner(
        &self,
        owner: crate::native_bridge::WindowDocumentOwner,
    ) -> Option<DocumentResourceLoader> {
        self.document_resource_loaders.get(owner)
    }

    pub(crate) fn document_resource_loader_for_dispatch_scope(
        &self,
        dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    ) -> Option<DocumentResourceLoader> {
        let target = self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)?;
        self.document_resource_loader_for_window_owner(target.owner())
    }

    pub(crate) fn parent_document_resource_loader_for_child_context(
        &self,
        handle: DomHandle,
    ) -> Option<DocumentResourceLoader> {
        match self.child_browsing_context_parent_handle(handle) {
            Some(parent) => {
                let owner = self.current_child_document_task_owner(parent)?;
                self.document_resource_loader_for_owner(owner)
            }
            None => self.current_main_document_resource_loader(),
        }
    }

    /// Registers the one resource authority owned by a committed Document.
    ///
    /// Callers must choose a navigation seed or an exact inherited authority
    /// before entering this boundary. There is deliberately no ambient loader
    /// fallback: using the wrong generation here would let later subresources
    /// escape the committed Document's policy and lifecycle.
    pub(crate) fn register_committed_document_resource_loader(
        &mut self,
        context: crate::network::context::DocumentFetchContext,
        source: crate::network::context::DocumentResourceAuthoritySource,
    ) -> DocumentResourceLoader {
        let owner = context.owner();
        let document_loader = DocumentResourceLoader::for_committed_document(context, source);
        self.document_resource_loaders
            .register(owner, document_loader.clone());
        document_loader
    }

    pub(crate) fn retire_document_resource_loader(
        &mut self,
        owner: crate::native_bridge::WindowDocumentOwner,
    ) -> Option<DocumentResourceLoader> {
        let retired = self.document_resource_loaders.retire(owner)?;
        let registry_id = retired.load_diagnostics().registry_id;
        let (aborted, detached_keepalive) =
            self.retire_document_resource_load_consumers(registry_id);
        tracing::debug!(
            ?owner,
            registry_id,
            aborted,
            detached_keepalive,
            "retired committed Document resource authority and its consumers"
        );
        Some(retired)
    }

    pub(crate) fn retire_all_document_resource_loaders(&mut self) {
        let retired = self.document_resource_loaders.retire_all();
        for loader in retired {
            let registry_id = loader.load_diagnostics().registry_id;
            let _ = self.retire_document_resource_load_consumers(registry_id);
        }
    }

    /// Replaces a Document authority while preserving its LocalWindow loads.
    ///
    /// `document.open()` replaces Moli's exact Document owner but
    /// does not abort an otherwise non-navigating Document's active Fetch,
    /// XHR, image, or WebSocket work. Existing leases move to the replacement
    /// lifecycle registry while retaining their captured request clients and
    /// policy snapshots. Only newly-created loads use the replacement
    /// Document context.
    pub(crate) fn replace_document_resource_loader_for_document_open(
        &mut self,
        retired_owner: crate::native_bridge::WindowDocumentOwner,
        context: crate::network::context::DocumentFetchContext,
        source: crate::network::context::DocumentResourceAuthoritySource,
    ) -> DocumentResourceLoader {
        assert_ne!(
            retired_owner,
            context.owner(),
            "a Document owner transition must allocate a new generation"
        );
        let current_owner = context.owner();
        let retired = self
            .document_resource_loaders
            .get(retired_owner)
            .expect("document.open() requires its exact source resource authority");
        let replacement = DocumentResourceLoader::for_committed_document(context, source);
        let transferred_loads = retired.transfer_existing_loads_to(&replacement);
        self.retire_document_resource_loader(retired_owner)
            .expect("a committed Document transition must retire its exact resource authority");
        self.document_resource_loaders
            .register(current_owner, replacement.clone());
        tracing::debug!(
            ?retired_owner,
            ?current_owner,
            transferred_loads,
            "transferred existing LocalWindow loads across document.open()"
        );
        replacement
    }

    pub(crate) fn replace_main_document_resource_loader(
        &mut self,
        transition: crate::frame_owner_model::MainDocumentOwnerTransition,
    ) {
        let retired_owner =
            crate::native_bridge::WindowDocumentOwner::Frame(transition.retired_owner());
        let current = self
            .document_resource_loaders
            .get(retired_owner)
            .expect("document.open() requires its exact source resource authority");
        let document_url = self.document_url().clone();
        let base_url = self.document_base_url_for_handle(self.document_handle());
        let context = crate::network::context::DocumentFetchContext::new(
            crate::native_bridge::WindowDocumentOwner::Frame(transition.current_owner()),
            document_url.clone(),
            base_url,
            moli_url::origin_ascii_serialization(&document_url),
        );
        self.replace_document_resource_loader_for_document_open(
            retired_owner,
            context,
            crate::network::context::DocumentResourceAuthoritySource::Inherited(current),
        );
        unsafe { &mut *self.runtime }.bind_document_resource_loader(
            self.document_resource_loaders.clone(),
            crate::native_bridge::WindowDocumentOwner::Frame(transition.current_owner()),
        );
    }

    pub(crate) fn set_wpt_extensions_enabled(&mut self, enabled: bool) {
        self.wpt_extensions_enabled = enabled;
    }

    #[cfg(feature = "wpt-extensions")]
    pub(crate) fn wpt_extensions_enabled(&self) -> bool {
        self.wpt_extensions_enabled
    }

    #[cfg(feature = "wpt-extensions")]
    pub(crate) fn delete_all_cookies_for_wpt(&mut self) {
        if let Some(resource_loader) = self.current_main_document_resource_loader() {
            resource_loader
                .request_client()
                .cookie_store()
                .lock()
                .clear();
        }
    }

    pub(crate) fn set_extra_http_headers(&mut self, headers: &[(String, String)]) {
        self.extra_http_headers = headers.to_vec();
        if let Some(resource_loader) = self.current_main_document_resource_loader() {
            resource_loader
                .request_client()
                .set_extra_http_headers(headers);
        }
        for worker in self.workers.values() {
            if let super::workers::WorkerExecutionState::Running { handle } = &worker.execution {
                handle.set_extra_http_headers(headers);
            }
        }
    }

    pub(crate) fn extra_http_headers(&self) -> &[(String, String)] {
        &self.extra_http_headers
    }

    pub(crate) fn set_document_content_security_policies(&mut self, policies: &[String]) {
        unsafe { &mut *self.runtime }.set_document_content_security_policies(policies);
    }

    pub(crate) fn document_content_security_policies(&self) -> &[String] {
        unsafe { &*self.runtime }.document_content_security_policies()
    }

    pub(crate) fn set_permission_overrides(
        &mut self,
        overrides: &[crate::protocol_types::PermissionOverrideRegistration],
    ) {
        self.permission_overrides = overrides.to_vec();
    }

    pub(crate) fn permission_overrides(
        &self,
    ) -> &[crate::protocol_types::PermissionOverrideRegistration] {
        &self.permission_overrides
    }

    pub(crate) fn set_locale_override(&mut self, locale: Option<&str>) {
        self.locale_override = locale.map(str::to_owned);
    }

    pub(crate) fn set_timezone_override(&mut self, timezone: Option<&str>) {
        self.timezone_override = timezone.map(str::to_owned);
    }

    pub(crate) fn locale_override(&self) -> Option<&str> {
        self.locale_override.as_deref()
    }

    pub(crate) fn timezone_override(&self) -> Option<&str> {
        self.timezone_override.as_deref()
    }

    pub(crate) fn set_idle_override(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) {
        self.idle_override = idle_override;
    }

    pub(crate) fn idle_override(&self) -> Option<crate::protocol_types::EmulatedIdleOverride> {
        self.idle_override
    }

    pub(crate) fn begin_protocol_user_gesture_activation(&mut self) {
        self.protocol_user_gesture_activation_depth = self
            .protocol_user_gesture_activation_depth
            .saturating_add(1);
    }

    pub(crate) fn end_protocol_user_gesture_activation(&mut self) {
        self.protocol_user_gesture_activation_depth = self
            .protocol_user_gesture_activation_depth
            .saturating_sub(1);
    }

    pub(crate) fn protocol_user_gesture_activation(&self) -> bool {
        self.protocol_user_gesture_activation_depth > 0
    }

    pub(crate) fn begin_webdriver_bidi_file_prompt_handler(&mut self, handler: &str) {
        self.webdriver_bidi_file_prompt_handler_stack
            .push(handler.to_owned());
    }

    pub(crate) fn end_webdriver_bidi_file_prompt_handler(&mut self) {
        self.webdriver_bidi_file_prompt_handler_stack.pop();
    }

    pub(crate) fn webdriver_bidi_should_auto_cancel_file_chooser(&self) -> bool {
        matches!(
            self.webdriver_bidi_file_prompt_handler_stack
                .last()
                .map(String::as_str),
            Some("accept" | "dismiss")
        )
    }

    pub(crate) fn set_emulated_media(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) {
        if self.emulated_media != *overrides {
            self.style_engine
                .bump_target_context_epoch_for_document(self.document_handle());
        }
        self.emulated_media = overrides.clone();
    }

    pub(crate) fn emulated_media(&self) -> &crate::protocol_types::EmulatedMediaOverrides {
        &self.emulated_media
    }

    pub(crate) fn set_viewport_surface(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) -> bool {
        let changed = self.viewport_surface != viewport_surface;
        if changed {
            self.style_engine
                .bump_target_context_epoch_for_document(self.document_handle());
        }
        self.viewport_surface = viewport_surface;
        changed
    }

    pub(crate) fn style_viewport(&self) -> StyleViewport {
        self.viewport_surface
            .map(StyleViewport::from_viewport_surface)
            .unwrap_or_else(|| {
                StyleViewport::new(
                    Some(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
                    Some(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
                )
                .with_screen_size(
                    Some(DEFAULT_WINDOW_SURFACE_PROFILE.screen_width),
                    Some(DEFAULT_WINDOW_SURFACE_PROFILE.screen_height),
                )
            })
    }

    pub(crate) fn viewport_surface(&self) -> Option<crate::protocol_types::ViewportSurface> {
        self.viewport_surface
    }

    pub(crate) fn document_base_url_for_handle(&self, document: DomHandle) -> url::Url {
        self.dom_host()
            .document_base_url_for_handle(document)
            .unwrap_or_else(|| self.document_url().clone())
    }

    pub(crate) fn document_url_for_handle(&self, document: DomHandle) -> url::Url {
        self.dom_host()
            .document_url_for_handle(document)
            .cloned()
            .unwrap_or_else(|| self.document_url().clone())
    }

    pub(crate) fn document_ready_state_for_handle(&self, document: DomHandle) -> String {
        self.dom_host()
            .document_ready_state_for_handle(document)
            .unwrap_or_else(|| self.host_document().ready_state())
            .as_str()
            .to_owned()
    }

    pub(crate) fn element_inline_style_base_url(&self, handle: DomHandle) -> url::Url {
        let document = if self.dom_host().node(handle).is_some_and(Node::is_document) {
            Some(handle)
        } else {
            self.dom_host().node(handle).and_then(Node::owner_document)
        };
        document
            .map(|document| self.document_base_url_for_handle(document))
            .unwrap_or_else(|| self.document_url().clone())
    }

    pub(crate) fn set_document_adopted_style_sheet_installations(
        &mut self,
        document: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .set_document_adopted_style_sheet_installations_with_host(
                unsafe { &*dom_host },
                document,
                installations,
            );
    }

    pub(crate) fn adopted_style_sheet_sources_for_document(
        &self,
        document: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        self.style_engine
            .adopted_style_sheet_sources_for_document(document)
    }

    pub(crate) fn sync_owner_style_sheet_text(&mut self, owner: DomHandle) {
        let css_text = self.dom_host().text_content(owner).unwrap_or_default();
        let dom_host = self.dom_host() as *const _;
        self.style_engine.sync_owner_style_sheet_text_with_host(
            unsafe { &*dom_host },
            owner,
            css_text,
        );
        self.install_owner_live_stylesheet(owner);
    }

    pub(crate) fn apply_stylesheet_owner_changes(&mut self, changes: &[DomStylesheetOwnerChange]) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .apply_stylesheet_owner_changes_with_host(unsafe { &*dom_host }, changes);
        let mut owners = Vec::new();
        let mut attribute_owners = Vec::new();
        let mut linked_attribute_owners = Vec::new();
        for change in changes {
            let owner = change.owner();
            let source_was_processed = match change.kind() {
                DomStylesheetOwnerChangeKind::Registered
                | DomStylesheetOwnerChangeKind::Contents => {
                    unsafe { &*dom_host }.is_connected(owner)
                }
                DomStylesheetOwnerChangeKind::OwnerDocumentChanged => true,
                DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected } => *connected,
                DomStylesheetOwnerChangeKind::Unregistered
                | DomStylesheetOwnerChangeKind::Attribute { .. } => false,
            };
            if unsafe { &*dom_host }.is_inline_style_sheet_owner(owner)
                && source_was_processed
                && !owners.contains(&owner)
            {
                owners.push(owner);
            }
            if unsafe { &*dom_host }.is_inline_style_sheet_owner(owner)
                && matches!(
                    change.kind(),
                    DomStylesheetOwnerChangeKind::Attribute {
                        namespace: None,
                        local_name,
                    } if local_name == "media"
                )
                && !attribute_owners.contains(&owner)
            {
                attribute_owners.push(owner);
            }
            if unsafe { &*dom_host }.is_html_element_named(owner, "link")
                && matches!(
                    change.kind(),
                    DomStylesheetOwnerChangeKind::Attribute {
                        namespace: None,
                        local_name,
                    } if local_name == "media"
                )
                && !linked_attribute_owners.contains(&owner)
            {
                linked_attribute_owners.push(owner);
            }
        }
        for owner in owners {
            self.install_owner_live_stylesheet(owner);
        }
        for owner in attribute_owners {
            self.refresh_owner_live_stylesheet_attributes(owner);
        }
        for owner in linked_attribute_owners {
            self.refresh_linked_live_stylesheet_attributes(owner);
        }
    }

    pub(crate) fn sync_owner_style_sheet_texts_for_document_tree_scopes(
        &mut self,
        document: DomHandle,
    ) {
        let owners = {
            let host = self.dom_host();
            let mut owners = Vec::new();
            for (_, candidate_handles) in host
                .stylesheet_candidate_tree_scope_snapshots_for_document(document)
                .iter()
            {
                owners.extend(
                    candidate_handles
                        .iter()
                        .copied()
                        .filter(|owner| host.is_inline_style_sheet_owner(*owner)),
                );
            }
            owners
        };
        for owner in owners {
            self.sync_owner_style_sheet_text(owner);
        }
    }

    pub(crate) fn sync_pending_document_style_sources(&mut self) {
        let pending = {
            let runtime: &mut DocumentRuntime = self;
            runtime.take_style_source_document_sync_pending()
        };
        if pending {
            self.sync_owner_style_sheet_texts_for_document_tree_scopes(self.document_handle());
        }
    }

    pub(crate) fn take_pending_main_document_owner_transitions(
        &mut self,
    ) -> Vec<crate::frame_owner_model::MainDocumentOwnerTransition> {
        self.frame_owner_store
            .take_pending_main_document_owner_transitions()
    }

    pub(crate) fn close_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        self.close_root_document_parser_stream(scope, host_ptr);
        self.sync_pending_document_style_sources();
    }

    pub(crate) fn owner_style_sheet_text(&self, owner: DomHandle) -> Option<String> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .owner_style_sheet_text_with_host(unsafe { &*dom_host }, owner)
    }

    pub(crate) fn owner_style_sheet_source(
        &self,
        owner: DomHandle,
    ) -> Option<StyloStylesheetSource> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .owner_style_sheet_source_with_host(unsafe { &*dom_host }, owner)
    }

    pub(crate) fn owner_live_stylesheet(
        &self,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .owner_live_stylesheet_with_host(unsafe { &*dom_host }, owner)
    }

    pub(crate) fn note_owner_live_stylesheet_mutation(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        if self.style_engine.refresh_owner_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        ) {
            self.style_engine
                .mark_owner_live_stylesheet_cssom_authoritative_with_host(
                    unsafe { &*dom_host },
                    owner,
                );
            let canceled_load_event_bindings =
                unsafe { &mut *self.runtime }.invalidate_style_related_state(owner);
            for binding in canceled_load_event_bindings {
                let settled = self.settle_main_style_load_event(binding);
                tracing::debug!(
                    owner = ?binding.owner(),
                    element = ?binding.element(),
                    load_delay_token = ?binding.load_delay_token(),
                    settled,
                    "settled invalidated connected-style lease at CSSOM commit"
                );
            }
            let host_ptr = self as *mut Self;
            unsafe { &mut *self.runtime }
                .apply_inline_cssom_source_change_after_invalidation(host_ptr, owner);
            return true;
        }
        if self.style_engine.refresh_linked_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        ) {
            self.clear_layout_rect_cache();
            self.queue_stylesheet_source_css_projection(owner);
            let host_ptr = self as *mut Self;
            unsafe { &mut *self.runtime }.apply_linked_cssom_source_change(host_ptr, owner);
            return true;
        }
        false
    }

    pub(crate) fn note_owner_live_stylesheet_runtime_state_change(
        &mut self,
        owner: DomHandle,
        stylesheet_id: crate::live_stylesheet::StylesheetId,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        let refreshed = self.style_engine.refresh_owner_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        ) || self.style_engine.refresh_linked_live_stylesheet_with_host(
            unsafe { &*dom_host },
            owner,
            stylesheet_id,
        );
        if refreshed {
            self.clear_layout_rect_cache();
        }
        refreshed
    }

    pub(crate) fn owner_style_sheet_processing_source(
        &self,
        owner: DomHandle,
    ) -> Option<std::sync::Arc<crate::style_engine::OwnerStyleSheetSource>> {
        self.style_engine.owner_style_sheet_processing_source(owner)
    }

    pub(crate) fn stylesheet_owner_is_csp_blocked(&self, owner: DomHandle) -> bool {
        unsafe { &*self.runtime }.stylesheet_owner_is_csp_blocked(owner)
    }

    pub(crate) fn set_owner_style_sheet_csp_suppressed(
        &mut self,
        owner: DomHandle,
        suppressed: bool,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .set_owner_style_sheet_csp_suppressed_with_host(
                unsafe { &*dom_host },
                owner,
                suppressed,
            )
    }

    pub(crate) fn install_linked_stylesheet(
        &mut self,
        operation: crate::document_runtime::InstallLinkedStylesheet,
    ) {
        self.clear_layout_rect_cache();
        let (owner, request_url, prepared) = operation.into_parts();
        let live_stylesheet = if prepared.has_import_rules() {
            self.dom_host()
                .owner_document_handle(owner)
                .and_then(|document| {
                    self.create_live_stylesheet_from_parsed_source(document, prepared.source())
                })
        } else {
            let shared_contents = prepared.source().shared_initial_contents().expect(
                "no-import prepared linked stylesheet must retain its immutable parsed contents",
            );
            let stylesheet =
                self.create_live_stylesheet_from_shared_initial_contents(shared_contents);
            Some(stylesheet)
        };
        if let Some(stylesheet) = &live_stylesheet {
            let media = self
                .dom_host()
                .get_attribute(owner, "media")
                .unwrap_or_default();
            stylesheet.set_media_text(&media);
            stylesheet.set_disabled(self.dom_host().get_attribute(owner, "disabled").is_some());
            stylesheet.set_origin_clean(prepared.source().origin_clean());
        }
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .install_linked_stylesheet_source_and_live_owner_with_host(
                unsafe { &*dom_host },
                owner,
                &request_url,
                prepared.source().clone(),
                live_stylesheet,
            );
        self.queue_stylesheet_source_css_projection(owner);
    }

    pub(crate) fn linked_stylesheet_source_for_owner(
        &self,
        owner: DomHandle,
    ) -> Option<StyloStylesheetSource> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .linked_stylesheet_source_for_owner_with_host(unsafe { &*dom_host }, owner)
    }

    pub(crate) fn linked_live_stylesheet(
        &self,
        owner: DomHandle,
    ) -> Option<crate::live_stylesheet::LiveStylesheetRef> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .linked_live_stylesheet_with_host(unsafe { &*dom_host }, owner)
    }

    pub(crate) fn install_cached_linked_stylesheet_for_owner(
        &mut self,
        owner: DomHandle,
        request_url: &url::Url,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        let Some(source) = self.style_engine.cached_linked_stylesheet_source_with_host(
            unsafe { &*dom_host },
            owner,
            request_url,
        ) else {
            return false;
        };
        let live_stylesheet = source
            .shared_initial_contents()
            .map(|shared_contents| {
                self.create_live_stylesheet_from_shared_initial_contents(shared_contents)
            })
            .or_else(|| {
                self.dom_host()
                    .owner_document_handle(owner)
                    .and_then(|document| {
                        self.create_live_stylesheet_from_parsed_source(document, &source)
                    })
            });
        if let Some(stylesheet) = &live_stylesheet {
            let media = unsafe { &*dom_host }
                .get_attribute(owner, "media")
                .unwrap_or_default();
            stylesheet.set_media_text(&media);
            stylesheet.set_disabled(
                unsafe { &*dom_host }
                    .get_attribute(owner, "disabled")
                    .is_some(),
            );
            stylesheet.set_origin_clean(source.origin_clean());
        }
        self.style_engine
            .install_linked_stylesheet_source_and_live_owner_with_host(
                unsafe { &*dom_host },
                owner,
                request_url,
                source,
                live_stylesheet,
            );
        self.queue_stylesheet_source_css_projection(owner);
        true
    }

    pub(crate) fn stylesheet_font_faces_for_owner(
        &self,
        owner: DomHandle,
    ) -> Option<std::sync::Arc<[crate::style_engine::StylesheetFontFaceDescriptor]>> {
        let dom_host = self.dom_host();
        if !dom_host.is_connected(owner)
            || dom_host.get_attribute(owner, "disabled").is_some()
            || !moli_web_mime::is_stylesheet_type_attribute(
                dom_host.get_attribute(owner, "type").as_deref(),
            )
        {
            return None;
        }
        if dom_host.is_inline_style_sheet_owner(owner) {
            return self
                .owner_style_sheet_processing_source(owner)
                .map(|source| source.font_faces());
        }
        if dom_host.is_html_element_named(owner, "link") {
            return self
                .linked_stylesheet_source_for_owner(owner)
                .map(|source| source.font_faces());
        }
        None
    }

    pub(crate) fn set_shadow_root_adopted_style_sheet_installations(
        &mut self,
        root: DomHandle,
        installations: Vec<AdoptedStyleSheetInstallation>,
    ) {
        self.clear_layout_rect_cache();
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .set_shadow_root_adopted_style_sheet_installations_with_host(
                unsafe { &*dom_host },
                root,
                installations,
            );
    }

    pub(crate) fn shadow_root_adopted_style_sheet_sources(
        &self,
        root: DomHandle,
    ) -> Vec<StyloStylesheetSource> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .shadow_root_adopted_style_sheet_sources_with_host(unsafe { &*dom_host }, root)
    }

    pub(crate) fn register_css_custom_property_for_document(
        &mut self,
        document: DomHandle,
        registration: CssCustomPropertyRegistration,
    ) -> Result<(), CssCustomPropertyRegistrationError> {
        self.clear_layout_rect_cache();
        let base_url = self.style_base_url_for_document(document);
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .register_css_custom_property_for_document_with_host(
                unsafe { &*dom_host },
                document,
                registration,
                base_url,
            )
    }

    pub(crate) fn validate_css_custom_property_registration_for_document(
        &self,
        document: DomHandle,
        registration: &CssCustomPropertyRegistration,
    ) -> style::stylist::RegisterCustomPropertyResult {
        let base_url = self.style_base_url_for_document(document);
        self.style_engine
            .validate_css_custom_property_registration(registration, base_url)
    }

    fn style_base_url_for_document(&self, document: DomHandle) -> url::Url {
        self.dom_host()
            .node(document)
            .and_then(Node::as_document)
            .map(|document_node| document_node.base_url().clone())
            .or_else(|| {
                (document == self.document_handle())
                    .then(|| self.dom_host().document_base_url())
                    .flatten()
            })
            .unwrap_or_else(|| self.document_url().clone())
    }

    pub(crate) fn registered_css_custom_property_registration(
        &self,
        document: DomHandle,
        name: &str,
    ) -> Option<CssCustomPropertyRegistration> {
        self.style_engine
            .registered_css_custom_property_registration_for_document(document, name)
    }

    pub(crate) fn script_css_custom_property_registrations(
        &self,
        document: DomHandle,
    ) -> Vec<CssCustomPropertyRegistration> {
        self.style_engine
            .script_css_custom_property_registrations_for_document(document)
    }

    pub(crate) fn computed_style_property_value_from_stylo(
        &self,
        handle: DomHandle,
        property: &str,
        pseudo_element: Option<&str>,
        inputs: &StyloComputedStyleInputs,
        read_document: DomHandle,
        viewport: StyleViewport,
    ) -> Option<String> {
        #[cfg(test)]
        self.stylo_computed_style_property_reads.set(
            self.stylo_computed_style_property_reads
                .get()
                .saturating_add(1),
        );
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .computed_style_property_value_with_document_context(
                self.dom_host(),
                self.document_url(),
                handle,
                property,
                pseudo_element,
                inputs,
                document_context.as_ref(),
                read_document,
                viewport,
            )
    }

    fn style_source_document_context(&self) -> OwnedStyleSourceDocumentContext {
        OwnedStyleSourceDocumentContext::new(self.document_handle()).with_child_documents(
            self.child_browsing_context_document_handles
                .values()
                .copied(),
        )
    }

    fn style_source_document_context_for_read_document(
        &self,
        read_document: DomHandle,
    ) -> OwnedStyleSourceDocumentContext {
        if read_document != self.document_handle()
            && self
                .lightweight_popup_id_for_document_handle(read_document)
                .is_some()
        {
            return OwnedStyleSourceDocumentContext::new(read_document);
        }
        self.style_source_document_context()
    }

    #[cfg(test)]
    pub(crate) fn drain_pending_style_invalidations_for_document_for_test(
        &self,
        document: DomHandle,
    ) {
        self.style_engine
            .drain_pending_style_invalidations_for_document_for_test(self.dom_host(), document);
    }

    pub(crate) fn drain_pending_style_invalidations_for_computed_style_read(&self) {
        let document_context = self.style_source_document_context();
        self.style_engine
            .drain_pending_style_invalidations_for_computed_style_read_with_document_context(
                self.dom_host(),
                self.document_handle(),
                document_context.as_ref(),
            );
    }

    pub(crate) fn drain_pending_style_invalidations_for_computed_style_read_for_document(
        &self,
        read_document: DomHandle,
    ) {
        let owner_document = if self
            .lightweight_popup_id_for_document_handle(read_document)
            .is_some()
        {
            read_document
        } else {
            self.document_handle()
        };
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .drain_pending_style_invalidations_for_computed_style_read_with_document_context(
                self.dom_host(),
                owner_document,
                document_context.as_ref(),
            );
    }

    pub(crate) fn drain_pending_style_invalidations_for_turn_exit(
        &self,
        boundary: crate::style_engine::StyleInvalidationTurnExitBoundary,
    ) {
        let document_context = self.style_source_document_context();
        self.style_engine
            .drain_pending_style_invalidations_for_turn_exit_with_document_context(
                self.dom_host(),
                document_context.as_ref(),
                boundary,
            );
        self.clear_layout_rect_cache();
    }

    pub(crate) fn style_target_context_epoch(&self) -> u64 {
        self.style_engine
            .target_context_epoch_for_document(self.document_handle())
    }

    pub(crate) fn computed_style_cache_generation_for_handle(&self, handle: DomHandle) -> u64 {
        let document = self
            .dom_host()
            .owner_document_handle(handle)
            .unwrap_or_else(|| self.document_handle());
        self.style_engine
            .computed_cache_generation_for_document(document)
    }

    pub(crate) fn cached_document_prepared_style_inputs(
        &self,
        document: DomHandle,
        key: &StyloDocumentComputedStyleInputCacheKey,
    ) -> Option<Rc<StyloPreparedComputedStyleInputs>> {
        self.style_engine
            .cached_document_prepared_style_inputs(document, key)
    }

    pub(crate) fn cache_document_prepared_style_inputs(
        &self,
        document: DomHandle,
        key: StyloDocumentComputedStyleInputCacheKey,
        inputs: Rc<StyloPreparedComputedStyleInputs>,
    ) {
        self.style_engine
            .cache_document_prepared_style_inputs(document, key, inputs);
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.style_engine
            .computed_cache_generation_for_document_for_test(document)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn computed_style_read_invariant_state(
        &self,
        document: DomHandle,
    ) -> (u64, u64, u64) {
        let (source_set_generation, retained_style_system_generation) = self
            .style_engine
            .computed_style_read_invariant_state(document);
        (
            self.dom_host().dom_version(),
            source_set_generation,
            retained_style_system_generation,
        )
    }

    #[cfg(test)]
    pub(crate) fn note_stylo_computed_style_input_build_for_test(&self) {
        self.stylo_computed_style_input_builds.set(
            self.stylo_computed_style_input_builds
                .get()
                .saturating_add(1),
        );
    }

    #[cfg(test)]
    pub(crate) fn stylo_computed_style_input_builds_for_test(&self) -> u64 {
        self.stylo_computed_style_input_builds.get()
    }

    #[cfg(test)]
    pub(crate) fn note_stylo_style_system_key_build_for_test(&self) {
        self.stylo_style_system_key_builds
            .set(self.stylo_style_system_key_builds.get().saturating_add(1));
    }

    #[cfg(test)]
    pub(crate) fn stylo_style_system_key_builds_for_test(&self) -> u64 {
        self.stylo_style_system_key_builds.get()
    }

    #[cfg(test)]
    pub(crate) fn inline_style_base_url_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.style_engine
            .inline_style_base_url_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.style_engine
            .computed_style_cache_entry_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_rebuild_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.style_engine
            .retained_style_system_rebuild_count_for_document_for_test(document)
    }

    #[cfg(test)]
    pub(crate) fn pending_style_invalidation_work_item_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.style_engine
            .pending_style_invalidation_work_item_count_for_document_for_test(document)
    }

    pub(crate) fn computed_style_snapshot_from_stylo_after_style_update(
        &self,
        handle: DomHandle,
        inputs: &StyloComputedStyleInputs,
        read_document: DomHandle,
        viewport: StyleViewport,
    ) -> Option<crate::style_engine::StyloComputedStyleSnapshot> {
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .computed_style_snapshot_after_style_update_with_document_context(
                self.dom_host(),
                self.document_url(),
                handle,
                inputs,
                document_context.as_ref(),
                read_document,
                viewport,
            )
    }

    pub(crate) fn computed_style_snapshot_from_stylo_with_prepared_inputs(
        &self,
        handle: DomHandle,
        inputs: &StyloPreparedComputedStyleInputs,
        read_document: DomHandle,
    ) -> Option<crate::style_engine::StyloComputedStyleSnapshot> {
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .computed_style_snapshot_after_style_update_with_prepared_inputs(
                self.dom_host(),
                self.document_url(),
                handle,
                inputs,
                document_context.as_ref(),
                read_document,
            )
    }

    pub(crate) fn computed_pseudo_style_snapshot_from_stylo_with_prepared_inputs(
        &self,
        handle: DomHandle,
        pseudo_element: &str,
        inputs: &StyloPreparedComputedStyleInputs,
        read_document: DomHandle,
    ) -> Option<crate::style_engine::StyloComputedStyleSnapshot> {
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .computed_pseudo_style_snapshot_after_style_update_with_prepared_inputs(
                self.dom_host(),
                self.document_url(),
                handle,
                pseudo_element,
                inputs,
                document_context.as_ref(),
                read_document,
            )
    }

    pub(crate) fn computed_anonymous_style_snapshot_from_stylo_with_prepared_inputs(
        &self,
        owner: DomHandle,
        parent_style: &style::properties::ComputedValues,
        anonymous_kind: crate::style_engine::StyloAnonymousBoxKind,
        inputs: &StyloPreparedComputedStyleInputs,
        read_document: DomHandle,
    ) -> Option<crate::style_engine::StyloComputedStyleSnapshot> {
        let document_context = self.style_source_document_context_for_read_document(read_document);
        self.style_engine
            .computed_anonymous_style_snapshot_after_style_update_with_prepared_inputs(
                self.dom_host(),
                self.document_url(),
                owner,
                parent_style,
                anonymous_kind,
                inputs,
                document_context.as_ref(),
                read_document,
            )
    }

    #[cfg(test)]
    pub(crate) fn stylo_computed_style_property_reads_for_test(&self) -> u64 {
        self.stylo_computed_style_property_reads.get()
    }

    pub(crate) fn set_document_url(&mut self, url: url::Url) -> bool {
        let target_change = DocumentRuntime::set_document_url(self, url);
        if let Some((previous_target, next_target)) = target_change {
            self.note_target_style_activity(previous_target, next_target);
        }
        target_change.is_some()
    }

    pub(crate) fn set_dom_document_url_for_handle(
        &mut self,
        document_handle: DomHandle,
        url: url::Url,
    ) -> bool {
        let previous_target = self.dom_host().document_target_element(document_handle);
        let changed = self
            .dom_host_mut()
            .set_document_url_for_handle(document_handle, url);
        let next_target = self.dom_host().document_target_element(document_handle);
        if changed && previous_target != next_target {
            self.note_target_style_activity(previous_target, next_target);
        }
        changed
    }

    pub(crate) fn set_dom_document_content_type_for_handle(
        &mut self,
        document_handle: DomHandle,
        content_type: impl Into<String>,
    ) -> bool {
        self.dom_host_mut()
            .set_document_content_type_for_handle(document_handle, content_type)
    }

    pub(crate) fn set_dom_document_ready_state_for_handle(
        &mut self,
        document_handle: DomHandle,
        ready_state: DocumentReadyState,
    ) -> bool {
        self.dom_host_mut()
            .set_document_ready_state_for_handle(document_handle, ready_state)
    }

    pub(crate) fn note_focus_style_activity(
        &mut self,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine.invalidate_for_focus_change_with_viewport(
            unsafe { &*dom_host },
            previous,
            next,
            &emulated_media,
            viewport,
        );
    }

    pub(crate) fn note_focus_style_activity_with_previous_focus_within(
        &mut self,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        previous_focus_within: Option<Vec<DomHandle>>,
    ) {
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine
            .invalidate_for_focus_change_with_previous_focus_within_and_viewport(
                unsafe { &*dom_host },
                previous,
                next,
                previous_focus_within,
                &emulated_media,
                viewport,
            );
    }

    pub(crate) fn note_target_style_activity(
        &mut self,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine
            .invalidate_for_target_change_with_viewport(
                unsafe { &*dom_host },
                previous,
                next,
                &emulated_media,
                viewport,
            );
    }

    pub(crate) fn note_element_state_style_activity_with_old_state(
        &mut self,
        element: DomHandle,
        state: StyloElementState,
        old_state: Option<StyloElementState>,
    ) {
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine
            .invalidate_for_element_state_change_with_old_state_and_viewport(
                unsafe { &*dom_host },
                element,
                state,
                old_state,
                &emulated_media,
                viewport,
            );
    }

    pub(crate) fn set_hovered_element_for_input(&mut self, hit: Option<DomHandle>) -> bool {
        let next = hover_chain_for_input_hit(self, hit);
        let previous = self.dom_host().hovered_element_handles();
        if previous == next {
            return false;
        }
        let changed = previous
            .iter()
            .chain(&next)
            .copied()
            .filter(|handle| previous.contains(handle) != next.contains(handle))
            .collect::<Vec<_>>();
        let old_states = changed
            .iter()
            .map(|handle| (*handle, self.retained_current_element_state(*handle)))
            .collect::<Vec<_>>();
        if !self.dom_host().set_hovered_element_handles(next) {
            return false;
        }
        for (handle, old_state) in old_states {
            self.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::HOVER,
                old_state,
            );
        }
        // Hover selectors may change box generation or geometry. The next hit
        // test must observe that new tree instead of waiting for a paint pass
        // to replace the latest owned layout snapshot.
        self.invalidate_layout_after_interaction_state_change();
        true
    }

    pub(crate) fn note_custom_state_style_activity(
        &mut self,
        element: DomHandle,
        state_name: &str,
        old_custom_states: Vec<String>,
    ) {
        self.note_custom_states_style_activity(
            element,
            vec![state_name.to_owned()],
            old_custom_states,
        );
    }

    pub(crate) fn note_custom_states_style_activity(
        &mut self,
        element: DomHandle,
        state_names: Vec<String>,
        old_custom_states: Vec<String>,
    ) {
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine
            .invalidate_for_custom_state_change_with_viewport(
                unsafe { &*dom_host },
                element,
                state_names,
                old_custom_states,
                &emulated_media,
                viewport,
            );
    }

    pub(crate) fn set_media_paused(&mut self, handle: DomHandle, paused: bool) -> bool {
        let old_state = self.retained_current_element_state(handle);
        let changed =
            unsafe { DocumentRuntime::set_media_paused(&mut *self.runtime, handle, paused) };
        if changed {
            self.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::PAUSED,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn set_media_muted(&mut self, handle: DomHandle, muted: bool) -> bool {
        let old_state = self.retained_current_element_state(handle);
        let changed =
            unsafe { DocumentRuntime::set_media_muted(&mut *self.runtime, handle, muted) };
        if changed {
            self.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::MUTED,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn set_media_seeking(&mut self, handle: DomHandle, seeking: bool) -> bool {
        let old_state = self.retained_current_element_state(handle);
        let changed =
            unsafe { DocumentRuntime::set_media_seeking(&mut *self.runtime, handle, seeking) };
        if changed {
            self.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::SEEKING,
                old_state,
            );
        }
        changed
    }

    pub(crate) fn retained_current_element_state(
        &self,
        element: DomHandle,
    ) -> Option<StyloElementState> {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .retained_current_element_state(unsafe { &*dom_host }, element)
    }

    pub(crate) fn set_input_value(&mut self, handle: DomHandle, value: &str) -> bool {
        self.clear_text_control_change_pending(handle);
        let old_state = self.retained_current_element_state(handle);
        let container_old_states = self.retained_old_states_for_validity_container_change(handle);
        let changed =
            unsafe { DocumentRuntime::set_input_value(&mut *self.runtime, handle, value) };
        if changed {
            self.note_input_value_state_style_activity(handle, old_state, &container_old_states);
        }
        changed
    }

    pub(crate) fn set_input_value_with_dirty(
        &mut self,
        handle: DomHandle,
        value: &str,
        dirty: bool,
    ) -> bool {
        self.clear_text_control_change_pending(handle);
        let old_state = self.retained_current_element_state(handle);
        let container_old_states = self.retained_old_states_for_validity_container_change(handle);
        let changed = unsafe {
            DocumentRuntime::set_input_value_with_dirty(&mut *self.runtime, handle, value, dirty)
        };
        if changed {
            self.note_input_value_state_style_activity(handle, old_state, &container_old_states);
        }
        changed
    }

    pub(crate) fn set_input_value_from_user_edit(
        &mut self,
        handle: DomHandle,
        value: &str,
    ) -> bool {
        let old_state = self.retained_current_element_state(handle);
        let container_old_states = self.retained_old_states_for_validity_container_change(handle);
        let changed = unsafe {
            DocumentRuntime::set_input_value_from_user_edit(&mut *self.runtime, handle, value)
        };
        if changed {
            self.note_input_value_state_style_activity(handle, old_state, &container_old_states);
        }
        changed
    }

    pub(crate) fn set_autofilled(&mut self, handle: DomHandle, autofilled: bool) -> bool {
        let old_state = self.retained_current_element_state(handle);
        let changed =
            unsafe { DocumentRuntime::set_autofilled(&mut *self.runtime, handle, autofilled) };
        if changed {
            self.note_element_state_style_activity_with_old_state(
                handle,
                StyloElementState::AUTOFILL,
                old_state,
            );
        }
        changed
    }

    fn note_input_value_state_style_activity(
        &mut self,
        handle: DomHandle,
        old_state: Option<StyloElementState>,
        container_old_states: &[(DomHandle, StyloElementState)],
    ) {
        self.note_element_state_style_activity_with_old_state(
            handle,
            StyloElementState::AUTOFILL
                | StyloElementState::PLACEHOLDER_SHOWN
                | StyloElementState::LTR
                | StyloElementState::RTL
                | StyloElementState::INRANGE
                | StyloElementState::OUTOFRANGE
                | StyloElementState::VALIDITY_STATES,
            old_state,
        );
        for (container, old_state) in container_old_states {
            self.note_element_state_style_activity_with_old_state(
                *container,
                StyloElementState::VALIDITY_STATES,
                Some(*old_state),
            );
        }
    }

    pub(crate) fn set_checked_state(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        checked: bool,
    ) -> bool {
        let old_states = self.retained_old_states_for_checked_state_change(handle, checked);
        unsafe {
            DocumentRuntime::set_checked_state_with_old_states(
                &mut *self.runtime,
                scope,
                host_ptr,
                handle,
                checked,
                &old_states,
            )
        }
    }

    pub(crate) fn set_checked_state_with_dirty(
        &mut self,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        checked: bool,
        dirty: bool,
    ) -> bool {
        let old_states = self.retained_old_states_for_checked_state_change(handle, checked);
        unsafe {
            DocumentRuntime::set_checked_state_with_dirty_and_old_states(
                &mut *self.runtime,
                host_ptr,
                handle,
                checked,
                dirty,
                &old_states,
            )
        }
    }

    fn retained_old_states_for_checked_state_change(
        &self,
        handle: DomHandle,
        checked: bool,
    ) -> Vec<(DomHandle, StyloElementState)> {
        self.dom_host()
            .checked_state_change_candidate_handles(handle, checked)
            .into_iter()
            .filter_map(|candidate| {
                self.retained_current_element_state(candidate)
                    .map(|state| (candidate, state))
            })
            .collect()
    }

    fn retained_old_states_for_validity_container_change(
        &self,
        handle: DomHandle,
    ) -> Vec<(DomHandle, StyloElementState)> {
        let mut containers = Vec::new();
        if let Some(form) = self.dom_host().form_control_owner(handle) {
            push_unique_handle(&mut containers, form);
        }
        let mut current = self.dom_host().parent_node(handle);
        while let Some(parent) = current {
            if self.dom_host().is_html_element_named(parent, "form")
                || self.dom_host().is_html_element_named(parent, "fieldset")
            {
                push_unique_handle(&mut containers, parent);
            }
            current = self.dom_host().parent_node(parent);
        }
        containers
            .into_iter()
            .filter_map(|container| {
                self.retained_current_element_state(container)
                    .map(|state| (container, state))
            })
            .collect()
    }

    pub(crate) fn note_style_mutation_effects(&mut self, effects: &[StyleMutationEffect]) {
        if style_mutation_effects_affect_layout_metric(effects) {
            self.clear_layout_rect_cache();
        }
        let dom_host = self.dom_host() as *const _;
        let emulated_media = self.emulated_media().clone();
        let viewport = self.style_viewport();
        self.style_engine.invalidate_for_mutations_with_viewport(
            unsafe { &*dom_host },
            effects,
            &emulated_media,
            viewport,
        );
    }

    pub(crate) fn note_element_inline_style_subtree_activity(&mut self, root: DomHandle) {
        self.clear_layout_rect_cache();
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .invalidate_inline_style_subtree(unsafe { &*dom_host }, root);
    }

    pub(crate) fn note_style_subtree_context_change(&mut self, root: DomHandle) {
        self.clear_layout_rect_cache();
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .invalidate_style_subtree(unsafe { &*dom_host }, root);
    }

    pub(crate) fn migrate_inline_style_metadata_in_subtree(&mut self, root: DomHandle) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .migrate_inline_style_metadata_subtree_with_host(unsafe { &*dom_host }, root);
    }

    pub(crate) fn ensure_element_inline_style_base_url(
        &mut self,
        handle: DomHandle,
        base_url: url::Url,
    ) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine.ensure_inline_style_base_url_with_host(
            unsafe { &*dom_host },
            handle,
            base_url,
        );
    }

    pub(crate) fn set_element_inline_style_base_url(
        &mut self,
        handle: DomHandle,
        base_url: url::Url,
    ) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine.set_inline_style_base_url_with_host(
            unsafe { &*dom_host },
            handle,
            base_url,
        );
    }

    pub(crate) fn existing_element_inline_style_base_url(
        &self,
        handle: DomHandle,
    ) -> Option<url::Url> {
        self.style_engine
            .inline_style_base_url_with_host(self.dom_host(), handle)
    }

    pub(crate) fn clear_element_inline_style_base_url(&mut self, handle: DomHandle) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .clear_inline_style_base_url_with_host(unsafe { &*dom_host }, handle);
    }

    pub(crate) fn set_element_inline_style_resolution_text(
        &mut self,
        handle: DomHandle,
        text: String,
    ) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .set_inline_style_resolution_text_with_host(unsafe { &*dom_host }, handle, text);
    }

    pub(crate) fn clear_element_inline_style_resolution_text(&mut self, handle: DomHandle) {
        let dom_host = self.dom_host() as *const _;
        self.style_engine
            .clear_inline_style_resolution_text_with_host(unsafe { &*dom_host }, handle);
    }

    pub(crate) fn set_element_inline_style_csp_state(
        &mut self,
        handle: DomHandle,
        state: crate::style_engine::InlineStyleCspState,
    ) -> bool {
        let dom_host = self.dom_host() as *const _;
        let changed = self.style_engine.set_inline_style_csp_state_with_host(
            unsafe { &*dom_host },
            handle,
            state,
        );
        if changed {
            self.note_element_inline_style_subtree_activity(handle);
        }
        changed
    }

    pub(crate) fn element_inline_style_csp_state(
        &self,
        handle: DomHandle,
    ) -> crate::style_engine::InlineStyleCspState {
        self.style_engine
            .inline_style_csp_state_with_host(self.dom_host(), handle)
    }

    pub(crate) fn element_inline_style_declaration_state(
        &self,
        handle: DomHandle,
    ) -> Option<&crate::css_style::CssInlineStyleDeclarationState> {
        self.inline_style_declarations.get(&handle)
    }

    pub(crate) fn set_element_inline_style_declaration_state(
        &mut self,
        handle: DomHandle,
        state: crate::css_style::CssInlineStyleDeclarationState,
    ) {
        self.inline_style_declarations.insert(handle, state);
    }

    pub(crate) fn clear_element_inline_style_declaration_state(&mut self, handle: DomHandle) {
        self.inline_style_declarations.remove(&handle);
        self.clear_element_inline_style_resolution_text(handle);
    }

    pub(crate) fn set_element_inline_style_current_base_url(&mut self, handle: DomHandle) {
        let base_url = self.element_inline_style_base_url(handle);
        self.set_element_inline_style_base_url(handle, base_url);
    }

    pub(crate) fn mark_disconnected_shadow_roots_in_subtree(&mut self, root: DomHandle) {
        for shadow_root in self.shadow_roots_in_subtree(root) {
            self.disconnected_shadow_roots.insert(shadow_root);
        }
    }

    pub(crate) fn clear_disconnected_shadow_roots_in_subtree(&mut self, root: DomHandle) {
        for shadow_root in self.shadow_roots_in_subtree(root) {
            self.disconnected_shadow_roots.remove(&shadow_root);
        }
    }

    pub(crate) fn shadow_root_is_disconnected_for_style(&self, root: DomHandle) -> bool {
        self.disconnected_shadow_roots.contains(&root)
    }

    pub(crate) fn shadow_roots_in_subtree(&self, root: DomHandle) -> Vec<DomHandle> {
        let mut roots = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if let Some(shadow_root) = self.dom_host().shadow_root_handle(handle) {
                roots.push(shadow_root);
                stack.push(shadow_root);
            }
            let mut child = self.dom_host().first_child(handle);
            while let Some(current) = child {
                stack.push(current);
                child = self.dom_host().next_sibling(current);
            }
        }
        roots
    }

    pub(crate) fn permission_state(&self, permission_name: &str) -> &str {
        let current_origin = unsafe {
            (&*self.runtime)
                .document_url()
                .origin()
                .ascii_serialization()
        };
        self.permission_state_for_origins(permission_name, &current_origin, &current_origin)
    }

    pub(crate) fn permission_state_for_origins(
        &self,
        permission_name: &str,
        requesting_origin: &str,
        embedding_origin: &str,
    ) -> &str {
        let mut default_state = None;

        for override_entry in self.permission_overrides.iter().rev() {
            let Some(name) = permission_override_name(override_entry) else {
                continue;
            };
            if !permission_names_match(name, permission_name) {
                continue;
            }

            let Some(configured_embedding_origin) = override_entry.origin.as_deref() else {
                if default_state.is_none() {
                    default_state = Some(override_entry.setting.as_str());
                }
                continue;
            };
            let configured_requesting_origin = override_entry
                .embedded_origin
                .as_deref()
                .unwrap_or(configured_embedding_origin);
            if configured_embedding_origin == embedding_origin
                && configured_requesting_origin == requesting_origin
            {
                return override_entry.setting.as_str();
            }
        }

        default_state.unwrap_or(match permission_name {
            "background-sync" | "periodic-background-sync" | "persistent-storage" => "granted",
            _ => "prompt",
        })
    }

    pub(crate) fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
        if let Some(resource_loader) = self.current_main_document_resource_loader() {
            resource_loader
                .request_client()
                .set_network_offline(offline);
        }
        for worker in self.workers.values() {
            if let super::workers::WorkerExecutionState::Running { handle } = &worker.execution {
                handle.set_network_offline(offline);
            }
        }
    }

    pub(crate) fn network_offline(&self) -> bool {
        self.network_offline
    }

    pub(crate) fn set_bypass_service_worker(&mut self, bypass: bool) {
        if let Some(resource_loader) = self.current_main_document_resource_loader() {
            resource_loader
                .request_client()
                .set_bypass_service_worker(bypass);
        }
    }

    pub(crate) fn set_blocked_url_patterns(&mut self, patterns: &[String]) {
        self.blocked_url_patterns = patterns.to_vec();
        if let Some(resource_loader) = self.current_main_document_resource_loader() {
            resource_loader
                .request_client()
                .set_blocked_url_patterns(patterns);
        }
        for worker in self.workers.values() {
            if let super::workers::WorkerExecutionState::Running { handle } = &worker.execution {
                handle.set_blocked_url_patterns(patterns);
            }
        }
    }

    pub(crate) fn blocked_url_patterns(&self) -> &[String] {
        &self.blocked_url_patterns
    }

    pub(crate) fn is_url_blocked(&self, url: &url::Url) -> bool {
        let value = url.as_str();
        self.blocked_url_patterns
            .iter()
            .any(|pattern| moli_fetch::url_pattern_matches(pattern, value))
    }

    pub(crate) fn set_fetch_subresource_interception(
        &mut self,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) {
        self.fetch_subresource_interception_enabled = enabled;
        self.fetch_subresource_interception_resource_type = resource_type;
        for worker in self.workers.values() {
            if let super::workers::WorkerExecutionState::Running { handle } = &worker.execution {
                handle.set_fetch_subresource_interception(enabled, resource_type);
            }
        }
    }

    pub(crate) fn should_intercept_subresource(
        &self,
        resource_type: SubresourceResourceType,
    ) -> bool {
        self.fetch_subresource_interception_enabled
            && self
                .fetch_subresource_interception_resource_type
                .is_none_or(|expected| expected.has_same_cdp_fetch_interception_type(resource_type))
    }

    pub(crate) fn fetch_subresource_interception_enabled(&self) -> bool {
        self.fetch_subresource_interception_enabled
    }

    pub(crate) fn fetch_subresource_interception_resource_type(
        &self,
    ) -> Option<SubresourceResourceType> {
        self.fetch_subresource_interception_resource_type
    }
}

fn style_mutation_effects_affect_layout_metric(effects: &[StyleMutationEffect]) -> bool {
    effects.iter().any(|effect| match effect {
        StyleMutationEffect::Attribute { name, .. } => {
            StyleAttributeImpact::for_attribute_name(name).affects_layout_metric()
        }
        StyleMutationEffect::ConnectedSubtree { .. }
        | StyleMutationEffect::DisconnectedSubtree { .. }
        | StyleMutationEffect::SlotAssignment { .. }
        | StyleMutationEffect::CharacterData { .. }
        | StyleMutationEffect::ChildList { .. } => true,
    })
}
