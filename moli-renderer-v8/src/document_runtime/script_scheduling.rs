mod document_write_scripts;
mod module_facade;
mod post_parse_lifecycle;
mod script_handles;

pub(super) use self::document_write_scripts::DocumentWriteCurrentScriptEventBehavior;
use super::*;

impl DocumentRuntime {
    pub(crate) fn enqueue_main_document_post_parse_work(
        &self,
        work: crate::page_task_queue::PostParsePageOwnedWork,
    ) -> Result<(), crate::page_task_queue::RendererPageMainDocumentRuntimeAdmissionError> {
        self.script_lifecycle
            .scripts()
            .enqueue_main_document_post_parse_work(work)
    }

    pub(crate) fn enqueue_main_parser_async_module_admission(
        &self,
        admission: crate::document_script_scheduler::MainParserAsyncModuleAdmission,
    ) -> Result<(), crate::page_task_queue::RendererPageMainDocumentRuntimeRouteClosed> {
        self.script_lifecycle
            .scripts()
            .enqueue_main_parser_async_module_admission(admission)
    }

    pub(crate) fn accept_main_parser_deferred_script(
        &mut self,
        task_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        script: crate::planning::PreparedScript,
        shared_load: Option<crate::planning::SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: std::collections::HashSet<
            crate::DocumentBlockingStylesheetSignature,
        >,
        load_delay_token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> Option<crate::document_runtime::PendingMainParserDeferredScriptStart> {
        let owner = crate::module_script_continuation::MainParserDocumentOwner::new(task_owner);
        let action = self
            .script_lifecycle
            .parser_module_document_scripts_mut()
            .claim_parser_deferred_script(
                owner,
                script,
                shared_load,
                document_character_set,
                blocking_signatures_before,
                load_delay_token,
            )?;
        Some(
            crate::document_runtime::PendingMainParserDeferredScriptStart::new(
                task_owner,
                load_delay_token,
                action,
            ),
        )
    }

    pub(crate) fn enqueue_main_parser_deferred_script_start(
        &mut self,
        start: crate::document_runtime::PendingMainParserDeferredScriptStart,
    ) {
        self.script_lifecycle
            .enqueue_main_parser_deferred_start(start);
    }

    pub(crate) fn take_main_parser_deferred_script_starts(
        &mut self,
    ) -> std::collections::VecDeque<crate::document_runtime::PendingMainParserDeferredScriptStart>
    {
        self.script_lifecycle.take_main_parser_deferred_starts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        module_script_continuation::MainParserDocumentOwner,
        parser::HtmlParser,
        {
            document_script_scheduler::{DocumentScriptExecutionLane, PageOwnedDocumentScriptWork},
            host::ScriptHandleSource,
            host::{ScriptEventKind, ScriptHandleExecutionSubject, ScriptHostEventSubject},
            page_task_queue::{PostParseLifecycleWork, PostParsePageOwnedWork},
            planning::ScriptSource,
            stylesheet_blocking::DocumentBlockingStylesheetSignature,
            types::{
                ScriptExecutionReport, ScriptKind, ScriptMode, ScriptRun, ScriptSkipReason,
                ScriptSourceKind,
            },
        },
    };
    use moli_fetch::FetchConfig;
    use url::Url;

    fn main_document_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
    }

    fn parse_document_with_blocking_stylesheet_inputs(
        html: &str,
    ) -> (
        NativeDom,
        Vec<crate::DocumentOwnedBlockingStylesheetDiscoveryInput>,
    ) {
        let (document, _, inputs) = crate::parse_html_test_fixture_with_parser_outputs(
            Url::parse("https://example.com/").unwrap(),
            html.to_owned(),
        );
        (document, inputs)
    }

    fn prepared_script(position: usize, node_index: usize, mode: ScriptMode) -> PreparedScript {
        PreparedScript {
            position,
            node_id: NodeId::new(node_index),
            kind: ScriptKind::Classic,
            mode,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::Loaded("console.log('x')".to_owned()),
            initiator_url: Url::parse("https://example.com/script.js").unwrap(),
            base_url: Url::parse("https://example.com/script.js").unwrap(),
            url: Url::parse("https://example.com/script.js").unwrap(),
            host_script_handle: None,
        }
    }

    fn post_parse_document_script_work(
        lane: DocumentScriptExecutionLane,
        script: PreparedScript,
    ) -> PostParsePageOwnedWork {
        PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::script(
            lane, script,
        ))
    }

    #[test]
    fn enqueue_post_parse_lifecycle_page_owned_work_routes_through_document_runtime_owner() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let mut report = ScriptExecutionReport::default();
        let detached_run = ScriptRun::skipped(
            NodeId::new(99),
            ScriptKind::Classic,
            ScriptMode::Async,
            ScriptSourceKind::External,
            Url::parse("https://example.com/detached.js").unwrap(),
            ScriptSkipReason::NotInMainDocument,
        );

        runtime.enqueue_post_parse_lifecycle_page_owned_work(
            &mut task_queue,
            vec![
                post_parse_document_script_work(
                    DocumentScriptExecutionLane::ClassicDefer,
                    prepared_script(10, 10, ScriptMode::Defer),
                ),
                PostParsePageOwnedWork::lifecycle_work(
                    PostParseLifecycleWork::test_domcontentloaded(),
                ),
                post_parse_document_script_work(
                    DocumentScriptExecutionLane::AsyncPhase,
                    prepared_script(20, 20, ScriptMode::Async),
                ),
                PostParsePageOwnedWork::lifecycle_work(
                    PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![detached_run.clone()]),
                ),
                PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::test_window_load()),
            ],
            &mut report,
        );

        assert!(matches!(
            task_queue.post_parse_pop_front(),
            Some(work) if work.is_defer_like_document_script()
        ));
        assert!(matches!(
            task_queue.post_parse_pop_front(),
            Some(work) if work.is_domcontentloaded_task()
        ));
        assert!(matches!(
            task_queue.post_parse_pop_front(),
            Some(work) if work.is_async_phase_document_script()
        ));
        assert!(matches!(
            task_queue.post_parse_pop_front(),
            Some(work) if work.detached_run_count() == 1
        ));
        assert!(matches!(
            task_queue.post_parse_pop_front(),
            Some(work) if work.is_window_load_task()
        ));
        assert!(task_queue.is_empty());
        assert!(report.runs().is_empty());
    }

    #[test]
    fn prepare_post_parse_lifecycle_page_owned_work_injects_boundaries_around_trailing_work() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let runtime = DocumentRuntime::new(&document);
        let detached_run = ScriptRun::skipped(
            NodeId::new(99),
            ScriptKind::Classic,
            ScriptMode::Async,
            ScriptSourceKind::External,
            Url::parse("https://example.com/detached.js").unwrap(),
            ScriptSkipReason::NotInMainDocument,
        );

        let work = runtime.prepare_post_parse_lifecycle_page_owned_work(
            main_document_owner(),
            vec![PostParsePageOwnedWork::lifecycle_work(
                PostParseLifecycleWork::RecordDetachedPostParseRuns(vec![detached_run]),
            )],
        );

        assert!(
            work.first()
                .is_some_and(|work| work.is_domcontentloaded_task())
        );
        assert!(matches!(
            work.get(1),
            Some(work) if work.detached_run_count() == 1
        ));
        assert!(work.get(2).is_some_and(|work| work.is_window_load_task()));
    }

    #[test]
    fn prepare_post_parse_lifecycle_page_owned_work_keeps_runtime_async_tail_after_domcontentloaded()
     {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let runtime = DocumentRuntime::new(&document);

        let work = runtime.prepare_post_parse_lifecycle_page_owned_work(
            main_document_owner(),
            vec![
                post_parse_document_script_work(
                    DocumentScriptExecutionLane::ClassicDefer,
                    prepared_script(10, 10, ScriptMode::Defer),
                ),
                post_parse_document_script_work(
                    DocumentScriptExecutionLane::ClassicDefer,
                    prepared_script(20, 20, ScriptMode::Defer),
                ),
                post_parse_document_script_work(
                    DocumentScriptExecutionLane::AsyncPhase,
                    prepared_script(30, 30, ScriptMode::Async),
                ),
            ],
        );

        assert!(
            work.first()
                .is_some_and(|work| work.is_defer_like_document_script())
        );
        assert!(
            work.get(1)
                .is_some_and(|work| work.is_defer_like_document_script())
        );
        assert!(
            work.get(2)
                .is_some_and(|work| work.is_domcontentloaded_task())
        );
        assert!(
            work.get(3)
                .is_some_and(|work| work.is_async_phase_document_script())
        );
        assert!(work.get(4).is_some_and(|work| work.is_window_load_task()));
    }
    #[tokio::test]
    async fn blocking_stylesheets_gate_defer_like_post_parse_work_by_snapshot() {
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'><script defer src='/app.js'></script></head><body></body></html>",
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let script = document.script_handles()[0];
        let blocker = blocking_stylesheet_inputs
            .first()
            .expect("stylesheet before defer script should be discovered");
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        let mut blocking_signatures_before = HashSet::new();
        blocking_signatures_before.insert(blocker.signature().clone());
        let work = PostParsePageOwnedWork::document_script_work_with_blocking_signatures(
            crate::document_script_scheduler::PageOwnedDocumentScriptWork::Script {
                lane: crate::document_script_scheduler::DocumentScriptExecutionLane::ClassicDefer,
                script: Box::new(prepared_script(10, script.index(), ScriptMode::Defer)),
                runtime_script_claim: None,
                source_network_result: None,
                load_delay_binding: None,
            },
            blocking_signatures_before,
        );

        assert!(
            runtime.post_parse_work_is_blocked_by_document_stylesheets(&work),
            "direct post-parse defer work should keep the same stylesheet snapshot gate"
        );
    }

    #[tokio::test]
    async fn parser_deferred_owner_source_keeps_snapshot_gate_without_queue_markers() {
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'><script defer src='/app.js'></script></head><body></body></html>",
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        let script_node = document.script_handles()[0];
        let blocker = blocking_stylesheet_inputs
            .first()
            .expect("stylesheet before defer script");
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let owner = MainParserDocumentOwner::new(task_owner);
        let mut blocking_signatures = HashSet::new();
        blocking_signatures.insert(blocker.signature().clone());
        runtime
            .parser_module_document_scripts_mut()
            .claim_ready_parser_deferred_script_for_test(
                owner,
                prepared_script(10, script_node.index(), ScriptMode::Defer),
                blocking_signatures,
            );
        assert_eq!(
            runtime
                .parser_module_document_scripts_mut()
                .seal_parser_deferred_scripts(owner),
            Ok(1)
        );

        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let mut report = ScriptExecutionReport::default();
        runtime.enqueue_post_parse_lifecycle_page_owned_work(
            &mut task_queue,
            vec![
                PostParsePageOwnedWork::main_parser_deferred_scripts(task_owner, 1),
                PostParsePageOwnedWork::lifecycle_work(
                    PostParseLifecycleWork::test_domcontentloaded(),
                ),
            ],
            &mut report,
        );
        assert!(
            task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_domcontentloaded_task),
            "parser marker should be document-owned state, not a queued lifecycle item"
        );
        assert!(
            runtime
                .poll_document_processing_action(&mut task_queue, Option::<&NativeDom>::None,)
                .is_none(),
            "snapshot stylesheet blocker should hold the parser owner source and DCL"
        );

        let DocumentBlockingStylesheetSignature::Link { url, .. } = blocker.signature() else {
            panic!("test fixture should discover a blocking link stylesheet");
        };
        runtime
            .stylesheet_lifecycle
            .fetches
            .enqueue_completion_for_testing(blocker.node_id(), url.clone(), true);
        runtime.drain_blocking_stylesheet_completions();
        let link_owner = DomHandle::new(blocker.node_id().index());
        let link_load = runtime
            .active_stylesheet_link_client_for_test(link_owner)
            .expect("parser stylesheet should retain its exact event client");
        // This low-level fixture has no ScriptVm to parse the root stylesheet and
        // publish its import graph. Complete the known-empty graph explicitly.
        runtime.note_stylesheet_import_graph_completion(link_load.fetch(), true);
        assert!(
            runtime.has_connected_style_event_for_test(),
            "the link load event should be resident in its independent DOM-manipulation source"
        );
        let ready = runtime
            .pop_connected_style_event_for_test()
            .expect("the typed link event should remain selectable");
        assert_eq!(ready.owner(), link_owner);
        runtime
            .stylesheet_lifecycle
            .owner_states
            .consume_link_event(&link_load);
        assert!(matches!(
            runtime.poll_document_processing_action(
                &mut task_queue,
                Option::<&NativeDom>::None,
            ),
            Some(DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.main_parser_deferred_scripts_owner() == Some(task_owner)
        ));
        assert!(
            task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_domcontentloaded_task),
            "DCL must remain queued until the released parser script completes"
        );
    }

    #[test]
    fn stale_parser_deferred_marker_without_owned_queue_cannot_block_dcl() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(99));
        let mut task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let mut report = ScriptExecutionReport::default();

        runtime.enqueue_post_parse_lifecycle_page_owned_work(
            &mut task_queue,
            vec![
                PostParsePageOwnedWork::main_parser_deferred_scripts(task_owner, 1),
                PostParsePageOwnedWork::lifecycle_work(
                    PostParseLifecycleWork::test_domcontentloaded(),
                ),
            ],
            &mut report,
        );

        assert!(runtime.main_parser_deferred_scripts_owner().is_none());
        assert!(
            task_queue
                .post_parse_front()
                .is_some_and(PostParsePageOwnedWork::is_domcontentloaded_task),
            "stale adapter marker must not arm a source after its parser queue was retired"
        );
    }

    #[tokio::test]
    async fn domcontentloaded_task_is_not_directly_blocked_by_stylesheets() {
        let (document, blocking_stylesheet_inputs) = parse_document_with_blocking_stylesheet_inputs(
            "<!doctype html><html><head><link rel=stylesheet href='/app.css'><script defer src='/app.js'></script></head><body></body></html>",
        );
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut runtime = DocumentRuntime::new_networked(&document, &loader);
        assert!(!blocking_stylesheet_inputs.is_empty());
        runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            blocking_stylesheet_inputs.iter(),
        );

        assert!(
            !runtime
                .page_task_is_blocked_by_document_stylesheets(&PageTask::DispatchDomContentLoaded)
        );
    }

    #[test]
    fn document_write_owned_handle_binding_uses_document_write_source() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script src='/written.js'></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let script_node = document.script_handles()[0];

        let handle = runtime.bind_document_write_owned_script_handle_for_node(script_node);

        assert_eq!(
            handle,
            format!("document-write-script-native-{}", script_node.index())
        );
        assert_eq!(
            runtime
                .script_lifecycle
                .scripts_mut()
                .script_host_event_subject(&handle),
            ScriptHostEventSubject {
                source: ScriptHandleSource::DocumentWriteOwned,
                execution: ScriptHandleExecutionSubject::PendingOrUnknown,
            }
        );
        assert_eq!(
            runtime
                .script_lifecycle
                .scripts_mut()
                .script_handle_followup_lane(&handle),
            Some(crate::document_runtime::DeferredPageTaskLane::PreDomContentLoaded),
            "document.write handle binding should keep the conservative pre-DCL default until the prepared script mode is known"
        );
    }

    #[test]
    fn inline_script_load_policy_does_not_require_a_host_handle() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script>window.inline = true;</script></head><body></body></html>"
                .to_owned(),
        );
        let runtime = DocumentRuntime::new(&document);
        let script_node = document.script_handles()[0];
        let script = PreparedScript {
            node_id: script_node,
            source_kind: ScriptSourceKind::Inline,
            source: ScriptSource::Inline("window.inline = true;".to_owned()),
            mode: ScriptMode::Normal,
            ..prepared_script(0, script_node.index(), ScriptMode::Normal)
        };

        assert!(script.host_script_handle.is_none());
        assert!(!runtime.script_event_requires_dispatch_for_script(ScriptEventKind::Load, &script));
    }

    #[test]
    fn parser_owned_module_failure_planning_uses_explicitly_bound_parser_handle() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script type='module' src='/app.mjs'></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let script_node = document.script_handles()[0];
        let expected_handle = runtime.bind_parser_owned_script_handle_for_node(script_node);
        let script = PreparedScript {
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleInOrder,
            node_id: script_node,
            host_script_handle: Some(expected_handle.clone()),
            ..prepared_script(0, script_node.index(), ScriptMode::ModuleInOrder)
        };

        let tasks = runtime.plan_script_failure_page_tasks(&script, "boom", None, None);

        assert!(matches!(
            tasks.first(),
            Some(PageTask::ReportWindowScriptFailure(_))
        ));
        assert_eq!(
            runtime
                .script_lifecycle
                .scripts_mut()
                .script_host_event_subject(&expected_handle),
            ScriptHostEventSubject {
                source: ScriptHandleSource::ParserOwned,
                execution: ScriptHandleExecutionSubject::PendingOrUnknown,
            }
        );
        assert_eq!(
            runtime
                .script_lifecycle
                .scripts_mut()
                .script_handle_followup_lane(&expected_handle),
            Some(crate::document_runtime::DeferredPageTaskLane::PreDomContentLoaded),
            "raw parser-owned registration starts with a conservative pre-DCL followup lane"
        );
    }

    #[test]
    #[should_panic(
        expected = "parser-owned script should bind host handle before failure planning"
    )]
    fn parser_created_module_failure_planning_production_path_rejects_missing_handle() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script type='module' src='/app.mjs'></script></head><body></body></html>"
                .to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let script_node = document.script_handles()[0];
        let script = PreparedScript {
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleInOrder,
            node_id: script_node,
            host_script_handle: None,
            ..prepared_script(0, script_node.index(), ScriptMode::ModuleInOrder)
        };

        let _ = runtime.plan_script_failure_page_tasks(&script, "boom", None, None);
    }

    #[test]
    fn non_parser_created_module_failure_planning_does_not_synthesize_parser_owned_handle() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        assert!(
            runtime
                .snapshot_document()
                .node(script_node)
                .is_some_and(|node| !node.flags().parser_created()),
            "dynamically created script should not be parser-created"
        );

        let script = PreparedScript {
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleInOrder,
            node_id: script_node,
            host_script_handle: None,
            ..prepared_script(0, script_node.index(), ScriptMode::ModuleInOrder)
        };

        let tasks = runtime.plan_script_failure_page_tasks(
            &script,
            "ModuleLinkFailed: module `/dep.mjs` does not export `missing`",
            None,
            None,
        );
        let unexpected_handle = format!("parser-script-native-{}", script_node.index());

        assert!(matches!(
            tasks.first(),
            Some(PageTask::ReportWindowScriptFailure(_))
        ));
        assert_eq!(
            runtime.resolve_host_script_handle(&unexpected_handle),
            None,
            "non-parser-created script failure planning should not synthesize a parser-owned handle"
        );
    }

    #[test]
    fn runtime_owned_in_order_prepared_script_does_not_wait_for_blocking_stylesheets() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "runtime-owned-style",
                script_node,
                ScriptHandleSource::RuntimeOwned,
            );
        let script = PreparedScript {
            mode: ScriptMode::InOrder,
            host_script_handle: Some("runtime-owned-style".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::InOrder)
        };

        assert!(
            !runtime.prepared_script_waits_for_blocking_stylesheets(&script),
            "runtime-owned in-order script should not wait on blocking stylesheets"
        );
    }

    #[test]
    fn parser_owned_defer_prepared_script_waits_for_blocking_stylesheets() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let head = runtime
            .snapshot_document()
            .document_head_handle()
            .expect("head should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(head, script_node),
            "parser-owned script should append into head"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "parser-owned-style",
                script_node,
                ScriptHandleSource::ParserOwned,
            );
        let script = PreparedScript {
            mode: ScriptMode::Defer,
            host_script_handle: Some("parser-owned-style".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::Defer)
        };

        assert!(
            runtime.prepared_script_waits_for_blocking_stylesheets(&script),
            "parser-owned defer script should keep stylesheet gating"
        );
    }

    #[test]
    fn runtime_owned_in_order_prepared_script_does_not_wait_until_domcontentloaded() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "runtime-owned-in-order",
                script_node,
                ScriptHandleSource::RuntimeOwned,
            );
        let script = PreparedScript {
            mode: ScriptMode::InOrder,
            host_script_handle: Some("runtime-owned-in-order".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::InOrder)
        };

        assert!(
            !runtime.prepared_script_waits_until_dom_content_loaded(&script),
            "runtime-owned classic in-order script should be eligible before DOMContentLoaded"
        );
    }

    #[test]
    fn runtime_owned_async_prepared_script_does_not_wait_until_domcontentloaded() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "runtime-owned-async",
                script_node,
                ScriptHandleSource::RuntimeOwned,
            );
        let script = PreparedScript {
            mode: ScriptMode::Async,
            host_script_handle: Some("runtime-owned-async".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::Async)
        };

        assert!(
            !runtime.prepared_script_waits_until_dom_content_loaded(&script),
            "runtime-owned async script should not stay behind DOMContentLoaded"
        );
    }

    #[test]
    fn runtime_owned_module_prepared_script_does_not_wait_until_domcontentloaded() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "runtime-owned-module",
                script_node,
                ScriptHandleSource::RuntimeOwned,
            );
        let script = PreparedScript {
            kind: ScriptKind::Module,
            mode: ScriptMode::Async,
            host_script_handle: Some("runtime-owned-module".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::Async)
        };

        assert!(
            !runtime.prepared_script_waits_until_dom_content_loaded(&script),
            "runtime-owned module should be eligible before DOMContentLoaded"
        );
    }

    #[test]
    fn domcontentloaded_state_does_not_reintroduce_runtime_owned_in_order_wait() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );
        runtime
            .script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                "runtime-owned-after-dcl",
                script_node,
                ScriptHandleSource::RuntimeOwned,
            );
        let script = PreparedScript {
            mode: ScriptMode::InOrder,
            host_script_handle: Some("runtime-owned-after-dcl".to_owned()),
            node_id: script_node,
            ..prepared_script(0, script_node.index(), ScriptMode::InOrder)
        };

        runtime.note_dom_content_loaded_dispatched();

        assert!(
            !runtime.prepared_script_waits_until_dom_content_loaded(&script),
            "runtime-owned in-order script should remain runnable after DOMContentLoaded as well"
        );
    }

    #[test]
    fn parser_owned_script_event_planning_resolves_the_preparation_time_handle() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head><script src='/missing.js'></script></head><body></body></html>"
                .to_owned(),
        );
        let script_node = document.script_handles()[0];
        let mut runtime = DocumentRuntime::new(&document);
        let bound_handle = runtime.bind_parser_owned_script_handle_for_node(script_node);

        let event = runtime
            .plan_parser_owned_script_event_task(ScriptEventKind::Error, script_node)
            .expect("parser-owned source failure should resolve its bound event target");

        assert_eq!(event.kind, ScriptEventKind::Error);
        assert_eq!(event.handle, bound_handle);
    }

    #[test]
    #[should_panic(expected = "parser-owned handle binding requires a parser-created <script>")]
    fn parser_owned_handle_binding_rejects_non_parser_created_script() {
        let document = HtmlParser.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        );
        let mut runtime = DocumentRuntime::new(&document);
        let body = runtime
            .snapshot_document()
            .document_body_handle()
            .expect("body should exist");
        let script_node = runtime.dom_host_mut().create_element("script");
        assert!(
            runtime.dom_host_mut().append_child(body, script_node),
            "dynamic script should append into body"
        );

        let _ = runtime.bind_parser_owned_script_handle_for_node(script_node);
    }
}
