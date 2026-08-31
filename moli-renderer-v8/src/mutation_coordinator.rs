use std::time::Instant;

use crate::dom::native::{DomHost, DomMutationEffects, NativeNodeId, ScriptPrepareTriggerKind};
use crate::style_engine::StyleMutationEffect;

use super::{
    host::{
        HostDocumentState, HostEventTargetRegistry, HostScriptScheduler,
        RuntimeScriptStartDecision, ScriptElementLoader, ScriptElementLoaderOptions,
    },
    native_bridge::{self, JsContextHost},
    observer_runtime,
    util::v8str,
};

#[derive(Debug, Default)]
pub(super) struct MutationCoordinator;

pub(super) struct MutationCoordinatorApplyResult {
    pub(super) changed: bool,
    pub(super) runtime_script_start_candidates: Vec<RuntimeScriptStartCandidate>,
    pub(super) removed_open_popovers: Vec<NativeNodeId>,
    pub(super) changed_slots: Vec<NativeNodeId>,
}

#[derive(Debug)]
pub(super) struct RuntimeScriptStartCandidate {
    node: NativeNodeId,
    host_script_handle: String,
}

impl RuntimeScriptStartCandidate {
    pub(super) fn into_parts(self) -> (NativeNodeId, String) {
        (self.node, self.host_script_handle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DomMutationSource {
    JsDomApi,
    ParserTreeSink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectedScriptMutationPolicy {
    PrepareAndStart,
    DeferToOwner,
    DoNotPrepare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RuntimeMutationOptions {
    pub(crate) source: DomMutationSource,
    pub(crate) connected_script_policy: ConnectedScriptMutationPolicy,
    pub(crate) hide_nonce_content_attributes: bool,
    pub(crate) dispatch_atomic_move_callbacks: bool,
    pub(crate) parser_created: bool,
    pub(crate) check_inline_style_csp: bool,
    pub(crate) defer_document_followups_to_parser_owner: bool,
}

impl RuntimeMutationOptions {
    pub(crate) const fn js_dom_api() -> Self {
        Self {
            source: DomMutationSource::JsDomApi,
            connected_script_policy: ConnectedScriptMutationPolicy::PrepareAndStart,
            hide_nonce_content_attributes: true,
            dispatch_atomic_move_callbacks: false,
            parser_created: false,
            check_inline_style_csp: true,
            defer_document_followups_to_parser_owner: false,
        }
    }

    pub(crate) const fn parser_tree_sink() -> Self {
        Self {
            source: DomMutationSource::ParserTreeSink,
            connected_script_policy: ConnectedScriptMutationPolicy::DeferToOwner,
            hide_nonce_content_attributes: false,
            dispatch_atomic_move_callbacks: false,
            parser_created: true,
            check_inline_style_csp: true,
            defer_document_followups_to_parser_owner: false,
        }
    }

    pub(crate) const fn child_parser_tree_sink() -> Self {
        Self {
            source: DomMutationSource::ParserTreeSink,
            connected_script_policy: ConnectedScriptMutationPolicy::DeferToOwner,
            hide_nonce_content_attributes: false,
            dispatch_atomic_move_callbacks: false,
            parser_created: true,
            check_inline_style_csp: false,
            defer_document_followups_to_parser_owner: true,
        }
    }

    pub(crate) const fn with_connected_script_policy(
        mut self,
        policy: ConnectedScriptMutationPolicy,
    ) -> Self {
        self.connected_script_policy = policy;
        self
    }

    pub(crate) const fn with_nonce_hiding(mut self, hide: bool) -> Self {
        self.hide_nonce_content_attributes = hide;
        self
    }

    pub(crate) const fn with_atomic_move_callbacks(mut self, dispatch: bool) -> Self {
        self.dispatch_atomic_move_callbacks = dispatch;
        self
    }

    pub(crate) const fn with_inline_style_csp_check(mut self, check: bool) -> Self {
        self.check_inline_style_csp = check;
        self
    }

    pub(crate) const fn prepares_connected_scripts(self) -> bool {
        matches!(
            self.connected_script_policy,
            ConnectedScriptMutationPolicy::PrepareAndStart
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ScriptStartRequest {
    handle: NativeNodeId,
    clears_force_async: bool,
}

impl ScriptStartRequest {
    pub(super) fn handle(&self) -> NativeNodeId {
        self.handle
    }

    pub(super) fn clears_force_async(&self) -> bool {
        self.clears_force_async
    }
}

#[derive(Debug, Default)]
struct ScriptStartRequests {
    requests: Vec<ScriptStartRequest>,
}

impl ScriptStartRequests {
    fn queue(&mut self, handle: NativeNodeId, clears_force_async: bool) {
        if let Some(existing) = self
            .requests
            .iter_mut()
            .find(|request| request.handle == handle)
        {
            existing.clears_force_async |= clears_force_async;
            return;
        }
        self.requests.push(ScriptStartRequest {
            handle,
            clears_force_async,
        });
    }

    fn into_tree_order(mut self, dom_host: &DomHost) -> Vec<ScriptStartRequest> {
        if self.requests.len() <= 1 {
            // Tree order is only meaningful when one mutation prepares multiple
            // scripts. Dynamic script insertion usually queues one handle; do
            // not walk the whole document for that common case.
            return self.requests;
        }

        let mut ordered = Vec::with_capacity(self.requests.len());
        collect_pending_script_start_requests_in_tree_order(
            dom_host,
            dom_host.document_handle(),
            &mut self.requests,
            &mut ordered,
        );
        ordered.extend(self.requests);
        ordered
    }
}

impl MutationCoordinator {
    pub(super) fn apply(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        dom_host: &mut DomHost,
        document: &HostDocumentState,
        _scripts: &mut HostScriptScheduler,
        _events: &mut HostEventTargetRegistry,
        effects: DomMutationEffects,
        options: RuntimeMutationOptions,
    ) -> MutationCoordinatorApplyResult {
        if !effects.did_change() {
            return MutationCoordinatorApplyResult {
                changed: false,
                runtime_script_start_candidates: Vec::new(),
                removed_open_popovers: Vec::new(),
                changed_slots: Vec::new(),
            };
        }
        if options.source == DomMutationSource::JsDomApi {
            Self::note_script_children_changed_by_api(dom_host, &effects);
        }
        unsafe { &mut *host_ptr }.note_app_manifest_link_mutation(dom_host, &effects);
        let cpu_profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = cpu_profile_enabled.then(Instant::now);
        tracing::trace!(
            target: "moli_dom_mutation_owner",
            source = ?options.source,
            connected_script_policy = ?options.connected_script_policy,
            parser_created = options.parser_created,
            hide_nonce_content_attributes = options.hide_nonce_content_attributes,
            dispatch_atomic_move_callbacks = options.dispatch_atomic_move_callbacks,
            "applying runtime mutation effects"
        );
        let style_effects_started = cpu_profile_enabled.then(Instant::now);
        let style_effects = StyleMutationEffect::from_dom_mutation_effects(dom_host, &effects);
        let style_effect_count = style_effects.len();
        let style_effects_us = style_effects_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let style_invalidation_started = cpu_profile_enabled.then(Instant::now);
        if !style_effects.is_empty() {
            unsafe { &mut *host_ptr }.note_style_mutation_effects(&style_effects);
        }
        let style_invalidation_us = style_invalidation_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();

        let timing_started = moli_trace::cdp_nav_timing_enabled().then(Instant::now);
        let prepare_connected_scripts = options.prepares_connected_scripts();
        let connected_script_root_count = if prepare_connected_scripts {
            effects.scripts().connected_roots().len()
        } else {
            0
        };
        let script_prepare_trigger_count = if prepare_connected_scripts {
            effects.scripts().prepare_triggers().len()
        } else {
            0
        };
        let mut script_prepare_connected_count = 0usize;
        let mut script_prepare_child_insertion_count = 0usize;
        let mut script_prepare_source_attribute_count = 0usize;
        let mut script_prepare_async_attribute_count = 0usize;
        if timing_started.is_some() && prepare_connected_scripts {
            for trigger in effects.scripts().prepare_triggers() {
                match trigger.kind() {
                    ScriptPrepareTriggerKind::Connected => {
                        script_prepare_connected_count += 1;
                    }
                    ScriptPrepareTriggerKind::ChildInsertion => {
                        script_prepare_child_insertion_count += 1;
                    }
                    ScriptPrepareTriggerKind::SourceAttributeAdded => {
                        script_prepare_source_attribute_count += 1;
                    }
                    ScriptPrepareTriggerKind::AsyncAttributeAdded => {
                        script_prepare_async_attribute_count += 1;
                    }
                }
            }
        }
        let mutation_record_count = effects.observer_records().records().len();
        let script_planning_started = cpu_profile_enabled.then(Instant::now);
        let script_start_requests = if prepare_connected_scripts {
            self.plan_connected_script_start_requests(dom_host, &effects)
        } else {
            Vec::new()
        };
        let script_start_request_count = script_start_requests.len();
        let script_planning_us = script_planning_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let observer_started = cpu_profile_enabled.then(Instant::now);
        observer_runtime::queue_mutation_records(scope, host_ptr, dom_host, &effects);
        let observer_us = observer_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();

        let script_start_started = cpu_profile_enabled.then(Instant::now);
        let mut runtime_script_start_candidates = Vec::new();
        for request in script_start_requests {
            if request.clears_force_async {
                let _ = dom_host.set_script_force_async(request.handle, false);
            }
            if let Some(candidate) = self.collect_connected_script_start_candidate(
                scope,
                host_ptr,
                dom_host,
                request.handle,
                document,
            ) {
                runtime_script_start_candidates.push(candidate);
            }
        }
        let script_start_us = script_start_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        if let Some(started) = timing_started
            && script_start_request_count > 0
        {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                connected_script_root_count,
                script_prepare_trigger_count,
                script_prepare_connected_count,
                script_prepare_child_insertion_count,
                script_prepare_source_attribute_count,
                script_prepare_async_attribute_count,
                script_start_request_count,
                mutation_record_count,
                elapsed_ms = started.elapsed().as_millis(),
                elapsed_us = started.elapsed().as_micros(),
                stage = "mutation_script_start_requests_done",
            );
        }

        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "mutation_coordinator_apply",
                    style_effect_count,
                    mutation_record_count,
                    script_start_request_count,
                    style_effects_us,
                    style_invalidation_us,
                    script_planning_us,
                    observer_us,
                    script_start_us,
                    total_us,
                );
            }
        }

        MutationCoordinatorApplyResult {
            changed: true,
            runtime_script_start_candidates,
            removed_open_popovers: effects.tree().removed_open_popovers().to_vec(),
            changed_slots: effects.slots().changed_slots().to_vec(),
        }
    }

    fn note_script_children_changed_by_api(dom_host: &mut DomHost, effects: &DomMutationEffects) {
        let mut scripts = Vec::new();
        for mutation in effects.style().child_list_mutations() {
            let target = mutation.target();
            if dom_host
                .node(target)
                .is_some_and(crate::dom::native::Node::is_script_element)
                && !scripts.contains(&target)
            {
                scripts.push(target);
            }
        }
        for &target in effects.style().character_data_mutations() {
            let Some(parent) = dom_host
                .node(target)
                .and_then(crate::dom::native::Node::parent_node)
            else {
                continue;
            };
            if dom_host
                .node(parent)
                .is_some_and(crate::dom::native::Node::is_script_element)
                && !scripts.contains(&parent)
            {
                scripts.push(parent);
            }
        }
        for script in scripts {
            let _ = dom_host.note_script_children_changed_by_api(script);
        }
    }

    fn collect_connected_scripts_in_subtree(
        &self,
        dom_host: &mut DomHost,
        root: NativeNodeId,
        requests: &mut ScriptStartRequests,
    ) {
        for script_handle in dom_host.connected_script_handles(root) {
            requests.queue(script_handle, false);
        }
    }

    pub(super) fn plan_connected_script_start_requests(
        &self,
        dom_host: &mut DomHost,
        effects: &DomMutationEffects,
    ) -> Vec<ScriptStartRequest> {
        let mut requests = ScriptStartRequests::default();
        for &root in effects.scripts().connected_roots() {
            self.collect_connected_scripts_in_subtree(dom_host, root, &mut requests);
        }
        for trigger in effects.scripts().prepare_triggers() {
            requests.queue(trigger.handle(), trigger.clears_script_force_async());
        }
        requests.into_tree_order(dom_host)
    }

    pub(super) fn collect_connected_script_start_candidate(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        dom_host: &mut DomHost,
        node: NativeNodeId,
        document: &HostDocumentState,
    ) -> Option<RuntimeScriptStartCandidate> {
        if !dom_host
            .node(node)
            .is_some_and(crate::dom::native::Node::is_script_element)
        {
            return None;
        }
        let owner_document_handle = dom_host.owner_document_handle(node)?;
        if owner_document_handle != dom_host.document_handle() {
            self.start_connected_child_document_script(
                scope,
                host_ptr,
                dom_host,
                node,
                owner_document_handle,
                document,
            );
            return None;
        }
        let wrapper = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, node)?;

        let host_script_handle =
            native_bridge::object_string_property(scope, wrapper, "__moliHandle").unwrap_or_else(
                || {
                    let handle = format!("dynamic-script-native-{}", node.index());
                    if let Some(value) = v8::String::new(scope, &handle) {
                        let key = v8str(scope, "__moliHandle");
                        let _ = wrapper.define_own_property(
                            scope,
                            key.into(),
                            value.into(),
                            v8::PropertyAttribute::DONT_ENUM,
                        );
                    }
                    handle
                },
            );
        Some(RuntimeScriptStartCandidate {
            node,
            host_script_handle,
        })
    }

    fn start_connected_child_document_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        dom_host: &mut DomHost,
        node: NativeNodeId,
        owner_document_handle: NativeNodeId,
        document: &HostDocumentState,
    ) {
        let (preparation, decision) = ScriptElementLoader::prepare(
            dom_host,
            document,
            node,
            ScriptElementLoaderOptions::with_scripting_enabled(
                unsafe { &*host_ptr }.document_scripting_enabled(owner_document_handle),
            ),
        )
        .into_parts();
        match decision {
            RuntimeScriptStartDecision::Skip { commit_start, .. } => {
                if commit_start {
                    let _ = dom_host.set_script_already_started(node, true);
                }
            }
            RuntimeScriptStartDecision::ExecuteInlineClassic { source } => {
                if unsafe { &mut *host_ptr }
                    .queue_child_dynamic_inline_classic_script_for_current_document(
                        scope,
                        owner_document_handle,
                        node,
                        source,
                    )
                {
                    let _ = dom_host.set_script_already_started(node, true);
                }
            }
            RuntimeScriptStartDecision::Queue {
                source,
                kind,
                mode,
                source_kind,
            } => {
                match unsafe { &mut *host_ptr }
                    .queue_child_dynamic_external_classic_script_for_current_document(
                        scope,
                        owner_document_handle,
                        node,
                        &preparation,
                        &source,
                        kind,
                        mode,
                        source_kind,
                    ) {
                    Ok(true) => {}
                    Ok(false) => tracing::debug!(
                        node = ?node,
                        owner_document_handle = ?owner_document_handle,
                        kind = ?kind,
                        mode = ?mode,
                        source_kind = ?source_kind,
                        "child runtime script has no supported current-document queue"
                    ),
                    Err(error) => tracing::warn!(
                        node = ?node,
                        owner_document_handle = ?owner_document_handle,
                        %error,
                        "failed to prepare child runtime external classic script"
                    ),
                }
            }
            RuntimeScriptStartDecision::RegisterImportMap { .. }
            | RuntimeScriptStartDecision::RejectExternalImportMap
            | RuntimeScriptStartDecision::QueueFailed { .. } => {}
        }
    }
}

fn collect_pending_script_start_requests_in_tree_order(
    dom_host: &DomHost,
    root: NativeNodeId,
    pending: &mut Vec<ScriptStartRequest>,
    ordered: &mut Vec<ScriptStartRequest>,
) {
    if pending.is_empty() {
        return;
    }
    if let Some(index) = pending.iter().position(|request| request.handle == root) {
        ordered.push(pending.remove(index));
        if pending.is_empty() {
            return;
        }
    }
    let mut child = dom_host.node(root).and_then(|node| node.first_child());
    while let Some(handle) = child {
        collect_pending_script_start_requests_in_tree_order(dom_host, handle, pending, ordered);
        if pending.is_empty() {
            return;
        }
        child = dom_host.node(handle).and_then(|node| node.next_sibling());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;
    use url::Url;

    fn test_url() -> Url {
        Url::parse("https://mutation-coordinator.test/").expect("valid test url")
    }

    #[test]
    fn script_start_requests_deduplicate_and_merge_force_async_clear() {
        let handle = NativeNodeId::new(42);
        let mut requests = ScriptStartRequests::default();

        requests.queue(handle, false);
        requests.queue(handle, true);
        requests.queue(handle, false);

        assert_eq!(requests.requests.len(), 1);
        assert!(
            requests
                .requests
                .first()
                .is_some_and(|request| request.clears_force_async)
        );
    }

    #[test]
    fn script_start_requests_emit_connected_handles_in_tree_order() {
        let mut host = DomHost::from_dom(NativeDom::new_html(test_url()));
        let document = host.document_handle();
        let first = host.create_element("script");
        let wrapper = host.create_element("div");
        let second = host.create_element("script");
        let disconnected = host.create_element("script");

        assert!(host.append_child(document, first));
        assert!(host.append_child(document, wrapper));
        assert!(host.append_child(wrapper, second));

        let mut requests = ScriptStartRequests::default();
        requests.queue(second, false);
        requests.queue(disconnected, true);
        requests.queue(first, true);

        let ordered = requests.into_tree_order(&host);
        let handles = ordered
            .iter()
            .map(|request| request.handle)
            .collect::<Vec<_>>();

        assert_eq!(handles, vec![first, second, disconnected]);
        assert!(ordered[0].clears_force_async);
        assert!(!ordered[1].clears_force_async);
        assert!(ordered[2].clears_force_async);
    }
}
