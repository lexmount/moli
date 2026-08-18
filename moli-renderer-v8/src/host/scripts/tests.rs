use super::{
    HostScriptScheduler, ModuleFailurePolicy, PreparedRuntimeScriptStart, QueuedScriptFailureKind,
    RuntimeScriptPreparationContext, RuntimeScriptStartDecision, ScriptEventDispatchPolicy,
    ScriptEventPolicy, ScriptEventSkipReason, ScriptFailurePageTaskPolicy,
    ScriptHandleExecutionSubject, ScriptHandleSource, ScriptHandleStartState,
    ScriptHostEventSubject, ScriptStartCommitKind, prepare_script_start,
};
use crate::{
    dom::{
        NodeId,
        native::{DocumentReadyState, DomHost, NativeNodeId, Node},
    },
    frame_owner_model::{DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId},
    parser::HtmlParser,
    {
        host::{HostDocumentState, ScriptEventKind, ScriptEventTask},
        page_task_queue::{
            PageTask, PostParsePageOwnedWork, RendererPageMainDocumentRuntimeAction,
            WindowScriptFailureReportTask,
        },
        planning::ScriptSource,
        types::{
            ScriptKind, ScriptMode, ScriptSchedulingInput, ScriptSkipReason, ScriptSourceKind,
            classify_script_mode,
        },
    },
};
use url::Url;

fn test_main_document_owner() -> FrameDocumentTaskOwner {
    FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
}

fn page_bound_script_scheduler(
    queue: &crate::page_task_queue::PageTaskQueueTestHarness,
) -> HostScriptScheduler {
    let runtime_sender = queue.owner_attached_runtime_page_task_sender_for_test();
    let mut scripts =
        HostScriptScheduler::with_page_task_injection(runtime_sender.page_task_sender());
    assert!(scripts.bind_main_document_runtime_producer(test_main_document_owner()));
    scripts
}

fn take_main_document_runtime_work(
    queue: &crate::page_task_queue::PageTaskQueueTestHarness,
) -> Option<PostParsePageOwnedWork> {
    let task = queue
        .task_sources()
        .take_main_document_runtime_for_executor_test()?;
    let RendererPageMainDocumentRuntimeAction::ExecuteReadyPostParseWork(work) = task.into_action()
    else {
        panic!("script host event must enqueue concrete post-parse work")
    };
    Some(work.into_post_parse_work())
}

#[test]
fn main_document_completion_recheck_reservation_coalesces_until_its_turn_begins() {
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    let owner = test_main_document_owner();

    assert!(scripts.enqueue_main_document_completion_recheck(owner));
    assert!(scripts.enqueue_main_document_completion_recheck(owner));
    assert!(matches!(
        take_main_document_runtime_work(&queue)
            .as_ref()
            .and_then(PostParsePageOwnedWork::as_lifecycle_work),
        Some(crate::page_task_queue::PostParseLifecycleWork::CheckMainDocumentCompletion {
            owner: queued_owner,
        }) if *queued_owner == owner
    ));
    assert!(take_main_document_runtime_work(&queue).is_none());

    scripts.begin_main_document_completion_recheck_turn();
    assert!(scripts.enqueue_main_document_completion_recheck(owner));
    assert!(take_main_document_runtime_work(&queue).is_some());
}

fn preparation(url: &str, _document: NodeId) -> RuntimeScriptPreparationContext {
    let document_url = Url::parse(url).expect("test url should parse");
    RuntimeScriptPreparationContext {
        base_url: document_url.clone(),
        document_url,
        fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
    }
}

#[test]
fn dynamic_scripts_classify_into_async_and_in_order_modes() {
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: true,
            async_attribute_present: false,
            defer_attribute_present: false,
            kind: ScriptKind::Classic,
            source_kind: ScriptSourceKind::External,
        }),
        Some(ScriptMode::Async)
    );
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: false,
            async_attribute_present: false,
            defer_attribute_present: false,
            kind: ScriptKind::Classic,
            source_kind: ScriptSourceKind::External,
        }),
        Some(ScriptMode::InOrder)
    );
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: false,
            async_attribute_present: true,
            defer_attribute_present: false,
            kind: ScriptKind::Module,
            source_kind: ScriptSourceKind::External,
        }),
        Some(ScriptMode::Async)
    );
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: false,
            async_attribute_present: false,
            defer_attribute_present: false,
            kind: ScriptKind::Module,
            source_kind: ScriptSourceKind::External,
        }),
        Some(ScriptMode::ModuleInOrder)
    );
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: false,
            async_attribute_present: false,
            defer_attribute_present: false,
            kind: ScriptKind::ImportMap,
            source_kind: ScriptSourceKind::External,
        }),
        None
    );
    assert_eq!(
        classify_script_mode(ScriptSchedulingInput {
            parser_inserted: false,
            allow_parser_blocking_modes: false,
            force_async: false,
            async_attribute_present: false,
            defer_attribute_present: false,
            kind: ScriptKind::Classic,
            source_kind: ScriptSourceKind::Inline,
        }),
        Some(ScriptMode::Normal)
    );
}

#[test]
fn draining_dynamic_scripts_keeps_async_and_in_order_lanes_separate() {
    let preparation = preparation("https://example.test/", NodeId::new(0));
    let mut scheduler = HostScriptScheduler::default();
    scheduler
        .queue_dynamic_script(
            &preparation,
            "slow",
            "/slow.js",
            ScriptSourceKind::External,
            ScriptKind::Classic,
            ScriptMode::InOrder,
        )
        .expect("in-order script should queue");
    scheduler
        .queue_dynamic_script(
            &preparation,
            "fast",
            "/fast.js",
            ScriptSourceKind::External,
            ScriptKind::Classic,
            ScriptMode::Async,
        )
        .expect("async script should queue");

    let batch = scheduler.drain_dynamic_scripts();
    assert_eq!(batch.in_order.len(), 1);
    assert!(batch.importmap_in_order.is_empty());
    assert!(batch.module_in_order.is_empty());
    assert_eq!(batch.async_scripts.len(), 1);
    assert_eq!(batch.in_order[0].mode, ScriptMode::InOrder);
    assert_eq!(batch.async_scripts[0].mode, ScriptMode::Async);
}

#[test]
fn dynamic_importmap_registration_does_not_enter_script_batch() {
    let preparation = preparation("https://example.test/", NodeId::new(0));
    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_dynamic_import_map(&preparation, "{\"imports\":{\"fixture\":\"/mod.js\"}}");

    let batch = scheduler.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert!(batch.importmap_in_order.is_empty());
    assert!(batch.module_in_order.is_empty());
    assert_eq!(
        scheduler
            .resolve_module_specifier("fixture", &preparation.base_url)
            .expect("registered import map should resolve"),
        Url::parse("https://example.test/mod.js").expect("mapped url")
    );
}

#[test]
fn draining_dynamic_module_scripts_keeps_distinct_module_lane() {
    let preparation = preparation("https://example.test/", NodeId::new(0));
    let mut scheduler = HostScriptScheduler::default();
    scheduler
        .queue_dynamic_script(
            &preparation,
            "module",
            "/mod.js",
            ScriptSourceKind::External,
            ScriptKind::Module,
            ScriptMode::ModuleInOrder,
        )
        .expect("module script should queue");

    let batch = scheduler.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert_eq!(batch.module_in_order.len(), 1);
    assert_eq!(batch.module_in_order[0].mode, ScriptMode::ModuleInOrder);
}

#[test]
fn queueing_dynamic_module_does_not_block_later_import_map_merge() {
    let preparation = preparation("https://example.test/", NodeId::new(0));
    let mut scheduler = HostScriptScheduler::default();
    scheduler
        .queue_dynamic_script(
            &preparation,
            "module",
            "/mod.js",
            ScriptSourceKind::External,
            ScriptKind::Module,
            ScriptMode::Async,
        )
        .expect("module script should queue");
    scheduler.register_dynamic_import_map(&preparation, "{\"imports\":{\"late\":\"/late.mjs\"}}");

    let batch = scheduler.drain_dynamic_scripts();
    assert_eq!(batch.async_scripts.len(), 1);
    assert!(batch.importmap_in_order.is_empty());
    assert_eq!(
        scheduler
            .resolve_module_specifier("late", &preparation.base_url)
            .expect("later import map should resolve a new specifier")
            .as_str(),
        "https://example.test/late.mjs"
    );
}

#[test]
fn prepared_runtime_script_start_captures_document_and_base_urls_at_prepare_time() {
    let initial_url =
        Url::parse("https://example.test/base/").expect("initial test url should parse");
    let document = HtmlParser.parse(
        initial_url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "relative.js"),
        "src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut document_state = HostDocumentState::new(initial_url);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "dynamic-script-handle",
        script,
        ScriptHandleSource::RuntimeOwned,
    );

    let prepared = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, script);
    document_state
        .set_url(Url::parse("https://mutated.test/other/").expect("mutated test url should parse"));

    assert_eq!(
        prepared
            .execute(&mut dom_host, &mut scripts, "dynamic-script-handle")
            .expect("prepared start should succeed"),
        None
    );

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.async_scripts.len(), 1);
    assert_eq!(
        batch.async_scripts[0].url,
        Url::parse("https://example.test/base/relative.js")
            .expect("prepared url should resolve against initial base")
    );
    assert_eq!(
        batch.async_scripts[0].initiator_url,
        Url::parse("https://example.test/base/")
            .expect("prepared initiator should retain the initial document URL")
    );
}

#[test]
fn parser_inserted_classification_differs_from_dynamic_classification() {
    let parser_mode = classify_script_mode(ScriptSchedulingInput {
        parser_inserted: true,
        allow_parser_blocking_modes: true,
        force_async: false,
        async_attribute_present: false,
        defer_attribute_present: false,
        kind: ScriptKind::Classic,
        source_kind: ScriptSourceKind::External,
    });
    let dynamic_mode = classify_script_mode(ScriptSchedulingInput {
        parser_inserted: false,
        allow_parser_blocking_modes: false,
        force_async: false,
        async_attribute_present: false,
        defer_attribute_present: false,
        kind: ScriptKind::Classic,
        source_kind: ScriptSourceKind::External,
    });

    assert_eq!(parser_mode, Some(ScriptMode::Normal));
    assert_eq!(dynamic_mode, Some(ScriptMode::InOrder));
}

#[test]
fn parser_history_can_survive_without_falling_back_to_parser_owned_modes() {
    let runtime_connected_mode = classify_script_mode(ScriptSchedulingInput {
        parser_inserted: true,
        allow_parser_blocking_modes: false,
        force_async: true,
        async_attribute_present: false,
        defer_attribute_present: false,
        kind: ScriptKind::Classic,
        source_kind: ScriptSourceKind::External,
    });

    assert_eq!(runtime_connected_mode, Some(ScriptMode::InOrder));
}

#[test]
fn runtime_script_start_uses_language_attribute_when_type_is_absent() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body><script language='javascript'>window.ok = true;</script><script language=' javascript '>window.bad = true;</script></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let document_state = HostDocumentState::new(url);
    let handles: Vec<_> = document
        .nodes()
        .iter()
        .filter(|node| node.is_html_element_named("script"))
        .map(|node| node.id())
        .collect();

    let first = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, handles[0]);
    assert!(matches!(
        first.decision,
        RuntimeScriptStartDecision::ExecuteInlineClassic { .. }
    ));

    let second = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, handles[1]);
    assert!(matches!(
        second.decision,
        RuntimeScriptStartDecision::Skip {
            reason: Some(ScriptSkipReason::UnsupportedType(_)),
            ..
        }
    ));
}

#[test]
fn runtime_script_start_applies_legacy_for_event_gate() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body><script for=' window ' event=' onload '>window.ok = true;</script><script for='window' event='onclick'>window.bad = true;</script></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let document_state = HostDocumentState::new(url);
    let handles: Vec<_> = document
        .nodes()
        .iter()
        .filter(|node| node.is_html_element_named("script"))
        .map(|node| node.id())
        .collect();

    let first = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, handles[0]);
    assert!(matches!(
        first.decision,
        RuntimeScriptStartDecision::ExecuteInlineClassic { .. }
    ));

    let second = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, handles[1]);
    assert!(matches!(
        second.decision,
        RuntimeScriptStartDecision::Skip {
            reason: Some(ScriptSkipReason::UnsupportedType(_)),
            ..
        }
    ));
}

#[test]
fn runtime_svg_script_ignores_html_only_classification_attributes() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let document_state = HostDocumentState::new(url);
    let body = document.document_body_handle().expect("body should exist");
    let script = dom_host
        .create_element_ns(Some("http://www.w3.org/2000/svg"), "script")
        .expect("SVG script should be creatable");
    for (name, value) in [
        ("nomodule", ""),
        ("defer", ""),
        ("language", "application/json"),
        ("for", "not-window"),
        ("event", "onclick"),
    ] {
        assert!(
            dom_host.set_attribute(script, name, value),
            "{name} should be set"
        );
    }
    assert!(
        dom_host.set_text_content(script, "window.svgScriptRan = true;"),
        "SVG script text should be set"
    );
    assert!(
        dom_host.append_child(body, script),
        "SVG script should connect"
    );

    let prepared = PreparedRuntimeScriptStart::analyze(&mut dom_host, &document_state, script);

    assert!(matches!(
        prepared.decision,
        RuntimeScriptStartDecision::ExecuteInlineClassic { .. }
    ));
}

#[test]
fn runtime_script_start_uses_direct_text_children_only() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let document_state = HostDocumentState::new(url);
    let body = document.document_body_handle().expect("body should exist");
    let script = dom_host.create_element("script");
    let span = dom_host.create_element("span");
    let descendant_text = dom_host.create_text_node("window.descendant = true;");
    assert!(dom_host.append_child(span, descendant_text));
    assert!(dom_host.append_child(script, span));
    assert!(dom_host.append_child(body, script));
    let mut scripts = HostScriptScheduler::default();

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "nested-text-script-handle",
        )
        .expect("nested text prepare should not fail"),
        None
    );
    assert!(
        !dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "descendant-only text should not mark the script already-started"
    );

    let text = dom_host.create_text_node("window.direct = true;");
    assert!(
        dom_host.append_child(script, text),
        "direct text child should append"
    );
    let source = prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        script,
        "nested-text-script-handle",
    )
    .expect("direct text prepare should not fail")
    .expect("direct text should start classic script");

    assert_eq!(source, "window.direct = true;");
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "direct text execution should mark the script already-started"
    );
}

#[test]
fn runtime_owned_in_order_scripts_created_while_loading_do_not_wait_for_domcontentloaded() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "/runtime.js"),
        "src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_script_handle_with_source(
        "runtime-script-handle",
        script,
        ScriptHandleSource::RuntimeOwned,
    );

    scheduler
        .queue_dynamic_script(
            &RuntimeScriptPreparationContext {
                document_url: Url::parse("https://example.test/").expect("test url should parse"),
                base_url: Url::parse("https://example.test/").expect("test url should parse"),
                fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            },
            "runtime-script-handle",
            "/runtime.js",
            ScriptSourceKind::External,
            ScriptKind::Classic,
            ScriptMode::InOrder,
        )
        .expect("runtime-owned in-order script should queue");

    assert!(
        !scheduler.script_handle_waits_until_dom_content_loaded("runtime-script-handle"),
        "runtime-owned in-order scripts are not parser ordered and must not wait for DOMContentLoaded"
    );
}

#[test]
fn runtime_owned_module_scripts_created_while_loading_do_not_wait_for_domcontentloaded() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "type", "module"),
        "type should set"
    );
    assert!(
        dom_host.set_attribute(script, "src", "/runtime.mjs"),
        "src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_script_handle_with_source(
        "runtime-module-handle",
        script,
        ScriptHandleSource::RuntimeOwned,
    );

    scheduler
        .queue_dynamic_script(
            &RuntimeScriptPreparationContext {
                document_url: Url::parse("https://example.test/").expect("test url should parse"),
                base_url: Url::parse("https://example.test/").expect("test url should parse"),
                fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            },
            "runtime-module-handle",
            "/runtime.mjs",
            ScriptSourceKind::External,
            ScriptKind::Module,
            ScriptMode::ModuleInOrder,
        )
        .expect("runtime-owned module script should queue");

    assert!(
        !scheduler.script_handle_waits_until_dom_content_loaded("runtime-module-handle"),
        "runtime-owned modules are not parser ordered and must not wait for DOMContentLoaded"
    );
}

#[test]
fn runtime_owned_in_order_scripts_created_after_loading_do_not_set_domcontentloaded_wait_flag() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "/runtime.js"),
        "src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_script_handle_with_source(
        "runtime-script-handle-after-loading",
        script,
        ScriptHandleSource::RuntimeOwned,
    );

    scheduler
        .queue_dynamic_script(
            &preparation("https://example.test/", NodeId::new(0)),
            "runtime-script-handle-after-loading",
            "/runtime.js",
            ScriptSourceKind::External,
            ScriptKind::Classic,
            ScriptMode::InOrder,
        )
        .expect("runtime-owned in-order script should queue");

    assert!(
        !scheduler
            .script_handle_waits_until_dom_content_loaded("runtime-script-handle-after-loading"),
        "runtime-owned in-order scripts created after loading should stay off the DOMContentLoaded gate"
    );
}

#[test]
fn runtime_owned_handles_do_not_wait_for_blocking_stylesheets() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_script_handle_with_source(
        "runtime-stylesheet-handle",
        script,
        ScriptHandleSource::RuntimeOwned,
    );

    assert!(
        !scheduler.script_handle_waits_for_blocking_stylesheets("runtime-stylesheet-handle"),
        "runtime-owned handles should not be stylesheet-blocking eligible"
    );
}

#[test]
fn parser_owned_handles_wait_for_blocking_stylesheets() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(dom_host.append_child(head, script), "script should connect");

    let mut scheduler = HostScriptScheduler::default();
    scheduler.register_script_handle_with_source(
        "parser-stylesheet-handle",
        script,
        ScriptHandleSource::ParserOwned,
    );

    assert!(
        scheduler.script_handle_waits_for_blocking_stylesheets("parser-stylesheet-handle"),
        "parser-owned handles should remain stylesheet-blocking eligible"
    );
}

#[test]
fn runtime_preparation_capture_uses_live_document_base_url() {
    let url = Url::parse("https://example.test/page/index.html").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut document_state = HostDocumentState::new(url);
    document_state.set_ready_state(DocumentReadyState::Interactive);
    assert!(
        dom_host.set_document_ready_state(DocumentReadyState::Loading),
        "live dom readyState should switch back to loading"
    );

    let base = dom_host.create_element("base");
    assert!(
        dom_host.set_attribute(base, "href", "../scripts/"),
        "base href should set"
    );
    let head = document.document_head_handle().expect("head should exist");
    assert!(dom_host.append_child(head, base), "base should connect");

    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "/runtime.js"),
        "src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let captured = RuntimeScriptPreparationContext::capture(&dom_host, &document_state, script);

    assert_eq!(
        captured.base_url,
        Url::parse("https://example.test/scripts/").expect("expected base URL should parse")
    );
    assert_eq!(
        captured.document_url,
        Url::parse("https://example.test/page/index.html")
            .expect("expected document URL should parse"),
        "a live base element must not replace the owning Document URL used for request attribution"
    );
    assert!(
        !captured.fetch_metadata.parser_inserted,
        "a script created through the DOM API must capture not-parser-inserted metadata"
    );
}

#[test]
fn connected_datablock_type_mutation_alone_does_not_prepare() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "type", "application/json"),
        "data-block type should set"
    );
    assert!(
        dom_host.set_text_content(script, "window.dataBlockBecameClassic = true;"),
        "text content should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let effects = dom_host.set_attribute_effects(script, "type", "text/javascript");

    assert!(effects.did_change(), "type mutation should change the DOM");
    assert!(
        effects.scripts().prepare_triggers().is_empty(),
        "changing type on an already-connected data block must not prepare it"
    );
}

#[test]
fn parser_created_script_type_mutation_alone_does_not_prepare() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url,
        "<!doctype html><html><head><script type=\"text/plain\">window.ran = true;</script></head></html>"
            .to_owned(),
    );
    let script = document
        .script_handles()
        .into_iter()
        .next()
        .expect("parsed script should exist");
    let mut dom_host = DomHost::from_dom(document);

    let effects = dom_host.remove_attribute_effects(script, "type");

    assert!(
        effects.scripts().prepare_triggers().is_empty(),
        "parser-created data-block script should not start from type mutation alone"
    );
}

#[test]
fn parser_created_script_text_mutation_after_type_change_still_prepares() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url,
        "<!doctype html><html><head><script type=\"text/plain\">window.ran = true;</script></head></html>"
            .to_owned(),
    );
    let script = document
        .script_handles()
        .into_iter()
        .next()
        .expect("parsed script should exist");
    let mut dom_host = DomHost::from_dom(document);
    let _ = dom_host.remove_attribute_effects(script, "type");

    let effects = dom_host.set_text_content_effects(script, "window.ran = true;\n");

    assert_eq!(
        effects.scripts().prepare_triggers().len(),
        1,
        "child/text mutation after type removal should still prepare the script"
    );
}

#[test]
fn parser_created_datablock_prepare_consumes_parser_inserted_state() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head><script type=\"text/plain\" src=\"/data.txt\"></script></head></html>"
            .to_owned(),
    );
    let script = document
        .script_handles()
        .into_iter()
        .next()
        .expect("parsed script should exist");
    let mut dom_host = DomHost::from_dom(document);
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url);

    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_parser_inserted_for_prepare()),
        "parser-created script starts with prepare parser-inserted state"
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| !element.script_async()),
        "DOM storage does not classify parser-created data-block scripts"
    );

    let prepared = prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        script,
        "parser-created-data-block-handle",
    )
    .expect("data-block prepare should not fail");

    assert_eq!(prepared, None);
    let element = dom_host
        .node(script)
        .and_then(Node::as_element)
        .expect("script should remain in DOM");
    assert!(
        !element.script_parser_inserted_for_prepare(),
        "inert data-block prepare should consume parser-inserted state"
    );
    assert!(
        element.script_async(),
        "inert data-block prepare should leave force-async observable"
    );
    assert!(
        !element.script_already_started(),
        "inert data-block prepare should remain startable"
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(
        batch.in_order.is_empty()
            && batch.async_scripts.is_empty()
            && batch.importmap_in_order.is_empty()
            && batch.module_in_order.is_empty(),
        "inert data-block prepare should not enqueue script work: {batch:?}"
    );
}

#[test]
fn parser_created_external_script_reactivation_keeps_in_order_prepare_position() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head><script id=\"test\" type=\"text/plain\" src=\"/one.js\"></script></head></html>"
            .to_owned(),
    );
    let original = document
        .script_handles()
        .into_iter()
        .next()
        .expect("parsed script should exist");
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            original,
            "original-script-handle",
        )
        .expect("the original data-block preparation should succeed"),
        None,
        "the original data block should remain non-executable"
    );
    let marker = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(marker, "src", "/two.js"),
        "marker src should set"
    );
    assert!(
        dom_host.set_script_force_async(marker, false),
        "marker.async = false should clear force-async"
    );
    assert!(
        dom_host.append_child(head, marker),
        "marker should connect after the original script"
    );

    prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        marker,
        "marker-script-handle",
    )
    .expect("marker prepare should queue");
    let _ = dom_host.remove_attribute_effects(original, "type");
    assert!(
        dom_host.set_script_force_async(original, false),
        "script.async = false should clear force-async before preparation"
    );
    let empty_text = dom_host.create_text_node("");
    assert!(
        dom_host.append_child(original, empty_text),
        "text mutation should trigger the original data-block script"
    );
    prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        original,
        "original-script-handle",
    )
    .expect("reactivated parser-created script should queue");

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.in_order.len(), 2);
    assert_eq!(
        batch.in_order[0].node_id, marker,
        "marker should stay first because it enters the ordered dynamic script queue first"
    );
    assert_eq!(batch.in_order[0].url, url.join("/two.js").unwrap());
    assert_eq!(batch.in_order[1].node_id, original);
    assert_eq!(batch.in_order[1].url, url.join("/one.js").unwrap());
}

#[test]
fn parser_created_external_script_reactivation_without_async_false_stays_async() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head><script id=\"test\" type=\"text/plain\" src=\"/one.js\"></script></head></html>"
            .to_owned(),
    );
    let original = document
        .script_handles()
        .into_iter()
        .next()
        .expect("parsed script should exist");
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            original,
            "original-script-handle",
        )
        .expect("the original data-block preparation should succeed"),
        None,
        "the original data block should remain non-executable"
    );
    let marker = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(marker, "src", "/two.js"),
        "marker src should set"
    );
    assert!(
        dom_host.set_script_force_async(marker, false),
        "marker.async = false should clear force-async"
    );
    assert!(
        dom_host.append_child(head, marker),
        "marker should connect after the original script"
    );

    prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        marker,
        "marker-script-handle",
    )
    .expect("marker prepare should queue");
    let _ = dom_host.remove_attribute_effects(original, "type");
    let empty_text = dom_host.create_text_node("");
    assert!(
        dom_host.append_child(original, empty_text),
        "text mutation should trigger the original data-block script"
    );
    prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        original,
        "original-script-handle",
    )
    .expect("reactivated parser-created script should queue");

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.in_order.len(), 1);
    assert_eq!(batch.in_order[0].node_id, marker);
    assert_eq!(batch.in_order[0].url, url.join("/two.js").unwrap());
    assert_eq!(batch.async_scripts.len(), 1);
    assert_eq!(batch.async_scripts[0].node_id, original);
    assert_eq!(batch.async_scripts[0].url, url.join("/one.js").unwrap());
}

#[test]
fn prepare_script_start_invalid_external_src_queues_error_and_commits_start() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "http://[::1"),
        "invalid src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("invalid external source should queue an error"),
        None
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "failed script preparation should still commit already-started"
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::QueueFailed
        ))
    );
    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.failed_scripts.len(), 1);
    let failed = &batch.failed_scripts[0];
    assert_eq!(failed.script.url, url);
    assert_eq!(failed.script.initiator_url, url);
    assert!(failed.message.contains("failed to resolve script src"));
    assert_eq!(failed.failure_kind, QueuedScriptFailureKind::Immediate);

    assert!(
        dom_host.set_attribute(script, "src", "/fixed.js"),
        "fixed src should set"
    );
    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("already-started script should remain inert"),
        None
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert!(batch.async_scripts.is_empty());
    assert!(batch.failed_scripts.is_empty());
}

#[test]
fn prepare_script_start_empty_src_queues_async_error() {
    let url = Url::parse("https://example.test/page.html").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(dom_host.set_attribute(script, "src", ""));
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "empty-src-script-handle",
        )
        .expect("empty src should queue a failed script"),
        None
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "empty src failure should still commit already-started"
    );
    assert_eq!(
        scripts.script_start_state("empty-src-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::QueueFailed
        ))
    );

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.failed_scripts.len(), 1);
    let failed = &batch.failed_scripts[0];
    assert_eq!(failed.script.source_kind, ScriptSourceKind::External);
    assert!(matches!(failed.script.source, ScriptSource::External));
    assert_eq!(failed.script.url, url);
    assert!(failed.message.contains("empty script src"));
    assert_eq!(failed.failure_kind, QueuedScriptFailureKind::Immediate);
}

#[test]
fn prepare_module_empty_src_queues_top_level_load_failure() {
    let url = Url::parse("https://example.test/page.html").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url);
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(dom_host.set_attribute(script, "type", "module"));
    assert!(dom_host.set_attribute(script, "src", ""));
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "empty-module-src-script-handle",
        )
        .expect("empty module src should queue a failed script"),
        None
    );

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.failed_scripts.len(), 1);
    let failed = &batch.failed_scripts[0];
    assert_eq!(failed.script.kind, ScriptKind::Module);
    assert_eq!(failed.script.source_kind, ScriptSourceKind::External);
    assert_eq!(
        failed.failure_kind,
        QueuedScriptFailureKind::ModuleTopLevelLoad
    );
}

#[test]
fn prepare_script_start_invalid_importmap_src_queues_error_and_commits_start() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "type", "importmap"),
        "importmap type should set"
    );
    assert!(
        dom_host.set_attribute(script, "src", "http://[::1"),
        "invalid importmap src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("invalid external importmap should queue an error"),
        None
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::RejectImportMap
        ))
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "failed importmap preparation should still commit already-started"
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.failed_scripts.is_empty());
    assert!(matches!(
        take_main_document_runtime_work(&queue).and_then(PostParsePageOwnedWork::into_page_task),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle,
        })) if handle == "dynamic-script-handle"
    ));

    assert!(
        dom_host.set_attribute(script, "src", "/map.json"),
        "fixed importmap src should set"
    );
    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("already-started importmap should remain inert"),
        None
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.failed_scripts.is_empty());
}

#[test]
fn prepare_script_start_registers_inline_importmap_without_script_queue() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "type", "importmap"),
        "importmap type should set"
    );
    let source = dom_host.create_text_node("{\"imports\":{\"fixture\":\"/module.js\"}}");
    assert!(
        dom_host.append_child(script, source),
        "source should attach"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("inline importmap registration should succeed"),
        None
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::RegisterImportMap
        ))
    );
    assert_eq!(
        scripts.script_host_event_subject("dynamic-script-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::RuntimeOwned,
            execution: ScriptHandleExecutionSubject::NonExecutable,
        }
    );
    assert_eq!(
        scripts
            .resolve_module_specifier("fixture", &url)
            .expect("registered map should resolve"),
        Url::parse("https://example.test/module.js").expect("mapped url")
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.importmap_in_order.is_empty());
    assert!(batch.failed_scripts.is_empty());
}

#[test]
fn invalid_inline_importmap_reports_failure_without_script_queue() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    let document_state = HostDocumentState::new(url);
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(dom_host.set_attribute(script, "type", "importmap"));
    let source = dom_host.create_text_node("not json");
    assert!(dom_host.append_child(script, source));
    assert!(dom_host.append_child(head, script));

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("invalid map reports asynchronously"),
        None
    );
    assert!(matches!(
        take_main_document_runtime_work(&queue).and_then(PostParsePageOwnedWork::into_page_task),
        Some(PageTask::ReportWindowScriptFailure(_))
    ));
    assert!(
        scripts
            .drain_dynamic_scripts()
            .importmap_in_order
            .is_empty()
    );
}

#[test]
fn prepare_script_start_nomodule_commits_skip_and_blocks_later_restart() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url);
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "nomodule", ""),
        "nomodule should set"
    );
    assert!(
        dom_host.set_text_content(script, "window.nomoduleShouldStayInert = true;"),
        "script text should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("nomodule prepare should succeed"),
        None
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "nomodule classic should commit already-started after start decision"
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::Skip
        ))
    );
    assert_eq!(
        scripts.script_host_event_subject("dynamic-script-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::RuntimeOwned,
            execution: ScriptHandleExecutionSubject::SkippedExecution,
        }
    );

    assert!(
        dom_host.remove_attribute(script, "nomodule"),
        "removing nomodule should mutate"
    );
    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("already-started prepare should succeed"),
        None
    );
    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert!(batch.importmap_in_order.is_empty());
    assert!(batch.module_in_order.is_empty());
    assert!(batch.async_scripts.is_empty());
}

#[test]
fn prepare_script_start_external_importmap_commits_error_without_script_work() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    let document_state = HostDocumentState::new(url);
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "type", "importmap"),
        "importmap type should set"
    );
    assert!(
        dom_host.set_attribute(script, "src", "/map.json"),
        "importmap src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("external importmap prepare should succeed"),
        None
    );
    assert!(
        dom_host
            .node(script)
            .and_then(Node::as_element)
            .is_some_and(|element| element.script_already_started()),
        "external importmap should commit already-started after unsupported start"
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::RejectImportMap
        ))
    );
    assert_eq!(
        scripts.script_host_event_subject("dynamic-script-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::RuntimeOwned,
            execution: ScriptHandleExecutionSubject::NonExecutable,
        }
    );

    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert!(batch.importmap_in_order.is_empty());
    assert!(batch.module_in_order.is_empty());
    assert!(batch.async_scripts.is_empty());
    assert!(batch.failed_scripts.is_empty());
    assert!(matches!(
        take_main_document_runtime_work(&queue).and_then(PostParsePageOwnedWork::into_page_task),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle,
        })) if handle == "dynamic-script-handle"
    ));
}

#[test]
fn prepare_script_start_committed_queue_stays_inert_across_src_mutation() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "/first.js"),
        "initial src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("initial queue should succeed"),
        None
    );
    assert!(
        dom_host.set_attribute(script, "src", "/second.js"),
        "src mutation should set"
    );

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("already-committed prepare should succeed"),
        None
    );

    let batch = scripts.drain_dynamic_scripts();
    assert!(batch.in_order.is_empty());
    assert_eq!(batch.async_scripts.len(), 1);
    assert_eq!(
        batch.async_scripts[0].url,
        url.join("/first.js").expect("first url should resolve")
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::Queue
        ))
    );
}

#[test]
fn prepare_script_start_reservation_blocks_orphaned_queue_entry_when_commit_is_denied() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url.clone());
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_attribute(script, "src", "/first.js"),
        "initial src should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("initial queue should succeed"),
        None
    );
    assert!(
        dom_host.set_script_already_started(script, false),
        "test should force DOM state out of sync"
    );
    assert!(
        dom_host.set_attribute(script, "src", "/second.js"),
        "src mutation should set"
    );

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("committed scheduler state should deny restart"),
        None
    );

    let batch = scripts.drain_dynamic_scripts();
    assert_eq!(batch.async_scripts.len(), 1);
    assert_eq!(
        batch.async_scripts[0].url,
        url.join("/first.js").expect("first url should resolve")
    );
}

#[test]
fn prepare_script_start_committed_inline_execution_stays_inert_after_reattach() {
    let url = Url::parse("https://example.test/").expect("test url should parse");
    let document = HtmlParser.parse(
        url.clone(),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    );
    let mut dom_host = DomHost::from_dom(document.clone());
    let mut scripts = HostScriptScheduler::default();
    let document_state = HostDocumentState::new(url);
    let head = document.document_head_handle().expect("head should exist");
    let script = dom_host.create_element("script");
    assert!(
        dom_host.set_text_content(script, "window.reattachShouldNotRerun = true;"),
        "script text should set"
    );
    assert!(dom_host.append_child(head, script), "script should connect");

    let source = prepare_script_start(
        &mut dom_host,
        &document_state,
        &mut scripts,
        script,
        "dynamic-script-handle",
    )
    .expect("inline classic should prepare")
    .expect("inline classic should execute");
    assert_eq!(source, "window.reattachShouldNotRerun = true;");

    assert!(dom_host.remove_child(head, script), "script should detach");
    assert!(
        dom_host.append_child(head, script),
        "script should reattach"
    );

    assert_eq!(
        prepare_script_start(
            &mut dom_host,
            &document_state,
            &mut scripts,
            script,
            "dynamic-script-handle",
        )
        .expect("reattach prepare should succeed"),
        None
    );
    assert_eq!(
        scripts.script_start_state("dynamic-script-handle"),
        Some(ScriptHandleStartState::Committed(
            ScriptStartCommitKind::ExecuteInline
        ))
    );
}

#[test]
fn specific_source_registration_overrides_unknown_handle_source() {
    let node = NativeNodeId::new(42);
    let mut scripts = HostScriptScheduler::default();

    scripts.register_script_handle_with_source("shared-handle", node, ScriptHandleSource::Unknown);
    assert_eq!(
        scripts.script_host_event_subject("shared-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::Unknown,
            execution: ScriptHandleExecutionSubject::PendingOrUnknown,
        }
    );

    scripts.register_script_handle_with_source(
        "shared-handle",
        node,
        ScriptHandleSource::DocumentWriteOwned,
    );

    assert_eq!(
        scripts.script_host_event_subject("shared-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::DocumentWriteOwned,
            execution: ScriptHandleExecutionSubject::PendingOrUnknown,
        }
    );
}

#[test]
fn execute_inline_commit_kind_skips_script_load_dispatch() {
    let node = NativeNodeId::new(7);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "inline-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("inline-handle", node));
    assert!(scripts.finish_script_start(
        "inline-handle",
        node,
        ScriptStartCommitKind::ExecuteInline
    ));

    assert_eq!(
        scripts.script_host_event_subject("inline-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::RuntimeOwned,
            execution: ScriptHandleExecutionSubject::InlineClassicExecution,
        }
    );
    assert_eq!(
        scripts.script_event_policy("inline-handle"),
        ScriptEventPolicy {
            load: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::InlineClassicLoad),
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    );
    assert_eq!(
        scripts
            .script_event_policy("inline-handle")
            .dispatch_policy(ScriptEventKind::Load),
        ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::InlineClassicLoad)
    );
    assert_eq!(
        scripts
            .script_event_policy("inline-handle")
            .dispatch_policy(ScriptEventKind::Error),
        ScriptEventDispatchPolicy::Dispatch
    );
}

#[test]
fn execute_prepared_commit_kind_maps_to_prepared_execution_subject() {
    let node = NativeNodeId::new(8);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "prepared-handle",
        node,
        ScriptHandleSource::ParserOwned,
    );
    assert!(scripts.reserve_script_start("prepared-handle", node));
    assert!(scripts.finish_script_start(
        "prepared-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    assert_eq!(
        scripts.script_host_event_subject("prepared-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::ParserOwned,
            execution: ScriptHandleExecutionSubject::PreparedExecution,
        }
    );
    assert_eq!(
        scripts.script_event_policy("prepared-handle"),
        ScriptEventPolicy {
            load: ScriptEventDispatchPolicy::Dispatch,
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    );
}

#[test]
fn inline_module_prepared_execution_skips_load_but_keeps_error_dispatch() {
    let node = NativeNodeId::new(80);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "inline-module-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("inline-module-handle", node));
    assert!(scripts.finish_script_start(
        "inline-module-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    assert_eq!(
        scripts.script_event_policy_for_script(
            ScriptKind::Module,
            ScriptSourceKind::Inline,
            Some("inline-module-handle"),
        ),
        ScriptEventPolicy {
            load: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::InlineModuleLoad),
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    );
    assert!(
        scripts
            .plan_script_event_task_for_script(
                ScriptEventKind::Load,
                ScriptKind::Module,
                ScriptSourceKind::Inline,
                "inline-module-handle",
            )
            .is_none(),
        "successful inline modules must not dispatch a load event"
    );
    assert!(matches!(
        scripts.plan_script_event_task_for_script(
            ScriptEventKind::Error,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
            "inline-module-handle",
        ),
        Some(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle,
        }) if handle == "inline-module-handle"
    ));
}

#[test]
fn queue_failed_commit_kind_maps_to_failed_queued_execution_subject() {
    let node = NativeNodeId::new(9);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "failed-handle",
        node,
        ScriptHandleSource::DocumentWriteOwned,
    );
    assert!(scripts.reserve_script_start("failed-handle", node));
    assert!(scripts.finish_script_start("failed-handle", node, ScriptStartCommitKind::QueueFailed));

    assert_eq!(
        scripts.script_host_event_subject("failed-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::DocumentWriteOwned,
            execution: ScriptHandleExecutionSubject::FailedQueuedExecution,
        }
    );
    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::ImportMap,
            ScriptSourceKind::Inline,
            Some("failed-handle"),
            "boom",
            None,
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Skip(
                ScriptEventSkipReason::InlineImportMapError
            ),
            report_window_failure: true,
            load_event_after_window_failure: false,
        }
    );
}

#[test]
fn runtime_owned_handle_defaults_to_runtime_source_before_commit() {
    let node = NativeNodeId::new(10);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "runtime-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );

    assert_eq!(
        scripts.script_host_event_subject("runtime-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::RuntimeOwned,
            execution: ScriptHandleExecutionSubject::PendingOrUnknown,
        }
    );
}

#[test]
fn execute_inline_commit_kind_skips_script_load_page_task_enqueue() {
    let node = NativeNodeId::new(10);
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    scripts.register_script_handle_with_source(
        "inline-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("inline-handle", node));
    assert!(scripts.finish_script_start(
        "inline-handle",
        node,
        ScriptStartCommitKind::ExecuteInline
    ));

    let task = scripts.plan_script_event_page_task(ScriptEventKind::Load, "inline-handle");

    assert!(task.is_none());
    assert!(take_main_document_runtime_work(&queue).is_none());
}

#[test]
fn script_event_page_tasks_preserve_production_runtime_fifo() {
    let node = NativeNodeId::new(11);
    let queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let mut scripts = page_bound_script_scheduler(&queue);
    scripts.register_script_handle_with_source(
        "external-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("external-handle", node));
    assert!(scripts.finish_script_start(
        "external-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let load_task = scripts
        .plan_script_event_page_task(ScriptEventKind::Load, "external-handle")
        .expect("load task should be planned");
    let error_task = scripts
        .plan_script_event_page_task(ScriptEventKind::Error, "external-handle")
        .expect("error task should be planned");
    scripts.enqueue_post_parse_lifecycle_page_task(load_task);
    scripts.enqueue_post_parse_lifecycle_page_task(error_task);

    assert!(matches!(
        take_main_document_runtime_work(&queue).and_then(PostParsePageOwnedWork::into_page_task),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Load,
            handle
        })) if handle == "external-handle"
    ));
    assert!(matches!(
        take_main_document_runtime_work(&queue).and_then(PostParsePageOwnedWork::into_page_task),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle
        })) if handle == "external-handle"
    ));
}

#[test]
fn classic_script_failure_plans_only_error_task() {
    let node = NativeNodeId::new(12);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "classic-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("classic-handle", node));
    assert!(scripts.finish_script_start(
        "classic-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Classic,
        ScriptSourceKind::External,
        Some("classic-handle"),
        "boom",
        Some("/classic.js"),
        None,
        None,
    );

    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle
        })) if handle == "classic-handle"
    ));
}

#[test]
fn module_graph_failure_plans_only_window_report_task() {
    let node = NativeNodeId::new(13);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "module-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("module-handle", node));
    assert!(scripts.finish_script_start(
        "module-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Module,
        ScriptSourceKind::External,
        Some("module-handle"),
        "ModuleLinkFailed: module `/dep.mjs` does not export `missing`",
        Some("/module.mjs"),
        None,
        None,
    );

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-handle"),
            "ModuleLinkFailed: module `/dep.mjs` does not export `missing`",
            None,
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::ModuleGraphFailure),
            report_window_failure: true,
            load_event_after_window_failure: true,
        }
    );
    assert_eq!(tasks.len(), 2);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::ReportWindowScriptFailure(WindowScriptFailureReportTask {
            message,
            filename,
            ..
        })) if message == "ModuleLinkFailed: module `/dep.mjs` does not export `missing`"
            && filename.as_deref() == Some("/module.mjs")
    ));
    assert!(matches!(
        tasks.get(1),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Load,
            handle
        })) if handle == "module-handle"
    ));
}

#[test]
fn inline_module_failure_policy_never_schedules_load() {
    let node = NativeNodeId::new(81);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "inline-module-failure-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("inline-module-failure-handle", node));
    assert!(scripts.finish_script_start(
        "inline-module-failure-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let graph_tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Module,
        ScriptSourceKind::Inline,
        Some("inline-module-failure-handle"),
        "module does not provide an export named `missing`",
        Some("/page.html"),
        Some(ModuleFailurePolicy::GraphFailure),
        None,
    );
    assert_eq!(graph_tasks.len(), 1);
    assert!(matches!(
        graph_tasks.first(),
        Some(PageTask::ReportWindowScriptFailure(_))
    ));

    let fetch_tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Module,
        ScriptSourceKind::Inline,
        Some("inline-module-failure-handle"),
        "module dependency returned HTTP 404",
        Some("/page.html"),
        Some(ModuleFailurePolicy::ModuleTreeLoadFailure),
        None,
    );
    assert_eq!(fetch_tasks.len(), 1);
    assert!(matches!(
        fetch_tasks.first(),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle,
        })) if handle == "inline-module-failure-handle"
    ));
}

#[test]
fn module_top_level_load_failure_plans_only_error_task() {
    let node = NativeNodeId::new(16);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "module-load-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("module-load-handle", node));
    assert!(scripts.finish_script_start(
        "module-load-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Module,
        ScriptSourceKind::External,
        Some("module-load-handle"),
        "script request `/module.mjs` returned HTTP 404",
        Some("/module.mjs"),
        None,
        None,
    );

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-load-handle"),
            "script request `/module.mjs` returned HTTP 404",
            None,
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Dispatch,
            report_window_failure: false,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle
        })) if handle == "module-load-handle"
    ));
}

#[test]
fn module_tree_load_failure_plans_only_error_task() {
    let node = NativeNodeId::new(18);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "module-tree-load-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("module-tree-load-handle", node));
    assert!(scripts.finish_script_start(
        "module-tree-load-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::Module,
        ScriptSourceKind::External,
        Some("module-tree-load-handle"),
        "module script `/dep.mjs` has invalid MIME type `text/plain`",
        Some("/module.mjs"),
        Some(ModuleFailurePolicy::ModuleTreeLoadFailure),
        None,
    );

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-tree-load-handle"),
            "module script `/dep.mjs` has invalid MIME type `text/plain`",
            Some(ModuleFailurePolicy::ModuleTreeLoadFailure),
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Dispatch,
            report_window_failure: false,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle
        })) if handle == "module-tree-load-handle"
    ));
}

#[test]
fn explicit_module_failure_policy_overrides_opaque_message_text() {
    let node = NativeNodeId::new(17);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "module-opaque",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("module-opaque", node));
    assert!(scripts.finish_script_start(
        "module-opaque",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-opaque"),
            "opaque failure",
            Some(ModuleFailurePolicy::TopLevelLoadFailure),
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Dispatch,
            report_window_failure: false,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-opaque"),
            "script request `/module.mjs` returned HTTP 404",
            Some(ModuleFailurePolicy::GraphFailure),
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::ModuleGraphFailure),
            report_window_failure: true,
            load_event_after_window_failure: true,
        }
    );
    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-opaque"),
            "script request `/module.mjs` returned HTTP 404",
            Some(ModuleFailurePolicy::ModuleTreeLoadFailure),
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Dispatch,
            report_window_failure: false,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::Module,
            ScriptSourceKind::External,
            Some("module-opaque"),
            "script request `/module.mjs` returned HTTP 404",
            Some(ModuleFailurePolicy::EvaluationFailure),
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Skip(ScriptEventSkipReason::ModuleGraphFailure),
            report_window_failure: true,
            load_event_after_window_failure: false,
        }
    );
}

#[test]
fn inline_importmap_failure_plans_only_window_report_task() {
    let node = NativeNodeId::new(15);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "importmap-handle",
        node,
        ScriptHandleSource::ParserOwned,
    );
    assert!(scripts.reserve_script_start("importmap-handle", node));
    assert!(scripts.finish_script_start("importmap-handle", node, ScriptStartCommitKind::Queue));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::ImportMap,
        ScriptSourceKind::Inline,
        Some("importmap-handle"),
        "boom",
        Some("/index.html"),
        None,
        None,
    );

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::ImportMap,
            ScriptSourceKind::Inline,
            Some("importmap-handle"),
            "boom",
            None,
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Skip(
                ScriptEventSkipReason::InlineImportMapError
            ),
            report_window_failure: true,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::ReportWindowScriptFailure(WindowScriptFailureReportTask {
            message,
            filename,
            ..
        })) if message == "boom" && filename.as_deref() == Some("/index.html")
    ));
}

#[test]
fn external_importmap_failure_plans_only_error_task() {
    let node = NativeNodeId::new(19);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "external-importmap-handle",
        node,
        ScriptHandleSource::RuntimeOwned,
    );
    assert!(scripts.reserve_script_start("external-importmap-handle", node));
    assert!(scripts.finish_script_start(
        "external-importmap-handle",
        node,
        ScriptStartCommitKind::QueueFailed
    ));

    let tasks = scripts.plan_script_failure_page_tasks(
        ScriptKind::ImportMap,
        ScriptSourceKind::External,
        Some("external-importmap-handle"),
        "external import maps are not supported",
        Some("/map.json"),
        None,
        None,
    );

    assert_eq!(
        scripts.script_failure_page_task_policy(
            ScriptKind::ImportMap,
            ScriptSourceKind::External,
            Some("external-importmap-handle"),
            "external import maps are not supported",
            None,
        ),
        ScriptFailurePageTaskPolicy {
            load_event: ScriptEventDispatchPolicy::Dispatch,
            error_event: ScriptEventDispatchPolicy::Dispatch,
            report_window_failure: false,
            load_event_after_window_failure: false,
        }
    );
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks.first(),
        Some(PageTask::DispatchScriptEvent(ScriptEventTask {
            kind: ScriptEventKind::Error,
            handle
        })) if handle == "external-importmap-handle"
    ));
}

#[test]
fn document_write_owned_prepared_execution_dispatches_script_events() {
    let node = NativeNodeId::new(14);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "document-write-prepared-handle",
        node,
        ScriptHandleSource::DocumentWriteOwned,
    );
    assert!(scripts.reserve_script_start("document-write-prepared-handle", node));
    assert!(scripts.finish_script_start(
        "document-write-prepared-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    assert_eq!(
        scripts.script_event_policy("document-write-prepared-handle"),
        ScriptEventPolicy {
            load: ScriptEventDispatchPolicy::Dispatch,
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    );
}

#[test]
fn unknown_source_prepared_execution_keeps_default_dispatch_policy() {
    let node = NativeNodeId::new(15);
    let mut scripts = HostScriptScheduler::default();
    scripts.register_script_handle_with_source(
        "unknown-prepared-handle",
        node,
        ScriptHandleSource::Unknown,
    );
    assert!(scripts.reserve_script_start("unknown-prepared-handle", node));
    assert!(scripts.finish_script_start(
        "unknown-prepared-handle",
        node,
        ScriptStartCommitKind::ExecutePrepared
    ));

    assert_eq!(
        scripts.script_host_event_subject("unknown-prepared-handle"),
        ScriptHostEventSubject {
            source: ScriptHandleSource::Unknown,
            execution: ScriptHandleExecutionSubject::PreparedExecution,
        }
    );
    assert_eq!(
        scripts.script_event_policy("unknown-prepared-handle"),
        ScriptEventPolicy {
            load: ScriptEventDispatchPolicy::Dispatch,
            error: ScriptEventDispatchPolicy::Dispatch,
        }
    );
}

#[test]
#[should_panic(expected = "script host-event planning requires a registered handle")]
fn plan_script_event_page_task_rejects_unregistered_handle() {
    let scripts = HostScriptScheduler::default();

    let _ = scripts.plan_script_event_page_task(ScriptEventKind::Load, "missing-handle");
}

#[test]
#[should_panic(expected = "script host-event planning requires a registered handle")]
fn plan_script_failure_page_tasks_rejects_unregistered_handle() {
    let scripts = HostScriptScheduler::default();

    let _ = scripts.plan_script_failure_page_tasks(
        ScriptKind::Classic,
        ScriptSourceKind::External,
        Some("missing-handle"),
        "boom",
        Some("/classic.js"),
        None,
        None,
    );
}
