use super::{
    ChildBrowsingContextBootstrap, ChildBrowsingContextSnapshot, ChildFrameAttachmentSnapshot,
    JsContextHost, NavigationActivationSeed, NavigationHistoryDocumentId, NavigationHistoryEntryId,
    NavigationHistoryEntryKey, NavigationHistoryEntrySeed, NavigationHistorySerializedEntry,
    child_documents::CompletedFrameOwnerResourceTiming,
};
use crate::{
    context_bootstrap::set_top_level_history_length_at_least_for_runtime_owner,
    document_runtime::{DocumentPolicyContainer, DocumentSandboxPolicy, DomHandle},
    frame_owner_model::{
        ChildFrameOwnerSnapshot, FrameDocumentTaskOwner, FrameId, FrameRealmId, FrameScriptJob,
        FrameScriptJobKind,
    },
    protocol_types::ChildFrameDocumentNetworkSnapshot,
    service_worker_runtime::ServiceWorkerClientId,
    types::ServiceWorkerClientNavigateContinuation,
};
use moli_page_types::{
    apply_child_browsing_context_javascript_url_navigation_to_entry_seed as apply_child_javascript_url_navigation_to_seed,
    apply_child_browsing_context_navigation_to_entry_seed as apply_child_navigation_to_seed,
    replace_child_browsing_context_navigation_in_entry_seed as replace_child_navigation_in_seed,
};
use std::collections::HashSet;
use url::Url;

mod classic_scripts;
mod discovery;
mod lookup;
mod module_scripts;
mod registry;
mod request_scope;
mod stylesheets;

pub(in crate::native_bridge::context_host) use classic_scripts::ChildParserClassicScriptCandidate;
pub(in crate::native_bridge::context_host) use classic_scripts::PendingChildExternalClassicDocumentScriptLoad;
pub(crate) use request_scope::WebStorageScope;
pub(in crate::native_bridge::context_host) use request_scope::{
    document_sandbox_policy_from_attribute, sandbox_attribute_forces_opaque_origin,
};

#[derive(Debug, Clone)]
pub(super) struct ChildBrowsingContextEntry {
    frame_id: String,
    current_document_loader_id: Option<String>,
    name: Option<String>,
    id: Option<String>,
    attribute_bootstrap: ChildBrowsingContextBootstrap,
    pending_attribute_bootstrap_commit: bool,
    pending_live_navigation: Option<ChildBrowsingContextBootstrap>,
    pending_live_navigation_reflects_window_state: bool,
    live_bootstrap: ChildBrowsingContextBootstrap,
    navigation_entry_seed: NavigationHistoryEntrySeed,
    committed_navigation_entry_seed: NavigationHistoryEntrySeed,
    cached_snapshot: Option<ChildBrowsingContextSnapshot>,
    document_policy_container: ChildDocumentPolicyContainer,
    completed_document_network: Option<CompletedChildDocumentNetwork>,
    completed_frame_owner_resource_timing: Option<CompletedFrameOwnerResourceTiming>,
    performance_time_origin: ChildPerformanceTimeOrigin,
    pending_document_load_id: Option<u64>,
    classic_script_document_state: classic_scripts::ChildClassicScriptDocumentState,
    document_domain_override: Option<String>,
    credentialless: bool,
    service_worker_client_id: Option<ServiceWorkerClientId>,
    pending_service_worker_client_id: Option<ServiceWorkerClientId>,
    pending_service_worker_client_navigation: Option<ServiceWorkerClientNavigateContinuation>,
    pending_top_level_history_length_increment: bool,
}

#[derive(Debug, Clone)]
struct CompletedChildDocumentNetwork {
    owner: FrameDocumentTaskOwner,
    snapshot: ChildFrameDocumentNetworkSnapshot,
}

pub(super) type ChildDocumentPolicyContainer = DocumentPolicyContainer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChildPerformanceTimeOrigin(u64);

impl ChildPerformanceTimeOrigin {
    pub(super) fn now() -> Self {
        Self(moli_time::unix_epoch_millis().to_bits())
    }

    pub(super) fn as_millis(self) -> f64 {
        f64::from_bits(self.0)
    }
}

pub(crate) struct ChildBrowsingContextNavigationSeedSnapshot {
    pub(crate) navigation_entry_seed: NavigationHistoryEntrySeed,
    pub(crate) committed_navigation_entry_seed: NavigationHistoryEntrySeed,
    pub(crate) pending_attribute_bootstrap_commit: bool,
    pub(crate) pending_live_navigation_reflects_window_state: bool,
}

pub(in crate::native_bridge::context_host) struct ChildBrowsingContextVisibleNavigationState {
    pub(in crate::native_bridge::context_host) href: String,
    pub(in crate::native_bridge::context_host) entry_seed: NavigationHistoryEntrySeed,
    pub(in crate::native_bridge::context_host) seed_is_committed: bool,
}

pub(in crate::native_bridge::context_host) struct ChildBrowsingContextFrameIdentitySnapshot {
    pub(in crate::native_bridge::context_host) frame_id: String,
    pub(in crate::native_bridge::context_host) name: Option<String>,
    pub(in crate::native_bridge::context_host) owner_element_id: Option<String>,
    pub(in crate::native_bridge::context_host) security_origin_inherited: bool,
}

impl ChildBrowsingContextEntry {
    fn matches_browsing_context_name(&self, key: &str) -> bool {
        self.name.as_deref() == Some(key)
    }

    pub(super) fn clear_document_runtime_state(&mut self) {
        self.clear_script_execution_state();
        self.clear_document_domain_override();
    }

    pub(super) fn document_domain_override(&self) -> Option<String> {
        self.document_domain_override.clone()
    }

    pub(super) fn set_document_domain_override(&mut self, domain: String) {
        self.document_domain_override = Some(domain);
    }

    pub(super) fn clear_document_domain_override(&mut self) {
        self.document_domain_override = None;
    }

    pub(super) fn window_name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }

    pub(super) fn set_window_name(&mut self, name: String) {
        self.name = Some(name);
    }

    pub(super) fn live_bootstrap(&self) -> ChildBrowsingContextBootstrap {
        self.live_bootstrap.clone()
    }

    pub(super) fn set_live_bootstrap(&mut self, bootstrap: ChildBrowsingContextBootstrap) {
        self.live_bootstrap = bootstrap;
    }

    pub(super) fn set_live_url_bootstrap(&mut self, url: Url) {
        self.set_live_bootstrap(ChildBrowsingContextBootstrap::Url(url));
    }

    pub(super) fn rewrite_live_url_bootstrap_after_load(&mut self, final_url: &Url) {
        if matches!(self.live_bootstrap, ChildBrowsingContextBootstrap::Url(_)) {
            self.set_live_url_bootstrap(final_url.clone());
        }
    }

    pub(super) fn attribute_bootstrap(&self) -> &ChildBrowsingContextBootstrap {
        &self.attribute_bootstrap
    }

    pub(super) fn attribute_bootstrap_changed(
        &self,
        bootstrap: &ChildBrowsingContextBootstrap,
    ) -> bool {
        self.attribute_bootstrap != *bootstrap
    }

    pub(super) fn pending_attribute_bootstrap_commit(&self) -> bool {
        self.pending_attribute_bootstrap_commit
    }

    pub(super) fn pending_attribute_bootstrap_commit_for_refresh(
        existing: Option<&Self>,
        is_new: bool,
        attribute_bootstrap_changed: bool,
        initial_about_blank_commit_is_synchronous: bool,
    ) -> bool {
        if initial_about_blank_commit_is_synchronous {
            return false;
        }
        if attribute_bootstrap_changed {
            return true;
        }
        existing
            .map(Self::pending_attribute_bootstrap_commit)
            .unwrap_or(is_new)
    }

    pub(super) fn clear_pending_attribute_bootstrap_commit(&mut self) {
        self.pending_attribute_bootstrap_commit = false;
    }

    pub(super) fn replace_attribute_bootstrap(&mut self, bootstrap: ChildBrowsingContextBootstrap) {
        self.attribute_bootstrap = bootstrap;
    }

    pub(super) fn current_document_url(&self) -> Option<Url> {
        if matches!(
            self.live_bootstrap,
            ChildBrowsingContextBootstrap::Srcdoc { .. }
        ) {
            return child_document_url_for_bootstrap(&self.live_bootstrap);
        }
        if self.pending_attribute_bootstrap_commit || self.has_pending_window_state() {
            return child_document_url_for_bootstrap(&self.live_bootstrap);
        }
        if let Some(url) =
            child_navigation_current_url_as_url(&self.committed_navigation_entry_seed)
        {
            return Some(url);
        }
        if let Some(snapshot) = &self.cached_snapshot {
            return Some(snapshot.url.clone());
        }
        child_document_url_for_bootstrap(&self.live_bootstrap)
    }

    pub(super) fn security_origin_inherited(&self) -> bool {
        self.live_bootstrap.security_origin_inherited()
    }

    pub(super) fn document_policy_container_snapshot(&self) -> ChildDocumentPolicyContainer {
        self.document_policy_container.clone()
    }

    pub(super) fn owner_credentialless(&self) -> bool {
        self.credentialless
    }

    pub(super) fn document_referrer(&self) -> &str {
        self.document_policy_container.document_referrer.as_str()
    }

    pub(super) fn document_referrer_policy(&self) -> Option<&str> {
        self.document_policy_container.referrer_policy.as_deref()
    }

    pub(super) fn document_credentialless(&self) -> bool {
        self.document_policy_container.credentialless
    }

    pub(super) fn document_credentialless_storage_nonce(
        &self,
    ) -> Option<moli_storage_key::OpaqueOriginNonce> {
        self.document_policy_container.credentialless_storage_nonce
    }

    pub(super) fn set_document_credentialless_state(
        &mut self,
        credentialless: bool,
        storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    ) {
        self.document_policy_container.credentialless = credentialless;
        self.document_policy_container.credentialless_storage_nonce = storage_nonce;
    }

    pub(super) fn document_sandbox_policy(&self) -> DocumentSandboxPolicy {
        self.document_policy_container.sandbox
    }

    pub(super) fn set_document_sandbox_policy(&mut self, sandbox: DocumentSandboxPolicy) {
        self.document_policy_container.sandbox = sandbox;
    }

    pub(super) fn document_sandbox_forces_opaque_origin(&self) -> bool {
        self.document_sandbox_policy().forces_opaque_origin
    }

    pub(super) fn document_sandbox_allows_scripts(&self) -> bool {
        self.document_sandbox_policy().allows_scripts
    }

    pub(super) fn response_content_security_policies(&self) -> &[String] {
        self.document_policy_container
            .response_content_security_policies
            .as_slice()
    }

    pub(super) fn document_content_security_policies(&self) -> &[String] {
        self.document_policy_container
            .document_content_security_policies
            .as_slice()
    }

    pub(super) fn response_content_security_report_only_policies(&self) -> &[String] {
        self.document_policy_container
            .response_content_security_report_only_policies
            .as_slice()
    }

    pub(super) fn has_response_content_security_policies(&self) -> bool {
        !self.response_content_security_policies().is_empty()
    }

    pub(super) fn content_security_reporting_endpoints(
        &self,
    ) -> crate::content_security_policy::ContentSecurityPolicyReportingEndpoints {
        self.document_policy_container
            .content_security_reporting_endpoints
            .clone()
    }

    pub(super) fn apply_loaded_document_policy(
        &mut self,
        policy_container: &ChildDocumentPolicyContainer,
        sandbox: DocumentSandboxPolicy,
        credentialless: bool,
        credentialless_storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    ) {
        self.document_policy_container.referrer_policy = policy_container.referrer_policy.clone();
        self.document_policy_container.cross_origin_embedder_policy =
            policy_container.cross_origin_embedder_policy;
        self.document_policy_container.document_isolation_policy =
            policy_container.document_isolation_policy;
        self.document_policy_container.cross_origin_isolated =
            policy_container.cross_origin_isolated;
        self.document_policy_container
            .document_content_security_policies =
            policy_container.document_content_security_policies.clone();
        self.document_policy_container
            .response_content_security_policies =
            policy_container.response_content_security_policies.clone();
        self.document_policy_container
            .response_content_security_report_only_policies = policy_container
            .response_content_security_report_only_policies
            .clone();
        self.document_policy_container
            .content_security_reporting_endpoints = policy_container
            .content_security_reporting_endpoints
            .clone();
        self.set_document_credentialless_state(credentialless, credentialless_storage_nonce);
        self.set_document_sandbox_policy(sandbox);
    }

    pub(super) fn sync_document_policy_from_snapshot(
        &mut self,
        snapshot: &ChildBrowsingContextSnapshot,
    ) {
        let sandbox = self
            .document_policy_container
            .sandbox
            .with_response_content_security_policy(snapshot.policy_container.sandbox);
        let credentialless = self.document_policy_container.credentialless;
        let credentialless_storage_nonce =
            self.document_policy_container.credentialless_storage_nonce;
        self.document_policy_container.referrer_policy =
            snapshot.policy_container.referrer_policy.clone();
        self.document_policy_container.cross_origin_embedder_policy =
            snapshot.policy_container.cross_origin_embedder_policy;
        self.document_policy_container.document_isolation_policy =
            snapshot.policy_container.document_isolation_policy;
        self.document_policy_container.cross_origin_isolated =
            snapshot.policy_container.cross_origin_isolated;
        self.document_policy_container
            .response_content_security_policies = snapshot
            .policy_container
            .response_content_security_policies
            .clone();
        self.document_policy_container
            .response_content_security_report_only_policies = snapshot
            .policy_container
            .response_content_security_report_only_policies
            .clone();
        self.document_policy_container
            .content_security_reporting_endpoints = snapshot
            .policy_container
            .content_security_reporting_endpoints
            .clone();
        self.set_document_credentialless_state(credentialless, credentialless_storage_nonce);
        self.set_document_sandbox_policy(sandbox);
    }

    pub(super) fn commit_pending_child_document_load(
        &mut self,
        final_url: &Url,
        policy_container: &ChildDocumentPolicyContainer,
        sandbox: DocumentSandboxPolicy,
        credentialless: bool,
        credentialless_storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    ) {
        self.set_live_url_bootstrap(final_url.clone());
        self.reset_performance_time_origin();
        self.clear_document_runtime_state();
        self.rewrite_current_navigation_url_after_load(final_url);
        self.apply_loaded_referrer_policy_to_current_document(
            policy_container.referrer_policy.clone(),
        );
        self.apply_loaded_document_policy(
            policy_container,
            sandbox,
            credentialless,
            credentialless_storage_nonce,
        );
        self.commit_current_navigation_entry_seed();
        self.clear_completed_document_network();
    }

    pub(super) fn commit_child_document_after_failed_async_start(
        &mut self,
        bootstrap: ChildBrowsingContextBootstrap,
        snapshot: &ChildBrowsingContextSnapshot,
        sandbox: DocumentSandboxPolicy,
        credentialless: bool,
        credentialless_storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    ) {
        self.set_live_bootstrap(bootstrap);
        self.reset_performance_time_origin();
        self.clear_pending_document_load();
        self.clear_document_runtime_state();
        self.clear_completed_document_network();
        self.set_document_credentialless_state(credentialless, credentialless_storage_nonce);
        self.set_document_sandbox_policy(sandbox);
        self.sync_document_policy_from_snapshot(snapshot);
        self.commit_current_navigation_entry_seed();
    }

    pub(super) fn commit_new_child_document(
        &mut self,
        bootstrap: ChildBrowsingContextBootstrap,
        snapshot: Option<&ChildBrowsingContextSnapshot>,
        sandbox: DocumentSandboxPolicy,
        credentialless: bool,
        credentialless_storage_nonce: Option<moli_storage_key::OpaqueOriginNonce>,
    ) {
        self.set_live_bootstrap(bootstrap);
        self.reset_performance_time_origin();
        self.clear_pending_document_load();
        self.clear_document_runtime_state();
        self.clear_completed_document_network();
        self.set_document_credentialless_state(credentialless, credentialless_storage_nonce);
        self.set_document_sandbox_policy(sandbox);
        if let Some(snapshot) = snapshot {
            self.sync_document_policy_from_snapshot(snapshot);
        }
        self.commit_current_navigation_entry_seed();
    }

    pub(super) fn cached_snapshot(&self) -> Option<ChildBrowsingContextSnapshot> {
        self.cached_snapshot.clone()
    }

    pub(super) fn cached_snapshot_ref(&self) -> Option<&ChildBrowsingContextSnapshot> {
        self.cached_snapshot.as_ref()
    }

    pub(super) fn has_cached_snapshot(&self) -> bool {
        self.cached_snapshot.is_some()
    }

    pub(super) fn set_cached_snapshot(&mut self, snapshot: Option<ChildBrowsingContextSnapshot>) {
        self.cached_snapshot = snapshot;
    }

    pub(super) fn cache_snapshot(&mut self, snapshot: ChildBrowsingContextSnapshot) {
        self.set_cached_snapshot(Some(snapshot));
    }

    pub(super) fn clear_cached_snapshot(&mut self) {
        self.set_cached_snapshot(None);
    }

    pub(super) fn cache_snapshot_for_current_url_bootstrap(
        &mut self,
        url: &Url,
        snapshot: &ChildBrowsingContextSnapshot,
    ) {
        if self.live_bootstrap == ChildBrowsingContextBootstrap::Url(url.clone()) {
            self.cache_snapshot(snapshot.clone());
        }
    }

    pub(super) fn frame_id(&self) -> &str {
        &self.frame_id
    }

    pub(super) fn current_document_loader_id(&self) -> Option<&str> {
        self.current_document_loader_id.as_deref()
    }

    pub(super) fn set_current_document_loader_id(&mut self, loader_id: String) {
        self.current_document_loader_id = Some(loader_id);
    }

    pub(super) fn clear_current_document_loader_id(&mut self) {
        self.current_document_loader_id = None;
    }

    pub(super) fn frame_identity_snapshot(&self) -> ChildBrowsingContextFrameIdentitySnapshot {
        ChildBrowsingContextFrameIdentitySnapshot {
            frame_id: self.frame_id.clone(),
            name: self.name.clone(),
            owner_element_id: self.id.clone(),
            security_origin_inherited: self.security_origin_inherited(),
        }
    }

    fn completed_document_network(&self) -> Option<CompletedChildDocumentNetwork> {
        self.completed_document_network.clone()
    }

    fn completed_document_network_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<CompletedChildDocumentNetwork> {
        if attribute_bootstrap_changed {
            None
        } else {
            self.completed_document_network()
        }
    }

    pub(super) fn bind_completed_document_network(
        &mut self,
        owner: FrameDocumentTaskOwner,
        network: Option<ChildFrameDocumentNetworkSnapshot>,
    ) {
        self.completed_document_network =
            network.map(|snapshot| CompletedChildDocumentNetwork { owner, snapshot });
    }

    pub(super) fn clear_completed_document_network(&mut self) {
        self.completed_document_network = None;
    }

    pub(super) fn take_completed_document_network_for_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ChildFrameDocumentNetworkSnapshot> {
        self.completed_document_network
            .take()
            .filter(|network| network.owner == owner)
            .map(|network| network.snapshot)
    }

    fn completed_frame_owner_resource_timing_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<CompletedFrameOwnerResourceTiming> {
        (!attribute_bootstrap_changed)
            .then(|| self.completed_frame_owner_resource_timing.clone())
            .flatten()
    }

    pub(super) fn bind_completed_frame_owner_resource_timing(
        &mut self,
        timing: Option<CompletedFrameOwnerResourceTiming>,
    ) {
        self.completed_frame_owner_resource_timing = timing;
    }

    pub(super) fn frame_owner_resource_timing_for_owner(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> Option<CompletedFrameOwnerResourceTiming> {
        self.completed_frame_owner_resource_timing
            .as_ref()
            .filter(|timing| timing.child_owner() == owner)
            .cloned()
    }

    pub(super) fn clear_frame_owner_resource_timing_if_owner(
        &mut self,
        owner: FrameDocumentTaskOwner,
    ) {
        if self
            .completed_frame_owner_resource_timing
            .as_ref()
            .is_some_and(|timing| timing.child_owner() == owner)
        {
            self.completed_frame_owner_resource_timing = None;
        }
    }

    pub(super) fn navigation_entry_seed(&self) -> NavigationHistoryEntrySeed {
        self.navigation_entry_seed.clone()
    }

    pub(super) fn committed_navigation_entry_seed(&self) -> NavigationHistoryEntrySeed {
        self.committed_navigation_entry_seed.clone()
    }

    pub(super) fn commit_current_navigation_entry_seed(&mut self) {
        self.committed_navigation_entry_seed = self.navigation_entry_seed.clone();
    }

    pub(super) fn restore_navigation_entry_seed_from_committed(&mut self) {
        self.navigation_entry_seed = self.committed_navigation_entry_seed.clone();
    }

    pub(super) fn rewrite_current_navigation_url_after_load(&mut self, final_url: &Url) {
        let final_url_string = final_url.as_str().to_owned();
        let current_index = self.navigation_entry_seed.current_index;
        if let Some(current_entry) = self
            .navigation_entry_seed
            .entries
            .iter_mut()
            .find(|entry| entry.history_index == current_index)
        {
            current_entry.url = final_url_string.clone();
        }
        if let Some(activation) = self.navigation_entry_seed.activation.as_mut()
            && activation.entry.history_index == current_index
        {
            activation.entry.url = final_url_string;
        }
        self.rewrite_live_url_bootstrap_after_load(final_url);
    }

    pub(super) fn apply_loaded_referrer_policy_to_current_document(
        &mut self,
        policy: Option<String>,
    ) {
        let Some(policy) = policy else {
            return;
        };
        let current_index = self.navigation_entry_seed.current_index;
        let current_document_id = self
            .navigation_entry_seed
            .entries
            .iter()
            .find(|snapshot| snapshot.history_index == current_index)
            .map(|snapshot| snapshot.document_id.clone());
        for snapshot in &mut self.navigation_entry_seed.entries {
            if current_document_id
                .as_ref()
                .is_some_and(|document_id| &snapshot.document_id == document_id)
            {
                snapshot.referrer_policy = Some(policy.clone());
            }
        }
        if let Some(activation) = self.navigation_entry_seed.activation.as_mut()
            && current_document_id
                .as_ref()
                .is_some_and(|document_id| &activation.entry.document_id == document_id)
        {
            activation.entry.referrer_policy = Some(policy);
        }
    }

    pub(super) fn navigation_seed_snapshot(&self) -> ChildBrowsingContextNavigationSeedSnapshot {
        ChildBrowsingContextNavigationSeedSnapshot {
            navigation_entry_seed: self.navigation_entry_seed.clone(),
            committed_navigation_entry_seed: self.committed_navigation_entry_seed.clone(),
            pending_attribute_bootstrap_commit: self.pending_attribute_bootstrap_commit,
            pending_live_navigation_reflects_window_state: self
                .pending_live_navigation_reflects_window_state,
        }
    }

    pub(super) fn restore_navigation_seed_snapshot(
        &mut self,
        snapshot: ChildBrowsingContextNavigationSeedSnapshot,
    ) {
        self.navigation_entry_seed = snapshot.navigation_entry_seed;
        self.committed_navigation_entry_seed = snapshot.committed_navigation_entry_seed;
        self.pending_attribute_bootstrap_commit = snapshot.pending_attribute_bootstrap_commit;
        self.pending_live_navigation_reflects_window_state =
            snapshot.pending_live_navigation_reflects_window_state;
    }

    pub(super) fn set_navigation_entry_seed(
        &mut self,
        entry_seed: NavigationHistoryEntrySeed,
    ) -> bool {
        let committed_current_document_id =
            child_navigation_current_document_id(&self.committed_navigation_entry_seed);
        let next_current_document_id = child_navigation_current_document_id(&entry_seed);
        let same_document_update = committed_current_document_id.is_none()
            || committed_current_document_id == next_current_document_id
            || self.pending_attribute_bootstrap_commit;
        self.navigation_entry_seed = entry_seed.clone();
        if same_document_update {
            self.committed_navigation_entry_seed = entry_seed;
            self.pending_attribute_bootstrap_commit = false;
            self.pending_live_navigation_reflects_window_state = false;
        }
        same_document_update
    }

    pub(super) fn replace_navigation_entry_seed(&mut self, entry_seed: NavigationHistoryEntrySeed) {
        self.navigation_entry_seed = entry_seed;
    }

    pub(super) fn replace_navigation_entry_seed_and_clear_pending_history_increment(
        &mut self,
        entry_seed: NavigationHistoryEntrySeed,
    ) {
        self.replace_navigation_entry_seed(entry_seed);
        self.clear_pending_top_level_history_length_increment();
    }

    pub(super) fn apply_navigation_to_entry_seed(&mut self, url: &Url) {
        apply_child_navigation_to_seed(&mut self.navigation_entry_seed, url, None, None);
    }

    pub(super) fn replace_navigation_in_entry_seed(&mut self, url: &Url) {
        replace_child_navigation_in_seed(&mut self.navigation_entry_seed, url, None, None);
    }

    pub(super) fn apply_javascript_url_navigation_to_entry_seed(&mut self) {
        apply_child_javascript_url_navigation_to_seed(&mut self.navigation_entry_seed);
    }

    pub(super) fn apply_queued_navigation_to_entry_seed(
        &mut self,
        url: &Url,
        replace_current: bool,
    ) {
        if url.scheme() == "javascript" {
            self.apply_javascript_url_navigation_to_entry_seed();
        } else if replace_current {
            self.replace_navigation_in_entry_seed(url);
        } else {
            self.apply_navigation_to_entry_seed(url);
        }
    }

    pub(super) fn apply_deferred_navigation_to_entry_seed(&mut self, url: &Url) {
        self.apply_navigation_to_entry_seed(url);
        self.clear_pending_top_level_history_length_increment();
    }

    pub(super) fn clear_navigation_activation(&mut self) {
        self.navigation_entry_seed.activation = None;
    }

    pub(super) fn navigation_seed_is_initial_about_blank_commit(&self) -> bool {
        self.navigation_entry_seed.current_index == 0
            && self.navigation_entry_seed.entries.len() == 1
            && self.navigation_entry_seed.entries[0].url == "about:blank"
    }

    pub(super) fn navigation_seed_is_initial_attribute_target(&self, url: &Url) -> bool {
        self.navigation_entry_seed.current_index == 1
            && self.navigation_entry_seed.entries.len() == 2
            && self.navigation_entry_seed.entries[0].url == "about:blank"
            && self.navigation_entry_seed.entries[1].url == url.as_str()
    }

    pub(super) fn apply_initial_attribute_target_navigation_entry(&mut self, url: &Url) {
        self.navigation_entry_seed
            .entries
            .push(NavigationHistorySerializedEntry {
                url: url.as_str().to_owned(),
                history_state_json: None,
                navigation_state_json: None,
                referrer_policy: None,
                document_id: NavigationHistoryDocumentId::allocate(),
                history_index: 1,
                index: 0,
                id: NavigationHistoryEntryId::allocate(),
                key: NavigationHistoryEntryKey::allocate(),
            });
        self.navigation_entry_seed.current_index = 1;
        self.mark_initial_attribute_target_navigation_activation();
    }

    pub(super) fn mark_initial_attribute_target_navigation_activation(&mut self) {
        self.navigation_entry_seed.activation = self
            .navigation_entry_seed
            .entries
            .get(1)
            .cloned()
            .map(|activation_entry| NavigationActivationSeed {
                entry: activation_entry,
                from: None,
                navigation_type: Some("replace".to_owned()),
            });
    }

    pub(super) fn pending_navigation_position(&self) -> Option<(u32, u32)> {
        if !self.has_pending_window_state() {
            return None;
        }
        let current_navigation_index =
            child_navigation_current_entry(&self.navigation_entry_seed)?.index;
        let visible_len = self
            .navigation_entry_seed
            .entries
            .iter()
            .map(|snapshot| snapshot.index)
            .max()
            .map_or(0, |index| index + 1);
        Some((current_navigation_index, visible_len))
    }

    pub(super) fn visible_window_navigation_state(
        &self,
    ) -> Option<ChildBrowsingContextVisibleNavigationState> {
        let has_pending_window_state = self.has_pending_window_state();
        let reflects_pending_window_state = self.reflects_pending_window_state();
        let has_uncommitted_navigation = reflects_pending_window_state
            || (!self.pending_attribute_bootstrap_commit
                && !has_pending_window_state
                && child_navigation_current_url(&self.navigation_entry_seed)
                    != child_navigation_current_url(&self.committed_navigation_entry_seed));
        let entry_seed = if has_uncommitted_navigation {
            self.navigation_entry_seed.clone()
        } else {
            self.committed_navigation_entry_seed.clone()
        };
        let href = child_navigation_current_url(&entry_seed)
            .map(str::to_owned)
            .or_else(|| {
                self.cached_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.url.to_string())
            })
            .or_else(|| child_document_url_string_for_bootstrap(&self.live_bootstrap))?;
        Some(ChildBrowsingContextVisibleNavigationState {
            href,
            entry_seed,
            seed_is_committed: !has_uncommitted_navigation,
        })
    }

    pub(super) fn performance_navigation_type(&self) -> &'static str {
        self.committed_navigation_entry_seed
            .activation
            .as_ref()
            .and_then(|activation| activation.navigation_type.as_deref())
            .map(|navigation_type| match navigation_type {
                "reload" => "reload",
                "traverse" => "traverse",
                _ => "navigate",
            })
            .unwrap_or("navigate")
    }

    pub(super) fn performance_time_origin_millis(&self) -> f64 {
        self.performance_time_origin.as_millis()
    }

    pub(super) fn performance_time_origin(&self) -> ChildPerformanceTimeOrigin {
        self.performance_time_origin
    }

    pub(super) fn reset_performance_time_origin(&mut self) {
        self.performance_time_origin = ChildPerformanceTimeOrigin::now();
    }

    pub(super) fn clear_script_execution_state(&mut self) {
        self.clear_child_classic_script_document_state();
    }

    pub(super) fn clear_pending_document_load(&mut self) {
        self.pending_document_load_id = None;
    }

    pub(super) fn mark_pending_document_load(&mut self, load_id: u64) {
        self.pending_document_load_id = Some(load_id);
    }

    pub(super) fn pending_document_load_id(&self) -> Option<u64> {
        self.pending_document_load_id
    }

    pub(super) fn pending_document_load_id_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<u64> {
        (!attribute_bootstrap_changed)
            .then_some(self.pending_document_load_id())
            .flatten()
    }

    pub(super) fn pending_document_load_matches(&self, load_id: u64) -> bool {
        self.pending_document_load_id == Some(load_id)
    }

    pub(super) fn mark_pending_top_level_history_length_increment(&mut self) {
        self.pending_top_level_history_length_increment = true;
    }

    pub(super) fn pending_top_level_history_length_increment(&self) -> bool {
        self.pending_top_level_history_length_increment
    }

    pub(super) fn clear_pending_top_level_history_length_increment(&mut self) {
        self.pending_top_level_history_length_increment = false;
    }

    pub(super) fn take_pending_top_level_history_length_increment(&mut self) -> bool {
        let increment = self.pending_top_level_history_length_increment;
        self.clear_pending_top_level_history_length_increment();
        increment
    }

    pub(super) fn clear_pending_form_submission_navigation(&mut self) {
        self.pending_live_navigation = None;
        self.clear_pending_top_level_history_length_increment();
    }

    pub(super) fn has_pending_live_navigation(&self) -> bool {
        self.pending_live_navigation.is_some()
    }

    pub(super) fn has_pending_window_state(&self) -> bool {
        self.has_pending_live_navigation() || self.pending_document_load_id.is_some()
    }

    pub(super) fn reflects_pending_window_state(&self) -> bool {
        self.pending_live_navigation_reflects_window_state && self.has_pending_window_state()
    }

    pub(super) fn has_pending_navigation_or_document_load(&self) -> bool {
        let waits_initial = self.attribute_navigation_waits_for_initial_about_blank();
        let current_url = child_navigation_current_url(&self.navigation_entry_seed);
        let committed_url = child_navigation_current_url(&self.committed_navigation_entry_seed);
        let pending_attribute_navigation = self.pending_attribute_bootstrap_commit
            && (waits_initial || current_url != committed_url);
        pending_attribute_navigation
            || self.has_pending_window_state()
            || waits_initial
            || current_url != committed_url
    }

    fn attribute_navigation_waits_for_initial_about_blank(&self) -> bool {
        if !matches!(
            self.live_bootstrap,
            ChildBrowsingContextBootstrap::AboutBlank
        ) {
            return false;
        }
        match &self.attribute_bootstrap {
            ChildBrowsingContextBootstrap::Url(url) if url.scheme() == "javascript" => {
                self.pending_attribute_bootstrap_commit
            }
            ChildBrowsingContextBootstrap::Url(url) => url.as_str() != "about:blank",
            ChildBrowsingContextBootstrap::Request(_) => true,
            ChildBrowsingContextBootstrap::AboutBlank
            | ChildBrowsingContextBootstrap::Srcdoc { .. } => false,
        }
    }

    pub(super) fn has_uncommitted_navigation_seed(&self) -> bool {
        self.has_pending_live_navigation()
            || self.pending_attribute_bootstrap_commit
            || child_navigation_current_url(&self.navigation_entry_seed)
                != child_navigation_current_url(&self.committed_navigation_entry_seed)
    }

    pub(super) fn pending_live_navigation(&self) -> Option<ChildBrowsingContextBootstrap> {
        self.pending_live_navigation.clone()
    }

    pub(super) fn pending_live_navigation_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<ChildBrowsingContextBootstrap> {
        (!attribute_bootstrap_changed)
            .then_some(self.pending_live_navigation())
            .flatten()
    }

    pub(super) fn pending_live_navigation_reflects_window_state(&self) -> bool {
        self.pending_live_navigation_reflects_window_state
    }

    pub(super) fn pending_live_navigation_reflects_window_state_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> bool {
        !attribute_bootstrap_changed && self.pending_live_navigation_reflects_window_state()
    }

    pub(super) fn pending_document_navigation_owner_is_current(&self, load_id: u64) -> bool {
        self.pending_document_load_matches(load_id)
            && !self.has_pending_live_navigation()
            && !self.pending_attribute_bootstrap_commit
    }

    pub(super) fn set_pending_navigation(
        &mut self,
        bootstrap: ChildBrowsingContextBootstrap,
        reflects_window_state: bool,
    ) {
        self.pending_attribute_bootstrap_commit = false;
        self.pending_live_navigation = Some(bootstrap);
        self.pending_live_navigation_reflects_window_state = reflects_window_state;
    }

    pub(super) fn clear_pending_navigation(&mut self) {
        self.pending_attribute_bootstrap_commit = false;
        self.pending_live_navigation = None;
        self.pending_live_navigation_reflects_window_state = false;
    }

    pub(super) fn has_pending_javascript_url_navigation(&self) -> bool {
        (self.pending_attribute_bootstrap_commit
            && matches!(
                self.attribute_bootstrap,
                ChildBrowsingContextBootstrap::Url(ref url) if url.scheme() == "javascript"
            ))
            || matches!(
                self.pending_live_navigation,
                Some(ChildBrowsingContextBootstrap::Url(ref url)) if url.scheme() == "javascript"
            )
    }

    pub(super) fn service_worker_client_id(&self) -> Option<ServiceWorkerClientId> {
        self.service_worker_client_id
    }

    pub(super) fn has_service_worker_client_id(&self, client_id: ServiceWorkerClientId) -> bool {
        self.service_worker_client_id == Some(client_id)
    }

    pub(super) fn set_service_worker_client_id(&mut self, client_id: ServiceWorkerClientId) {
        self.service_worker_client_id = Some(client_id);
    }

    pub(super) fn take_service_worker_client_id(&mut self) -> Option<ServiceWorkerClientId> {
        self.service_worker_client_id.take()
    }

    pub(super) fn pending_service_worker_client_id(&self) -> Option<ServiceWorkerClientId> {
        self.pending_service_worker_client_id
    }

    pub(super) fn pending_service_worker_client_id_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<ServiceWorkerClientId> {
        (!attribute_bootstrap_changed)
            .then_some(self.pending_service_worker_client_id())
            .flatten()
    }

    pub(super) fn stale_pending_service_worker_client_id_for_refresh(
        &self,
        attribute_bootstrap_changed: bool,
    ) -> Option<ServiceWorkerClientId> {
        attribute_bootstrap_changed
            .then_some(self.pending_service_worker_client_id())
            .flatten()
    }

    pub(super) fn set_pending_service_worker_client_id(
        &mut self,
        client_id: ServiceWorkerClientId,
    ) {
        self.pending_service_worker_client_id = Some(client_id);
    }

    pub(super) fn take_pending_service_worker_client_id(
        &mut self,
    ) -> Option<ServiceWorkerClientId> {
        self.pending_service_worker_client_id.take()
    }

    pub(super) fn promote_pending_service_worker_client_id(
        &mut self,
    ) -> Option<ServiceWorkerClientId> {
        let pending_client_id = self.take_pending_service_worker_client_id()?;
        if self.service_worker_client_id == Some(pending_client_id) {
            return None;
        }
        self.service_worker_client_id.replace(pending_client_id)
    }

    pub(super) fn set_pending_service_worker_client_navigation(
        &mut self,
        continuation: ServiceWorkerClientNavigateContinuation,
    ) {
        self.pending_service_worker_client_navigation = Some(continuation);
    }

    pub(super) fn pending_service_worker_client_navigation(
        &self,
    ) -> Option<ServiceWorkerClientNavigateContinuation> {
        self.pending_service_worker_client_navigation.clone()
    }

    pub(super) fn take_pending_service_worker_client_navigation(
        &mut self,
    ) -> Option<ServiceWorkerClientNavigateContinuation> {
        self.pending_service_worker_client_navigation.take()
    }

    pub(super) fn take_pending_service_worker_client_navigation_for_current_client(
        &mut self,
    ) -> Option<(
        ServiceWorkerClientId,
        ServiceWorkerClientNavigateContinuation,
    )> {
        Some((
            self.service_worker_client_id()?,
            self.take_pending_service_worker_client_navigation()?,
        ))
    }
}

impl JsContextHost {
    pub(crate) fn pending_live_child_browsing_context_navigation_snapshot(
        &self,
    ) -> Vec<(DomHandle, ChildBrowsingContextBootstrap)> {
        self.child_browsing_contexts
            .iter()
            .filter_map(|(handle, entry)| {
                entry
                    .pending_live_navigation()
                    .map(|pending| (*handle, pending))
            })
            .collect()
    }

    pub(crate) fn child_browsing_context_pending_live_navigation(
        &self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextBootstrap> {
        self.child_browsing_contexts
            .get(&handle)
            .and_then(|entry| entry.pending_live_navigation())
    }

    pub(crate) fn child_browsing_context_has_pending_navigation_or_document_load(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(ChildBrowsingContextEntry::has_pending_navigation_or_document_load)
    }

    pub(crate) fn child_browsing_context_has_pending_javascript_url_navigation(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(ChildBrowsingContextEntry::has_pending_javascript_url_navigation)
    }

    pub(crate) fn cancel_previous_pending_form_submission_child_navigation(
        &mut self,
        form: DomHandle,
        target: DomHandle,
    ) {
        let mut remove_form_entry = false;
        let Some(previous_targets) = self.pending_form_submission_child_targets.get_mut(&form)
        else {
            return;
        };
        if !previous_targets.contains(&target) {
            return;
        }
        previous_targets.retain(|candidate| *candidate != target);
        if previous_targets.is_empty() {
            remove_form_entry = true;
        }
        if remove_form_entry {
            self.pending_form_submission_child_targets.remove(&form);
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&target) {
            entry.clear_pending_form_submission_navigation();
        }
        self.clear_pending_service_worker_child_client(target);
        self.retire_current_child_navigation_commit_task(target);
        self.cancel_child_document_script_work(target);
        self.clear_pending_child_document_loads_for_handle(target);
    }

    pub(crate) fn cancel_pending_form_submission_child_navigations_for_form(
        &mut self,
        form: DomHandle,
    ) {
        let Some(previous_targets) = self.pending_form_submission_child_targets.remove(&form)
        else {
            return;
        };
        for target in previous_targets {
            if let Some(entry) = self.child_browsing_contexts.get_mut(&target) {
                entry.clear_pending_form_submission_navigation();
            }
            self.clear_pending_service_worker_child_client(target);
            self.retire_current_child_navigation_commit_task(target);
            self.cancel_child_document_script_work(target);
            self.clear_pending_child_document_loads_for_handle(target);
        }
    }

    pub(crate) fn mark_pending_form_submission_child_navigation(
        &mut self,
        form: DomHandle,
        target: DomHandle,
    ) {
        let targets = self
            .pending_form_submission_child_targets
            .entry(form)
            .or_default();
        if !targets.contains(&target) {
            targets.push(target);
        }
    }

    pub(crate) fn clear_pending_form_submission_child_target(&mut self, target: DomHandle) {
        self.pending_form_submission_child_targets
            .retain(|_, targets| {
                targets.retain(|candidate| *candidate != target);
                !targets.is_empty()
            });
    }

    pub(crate) fn child_browsing_context_content_security_policies(
        &self,
        handle: DomHandle,
    ) -> Option<&[String]> {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.document_content_security_policies())
    }

    pub(crate) fn child_browsing_context_navigation_seed_snapshot(
        &self,
        handle: DomHandle,
    ) -> Option<ChildBrowsingContextNavigationSeedSnapshot> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        Some(entry.navigation_seed_snapshot())
    }

    pub(crate) fn restore_child_browsing_context_navigation_seed_snapshot(
        &mut self,
        handle: DomHandle,
        snapshot: ChildBrowsingContextNavigationSeedSnapshot,
    ) {
        if let Some(entry) = self.child_browsing_contexts.get_mut(&handle) {
            entry.restore_navigation_seed_snapshot(snapshot);
        }
    }

    pub(crate) fn set_child_browsing_context_name(&mut self, handle: DomHandle, name: String) {
        let Some(entry) = self.child_browsing_contexts.get_mut(&handle) else {
            return;
        };
        entry.set_window_name(name);
    }
}

fn child_navigation_current_url(seed: &NavigationHistoryEntrySeed) -> Option<&str> {
    child_navigation_current_entry(seed).map(|entry| entry.url.as_str())
}

fn child_navigation_current_document_id(seed: &NavigationHistoryEntrySeed) -> Option<&str> {
    child_navigation_current_entry(seed).map(|entry| entry.document_id.as_str())
}

fn child_navigation_current_entry(
    seed: &NavigationHistoryEntrySeed,
) -> Option<&moli_page_types::NavigationHistorySerializedEntry> {
    seed.entries
        .iter()
        .find(|entry| entry.history_index == seed.current_index)
}

fn child_navigation_current_url_as_url(seed: &NavigationHistoryEntrySeed) -> Option<Url> {
    child_navigation_current_url(seed).and_then(|url| Url::parse(url).ok())
}

fn child_document_url_for_bootstrap(bootstrap: &ChildBrowsingContextBootstrap) -> Option<Url> {
    match bootstrap {
        ChildBrowsingContextBootstrap::AboutBlank => Url::parse("about:blank").ok(),
        ChildBrowsingContextBootstrap::Url(url) => Some(url.clone()),
        ChildBrowsingContextBootstrap::Request(request) => Some(request.url.clone()),
        ChildBrowsingContextBootstrap::Srcdoc { .. } => Url::parse("about:srcdoc").ok(),
    }
}

fn child_document_url_string_for_bootstrap(
    bootstrap: &ChildBrowsingContextBootstrap,
) -> Option<String> {
    match bootstrap {
        ChildBrowsingContextBootstrap::AboutBlank => Some("about:blank".to_owned()),
        ChildBrowsingContextBootstrap::Url(url) => Some(url.to_string()),
        ChildBrowsingContextBootstrap::Request(request) => Some(request.url.to_string()),
        ChildBrowsingContextBootstrap::Srcdoc { .. } => Some("about:srcdoc".to_owned()),
    }
}
