use tracing::debug;

use super::*;
use crate::frame_owner_model::MainDocumentStyleLoadEventBinding;
use crate::link_as::{LinkAsDestination, link_as_destination};
use crate::module_runtime::{
    ModuleMapKey, NativeModuleSingleFetchRequest, NativeModulepreloadLinkClient,
};
use crate::planning::{ScriptFetchMetadata, module_script_credentials_mode};
use crate::service_worker_runtime::ServiceWorkerRequestDestination;
use crate::stylesheet_blocking::{
    StylesheetFetchOptions, connected_preload_like_link_url,
    document_owned_blocking_stylesheet_candidate_for_node, link_rel_includes_token,
    preload_like_link_loads_stylesheet, stylesheet_link_disposition,
    stylesheet_preload_link_request,
};
use crate::types::{AsyncSubresourceFetchResponseFilter, SubresourceResourceType};
use moli_fetch::{FetchPriorityHint, RequestCredentialsMode, RequestResourceType};

struct ConnectedLinkReadinessFetchResponse {
    response: crate::protocol_types::NavigationResponse,
    origin_clean: bool,
    load_event_successful: bool,
}

impl ConnectedLinkReadinessFetchResponse {
    fn new(
        response: crate::protocol_types::NavigationResponse,
        origin_clean: bool,
        load_event_successful: bool,
    ) -> Self {
        Self {
            response,
            origin_clean,
            load_event_successful,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ConnectedModulepreloadStart {
    request: NativeModuleSingleFetchRequest,
    link_client: Arc<NativeModulepreloadLinkClient>,
}

impl ConnectedModulepreloadStart {
    fn new(
        request: NativeModuleSingleFetchRequest,
        link_client: Arc<NativeModulepreloadLinkClient>,
    ) -> Self {
        Self {
            request,
            link_client,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        NativeModuleSingleFetchRequest,
        Arc<NativeModulepreloadLinkClient>,
    ) {
        (self.request, self.link_client)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ConnectedStyleLoadPrimeResult {
    modulepreload_starts: Vec<ConnectedModulepreloadStart>,
    runtime_warnings: Vec<String>,
    completed_stylesheet_clients: Vec<StylesheetLinkClientTerminal>,
}

/// One DOM-derived connected-style operation awaiting lifecycle commit.
///
/// The plan contains no `ContextHost` state and owns no load-delay token. Its
/// caller must commit `event_plan` and apply the result synchronously, before
/// running script or another DOM mutation.
#[derive(Debug)]
pub(crate) struct PreparedConnectedStyleLoad {
    owner: DomHandle,
    csp_blocked: bool,
    remember_before_initial_scan: bool,
    event_plan: ConnectedStyleLoadEventPlan,
}

#[derive(Debug)]
pub(crate) struct PreparedStylesheetOwnerRuntimeChange {
    owner: DomHandle,
    cached_linked_stylesheet_url: Option<Url>,
}

#[derive(Debug)]
pub(crate) struct PreparedStylesheetOwnerRuntimeChanges {
    canceled_load_event_bindings: Vec<MainDocumentStyleLoadEventBinding>,
    owner_changes: Vec<PreparedStylesheetOwnerRuntimeChange>,
}

impl PreparedStylesheetOwnerRuntimeChanges {
    fn new(
        canceled_load_event_bindings: Vec<MainDocumentStyleLoadEventBinding>,
        owner_changes: Vec<PreparedStylesheetOwnerRuntimeChange>,
    ) -> Self {
        Self {
            canceled_load_event_bindings,
            owner_changes,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<MainDocumentStyleLoadEventBinding>,
        Vec<PreparedStylesheetOwnerRuntimeChange>,
    ) {
        (self.canceled_load_event_bindings, self.owner_changes)
    }
}

impl PreparedStylesheetOwnerRuntimeChange {
    fn new(owner: DomHandle, cached_linked_stylesheet_url: Option<Url>) -> Self {
        Self {
            owner,
            cached_linked_stylesheet_url,
        }
    }

    pub(crate) fn owner(&self) -> DomHandle {
        self.owner
    }

    pub(crate) fn cached_linked_stylesheet_url(&self) -> Option<&Url> {
        self.cached_linked_stylesheet_url.as_ref()
    }
}

impl PreparedConnectedStyleLoad {
    fn new(
        owner: DomHandle,
        csp_blocked: bool,
        remember_before_initial_scan: bool,
        event_plan: ConnectedStyleLoadEventPlan,
    ) -> Self {
        Self {
            owner,
            csp_blocked,
            remember_before_initial_scan,
            event_plan,
        }
    }

    pub(crate) fn owner(&self) -> DomHandle {
        self.owner
    }

    pub(crate) fn event_plan(&self) -> ConnectedStyleLoadEventPlan {
        self.event_plan
    }
}

impl ConnectedStyleLoadPrimeResult {
    fn push_modulepreload_start(&mut self, start: ConnectedModulepreloadStart) {
        self.modulepreload_starts.push(start);
    }

    fn push_runtime_warning(&mut self, warning: impl Into<String>) {
        self.runtime_warnings.push(warning.into());
    }

    fn push_completed_stylesheet_client(&mut self, client: StylesheetLinkClientTerminal) {
        self.completed_stylesheet_clients.push(client);
    }

    pub(in crate::document_runtime) fn extend(&mut self, other: Self) {
        self.modulepreload_starts.extend(other.modulepreload_starts);
        self.runtime_warnings.extend(other.runtime_warnings);
        self.completed_stylesheet_clients
            .extend(other.completed_stylesheet_clients);
    }

    pub(crate) fn take_completed_stylesheet_clients(
        &mut self,
    ) -> Vec<StylesheetLinkClientTerminal> {
        std::mem::take(&mut self.completed_stylesheet_clients)
    }

    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (Vec<ConnectedModulepreloadStart>, Vec<String>) {
        debug_assert!(
            self.completed_stylesheet_clients.is_empty(),
            "completed stylesheet clients require immediate settlement"
        );
        (self.modulepreload_starts, self.runtime_warnings)
    }

    pub(crate) fn into_parts_with_completed_stylesheet_clients(
        self,
    ) -> (
        Vec<ConnectedModulepreloadStart>,
        Vec<String>,
        Vec<StylesheetLinkClientTerminal>,
    ) {
        (
            self.modulepreload_starts,
            self.runtime_warnings,
            self.completed_stylesheet_clients,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConnectedStyleOwnerKind {
    ClassicStyle,
    DeclarativeCssModule,
    Link,
}

impl ConnectedStyleOwnerKind {
    fn uses_connected_load_lifecycle(self) -> bool {
        !matches!(self, Self::DeclarativeCssModule)
    }
}

impl DocumentRuntime {
    pub(crate) fn prepare_initial_connected_style_loads(
        &mut self,
    ) -> Vec<PreparedConnectedStyleLoad> {
        if self.initial_connected_style_loads_queued {
            return Vec::new();
        }
        self.initial_connected_style_loads_queued = true;
        if self.author_styles_disabled() {
            self.stylesheet_lifecycle
                .pre_initial_scan_processed_owners
                .clear();
            return Vec::new();
        }
        self.late_preload_stylesheet_handles =
            self.collect_late_preload_stylesheet_handles_for_initial_scan(self.document_handle());
        let prepared = self.prepare_connected_style_loads(self.document_handle(), true);
        self.stylesheet_lifecycle
            .pre_initial_scan_processed_owners
            .clear();
        prepared
    }

    #[cfg(test)]
    pub(crate) fn queue_initial_connected_style_loads(&mut self) {
        let prepared = self.prepare_initial_connected_style_loads();
        self.commit_prepared_connected_style_loads_for_test(prepared);
    }

    #[cfg(test)]
    pub(crate) fn queue_connected_style_loads(&mut self, root: DomHandle) {
        let prepared = self.prepare_connected_style_loads(root, false);
        self.commit_prepared_connected_style_loads_for_test(prepared);
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn connected_style_load_handles(
        &self,
        root: DomHandle,
    ) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if let Some(element) = self.dom_host.node(handle).and_then(Node::as_element)
                && self.dom_host.is_connected(handle)
                && connected_style_owner_kind(element)
                    .is_some_and(ConnectedStyleOwnerKind::uses_connected_load_lifecycle)
            {
                handles.push(handle);
            }
            let mut next = self.dom_host.first_child(handle);
            let stack_start = stack.len();
            while let Some(child) = next {
                stack.push(child);
                next = self.dom_host.next_sibling(child);
            }
            stack[stack_start..].reverse();
        }
        handles
    }

    fn collect_late_preload_stylesheet_handles_for_initial_scan(
        &self,
        root: DomHandle,
    ) -> std::collections::HashSet<DomHandle> {
        let mut handles = std::collections::HashSet::new();
        let mut image_seen = false;
        let mut stack = vec![root];
        while let Some(handle) = stack.pop() {
            if let Some(element) = self.dom_host.node(handle).and_then(Node::as_element) {
                if image_seen && preload_link_loads_stylesheet(element) {
                    handles.insert(handle);
                }
                if parser_image_source_present(element) {
                    image_seen = true;
                }
            }

            let mut next = self.dom_host.first_child(handle);
            let stack_start = stack.len();
            while let Some(child) = next {
                stack.push(child);
                next = self.dom_host.next_sibling(child);
            }
            stack[stack_start..].reverse();
        }
        handles
    }

    pub(in crate::document_runtime) fn prepare_connected_style_loads(
        &mut self,
        root: DomHandle,
        skip_existing: bool,
    ) -> Vec<PreparedConnectedStyleLoad> {
        if self.author_styles_disabled() || !self.dom_host.is_connected(root) {
            return Vec::new();
        }
        let load_handles = if self.dom_host.node(root).is_some_and(Node::is_document)
            || self.dom_host.is_shadow_root(root)
        {
            self.dom_host
                .stylesheet_candidate_handles_for_tree_scope(root)
        } else {
            Arc::new(vec![root])
        };
        self.prepare_connected_style_load_handles(&load_handles, skip_existing)
    }

    fn prepare_connected_style_load_handles(
        &mut self,
        handles: &[DomHandle],
        skip_existing: bool,
    ) -> Vec<PreparedConnectedStyleLoad> {
        let mut owners = handles
            .iter()
            .copied()
            .filter_map(|handle| {
                let kind = self
                    .dom_host
                    .node(handle)
                    .and_then(Node::as_element)
                    .and_then(connected_style_owner_kind)?;
                (self.dom_host.is_connected(handle)
                    && (skip_existing
                        || self.initial_connected_style_loads_queued
                        || !self.parser_created_style_load_waits_for_initial_scan(handle)))
                .then_some((handle, kind))
            })
            .collect::<Vec<_>>();
        if skip_existing {
            owners.sort_by_key(|(handle, _)| {
                let node_id = NodeId::new(handle.index());
                if stylesheet_link_disposition(&self.dom_host, node_id)
                    .is_some_and(|disposition| disposition.is_blocking())
                {
                    0
                } else {
                    1
                }
            });
        }
        owners
            .into_iter()
            .filter_map(|(handle, kind)| {
                debug!(
                    handle = ?handle,
                    rel = self
                        .dom_host
                        .get_attribute(handle, "rel")
                        .unwrap_or_default(),
                    href = self
                        .dom_host
                        .get_attribute(handle, "href")
                        .unwrap_or_default(),
                    "processing connected style/link owner"
                );
                let processed_before_initial_scan = skip_existing
                    && self
                        .stylesheet_lifecycle
                        .pre_initial_scan_processed_owners
                        .contains(&handle);
                if skip_existing
                    && (processed_before_initial_scan
                        || self.connected_style_load_is_queued(handle))
                {
                    return None;
                }
                let csp_blocked = self.stylesheet_owner_is_csp_blocked(handle);
                if !csp_blocked && !kind.uses_connected_load_lifecycle() {
                    return None;
                }
                let event_plan = if !csp_blocked
                    && self.connected_owner_uses_non_blocking_modulepreload_identity(handle)
                {
                    ConnectedStyleLoadEventPlan::non_blocking_modulepreload(handle)
                } else {
                    ConnectedStyleLoadEventPlan::load_delaying(handle)
                };
                Some(PreparedConnectedStyleLoad::new(
                    handle,
                    csp_blocked,
                    !skip_existing && !self.initial_connected_style_loads_queued,
                    event_plan,
                ))
            })
            .collect()
    }

    /// Apply one already-committed plan without consulting ContextHost for
    /// lifecycle authority.
    ///
    /// `host_ptr` remains only for the pre-existing synchronous live-
    /// stylesheet reads performed while priming imports. It must never be used
    /// here to acquire or upgrade a load-event lease.
    pub(crate) fn apply_prepared_connected_style_load(
        &mut self,
        prepared: PreparedConnectedStyleLoad,
        inline_source: Option<Arc<crate::style_engine::OwnerStyleSheetSource>>,
        event_admission: ConnectedStyleLoadEventAdmission,
        host_ptr: *mut JsContextHost,
    ) -> ConnectedStyleLoadPrimeResult {
        assert!(
            event_admission.matches_plan(prepared.event_plan),
            "connected-style commit must match its synchronously prepared plan"
        );
        let handle = prepared.owner;
        if prepared.remember_before_initial_scan {
            self.stylesheet_lifecycle
                .pre_initial_scan_processed_owners
                .insert(handle);
        }
        if prepared.csp_blocked {
            self.complete_immediate_owner_processing(
                handle,
                self.connected_style_event_element_kind(handle),
                false,
                event_admission.load_event_binding(),
            );
            return ConnectedStyleLoadPrimeResult::default();
        }
        let inline_source = inline_source.or_else(|| self.inline_style_source_for_test(handle));
        self.enqueue_connected_style_load(handle, inline_source, host_ptr, Some(event_admission))
    }

    #[cfg(test)]
    fn commit_prepared_connected_style_loads_for_test(
        &mut self,
        prepared: Vec<PreparedConnectedStyleLoad>,
    ) {
        let mut result = ConnectedStyleLoadPrimeResult::default();
        for prepared in prepared {
            let plan = prepared.event_plan();
            let admission = match plan {
                ConnectedStyleLoadEventPlan::LoadDelaying { element } => {
                    ConnectedStyleLoadEventAdmission::LoadDelaying(
                        MainDocumentStyleLoadEventBinding::unowned_for_document_runtime_test(
                            element,
                        ),
                    )
                }
                ConnectedStyleLoadEventPlan::NonBlockingModulepreload { element } => {
                    ConnectedStyleLoadEventAdmission::NonBlockingModulepreload(
                        crate::frame_owner_model::DocumentLinkEventOwner::unowned_for_document_runtime_test(
                            element,
                        ),
                    )
                }
            };
            result.extend(self.apply_prepared_connected_style_load(
                prepared,
                None,
                admission,
                std::ptr::null_mut(),
            ));
        }
        self.pending_connected_style_load_prime_result
            .extend(result);
    }

    fn parser_created_style_load_waits_for_initial_scan(&self, handle: DomHandle) -> bool {
        let Some(node) = self.dom_host.node(handle) else {
            return false;
        };
        if !node.flags().parser_created() {
            return false;
        }
        let Some(element) = node.as_element() else {
            return false;
        };
        if super::is_inline_style_element(element) {
            return true;
        }
        if element
            .attribute("media")
            .is_none_or(|media| media.trim().is_empty())
        {
            return false;
        }
        stylesheet_link_disposition(&self.dom_host, NodeId::new(handle.index()))
            .is_some_and(|disposition| !disposition.is_blocking())
    }

    fn connected_style_load_is_queued(&self, handle: DomHandle) -> bool {
        self.stylesheet_lifecycle
            .pending_connected_loads
            .iter()
            .any(|queued| queued.owner() == handle)
            || self
                .stylesheet_lifecycle
                .owner_states
                .has_lifecycle_state(handle)
    }

    /// Choose the load-event lifecycle from the link relation, independently
    /// of whether the current request is valid or fetchable.
    ///
    /// HTML requires every `modulepreload` processing outcome to be
    /// non-load-delaying. Invalid `as`, non-matching media and malformed URLs
    /// therefore retain only exact Document/element identity for any terminal
    /// event. A stylesheet relation takes precedence when both tokens occur.
    fn connected_owner_uses_non_blocking_modulepreload_identity(&self, handle: DomHandle) -> bool {
        let node_id = NodeId::new(handle.index());
        if stylesheet_link_disposition(&self.dom_host, node_id).is_some() {
            return false;
        }
        self.dom_host
            .node(handle)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_element("link")
                    && element
                        .attribute("rel")
                        .is_some_and(|rel| link_rel_includes_token(rel, "modulepreload"))
            })
    }

    fn settle_connected_style_load_binding(
        &mut self,
        host_ptr: *mut JsContextHost,
        binding: Option<MainDocumentStyleLoadEventBinding>,
        reason: &'static str,
    ) -> bool {
        let Some(binding) = binding else {
            return false;
        };
        #[cfg(test)]
        if host_ptr.is_null() {
            tracing::debug!(?binding, reason, "settled test-only connected style load");
            return true;
        }
        let settled = unsafe { &mut *host_ptr }.settle_main_style_load_event(binding);
        tracing::debug!(
            owner = ?binding.owner(),
            element = ?binding.element(),
            load_delay_token = ?binding.load_delay_token(),
            settled,
            reason,
            "settled main connected style load before event posting"
        );
        settled
    }

    fn settle_connected_style_load_admission(
        &mut self,
        host_ptr: *mut JsContextHost,
        admission: Option<ConnectedStyleLoadEventAdmission>,
        reason: &'static str,
    ) -> bool {
        self.settle_connected_style_load_binding(
            host_ptr,
            admission.and_then(ConnectedStyleLoadEventAdmission::load_event_binding),
            reason,
        )
    }

    fn load_event_binding_for_connected_admission(
        admission: Option<ConnectedStyleLoadEventAdmission>,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        admission.and_then(ConnectedStyleLoadEventAdmission::load_event_binding)
    }

    #[cfg(test)]
    pub(crate) fn connected_style_load_is_queued_for_test(&self, handle: DomHandle) -> bool {
        self.connected_style_load_is_queued(handle)
    }

    pub(super) fn push_ready_connected_style_load(&mut self, ready: ReadyConnectedStyleLoad) {
        let producer = self
            .stylesheet_lifecycle
            .task_producer
            .as_ref()
            .expect("a live Document must bind its stylesheet Page task producer");
        let sent = producer.send_connected_style_event(ready);
        assert!(
            sent.is_ok(),
            "a live stylesheet owner must publish its event to the bound Page source"
        );
    }

    fn complete_immediate_owner_processing(
        &mut self,
        handle: DomHandle,
        element_kind: ConnectedStyleEventElementKind,
        successful: bool,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
    ) {
        self.stylesheet_lifecycle
            .owner_states
            .clear_async_operations(handle);
        let operation = ConnectedLoadOperation::new_with_load_event_binding(
            handle,
            element_kind,
            ConnectedLoadParameters::ImmediateOwnerProcessing,
            None,
            load_event_binding,
        );
        self.stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));
        let _ = self
            .stylesheet_lifecycle
            .owner_states
            .accept_completion(&operation, 0, true);
        self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
            operation, successful,
        ));
    }

    fn connected_style_event_element_kind(
        &self,
        handle: DomHandle,
    ) -> ConnectedStyleEventElementKind {
        if self.dom_host.is_inline_style_sheet_owner(handle) {
            ConnectedStyleEventElementKind::Style
        } else if self.dom_host.is_html_element_named(handle, "link") {
            ConnectedStyleEventElementKind::Link
        } else {
            panic!(
                "connected stylesheet processing owner {handle:?} must be a <style> or <link> element"
            );
        }
    }

    fn enqueue_connected_style_load(
        &mut self,
        handle: DomHandle,
        inline_source: Option<Arc<crate::style_engine::OwnerStyleSheetSource>>,
        host_ptr: *mut JsContextHost,
        event_admission: Option<ConnectedStyleLoadEventAdmission>,
    ) -> ConnectedStyleLoadPrimeResult {
        if self.current_document_resource_loader().is_some()
            && !self.parser_created_style_import_waits_for_blocking_discovery(handle)
        {
            self.prime_connected_style_load_handle(handle, inline_source, host_ptr, event_admission)
        } else {
            self.stylesheet_lifecycle.pending_connected_loads.push_back(
                QueuedConnectedStyleLoad::new(handle, inline_source, event_admission),
            );
            ConnectedStyleLoadPrimeResult::default()
        }
    }

    /// A parser-created inline sheet has one canonical import load graph.  The
    /// parser reports the blocking discovery input only after the DOM mutation
    /// that closes the `<style>` element has been applied.  During that small
    /// handoff window, starting the generic connected-owner operation would
    /// duplicate the imports that the blocking operation is about to start.
    fn parser_created_style_import_waits_for_blocking_discovery(&self, handle: DomHandle) -> bool {
        if !self.is_parser_insertion_session_active() {
            return false;
        }
        let Some(node) = self.dom_host.node(handle) else {
            return false;
        };
        if !node.flags().parser_created() || !node.is_html_element_named("style") {
            return false;
        }
        document_owned_blocking_stylesheet_candidate_for_node(
            &self.dom_host,
            NodeId::new(handle.index()),
        )
        .is_some()
    }

    pub(crate) fn has_pending_style_loads(&self) -> bool {
        !self.stylesheet_lifecycle.pending_connected_loads.is_empty()
            || self
                .stylesheet_lifecycle
                .owner_states
                .has_pending_connected_operation()
            || self
                .stylesheet_lifecycle
                .owner_states
                .has_pending_link_state()
    }

    pub(in crate::document_runtime) fn prime_pending_connected_style_loads_for_owner(
        &mut self,
        host_ptr: *mut JsContextHost,
    ) -> ConnectedStyleLoadPrimeResult {
        let pending_loads = std::mem::take(&mut self.stylesheet_lifecycle.pending_connected_loads);
        let mut result = std::mem::take(&mut self.pending_connected_style_load_prime_result);
        for queued in pending_loads {
            result.extend(self.prime_connected_style_load_handle(
                queued.owner(),
                queued.inline_source().cloned(),
                host_ptr,
                queued.event_admission(),
            ));
        }
        result
    }

    #[cfg(test)]
    pub(super) fn prime_pending_connected_style_loads(&mut self) -> ConnectedStyleLoadPrimeResult {
        self.prime_pending_connected_style_loads_for_owner(std::ptr::null_mut())
    }

    fn prime_connected_style_load_handle(
        &mut self,
        handle: DomHandle,
        inline_source: Option<Arc<crate::style_engine::OwnerStyleSheetSource>>,
        host_ptr: *mut JsContextHost,
        event_admission: Option<ConnectedStyleLoadEventAdmission>,
    ) -> ConnectedStyleLoadPrimeResult {
        let mut result = ConnectedStyleLoadPrimeResult::default();
        if !self.dom_host.is_connected(handle) {
            self.settle_connected_style_load_admission(
                host_ptr,
                event_admission,
                "style owner disconnected before processing",
            );
            self.invalidate_stylesheet_owner_operations(handle);
            return result;
        }
        if let Some(admission) = event_admission {
            let expects_modulepreload_identity =
                self.connected_owner_uses_non_blocking_modulepreload_identity(handle);
            let admission_matches_current_processing = matches!(
                (expects_modulepreload_identity, admission),
                (
                    true,
                    ConnectedStyleLoadEventAdmission::NonBlockingModulepreload(_)
                ) | (false, ConnectedStyleLoadEventAdmission::LoadDelaying(_))
            );
            if !admission_matches_current_processing {
                self.settle_connected_style_load_admission(
                    host_ptr,
                    Some(admission),
                    "connected style owner changed after lifecycle commit",
                );
                self.invalidate_stylesheet_owner_operations(handle);
                tracing::debug!(
                    ?handle,
                    expects_modulepreload_identity,
                    "discarded stale connected-style lifecycle commit"
                );
                return result;
            }
        }
        let element_kind = self.connected_style_event_element_kind(handle);
        let node_id = NodeId::new(handle.index());
        if let Some(disposition) = stylesheet_link_disposition(&self.dom_host, node_id) {
            let load_event_binding =
                Self::load_event_binding_for_connected_admission(event_admission);
            let stylesheet_fetcher = self.stylesheet_fetcher();
            let document_url = self
                .dom_host
                .node(self.dom_host.document_handle())
                .and_then(Node::as_document)
                .map(|document| document.url().clone())
                .expect("live dom host must retain a document url");
            if disposition.is_blocking() {
                self.note_discovered_live_blocking_stylesheets();
            }
            let fetch = self.stylesheet_lifecycle.fetches.adopt_or_begin_link_load(
                &stylesheet_fetcher,
                node_id,
                document_url.clone(),
                disposition.url().clone(),
                disposition.options().clone(),
            );
            let import_completion =
                self.initial_stylesheet_import_completion(disposition.url(), &fetch);
            let existing_load = self
                .stylesheet_lifecycle
                .owner_states
                .link_state(handle)
                .map(|state| Arc::clone(state.active_load()))
                .filter(|load| {
                    load.installs_stylesheet()
                        && load.request_url() == disposition.url()
                        && load.fetch().ptr_eq(&fetch)
                });
            let load = if let Some(load) = existing_load {
                if let Some(binding) = load_event_binding
                    && !load.bind_load_event(binding)
                {
                    self.settle_connected_style_load_binding(
                        host_ptr,
                        Some(binding),
                        "duplicate stylesheet owner admission",
                    );
                }
                load
            } else {
                let load = StylesheetLinkClient::new_with_load_event_binding(
                    handle,
                    disposition.url().clone(),
                    fetch,
                    load_event_binding,
                );
                self.install_stylesheet_link_state(
                    handle,
                    LinkStyleState::new(Arc::clone(&load), import_completion),
                );
                load
            };
            // `adopt_or_begin_link_load` may attach this owner to a fetch that
            // completed for an earlier link. Such an admission has no future
            // network terminal, so deliver only this exact client now.
            if let Some(client) = self.promote_stylesheet_link_client_if_ready(Arc::clone(&load)) {
                result.push_completed_stylesheet_client(client);
            }
            // Network and data responses are processed through the same exact
            // stylesheet terminal. That boundary owns import discovery and
            // starts the shared dependency graph once per physical fetch.
            self.stylesheet_lifecycle
                .owner_states
                .clear_connected_operation(handle);
            return result;
        }
        if let Some(url) = connected_preload_like_link_url(&self.dom_host, node_id) {
            let element = self.dom_host.node(handle).and_then(Node::as_element);
            let resource_type = element
                .map(preload_like_link_resource_type)
                .unwrap_or(SubresourceResourceType::Fetch);
            let is_modulepreload =
                self.connected_owner_uses_non_blocking_modulepreload_identity(handle);
            if !self.document_scripting_enabled()
                && (is_modulepreload || resource_type == SubresourceResourceType::Script)
            {
                // Script-disabled Documents do not even fetch page-authored
                // preload/modulepreload resources. Release any load-event
                // lease without posting a synthetic terminal event.
                self.stylesheet_lifecycle
                    .owner_states
                    .clear_async_operations(handle);
                self.settle_connected_style_load_admission(
                    host_ptr,
                    event_admission,
                    "script execution disabled",
                );
                return result;
            }
            if let Some((key, preload)) = self.connected_modulepreload_request(handle, &url) {
                let unchanged = self
                    .stylesheet_lifecycle
                    .owner_states
                    .pending_native_modulepreload(handle)
                    .is_some_and(|client| client.key() == &key);
                if unchanged {
                    self.settle_connected_style_load_admission(
                        host_ptr,
                        event_admission,
                        "duplicate modulepreload owner admission",
                    );
                    return result;
                }
                let main_document_event_owner =
                    event_admission.and_then(|admission| match admission {
                        ConnectedStyleLoadEventAdmission::NonBlockingModulepreload(owner) => {
                            Some(owner)
                        }
                        ConnectedStyleLoadEventAdmission::LoadDelaying(_) => None,
                    });
                let Some(main_document_event_owner) = main_document_event_owner else {
                    tracing::debug!(
                        ?handle,
                        "discarded main modulepreload without an exact Document event owner"
                    );
                    return result;
                };
                let client = NativeModulepreloadLinkClient::new_with_main_document_event_owner(
                    handle,
                    key,
                    main_document_event_owner,
                );
                self.stylesheet_lifecycle
                    .owner_states
                    .install_pending_native_modulepreload(Arc::clone(&client));
                result.push_modulepreload_start(ConnectedModulepreloadStart::new(preload, client));
                return result;
            }
            let load_event_binding =
                Self::load_event_binding_for_connected_admission(event_admission);
            if connected_modulepreload_has_non_matching_media(&self.dom_host, handle) {
                self.stylesheet_lifecycle
                    .owner_states
                    .clear_async_operations(handle);
                self.settle_connected_style_load_binding(
                    host_ptr,
                    load_event_binding,
                    "modulepreload media did not match",
                );
                return result;
            }
            if let Some(invalid_as) = connected_modulepreload_invalid_as(&self.dom_host, handle) {
                self.stylesheet_lifecycle
                    .owner_states
                    .clear_async_operations(handle);
                if !self.modulepreload_invalid_as_link_errors.insert(handle) {
                    self.settle_connected_style_load_binding(
                        host_ptr,
                        load_event_binding,
                        "duplicate invalid modulepreload processing",
                    );
                    return result;
                }
                result.push_runtime_warning(format!(
                    "<link rel=modulepreload> has an invalid `as` value {invalid_as}"
                ));
                let operation = ConnectedLoadOperation::new_with_load_event_binding(
                    handle,
                    element_kind,
                    ConnectedLoadParameters::ImmediateOwnerProcessing,
                    None,
                    load_event_binding,
                );
                self.stylesheet_lifecycle
                    .owner_states
                    .install_pending_operation(Arc::clone(&operation));
                let _ = self
                    .stylesheet_lifecycle
                    .owner_states
                    .accept_completion(&operation, 0, true);
                self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
                    operation, false,
                ));
                return result;
            }
            if let Some(request) = stylesheet_preload_link_request(&self.dom_host, node_id) {
                let request_resource_type = element
                    .and_then(|element| {
                        preload_like_link_request_resource_type(
                            element,
                            SubresourceResourceType::Stylesheet,
                            self.late_preload_stylesheet_handles.contains(&handle),
                        )
                    })
                    .unwrap_or(RequestResourceType::CssStyleSheet);
                let Ok(fetch) = self.preload_stylesheet_with_request_metadata(
                    request.url().clone(),
                    request.options().clone(),
                    request_resource_type,
                    true,
                ) else {
                    self.complete_immediate_owner_processing(
                        handle,
                        element_kind,
                        false,
                        load_event_binding,
                    );
                    return result;
                };
                let load = StylesheetLinkClient::new_preload_with_load_event_binding(
                    handle,
                    request.url().clone(),
                    fetch,
                    load_event_binding,
                );
                self.install_stylesheet_link_state(
                    handle,
                    LinkStyleState::new(Arc::clone(&load), StylesheetCompletionState::Succeeded),
                );
                if let Some(client) = self.promote_stylesheet_link_client_if_ready(load) {
                    result.push_completed_stylesheet_client(client);
                }
                return result;
            }
            let fetch_options = element
                .map(|element| {
                    preload_like_link_readiness_fetch_options(
                        element,
                        self.late_preload_stylesheet_handles.contains(&handle),
                    )
                })
                .unwrap_or_else(|| ConnectedLinkReadinessFetchOptions {
                    resource_type,
                    request_resource_type: crate::network::request_resource_type_for_subresource(
                        resource_type,
                    ),
                    script_fetch_metadata: None,
                    request_mode: moli_fetch::RequestMode::Cors,
                    credentials_mode: RequestCredentialsMode::Include,
                    fetch_priority_hint: None,
                    link_preload: false,
                    link_fetch_options: StylesheetFetchOptions::default(),
                });
            let parameters = ConnectedLoadParameters::PreloadLikeLink {
                url: url.clone(),
                options: Arc::new(fetch_options.clone()),
            };
            let unchanged = self
                .stylesheet_lifecycle
                .owner_states
                .pending_operation(handle)
                .is_some_and(|pending| pending.parameters == parameters);
            if unchanged {
                return result;
            }
            let operation = ConnectedLoadOperation::new_with_load_event_binding(
                handle,
                element_kind,
                parameters,
                None,
                load_event_binding,
            );
            self.stylesheet_lifecycle
                .owner_states
                .install_pending_operation(Arc::clone(&operation));
            let resource_loader = self
                .current_document_resource_loader()
                .expect("connected preload-like link requires its Document authority");
            let loader = resource_loader.request_client().clone();
            if !loader.optional_resource_fetch_enabled(resource_type) {
                let _ = self
                    .stylesheet_lifecycle
                    .owner_states
                    .accept_completion(&operation, 0, true);
                self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
                    operation, true,
                ));
                return result;
            }
            let task_producer = self
                .stylesheet_lifecycle
                .task_producer
                .clone()
                .expect("connected stylesheet fetch requires a bound Page task producer");
            let service_worker_context = self
                .stylesheet_lifecycle
                .service_worker_connected_link_context
                .clone();
            let document_url = self
                .dom_host
                .node(self.dom_host.document_handle())
                .and_then(Node::as_document)
                .map(|document| document.url().clone())
                .expect("live dom host must retain a document url")
                .clone();
            let start_unix_millis = moli_time::unix_epoch_millis();
            let resource_task_runner = resource_loader.task_runner();
            resource_loader.spawn_resource_task(async move {
                let result = fetch_connected_link_readiness_with_service_worker(
                    loader,
                    resource_task_runner,
                    document_url.clone(),
                    url.clone(),
                    fetch_options,
                    service_worker_context,
                )
                .await;
                let completion = ConnectedLoadCompletion {
                    operation,
                    successful: result
                        .as_ref()
                        .is_ok_and(|response| response.load_event_successful),
                    network_results: vec![ConnectedLoadNetworkResult {
                        stylesheet_fetch: None,
                        blocking_operation: None,
                        source_operation: None,
                        import_roots: Vec::new(),
                        document_url,
                        request_url: url,
                        source_owners: vec![handle],
                        resource_type,
                        start_unix_millis: Some(start_unix_millis),
                        origin_clean: result.as_ref().is_ok_and(|response| response.origin_clean),
                        result: result.map(|response| response.response),
                    }],
                };
                let _ = task_producer.send_connected_completion(completion);
            });
            return result;
        }
        let load_event_binding = Self::load_event_binding_for_connected_admission(event_admission);
        if let Some(source) = inline_source
            && source.owner() == handle
        {
            let urls = if host_ptr.is_null() {
                source.import_urls().to_vec()
            } else {
                unsafe { &*host_ptr }
                    .owner_live_stylesheet(handle)
                    .map(|stylesheet| {
                        stylesheet
                            .pending_import_requests_in_graph()
                            .into_iter()
                            .map(|request| request.url)
                            .collect()
                    })
                    .unwrap_or_else(|| source.import_urls().to_vec())
            };
            if !urls.is_empty() {
                self.prime_connected_style_import_loads(
                    InlineStyleImportSource::new(source),
                    urls,
                    load_event_binding,
                    host_ptr,
                );
                return result;
            }
            let source = InlineStyleImportSource::new(source);
            let roots = self.connected_style_import_roots(&source, host_ptr);
            let operation = ConnectedLoadOperation::new_with_load_event_binding(
                handle,
                element_kind,
                ConnectedLoadParameters::StyleImports {
                    source,
                    urls,
                    roots,
                },
                None,
                load_event_binding,
            );
            self.stylesheet_lifecycle
                .owner_states
                .install_pending_operation(Arc::clone(&operation));
            let _ = self
                .stylesheet_lifecycle
                .owner_states
                .accept_completion(&operation, 0, true);
            self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
                operation, true,
            ));
            return result;
        }
        self.stylesheet_lifecycle
            .owner_states
            .clear_async_operations(handle);
        let operation = ConnectedLoadOperation::new_with_load_event_binding(
            handle,
            element_kind,
            ConnectedLoadParameters::ImmediateOwnerProcessing,
            None,
            load_event_binding,
        );
        self.stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));
        let _ = self
            .stylesheet_lifecycle
            .owner_states
            .accept_completion(&operation, 0, true);
        self.push_ready_connected_style_load(ReadyConnectedStyleLoad::for_operation(
            operation, true,
        ));
        result
    }

    fn prime_connected_style_import_loads(
        &mut self,
        source: InlineStyleImportSource,
        urls: Vec<Url>,
        load_event_binding: Option<MainDocumentStyleLoadEventBinding>,
        host_ptr: *mut JsContextHost,
    ) {
        let handle = source.owner();
        let roots = self.connected_style_import_roots(&source, host_ptr);
        let urls = match connected_style_import_readiness(urls.clone()) {
            ConnectedStyleImportReadiness::Ready(mut successful) => {
                if !host_ptr.is_null() {
                    for root in &roots {
                        if let Some(graph_successful) = unsafe { &*host_ptr }
                            .install_live_stylesheet_import_graph(root.clone(), &[])
                        {
                            successful &= graph_successful;
                            let _ = unsafe { &mut *host_ptr }
                                .refresh_live_stylesheet_after_import_graph(
                                    root.owner,
                                    root.stylesheet_id,
                                );
                        }
                    }
                }
                let operation = ConnectedLoadOperation::new_with_load_event_binding(
                    handle,
                    ConnectedStyleEventElementKind::Style,
                    ConnectedLoadParameters::StyleImports {
                        source,
                        urls,
                        roots,
                    },
                    None,
                    load_event_binding,
                );
                self.stylesheet_lifecycle
                    .owner_states
                    .install_pending_operation(Arc::clone(&operation));
                let _ = self
                    .stylesheet_lifecycle
                    .owner_states
                    .accept_completion(&operation, 0, true);
                self.note_connected_style_import_completion(&operation, successful);
                return;
            }
            ConnectedStyleImportReadiness::Pending(urls) => urls,
        };
        let parameters = ConnectedLoadParameters::StyleImports {
            source,
            urls: urls.clone(),
            roots,
        };
        let blocking_signature =
            DocumentBlockingStylesheetSignature::ParserCreatedStyleImport { urls: urls.clone() };
        let blocking_operation = self
            .stylesheet_lifecycle
            .fetches
            .blocking_operation(NodeId::new(handle.index()), &blocking_signature);
        let unchanged = self
            .stylesheet_lifecycle
            .owner_states
            .pending_operation(handle)
            .is_some_and(|pending| {
                pending.matches_processing(&parameters, blocking_operation.as_ref())
            });
        if unchanged {
            return;
        }
        let operation = ConnectedLoadOperation::new_with_load_event_binding(
            handle,
            ConnectedStyleEventElementKind::Style,
            parameters,
            blocking_operation,
            load_event_binding,
        );
        self.stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));
        if self.connected_style_import_uses_blocking_stylesheet(&operation) {
            return;
        }
        let task_producer = self
            .stylesheet_lifecycle
            .task_producer
            .clone()
            .expect("connected stylesheet import requires a bound Page task producer");
        let stylesheet_fetcher = self.stylesheet_fetcher();
        let document_url = self
            .dom_host
            .node(self.dom_host.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
            .expect("live dom host must retain a document url")
            .clone();
        let resource_loader = self
            .current_document_resource_loader()
            .expect("connected stylesheet import requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let (graph, network_results) = fetch_observed_stylesheet_import_graph(
                stylesheet_fetcher,
                document_url,
                urls,
                vec![handle],
            )
            .await;
            let completion = ConnectedLoadCompletion {
                operation,
                successful: graph.successful(),
                network_results,
            };
            let _ = task_producer.send_connected_completion(completion);
        });
    }

    fn connected_style_import_roots(
        &self,
        source: &InlineStyleImportSource,
        host_ptr: *mut JsContextHost,
    ) -> Vec<ConnectedStyleImportRoot> {
        if host_ptr.is_null() {
            return Vec::new();
        }
        let owner = source.owner();
        unsafe { &*host_ptr }
            .owner_live_stylesheet(owner)
            .map(|stylesheet| ConnectedStyleImportRoot::new(owner, &stylesheet, false))
            .into_iter()
            .collect()
    }

    fn prime_live_stylesheet_import_loads(
        &mut self,
        owner: DomHandle,
        stylesheet: crate::live_stylesheet::LiveStylesheetRef,
        root_is_external_resource: bool,
        host_ptr: *mut JsContextHost,
    ) {
        let urls = stylesheet
            .pending_import_requests_in_graph()
            .into_iter()
            .map(|request| request.url)
            .collect::<Vec<_>>();
        if urls.is_empty() {
            return;
        }
        let root = ConnectedStyleImportRoot::new(owner, &stylesheet, root_is_external_resource);
        let urls = match connected_style_import_readiness(urls) {
            ConnectedStyleImportReadiness::Ready(_) => {
                if unsafe { &*host_ptr }
                    .install_live_stylesheet_import_graph(root.clone(), &[])
                    .is_some()
                {
                    let _ = unsafe { &mut *host_ptr }
                        .refresh_live_stylesheet_after_import_graph(owner, stylesheet.id());
                }
                return;
            }
            ConnectedStyleImportReadiness::Pending(urls) => urls,
        };
        let task_producer = self
            .stylesheet_lifecycle
            .task_producer
            .clone()
            .expect("live stylesheet import requires a bound Page task producer");
        let stylesheet_fetcher = self.stylesheet_fetcher();
        let document_url = self
            .dom_host
            .node(self.dom_host.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())
            .expect("live dom host must retain a document url");
        let resource_loader = self
            .current_document_resource_loader()
            .expect("live stylesheet import requires its Document authority");
        resource_loader.spawn_resource_task(async move {
            let (_, mut network_results) = fetch_observed_stylesheet_import_graph(
                stylesheet_fetcher,
                document_url,
                urls,
                vec![owner],
            )
            .await;
            for result in &mut network_results {
                result.import_roots.push(root.clone());
            }
            let _ = task_producer.send_live_import_completion(LiveStylesheetImportLoadCompletion {
                network_results,
            });
        });
    }

    pub(crate) fn prime_document_lifecycle_processing_for_owner(
        &mut self,
        host_ptr: *mut JsContextHost,
    ) -> ConnectedStyleLoadPrimeResult {
        self.prime_pending_connected_style_loads_for_owner(host_ptr)
    }

    #[cfg(test)]
    pub(crate) fn prime_document_lifecycle_processing(&mut self) -> ConnectedStyleLoadPrimeResult {
        self.prime_document_lifecycle_processing_for_owner(std::ptr::null_mut())
    }

    pub(crate) fn prepare_stylesheet_owner_runtime_changes(
        &mut self,
        changes: &[crate::dom::native::DomStylesheetOwnerChange],
    ) -> PreparedStylesheetOwnerRuntimeChanges {
        let mut transitions = Vec::<(DomHandle, bool)>::new();
        for change in changes {
            let should_queue = match change.kind() {
                DomStylesheetOwnerChangeKind::Registered
                | DomStylesheetOwnerChangeKind::Contents
                | DomStylesheetOwnerChangeKind::OwnerDocumentChanged
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: true } => true,
                DomStylesheetOwnerChangeKind::Unregistered
                | DomStylesheetOwnerChangeKind::TreeConnectionChanged { connected: false } => false,
                DomStylesheetOwnerChangeKind::Attribute {
                    namespace,
                    local_name,
                } => {
                    if self.dom_host.is_inline_style_sheet_owner(change.owner())
                        && local_name == "disabled"
                    {
                        continue;
                    }
                    if namespace.is_some()
                        || !super::attribute_reprocesses_connected_stylesheet(local_name)
                    {
                        continue;
                    }
                    let disabled = local_name == "disabled"
                        && self
                            .dom_host
                            .get_attribute(change.owner(), "disabled")
                            .is_some();
                    !disabled
                }
            };
            if let Some((_, queued)) = transitions
                .iter_mut()
                .find(|(owner, _)| *owner == change.owner())
            {
                *queued = should_queue;
            } else {
                transitions.push((change.owner(), should_queue));
            }
        }

        let mut canceled_load_event_bindings = Vec::new();
        let mut prepared = Vec::new();
        for (owner, should_queue) in transitions {
            canceled_load_event_bindings.extend(self.invalidate_style_related_state(owner));
            if !should_queue {
                continue;
            }
            let cached_linked_stylesheet_url = self
                .dom_host
                .is_html_element_named(owner, "link")
                .then(|| stylesheet_link_disposition(&self.dom_host, NodeId::new(owner.index())))
                .flatten()
                .map(|disposition| disposition.url().clone());
            prepared.push(PreparedStylesheetOwnerRuntimeChange::new(
                owner,
                cached_linked_stylesheet_url,
            ));
        }
        PreparedStylesheetOwnerRuntimeChanges::new(canceled_load_event_bindings, prepared)
    }

    pub(crate) fn apply_inline_cssom_source_change_after_invalidation(
        &mut self,
        host_ptr: *mut JsContextHost,
        owner: DomHandle,
    ) {
        if self.author_styles_disabled() {
            return;
        }
        self.queue_stylesheet_source_css_projection(owner);
        if let Some(stylesheet) = unsafe { &*host_ptr }.owner_live_stylesheet(owner) {
            self.prime_live_stylesheet_import_loads(owner, stylesheet, false, host_ptr);
        }
    }

    pub(crate) fn apply_linked_cssom_source_change(
        &mut self,
        host_ptr: *mut JsContextHost,
        owner: DomHandle,
    ) {
        if self.author_styles_disabled() {
            return;
        }
        self.queue_stylesheet_source_css_projection(owner);
        if let Some(stylesheet) = unsafe { &*host_ptr }.linked_live_stylesheet(owner) {
            self.prime_live_stylesheet_import_loads(owner, stylesheet, true, host_ptr);
        }
    }

    pub(crate) fn queue_stylesheet_source_css_projection(&mut self, owner: DomHandle) {
        if !self
            .pending_stylesheet_source_css_projection_owners
            .contains(&owner)
        {
            self.pending_stylesheet_source_css_projection_owners
                .push(owner);
        }
    }

    pub(crate) fn apply_pending_stylesheet_source_css_projections(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        if self.author_styles_disabled() {
            self.pending_stylesheet_source_css_projection_owners.clear();
            return;
        }
        let owners = std::mem::take(&mut self.pending_stylesheet_source_css_projection_owners);
        for owner in owners {
            crate::native_bridge::document::apply_stylesheet_source_css_projection(
                scope,
                unsafe { &*host_ptr },
                owner,
            );
        }
    }

    pub(crate) fn invalidate_style_related_state(
        &mut self,
        handle: DomHandle,
    ) -> Vec<MainDocumentStyleLoadEventBinding> {
        let had_pending_blocker = !self.parser_blocking_stylesheet_release_is_ready();
        let node_id = NodeId::new(handle.index());
        self.stylesheet_lifecycle
            .pre_initial_scan_processed_owners
            .remove(&handle);
        let mut canceled_bindings = Vec::new();
        self.stylesheet_lifecycle
            .pending_connected_loads
            .retain(|candidate| {
                if candidate.owner() != handle {
                    return true;
                }
                canceled_bindings.extend(
                    candidate
                        .event_admission()
                        .and_then(ConnectedStyleLoadEventAdmission::load_event_binding),
                );
                false
            });
        canceled_bindings.extend(
            self.stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(handle),
        );
        self.invalidate_stylesheet_owner_operations(handle);
        self.modulepreload_invalid_as_link_errors.remove(&handle);
        self.stylesheet_lifecycle.fetches.invalidate_node(node_id);
        self.request_parser_if_stylesheet_blockers_released(had_pending_blocker);
        canceled_bindings
    }

    pub(crate) fn has_pending_ready_connected_style_loads(&mut self) -> bool {
        #[cfg(test)]
        {
            self.has_connected_style_event_for_test()
                || !self
                    .stylesheet_lifecycle
                    .injected_ready_connected_loads
                    .is_empty()
        }
        #[cfg(not(test))]
        {
            false
        }
    }

    pub(crate) fn dispatch_pending_style_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        ready: ReadyConnectedStyleLoad,
    ) -> bool {
        self.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
        let handle = ready.owner();
        let native_modulepreload = match ready.operation() {
            ReadyConnectedStyleLoadOperation::Connected(operation) => {
                self.stylesheet_lifecycle
                    .owner_states
                    .consume_operation_event(operation);
                None
            }
            ReadyConnectedStyleLoadOperation::StylesheetLink(load) => {
                self.stylesheet_lifecycle
                    .owner_states
                    .consume_link_event(load);
                None
            }
            ReadyConnectedStyleLoadOperation::NativeModulepreload(client) => {
                self.stylesheet_lifecycle
                    .owner_states
                    .consume_native_modulepreload_event(client);
                Some(client)
            }
        };
        let load_succeeded = native_modulepreload
            .and_then(|client| self.native_modulepreload_style_result(host_ptr, client))
            .unwrap_or_else(|| ready.successful());
        self.dispatch_style_or_link_load_event(scope, host_ptr, handle, load_succeeded);
        true
    }

    pub(crate) fn dispatch_preload_like_link_error_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) {
        self.dispatch_style_or_link_load_event(scope, host_ptr, handle, false);
    }

    fn dispatch_style_or_link_load_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        load_succeeded: bool,
    ) {
        let Some(_element) = self.dom_host.node(handle).and_then(Node::as_element) else {
            return;
        };
        let event_name = if load_succeeded { "load" } else { "error" };
        let Some(event) =
            native_bridge::element::construct_simple_event(scope, event_name, false, false, false)
        else {
            return;
        };
        debug!(
            handle = ?handle,
            rel = self
                .dom_host
                .get_attribute(handle, "rel")
                .unwrap_or_default(),
            href = self
                .dom_host
                .get_attribute(handle, "href")
                .unwrap_or_default(),
            event = event_name,
            "dispatching pending style/link load"
        );
        let _ = native_bridge::element::dispatch_public_event(scope, host_ptr, handle, event);
    }

    fn native_modulepreload_style_result(
        &mut self,
        host_ptr: *mut JsContextHost,
        client: &Arc<NativeModulepreloadLinkClient>,
    ) -> Option<bool> {
        if client.key().kind() != crate::module_runtime::ModuleKind::Css {
            return None;
        }
        let url = client.key().url();
        let host = unsafe { &*host_ptr };
        if host.css_module_failed_for_url(url) {
            return Some(false);
        }
        host.css_module_text_for_url(url).map(|_| true)
    }

    #[cfg(test)]
    fn inline_style_source_for_test(
        &self,
        handle: DomHandle,
    ) -> Option<Arc<crate::style_engine::OwnerStyleSheetSource>> {
        let element = self.dom_host.node(handle).and_then(Node::as_element)?;
        if !super::is_inline_style_element(element) {
            return None;
        }
        let parser_base = self
            .dom_host
            .owner_document_handle(handle)
            .and_then(|document| self.dom_host.node(document))
            .and_then(Node::as_document)
            .map(|document| document.base_url().clone())
            .or_else(|| self.dom_host.document_base_url())
            .or_else(|| self.dom_host.document_url().cloned())?;
        Some(Arc::new(crate::style_engine::OwnerStyleSheetSource::new(
            handle,
            self.dom_host.text_content(handle).unwrap_or_default(),
            parser_base,
        )))
    }

    #[cfg(not(test))]
    fn inline_style_source_for_test(
        &self,
        _handle: DomHandle,
    ) -> Option<Arc<crate::style_engine::OwnerStyleSheetSource>> {
        None
    }

    pub(crate) fn pop_ready_connected_style_load(&mut self) -> Option<ReadyConnectedStyleLoad> {
        #[cfg(test)]
        {
            if let Some(ready) = self.pop_connected_style_event_for_test() {
                return Some(ready);
            }
            self.stylesheet_lifecycle
                .injected_ready_connected_loads
                .pop_front()
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    pub(crate) fn accept_native_modulepreload_link_client_terminals(
        &mut self,
        key: &ModuleMapKey,
        clients: Vec<Arc<NativeModulepreloadLinkClient>>,
        successful: bool,
    ) -> Vec<PendingNativeModulepreloadLinkEvent> {
        let mut accepted = Vec::new();
        for client in clients {
            if client.key() != key
                || !self
                    .stylesheet_lifecycle
                    .owner_states
                    .accept_native_modulepreload_completion(&client)
            {
                continue;
            }
            accepted.push(PendingNativeModulepreloadLinkEvent::new(client, successful));
        }
        accepted
    }

    pub(crate) fn enqueue_ready_native_modulepreload_link_event(
        &mut self,
        ready: ReadyConnectedStyleLoad,
    ) {
        debug_assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(_)
        ));
        self.push_ready_connected_style_load(ready);
    }

    fn connected_modulepreload_request(
        &self,
        handle: DomHandle,
        url: &url::Url,
    ) -> Option<(ModuleMapKey, NativeModuleSingleFetchRequest)> {
        let element = self.dom_host.node(handle).and_then(Node::as_element)?;
        if !self.connected_modulepreload_url_matches(handle, element, url) {
            return None;
        }
        let document_url = self
            .dom_host
            .node(self.dom_host.document_handle())
            .and_then(Node::as_document)
            .map(|document| document.url().clone())?;
        let candidate = modulepreload_fetch_candidate(
            element,
            url.clone(),
            &document_url,
            self.resolve_module_integrity(url),
        )?;
        Some((candidate.key, candidate.request))
    }

    fn connected_modulepreload_url_matches(
        &self,
        handle: DomHandle,
        element: &crate::dom::native::Element,
        url: &url::Url,
    ) -> bool {
        if !element.is_html_element("link")
            || !element
                .attribute("rel")
                .is_some_and(|rel| link_rel_includes_token(rel, "modulepreload"))
        {
            return false;
        }
        element
            .attribute("href")
            .map(str::trim)
            .is_some_and(|href| !href.is_empty())
            && modulepreload_as_state(element) != ModulepreloadAsState::Invalid
            && connected_preload_like_link_url(&self.dom_host, NodeId::new(handle.index()))
                .as_ref()
                .is_some_and(|candidate| candidate == url)
    }

    #[cfg(test)]
    pub(crate) fn enqueue_ready_connected_style_load_for_test(&mut self, handle: DomHandle) {
        let element_kind = if self.dom_host.is_inline_style_sheet_owner(handle) {
            ConnectedStyleEventElementKind::Style
        } else {
            assert!(
                self.dom_host.is_html_element_named(handle, "link"),
                "test stylesheet event owner must be a <style> or <link>"
            );
            ConnectedStyleEventElementKind::Link
        };
        self.stylesheet_lifecycle
            .injected_ready_connected_loads
            .push_back(ReadyConnectedStyleLoad::for_owner(
                handle,
                true,
                element_kind,
            ));
    }

    #[cfg(test)]
    pub(crate) fn active_stylesheet_link_client_for_test(
        &self,
        handle: DomHandle,
    ) -> Option<Arc<StylesheetLinkClient>> {
        self.stylesheet_lifecycle
            .owner_states
            .link_state(handle)
            .map(LinkStyleState::active_load)
            .map(Arc::clone)
    }

    #[cfg(test)]
    pub(in crate::document_runtime) fn accept_stylesheet_link_client_completion_for_test(
        &mut self,
        handle: DomHandle,
        load: &Arc<StylesheetLinkClient>,
        successful: bool,
    ) -> bool {
        self.stylesheet_lifecycle
            .owner_states
            .link_state_mut(handle)
            .is_some_and(|state| state.accept_resource_completion(load, successful).is_some())
    }

    #[cfg(test)]
    pub(crate) fn enqueue_pending_connected_style_load_for_test(&mut self, handle: DomHandle) {
        let load_event_binding =
            Some(MainDocumentStyleLoadEventBinding::unowned_for_document_runtime_test(handle));
        self.stylesheet_lifecycle
            .pending_connected_loads
            .push_back(QueuedConnectedStyleLoad::new(
                handle,
                None,
                load_event_binding.map(ConnectedStyleLoadEventAdmission::LoadDelaying),
            ));
    }
}

fn connected_modulepreload_invalid_as(
    dom_host: &crate::dom::native::DomHost,
    handle: DomHandle,
) -> Option<String> {
    let element = dom_host.node(handle).and_then(Node::as_element)?;
    if !element.is_html_element("link")
        || !element
            .attribute("rel")
            .is_some_and(|rel| link_rel_includes_token(rel, "modulepreload"))
        || connected_preload_like_link_url(dom_host, NodeId::new(handle.index())).is_none()
        || modulepreload_as_state(element) != ModulepreloadAsState::Invalid
    {
        return None;
    }
    element
        .attribute("as")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn connected_modulepreload_has_non_matching_media(
    dom_host: &crate::dom::native::DomHost,
    handle: DomHandle,
) -> bool {
    let Some(element) = dom_host.node(handle).and_then(Node::as_element) else {
        return false;
    };
    element.is_html_element("link")
        && element
            .attribute("rel")
            .is_some_and(|rel| link_rel_includes_token(rel, "modulepreload"))
        && connected_preload_like_link_url(dom_host, NodeId::new(handle.index())).is_some()
        && !modulepreload_media_matches(element)
}

fn connected_style_owner_kind(
    element: &crate::dom::native::Element,
) -> Option<ConnectedStyleOwnerKind> {
    if super::is_inline_style_element(element) {
        return if is_declarative_css_module_style_element(element) {
            Some(ConnectedStyleOwnerKind::DeclarativeCssModule)
        } else if crate::style_engine::stylesheet_owner_type_is_supported(element) {
            Some(ConnectedStyleOwnerKind::ClassicStyle)
        } else {
            None
        };
    }
    if !element.is_html_element("link") || element.attribute("disabled").is_some() {
        return None;
    }
    match (element.attribute("rel"), element.attribute("href")) {
        (Some(rel), Some(href))
            if !href.trim().is_empty()
                && (link_rel_includes_token(rel, "stylesheet")
                    || link_rel_includes_token(rel, "preload")
                    || link_rel_includes_token(rel, "modulepreload")
                    || link_rel_includes_token(rel, "prefetch")
                    || link_rel_includes_token(rel, "compression-dictionary")) =>
        {
            Some(ConnectedStyleOwnerKind::Link)
        }
        _ => None,
    }
}

fn preload_like_link_resource_type(
    element: &crate::dom::native::Element,
) -> SubresourceResourceType {
    let Some(rel) = element.attribute("rel") else {
        return SubresourceResourceType::Fetch;
    };
    if link_rel_includes_token(rel, "compression-dictionary") {
        return SubresourceResourceType::Dictionary;
    }
    if link_rel_is_standalone_prefetch(rel) {
        return SubresourceResourceType::Fetch;
    }
    let href = element.attribute("href").unwrap_or_default();
    if preload_like_link_loads_stylesheet(rel, element.attribute("as"), href) {
        return SubresourceResourceType::Stylesheet;
    }
    match link_as_destination(element.attribute("as")) {
        LinkAsDestination::Script
        | LinkAsDestination::AudioWorklet
        | LinkAsDestination::PaintWorklet
        | LinkAsDestination::ServiceWorker
        | LinkAsDestination::SharedWorker
        | LinkAsDestination::Worker => SubresourceResourceType::Script,
        LinkAsDestination::Image => SubresourceResourceType::Image,
        LinkAsDestination::Font => SubresourceResourceType::Font,
        LinkAsDestination::Audio => SubresourceResourceType::Audio,
        LinkAsDestination::Video => SubresourceResourceType::Video,
        LinkAsDestination::Track => SubresourceResourceType::TextTrack,
        LinkAsDestination::None => {
            let path = href.split(['?', '#']).next().unwrap_or_default();
            if path.ends_with(".js") || link_rel_includes_token(rel, "modulepreload") {
                SubresourceResourceType::Script
            } else {
                SubresourceResourceType::Fetch
            }
        }
        LinkAsDestination::Document
        | LinkAsDestination::Embed
        | LinkAsDestination::Fetch
        | LinkAsDestination::Frame
        | LinkAsDestination::IFrame
        | LinkAsDestination::Json
        | LinkAsDestination::Manifest
        | LinkAsDestination::Object
        | LinkAsDestination::Report
        | LinkAsDestination::Style
        | LinkAsDestination::Text
        | LinkAsDestination::WebIdentity
        | LinkAsDestination::Xslt => SubresourceResourceType::Fetch,
    }
}

fn preload_link_loads_stylesheet(element: &crate::dom::native::Element) -> bool {
    if !element.is_html_element("link") {
        return false;
    }
    let Some(rel) = element.attribute("rel") else {
        return false;
    };
    let Some(href) = element.attribute("href").map(str::trim) else {
        return false;
    };
    !href.is_empty() && preload_like_link_loads_stylesheet(rel, element.attribute("as"), href)
}

fn parser_image_source_present(element: &crate::dom::native::Element) -> bool {
    element.is_html_element("img")
        && (element
            .attribute("src")
            .map(str::trim)
            .is_some_and(|src| !src.is_empty())
            || element
                .attribute("srcset")
                .map(str::trim)
                .is_some_and(|srcset| !srcset.is_empty()))
}

fn link_rel_is_standalone_prefetch(rel: &str) -> bool {
    link_rel_includes_token(rel, "prefetch")
        && !link_rel_includes_token(rel, "preload")
        && !link_rel_includes_token(rel, "modulepreload")
}

fn preload_like_link_request_resource_type(
    element: &crate::dom::native::Element,
    resource_type: SubresourceResourceType,
    late_document_from_preload_scanner: bool,
) -> Option<RequestResourceType> {
    let rel = element.attribute("rel").unwrap_or_default();
    if link_rel_includes_token(rel, "compression-dictionary") {
        // Chromium fetches `<link rel=compression-dictionary>` as loader
        // ResourceType::kDictionary with VeryLow priority. DevTools does not
        // expose a dedicated Dictionary token, so Moli keeps the page/CDP
        // resource type separate (`SubresourceResourceType::Dictionary` reports
        // as "Other") while preserving the scheduler input here.
        return Some(RequestResourceType::Dictionary);
    }
    if link_rel_is_standalone_prefetch(rel) {
        // Chromium exposes link prefetch as DevTools/Fetch resource type
        // "Fetch", but its loader resource type is kLinkPrefetch, which carries
        // VeryLow priority. Keep those two observable boundaries separate here:
        // `resource_type` is what CDP/page records see, and
        // `request_resource_type` is the fetch scheduler's priority input.
        return Some(RequestResourceType::LinkPrefetch);
    }
    if link_rel_includes_token(rel, "preload")
        && element
            .attribute("as")
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("fetch"))
    {
        // Blink maps `<link rel=preload as=fetch>` to ResourceType::kRaw even
        // though DevTools still reports the request as protocol type "Fetch".
        // Keep those boundaries split: `resource_type` drives page/CDP records
        // and preload bookkeeping, while `request_resource_type` drives the
        // transport scheduler. If this falls through to Moli's default
        // Script request kind, future Raw-specific priority/flag handling would
        // be hidden behind a misleading script classification.
        return Some(RequestResourceType::Raw);
    }
    if late_document_from_preload_scanner && resource_type == SubresourceResourceType::Stylesheet {
        // Chromium lowers CSS stylesheets discovered late by the in-document
        // preload scanner to Medium. This applies only to the initial document
        // scan signal recorded by DocumentRuntime; dynamically inserted links
        // and true parser-blocking stylesheet fetches keep their normal
        // stylesheet priority.
        return Some(RequestResourceType::LatePreloadCssStyleSheet);
    }
    crate::network::request_resource_type_for_subresource(resource_type)
}

fn preload_like_link_readiness_fetch_options(
    element: &crate::dom::native::Element,
    late_document_from_preload_scanner: bool,
) -> ConnectedLinkReadinessFetchOptions {
    let resource_type = preload_like_link_resource_type(element);
    let rel = element.attribute("rel").unwrap_or_default();
    let cross_origin = element.attribute("crossorigin");
    let request_resource_type = preload_like_link_request_resource_type(
        element,
        resource_type,
        late_document_from_preload_scanner,
    );
    let fetch_priority_hint = FetchPriorityHint::from_attribute(element.attribute("fetchpriority"));
    let link_fetch_options = StylesheetFetchOptions::from_link_attributes(
        element.attribute("crossorigin"),
        element.attribute("referrerpolicy"),
        element.attribute("integrity"),
        element
            .cryptographic_nonce()
            .or_else(|| element.attribute("nonce")),
        element.attribute("charset"),
        element.attribute("fetchpriority"),
    );
    let script_fetch_metadata = (resource_type == SubresourceResourceType::Script).then(|| {
        ScriptFetchMetadata::from_script_attributes(
            element.attribute("crossorigin"),
            element.attribute("referrerpolicy"),
            None,
            element.attribute("integrity"),
            element
                .cryptographic_nonce()
                .or_else(|| element.attribute("nonce")),
            element.attribute("fetchpriority"),
        )
    });
    let modulepreload = link_rel_includes_token(rel, "modulepreload");
    let request_mode = if resource_type == SubresourceResourceType::Fetch
        && link_rel_includes_token(rel, "preload")
        && element
            .attribute("as")
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("fetch"))
        && cross_origin.is_none()
    {
        moli_fetch::RequestMode::NoCors
    } else {
        moli_fetch::RequestMode::Cors
    };
    let credentials_mode = if resource_type == SubresourceResourceType::Dictionary {
        // Blink sets compression dictionary requests to CORS mode with
        // credentials omitted and no referrer. Request::new already uses CORS
        // for subresources; Moli does not yet model per-request referrer
        // policy for this loader path, so credentials and priority are the
        // observable pieces preserved here.
        RequestCredentialsMode::Omit
    } else if cross_origin.is_some() {
        link_crossorigin_credentials_mode(cross_origin)
    } else if modulepreload {
        module_script_credentials_mode(
            script_fetch_metadata
                .as_ref()
                .and_then(|metadata| metadata.cross_origin.as_deref()),
        )
    } else {
        RequestCredentialsMode::Include
    };
    ConnectedLinkReadinessFetchOptions {
        resource_type,
        request_resource_type,
        script_fetch_metadata,
        request_mode,
        credentials_mode,
        fetch_priority_hint,
        link_preload: link_rel_includes_token(rel, "preload"),
        link_fetch_options,
    }
}

#[cfg(test)]
async fn fetch_connected_link_readiness(
    loader: ResourceRequestClient,
    document_url: Url,
    url: Url,
    options: ConnectedLinkReadinessFetchOptions,
) -> Result<crate::protocol_types::NavigationResponse, String> {
    let request = connected_link_readiness_request(&document_url, &url, &options);
    fetch_connected_link_readiness_with_request(loader, url, options, request).await
}

async fn fetch_connected_link_readiness_with_service_worker(
    loader: ResourceRequestClient,
    resource_task_runner: crate::network::RendererResourceTaskRunner,
    document_url: Url,
    url: Url,
    options: ConnectedLinkReadinessFetchOptions,
    service_worker_context: Option<ServiceWorkerConnectedLinkContext>,
) -> Result<ConnectedLinkReadinessFetchResponse, String> {
    let request = connected_link_readiness_request(&document_url, &url, &options);
    if let (Some(context), Some(destination)) = (
        service_worker_context,
        ServiceWorkerRequestDestination::for_subresource_resource_type(options.resource_type),
    ) {
        match context
            .browser_context_runtime
            .fetch_service_worker_subresource_for_client_with_metadata(
                context.client_id,
                document_url.clone(),
                &request,
                &loader,
                resource_task_runner,
                destination,
                options.resource_type,
            )
            .await
        {
            Ok(Some(response)) => {
                let response_filter = response.response_filter;
                let origin_clean =
                    connected_link_origin_clean_from_service_worker_filter(response_filter);
                let response = *response.response;
                let load_event_successful =
                    connected_link_load_event_successful(&response, response_filter);
                return Ok(ConnectedLinkReadinessFetchResponse::new(
                    response,
                    origin_clean,
                    load_event_successful,
                ));
            }
            Ok(None) => {}
            Err(error) => {
                return Err(format!(
                    "failed to fetch preload-like link `{url}` through service worker: {error}"
                ));
            }
        }
    }
    fetch_connected_link_readiness_with_request(loader, url, options, request)
        .await
        .map(|response| {
            let load_event_successful = connected_link_load_event_successful(&response, None);
            let origin_clean = moli_url::same_origin(&document_url, &response.final_url);
            ConnectedLinkReadinessFetchResponse::new(response, origin_clean, load_event_successful)
        })
}

fn connected_link_readiness_request(
    document_url: &Url,
    url: &Url,
    options: &ConnectedLinkReadinessFetchOptions,
) -> moli_fetch::Request {
    debug_assert_ne!(
        options.resource_type,
        SubresourceResourceType::Stylesheet,
        "stylesheet preloads must use the typed stylesheet resource owner"
    );
    let request = moli_fetch::Request::new("GET", url.as_str(), None, vec![])
        .expect("preload-like link url should already be parsed")
        .with_page_network_policy()
        .with_initiator_url(document_url);
    let mut request = request
        .with_request_mode(options.request_mode)
        .with_credentials_mode(options.credentials_mode);
    if options.link_preload {
        request = request.with_link_preload();
    }
    if options.fetch_priority_hint.is_some() {
        request = request.with_fetch_priority_hint(options.fetch_priority_hint);
    }
    if let Some(resource_type) = options.request_resource_type {
        request = request.with_resource_type(resource_type);
    }
    if let Some(metadata) = options.script_fetch_metadata.as_ref() {
        request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            cross_origin: metadata.cross_origin.clone(),
            referrer_policy: metadata.referrer_policy.clone(),
            document_referrer_policy: None,
            charset: metadata.charset.clone(),
            integrity: metadata.integrity.clone(),
            nonce: metadata.nonce.clone(),
            fetch_priority: metadata.fetch_priority,
            scheduler_priority: None,
        });
    } else if !options.link_fetch_options.is_empty() {
        request =
            request.with_subresource_request_metadata(moli_fetch::SubresourceRequestMetadata {
                referrer_policy: options
                    .link_fetch_options
                    .referrer_policy()
                    .map(str::to_owned),
                document_referrer_policy: None,
                integrity: options.link_fetch_options.integrity().map(str::to_owned),
            });
    }
    request
}

fn connected_link_origin_clean_from_service_worker_filter(
    response_filter: Option<AsyncSubresourceFetchResponseFilter>,
) -> bool {
    match response_filter {
        Some(AsyncSubresourceFetchResponseFilter::Opaque)
        | Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect) => false,
        None => true,
    }
}

fn link_crossorigin_credentials_mode(cross_origin: Option<&str>) -> RequestCredentialsMode {
    if cross_origin
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials"))
    {
        RequestCredentialsMode::Include
    } else {
        RequestCredentialsMode::SameOrigin
    }
}

fn connected_link_load_event_successful(
    response: &crate::protocol_types::NavigationResponse,
    response_filter: Option<AsyncSubresourceFetchResponseFilter>,
) -> bool {
    match response_filter {
        Some(AsyncSubresourceFetchResponseFilter::Opaque)
        | Some(AsyncSubresourceFetchResponseFilter::OpaqueRedirect) => true,
        None => (200..=299).contains(&response.status),
    }
}

async fn fetch_connected_link_readiness_with_request(
    loader: ResourceRequestClient,
    url: Url,
    options: ConnectedLinkReadinessFetchOptions,
    request: moli_fetch::Request,
) -> Result<crate::protocol_types::NavigationResponse, String> {
    let response = if options.resource_type == SubresourceResourceType::Script {
        loader.fetch_cacheable_script_text_stream(request).await
    } else {
        loader.fetch_text_stream(request).await
    };
    response
        .map(crate::protocol_types::NavigationResponse::from)
        .map_err(|error| format!("failed to fetch preload-like link `{url}`: {error}"))
}

#[cfg(test)]
mod tests {
    use super::super::import_graph::{
        DataStylesheetImportReadiness, data_stylesheet_import_readiness,
    };
    use super::*;
    use crate::module_runtime::{
        ModuleAttributesKey, ModuleLoadError, ModuleLoadStage, ModuleMapFetchDisposition,
        ModuleMapKey, ModuleSource,
    };
    use crate::network::ResourceRequestClient;
    use crate::{dom::native::Node, parser::HtmlParser};
    use anyhow::Result;
    use moli_fetch::FetchConfig;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };
    use url::Url;

    fn first_style_handle(document: &crate::dom::native::NativeDom) -> DomHandle {
        let head = document.document_head_handle().expect("head handle");
        document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("style"))
            })
            .expect("style handle")
    }

    fn first_link_element(
        document: &crate::dom::native::NativeDom,
    ) -> &crate::dom::native::Element {
        let link = first_link_handle(document);
        document
            .node(link)
            .and_then(Node::as_element)
            .expect("link element")
    }

    fn first_link_handle(document: &crate::dom::native::NativeDom) -> DomHandle {
        let head = document.document_head_handle().expect("head handle");
        document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle")
    }

    fn parse_document_with_blocking_stylesheet_inputs(
        final_url: Url,
        html: impl Into<String>,
    ) -> (
        crate::dom::native::NativeDom,
        Vec<crate::DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) {
        let (document, _, inputs) =
            crate::parse_html_test_fixture_with_parser_outputs(final_url, html.into());
        (document, inputs)
    }

    #[test]
    fn blocked_csp_disposition_does_not_hide_initial_style_error_task() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>body { color: red }</style></head></html>"
                .to_owned(),
        );
        let style = first_style_handle(&document);
        let mut runtime = DocumentRuntime::new(&document);

        runtime
            .stylesheet_lifecycle
            .owner_states
            .set_csp_disposition(style, StylesheetOwnerCspDisposition::Blocked);

        assert!(runtime.stylesheet_owner_is_csp_blocked(style));
        assert!(
            !runtime.connected_style_load_is_queued_for_test(style),
            "policy state alone must not impersonate queued lifecycle work"
        );

        runtime.queue_initial_connected_style_loads();

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("blocked parser style should queue an error event task");
        assert_eq!(ready.owner(), style);
        assert!(!ready.successful());
    }

    fn link_handle_by_href(document: &crate::dom::native::NativeDom, href: &str) -> DomHandle {
        let mut stack = vec![document.document_node_id()];
        while let Some(handle) = stack.pop() {
            if document
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| {
                    element.is_html_element("link") && element.attribute("href") == Some(href)
                })
            {
                return handle;
            }
            let mut children = document.child_nodes(handle).unwrap_or_default();
            children.reverse();
            stack.extend(children);
        }
        panic!("link with href `{href}` should exist");
    }

    async fn prime_pending_data_stylesheet_event(
        runtime: &mut DocumentRuntime,
        owner: DomHandle,
    ) -> ReadyConnectedStyleLoad {
        let (starts, warnings, completed_clients) = runtime
            .prime_pending_connected_style_loads()
            .into_parts_with_completed_stylesheet_clients();
        assert!(starts.is_empty(), "stylesheet must not start module work");
        assert!(warnings.is_empty(), "stylesheet warnings: {warnings:?}");
        assert!(
            completed_clients
                .iter()
                .all(|client| client.load().owner() == owner),
            "a completed admission must belong to the exact requested owner"
        );

        runtime.drain_ready_connected_style_load_completions();
        if let Some(ready) = runtime.pop_ready_connected_style_load() {
            assert_eq!(ready.owner(), owner);
            return ready;
        }

        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "data stylesheet typed Networking source should remain open"
        );
        runtime.drain_ready_connected_style_load_completions();
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("data stylesheet completion should publish one event task");
        assert_eq!(ready.owner(), owner);
        ready
    }

    const MODULEPRELOAD_AS_MATRIX_GOOD_VALUES: &[&str] = &[
        "",
        "invalid-dest",
        "sCrIpT",
        "style",
        "json",
        "text",
        "audioworklet",
        "paintworklet",
        "script",
        "serviceworker",
        "sharedworker",
        "worker",
    ];

    const MODULEPRELOAD_AS_MATRIX_BAD_VALUES: &[&str] = &[
        "audio",
        "document",
        "embed",
        "font",
        "frame",
        "iframe",
        "image",
        "manifest",
        "object",
        "report",
        "track",
        "video",
        "webidentity",
        "xslt",
        "fetch",
        "iMaGe",
    ];

    fn modulepreload_as_matrix_markup() -> String {
        let mut markup = "<!doctype html><html><head>".to_owned();
        for value in MODULEPRELOAD_AS_MATRIX_GOOD_VALUES
            .iter()
            .chain(MODULEPRELOAD_AS_MATRIX_BAD_VALUES)
            .enumerate()
        {
            let (index, value) = value;
            let slug = if value.is_empty() {
                format!("{index}-empty")
            } else {
                format!("{index}-{}", value.to_ascii_lowercase())
            };
            let href = match *value {
                "style" => format!("/{slug}.css"),
                "json" => format!("/{slug}.json"),
                "text" => format!("/{slug}.txt"),
                _ => format!("/{slug}.js"),
            };
            markup.push_str(&format!(
                "<link rel=\"modulepreload\" href=\"{href}\" as=\"{value}\">"
            ));
        }
        markup.push_str("</head><body></body></html>");
        markup
    }

    fn register_modulepreload_link_client(
        runtime: &mut DocumentRuntime,
        handle: DomHandle,
        key: &ModuleMapKey,
    ) -> Arc<NativeModulepreloadLinkClient> {
        assert!(matches!(
            runtime.start_or_join_native_module_fetch(key.clone()),
            ModuleMapFetchDisposition::StartedFetch(_)
        ));
        let client = NativeModulepreloadLinkClient::new(handle, key.clone());
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_native_modulepreload(Arc::clone(&client));
        runtime.suspend_native_modulepreload_link_clients(key.clone(), vec![Arc::clone(&client)]);
        client
    }

    fn dispatch_modulepreload_terminal_clients(runtime: &mut DocumentRuntime, successful: bool) {
        let notification = match runtime
            .take_next_native_module_owner_event()
            .expect("module map terminal transition should post an owner event")
        {
            crate::module_runtime::NativeModuleOwnerEvent::ModuleMapTerminalNotification(
                notification,
            ) => notification,
            crate::module_runtime::NativeModuleOwnerEvent::ModulepreloadLinkError(_) => {
                panic!("expected modulepreload terminal notification owner event")
            }
        };
        let (key, clients, notification_successful) = notification.into_parts();
        assert_eq!(notification_successful, successful);
        let (_, _, link_clients) = clients.into_parts();
        debug_assert!(
            link_clients
                .iter()
                .all(|client| client.frame_document_client().is_none()),
            "main document modulator received a child modulepreload client"
        );
        complete_modulepreload_link_clients(runtime, &key, link_clients, successful);
    }

    fn complete_modulepreload_link_clients(
        runtime: &mut DocumentRuntime,
        key: &ModuleMapKey,
        clients: Vec<Arc<NativeModulepreloadLinkClient>>,
        successful: bool,
    ) {
        let terminals =
            runtime.accept_native_modulepreload_link_client_terminals(key, clients, successful);
        for pending_event in terminals {
            runtime.enqueue_ready_native_modulepreload_link_event(pending_event.into_ready_event());
        }
    }

    #[test]
    fn pending_modulepreload_event_becomes_ready_without_a_load_event_binding() {
        let link = DomHandle::new(42);
        let key = ModuleMapKey::java_script(
            Url::parse("https://example.test/immutable-client.mjs").unwrap(),
        );
        let client = NativeModulepreloadLinkClient::new(link, key);
        let pending_event = PendingNativeModulepreloadLinkEvent::new(Arc::clone(&client), true);
        let ready = pending_event.into_ready_event();

        assert_eq!(ready.load_event_binding(), None);
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &client)
        ));
    }

    #[test]
    fn native_module_source_completion_marks_modulepreload_script_link_ready() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let url = Url::parse("https://example.test/app.mjs").unwrap();
        let key = ModuleMapKey::java_script(url);
        let mut runtime = DocumentRuntime::new(&document);

        let client = register_modulepreload_link_client(&mut runtime, link, &key);
        runtime
            .insert_native_module_source(key.clone(), ModuleSource::text("export {};".to_owned()));
        dispatch_modulepreload_terminal_clients(&mut runtime, true);

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("modulepreload load event task");
        assert_eq!(ready.owner(), link);
        assert!(ready.successful());
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &client)
        ));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
    }

    #[test]
    fn native_module_failure_completion_marks_modulepreload_script_link_ready_for_error() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let url = Url::parse("https://example.test/app.mjs").unwrap();
        let key = ModuleMapKey::java_script(url);
        let mut runtime = DocumentRuntime::new(&document);

        let client = register_modulepreload_link_client(&mut runtime, link, &key);
        runtime.mark_native_module_failed(
            key.clone(),
            ModuleLoadError::new(ModuleLoadStage::Fetch, "network error"),
        );
        dispatch_modulepreload_terminal_clients(&mut runtime, false);

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("modulepreload error event task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &client)
        ));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
    }

    #[test]
    fn native_module_completion_marks_css_modulepreload_link_ready_when_as_style() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/style.css\" as=\"style\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let url = Url::parse("https://example.test/style.css").unwrap();
        let key = ModuleMapKey::css_with_attributes(
            url,
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "css".to_owned())]),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let client = register_modulepreload_link_client(&mut runtime, link, &key);
        runtime
            .insert_native_module_source(key.clone(), ModuleSource::text("export {};".to_owned()));
        dispatch_modulepreload_terminal_clients(&mut runtime, true);

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("CSS modulepreload load event task");
        assert_eq!(ready.owner(), link);
        assert!(ready.successful());
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &client)
        ));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
    }

    #[test]
    fn implicit_css_suffix_modulepreload_keeps_its_captured_javascript_key() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/style.css\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let url = Url::parse("https://example.test/style.css").unwrap();
        let key = ModuleMapKey::java_script(url);
        let mut runtime = DocumentRuntime::new(&document);

        let client = register_modulepreload_link_client(&mut runtime, link, &key);
        runtime.insert_native_module_source(
            key.clone(),
            ModuleSource::text("export default 'not inferred from .css';".to_owned()),
        );
        dispatch_modulepreload_terminal_clients(&mut runtime, true);

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("captured JavaScript modulepreload event task");
        assert!(ready.successful());
        assert_eq!(
            client.key().kind(),
            crate::module_runtime::ModuleKind::JavaScript
        );
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &client)
        ));
    }

    #[test]
    fn stale_same_key_native_modulepreload_client_cannot_complete_new_processing() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=modulepreload href=/same.mjs></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let key = ModuleMapKey::java_script(Url::parse("https://example.test/same.mjs").unwrap());
        let first = NativeModulepreloadLinkClient::new(link, key.clone());
        let current = NativeModulepreloadLinkClient::new(link, key.clone());
        let mut runtime = DocumentRuntime::new(&document);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_native_modulepreload(Arc::clone(&first));
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_native_modulepreload(Arc::clone(&current));

        complete_modulepreload_link_clients(&mut runtime, &key, vec![Arc::clone(&first)], true);
        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "an old same-key client must not acquire the current processing's event authority"
        );

        complete_modulepreload_link_clients(&mut runtime, &key, vec![Arc::clone(&current)], true);
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("current same-key client event task");
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &current)
        ));
    }

    #[test]
    fn posted_native_modulepreload_event_keeps_accepted_client_after_reprocess() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=modulepreload href=/same.mjs></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let key = ModuleMapKey::java_script(Url::parse("https://example.test/same.mjs").unwrap());
        let accepted = NativeModulepreloadLinkClient::new(link, key.clone());
        let replacement = NativeModulepreloadLinkClient::new(link, key.clone());
        let mut runtime = DocumentRuntime::new(&document);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_native_modulepreload(Arc::clone(&accepted));
        complete_modulepreload_link_clients(&mut runtime, &key, vec![Arc::clone(&accepted)], true);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_native_modulepreload(replacement);

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("already-posted event must survive owner reprocessing");
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::NativeModulepreload(ready_client)
                if NativeModulepreloadLinkClient::ptr_eq(ready_client, &accepted)
        ));
    }

    #[test]
    fn connected_modulepreload_script_link_returns_owner_start() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\"></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        let prime_result = runtime.prime_pending_connected_style_loads();

        let (starts, warnings) = prime_result.into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(
            starts.len(),
            1,
            "new connected JS modulepreload should return one owner start action"
        );
        let (preload, link_client) = starts
            .into_iter()
            .next()
            .expect("connected JS modulepreload start should be present")
            .into_parts();
        assert_eq!(
            preload.source_url().as_str(),
            "https://example.test/app.mjs"
        );
        let outcome = runtime
            .fetch_single_native_module_for_modulepreload_link(preload, link_client)
            .expect("modulepreload owner registration should succeed");
        let (fetch_start, terminal) = outcome.into_parts();
        assert!(matches!(
            fetch_start,
            crate::module_runtime::NativeModulepreloadFetchStart::Started(_)
        ));
        assert!(terminal.is_none());
        assert_eq!(
            runtime
                .script_lifecycle
                .scripts_mut()
                .modulepreload_link_client_count_for_testing(),
            1,
            "JS modulepreload link client should be owned by the module map entry"
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation(),
            "the owner state and module map must share the exact pending modulepreload client"
        );
        assert!(
            !runtime.has_pending_style_loads(),
            "an in-flight modulepreload fetch must not delay the document load event"
        );
        assert!(runtime.pop_ready_connected_style_load().is_none());
        Ok(())
    }

    #[test]
    fn script_disabled_document_does_not_start_connected_script_preloads() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel='preload' as='script' href='/classic.js'>",
                "<link rel='modulepreload' href='/module.mjs'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        runtime.set_script_execution_disabled(true);

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();

        assert!(starts.is_empty());
        assert!(warnings.is_empty());
        assert!(
            !runtime.has_pending_style_loads(),
            "inert script preloads must not retain load-event or module-map work"
        );
        assert!(runtime.pop_ready_connected_style_load().is_none());
        Ok(())
    }

    #[test]
    fn connected_modulepreload_supported_as_values_return_owner_starts() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"modulepreload\" href=\"/worker.mjs\" as=\"worker\">",
                "<link rel=\"modulepreload\" href=\"/audio-worklet.mjs\" as=\"audioworklet\">",
                "<link rel=\"modulepreload\" href=\"/normalized.mjs\" as=\"invalid-dest\">",
                "<link rel=\"modulepreload\" href=\"/data.json\" as=\"json\">",
                "<link rel=\"modulepreload\" href=\"/theme.css\" as=\"style\">",
                "<link rel=\"modulepreload\" href=\"/data.txt\" as=\"text\">",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        let keys = starts
            .into_iter()
            .map(|start| {
                let (request, _link_client) = start.into_parts();
                request.module_key().clone()
            })
            .collect::<Vec<_>>();

        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/worker.mjs").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/audio-worklet.mjs").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/normalized.mjs").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::json_with_attributes(
            Url::parse("https://example.test/data.json").unwrap(),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "json".to_owned())]),
        )));
        assert!(keys.contains(&ModuleMapKey::css_with_attributes(
            Url::parse("https://example.test/theme.css").unwrap(),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "css".to_owned())]),
        )));
        assert!(keys.contains(&ModuleMapKey::modulepreload_text(
            Url::parse("https://example.test/data.txt").unwrap(),
        )));
        Ok(())
    }

    #[test]
    fn connected_modulepreload_as_values_match_wpt_matrix() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            modulepreload_as_matrix_markup(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        let keys = starts
            .into_iter()
            .map(|start| {
                let (request, _link_client) = start.into_parts();
                request.module_key().clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), MODULEPRELOAD_AS_MATRIX_GOOD_VALUES.len());
        assert_eq!(warnings.len(), MODULEPRELOAD_AS_MATRIX_BAD_VALUES.len());
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/1-invalid-dest.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/8-script.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.test/11-worker.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::json_with_attributes(
            Url::parse("https://example.test/4-json.json").unwrap(),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "json".to_owned())]),
        )));
        assert!(keys.contains(&ModuleMapKey::css_with_attributes(
            Url::parse("https://example.test/3-style.css").unwrap(),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "css".to_owned())]),
        )));
        assert!(keys.contains(&ModuleMapKey::modulepreload_text(
            Url::parse("https://example.test/5-text.txt").unwrap(),
        )));
        let mut owner_link_errors = 0;
        while let Some(ready) = runtime.pop_ready_connected_style_load() {
            assert!(
                !ready.successful(),
                "invalid modulepreload destinations must queue link error tasks"
            );
            assert!(matches!(
                ready.operation(),
                ReadyConnectedStyleLoadOperation::Connected(operation)
                    if operation.parameters == ConnectedLoadParameters::ImmediateOwnerProcessing
            ));
            owner_link_errors += 1;
        }
        assert_eq!(owner_link_errors, MODULEPRELOAD_AS_MATRIX_BAD_VALUES.len());
        assert!(
            runtime.take_next_native_module_owner_event().is_none(),
            "invalid-as link errors must not re-enter the legacy module owner-event queue"
        );
        Ok(())
    }

    #[test]
    fn connected_modulepreload_ignores_fetchpriority_hint() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"modulepreload\" href=\"/high.mjs\" fetchpriority=\"high\">",
                "<link rel=\"modulepreload\" href=\"/low.mjs\" fetchpriority=\"low\">",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");

        assert_eq!(starts.len(), 2);
        for start in starts {
            let (request, _link_client) = start.into_parts();
            assert_eq!(
                request.fetch_metadata().fetch_priority_for_test(),
                None,
                "Chromium modulepreload uses FetchPriorityHint::kAuto rather than the link fetchpriority attribute"
            );
        }
        Ok(())
    }

    #[test]
    fn connected_modulepreload_skips_non_matching_media() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"modulepreload\" href=\"/screen.mjs\" media=\"screen\">",
                "<link rel=\"modulepreload\" href=\"/print.mjs\" media=\"print\">",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let _screen = link_handle_by_href(&document, "/screen.mjs");
        let _print = link_handle_by_href(&document, "/print.mjs");

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        let keys = starts
            .into_iter()
            .map(|start| {
                let (request, _link_client) = start.into_parts();
                request.module_key().clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            keys,
            vec![ModuleMapKey::java_script(
                Url::parse("https://example.test/screen.mjs").unwrap()
            )]
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation(),
            "the matching modulepreload keeps its exact owner client; the non-matching link contributes no operation"
        );
        assert!(runtime.pop_ready_connected_style_load().is_none());
        Ok(())
    }

    #[test]
    fn connected_modulepreload_invalid_as_records_warning_and_error_event() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/bad.bin\" as=\"image\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();

        assert!(
            starts.is_empty(),
            "invalid modulepreload `as` must not start an owner fetch"
        );
        assert_eq!(
            warnings,
            vec!["<link rel=modulepreload> has an invalid `as` value image".to_owned()]
        );
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("connected invalid modulepreload should queue an exact link error task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());
        assert!(
            runtime.take_next_native_module_owner_event().is_none(),
            "invalid-as link errors must not detour through raw module owner events"
        );
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connected_modulepreload_start_survives_installed_loader_fast_path() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\"></head><body></body></html>"
                .to_owned(),
        );
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        runtime.queue_initial_connected_style_loads();
        let prime_result = runtime.prime_pending_connected_style_loads();

        let (starts, warnings) = prime_result.into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(
            starts.len(),
            1,
            "installed-loader connected load fast path must not drop modulepreload owner starts"
        );
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn connected_modulepreload_invalid_as_warning_survives_installed_loader_fast_path()
    -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/bad.bin\" as=\"image\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        runtime.queue_initial_connected_style_loads();
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();

        assert!(
            starts.is_empty(),
            "invalid modulepreload `as` must not start an owner fetch"
        );
        assert_eq!(
            warnings,
            vec!["<link rel=modulepreload> has an invalid `as` value image".to_owned()]
        );
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("connected invalid modulepreload should queue an exact link error task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());
        Ok(())
    }

    #[test]
    fn connected_modulepreload_script_link_reuses_fetched_module_map_entry() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let url = Url::parse("https://example.test/app.mjs").unwrap();
        let key = ModuleMapKey::java_script(url);
        let mut runtime = DocumentRuntime::new(&document);

        runtime.insert_native_module_source(key, ModuleSource::text("export {};".to_owned()));
        runtime.queue_initial_connected_style_loads();
        let prime_result = runtime.prime_pending_connected_style_loads();
        let (starts, warnings) = prime_result.into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(starts.len(), 1);
        let (preload, link_client) = starts
            .into_iter()
            .next()
            .expect("connected JS modulepreload start should be present")
            .into_parts();
        let outcome = runtime
            .fetch_single_native_module_for_modulepreload_link(preload, link_client)
            .expect("modulepreload owner registration should succeed");
        let (fetch_start, terminal) = outcome.into_parts();
        assert_eq!(
            fetch_start,
            crate::module_runtime::NativeModulepreloadFetchStart::AlreadyComplete
        );
        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "an accepted cached terminal must wait for ScriptVm lifecycle binding before enqueue"
        );
        let pending_event =
            terminal.expect("cached modulepreload should return one pending link event");
        runtime.enqueue_ready_native_modulepreload_link_event(pending_event.into_ready_event());

        assert!(
            !runtime.has_inflight_native_modulepreload_fetch(),
            "fetched module map entries should satisfy connected modulepreload links directly"
        );
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("cached modulepreload load event task");
        assert_eq!(ready.owner(), link);
        assert!(ready.successful());
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        Ok(())
    }

    #[test]
    fn processed_inline_source_ignores_imports_after_style_rules() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>.ready { color: green; } @import url('/late.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let style = first_style_handle(&document);
        let runtime = DocumentRuntime::new(&document);

        assert!(
            runtime
                .inline_style_source_for_test(style)
                .expect("processed inline source")
                .import_urls()
                .is_empty()
        );
    }

    #[test]
    fn processed_inline_source_allows_charset_before_imports() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>@charset \"utf-8\"; @import url('/early.css'); .ready { color: green; }</style></head><body></body></html>"
                .to_owned(),
        );
        let style = first_style_handle(&document);
        let runtime = DocumentRuntime::new(&document);

        assert_eq!(
            runtime
                .inline_style_source_for_test(style)
                .expect("processed inline source")
                .import_urls(),
            &[Url::parse("https://example.test/early.css").unwrap()]
        );
    }

    #[test]
    fn processed_inline_source_deduplicates_imports_in_document_order() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head><style>",
                "@import url('/shared.css');",
                "@import url('/other.css');",
                "@import url('/shared.css');",
                ".ready { color: green; }",
                "</style></head><body></body></html>",
            )
            .to_owned(),
        );
        let style = first_style_handle(&document);
        let runtime = DocumentRuntime::new(&document);

        assert_eq!(
            runtime
                .inline_style_source_for_test(style)
                .expect("processed inline source")
                .import_urls(),
            &[
                Url::parse("https://example.test/shared.css").unwrap(),
                Url::parse("https://example.test/other.css").unwrap(),
            ]
        );
    }

    #[test]
    fn blocking_stylesheet_promotion_leaves_independent_style_import_operation_pending() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>@import url('/runtime.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let style = first_style_handle(&document);
        let mut runtime = DocumentRuntime::new(&document);
        let source = runtime
            .inline_style_source_for_test(style)
            .expect("processed inline source");
        let operation = ConnectedLoadOperation::new_for_test(
            style,
            ConnectedStyleEventElementKind::Style,
            ConnectedLoadParameters::StyleImports {
                source: InlineStyleImportSource::new(source),
                urls: vec![Url::parse("https://example.test/runtime.css").unwrap()],
                roots: Vec::new(),
            },
            None,
        );
        runtime
            .stylesheet_lifecycle
            .owner_states
            .install_pending_operation(Arc::clone(&operation));

        runtime.reconcile_connected_style_imports_with_blocking_stylesheets();

        assert!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .is_empty()
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .pending_operation(style)
                .is_some_and(|pending| ConnectedLoadOperation::ptr_eq(pending, &operation))
        );
    }

    #[test]
    fn data_stylesheet_relative_import_uses_stylesheet_url_base() {
        let stylesheet_url = Url::parse("data:/,@import url('x/');").unwrap();

        assert_eq!(
            data_stylesheet_import_readiness(&stylesheet_url),
            DataStylesheetImportReadiness::Imports(vec![
                Url::parse("data:/,@import%20url('x/x/").unwrap()
            ])
        );
    }

    #[tokio::test]
    async fn data_stylesheet_link_with_failed_import_queues_error_event() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"stylesheet\" href=\"data:/,@import url('x/');\">",
                "</head><body></body></html>"
            )
            .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_pending_connected_style_loads();
        let load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("active linked stylesheet processing object");
        runtime.drain_ready_connected_style_load_completions();
        if !runtime.has_pending_ready_connected_style_loads() {
            assert!(
                runtime.wait_for_stylesheet_networking_task_for_test().await,
                "data stylesheet import must publish a typed Networking completion"
            );
            runtime.drain_ready_connected_style_load_completions();
        }

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("failed linked stylesheet event task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());
        assert!(matches!(
            ready.operation(),
            ReadyConnectedStyleLoadOperation::StylesheetLink(ready_load)
                if StylesheetLinkClient::ptr_eq(ready_load, &load)
        ));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        Ok(())
    }

    #[test]
    fn connected_style_data_import_without_nested_import_queues_load_event() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<style>@import url('data:text/css,');</style>",
                "</head><body></body></html>"
            )
            .to_owned(),
        );
        let style = first_style_handle(&document);
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_pending_connected_style_loads();

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("inline style import load event task");
        assert_eq!(ready.owner(), style);
        assert!(ready.successful());
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        Ok(())
    }

    #[test]
    fn connected_style_data_import_failure_queues_error_event() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<style>@import url('data:/,@import url(\"x/\");');</style>",
                "</head><body></body></html>"
            )
            .to_owned(),
        );
        let style = first_style_handle(&document);
        let mut runtime = DocumentRuntime::new(&document);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_pending_connected_style_loads();

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("inline style import error event task");
        assert_eq!(ready.owner(), style);
        assert!(!ready.successful());
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        Ok(())
    }

    #[test]
    fn modulepreload_link_readiness_uses_module_script_credentials() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\" crossorigin=\"use-credentials\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Script);
        assert_eq!(options.credentials_mode, RequestCredentialsMode::Include);
        assert_eq!(
            options
                .script_fetch_metadata
                .as_ref()
                .and_then(|metadata| metadata.cross_origin.as_deref()),
            Some("use-credentials")
        );
    }

    #[test]
    fn anonymous_modulepreload_link_readiness_uses_same_origin_credentials() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"modulepreload\" href=\"/app.mjs\" crossorigin=\"anonymous\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Script);
        assert_eq!(options.credentials_mode, RequestCredentialsMode::SameOrigin);
        assert_eq!(
            options
                .script_fetch_metadata
                .as_ref()
                .and_then(|metadata| metadata.cross_origin.as_deref()),
            Some("anonymous")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn typed_stylesheet_preload_origin_clean_uses_final_url() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let request_size = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..request_size]);
            assert!(
                request.starts_with("GET /cross.css "),
                "unexpected stylesheet request: {request}"
            );
            const BODY: &str = ".cross { color: rgb(1, 2, 3); }";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        BODY.len(),
                        BODY
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let stylesheet_url = Url::parse(&format!("http://{addr}/cross.css"))?;
        let document_url = Url::parse("http://127.0.0.1:1/page.html")?;
        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let fetcher =
            crate::stylesheet_blocking::RendererStylesheetFetcher::for_speculative_preload(
                loader,
                crate::network::RendererResourceTaskRunner::from_current_tokio()?,
                None,
                RequestResourceType::CssStyleSheet,
                true,
            );
        let terminal = crate::stylesheet_blocking::StylesheetFetcher::fetch_stylesheet_resource(
            &fetcher,
            document_url,
            stylesheet_url,
            StylesheetFetchOptions::default(),
        )
        .await;
        server.await?;

        assert_eq!(terminal.origin_clean(), Some(false));
        assert_eq!(
            terminal
                .ready_response()
                .expect("stylesheet response should be usable")
                .body_text(),
            ".cross { color: rgb(1, 2, 3); }"
        );
        Ok(())
    }

    #[tokio::test]
    async fn image_preload_link_becomes_ready_without_network_when_image_fetch_disabled()
    -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"image\" href=\"http://127.0.0.1:9/image.png\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        runtime.prime_pending_connected_style_loads();

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("disabled image fetch load event task");
        assert_eq!(ready.owner(), link);
        assert!(ready.successful());
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .has_pending_operation()
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .ready_connected_load_network_results
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn font_preload_link_readiness_tracks_font_type_and_priority_hint() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"font\" href=\"/font.woff2\" fetchpriority=\"high\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Font);
        assert!(options.link_preload);
        assert_eq!(options.fetch_priority_hint, Some(FetchPriorityHint::High));
    }

    #[tokio::test]
    async fn style_preload_request_attribute_change_replaces_exact_client_and_fetch() -> Result<()>
    {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=preload as=style href='http://127.0.0.1:9/a.css' crossorigin=anonymous></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.prime_connected_style_load_handle(link, None, std::ptr::null_mut(), None);
        let first = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("first style preload client");
        assert!(!first.installs_stylesheet());

        assert!(
            runtime
                .dom_host
                .set_attribute(link, "crossorigin", "use-credentials")
        );
        runtime.prime_connected_style_load_handle(link, None, std::ptr::null_mut(), None);
        let second = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("reprocessed style preload client");

        assert!(!StylesheetLinkClient::ptr_eq(&first, &second));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .accepts_stylesheet_link_client(&first),
            "owner replacement must revoke the old exact client"
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .accepts_stylesheet_link_client(&second),
            "the replacement client must be authoritative"
        );
        assert!(
            !runtime
                .stylesheet_lifecycle
                .link_client_index
                .contains(&first),
            "owner replacement must unregister the old pending client"
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .link_client_index
                .contains(&second),
            "the replacement pending client must remain indexed"
        );
        assert!(
            !first.fetch().ptr_eq(second.fetch()),
            "incompatible credentials must select a different typed resource"
        );
        assert!(!second.installs_stylesheet());
        assert_eq!(
            second.fetch().options().cross_origin(),
            Some("use-credentials")
        );
        assert_eq!(
            second.fetch().options().request_mode_and_credentials().1,
            RequestCredentialsMode::Include
        );
        Ok(())
    }

    #[tokio::test]
    async fn stylesheet_and_modulepreload_switches_drop_stale_event_bindings() -> Result<()> {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=preload as=style href='http://127.0.0.1:9/shared'></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_connected_style_loads(link);
        let first_style_client = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("initial stylesheet preload client");
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(link)
                .len(),
            1
        );

        assert!(runtime.dom_host.set_attribute(link, "rel", "modulepreload"));
        drop(runtime.invalidate_style_related_state(link));
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(link)
                .is_empty(),
            "stylesheet invalidation must remove its event binding"
        );
        runtime.queue_connected_style_loads(link);
        let (modulepreload_starts, warnings) =
            runtime.prime_pending_connected_style_loads().into_parts();
        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(modulepreload_starts.len(), 1);
        assert!(
            runtime
                .active_stylesheet_link_client_for_test(link)
                .is_none()
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(link)
                .is_empty(),
            "an in-flight modulepreload must retain identity without a load-delay binding"
        );

        assert!(runtime.dom_host.set_attribute(link, "rel", "preload"));
        drop(runtime.invalidate_style_related_state(link));
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(link)
                .is_empty(),
            "modulepreload invalidation must remove its event binding"
        );
        runtime.queue_connected_style_loads(link);
        let second_style_client = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("replacement stylesheet preload client");

        assert!(!StylesheetLinkClient::ptr_eq(
            &first_style_client,
            &second_style_client
        ));
        assert!(
            !runtime
                .stylesheet_lifecycle
                .owner_states
                .accepts_stylesheet_link_client(&first_style_client)
        );
        assert!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .accepts_stylesheet_link_client(&second_style_client)
        );
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .owner_states
                .cancelable_load_event_bindings(link)
                .len(),
            1,
            "only the replacement stylesheet processing may retain a binding"
        );
        Ok(())
    }

    #[test]
    fn media_preload_link_readiness_tracks_audio_and_video_types() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"preload\" as=\"audio\" href=\"/clip.ogg\">",
                "<link rel=\"preload\" as=\"video\" href=\"/clip.mp4\">",
                "</head><body></body></html>"
            )
            .to_owned(),
        );
        let links = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .expect("head links");
        let audio_options = preload_like_link_readiness_fetch_options(
            document
                .node(links[0])
                .and_then(crate::dom::native::Node::as_element)
                .expect("audio preload link"),
            false,
        );
        let video_options = preload_like_link_readiness_fetch_options(
            document
                .node(links[1])
                .and_then(crate::dom::native::Node::as_element)
                .expect("video preload link"),
            false,
        );

        assert_eq!(audio_options.resource_type, SubresourceResourceType::Audio);
        assert_eq!(video_options.resource_type, SubresourceResourceType::Video);
        assert_eq!(
            audio_options.request_resource_type,
            Some(RequestResourceType::Media)
        );
        assert_eq!(
            video_options.request_resource_type,
            Some(RequestResourceType::Media)
        );
    }

    #[test]
    fn late_style_preload_link_readiness_uses_medium_stylesheet_request_type() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"style\" href=\"/late.css\"></head><body></body></html>"
                .to_owned(),
        );
        let element = first_link_element(&document);
        let request = stylesheet_preload_link_request(
            &document,
            NodeId::new(first_link_handle(&document).index()),
        )
        .expect("style preload request");
        assert_eq!(
            preload_like_link_request_resource_type(
                element,
                SubresourceResourceType::Stylesheet,
                true,
            ),
            Some(RequestResourceType::LatePreloadCssStyleSheet)
        );
        assert_eq!(
            request.options().request_mode_and_credentials(),
            (
                moli_fetch::RequestMode::NoCors,
                RequestCredentialsMode::Include,
            )
        );
    }

    #[test]
    fn initial_scan_marks_preload_stylesheet_after_first_image_as_late() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"preload\" as=\"style\" href=\"/early.css\">",
                "</head><body>",
                "<img src=\"hero.png\">",
                "<link rel=\"preload\" as=\"style\" href=\"/late.css\">",
                "</body></html>"
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let early = link_handle_by_href(&document, "/early.css");
        let late = link_handle_by_href(&document, "/late.css");

        runtime.queue_initial_connected_style_loads();

        assert!(!runtime.late_preload_stylesheet_handles.contains(&early));
        assert!(runtime.late_preload_stylesheet_handles.contains(&late));
    }

    #[test]
    fn parser_created_non_blocking_media_stylesheet_waits_for_initial_scan() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"stylesheet\" media=\"print\" href=\"/print.css\"></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let link = first_link_handle(&document);

        runtime.queue_connected_style_loads(link);
        assert!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .is_empty()
        );

        runtime.queue_initial_connected_style_loads();
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .iter()
                .map(|queued| queued.owner())
                .collect::<Vec<_>>(),
            vec![link]
        );
    }

    #[test]
    fn parser_created_style_element_waits_for_initial_scan() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>@import url('/missing.css');</style></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let style = first_style_handle(&document);

        runtime.queue_connected_style_loads(style);
        assert!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .is_empty()
        );

        runtime.queue_initial_connected_style_loads();
        assert_eq!(
            runtime
                .stylesheet_lifecycle
                .pending_connected_loads
                .iter()
                .map(|queued| queued.owner())
                .collect::<Vec<_>>(),
            vec![style]
        );
    }

    #[test]
    fn initial_scan_does_not_repeat_a_previously_processed_connected_style() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><style>body { color: red; }</style></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let style = first_style_handle(&document);
        runtime
            .dom_host
            .node_mut(style)
            .expect("style node")
            .set_parser_created(false);

        runtime.queue_connected_style_loads(style);
        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        assert!(starts.is_empty(), "inline style must not start module work");
        assert!(warnings.is_empty(), "stylesheet warnings: {warnings:?}");
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("connected mutation should post one style event");
        let ReadyConnectedStyleLoadOperation::Connected(operation) = ready.operation() else {
            panic!("inline style should use a connected-load operation");
        };
        runtime
            .stylesheet_lifecycle
            .owner_states
            .consume_operation_event(operation);
        assert!(
            !runtime.connected_style_load_is_queued_for_test(style),
            "event settlement should remove transient lifecycle residence"
        );

        runtime.queue_initial_connected_style_loads();

        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "the one-time discovery scan must not process the same connected instance again"
        );
    }

    #[tokio::test]
    async fn parser_roundtrip_does_not_reschedule_connected_style_event() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"stylesheet\" href=\"data:text/css,body{}\"></head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        let ready = prime_pending_data_stylesheet_event(&mut runtime, link).await;
        assert!(ready.successful());

        let parser_snapshot = runtime.snapshot_document();
        runtime.replace_live_document_with_document(parser_snapshot);
        runtime.queue_initial_connected_style_loads();

        assert!(runtime.pop_ready_connected_style_load().is_none());
        assert!(
            runtime.stylesheet_lifecycle.owner_states.has_owner(link),
            "the exact posted operation must remain the owner fact until event consumption"
        );

        drop(runtime.invalidate_style_related_state(link));
        runtime.queue_connected_style_loads(link);
        let ready = prime_pending_data_stylesheet_event(&mut runtime, link).await;
        assert!(
            ready.successful(),
            "a style-affecting mutation must start a new load-event request sequence"
        );
    }

    #[tokio::test]
    async fn queued_connected_admission_reuses_parser_bound_terminal_client() {
        let stylesheet_url = Url::parse("data:text/css,body{}").unwrap();
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            Url::parse("https://example.test/page").unwrap(),
            format!(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"{stylesheet_url}\"></head><body></body></html>"
            ),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        // Model the narrow state under test: parser discovery has queued the
        // connected client, but the connected-load admission has not run yet.
        // A committed Document normally primes its initial scan immediately.
        runtime.enqueue_pending_connected_style_load_for_test(link);
        let speculative = runtime
            .preload_stylesheet(stylesheet_url, StylesheetFetchOptions::default())
            .expect("stylesheet speculation should be admitted");
        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "speculative stylesheet should publish one Networking terminal"
        );
        assert!(runtime.apply_next_stylesheet_networking_task_for_test());
        assert!(speculative.terminal().is_some());

        let completed_clients = runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );
        assert_eq!(completed_clients.len(), 1);
        assert_eq!(completed_clients[0].load().owner(), link);
        let parser_client = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("parser discovery should bind the exact owner client");
        assert!(
            parser_client.load_event_binding().is_none(),
            "the queued connected admission still owns the load delay binding"
        );

        let (starts, warnings) = runtime.prime_pending_connected_style_loads().into_parts();
        assert!(starts.is_empty(), "stylesheet must not start module work");
        assert!(warnings.is_empty(), "stylesheet warnings: {warnings:?}");
        let connected_client = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("connected admission should retain the owner client");
        assert!(StylesheetLinkClient::ptr_eq(
            &parser_client,
            &connected_client
        ));
        assert!(
            connected_client.load_event_binding().is_some(),
            "connected admission should attach its load delay binding"
        );

        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("the terminal owner should publish one load event");
        assert_eq!(ready.owner(), link);
        assert!(ready.successful());
        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "one owner operation must not publish a second load event"
        );
    }

    #[tokio::test]
    async fn cached_stylesheet_fetch_admission_posts_event_without_another_network_terminal() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let href = "data:text/css,body{}";
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            format!(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"{href}\"></head><body></body></html>"
            ),
        );
        let first_link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        let first_ready = prime_pending_data_stylesheet_event(&mut runtime, first_link).await;
        assert!(first_ready.successful());
        let first_physical = runtime.take_ready_stylesheet_network_results();
        assert_eq!(first_physical.len(), 1);
        assert!(first_physical[0].source_owners.is_empty());

        let head = runtime
            .dom_host()
            .document_head_handle()
            .expect("document head");
        let second_link = runtime.dom_host_mut().create_element("link");
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(second_link, "rel", "stylesheet")
        );
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(second_link, "href", href)
        );
        assert!(runtime.dom_host_mut().append_child(head, second_link));

        runtime.queue_connected_style_loads(second_link);
        let (starts, warnings, completed_clients) = runtime
            .prime_pending_connected_style_loads()
            .into_parts_with_completed_stylesheet_clients();
        assert!(starts.is_empty(), "stylesheet must not start module work");
        assert!(warnings.is_empty(), "stylesheet warnings: {warnings:?}");
        assert_eq!(completed_clients.len(), 1);
        assert_eq!(completed_clients[0].load().owner(), second_link);

        let second_ready = runtime
            .pop_ready_connected_style_load()
            .expect("cached fetch admission must publish its event immediately");
        assert_eq!(second_ready.owner(), second_link);
        assert!(second_ready.successful());
        assert!(
            runtime.take_ready_stylesheet_network_results().is_empty(),
            "late compatible client admission must not publish another physical terminal"
        );
        let selected_physical_client = runtime.take_ready_stylesheet_link_client_terminals();
        assert_eq!(selected_physical_client.len(), 1);
        assert_eq!(selected_physical_client[0].load().owner(), first_link);
        let (_, _, repeated_clients) = runtime
            .prime_pending_connected_style_loads()
            .into_parts_with_completed_stylesheet_clients();
        assert!(
            repeated_clients.is_empty(),
            "repeated lifecycle priming must not settle one client twice"
        );
        assert!(
            !runtime.apply_ready_stylesheet_networking_tasks_for_test(),
            "cached admission must not require a second network terminal"
        );
    }

    #[tokio::test]
    async fn completed_ownerless_failure_releases_parser_before_link_error_dispatch() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("stylesheet server");
        let addr = listener.local_addr().expect("stylesheet server address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("stylesheet request");
            let mut buffer = [0_u8; 1024];
            let request_size = socket.read(&mut buffer).await.expect("request bytes");
            let request = String::from_utf8_lossy(&buffer[..request_size]);
            assert!(request.starts_with("GET /missing.css "));
            socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Type: text/css\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .expect("stylesheet response");
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html")).unwrap();
        let stylesheet_url = document_url.join("/missing.css").unwrap();
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            document_url,
            "<!doctype html><html><head>\
             <link rel=\"stylesheet\" href=\"/missing.css\">\
             <script>globalThis.afterStyle = true</script>\
             </head><body></body></html>"
                .to_owned(),
        );
        let link = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        let speculative = runtime
            .preload_stylesheet(stylesheet_url, StylesheetFetchOptions::default())
            .expect("ownerless stylesheet should be admitted");
        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "ownerless failure should publish one Networking terminal"
        );
        assert!(runtime.apply_next_stylesheet_networking_task_for_test());
        server.await.expect("stylesheet server should finish");
        assert!(
            speculative
                .terminal()
                .is_some_and(|terminal| !terminal.is_ready())
        );

        let completed_clients = runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );
        assert_eq!(completed_clients.len(), 1);
        assert_eq!(completed_clients[0].load().owner(), link);
        let signatures = blocking_stylesheet_inputs
            .iter()
            .map(|input| input.signature().clone())
            .collect::<Vec<_>>();
        assert!(
            !runtime.has_pending_document_owned_blocking_stylesheet_signatures(signatures.iter()),
            "the completed failure must settle the physical stylesheet blocker"
        );
        assert!(
            !runtime.has_pending_parser_script_blocking_stylesheet_signatures(signatures.iter()),
            "a terminal stylesheet failure must release script execution before its error task"
        );

        let load = runtime
            .active_stylesheet_link_client_for_test(link)
            .expect("parser discovery should bind the late stylesheet client");
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("late client admission should publish its exact error task");
        assert_eq!(ready.owner(), link);
        assert!(!ready.successful());
        runtime
            .stylesheet_lifecycle
            .owner_states
            .consume_link_event(&load);
        assert!(
            !runtime.has_pending_document_owned_blocking_stylesheet_signatures(signatures.iter()),
            "dispatch must not recreate the settled physical stylesheet blocker"
        );
        assert!(
            !runtime.has_pending_parser_script_blocking_stylesheet_signatures(signatures.iter()),
            "dispatching the exact link error task must not recreate the released script gate"
        );
        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "the exact link failure should publish only one error task"
        );
    }

    #[tokio::test]
    async fn pending_style_preload_and_stylesheet_clients_share_one_typed_fetch() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("stylesheet server");
        let addr = listener.local_addr().expect("stylesheet server address");
        let (accepted_tx, accepted_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("stylesheet request");
            let mut buffer = [0_u8; 1024];
            let request_size = socket.read(&mut buffer).await.expect("request bytes");
            let request = String::from_utf8_lossy(&buffer[..request_size]);
            assert!(request.starts_with("GET /shared.css "));
            accepted_tx.send(()).expect("accepted receiver");
            release_rx.await.expect("stylesheet release");
            let body = ".shared { color: green; }";
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .expect("stylesheet response");
        });

        let document_url = Url::parse(&format!("http://{addr}/page.html")).unwrap();
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            document_url,
            "<!doctype html><html><head>\
             <link rel=\"preload\" as=\"style\" href=\"/shared.css\">\
             <link rel=\"stylesheet\" href=\"/shared.css\">\
             </head><body></body></html>"
                .to_owned(),
        );
        let links = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .expect("head links");
        let preload = links[0];
        let stylesheet = links[1];
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_connected_style_loads(preload);
        runtime.prime_pending_connected_style_loads();
        tokio::time::timeout(std::time::Duration::from_secs(2), accepted_rx)
            .await
            .expect("style preload should reach the server")
            .expect("style preload accepted signal");
        let preload_client = runtime
            .active_stylesheet_link_client_for_test(preload)
            .expect("preload client");
        assert!(!preload_client.installs_stylesheet());

        runtime.queue_connected_style_loads(stylesheet);
        runtime.prime_pending_connected_style_loads();
        let stylesheet_client = runtime
            .active_stylesheet_link_client_for_test(stylesheet)
            .expect("stylesheet client");
        assert!(stylesheet_client.installs_stylesheet());
        assert!(
            preload_client.fetch().ptr_eq(stylesheet_client.fetch()),
            "pending preload and stylesheet clients must join the exact typed resource"
        );

        release_tx.send(()).expect("release stylesheet response");
        assert!(
            runtime.wait_for_stylesheet_networking_task_for_test().await,
            "shared typed fetch should publish one Networking terminal"
        );
        runtime.drain_ready_connected_style_load_completions();
        server.await.expect("stylesheet server should finish");

        let physical = runtime.take_ready_stylesheet_network_results();
        assert_eq!(physical.len(), 1);
        let terminals = runtime.take_ready_stylesheet_link_client_terminals();
        assert_eq!(terminals.len(), 2);
        assert_eq!(
            terminals
                .iter()
                .filter(|terminal| terminal.load().installs_stylesheet())
                .count(),
            1
        );
        // ScriptVm normally parses the terminal body and completes the shared
        // import graph. This low-level fixture has no ScriptVm, so publish the
        // known-empty graph explicitly before asserting the install client's
        // load event.
        runtime.note_stylesheet_import_graph_completion(
            stylesheet_client.fetch(),
            Arc::new(
                crate::stylesheet_blocking::StylesheetImportGraphFetchResult::new(true, Vec::new()),
            ),
        );
        let mut ready_owners = Vec::new();
        while let Some(ready) = runtime.pop_ready_connected_style_load() {
            assert!(ready.successful());
            ready_owners.push(ready.owner().index());
        }
        assert_eq!(
            ready_owners,
            vec![preload.index(), stylesheet.index()],
            "shared resource clients must be notified in registration order"
        );
        assert!(
            !runtime.apply_ready_stylesheet_networking_tasks_for_test(),
            "shared clients must not leave a second physical completion"
        );
    }

    #[tokio::test]
    async fn completed_style_preload_admits_late_stylesheet_install_client() {
        let href = "data:text/css,.late%7Bcolor%3Agreen%7D";
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            format!(
                "<!doctype html><html><head><link rel=\"preload\" as=\"style\" href=\"{href}\"></head><body></body></html>"
            ),
        );
        let preload = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        let preload_ready = prime_pending_data_stylesheet_event(&mut runtime, preload).await;
        assert!(preload_ready.successful());
        let preload_client = runtime
            .active_stylesheet_link_client_for_test(preload)
            .expect("preload client");
        assert!(!preload_client.installs_stylesheet());
        assert_eq!(runtime.take_ready_stylesheet_network_results().len(), 1);
        let preload_terminals = runtime.take_ready_stylesheet_link_client_terminals();
        assert_eq!(preload_terminals.len(), 1);
        assert!(!preload_terminals[0].load().installs_stylesheet());

        let head = runtime.dom_host().document_head_handle().expect("head");
        let stylesheet = runtime.dom_host_mut().create_element("link");
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(stylesheet, "rel", "stylesheet")
        );
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(stylesheet, "href", href)
        );
        assert!(runtime.dom_host_mut().append_child(head, stylesheet));
        runtime.queue_connected_style_loads(stylesheet);
        let (_, _, completed_clients) = runtime
            .prime_pending_connected_style_loads()
            .into_parts_with_completed_stylesheet_clients();

        let stylesheet_client = runtime
            .active_stylesheet_link_client_for_test(stylesheet)
            .expect("late stylesheet client");
        assert!(stylesheet_client.installs_stylesheet());
        assert!(preload_client.fetch().ptr_eq(stylesheet_client.fetch()));
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("late stylesheet client event");
        assert_eq!(ready.owner(), stylesheet);
        assert!(ready.successful());
        assert!(runtime.take_ready_stylesheet_network_results().is_empty());
        assert_eq!(completed_clients.len(), 1);
        assert!(completed_clients[0].load().installs_stylesheet());
        assert!(
            !runtime.apply_ready_stylesheet_networking_tasks_for_test(),
            "completed-client admission must not create another physical task"
        );
    }

    #[tokio::test]
    async fn completed_stylesheet_admits_late_preload_event_only_client() {
        let href = "data:text/css,.first%7Bcolor%3Agreen%7D";
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            format!(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"{href}\"></head><body></body></html>"
            ),
        );
        let stylesheet = first_link_handle(&document);
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);

        runtime.queue_initial_connected_style_loads();
        let stylesheet_ready = prime_pending_data_stylesheet_event(&mut runtime, stylesheet).await;
        assert!(stylesheet_ready.successful());
        let stylesheet_client = runtime
            .active_stylesheet_link_client_for_test(stylesheet)
            .expect("stylesheet client");
        assert!(stylesheet_client.installs_stylesheet());
        assert_eq!(runtime.take_ready_stylesheet_network_results().len(), 1);
        assert_eq!(
            runtime.take_ready_stylesheet_link_client_terminals().len(),
            1
        );

        let head = runtime.dom_host().document_head_handle().expect("head");
        let preload = runtime.dom_host_mut().create_element("link");
        assert!(
            runtime
                .dom_host_mut()
                .set_attribute(preload, "rel", "preload")
        );
        assert!(runtime.dom_host_mut().set_attribute(preload, "as", "style"));
        assert!(runtime.dom_host_mut().set_attribute(preload, "href", href));
        assert!(runtime.dom_host_mut().append_child(head, preload));
        runtime.queue_connected_style_loads(preload);
        let (_, _, completed_clients) = runtime
            .prime_pending_connected_style_loads()
            .into_parts_with_completed_stylesheet_clients();

        let preload_client = runtime
            .active_stylesheet_link_client_for_test(preload)
            .expect("late preload client");
        assert!(!preload_client.installs_stylesheet());
        assert!(stylesheet_client.fetch().ptr_eq(preload_client.fetch()));
        let ready = runtime
            .pop_ready_connected_style_load()
            .expect("late preload client event");
        assert_eq!(ready.owner(), preload);
        assert!(ready.successful());
        assert!(runtime.take_ready_stylesheet_network_results().is_empty());
        assert_eq!(completed_clients.len(), 1);
        assert!(!completed_clients[0].load().installs_stylesheet());
    }

    #[test]
    fn prefetch_link_readiness_uses_fetch_cdp_type_and_link_prefetch_request_type() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"prefetch\" as=\"image\" href=\"/next.png\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Fetch);
        assert_eq!(
            options.request_resource_type,
            Some(RequestResourceType::LinkPrefetch)
        );
        assert!(!options.link_preload);
        assert!(options.script_fetch_metadata.is_none());
    }

    #[test]
    fn fetch_preload_link_readiness_uses_fetch_cdp_type_and_raw_request_type() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"fetch\" href=\"/data.json\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Fetch);
        assert_eq!(
            options.request_resource_type,
            Some(RequestResourceType::Raw)
        );
        assert_eq!(options.request_mode, moli_fetch::RequestMode::NoCors);
        assert!(options.link_preload);
        assert!(options.script_fetch_metadata.is_none());
    }

    #[test]
    fn crossorigin_fetch_preload_link_readiness_uses_cors_mode() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel=\"preload\" as=\"fetch\" href=\"/anonymous.json\" crossorigin=\"anonymous\">",
                "<link rel=\"preload\" as=\"fetch\" href=\"/credentials.json\" crossorigin=\"use-credentials\">",
                "</head><body></body></html>"
            )
            .to_owned(),
        );
        let links = document
            .document_head_handle()
            .and_then(|head| document.child_nodes(head))
            .expect("head links");
        let anonymous_options = preload_like_link_readiness_fetch_options(
            document
                .node(links[0])
                .and_then(crate::dom::native::Node::as_element)
                .expect("anonymous fetch preload link"),
            false,
        );
        let credentials_options = preload_like_link_readiness_fetch_options(
            document
                .node(links[1])
                .and_then(crate::dom::native::Node::as_element)
                .expect("credentialed fetch preload link"),
            false,
        );

        assert_eq!(
            anonymous_options.request_mode,
            moli_fetch::RequestMode::Cors
        );
        assert_eq!(
            anonymous_options.credentials_mode,
            RequestCredentialsMode::SameOrigin
        );
        assert_eq!(
            credentials_options.request_mode,
            moli_fetch::RequestMode::Cors
        );
        assert_eq!(
            credentials_options.credentials_mode,
            RequestCredentialsMode::Include
        );
    }

    #[test]
    fn style_preload_link_without_crossorigin_uses_no_cors_request_parameters() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"style\" href=\"/style.css\"></head><body></body></html>"
                .to_owned(),
        );
        let preload = stylesheet_preload_link_request(
            &document,
            NodeId::new(first_link_handle(&document).index()),
        )
        .expect("style preload request");
        assert_eq!(
            preload.options().request_mode_and_credentials(),
            (
                moli_fetch::RequestMode::NoCors,
                RequestCredentialsMode::Include,
            )
        );
        let request = crate::stylesheet_blocking::apply_stylesheet_request_parameters(
            moli_fetch::Request::new("GET", "https://example.test/style.css", None, vec![])
                .expect("request"),
            preload.options(),
        );
        assert_eq!(
            request.browser_request_metadata(),
            Some(moli_fetch::BrowserRequestMetadata::Style)
        );
    }

    #[test]
    fn anonymous_style_preload_uses_cors_request_parameters() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"preload\" as=\"style\" href=\"/style.css\" crossorigin=\"anonymous\"></head><body></body></html>"
                .to_owned(),
        );
        let preload = stylesheet_preload_link_request(
            &document,
            NodeId::new(first_link_handle(&document).index()),
        )
        .expect("style preload request");
        assert_eq!(
            preload.options().request_mode_and_credentials(),
            (
                moli_fetch::RequestMode::Cors,
                RequestCredentialsMode::SameOrigin,
            )
        );
        let request = crate::stylesheet_blocking::apply_stylesheet_request_parameters(
            moli_fetch::Request::new("GET", "https://example.test/style.css", None, vec![])
                .expect("request"),
            preload.options(),
        );
        assert_eq!(
            request.browser_request_metadata(),
            Some(moli_fetch::BrowserRequestMetadata::Style)
        );
    }

    #[test]
    fn compression_dictionary_link_readiness_uses_other_cdp_type_and_dictionary_request_type() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"compression-dictionary\" href=\"/dict.bin\"></head><body></body></html>"
                .to_owned(),
        );
        let options =
            preload_like_link_readiness_fetch_options(first_link_element(&document), false);

        assert_eq!(options.resource_type, SubresourceResourceType::Dictionary);
        assert_eq!(options.resource_type.as_cdp_type(), "Other");
        assert_eq!(
            options.request_resource_type,
            Some(RequestResourceType::Dictionary)
        );
        assert_eq!(options.credentials_mode, RequestCredentialsMode::Omit);
        assert!(!options.link_preload);
        assert!(options.script_fetch_metadata.is_none());
    }

    #[test]
    fn connected_style_load_handles_include_prefetch_links() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"prefetch\" href=\"/next.html\"></head><body></body></html>"
                .to_owned(),
        );
        let runtime = DocumentRuntime::new(&document);
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle");

        assert_eq!(runtime.connected_style_load_handles(head), vec![link]);
    }

    #[test]
    fn connected_style_load_handles_include_compression_dictionary_links() {
        let parser = HtmlParser::SCRIPTING_ENABLED;
        let document = parser.parse(
            Url::parse("https://example.test/page").unwrap(),
            "<!doctype html><html><head><link rel=\"compression-dictionary\" href=\"/dict.bin\"></head><body></body></html>"
                .to_owned(),
        );
        let runtime = DocumentRuntime::new(&document);
        let head = document.document_head_handle().expect("head handle");
        let link = document
            .child_nodes(head)
            .expect("head children")
            .into_iter()
            .find(|handle| {
                document
                    .node(*handle)
                    .and_then(Node::as_element)
                    .is_some_and(|element| element.is_html_element("link"))
            })
            .expect("link handle");

        assert_eq!(runtime.connected_style_load_handles(head), vec![link]);
    }

    async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            let read = stream.read(&mut byte).await?;
            if read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "client closed before sending complete request",
                ));
            }
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                return Ok(());
            }
        }
    }

    #[test]
    fn connected_link_service_worker_destination_tracks_resource_type() {
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Stylesheet
            ),
            Some(ServiceWorkerRequestDestination::Style)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Image
            ),
            Some(ServiceWorkerRequestDestination::Image)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Font
            ),
            Some(ServiceWorkerRequestDestination::Font)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Script
            ),
            Some(ServiceWorkerRequestDestination::Script)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Audio
            ),
            Some(ServiceWorkerRequestDestination::Audio)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Video
            ),
            Some(ServiceWorkerRequestDestination::Video)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::TextTrack
            ),
            Some(ServiceWorkerRequestDestination::Track)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::CspReport
            ),
            Some(ServiceWorkerRequestDestination::Report)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Dictionary
            ),
            Some(ServiceWorkerRequestDestination::Dictionary)
        );
        assert_eq!(
            ServiceWorkerRequestDestination::for_subresource_resource_type(
                SubresourceResourceType::Ping
            ),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn script_preload_like_link_readiness_joins_script_text_cache() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let body = "export const joined = true;";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let loader_owner = ResourceRequestClient::new(&FetchConfig::default())?;
        let loader = loader_owner.handle();
        let document_url = Url::parse(&format!("http://{addr}/page"))?;
        let script_url = Url::parse(&format!("http://{addr}/app.mjs"))?;
        let metadata = ScriptFetchMetadata {
            cross_origin: Some("anonymous".to_owned()),
            ..ScriptFetchMetadata::default()
        };
        let script_request = moli_fetch::Request::new("GET", script_url.as_str(), None, vec![])?
            .with_page_network_policy()
            .with_initiator_url(&document_url)
            .with_credentials_mode(module_script_credentials_mode(
                metadata.cross_origin.as_deref(),
            ))
            .with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
                cross_origin: metadata.cross_origin.clone(),
                referrer_policy: metadata.referrer_policy.clone(),
                document_referrer_policy: None,
                charset: metadata.charset.clone(),
                integrity: metadata.integrity.clone(),
                nonce: metadata.nonce.clone(),
                fetch_priority: metadata.fetch_priority,
                scheduler_priority: None,
            });

        let script_response = loader
            .fetch_cacheable_script_text_stream(script_request)
            .await?;
        server.await?;

        let link_response = fetch_connected_link_readiness(
            loader,
            document_url,
            script_url,
            ConnectedLinkReadinessFetchOptions {
                resource_type: SubresourceResourceType::Script,
                request_resource_type: Some(RequestResourceType::Script),
                script_fetch_metadata: Some(metadata),
                request_mode: moli_fetch::RequestMode::Cors,
                credentials_mode: RequestCredentialsMode::SameOrigin,
                fetch_priority_hint: None,
                link_preload: true,
                link_fetch_options: StylesheetFetchOptions::from_link_attributes(
                    Some("anonymous"),
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

        assert_eq!(script_response.body_text(), "export const joined = true;");
        assert_eq!(link_response.body_text(), "export const joined = true;");
        Ok(())
    }
}
