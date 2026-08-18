use super::*;
use crate::module_script_continuation::ModuleScriptContinuationStore;
use crate::page_task_queue::PostParsePageOwnedWork;

mod document_processing;
mod post_parse_owner;
#[cfg(test)]
use post_parse_owner::PostParsePageTaskPopBlocker;

const IMAGE_PRIORITY_BOOST_TARGET: usize = 5;
const SMALL_IMAGE_MAX_AREA: f64 = 10_000.0;

fn image_dimension_for_priority_boost(value: Option<&str>) -> Option<f64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let dimension = value.parse::<f64>().ok()?;
    (dimension.is_finite() && dimension >= 0.0).then_some(dimension)
}

fn image_is_small_for_priority_boost(element: &crate::dom::native::Element) -> bool {
    // Mirrors Chromium's ResourceFetcher small-image test for the first-N image
    // priority boost. If both dimensions are known, <= 10000px^2 is small. If
    // only one known dimension is zero, it is also small. Unknown dimensions are
    // intentionally treated as non-small so they remain eligible for the
    // first-N boost, matching Chromium's preload-scanner behavior.
    let width = image_dimension_for_priority_boost(element.attribute("width"));
    let height = image_dimension_for_priority_boost(element.attribute("height"));
    if let (Some(width), Some(height)) = (width, height) {
        return width * height <= SMALL_IMAGE_MAX_AREA;
    }
    width == Some(0.0) || height == Some(0.0)
}

// Document lifecycle state stays owned by `DocumentRuntime`; this module groups
// the host document state accessors, current-script/parser visibility
// bookkeeping, and pending resource-load delivery that feed DCL/load decisions.
impl DocumentRuntime {
    pub(crate) fn bind_main_document_script_preload_store(
        &mut self,
        store: crate::runtime::DocumentScriptPreloadStore,
    ) {
        self.main_document_script_preloads = store;
    }

    pub(crate) fn bind_main_document_runtime_producer(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        self.script_lifecycle
            .scripts_mut()
            .bind_main_document_runtime_producer(owner)
    }

    pub(crate) fn has_main_document_runtime_route(&self) -> bool {
        self.script_lifecycle
            .scripts()
            .has_main_document_runtime_route()
    }

    pub(crate) fn bind_stylesheet_task_producer(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        let sender = self.stylesheet_lifecycle.task_sender.clone();
        let producer = sender.bind_producer(owner);
        let completion_producer = producer.clone();
        self.stylesheet_lifecycle
            .fetches
            .set_completion_publisher(Some(Arc::new(move |completion| {
                let _ = completion_producer.send_blocking_completion(completion);
            })));
        self.stylesheet_lifecycle.task_producer = Some(producer);
    }

    pub(crate) fn enqueue_main_document_completion_recheck(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        self.script_lifecycle
            .scripts_mut()
            .enqueue_main_document_completion_recheck(owner)
    }

    pub(crate) fn begin_main_document_completion_recheck_turn(&mut self) {
        self.script_lifecycle
            .scripts_mut()
            .begin_main_document_completion_recheck_turn();
    }

    /// Publish one explicit lifecycle payload through the production
    /// main-Document runtime producer for selected-task tests.
    ///
    /// This helper only replaces the page algorithm that creates the payload;
    /// admission, exact-owner stamping, source selection, body execution and
    /// task completion remain the production path.
    #[cfg(test)]
    pub(crate) fn enqueue_main_document_runtime_lifecycle_work_for_test(
        &mut self,
        work: PostParseLifecycleWork,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .enqueue_post_parse_lifecycle_work(work);
    }

    pub(super) fn stylesheet_fetcher(
        &self,
    ) -> crate::stylesheet_blocking::RendererStylesheetFetcher {
        let authority = self
            .current_document_resource_loader()
            .expect("stylesheet fetch requires its committed Document authority");
        crate::stylesheet_blocking::RendererStylesheetFetcher::new(
            authority.request_client().clone(),
            authority.task_runner(),
            self.stylesheet_service_worker_fetch_context(),
        )
    }

    pub(super) fn speculative_stylesheet_fetcher(
        &self,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> crate::stylesheet_blocking::RendererStylesheetFetcher {
        let authority = self
            .current_document_resource_loader()
            .expect("stylesheet preload requires its committed Document authority");
        crate::stylesheet_blocking::RendererStylesheetFetcher::for_speculative_preload(
            authority.request_client().clone(),
            authority.task_runner(),
            self.stylesheet_service_worker_fetch_context(),
            request_resource_type,
            link_preload,
        )
    }

    fn stylesheet_service_worker_fetch_context(
        &self,
    ) -> Option<crate::stylesheet_blocking::ServiceWorkerStylesheetFetchContext> {
        self.stylesheet_lifecycle
            .service_worker_connected_link_context
            .as_ref()
            .map(
                |context| crate::stylesheet_blocking::ServiceWorkerStylesheetFetchContext {
                    browser_context_runtime: context.browser_context_runtime.clone(),
                    client_id: context.client_id,
                },
            )
    }

    pub(crate) fn take_pending_devtools_dom_mutations(
        &mut self,
    ) -> Vec<super::devtools_mutations::DevToolsDomMutationFact> {
        std::mem::take(&mut self.pending_devtools_dom_mutations)
    }

    pub(super) fn queue_devtools_dom_mutations(
        &mut self,
        mutations: Vec<super::devtools_mutations::DevToolsDomMutationFact>,
    ) {
        if mutations.is_empty() {
            return;
        }
        self.pending_devtools_dom_mutations.extend(mutations);
    }
    pub(crate) fn has_all_blocking_stylesheets_resolved(&self) -> bool {
        !self.stylesheet_lifecycle.fetches.has_any_pending_entries()
    }

    pub(crate) fn set_cookie_store(
        &mut self,
        cookie_store: moli_cookie_jar::SharedBrowserCookieStore,
    ) {
        self.document.set_cookie_store(cookie_store);
    }

    pub(crate) fn clear_cookie_store(&mut self) {
        self.document.clear_cookie_store();
    }

    pub(crate) fn apply_document_cookie_facade_overrides(
        &mut self,
        overrides: &moli_cookie_jar::BrowserCookieFacadeOverrides,
    ) {
        self.document.apply_cookie_facade_overrides(overrides);
    }

    pub(crate) fn clear_document_cookie_facade_overrides(&mut self) {
        self.document.clear_cookie_facade_overrides();
    }

    pub(crate) fn document_cookie_telemetry_snapshot(
        &self,
    ) -> crate::DocumentCookieFacadeTelemetrySnapshot {
        self.document.document_cookie_telemetry_snapshot()
    }

    pub(crate) fn document_cookie_owner_snapshot(&self) -> crate::DocumentCookieOwnerSnapshot {
        self.document.document_cookie_owner_snapshot()
    }

    pub(crate) fn open_document(&mut self) {
        // Production runtimes replace this temporary standalone identity with
        // the exact new Frame owner in the surrounding owner transaction.
        // Rotating first invalidates parser work from the retired Document
        // even while that transaction is still being committed.
        self.document_incarnation = DocumentRuntimeIncarnationIdentity::standalone();
        // Blink's Document::ImplicitOpen removes every current child and leaves
        // the Document empty. The parser creates html/head/body only when input
        // is subsequently consumed (or when it is closed with empty input).
        // The V8-facing owner performs the observable all-children mutation
        // before entering this state; this clear is also needed by direct
        // runtime callers that do not own a V8 mutation surface.
        self.dom_host.clear_document_contents();
        self.dom_host
            .set_html_quirks_mode_for_parser(html5ever::tree_builder::QuirksMode::NoQuirks);
        self.document.open_document();
        self.prepare_top_level_meta_refresh_for_document_open();
        self.reset_document_owned_stylesheet_lifecycle();
        self.reset_main_parser_continuation_for_document_replacement();
        self.in_document_image_priority_boost_count = 0;
        self.parser_discovered_modulepreloads.clear();
        self.modulepreload_invalid_as_link_errors.clear();
        // Runtime binding calls are historical Page observations accepted at
        // the invoking realm boundary. `document.open()` replaces the
        // Document but does not undo a call that already happened; the exact
        // Page attachment is validated later by the protocol output route.
        // Same-Document history mutation has already taken effect before
        // `document.open()`. Keep its protocol handoff queued with the exact
        // source Document that produced it; replacing the Document shell does
        // not undo the browsing context's URL or session-history mutation.
        self.script_lifecycle.clear_for_document_replacement();
        self.post_parse_schedule_invalidated = true;
        self.dom_content_loaded_dispatched = false;
        self.pending_inspector_issues.clear();
        self.quirks_mode_issue_reported = false;
        self.document_write_script_preload_scanner = None;
        self.main_document_script_preloads = Default::default();
        self.document_write_script_preloads.clear();
        self.pending_document_write_external_script_load = None;
        self.pending_document_write_stylesheet_blocked_script = None;
        self.pending_document_write_stylesheet_parser_pause = None;
        self.root_document_parser = None;
        self.delivered_meta_content_security_policies
            .get_mut()
            .clear();
        self.processed_meta_content_security_policy_handles
            .get_mut()
            .clear();
        self.document_input_stream_opened = true;
        self.set_document_ready_state(DocumentReadyState::Loading);
    }

    fn reset_document_owned_stylesheet_lifecycle(&mut self) {
        let task_sender = self.stylesheet_lifecycle.task_sender.clone();
        let service_worker_connected_link_context = self
            .stylesheet_lifecycle
            .service_worker_connected_link_context
            .clone();
        #[cfg(test)]
        let task_test_residence = self.stylesheet_lifecycle.task_test_residence.take();
        self.stylesheet_lifecycle = StylesheetLifecycleState::new(task_sender);
        self.stylesheet_lifecycle
            .service_worker_connected_link_context = service_worker_connected_link_context;
        #[cfg(test)]
        {
            self.stylesheet_lifecycle.task_test_residence = task_test_residence;
            if self.stylesheet_lifecycle.task_test_residence.is_some() {
                self.bind_stylesheet_task_producer(
                    super::runtime_core::test_stylesheet_document_owner(),
                );
            }
        }
        self.pending_stylesheet_source_css_projection_owners.clear();
        self.pending_connected_style_load_prime_result = ConnectedStyleLoadPrimeResult::default();
        self.initial_connected_style_loads_queued = false;
        self.late_preload_stylesheet_handles.clear();
    }

    pub(crate) fn bind_main_document_task_owner(&mut self, owner: FrameDocumentTaskOwner) {
        self.document_incarnation = DocumentRuntimeIncarnationIdentity::MainFrame(owner);
    }

    pub(crate) fn parser_module_scripts(&self) -> &ModuleScriptContinuationStore {
        self.script_lifecycle.parser_module_scripts()
    }

    pub(crate) fn parser_module_scripts_mut(&mut self) -> &mut ModuleScriptContinuationStore {
        self.script_lifecycle.parser_module_scripts_mut()
    }

    pub(crate) fn parser_module_document_scripts(
        &self,
    ) -> &crate::module_script_continuation::MainDocumentScriptSchedulerStore {
        self.script_lifecycle.parser_module_document_scripts()
    }

    pub(crate) fn parser_module_document_scripts_mut(
        &mut self,
    ) -> &mut crate::module_script_continuation::MainDocumentScriptSchedulerStore {
        self.script_lifecycle.parser_module_document_scripts_mut()
    }

    pub(crate) fn arm_main_parser_deferred_scripts(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        self.script_lifecycle
            .arm_main_parser_deferred_scripts(owner);
    }

    pub(crate) fn main_parser_deferred_scripts_owner(
        &self,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        self.script_lifecycle.main_parser_deferred_scripts_owner()
    }

    pub(crate) fn disarm_main_parser_deferred_scripts(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        self.script_lifecycle
            .disarm_main_parser_deferred_scripts(owner);
    }

    pub(crate) fn note_dom_content_loaded_dispatched(&mut self) {
        self.dom_content_loaded_dispatched = true;
    }

    pub(crate) fn dom_content_loaded_dispatched(&self) -> bool {
        self.dom_content_loaded_dispatched
    }

    /// Whether parser-owned state still prevents a pending replacement
    /// admission from becoming executable.
    ///
    /// This is deliberately not lifecycle admission state. The lifecycle
    /// journal owns the exact `Pending -> Active` admission, while the
    /// document runtime owns only the input stream and parser-script source
    /// facts that can block it.
    pub(crate) fn document_replacement_parser_is_blocked(&self) -> bool {
        self.document.replace_on_close() || self.has_pending_document_write_parser_blocking_work()
    }

    pub(crate) fn document_input_stream_opened(&self) -> bool {
        self.document_input_stream_opened
    }

    pub(crate) fn document_url(&self) -> &Url {
        self.document.url()
    }

    pub(crate) fn claim_main_image_priority_boost(&mut self, handle: DomHandle) -> bool {
        let eligible = self.dom_host.is_connected(handle)
            && self.dom_host.owner_document_handle(handle) == Some(self.dom_host.document_handle())
            && self
                .dom_host
                .node(handle)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| {
                    element.is_html_element("img") && !image_is_small_for_priority_boost(element)
                });
        if !eligible || self.in_document_image_priority_boost_count >= IMAGE_PRIORITY_BOOST_TARGET {
            return false;
        }
        self.in_document_image_priority_boost_count += 1;
        true
    }

    pub(crate) fn set_document_url(
        &mut self,
        url: Url,
    ) -> Option<(Option<DomHandle>, Option<DomHandle>)> {
        let document_handle = self.dom_host.document_handle();
        let previous_target = self.dom_host.document_target_element(document_handle);
        self.document.set_url(url.clone());
        let changed = self.dom_host.set_document_url(url);
        let next_target = self.dom_host.document_target_element(document_handle);
        (changed && previous_target != next_target).then_some((previous_target, next_target))
    }

    pub(crate) fn set_document_ready_state(&mut self, state: DocumentReadyState) {
        self.document.set_ready_state(state);
        let _ = self.dom_host.set_document_ready_state(state);
    }

    pub(crate) fn host_document(&self) -> &HostDocumentState {
        &self.document
    }

    pub(crate) fn host_document_mut(&mut self) -> &mut HostDocumentState {
        &mut self.document
    }

    fn current_script_context(&self) -> Option<&CurrentScriptContext> {
        self.script_context_stack.last()
    }

    pub(crate) fn current_script_handle(&self) -> Option<DomHandle> {
        self.current_script_context()
            .and_then(|context| context.handle)
            .filter(|handle| self.dom_host.node(*handle).is_some())
    }

    fn push_current_script_context(&mut self, spec: CurrentScriptContextSpec) {
        let CurrentScriptContextSpec {
            handle,
            parser_write_insertion_point_active,
            parser_insertion_controller,
        } = spec;
        let handle = handle.filter(|node| self.dom_host.node(*node).is_some());
        let parser_insertion_controller = parser_insertion_controller
            .or_else(|| {
                self.current_script_context().and_then(|context| {
                    context
                        .parser_connected
                        .as_ref()
                        .map(|parser| parser.insertion_controller.clone())
                })
            })
            .filter(|_| parser_write_insertion_point_active);
        let parser_connected = parser_insertion_controller.map(|insertion_controller| {
            let input_context = insertion_controller.input_session().enter_pending_context();
            ParserConnectedScriptContext {
                insertion_controller,
                _input_context: input_context,
            }
        });
        self.script_context_stack.push(CurrentScriptContext {
            handle,
            parser_connected,
        });
    }

    pub(crate) fn set_current_script_context(&mut self, spec: CurrentScriptContextSpec) {
        self.push_current_script_context(spec);
    }

    pub(crate) fn clear_current_script_handle(&mut self) {
        self.script_context_stack.pop();
    }

    pub(crate) fn has_active_parser_write_insertion_point(&self) -> bool {
        self.current_script_context()
            .is_some_and(|context| context.parser_connected.is_some())
    }

    pub(super) fn current_parser_insertion_controller(&self) -> Option<ParserInsertionController> {
        self.current_script_context()
            .and_then(|context| context.parser_connected.as_ref())
            .map(|context| context.insertion_controller.clone())
    }

    pub(crate) fn take_post_parse_schedule_invalidated(&mut self) -> bool {
        let invalidated = self.post_parse_schedule_invalidated;
        self.post_parse_schedule_invalidated = false;
        invalidated
    }

    pub(crate) fn take_post_parse_schedule_rebuild(&mut self) -> bool {
        self.take_post_parse_schedule_invalidated()
    }

    pub(crate) fn queue_current_main_document_image_load_events(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let handles = (0..self.dom_host.dom().nodes().len())
            .map(crate::document_runtime::DomHandle::new)
            .collect::<Vec<_>>();
        for handle in handles {
            let Some(node) = self.dom_host.node(handle) else {
                continue;
            };
            if !node.is_connected()
                || node.owner_document() != Some(self.dom_host.document_handle())
            {
                continue;
            }
            let Some(element) = node.as_element() else {
                continue;
            };
            if !element.is_html_element("img") || element.image_load_dispatched() {
                continue;
            }
            native_bridge::element::queue_image_load_event_if_needed_with_initiator(
                scope,
                host_ptr,
                handle,
                crate::types::SubresourceRequestInitiatorType::Parser,
            );
        }
    }

    pub(crate) fn queue_current_main_document_media_loads(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) {
        let handles = (0..self.dom_host.dom().nodes().len())
            .map(DomHandle::new)
            .collect::<Vec<_>>();
        for handle in handles {
            let Some(node) = self.dom_host.node(handle) else {
                continue;
            };
            if !node.is_connected()
                || node.owner_document() != Some(self.dom_host.document_handle())
            {
                continue;
            }
            let Some(element) = node.as_element() else {
                continue;
            };
            if element.is_html_element("audio") || element.is_html_element("video") {
                native_bridge::element::queue_media_load_if_needed(scope, host_ptr, handle);
            } else if element.is_html_element("track") {
                native_bridge::element::queue_default_text_track_mode_if_needed(
                    scope, host_ptr, handle,
                );
                native_bridge::element::queue_text_track_load_if_needed(scope, host_ptr, handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::{cell::RefCell, rc::Rc};

    use url::Url;

    use crate::module_runtime::{ModuleKind, ModuleMapKey};
    use crate::{
        dom::{
            NodeId,
            native::{DocumentReadyState, NativeDom, Node},
        },
        parser::HtmlParser,
        {
            document_runtime::{
                DocumentProcessingAction, PostParseOwnerDriverStep,
                parser_prepared_script_page_owned_work,
            },
            document_script_scheduler::{DocumentScriptExecutionLane, PageOwnedDocumentScriptWork},
            page_task_queue::{PageTask, PostParseLifecycleWork, PostParsePageOwnedWork},
            planning::{ScriptSource, SharedScriptSourceLoad},
            types::{ScriptKind, ScriptMode, ScriptRun, ScriptSkipReason, ScriptSourceKind},
        },
    };

    use super::{
        CurrentScriptContextSpec, DocumentRuntime, DomHandle, ParserInsertionController,
        PostParsePageTaskPopBlocker,
    };

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

    #[test]
    fn parser_insertion_context_does_not_require_current_script() {
        let url = Url::parse("https://example.com/").unwrap();
        let document = HtmlParser.parse(
            url.clone(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let stream = Rc::new(RefCell::new(HtmlParser.start_document(url)));
        let mut runtime = DocumentRuntime::new(&document);

        runtime.set_current_script_context(CurrentScriptContextSpec {
            handle: None,
            parser_write_insertion_point_active: true,
            parser_insertion_controller: Some(ParserInsertionController::for_stream(stream)),
        });

        assert_eq!(runtime.current_script_handle(), None);
        assert!(runtime.has_active_parser_write_insertion_point());
        assert!(runtime.current_parser_insertion_controller().is_some());
    }

    fn modulepreload_as_matrix_markup(base_href: &str) -> String {
        let mut markup = format!("<!doctype html><html><head><base href='{base_href}'>");
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
                "style" => format!("{slug}.css"),
                "json" => format!("{slug}.json"),
                "text" => format!("{slug}.txt"),
                _ => format!("{slug}.js"),
            };
            markup.push_str(&format!(
                "<link rel='modulepreload' href='{href}' as='{value}'>"
            ));
        }
        markup.push_str("</head><body></body></html>");
        markup
    }

    fn parser_modulepreload_link_handles(runtime: &DocumentRuntime) -> Vec<DomHandle> {
        let mut handles = Vec::new();
        let mut stack = vec![runtime.dom_host.document_node_id()];
        while let Some(handle) = stack.pop() {
            let mut children = runtime.dom_host.child_handles(handle).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
            if runtime
                .dom_host
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| crate::modulepreload::modulepreload_href(element).is_some())
            {
                handles.push(handle);
            }
        }
        handles
    }

    fn prepared_script(position: usize, mode: ScriptMode) -> crate::planning::PreparedScript {
        crate::planning::PreparedScript {
            position,
            node_id: NodeId::new(position + 1),
            kind: ScriptKind::Classic,
            mode,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            base_url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            initiator_url: Url::parse("https://example.com/index.html").unwrap(),
            host_script_handle: None,
        }
    }

    fn post_parse_script_waiting_for_source(
        lane: DocumentScriptExecutionLane,
        script: crate::planning::PreparedScript,
        source_load: SharedScriptSourceLoad,
    ) -> PostParsePageOwnedWork {
        PostParsePageOwnedWork::document_script_work_with_blocking_signatures(
            PageOwnedDocumentScriptWork::script_waiting_for_source(lane, script, source_load),
            HashSet::new(),
        )
    }

    fn parser_owned_script_work(position: usize) -> PostParsePageOwnedWork {
        parser_prepared_script_page_owned_work(
            prepared_script(position, ScriptMode::Normal),
            Default::default(),
        )
    }

    fn skipped_script_run(position: usize) -> ScriptRun {
        ScriptRun::skipped(
            NodeId::new(position + 1),
            ScriptKind::Classic,
            ScriptMode::Normal,
            ScriptSourceKind::Inline,
            Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            ScriptSkipReason::NotInMainDocument,
        )
    }

    fn lifecycle_work(work: PostParseLifecycleWork) -> PostParsePageOwnedWork {
        PostParsePageOwnedWork::lifecycle_work(work)
    }

    fn is_parser_blocking_document_script_action(
        action: &DocumentProcessingAction,
        position: usize,
    ) -> bool {
        matches!(
            action,
            DocumentProcessingAction::PostParsePageOwnedWork(work)
                if work
                    .as_script()
                    .is_some_and(|script| script.position == position)
        )
    }

    #[test]
    fn runtime_binding_queue_is_drained_once() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        runtime.absorb_runtime_binding_calls(vec![
            crate::protocol_types::PendingRuntimeBindingCall {
                source: crate::protocol_types::RuntimeBindingCallSourceIdentity::new(1, 1),
                name: "testBinding".to_owned(),
                payload: "payload".to_owned(),
                execution_context_id: 1,
            },
        ]);

        assert_eq!(runtime.pending_runtime_binding_call_count(), 1);

        let calls = runtime.take_runtime_binding_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(runtime.pending_runtime_binding_call_count(), 0);
    }

    #[test]
    fn document_replacement_parser_blocker_tracks_the_actual_input_stream() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        assert!(!runtime.document_replacement_parser_is_blocked());
        runtime.open_document();
        assert!(runtime.document_replacement_parser_is_blocked());
        assert!(runtime.host_document_mut().close_live_document_stream());
        assert!(!runtime.document_replacement_parser_is_blocked());

        runtime.set_document_ready_state(DocumentReadyState::Complete);
        assert!(!runtime.host_document_mut().close_live_document_stream());
        assert_eq!(
            runtime.host_document().ready_state(),
            DocumentReadyState::Complete,
            "repeated document.close must not rewind an already closed input stream"
        );
    }

    #[test]
    fn parser_discovered_modulepreloads_use_as_module_type_for_keys() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<base href='https://cdn.example/assets/'>",
                "<link rel='modulepreload' href='shared' as='script'>",
                "<link rel='modulepreload' href='audio-worklet.js' as='audioworklet'>",
                "<link rel='modulepreload' href='paint-worklet.js' as='paintworklet'>",
                "<link rel='modulepreload' href='service-worker.js' as='serviceworker'>",
                "<link rel='modulepreload' href='shared-worker.js' as='sharedworker'>",
                "<link rel='modulepreload' href='worker.js' as='worker'>",
                "<link rel='modulepreload' href='normalized.js' as='invalid-dest'>",
                "<link rel='modulepreload' href='shared' as='json'>",
                "<link rel='modulepreload' href='style.css' as='style'>",
                "<link rel='modulepreload' href='data.txt' as='text'>",
                "<link rel='modulepreload' href='invalid.bin' as='image'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let handles = parser_modulepreload_link_handles(&runtime);
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles)
            .into_parts();
        let mut keys = requests
            .into_iter()
            .map(|request| request.module_key().clone())
            .collect::<Vec<_>>();
        keys.sort_by(|left, right| {
            left.url()
                .as_str()
                .cmp(right.url().as_str())
                .then_with(|| format!("{:?}", left.kind()).cmp(&format!("{:?}", right.kind())))
        });

        assert_eq!(keys.len(), 10);
        assert_eq!(
            warnings,
            vec!["<link rel=modulepreload> has an invalid `as` value image".to_owned()]
        );
        assert_eq!(link_error_tasks, 1);
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://cdn.example/assets/shared").unwrap()
        )));
        for path in [
            "audio-worklet.js",
            "paint-worklet.js",
            "service-worker.js",
            "shared-worker.js",
            "worker.js",
            "normalized.js",
        ] {
            assert!(keys.contains(&ModuleMapKey::java_script(
                Url::parse(&format!("https://cdn.example/assets/{path}")).unwrap()
            )));
        }
        assert!(keys.contains(&ModuleMapKey::json_with_attributes(
            Url::parse("https://cdn.example/assets/shared").unwrap(),
            crate::module_runtime::ModuleAttributesKey::from_pairs(vec![(
                "type".to_owned(),
                "json".to_owned(),
            )]),
        )));
        assert!(keys.contains(&ModuleMapKey::modulepreload_text(
            Url::parse("https://cdn.example/assets/data.txt").unwrap(),
        )));
        assert!(keys.contains(&ModuleMapKey::css_with_attributes(
            Url::parse("https://cdn.example/assets/style.css").unwrap(),
            crate::module_runtime::ModuleAttributesKey::from_pairs(vec![(
                "type".to_owned(),
                "css".to_owned(),
            )]),
        )));
        assert!(
            !keys
                .iter()
                .any(|key| key.url().as_str().contains("invalid"))
        );
        assert!(
            keys.iter()
                .any(|key| matches!(key.kind(), ModuleKind::Json))
        );
    }

    #[test]
    fn parser_discovered_modulepreload_as_values_match_wpt_matrix() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            modulepreload_as_matrix_markup("https://cdn.example/assets/"),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let handles = parser_modulepreload_link_handles(&runtime);
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles)
            .into_parts();
        let keys = requests
            .iter()
            .map(|request| request.module_key().clone())
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), MODULEPRELOAD_AS_MATRIX_GOOD_VALUES.len());
        assert_eq!(warnings.len(), MODULEPRELOAD_AS_MATRIX_BAD_VALUES.len());
        assert_eq!(link_error_tasks, MODULEPRELOAD_AS_MATRIX_BAD_VALUES.len());
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://cdn.example/assets/1-invalid-dest.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://cdn.example/assets/8-script.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://cdn.example/assets/11-worker.js").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::json_with_attributes(
            Url::parse("https://cdn.example/assets/4-json.json").unwrap(),
            crate::module_runtime::ModuleAttributesKey::from_pairs(vec![(
                "type".to_owned(),
                "json".to_owned(),
            )]),
        )));
        assert!(keys.contains(&ModuleMapKey::css_with_attributes(
            Url::parse("https://cdn.example/assets/3-style.css").unwrap(),
            crate::module_runtime::ModuleAttributesKey::from_pairs(vec![(
                "type".to_owned(),
                "css".to_owned(),
            )]),
        )));
        assert!(keys.contains(&ModuleMapKey::modulepreload_text(
            Url::parse("https://cdn.example/assets/5-text.txt").unwrap(),
        )));
        assert!(
            warnings
                .iter()
                .filter(|warning| warning.as_str()
                    == "<link rel=modulepreload> has an invalid `as` value image")
                .count()
                >= 2,
            "image and iMaGe should both be rejected as known non-script-like destinations"
        );
    }

    #[test]
    fn parser_discovered_modulepreload_invalid_as_warns_each_link_once() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel='modulepreload' href='bad.bin' as='image'>",
                "<link rel='modulepreload' href='bad.bin' as='IMAGE'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let handles = parser_modulepreload_link_handles(&runtime);
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles.clone())
            .into_parts();

        assert!(
            requests.is_empty(),
            "invalid modulepreload `as` must not start parser-discovered fetches"
        );
        assert_eq!(
            warnings,
            vec![
                "<link rel=modulepreload> has an invalid `as` value image".to_owned(),
                "<link rel=modulepreload> has an invalid `as` value image".to_owned()
            ]
        );
        assert_eq!(
            link_error_tasks, 2,
            "each invalid parser-discovered link needs its own networking-task error event"
        );
        assert!(runtime.pop_ready_connected_style_load().is_none());
        assert!(
            runtime.has_ready_native_module_owner_event(),
            "parser-discovered invalid modulepreload should post native module owner events"
        );
        let first_event = runtime
            .take_next_native_module_owner_event()
            .expect("first invalid modulepreload should queue a link error event");
        assert!(
            matches!(
                first_event,
                crate::module_runtime::NativeModuleOwnerEvent::ModulepreloadLinkError(_)
            ),
            "parser-discovered invalid modulepreload should use module owner event lane"
        );
        let second_event = runtime
            .take_next_native_module_owner_event()
            .expect("second invalid modulepreload should queue a link error event");
        assert!(
            matches!(
                second_event,
                crate::module_runtime::NativeModuleOwnerEvent::ModulepreloadLinkError(_)
            ),
            "parser-discovered invalid modulepreload should use module owner event lane"
        );
        assert!(
            runtime.take_next_native_module_owner_event().is_none(),
            "invalid modulepreload link error events should be consumed once"
        );
        runtime.queue_initial_connected_style_loads();
        assert!(
            runtime.pop_ready_connected_style_load().is_none(),
            "connected initial scan must not queue a duplicate invalid-as link error event"
        );

        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles)
            .into_parts();
        assert!(requests.is_empty());
        assert!(
            warnings.is_empty(),
            "dirty rescans should not repeat the same parser-discovered invalid-as link warnings"
        );
        assert_eq!(
            link_error_tasks, 0,
            "dirty rescans should not queue duplicate link error events"
        );
        assert!(
            runtime.take_next_native_module_owner_event().is_none(),
            "dirty rescans should not queue duplicate native module owner link error events"
        );
    }

    #[test]
    fn parser_discovered_modulepreloads_ignore_fetchpriority_hint() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel='modulepreload' href='high.mjs' fetchpriority='high'>",
                "<link rel='modulepreload' href='low.mjs' fetchpriority='low'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let handles = parser_modulepreload_link_handles(&runtime);
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles)
            .into_parts();

        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(link_error_tasks, 0);
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert_eq!(
                request.fetch_metadata().fetch_priority_for_test(),
                None,
                "Chromium modulepreload uses FetchPriorityHint::kAuto rather than the link fetchpriority attribute"
            );
        }
    }

    #[test]
    fn parser_discovered_modulepreloads_skip_non_matching_media() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            concat!(
                "<!doctype html><html><head>",
                "<link rel='modulepreload' href='screen.mjs' media='screen'>",
                "<link rel='modulepreload' href='wide.mjs' media='(min-width: 100px)'>",
                "<link rel='modulepreload' href='print.mjs' media='print'>",
                "<link rel='modulepreload' href='narrow.mjs' media='(max-width: 1px)'>",
                "</head><body></body></html>",
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let handles = parser_modulepreload_link_handles(&runtime);
        let (requests, warnings, link_error_tasks) = runtime
            .accept_parser_discovered_modulepreload_links(handles)
            .into_parts();
        let keys = requests
            .into_iter()
            .map(|request| request.module_key().clone())
            .collect::<Vec<_>>();

        assert!(warnings.is_empty(), "runtime warnings: {warnings:?}");
        assert_eq!(link_error_tasks, 0);
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.com/pages/screen.mjs").unwrap()
        )));
        assert!(keys.contains(&ModuleMapKey::java_script(
            Url::parse("https://example.com/pages/wide.mjs").unwrap()
        )));
        assert!(
            !keys
                .iter()
                .any(|key| key.url().as_str().contains("print.mjs"))
        );
        assert!(
            !keys
                .iter()
                .any(|key| key.url().as_str().contains("narrow.mjs"))
        );
    }

    #[test]
    fn parser_discovered_images_mark_first_five_non_small_priority_boost_candidates() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/pages/index.html").unwrap(),
            concat!(
                "<!doctype html><html><body>",
                "<img src='small.png' width='10' height='10'>",
                "<img src='hero-1.png'>",
                "<img src='hero-2.png' width='101' height='100'>",
                "<img src='explicit-low.png' width='120' height='120' fetchpriority='low'>",
                "<img src='zero.png' width='0'>",
                "<img src='hero-3.png' width='200'>",
                "<img src='hero-4.png' height='200'>",
                "<img src='hero-5.png' width='120' height='120'>",
                "<img src='hero-6.png' width='120' height='120'>",
                "</body></html>"
            )
            .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);

        let image_handles = (0..runtime.dom_host().dom().nodes().len())
            .map(crate::document_runtime::DomHandle::new)
            .filter(|handle| {
                runtime
                    .dom_host()
                    .node(*handle)
                    .and_then(crate::dom::native::Node::as_element)
                    .is_some_and(|element| element.is_html_element("img"))
            })
            .collect::<Vec<_>>();
        let images = image_handles
            .into_iter()
            .map(|handle| {
                let source = runtime
                    .dom_host()
                    .node(handle)
                    .and_then(crate::dom::native::Node::as_element)
                    .and_then(|element| element.attribute("src"))
                    .expect("image source")
                    .to_owned();
                (source, runtime.claim_main_image_priority_boost(handle))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            images,
            vec![
                ("small.png".to_owned(), false),
                ("hero-1.png".to_owned(), true),
                ("hero-2.png".to_owned(), true),
                ("explicit-low.png".to_owned(), true),
                ("zero.png".to_owned(), false),
                ("hero-3.png".to_owned(), true),
                ("hero-4.png".to_owned(), true),
                ("hero-5.png".to_owned(), false),
                ("hero-6.png".to_owned(), false),
            ],
            "Chromium counts the first five non-small in-document image candidates before author-hint filtering"
        );
    }

    #[tokio::test]
    async fn domcontentloaded_overtakes_pending_connected_style_load() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

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
            .expect("stylesheet link");

        runtime.enqueue_pending_connected_style_load_for_test(link);

        assert!(matches!(
            runtime.poll_document_processing_action(
                &mut task_queue,
                Option::<&NativeDom>::None
            ),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    }

    #[test]
    fn parse_time_lifecycle_page_task_materializes_as_page_owned_work() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue
            .enqueue_parser_boundary(crate::page_task_queue::PageTask::DispatchDomContentLoaded);

        assert!(matches!(
            runtime.poll_document_processing_action(&mut task_queue, Some(&document)),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    }

    #[test]
    fn ready_document_processing_wake_allows_domcontentloaded_past_pending_connected_style_load() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

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
            .expect("stylesheet link");

        runtime.enqueue_pending_connected_style_load_for_test(link);

        assert!(runtime.has_ready_document_processing_wake(&mut task_queue));
    }

    #[test]
    fn parse_time_pending_processing_ignores_pending_connected_style_load() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let runtime = {
            let mut runtime = DocumentRuntime::new(&document);
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
                .expect("stylesheet link");

            runtime.enqueue_pending_connected_style_load_for_test(link);
            runtime
        };
        let task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();

        assert!(!runtime.has_pending_parse_time_document_processing(&task_queue));
    }

    #[test]
    fn pending_connected_style_queue_is_not_window_load_readiness_fact_source() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue
            .extend_post_parse_work([lifecycle_work(PostParseLifecycleWork::test_window_load())]);

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
            .expect("stylesheet link");

        runtime.enqueue_pending_connected_style_load_for_test(link);

        assert!(
            runtime.has_ready_document_processing_wake(&mut task_queue),
            "DocumentRuntime style queues must not replace document-owned load-delay tokens"
        );
    }

    #[tokio::test]
    async fn domcontentloaded_is_not_blocked_by_post_domcontentloaded_runtime_backlog() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, true),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    }

    #[tokio::test]
    async fn post_parse_owner_driver_step_keeps_domcontentloaded_ready_with_runtime_backlog() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, true),
            PostParseOwnerDriverStep::Ready(action)
                if matches!(
                    action.as_ref(),
                    DocumentProcessingAction::PostParsePageOwnedWork(work)
                        if work.is_domcontentloaded_task()
                )
        ));
    }

    #[tokio::test]
    async fn ready_connected_style_load_does_not_overtake_domcontentloaded() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

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
            .expect("stylesheet link");

        runtime
            .stylesheet_lifecycle
            .injected_ready_connected_loads
            .push_back(crate::document_runtime::ReadyConnectedStyleLoad::for_owner(
                link,
                true,
                crate::document_runtime::ConnectedStyleEventElementKind::Link,
            ));

        assert!(matches!(
            runtime.poll_document_processing_action(
                &mut task_queue,
                Option::<&NativeDom>::None
            ),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
        assert!(matches!(
            runtime.poll_document_processing_action(
                &mut task_queue,
                Option::<&NativeDom>::None
            ),
            Some(DocumentProcessingAction::DispatchConnectedStyleLoad(ready))
                if ready.owner() == link
        ));
    }

    #[tokio::test]
    async fn ready_connected_style_load_dispatches_while_defer_source_is_pending() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([post_parse_script_waiting_for_source(
            DocumentScriptExecutionLane::ClassicDefer,
            prepared_script(1, ScriptMode::Defer),
            SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
        )]);

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
            .expect("stylesheet link");

        runtime
            .stylesheet_lifecycle
            .injected_ready_connected_loads
            .push_back(crate::document_runtime::ReadyConnectedStyleLoad::for_owner(
                link,
                true,
                crate::document_runtime::ConnectedStyleEventElementKind::Link,
            ));

        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, false),
            PostParseOwnerDriverStep::Ready(action)
                if matches!(
                    action.as_ref(),
                    DocumentProcessingAction::DispatchConnectedStyleLoad(ready)
                        if ready.owner() == link
                )
        ));
        assert!(
            task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load)
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn parser_owned_pre_domcontentloaded_task_waits_behind_defer_like_front() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let loader =
            crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
                .expect("test Document resource loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([PostParsePageOwnedWork::document_script_work(
            PageOwnedDocumentScriptWork::script(
                DocumentScriptExecutionLane::ClassicDefer,
                prepared_script(1, ScriptMode::Defer),
            ),
        )]);
        runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
            parser_owned_script_work(2),
        ]);

        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.as_script().is_some_and(|script| script.position == 1)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_parse_owner_dispatches_ready_connected_style_before_reporting_script_source_wait()
    {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let (_source_ready_tx, source_ready_rx) = tokio::sync::oneshot::channel::<()>();
        let source_load = SharedScriptSourceLoad::spawn_for_test(async move {
            source_ready_rx
                .await
                .expect("test should release the deferred source load");
            Ok("window.deferLoaded = true;".to_owned())
        });
        task_queue.extend_post_parse_work([post_parse_script_waiting_for_source(
            DocumentScriptExecutionLane::ClassicDefer,
            prepared_script(1, ScriptMode::Defer),
            source_load,
        )]);

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
            .expect("stylesheet link");
        runtime.enqueue_ready_connected_style_load_for_test(link);

        assert!(runtime.has_ready_document_processing_wake(&mut task_queue));
        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, false),
            PostParseOwnerDriverStep::Ready(action)
                if matches!(
                    action.as_ref(),
                    DocumentProcessingAction::DispatchConnectedStyleLoad(ready)
                        if ready.owner() == link
                )
        ));
        assert!(
            task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_waiting_for_source_load)
        );
        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, false),
            PostParseOwnerDriverStep::AwaitProgress
        ));
    }

    #[test]
    fn parser_owned_pre_domcontentloaded_delivery_drains_ahead_of_window_load() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue
            .extend_post_parse_work([lifecycle_work(PostParseLifecycleWork::test_window_load())]);

        assert!(
            runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                parser_owned_script_work(3),
            ])
        );
        assert!(matches!(
            task_queue
                .post_parse_front()
                .and_then(PostParsePageOwnedWork::as_page_task),
            Some(PageTask::DispatchWindowLoad)
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 3)
        ));
    }

    #[test]
    fn parser_owned_pre_domcontentloaded_delivery_keeps_owner_source_behind_run_record_front() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::RecordDocumentScriptRun {
                position: 4,
                run: skipped_script_run(4),
            },
        )]);

        assert!(
            runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                parser_owned_script_work(5),
            ])
        );
        assert!(matches!(
            task_queue
                .post_parse_front()
                .and_then(PostParsePageOwnedWork::as_page_task),
            Some(PageTask::RecordDocumentScriptRun { position: 4, .. })
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if matches!(
                    work.as_page_task(),
                    Some(PageTask::RecordDocumentScriptRun { position: 4, .. })
                )
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 5)
        ));
    }

    #[test]
    fn parser_owned_pre_domcontentloaded_delivery_preserves_owner_order_ahead_of_domcontentloaded()
    {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);

        assert!(
            runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
                parser_owned_script_work(6),
                parser_owned_script_work(7),
            ])
        );
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 6)
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 7)
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    }

    #[test]
    fn parser_owned_pre_domcontentloaded_owner_source_drains_when_queue_is_empty() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
            parser_owned_script_work(9),
            parser_owned_script_work(10),
        ]);

        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 9)
        ));
        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 10)
        ));
        assert!(task_queue.post_parse_pop_front().is_none());
    }

    #[tokio::test]
    async fn post_parse_owner_driver_step_keeps_non_window_tail_ready_with_runtime_backlog() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![]),
        )]);

        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, true),
            PostParseOwnerDriverStep::Ready(action)
                if matches!(
                    action.as_ref(),
                    DocumentProcessingAction::PostParsePageOwnedWork(work)
                        if matches!(
                            work.as_page_task(),
                            Some(PageTask::RecordDetachedPostParseRuns(runs)) if runs.is_empty()
                        )
                )
        ));
    }

    #[test]
    fn parser_owned_pre_domcontentloaded_task_overtakes_other_post_parse_front() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![]),
        )]);
        runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
            parser_owned_script_work(11),
        ]);

        let readiness = runtime.post_parse_owner_readiness(&mut task_queue, false);
        assert!(readiness.blocks_page_task_pop);
        assert!(!readiness.should_poll_document_processing);

        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 11)
        ));
        assert!(matches!(
            task_queue.post_parse_front().and_then(PostParsePageOwnedWork::as_page_task),
            Some(PageTask::RecordDetachedPostParseRuns(runs)) if runs.is_empty()
        ));
    }

    #[test]
    fn post_parse_page_task_pop_blocker_distinguishes_parser_owned_source_from_window_load_backlog()
    {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();

        assert!(matches!(
            runtime.post_parse_page_task_pop_blocker(&mut task_queue, true, false),
            Some(PostParsePageTaskPopBlocker::ParserOwnedPreDomContentLoadedTask)
        ));

        task_queue
            .extend_post_parse_work([lifecycle_work(PostParseLifecycleWork::test_window_load())]);

        assert!(matches!(
            runtime.post_parse_page_task_pop_blocker(&mut task_queue, false, true),
            Some(
                PostParsePageTaskPopBlocker::WindowLoadWaitingForPostDomContentLoadedRuntimeBacklog
            )
        ));
        assert!(
            runtime
                .post_parse_page_task_pop_blocker(&mut task_queue, false, false)
                .is_none()
        );
    }

    #[test]
    fn post_parse_owner_readiness_keeps_domcontentloaded_unblocked_with_parser_owned_source() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue.extend_post_parse_work([lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        )]);
        runtime.send_parser_owned_pre_domcontentloaded_page_owned_work(vec![
            parser_owned_script_work(12),
        ]);

        let readiness = runtime.post_parse_owner_readiness(&mut task_queue, false);
        assert!(
            readiness.blocks_page_task_pop,
            "DCL is a lifecycle boundary, so parser-owned pre-DCL work should drain as an owner action before it"
        );
        assert!(!readiness.should_poll_document_processing);
        assert!(readiness.has_pending_progress_source);

        assert!(matches!(
            runtime.poll_next_ready_post_parse_owner_action(&mut task_queue, false),
            Some(action) if is_parser_blocking_document_script_action(&action, 12)
        ));
        assert!(matches!(
            task_queue
                .post_parse_front()
                .and_then(PostParsePageOwnedWork::as_page_task),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[tokio::test]
    async fn post_parse_owner_driver_step_awaits_when_window_load_is_blocked_by_runtime_backlog() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        task_queue
            .extend_post_parse_work([lifecycle_work(PostParseLifecycleWork::test_window_load())]);

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
            .expect("stylesheet link");
        runtime.enqueue_pending_connected_style_load_for_test(link);

        assert!(matches!(
            runtime.poll_next_post_parse_owner_driver_step(&mut task_queue, true),
            PostParseOwnerDriverStep::AwaitProgress
        ));
    }
}
