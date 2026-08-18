use super::*;
#[cfg(test)]
use crate::DocumentBlockingStylesheetSignature;
use crate::document_script_scheduler::{
    ParseTimeTurn, ParseTimeTurnTrigger, ParseVisibleReadyTurnDisposition,
    ParseVisibleReadyTurnPhase,
};
use crate::live_document_parser::{DocumentParserSession, ParserStopReason};
use crate::page_task_queue::PostParsePageOwnedWork;
#[cfg(test)]
use crate::parser::ScriptSource;
#[cfg(test)]
use crate::parser::{ParserPumpStep, ParserScriptHandoff, ParserYield, PreparedScript};
#[cfg(test)]
use crate::planning::SharedScriptSourceLoad;
#[cfg(test)]
use crate::runtime::page_vm::{ScannedStylesheetAdmission, ScannedStylesheetDeferral};
#[cfg(test)]
use moli_fetch::FetchConfig;
use moli_fetch::StreamingRawResponse;
#[cfg(test)]
use std::collections::HashSet;

mod bootstrap;
mod document_turn;
mod loop_protocol;
mod owner_turn;
mod page_task_turn;
mod parser_blocking_document_script;
mod parser_blocking_execution;
mod parser_blocking_owner;
mod parser_blocking_pending;
mod parser_blocking_source;
mod parser_blocking_task;
mod parser_turn;
mod pending_residence;
mod phase_two;
mod scaffold;
mod state;
mod streaming;
mod streaming_input;
mod streaming_residence;
mod wait;

#[cfg(test)]
mod streaming_admission_tests;
#[cfg(test)]
use self::document_turn::PendingParsingBlockingWake;
use self::document_turn::{
    DocumentTurnContext, pending_parsing_blocking_wake_prefers_ready_task_drain,
};
pub(super) use self::loop_protocol::ParseTimePageVmCreationOutcome;
use self::loop_protocol::{
    ParseTimePageVmStreamingBootstrapOutcome, ParseTimePageVmStreamingProgress,
};
use self::loop_protocol::{ParseTimePhaseOnePump, ParseTimePhaseTransitionReason};
use self::owner_turn::{OwnerStepProgress, owner_step_progress_after_current_document_stop};
use self::page_task_turn::{
    execute_page_owned_document_script_failure_turn_on_local_task,
    execute_page_owned_document_script_turn_on_local_task,
    execute_page_owned_work_turn_on_local_task,
};
use self::parser_blocking_pending::PendingParsingBlockingClassicScriptRunner;
#[cfg(test)]
use self::parser_blocking_pending::{
    PendingParserBlockingSourceLoad, parser_blocking_classic_metadata_for_test,
    parser_blocking_classic_script_for_test, parser_blocking_classic_source_load_for_test,
};
#[cfg(test)]
use self::parser_blocking_source::{
    MainParserBlockingSourceDisposition, parser_blocking_script_can_start_external_source_load,
    prepare_main_parser_blocking_source_load,
};
use self::parser_turn::{PageTaskTurnResult, ParserDriver};
#[cfg(test)]
use self::parser_turn::{
    ParserStepAdvanceOutcome, ScriptHandoffOutcome, bind_parser_owned_script_handle,
};
pub(super) use self::pending_residence::{PendingPhaseOneResidence, PendingPhaseOneResumeOutcome};
pub(super) use self::state::ConcurrentParseTimeRuntime;
use self::state::{ParseTimeDriverState, ParseTimeOwner, PendingParsingBlockingWait};
pub use self::streaming::ExternalRawDocumentBodyStream;
pub(super) use self::streaming::{
    StreamingHtmlPageCreationResult, StreamingNavigationPageCreationResult,
    response_headers_indicate_download,
};
pub(super) use self::streaming_residence::PendingStreamingPhaseOneContinuation;
pub(super) use self::wait::{PhaseOneResidenceAdmission, PhaseOneRestoreRequirement};

#[cfg(test)]
mod document_write_resource_tests;
#[cfg(test)]
mod non_executable_script_tests;
#[cfg(test)]
use super::script_preloads::*;
use super::script_preloads::{BufferedDocumentPreloadState, ServiceWorkerScriptPreloadContext};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentOwnedBlockingStylesheetDiscoveryInput;
    use crate::document_runtime::{DocumentRuntime, DomHandle, ParserPostStepRuntimeWorkForTest};
    use crate::parser::ParserDomMutation;
    use moli_dom::native::{Element, Node};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct PhaseOnePageVmHarness {
        page_vm: PageVm,
        loader: &'static ResourceRequestClient,
        state: &'static mut ParseTimeDriverState,
    }

    pub(super) fn run_phase_one_large_stack_test<F>(thread_name: &'static str, test: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::Builder::new()
            .name(thread_name.to_owned())
            .stack_size(8 * 1024 * 1024)
            .spawn(test)
            .expect("large-stack phase-one test thread should spawn")
            .join()
            .expect("large-stack phase-one test thread should finish");
    }

    fn bind_preload_state_to_current_test_runtime(cache: &mut BufferedDocumentPreloadState) {
        cache.bind_resource_runtime(
            None,
            Some(
                crate::network::RendererResourceTaskRunner::from_current_tokio()
                    .expect("phase-one resource test requires its Tokio runtime"),
            ),
        );
    }

    fn solid_paint_rect(
        snapshot: &moli_layout::PaintSnapshot,
        expected: moli_layout::PaintColor,
    ) -> moli_layout::PaintRect {
        snapshot
            .fragments
            .iter()
            .find_map(|fragment| {
                fragment
                    .solid_fill_in_surface()
                    .filter(|(_, color)| *color == expected)
                    .map(|(rect, _)| rect)
            })
            .unwrap_or_else(|| panic!("missing {expected:?} in {:?}", snapshot.fragments))
    }

    fn assert_paint_rect(actual: moli_layout::PaintRect, expected: moli_layout::PaintRect) {
        for (name, actual, expected) in [
            ("x", actual.x, expected.x),
            ("y", actual.y, expected.y),
            ("width", actual.width, expected.width),
            ("height", actual.height, expected.height),
        ] {
            assert!(
                (actual - expected).abs() <= 0.01,
                "{name}: expected {expected}, got {actual}; rect={actual:?}"
            );
        }
    }

    fn rgb(red: u8, green: u8, blue: u8) -> moli_layout::PaintColor {
        moli_layout::PaintColor::new(
            f32::from(red) / 255.0,
            f32::from(green) / 255.0,
            f32::from(blue) / 255.0,
            1.0,
        )
    }

    async fn render_test_snapshot(html: &'static str) -> moli_layout::PaintSnapshot {
        let mut page = parse_phase_one_html_into_page_vm_for_test(html).await;
        page.vm_mut().sync_live_document_style_sources();
        page.vm_mut()
            .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
            .expect("test layout should succeed")
            .expect("test fixture should have a document element")
    }

    fn glyph_min_x(snapshot: &moli_layout::PaintSnapshot, color: moli_layout::PaintColor) -> f32 {
        snapshot
            .fragments
            .iter()
            .filter_map(|fragment| match fragment {
                moli_layout::PaintFragment::GlyphRun(run) if run.color == color => run
                    .glyphs_in_surface()
                    .into_iter()
                    .map(|glyph| glyph.x)
                    .reduce(f32::min),
                _ => None,
            })
            .reduce(f32::min)
            .unwrap_or_else(|| panic!("missing glyph run with {color:?}"))
    }

    fn new_phase_one_page_vm_harness_for_test() -> PhaseOnePageVmHarness {
        new_phase_one_page_vm_harness_for_test_with_env(default_test_page_vm_env_config())
    }

    fn new_phase_one_page_vm_harness_for_test_with_env(
        env: PageVmEnvConfig,
    ) -> PhaseOnePageVmHarness {
        let _js_runtime = crate::JsRuntime::initialize();
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = Box::leak(Box::new(
            ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
        ));
        let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
        let parser_dom_host = state
            .parser_session
            .stream_handle()
            .borrow_mut()
            .take_parser_stream_dom_host();
        let local_executor = JsLocalExecutor::new();
        let runtime_hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
        state.buffered_document_preloads.bind_resource_runtime(
            runtime_hooks.owner_wake(),
            runtime_hooks.resource_task_runner(),
        );
        let page_vm = PageVm::new(
            PageId::new_for_testing(1),
            local_executor,
            loader,
            &env,
            runtime_hooks,
            parser_dom_host,
            Instant::now(),
        )
        .expect("page vm");
        PhaseOnePageVmHarness {
            page_vm,
            loader,
            state,
        }
    }

    fn new_phase_one_page_vm_for_test() -> PageVm {
        new_phase_one_page_vm_harness_for_test().page_vm
    }

    fn activate_standalone_main_parser_continuation_for_test(page_vm: &mut PageVm) {
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("standalone parser fixture should bind its main Document owner");
        page_vm
            .vm_mut()
            .document_runtime
            .activate_main_parser_continuation(owner);
    }

    fn create_connected_html_body_for_test(page_vm: &mut PageVm) -> DomHandle {
        let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
        let document = dom_host.document_handle();
        let html = dom_host.create_parser_element_without_attributes(
            "html".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        let body = dom_host.create_parser_element_without_attributes(
            "body".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.append_child(document, html));
        assert!(dom_host.append_child(html, body));
        body
    }

    async fn run_element_toggle_tasks_for_test(
        page_vm: &mut PageVm,
        loader: &ResourceRequestClient,
        expected_count: usize,
        context: &str,
    ) {
        for _ in 0..expected_count {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        crate::runtime::page_vm::PageSelectedTaskTestSelector::DomManipulation(
                            crate::runtime::page_vm::PageDomManipulationTestFamily::ElementToggle,
                        ),
                        loader,
                    )
                    .await
                    .unwrap_or_else(|error| panic!("{context}: {error}")),
                "{context}: expected another element-toggle task"
            );
        }
        assert!(
            !page_vm
                .run_exact_selected_page_task_for_test(
                    crate::runtime::page_vm::PageSelectedTaskTestSelector::DomManipulation(
                        crate::runtime::page_vm::PageDomManipulationTestFamily::ElementToggle,
                    ),
                    loader,
                )
                .await
                .unwrap_or_else(|error| panic!("{context}: {error}")),
            "{context}: element-toggle source should contain exactly {expected_count} tasks"
        );
    }

    fn create_parser_resource_fragment_for_test(
        page_vm: &mut PageVm,
        id_prefix: &str,
    ) -> (DomHandle, DomHandle, DomHandle, DomHandle) {
        let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
        let fragment = dom_host.create_document_fragment();
        let container = dom_host.create_parser_element_without_attributes(
            "section".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.set_attribute(container, "id", &format!("{id_prefix}-root")));

        let image = dom_host.create_parser_element_without_attributes(
            "img".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.set_attribute(image, "id", &format!("{id_prefix}-image")));
        assert!(dom_host.set_attribute(
            image,
            "src",
            "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"
        ));

        let video = dom_host.create_parser_element_without_attributes(
            "video".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.set_attribute(video, "id", &format!("{id_prefix}-video")));
        assert!(dom_host.set_attribute(video, "controls", ""));
        assert!(dom_host.set_attribute(video, "loading", "lazy"));
        assert!(dom_host.set_attribute(video, "src", "data:video/mp4;base64,AAAA"));

        let track = dom_host.create_parser_element_without_attributes(
            "track".to_owned(),
            "http://www.w3.org/1999/xhtml".to_owned(),
            None,
        );
        assert!(dom_host.set_attribute(track, "id", &format!("{id_prefix}-track")));
        assert!(dom_host.set_attribute(track, "default", ""));
        assert!(dom_host.set_attribute(track, "src", "captions/en.vtt"));

        assert!(dom_host.append_child(video, track));
        assert!(dom_host.append_child(container, image));
        assert!(dom_host.append_child(container, video));
        assert!(dom_host.append_child(fragment, container));
        (fragment, container, image, video)
    }

    fn apply_parser_dom_mutation_for_test(
        page_vm: &mut PageVm,
        mutation: ParserDomMutation,
        context: &'static str,
    ) -> ParserPostStepRuntimeWorkForTest {
        let vm = page_vm.vm_mut();
        vm.with_dom_host_parse_step(|vm| {
            vm.apply_parser_dom_mutation_to_live_dom_host_in_default_context(mutation)
        })
        .expect(context);
        vm.document_runtime
            .take_pending_parser_post_step_runtime_work_for_test()
    }

    fn take_next_dom_manipulation_task_for_test(
        page_vm: &PageVm,
    ) -> crate::page_task_queue::RendererPageDomManipulationTask {
        let task = page_vm
            .page_task_executor_sources_for_test()
            .take_scheduler_task_for_executor_test(|descriptor| {
                matches!(
                    descriptor,
                    crate::page_task_queue::RendererPageReadyDescriptor::DomManipulation { .. }
                )
            })
            .expect("one DOM-manipulation task should remain queued");
        let crate::page_task_queue::RendererPageSchedulerTask::DomManipulation(task) = task else {
            unreachable!("DOM-manipulation descriptor must dequeue its own source")
        };
        task
    }

    fn apply_parser_dom_mutation_and_run_post_step_work_for_test(
        page_vm: &mut PageVm,
        mutation: ParserDomMutation,
        context: &'static str,
        post_step_context: &'static str,
    ) -> bool {
        let pending_work = apply_parser_dom_mutation_for_test(page_vm, mutation, context);
        let had_pending_work = !pending_work.is_empty();
        page_vm
            .vm_mut()
            .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                pending_work,
            )
            .expect(post_step_context);
        had_pending_work
    }

    async fn parse_phase_one_html_into_page_vm_for_test(html: &'static str) -> PageVm {
        parse_phase_one_html_into_page_vm_for_test_with_env(html, default_test_page_vm_env_config())
            .await
    }

    async fn parse_phase_one_html_into_page_vm_for_test_with_env(
        html: &'static str,
        env: PageVmEnvConfig,
    ) -> PageVm {
        let PhaseOnePageVmHarness {
            mut page_vm,
            loader,
            state,
        } = new_phase_one_page_vm_harness_for_test_with_env(env);
        let mut driver = ParserDriver {
            loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            input_closed: &state.input_closed,
        };

        let local_executor = page_vm.local_executor.clone();
        let page_vm_ptr: *mut PageVm = &mut page_vm;
        let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
        let outcome = super::access::run_named_owner_local_task(
            local_executor,
            "phase-one parser local task channel closed",
            async move {
                let page_vm = unsafe { &mut *page_vm_ptr };
                let driver = unsafe { &mut *driver_ptr };
                driver.advance_parser_step(page_vm, html, None).await
            },
        )
        .await
        .expect("parser step should complete");
        assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));
        page_vm
    }

    #[test]
    fn layout_renderer_constructs_phase_one_roles_from_native_dom_and_stylo() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body, main { display: block; margin: 0 }
#contents { display: contents }
#grid { display: grid }
#grid::before { content: "before"; display: inline }
#cell { display: table-cell }
#item { display: list-item }
#item::marker { content: "marker" }
#unboxed-object { display: contents }
#hidden-control { display: block }
#float { float: left }
#clear { clear: both }
#fixed { position: fixed }
#sticky { position: sticky }
</style></head><body><main id="root">
<div id="contents"><section id="grid">direct <span id="grid-child">child</span></section></div>
<span id="cell">cell</span><li id="item">item</li><input id="control" type="checkbox">
<button id="button">button</button><input id="hidden-control" type="hidden">
<object id="unboxed-object"><span id="object-fallback">must-not-layout</span></object>
<picture id="picture"><span id="picture-child">picture child</span></picture>
<div id="float"></div><div id="clear"></div><div id="fixed"></div><div id="sticky"></div>
</main></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let tree = page_vm
                .vm()
                .normalized_layout_box_tree_for_test()
                .expect("native layout construction should succeed")
                .expect("fixture should have a document element");
            let source_line = |source: &str| {
                tree.lines()
                    .find(|line| line.contains(source))
                    .unwrap_or_else(|| panic!("missing {source} in box tree:\n{tree}"))
            };
            assert!(tree.contains("principal-grid"), "{tree}");
            assert!(tree.contains("source=section#grid"), "{tree}");
            assert!(
                !source_line("source=section#grid").contains("capability=grid-layout-deferred"),
                "{tree}"
            );
            assert!(tree.contains("anonymous-grid-item"), "{tree}");
            assert!(tree.contains("pseudo-before"), "{tree}");
            assert!(tree.contains("anonymous-table-wrapper"), "{tree}");
            assert!(tree.contains("anonymous-table-row-group"), "{tree}");
            assert!(tree.contains("anonymous-table-row"), "{tree}");
            assert!(tree.contains("source=span#cell"), "{tree}");
            assert!(
                !source_line("source=span#cell").contains("capability=table-layout-deferred"),
                "{tree}"
            );
            assert!(tree.contains("pseudo-marker"), "{tree}");
            assert!(
                !source_line("source=li#item").contains("capability=list-marker-layout-deferred"),
                "{tree}"
            );
            assert!(tree.contains("source=input#control"), "{tree}");
            assert!(tree.contains("category=form-input-checkbox"), "{tree}");
            assert!(tree.contains("replaced=form-control"), "{tree}");
            assert!(
                tree.contains("display=inline-block source=button#button"),
                "{tree}"
            );
            assert!(!tree.contains("source=div#contents"), "{tree}");
            assert!(!tree.contains("source=input#hidden-control"), "{tree}");
            assert!(!tree.contains("source=object#unboxed-object"), "{tree}");
            assert!(!tree.contains("source=span#object-fallback"), "{tree}");
            assert!(!tree.contains("source=picture#picture"), "{tree}");
            assert!(tree.contains("source=span#picture-child"), "{tree}");
            for source in ["source=div#float", "source=div#clear"] {
                assert!(
                    !source_line(source).contains("capability=float-or-clear-layout-deferred"),
                    "{tree}"
                );
            }
            assert!(
                !source_line("source=div#fixed")
                    .contains("capability=fixed-position-layout-deferred"),
                "{tree}"
            );
            assert!(
                !source_line("source=div#sticky")
                    .contains("capability=sticky-position-layout-deferred"),
                "{tree}"
            );
        }));
    }

    #[test]
    fn layout_renderer_computes_phase_two_grid_calc_and_positioned_geometry_from_stylo() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { display: block; margin: 0; padding: 0 }
body { background: rgb(0, 0, 255) }
#grid { display: grid; width: 400px; height: 200px; gap: 20px 10px;
  grid-template-columns: 100px 1fr; grid-template-rows: 50px 1fr; grid-auto-rows: 30px }
#first { background: rgb(255, 0, 0); grid-column: 1; grid-row: 1 }
#second { background: rgb(0, 255, 0); grid-column: 2; grid-row: 1 / 3 }
#implicit { background: rgb(0, 128, 128); grid-column: 1; grid-row: 3 }
#calc { background: rgb(255, 255, 0); width: calc(50% - 10px); height: 20px }
#positioned { box-sizing: content-box; position: relative; margin: 30px;
  width: 400px; height: 300px; padding: 20px; border: 5px solid transparent }
#static { margin: 10px; width: 200px; height: 100px }
#absolute { position: absolute; left: 10%; top: 25%; width: 50%; height: 10px;
  background: rgb(0, 255, 255) }
#fixed { position: fixed; right: 10px; bottom: 20px; width: 30px; height: 40px;
  background: rgb(255, 0, 255) }
</style></head><body><div id="grid"><div id="first"></div><div id="second"></div><div id="implicit"></div></div><div id="calc"></div><div id="positioned"><div id="static"><div id="absolute"></div></div><div id="fixed"></div></div></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("native layout should succeed")
                .expect("fixture should have a document element");
            assert_eq!(
                snapshot.canvas_color,
                moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0)
            );
            assert_eq!(
                snapshot.content_size,
                moli_layout::PaintSize::new(800.0, 630.0)
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 100.0, 50.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(110.0, 0.0, 290.0, 150.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(
                        0.0,
                        128.0 / 255.0,
                        128.0 / 255.0,
                        1.0,
                    ),
                ),
                moli_layout::PaintRect::new(0.0, 170.0, 100.0, 30.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 200.0, 390.0, 20.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(79.0, 340.0, 220.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(760.0, 540.0, 30.0, 40.0),
            );
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "grid-layout-deferred"
                    && diagnostic.code != "fixed-position-layout-deferred"
            }));
        }));
    }

    #[test]
    fn layout_renderer_projects_static_position_and_flex_order_from_stylo() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { display: block; margin: 0; padding: 0 }
#static { position: static; left: 80px; top: 40px; width: 20px; height: 10px;
  background: rgb(255, 0, 0) }
#flex { display: flex; width: 300px; height: 40px; align-items: center }
#late { order: 2; flex: 1 1 100px; height: 20px; background: rgb(0, 0, 255) }
#early { order: -1; flex: 3 1 100px; height: 40px; background: rgb(0, 255, 0) }
#cb { position: relative; width: 200px; height: 100px }
#mid { margin: 10px; width: 80px; height: 30px }
#auto-static { position: absolute; width: 20px; height: 10px;
  background: rgb(255, 255, 0) }
#intrinsic { width: min-content; height: 10px; min-height: min-content }
#basis { display: flex }
#basis-child { flex: 0 0 content; min-width: 0; width: 10px; height: 10px;
  background: rgb(255, 128, 0) }
#basis-content { width: 80px; height: 10px }
#clamped { width: 90%; max-width: 240px; height: 10px; margin-left: 30px;
  background: rgb(128, 0, 128) }
</style></head><body><div id="static"></div><div id="flex"><div id="late"></div><div id="early"></div></div><div id="cb"><div id="mid"><div id="auto-static"></div></div></div><div id="intrinsic"></div><div id="basis"><div id="basis-child"><div id="basis-content"></div></div></div><div id="clamped"></div></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("native layout should succeed")
                .expect("fixture should have a document element");

            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 128.0 / 255.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 170.0, 80.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 0.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 20.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 10.0, 175.0, 40.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(0.0, 0.0, 1.0, 1.0),
                ),
                moli_layout::PaintRect::new(175.0, 20.0, 125.0, 20.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(1.0, 1.0, 0.0, 1.0),
                ),
                moli_layout::PaintRect::new(10.0, 60.0, 20.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(
                        128.0 / 255.0,
                        0.0,
                        128.0 / 255.0,
                        1.0,
                    ),
                ),
                moli_layout::PaintRect::new(30.0, 180.0, 240.0, 10.0),
            );
            assert!(snapshot.diagnostics.iter().all(|diagnostic| {
                diagnostic.code != "positioned-static-position-deferred"
                    || !diagnostic.message.contains("div#auto-static")
            }));
            assert!(snapshot.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "intrinsic-sizing-keyword-deferred"
                    && diagnostic.message.contains("div#intrinsic")
            }));
            assert!(snapshot
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "flex-basis-content-deferred"));
        }));
    }

    #[test]
    fn layout_renderer_preserves_calc_min_width_in_float_intrinsic_contribution() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#host{width:0}
#outer{float:left;height:30px;background:rgb(101,102,103)}
#inner{width:40px;min-width:calc(160px + 0%);height:20px;background:rgb(111,112,113)}
#definite{clear:both;width:200px;height:20px}
#definite-child{width:40px;min-width:calc(20px + 50%);height:20px;background:rgb(121,122,123)}
</style></head><body><div id=host><div id=outer><div id=inner></div></div></div>
<div id=definite><div id=definite-child></div></div></body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("float intrinsic layout should succeed")
                .expect("fixture should have a document element");
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(101.0 / 255.0, 102.0 / 255.0, 103.0 / 255.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 160.0, 30.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(111.0 / 255.0, 112.0 / 255.0, 113.0 / 255.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 0.0, 160.0, 20.0),
            );
            assert_paint_rect(
                solid_paint_rect(
                    &snapshot,
                    moli_layout::PaintColor::new(121.0 / 255.0, 122.0 / 255.0, 123.0 / 255.0, 1.0),
                ),
                moli_layout::PaintRect::new(0.0, 30.0, 120.0, 20.0),
            );
        }));
    }

    #[test]
    fn layout_renderer_clamps_definite_ifc_probes_to_intrinsic_contributions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{display:flow-root}.zero{width:0}.wide{width:400px}
.outer{float:left;height:30px}.inner{height:20px}
.atom{display:inline-block;width:60px;height:20px}
#zero-outer{background:rgb(131,132,133)}#wide-outer{background:rgb(141,142,143)}
</style></head><body>
<div class="host zero"><div id=zero-outer class=outer><div class=inner><span class=atom></span></div></div></div>
<div class="host wide"><div id=wide-outer class=outer><div class=inner><span class=atom></span></div></div></div>
</body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("intrinsic IFC probe layout should succeed")
                .expect("fixture should have a document element");
            for (color, expected) in [
                (
                    moli_layout::PaintColor::new(
                        131.0 / 255.0,
                        132.0 / 255.0,
                        133.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        141.0 / 255.0,
                        142.0 / 255.0,
                        143.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 60.0, 30.0),
                ),
            ] {
                assert_paint_rect(solid_paint_rect(&snapshot, color), expected);
            }
        }));
    }

    #[test]
    fn layout_renderer_resolves_cyclic_preferred_width_after_parent_contribution() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{display:flow-root;width:0}.outer{float:left;width:auto;height:30px}
.inner{height:20px}.raw{width:50%}.calc{width:calc(40px + 0%)}
.atom{display:inline-block;width:60px;height:20px}
#raw-outer{background:rgb(151,152,153)}#raw-inner{background:rgb(161,162,163)}
#calc-outer{background:rgb(171,172,173)}#calc-inner{background:rgb(181,182,183)}
</style></head><body>
<div class=host><div id=raw-outer class=outer><div id=raw-inner class="inner raw"><span class=atom></span></div></div></div>
<div class=host><div id=calc-outer class=outer><div id=calc-inner class="inner calc"><span class=atom></span></div></div></div>
</body></html>"#,
            )
            .await;
            page_vm.vm_mut().sync_live_document_style_sources();

            let snapshot = page_vm
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("cyclic preferred-width layout should succeed")
                .expect("fixture should have a document element");
            for (color, expected) in [
                (
                    moli_layout::PaintColor::new(
                        151.0 / 255.0,
                        152.0 / 255.0,
                        153.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        161.0 / 255.0,
                        162.0 / 255.0,
                        163.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 0.0, 30.0, 20.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        171.0 / 255.0,
                        172.0 / 255.0,
                        173.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 60.0, 30.0),
                ),
                (
                    moli_layout::PaintColor::new(
                        181.0 / 255.0,
                        182.0 / 255.0,
                        183.0 / 255.0,
                        1.0,
                    ),
                    moli_layout::PaintRect::new(0.0, 30.0, 40.0, 20.0),
                ),
            ] {
                assert_paint_rect(solid_paint_rect(&snapshot, color), expected);
            }
        }));
    }

    #[test]
    fn fixed_table_layout_distributes_unresolved_columns_equally() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#calc-cell{width:calc(50% - 20px);background:rgb(61,62,63)}
#remaining-cell{background:rgb(71,72,73)}
</style></head><body><table><tr><td id=calc-cell></td><td id=remaining-cell></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(61, 62, 63)),
                moli_layout::PaintRect::new(0.0, 0.0, 150.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(71, 72, 73)),
                moli_layout::PaintRect::new(150.0, 0.0, 150.0, 10.0),
            );
        }));
    }

    #[test]
    fn fixed_table_layout_distributes_first_row_colspan_constraints() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{table-layout:fixed;border-spacing:0}td{border:0;padding:0;height:10px}
#lengths{width:300px}#l0{background:rgb(10,20,30)}#l1{background:rgb(20,30,40)}#l2{background:rgb(30,40,50)}#l3{background:rgb(40,50,60)}
#percents{width:500px}#p0{background:rgb(50,60,70)}#p1{background:rgb(60,70,80)}#p2{background:rgb(70,80,90)}#p3{background:rgb(80,90,100)}#p4{background:rgb(90,100,110)}
#priority{width:300px}#c0{background:rgb(100,110,120)}#c1{background:rgb(110,120,130)}#c2{background:rgb(120,130,140)}
#spacing{width:350px;border-spacing:10px}#s0{background:rgb(130,140,150)}#s1{background:rgb(140,150,160)}#s2{background:rgb(150,160,170)}#s3{background:rgb(160,170,180)}
</style></head><body>
<table id=lengths><tr><td colspan=2 style="width:100px"></td><td colspan=2 style="width:200px"></td></tr><tr><td id=l0></td><td id=l1></td><td id=l2></td><td id=l3></td></tr></table>
<table id=percents><tr><td colspan=2 style="width:40%"></td><td colspan=2 style="width:20%"></td><td style="width:40%"></td></tr><tr><td id=p0></td><td id=p1></td><td id=p2></td><td id=p3></td><td id=p4></td></tr></table>
<table id=priority><col style="width:80px"><col><col><tr><td colspan=2 style="width:200px"></td><td></td></tr><tr><td id=c0></td><td id=c1></td><td id=c2></td></tr></table>
<table id=spacing><tr><td colspan=2 style="width:110px"></td><td colspan=2 style="width:210px"></td></tr><tr><td id=s0></td><td id=s1></td><td id=s2></td><td id=s3></td></tr></table>
</body></html>"#,
            )
            .await;

            // These values are Chromium's fixed-table constraint geometry.
            // Wide first-row cells contribute one constraint that is divided
            // over their tracks; explicit columns retain priority, and inner
            // border-spacing is removed before division.
            for (color, expected) in [
                (rgb(10, 20, 30), (0.0, 10.0, 50.0)),
                (rgb(20, 30, 40), (50.0, 10.0, 50.0)),
                (rgb(30, 40, 50), (100.0, 10.0, 100.0)),
                (rgb(40, 50, 60), (200.0, 10.0, 100.0)),
                (rgb(50, 60, 70), (0.0, 30.0, 100.0)),
                (rgb(60, 70, 80), (100.0, 30.0, 100.0)),
                (rgb(70, 80, 90), (200.0, 30.0, 50.0)),
                (rgb(80, 90, 100), (250.0, 30.0, 50.0)),
                (rgb(90, 100, 110), (300.0, 30.0, 200.0)),
                (rgb(100, 110, 120), (0.0, 50.0, 80.0)),
                (rgb(110, 120, 130), (80.0, 50.0, 100.0)),
                (rgb(120, 130, 140), (180.0, 50.0, 120.0)),
                (rgb(130, 140, 150), (10.0, 90.0, 50.0)),
                (rgb(140, 150, 160), (70.0, 90.0, 50.0)),
                (rgb(150, 160, 170), (130.0, 90.0, 100.0)),
                (rgb(160, 170, 180), (240.0, 90.0, 100.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
    }

    #[test]
    fn fixed_table_layout_grows_to_its_definite_column_minimum() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
col{width:200px}td{padding:0;border:0;height:10px}
#first{background:rgb(81,82,83)}#second{background:rgb(91,92,93)}
#collapsed{border-collapse:collapse}
#collapsed-first{background:rgb(171,172,173)}#collapsed-second{background:rgb(181,182,183)}
</style></head><body>
<table><col><col><tr><td id=first></td><td id=second></td></tr></table>
<table id=collapsed><col><col><tr><td id=collapsed-first></td><td id=collapsed-second></td></tr></table>
</body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(81, 82, 83)),
                moli_layout::PaintRect::new(0.0, 0.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(91, 92, 93)),
                moli_layout::PaintRect::new(200.0, 0.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(171, 172, 173)),
                moli_layout::PaintRect::new(0.0, 10.0, 200.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(181, 182, 183)),
                moli_layout::PaintRect::new(200.0, 10.0, 200.0, 10.0),
            );
        }));
    }

    #[test]
    fn fixed_table_layout_grows_fixed_columns_when_no_auto_column_remains() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:400px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#first{width:50px;background:rgb(101,102,103)}
#second{width:100px;background:rgb(111,112,113)}
#third{width:25%;background:rgb(121,122,123)}
</style></head><body><table><tr><td id=first></td><td id=second></td><td id=third></td></tr></table></body></html>"#,
            )
            .await;

            for (color, expected) in [
                (rgb(101, 102, 103), (0.0, 100.0)),
                (rgb(111, 112, 113), (100.0, 200.0)),
                (rgb(121, 122, 123), (300.0, 100.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, 0.0, expected.1, 10.0),
                );
            }
        }));
    }

    #[test]
    fn fixed_table_layout_adds_content_box_padding_to_percent_cell_measure() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:300px;table-layout:fixed}
td{border:0;height:10px;padding-top:0;padding-bottom:0}
#first{box-sizing:content-box;width:50%;padding-left:10px;padding-right:10px;background:rgb(131,132,133)}
#second{padding-left:0;padding-right:0;background:rgb(141,142,143)}
</style></head><body><table><tr><td id=first></td><td id=second></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(131, 132, 133)),
                moli_layout::PaintRect::new(0.0, 0.0, 170.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(141, 142, 143)),
                moli_layout::PaintRect::new(170.0, 0.0, 130.0, 10.0),
            );
        }));
    }

    #[test]
    fn fixed_table_layout_clamps_border_box_cell_width_to_its_insets() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{border-spacing:0;width:100px;table-layout:fixed}
td{border:0;height:10px;padding-top:0;padding-bottom:0}
#first{box-sizing:border-box;width:10px;padding-left:20px;padding-right:20px;background:rgb(151,152,153)}
#second{padding-left:0;padding-right:0;background:rgb(161,162,163)}
</style></head><body><table><tr><td id=first></td><td id=second></td></tr></table></body></html>"#,
            )
            .await;

            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(151, 152, 153)),
                moli_layout::PaintRect::new(0.0, 0.0, 40.0, 10.0),
            );
            assert_paint_rect(
                solid_paint_rect(&snapshot, rgb(161, 162, 163)),
                moli_layout::PaintRect::new(40.0, 0.0, 60.0, 10.0),
            );
        }));
    }

    #[test]
    fn fixed_table_layout_preserves_native_and_explicit_box_sizing() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
table{position:absolute;left:0;border:4px solid black;border-spacing:0;width:100px;table-layout:fixed}
td{padding:0;border:0;height:10px}
#native{top:0}#content{top:30px;box-sizing:content-box}
#native-first{background:rgb(191,192,193)}#native-second{background:rgb(201,202,203)}
#content-first{background:rgb(211,212,213)}#content-second{background:rgb(221,222,223)}
</style></head><body>
<table id=native><tr><td id=native-first></td><td id=native-second></td></tr></table>
<table id=content><tr><td id=content-first></td><td id=content-second></td></tr></table>
</body></html>"#,
            )
            .await;

            for (color, expected) in [
                (rgb(191, 192, 193), (4.0, 4.0, 46.0)),
                (rgb(201, 202, 203), (50.0, 4.0, 46.0)),
                (rgb(211, 212, 213), (4.0, 34.0, 50.0)),
                (rgb(221, 222, 223), (54.0, 34.0, 50.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&snapshot, color),
                    moli_layout::PaintRect::new(expected.0, expected.1, expected.2, 10.0),
                );
            }
        }));
    }

    #[test]
    fn layout_renderer_projects_current_blitz_taffy_parity_from_stylo() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let rgb = |red: u8, green: u8, blue: u8| {
                moli_layout::PaintColor::new(
                    f32::from(red) / 255.0,
                    f32::from(green) / 255.0,
                    f32::from(blue) / 255.0,
                    1.0,
                )
            };

            let mut inline_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
.host{width:200px;height:40px;font-size:0;line-height:0}
.atom{display:inline-block;vertical-align:top;width:20px;height:10px;position:relative}
#ltr-atomic{left:10%;right:40px;top:25%;bottom:20px;background:rgb(201,1,1)}
#rtl{direction:rtl;text-align:left}#rtl-atomic{direction:ltr;left:10px;right:10%;top:auto;bottom:5px;background:rgb(1,201,1)}
#right-row{display:flex;width:200px;height:20px;justify-content:right}#right-item{width:20px;height:10px;background:rgb(1,1,201)}
#column-row{display:flex;flex-direction:column;width:50px;height:100px;justify-content:right}#column-item{width:20px;height:20px;background:rgb(201,201,1)}
#self-grid{display:grid;width:200px;height:20px}#self-item{width:20px;height:10px;direction:rtl;justify-self:self-start;background:rgb(201,1,201)}
</style></head><body><div id=ltr class=host><div id=ltr-atomic class=atom></div></div>
<div id=rtl class=host><div id=rtl-atomic class=atom></div></div>
<div id=right-row><div id=right-item></div></div>
<div id=column-row><div id=column-item></div></div>
<div id=self-grid><div id=self-item></div></div></body></html>"#,
            )
            .await;
            inline_page.vm_mut().sync_live_document_style_sources();
            let inline = inline_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("inline/alignment layout should succeed")
                .expect("inline/alignment fixture should have a document element");
            for (color, expected) in [
                (rgb(201, 1, 1), (20.0, 10.0, 20.0, 10.0)),
                (rgb(1, 201, 1), (-20.0, 35.0, 20.0, 10.0)),
                (rgb(1, 1, 201), (180.0, 80.0, 20.0, 10.0)),
                (rgb(201, 201, 1), (0.0, 100.0, 20.0, 20.0)),
                (rgb(201, 1, 201), (180.0, 200.0, 20.0, 10.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&inline, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut flow_grid_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#flow{display:flow-root;width:100px;background:rgb(11,12,13)}#float{float:left;width:40px;height:30px;background:rgb(21,22,23)}#after{width:10px;height:5px;background:rgb(31,32,33)}
#areas{display:grid;width:300px;height:40px;grid-template-areas:'a a b';grid-template-columns:50px 100px 150px;grid-template-rows:40px}
#area-a{grid-area:a;background:rgb(41,42,43)}#area-b{grid-area:b;background:rgb(51,52,53)}
</style></head><body><div id=flow><div id=float></div></div><div id=after></div>
<div id=areas><div id=area-a></div><div id=area-b></div></div></body></html>"#,
            )
            .await;
            flow_grid_page
                .vm_mut()
                .sync_live_document_style_sources();
            let flow_grid = flow_grid_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("flow-root/grid-area layout should succeed")
                .expect("flow-root/grid-area fixture should have a document element");
            for (color, expected) in [
                (rgb(11, 12, 13), (0.0, 0.0, 100.0, 30.0)),
                (rgb(21, 22, 23), (0.0, 0.0, 40.0, 30.0)),
                (rgb(31, 32, 33), (0.0, 30.0, 10.0, 5.0)),
                (rgb(41, 42, 43), (0.0, 35.0, 150.0, 40.0)),
                (rgb(51, 52, 53), (150.0, 35.0, 150.0, 40.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&flow_grid, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut replaced_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
#image{display:block;width:120px;height:auto;aspect-ratio:0/1;background:rgb(81,82,83)}
</style></head><body><svg id=image width=80 height=40 viewBox="0 0 80 40"></svg></body></html>"#,
            )
            .await;
            replaced_page
                .vm_mut()
                .sync_live_document_style_sources();
            let replaced = replaced_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(800, 600, 1.0))
                .expect("replaced ratio layout should succeed")
                .expect("replaced ratio fixture should have a document element");
            assert_paint_rect(
                solid_paint_rect(&replaced, rgb(81, 82, 83)),
                moli_layout::PaintRect::new(0.0, 0.0, 120.0, 60.0),
            );
        }));
    }

    #[test]
    fn table_ua_defaults_match_chromium_spacing_indent_and_border_color() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let snapshot = render_test_snapshot(
                r#"<!doctype html><html><head><style>
html,body{margin:0;padding:0}
body{text-indent:30px}
td{width:10px;height:10px;padding:0;border:0;font-size:0}
#first{background:rgb(21,31,41)}#second{background:rgb(22,32,42)}
#indent-probe{display:inline-block;width:4px;height:4px;background:rgb(23,33,43)}
#bordered{border-style:solid;border-width:1px}
</style></head><body>
<table><tr><td id=first><span id=indent-probe></span></td><td id=second></td></tr></table>
<table id=bordered><tr><td></td></tr></table>
</body></html>"#,
            )
            .await;

            let first = solid_paint_rect(&snapshot, rgb(21, 31, 41));
            let second = solid_paint_rect(&snapshot, rgb(22, 32, 42));
            let indent_probe = solid_paint_rect(&snapshot, rgb(23, 33, 43));
            assert_paint_rect(first, moli_layout::PaintRect::new(2.0, 2.0, 10.0, 10.0));
            assert_paint_rect(second, moli_layout::PaintRect::new(14.0, 2.0, 10.0, 10.0));
            assert!(
                (indent_probe.x - first.x).abs() <= 0.01,
                "table should reset inherited text-indent: first={first:?}, probe={indent_probe:?}"
            );

            let gray = rgb(128, 128, 128);
            assert!(
                snapshot.fragments.iter().any(|fragment| matches!(
                    fragment,
                    moli_layout::PaintFragment::Border { widths, colors, .. }
                        if widths.top == 1.0
                            && widths.right == 1.0
                            && widths.bottom == 1.0
                            && widths.left == 1.0
                            && colors.top == gray
                            && colors.right == gray
                            && colors.bottom == gray
                            && colors.left == gray
                )),
                "table should inherit Chromium's gray UA border color: {:?}",
                snapshot.fragments
            );
        }));
    }

    #[test]
    fn layout_renderer_computes_phase_four_special_formatting_geometry_from_native_dom_and_stylo() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let rgb = |red: u8, green: u8, blue: u8| {
                moli_layout::PaintColor::new(
                    f32::from(red) / 255.0,
                    f32::from(green) / 255.0,
                    f32::from(blue) / 255.0,
                    1.0,
                )
            };

            let mut table_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#table { border-spacing: 5px 7px; width: 300px; table-layout: fixed; background: rgb(1,2,3) }
#caption { height: 20px; background: rgb(11,12,13) }
#columns { background: rgb(21,22,23) } #column-a { width: 80px; background: rgb(31,32,33) }
#column-bc { background: rgb(41,42,43) } #body { background: rgb(51,52,53) }
#first { height: 30px; background: rgb(61,62,63) }
#second { height: 40px; background: rgb(71,72,73) }
td { padding: 0; border: 0 } #a { background: rgb(81,82,83) }
#b { background: rgb(91,92,93) } #c { background: rgb(101,102,103) }
#d { background: rgb(111,112,113) }
</style></head><body><table id="table"><caption id="caption">cap</caption>
<colgroup id="columns"><col id="column-a"><col id="column-bc" span="2"></colgroup>
<tbody id="body"><tr id="first"><td id="a" rowspan="2">A</td><td id="b" colspan="2">B</td></tr>
<tr id="second"><td id="c">C</td><td id="d">D</td></tr></tbody></table></body></html>"#,
            )
            .await;
            table_page.vm_mut().sync_live_document_style_sources();
            let table = table_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(400, 240, 1.0))
                .expect("table layout should succeed")
                .expect("table fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 300.0, 111.0)),
                (rgb(11, 12, 13), (0.0, 0.0, 300.0, 20.0)),
                (rgb(21, 22, 23), (5.0, 27.0, 290.0, 77.0)),
                (rgb(31, 32, 33), (5.0, 27.0, 80.0, 77.0)),
                (rgb(41, 42, 43), (90.0, 27.0, 205.0, 77.0)),
                (rgb(51, 52, 53), (5.0, 27.0, 290.0, 77.0)),
                (rgb(61, 62, 63), (5.0, 27.0, 290.0, 30.0)),
                (rgb(71, 72, 73), (5.0, 64.0, 290.0, 40.0)),
                (rgb(81, 82, 83), (5.0, 27.0, 80.0, 77.0)),
                (rgb(91, 92, 93), (90.0, 27.0, 205.0, 30.0)),
                (rgb(101, 102, 103), (90.0, 64.0, 100.0, 40.0)),
                (rgb(111, 112, 113), (195.0, 64.0, 100.0, 40.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&table, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut collapsed_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#collapsed { border-collapse: collapse }
#collapsed td { width: 20px; height: 10px; border: 2px solid black }
</style></head><body><table id="collapsed"><tr><td>A</td><td>B</td></tr></table></body></html>"#,
            )
            .await;
            collapsed_page.vm_mut().sync_live_document_style_sources();
            let collapsed = collapsed_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(160, 100, 1.0))
                .expect("collapsed table should remain renderable")
                .expect("collapsed table fixture should have a document element");
            assert!(collapsed
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "collapsed-table-border-fallback"));
            assert!(collapsed.fragments.iter().any(|fragment| matches!(
                fragment,
                moli_layout::PaintFragment::Border { widths, .. }
                    if widths.top == 2.0 || widths.left == 2.0
            )));

            let mut list_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#list { margin: 0; padding-left: 40px; width: 200px; font-size: 20px; line-height: 20px }
li::marker { color: rgb(201,202,203) }
#first { color: rgb(121,122,123); background: rgb(1,2,3) }
#valued { color: rgb(131,132,133); background: rgb(11,12,13) }
#inside { color: rgb(141,142,143); background: rgb(21,22,23); list-style-position: inside }
#custom { color: rgb(151,152,153); background: rgb(31,32,33); list-style: none }
#custom::marker { content: "X "; color: rgb(201,202,203) }
</style></head><body><ol id="list" reversed start="5"><li id="first">AA</li><li id="valued" value="9">BB</li>
<li id="inside">CC</li><li id="custom">DD</li></ol></body></html>"#,
            )
            .await;
            list_page.vm_mut().sync_live_document_style_sources();
            let list = list_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(280, 180, 1.0))
                .expect("list marker layout should succeed")
                .expect("list fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (40.0, 0.0, 200.0, 20.0)),
                (rgb(11, 12, 13), (40.0, 20.0, 200.0, 20.0)),
                (rgb(21, 22, 23), (40.0, 40.0, 200.0, 20.0)),
                (rgb(31, 32, 33), (40.0, 60.0, 200.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&list, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
            let first_text_x = glyph_min_x(&list, rgb(121, 122, 123));
            let valued_text_x = glyph_min_x(&list, rgb(131, 132, 133));
            let inside_text_x = glyph_min_x(&list, rgb(141, 142, 143));
            let custom_text_x = glyph_min_x(&list, rgb(151, 152, 153));
            assert!((first_text_x - 40.0).abs() <= 0.01);
            assert!((valued_text_x - 40.0).abs() <= 0.01);
            assert!(inside_text_x > first_text_x);
            assert!((custom_text_x - 40.0).abs() <= 0.01);
            assert!(glyph_min_x(&list, rgb(201, 202, 203)) < 40.0);
            assert!(
                list.diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "list-marker-layout-deferred")
            );

            let mut flow_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
/* A zero-size font makes the 4px half-leading below the baseline deterministic. */
html, body { margin: 0; padding: 0; font-size: 0; line-height: 8px }
#flow { display: flow-root; width: 200px }
#left { float: left; width: 60px; height: 40px; background: rgb(1,2,3) }
#right { float: right; width: 50px; height: 30px; background: rgb(11,12,13) }
.atom { display: inline-block; height: 20px }
#first-atom { width: 84px; background: rgb(21,22,23) }
#second-atom { width: 60px; background: rgb(31,32,33) }
#clear-root { width: 200px } #clear-float { float: left; width: 70px; height: 35px; background: rgb(41,42,43) }
#clear { clear: both; height: 10px; background: rgb(51,52,53) }
</style></head><body><div id="flow"><div id="left"></div><div id="right"></div><span id="first-atom" class="atom"></span><span id="second-atom" class="atom"></span></div>
<div id="clear-root"><div id="clear-float"></div><div id="clear"></div></div></body></html>"#,
            )
            .await;
            flow_page.vm_mut().sync_live_document_style_sources();
            let flow = flow_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(240, 180, 1.0))
                .expect("float layout should succeed")
                .expect("float fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 60.0, 40.0)),
                (rgb(11, 12, 13), (150.0, 0.0, 50.0, 30.0)),
                (rgb(21, 22, 23), (60.0, 0.0, 84.0, 20.0)),
                (rgb(31, 32, 33), (60.0, 24.0, 60.0, 20.0)),
                (rgb(41, 42, 43), (0.0, 48.0, 70.0, 35.0)),
                (rgb(51, 52, 53), (0.0, 83.0, 200.0, 10.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&flow, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut replaced_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
img, canvas, iframe, svg { display: block; margin: 0; border: 0; padding: 0 }
#image { width: 120px; height: auto; background: rgb(1,2,3) }
#canvas { background: rgb(11,12,13) } #frame { background: rgb(21,22,23) }
#svg { background: rgb(31,32,33) }
</style></head><body><img id="image" width="80" height="40" alt=""><canvas id="canvas" width="600"></canvas>
<iframe id="frame" width="90" height="45"></iframe><svg id="svg" width="70" height="35"></svg></body></html>"#,
            )
            .await;
            replaced_page.vm_mut().sync_live_document_style_sources();
            let replaced = replaced_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(700, 700, 1.0))
                .expect("replaced layout should succeed")
                .expect("replaced fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 120.0, 60.0)),
                (rgb(11, 12, 13), (0.0, 60.0, 600.0, 150.0)),
                (rgb(21, 22, 23), (0.0, 210.0, 90.0, 45.0)),
                (rgb(31, 32, 33), (0.0, 255.0, 70.0, 35.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&replaced, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
            assert_eq!(
                replaced
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code == "replaced-content-placeholder")
                    .count(),
                1,
                "the unavailable image must retain its placeholder while the live initial-empty iframe is composed"
            );
            assert_eq!(
                replaced
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.code == "canvas-content-unavailable")
                    .count(),
                1,
                "unavailable canvas pixels must retain a transparent-fallback diagnostic"
            );

            let mut controls_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
input, textarea, select, button { display: block; box-sizing: content-box; margin: 0; border: 0; padding: 0; font-size: 20px; line-height: 20px }
#input { background: rgb(1,2,3) } #textarea { background: rgb(11,12,13) }
#select { background: rgb(21,22,23) } #checkbox { background: rgb(31,32,33) }
#radio { background: rgb(41,42,43) } #button { width: 48px; background: rgb(51,52,53) }
</style></head><body><input id="input" size="4" value="AAAA"><textarea id="textarea" cols="4" rows="2">AAAA</textarea>
<select id="select" size="2"><option>A</option><option selected>AAAA</option></select>
<input id="checkbox" type="checkbox" checked><input id="radio" type="radio"><button id="button">AAAA</button></body></html>"#,
            )
            .await;
            controls_page.vm_mut().sync_live_document_style_sources();
            let controls = controls_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(500, 400, 1.0))
                .expect("form layout should succeed")
                .expect("form fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 0.0, 48.0, 20.0)),
                (rgb(11, 12, 13), (0.0, 20.0, 63.0, 40.0)),
                (rgb(21, 22, 23), (0.0, 60.0, 52.0, 50.0)),
                (rgb(31, 32, 33), (0.0, 110.0, 13.0, 13.0)),
                (rgb(41, 42, 43), (0.0, 123.0, 13.0, 13.0)),
                (rgb(51, 52, 53), (0.0, 136.0, 48.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&controls, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }

            let mut positioned_page = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><style>
html, body { margin: 0; padding: 0 }
#clip { overflow: clip; width: 100px; height: 50px; margin-top: 100px }
#clip-sticky { position: sticky; top: 10px; width: 30px; height: 20px; background: rgb(1,2,3) }
#scroll { overflow: hidden; width: 100px; height: 50px }
#scroll-sticky { position: sticky; top: 10px; width: 30px; height: 20px; background: rgb(11,12,13) }
#transform { transform: translate(0); margin-left: 50px; width: 100px; height: 100px }
#fixed { position: fixed; right: 0; top: 0; width: 20px; height: 20px; background: rgb(21,22,23) }
</style></head><body><div id="clip"><div id="clip-sticky"></div></div><div id="scroll"><div id="scroll-sticky"></div></div>
<div id="transform"><div id="fixed"></div></div></body></html>"#,
            )
            .await;
            positioned_page.vm_mut().sync_live_document_style_sources();
            let positioned = positioned_page
                .vm_mut()
                .screenshot_layout_snapshot(moli_layout::PaintViewport::new(320, 360, 1.0))
                .expect("positioned layout should succeed")
                .expect("positioned fixture should have a document element");
            for (color, expected) in [
                (rgb(1, 2, 3), (0.0, 100.0, 30.0, 20.0)),
                (rgb(11, 12, 13), (0.0, 160.0, 30.0, 20.0)),
                (rgb(21, 22, 23), (130.0, 200.0, 20.0, 20.0)),
            ] {
                assert_paint_rect(
                    solid_paint_rect(&positioned, color),
                    moli_layout::PaintRect::new(
                        expected.0, expected.1, expected.2, expected.3,
                    ),
                );
            }
        }));
    }

    #[test]
    fn parser_body_onerror_attribute_replaces_prior_window_handler_before_compilation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head><script>
globalThis.priorBodyErrorHandlerRan = false;
window.onerror = () => {
  globalThis.priorBodyErrorHandlerRan = true;
  return true;
};
</script></head><body onerror="{">
<script>for(;) {}</script>
<script>
document.body.setAttribute('data-error-state', [
  globalThis.priorBodyErrorHandlerRan,
  window.onerror === null
].join('|'));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-error-state"),
                Some("false|true"),
                "parser body handler registration must clear the prior Window handler before compiling invalid source"
            );
        }));
    }

    fn prepared_external_classic(url: &str) -> PreparedScript {
        let url = Url::parse(url).expect("test script url");
        PreparedScript {
            position: 0,
            node_id: NodeId::new(1),
            kind: crate::types::ScriptKind::Classic,
            mode: crate::types::ScriptMode::Normal,
            source_kind: crate::types::ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            initiator_url: url.clone(),
            base_url: url.clone(),
            url,
            host_script_handle: None,
        }
    }

    fn prepared_external_module(url: &str) -> PreparedScript {
        let mut script = prepared_external_classic(url);
        script.kind = crate::types::ScriptKind::Module;
        script.mode = crate::types::ScriptMode::ModuleDefer;
        script
    }

    #[tokio::test]
    async fn main_parser_classic_pending_script_cancels_after_document_open() {
        use crate::parser_script::context::ParserClassicScriptDocumentOwnerState;

        let mut harness = new_phase_one_page_vm_harness_for_test();
        let captured_owner = harness
            .page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("initial main document owner");
        let mut script =
            prepared_external_classic("https://main-classic-stable-owner.test/parser-blocking.js");
        script.source_kind = crate::types::ScriptSourceKind::Inline;
        script.source = ScriptSource::Inline("window.__stableOwner = true".to_owned());
        let script_handle = script.node_id;
        let mut runner = PendingParsingBlockingClassicScriptRunner::from_parser_blocking_script(
            parser_blocking_pending::main_parser_blocking_classic_script_item(
                captured_owner,
                crate::parser_script::payload::ParserPreparedClassicScript::new(
                    crate::parser_script::payload::ParserClassicScriptMetadata::new(
                        script_handle,
                        1,
                    ),
                    script,
                ),
                HashSet::new(),
                None,
            ),
        );
        assert_eq!(
            runner
                .current_parser_blocking_context()
                .expect("captured parser classic context")
                .parser_classic_document_task_owner(),
            captured_owner
        );

        harness
            .page_vm
            .vm_mut()
            .eval("document.open(); 'replaced'")
            .expect("document.open should rotate the main document owner");
        let replacement_owner = harness
            .page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement main document owner");
        assert_ne!(replacement_owner, captured_owner);

        assert!(matches!(
            parser_blocking_execution::resolve_main_parser_blocking_classic_after_runtime_gate(
                &mut harness.state.parser_session,
                &mut harness.page_vm,
                &mut runner,
                "stale owner test must not execute",
            )
            .await
            .expect("stale owner should produce a typed parser cancellation outcome"),
            parser_blocking_execution::MainParserBlockingExecutionOutcome::StoppedCurrentDocument
        ));
        assert_eq!(
            runner
                .current_parser_blocking_context()
                .expect("stale PendingScript should retain its captured context")
                .parser_classic_document_task_owner(),
            captured_owner,
            "replacement currentness must not rewrite the PendingScript owner"
        );
    }

    #[test]
    fn phase_one_owner_stop_distinguishes_document_replacement_from_navigation() {
        let mut replacement = new_phase_one_page_vm_harness_for_test();
        replacement
            .page_vm
            .vm_mut()
            .eval("document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();")
            .expect("document replacement should evaluate");
        assert!(
            !replacement.page_vm.vm().has_pending_location_navigation(),
            "document.open must not create a location navigation"
        );
        assert_eq!(
            owner_step_progress_after_current_document_stop(&replacement.page_vm),
            OwnerStepProgress::DocumentReplaced
        );

        let mut navigation = new_phase_one_page_vm_harness_for_test();
        navigation
            .page_vm
            .vm_mut()
            .eval("location.href = 'https://example.test/next.html'")
            .expect("location navigation should evaluate");
        assert!(
            navigation.page_vm.vm().has_pending_location_navigation(),
            "location assignment should queue a top-level navigation"
        );
        assert_eq!(
            owner_step_progress_after_current_document_stop(&navigation.page_vm),
            OwnerStepProgress::TriggeredNavigation
        );
    }

    fn native_dom_has_element_id(dom: &moli_dom::native::NativeDom, id: &str) -> bool {
        dom.nodes()
            .iter()
            .filter_map(Node::as_element)
            .any(|element| element.attribute("id") == Some(id))
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

    async fn spawn_single_script_server(body: &'static str) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test script server should bind");
        let addr = listener
            .local_addr()
            .expect("test script server should expose address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("test script server should accept one request");
            read_http_request_head(&mut stream)
                .await
                .expect("test script server should read request");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("test script server should write response");
        });
        (
            Url::parse(&format!("http://{addr}/write.js")).expect("test script url"),
            server,
        )
    }

    async fn read_http_request_path(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
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
                break;
            }
        }
        let request = String::from_utf8_lossy(&request);
        request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .map(str::to_owned)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request path")
            })
    }

    async fn spawn_counting_parser_asset_server(
        script_body: &'static str,
    ) -> (
        Url,
        Arc<AtomicUsize>,
        Arc<tokio::sync::Notify>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test parser asset server should bind");
        let addr = listener
            .local_addr()
            .expect("test parser asset server should expose address");
        let script_requests = Arc::new(AtomicUsize::new(0));
        let server_script_requests = Arc::clone(&script_requests);
        let stylesheet_release = Arc::new(tokio::sync::Notify::new());
        let server_stylesheet_release = Arc::clone(&stylesheet_release);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("test parser asset server should accept request");
                let path = read_http_request_path(&mut stream)
                    .await
                    .expect("test parser asset server should read request");
                let (content_type, body) = if path.starts_with("/blocking.js") {
                    server_script_requests.fetch_add(1, Ordering::SeqCst);
                    ("text/javascript", script_body)
                } else if path.starts_with("/app.css") {
                    server_stylesheet_release.notified().await;
                    ("text/css", "body { color: black; }")
                } else {
                    ("text/plain", "not found")
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("test parser asset server should write response");
            }
        });
        (
            Url::parse(&format!("http://{addr}/blocking.js")).expect("test script url"),
            script_requests,
            stylesheet_release,
            server,
        )
    }

    fn preload_request_urls(requests: Vec<BufferedScriptPreloadRequest>) -> Vec<Url> {
        requests.into_iter().map(|request| request.url).collect()
    }

    fn classic_preload_key(url: &str) -> BufferedScriptPreloadKey {
        BufferedScriptPreloadKey::new(
            Url::parse(url).expect("test preload url"),
            crate::types::ScriptKind::Classic,
            &crate::planning::ScriptFetchMetadata::default(),
        )
        .expect("classic scripts are preloadable")
    }

    pub(super) fn default_test_page_vm_env_config() -> PageVmEnvConfig {
        PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id: None,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers: vec![],
            document_content_security_policies: Vec::new(),
            response_content_security_policies: Vec::new(),
            response_content_security_report_only_policies: Vec::new(),
            response_referrer_policy: None,
            content_security_reporting_endpoints:
                crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
            cross_origin_embedder_policy: Default::default(),
            document_isolation_policy: Default::default(),
            cross_origin_isolated: false,
            document_default_language: None,
            document_last_modified: None,
            locale_override: None,
            timezone_override: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            cpu_throttling_rate: 1.0,
            emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
            idle_override: None,
            viewport_surface: None,
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_policy: moli_page_types::LayoutPolicy::default(),
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        }
    }

    fn default_test_page_vm_env_config_with(
        update: impl FnOnce(&mut PageVmEnvConfig),
    ) -> PageVmEnvConfig {
        let mut env = default_test_page_vm_env_config();
        update(&mut env);
        env
    }

    fn ready_preload_entry_for_script(
        script: &PreparedScript,
        source: &str,
    ) -> BufferedScriptPreloadEntry {
        ready_preload_entry_for_script_with_resource_type(
            script,
            source,
            moli_fetch::RequestResourceType::ParserBlockingScript,
        )
    }

    fn preload_request_for_script(
        script: &PreparedScript,
        resource_type_hint: moli_fetch::RequestResourceType,
    ) -> BufferedScriptPreloadRequest {
        BufferedScriptPreloadRequest {
            url: script.url.clone(),
            initiator_url: script.initiator_url.clone(),
            kind_hint: script.kind,
            mode_hint: script.mode,
            resource_type_hint,
            fetch_metadata: script.fetch_metadata.clone(),
        }
    }

    fn ready_preload_entry_for_script_with_resource_type(
        script: &PreparedScript,
        source: &str,
        resource_type_hint: moli_fetch::RequestResourceType,
    ) -> BufferedScriptPreloadEntry {
        let request = preload_request_for_script(script, resource_type_hint);
        BufferedScriptPreloadEntry {
            request,
            load: SharedScriptSourceLoad::ready_ok(source),
        }
    }

    #[test]
    fn buffered_html_preload_scan_collects_future_external_scripts_without_importmaps() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let urls = collect_preloadable_external_script_urls_from_html(
            &final_url,
            r#"
                <script src="/vendor.js"></script>
                <script defer src="/defer.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
                <script nomodule type="module" src="/module-nomodule.mjs"></script>
                <script type="importmap" src="/importmap.json"></script>
                <script nomodule src="/legacy.js"></script>
                <script>window.inline = true;</script>
            "#,
        );

        assert_eq!(
            urls,
            vec![
                Url::parse("https://example.test/vendor.js").expect("vendor url"),
                Url::parse("https://example.test/defer.js").expect("defer url"),
                Url::parse("https://example.test/async.js").expect("async url"),
                Url::parse("https://example.test/module.mjs").expect("module url"),
                Url::parse("https://example.test/module-nomodule.mjs")
                    .expect("nomodule module url"),
            ]
        );
    }

    #[test]
    fn buffered_html_preload_scan_leaves_modulepreload_to_native_module_map() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <link rel="dns-prefetch modulepreload" href="/entry.mjs">
                <link rel="MODULEPRELOAD" href="/entry.mjs">
                <link rel="preload" as="script" href="/classic.js">
                <link rel="modulepreload" as="style" href="/theme.css">
                <link rel="modulepreload" href="/theme.css?version=1">
                <link rel="modulepreload" href="data:text/javascript,export%20default%201">
            "#,
        );

        assert_eq!(
            requests,
            Vec::new(),
            "modulepreload must not enter the legacy script-text preload cache; the parser publishes exact link candidates to the native module map"
        );
    }

    #[test]
    fn buffered_modulepreload_does_not_feed_later_module_script_text_cache() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut cache = BufferedDocumentPreloadState::default();

            cache.append_to_main_document_scan(
                &final_url,
                r#"<link rel="modulepreload" href="/entry.mjs">"#,
                &loader,
            );

            let consumer = prepared_external_module("https://example.test/entry.mjs");
            assert!(
                cache.shared_preload_for_script(&consumer).is_none(),
                "modulepreload should reserve the native module map entry instead of becoming reusable script text"
            );
        });
    }

    #[test]
    fn buffered_module_script_scan_waits_for_native_module_map_admission() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();

        cache.append_to_main_document_scan(
            &final_url,
            r#"<script type="module" src="/entry.mjs"></script>"#,
            &loader,
        );

        assert!(
            cache.entries.is_empty(),
            "module scripts must not start in the legacy SharedScriptSourceLoad cache"
        );
        assert_eq!(
            preload_request_urls(cache.take_pending_script_preloads_for_test()),
            vec![Url::parse("https://example.test/entry.mjs").expect("module url")],
            "PageVm bootstrap must receive the scanned module and register it in the native module map"
        );
    }

    #[test]
    fn prebootstrap_preload_filter_skips_async_classic_scripts() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let urls = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script src="/normal.js"></script>
                <script defer src="/defer.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
            "#,
        )
        .into_iter()
        .filter(prebootstrap_preload_request_is_dcl_relevant)
        .map(|request| request.url)
        .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                Url::parse("https://example.test/normal.js").expect("normal url"),
                Url::parse("https://example.test/defer.js").expect("defer url"),
                Url::parse("https://example.test/module.mjs").expect("module url"),
            ]
        );
    }

    #[test]
    fn html_preload_scanner_marks_late_classic_script_after_image() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script src="/early.js"></script>
                <img src="/hero.png">
                <script src="/late.js"></script>
                <script async src="/async.js"></script>
                <script type="module" src="/module.mjs"></script>
            "#,
        );

        let resource_types = requests
            .iter()
            .map(|request| (request.url.path().to_owned(), request.resource_type_hint))
            .collect::<Vec<_>>();
        assert_eq!(
            resource_types,
            vec![
                (
                    "/early.js".to_owned(),
                    moli_fetch::RequestResourceType::ParserBlockingScript,
                ),
                (
                    "/late.js".to_owned(),
                    moli_fetch::RequestResourceType::LatePreloadScript,
                ),
                (
                    "/async.js".to_owned(),
                    moli_fetch::RequestResourceType::ClassicAsyncOrDeferScript,
                ),
                (
                    "/module.mjs".to_owned(),
                    moli_fetch::RequestResourceType::Script,
                ),
            ]
        );
    }

    #[test]
    fn html_preload_scanner_collects_only_exact_eager_image_candidates() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
        let batch = scanner.scan_chunk(
            r#"
                <img src="hero.png" fetchpriority="HIGH">
                <img src="hero.png">
                <img src="lazy.png" loading="lazy">
                <img src="responsive-fallback.png" srcset="responsive.png 1x">
                <img src="cors.png" crossorigin>
                <picture><source srcset="wide.png"><img src="picture-fallback.png"></picture>
                <img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yw=">
                <template><img src="template.png"></template>
                <img src="/plain.png">
            "#,
        );

        assert_eq!(
            batch
                .image_requests
                .iter()
                .map(|request| request.url.as_str())
                .collect::<Vec<_>>(),
            [
                "https://example.test/docs/hero.png",
                "https://example.test/plain.png",
            ]
        );
        assert_eq!(
            batch.image_requests[0].fetch_priority,
            Some(moli_fetch::FetchPriorityHint::High)
        );
        assert_eq!(batch.image_requests[1].fetch_priority, None);
    }

    #[test]
    fn incremental_html_preload_scanner_handles_split_script_tag_boundaries() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        assert!(scanner.scan_script_chunk("<script sr").is_empty());
        assert_eq!(
            preload_request_urls(scanner.scan_script_chunk("c=\"/split.js\"></script>")),
            vec![Url::parse("https://example.test/split.js").expect("split url")]
        );
        assert!(scanner.finish_script_scan().is_empty());
    }

    #[test]
    fn incremental_html_preload_scanner_handles_split_stylesheet_tag_boundaries() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        let first = scanner.scan_chunk("<link rel=\"style");
        assert!(first.script_requests.is_empty());
        assert!(first.stylesheet_requests.is_empty());

        let second = scanner.scan_chunk("sheet\" href=\"/split.css\">");
        assert!(second.script_requests.is_empty());
        assert_eq!(
            second
                .stylesheet_requests
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![Url::parse("https://example.test/split.css").expect("split url")]
        );
        assert!(scanner.finish_scan().stylesheet_requests.is_empty());
    }

    #[test]
    fn incremental_html_preload_scanner_ignores_nested_template_contents_across_chunks() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        let first = scanner.scan_chunk(
            r#"
                <template>
                    <script src="/inside-script.js"></script>
                    <template><link rel="stylesheet" href="/nested.css">
            "#,
        );
        assert!(first.script_requests.is_empty());
        assert!(first.stylesheet_requests.is_empty());

        let second = scanner.scan_chunk(
            r#"
                    </template>
                    <link rel="preload" as="style" href="/inside-preload.css">
                </template>
                <script src="/outside-script.js"></script>
                <link rel="stylesheet" href="/outside.css">
            "#,
        );
        assert_eq!(
            preload_request_urls(second.script_requests),
            vec![Url::parse("https://example.test/outside-script.js").expect("outside script url")]
        );
        assert_eq!(
            second
                .stylesheet_requests
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![Url::parse("https://example.test/outside.css").expect("outside stylesheet url")]
        );
        let finished = scanner.finish_scan();
        assert!(finished.script_requests.is_empty());
        assert!(finished.stylesheet_requests.is_empty());
    }

    #[test]
    fn html_preload_scanner_uses_first_valid_base_for_later_resources() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let stylesheets = collect_preloadable_stylesheet_requests_from_html(
            &final_url,
            r#"
                <link rel="stylesheet" href="before.css">
                <base href="http://[">
                <base href="https://cdn.example/assets/">
                <link rel="stylesheet" href="after.css">
                <base href="https://ignored.example/">
                <link rel="stylesheet" href="last.css">
            "#,
        );
        let scripts = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <base href="https://cdn.example/assets/">
                <script src="entry.js"></script>
            "#,
        );

        assert_eq!(
            stylesheets
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![
                Url::parse("https://example.test/docs/before.css").expect("before url"),
                Url::parse("https://cdn.example/assets/after.css").expect("after url"),
                Url::parse("https://cdn.example/assets/last.css").expect("last url"),
            ]
        );
        assert_eq!(
            preload_request_urls(scripts),
            vec![Url::parse("https://cdn.example/assets/entry.js").expect("script url")]
        );
    }

    #[test]
    fn html_preload_scanner_collects_stylesheets_after_meta_csp_for_owner_admission() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_stylesheet_requests_from_html(
            &final_url,
            r#"
                <link rel="stylesheet" href="/before.css">
                <meta http-equiv="CONTENT-SECURITY-POLICY" content="style-src 'none'">
                <link rel="preload" as="style" href="/after.css">
            "#,
        );

        assert_eq!(
            requests
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![
                Url::parse("https://example.test/before.css").expect("before url"),
                Url::parse("https://example.test/after.css").expect("after url"),
            ]
        );
    }

    #[test]
    fn html_preload_scanner_keeps_script_and_stylesheet_same_url_distinct() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
        let batch = scanner.scan_chunk(
            r#"
                <script src="/shared"></script>
                <link rel="stylesheet" href="/shared">
            "#,
        );

        assert_eq!(batch.script_requests.len(), 1);
        assert_eq!(batch.stylesheet_requests.len(), 1);
        assert_eq!(
            batch.script_requests[0].url,
            batch.stylesheet_requests[0].url
        );
    }

    #[test]
    fn html_preload_scanner_filters_and_carries_stylesheet_attributes() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_stylesheet_requests_from_html(
            &final_url,
            r#"
                <link rel=" STYLEsheet " href="/theme.css"
                      media="screen and (min-width: 400px)"
                      crossorigin="anonymous"
                      referrerpolicy="no-referrer"
                      integrity="sha256-test"
                      nonce="nonce-1"
                      charset="utf-8"
                      fetchpriority="high">
                <link rel="preload" as="STYLE" href="/preload.css">
                <link rel="preload" as="script" href="/not-style.css">
                <link rel="stylesheet" disabled href="/disabled.css">
                <link rel="stylesheet" type="text/plain" href="/wrong-type.css">
                <link rel="stylesheet" href="data:text/css,body{}">
                <link rel="stylesheet" href="">
            "#,
        );

        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].url,
            Url::parse("https://example.test/theme.css").expect("theme url")
        );
        assert_eq!(
            requests[0].media.as_deref(),
            Some("screen and (min-width: 400px)")
        );
        assert_eq!(requests[0].options.cross_origin(), Some("anonymous"));
        assert_eq!(requests[0].options.referrer_policy(), Some("no-referrer"));
        assert_eq!(requests[0].options.integrity(), Some("sha256-test"));
        assert_eq!(requests[0].options.nonce(), Some("nonce-1"));
        assert_eq!(requests[0].options.charset(), Some("utf-8"));
        assert_eq!(requests[0].options.fetch_priority(), Some("high"));
        assert_eq!(
            requests[1].url,
            Url::parse("https://example.test/preload.css").expect("preload url")
        );
    }

    #[test]
    fn html_preload_scanner_dedupes_stylesheet_and_style_preload_resource() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_stylesheet_requests_from_html(
            &final_url,
            r#"
                <link rel="preload" as="style" href="/shared.css">
                <link rel="stylesheet" href="/shared.css">
            "#,
        );

        assert_eq!(
            requests.len(),
            1,
            "scanner descriptors represent physical resources rather than DOM clients"
        );
    }

    #[test]
    fn scanned_stylesheet_starts_without_waiting_for_parser_dom_ownership() {
        run_phase_one_large_stack_test("scanned-stylesheet-ownerless-admission", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                        .await
                        .expect("stylesheet probe server should bind");
                    let addr = listener
                        .local_addr()
                        .expect("stylesheet probe server should expose address");
                    let server = tokio::spawn(async move {
                        let (mut stream, _) = listener
                            .accept()
                            .await
                            .expect("stylesheet probe server should accept request");
                        let path = read_http_request_path(&mut stream)
                            .await
                            .expect("stylesheet probe server should read request");
                        let body = "body { color: black; }";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("stylesheet probe server should write response");
                        path
                    });

                    let mut scanner = IncrementalHtmlPreloadScanner::new(
                        Url::parse(&format!("http://{addr}/page.html")).expect("document url"),
                    );
                    let batch = scanner.scan_chunk(
                        r#"<script src="/blocking.js"></script><link rel="stylesheet" href="/app.css">"#,
                    );
                    assert_eq!(batch.script_requests.len(), 1);
                    assert_eq!(batch.stylesheet_requests.len(), 1);

                    let mut page_vm = new_phase_one_page_vm_for_test();
                    admit_stylesheet_preloads(&mut page_vm, batch.stylesheet_requests);
                    let path = tokio::time::timeout(std::time::Duration::from_secs(2), server)
                        .await
                        .expect("ownerless stylesheet request should start before parser input")
                        .expect("stylesheet probe server should finish");
                    assert_eq!(path, "/app.css");
                    assert!(
                        !native_dom_has_element_id(
                            &page_vm.vm().snapshot_live_document(),
                            "sheet"
                        ),
                        "speculative admission must not synthesize a DOM owner"
                    );
                }));
        });
    }

    #[test]
    fn scanned_stylesheet_admission_respects_media_and_fetch_interception() {
        run_phase_one_large_stack_test("scanned-stylesheet-admission-gates", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let mut page_vm = new_phase_one_page_vm_for_test();
                let options = crate::stylesheet_blocking::StylesheetFetchOptions::default();

                assert_eq!(
                    page_vm.admit_scanned_stylesheet_preload(
                        Url::parse("data:text/css,body{}").expect("data stylesheet"),
                        Some("print"),
                        options.clone(),
                        moli_fetch::RequestResourceType::CssStyleSheet,
                        false,
                    ),
                    ScannedStylesheetAdmission::DeferredToParser(
                        ScannedStylesheetDeferral::MediaMismatch
                    ),
                    "a nonmatching media query must leave the request to the real DOM owner"
                );

                page_vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Stylesheet),
                );
                assert_eq!(
                    page_vm.admit_scanned_stylesheet_preload(
                        Url::parse("data:text/css,html{}").expect("data stylesheet"),
                        Some("screen"),
                        options.clone(),
                        moli_fetch::RequestResourceType::CssStyleSheet,
                        false,
                    ),
                    ScannedStylesheetAdmission::DeferredToParser(
                        ScannedStylesheetDeferral::FetchInterception
                    ),
                    "stylesheet interception must conservatively disable speculative admission"
                );

                page_vm.set_fetch_subresource_interception(
                    true,
                    Some(crate::types::SubresourceResourceType::Script),
                );
                assert_eq!(
                    page_vm.admit_scanned_stylesheet_preload(
                        Url::parse("data:text/css,p{}").expect("data stylesheet"),
                        Some("screen"),
                        options.clone(),
                        moli_fetch::RequestResourceType::CssStyleSheet,
                        false,
                    ),
                    ScannedStylesheetAdmission::Admitted,
                    "interception for another resource type must not disable stylesheet scanning"
                );

                page_vm
                    .vm_mut()
                    .set_response_content_security_policies(&["style-src 'none'".to_owned()]);
                assert_eq!(
                    page_vm.admit_scanned_stylesheet_preload(
                        Url::parse("https://example.test/blocked.css").expect("blocked stylesheet"),
                        Some("screen"),
                        options,
                        moli_fetch::RequestResourceType::CssStyleSheet,
                        false,
                    ),
                    ScannedStylesheetAdmission::DeferredToParser(
                        ScannedStylesheetDeferral::ContentSecurityPolicy
                    ),
                    "response CSP must defer speculative admission to the DOM owner"
                );
            }));
        });
    }

    #[test]
    fn scanned_stylesheet_admission_uses_processed_meta_csp() {
        run_phase_one_large_stack_test("scanned-stylesheet-meta-csp", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let mut page_vm = new_phase_one_page_vm_for_test();
                let body = create_connected_html_body_for_test(&mut page_vm);
                let meta = {
                    let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                    let meta = dom_host.create_parser_element_without_attributes(
                        "meta".to_owned(),
                        "http://www.w3.org/1999/xhtml".to_owned(),
                        None,
                    );
                    assert!(dom_host.set_attribute(meta, "http-equiv", "content-security-policy"));
                    assert!(dom_host.set_attribute(meta, "content", "style-src 'none'"));
                    assert!(dom_host.append_child(body, meta));
                    meta
                };
                page_vm
                    .vm()
                    .document_runtime
                    .process_parser_meta_content_security_policy(meta);

                assert_eq!(
                    page_vm.admit_scanned_stylesheet_preload(
                        Url::parse("https://example.test/blocked-by-meta.css")
                            .expect("blocked stylesheet"),
                        Some("screen"),
                        crate::stylesheet_blocking::StylesheetFetchOptions::default(),
                        moli_fetch::RequestResourceType::CssStyleSheet,
                        false,
                    ),
                    ScannedStylesheetAdmission::DeferredToParser(
                        ScannedStylesheetDeferral::ContentSecurityPolicy
                    ),
                    "ownerless stylesheet admission must include already processed meta policies"
                );
            }));
        });
    }

    #[test]
    fn incremental_html_preload_scanner_collects_descriptors_after_meta_csp() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
        let batch = scanner.scan_chunk(
            r#"
                <script src="/before.js"></script>
                <meta http-equiv="Content-Security-Policy"
                      content="script-src 'none'">
                <script src="/after.js"></script>
            "#,
        );

        assert_eq!(batch.discovered_meta_csp_count, 1);
        assert_eq!(
            preload_request_urls(batch.script_requests),
            vec![
                Url::parse("https://example.test/before.js").expect("before url"),
                Url::parse("https://example.test/after.js").expect("after url"),
            ],
            "the scanner must preserve descriptors on both sides of the policy boundary"
        );
    }

    #[test]
    fn incremental_html_preload_scanner_matches_chromium_http_equiv_whitespace_behavior() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);
        let batch = scanner.scan_chunk(
            r#"<meta http-equiv=" content-security-policy " content="script-src 'none'">
               <script src="/ordinary.js"></script>"#,
        );

        assert_eq!(batch.discovered_meta_csp_count, 0);
        assert_eq!(batch.script_requests.len(), 1);
    }

    #[test]
    fn incremental_html_preload_scanner_reports_split_meta_once_and_keeps_collecting() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        let first = scanner.scan_chunk(r#"<meta http-equiv="Content-Security-"#);
        assert_eq!(first.discovered_meta_csp_count, 0);
        assert!(first.script_requests.is_empty());
        let second = scanner.scan_chunk(
            r#"Policy" content="script-src 'none'"><script src="/pending.js"></script>"#,
        );
        assert_eq!(second.discovered_meta_csp_count, 1);
        assert_eq!(
            preload_request_urls(second.script_requests),
            vec![Url::parse("https://example.test/pending.js").expect("pending url")]
        );
        let finished = scanner.finish_scan();
        assert_eq!(finished.discovered_meta_csp_count, 0);
        assert!(finished.script_requests.is_empty());
    }

    #[test]
    fn insertion_preload_scanner_remains_conservative_after_meta_csp() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new_conservative(final_url);
        let batch = scanner.scan_chunk(
            r#"<script src="/before.js"></script>
               <meta http-equiv="content-security-policy" content="script-src 'self'">
               <script src="/after.js"></script>"#,
        );

        assert_eq!(batch.discovered_meta_csp_count, 1);
        assert_eq!(
            preload_request_urls(batch.script_requests),
            vec![Url::parse("https://example.test/before.js").expect("before url")]
        );
    }

    #[test]
    fn response_csp_defers_script_preloads_to_parser_admission() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();
        cache.set_response_csp_requires_parser_admission(true);

        cache.append_to_main_document_scan(
            &final_url,
            r#"<script src="/blocked.js"></script>"#,
            &loader,
        );

        assert!(
            cache.entries.is_empty(),
            "an enforced response policy must be evaluated by the parser policy owner before any script request starts"
        );
        assert_eq!(cache.pending_preload_counts_for_test(), (1, 0));
    }

    #[test]
    fn meta_csp_gate_waits_for_every_scanner_seen_policy() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();
        cache.append_to_main_document_scan(
            &final_url,
            r#"
                <meta http-equiv="content-security-policy" content="script-src 'self'">
                <script src="/first.js"></script>
                <meta http-equiv="content-security-policy" content="script-src 'self'">
                <script src="/second.js"></script>
            "#,
            &loader,
        );

        assert_eq!(cache.meta_csp_counts_for_test(), (2, 0));
        assert_eq!(cache.pending_preload_counts_for_test(), (2, 0));
        cache.note_parser_processed_meta_csp(1);
        assert!(cache.take_pending_script_preloads_for_test().is_empty());
        assert_eq!(cache.meta_csp_counts_for_test(), (2, 1));
        cache.note_parser_processed_meta_csp(1);
        assert_eq!(cache.take_pending_script_preloads_for_test().len(), 2);
        assert_eq!(cache.meta_csp_counts_for_test(), (2, 2));
    }

    #[test]
    fn parser_clients_claim_pre_meta_pending_descriptors_before_gate_drain() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();
        cache.append_to_main_document_scan(
            &final_url,
            r#"
                <script src="/before.js"></script>
                <link rel="stylesheet" href="/before.css">
                <meta http-equiv="content-security-policy" content="default-src 'self'">
                <script src="/after.js"></script>
                <link rel="stylesheet" href="/after.css">
            "#,
            &loader,
        );
        assert_eq!(cache.pending_preload_counts_for_test(), (2, 2));

        cache.claim_pending_script_preload_for_parser(&prepared_external_classic(
            "https://example.test/before.js",
        ));
        let stylesheet_candidate =
            moli_stylesheet_blocking::DocumentOwnedBlockingStylesheetCandidate::Link {
                node_id: NodeId::new(11),
                url: Url::parse("https://example.test/before.css").expect("before stylesheet"),
                options: crate::stylesheet_blocking::StylesheetFetchOptions::default(),
            };
        cache.claim_pending_stylesheet_preloads_for_parser(&[
            DocumentOwnedBlockingStylesheetDiscoveryInput::from(&stylesheet_candidate),
        ]);
        assert_eq!(cache.pending_preload_counts_for_test(), (1, 1));

        cache.note_parser_processed_meta_csp(1);
        assert_eq!(
            preload_request_urls(cache.take_pending_script_preloads_for_test()),
            vec![Url::parse("https://example.test/after.js").expect("after script")]
        );
        assert_eq!(
            cache
                .take_pending_stylesheet_preloads()
                .into_iter()
                .map(|request| request.url)
                .collect::<Vec<_>>(),
            vec![Url::parse("https://example.test/after.css").expect("after stylesheet")]
        );
    }

    #[test]
    fn meta_csp_pending_descriptor_budget_falls_back_to_parser() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut html =
            r#"<meta http-equiv="content-security-policy" content="script-src 'self'">"#.to_owned();
        for index in 0..(MAX_PENDING_CSP_PRELOAD_CANDIDATES + 4) {
            html.push_str(&format!(r#"<script src="/{index}.js"></script>"#));
        }
        let mut cache = BufferedDocumentPreloadState::default();
        cache.append_to_main_document_scan(&final_url, &html, &loader);

        assert_eq!(
            cache.pending_preload_counts_for_test(),
            (MAX_PENDING_CSP_PRELOAD_CANDIDATES, 0),
            "overflow candidates must be left to their real parser elements"
        );
    }

    #[test]
    fn parser_meta_csp_acknowledgement_admits_all_scanned_scripts_at_first_handoff() {
        run_phase_one_large_stack_test("parser-meta-csp-preload-admission", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let html = r#"
                    <!doctype html><html><head>
                    <meta http-equiv="content-security-policy" content="script-src 'self'">
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                    <script src="/third.js"></script>
                    </head></html>
                "#;
                let PhaseOnePageVmHarness {
                    mut page_vm,
                    loader,
                    state,
                } = new_phase_one_page_vm_harness_for_test();
                activate_standalone_main_parser_continuation_for_test(&mut page_vm);
                let final_url = state.final_url.clone();
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(&final_url, html, loader);
                assert_eq!(
                    state
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (3, 0)
                );

                let mut driver = ParserDriver {
                    loader,
                    final_url: &state.final_url,
                    parser_session: &mut state.parser_session,
                    scheduler: &mut state.scheduler,
                    pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                    buffered_document_preloads: &mut state.buffered_document_preloads,
                    service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                    input_closed: &state.input_closed,
                };
                let _ = driver
                    .advance_parser_step(&mut page_vm, html, None)
                    .await
                    .expect("parser should reach its first external script boundary");

                assert_eq!(
                    driver
                        .buffered_document_preloads
                        .meta_csp_counts_for_test(),
                    (1, 1),
                    "the parser-connected meta must acknowledge exactly one scanner checkpoint"
                );
                assert_eq!(
                    driver.buffered_document_preloads.entries.len(),
                    3,
                    "one policy acknowledgement must admit later descriptors before the parser reaches them"
                );
                assert_eq!(
                    driver
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (0, 0)
                );
            }));
        });
    }

    #[test]
    fn parser_meta_csp_admission_starts_later_request_before_first_settles() {
        run_phase_one_large_stack_test("parser-meta-csp-concurrent-preloads", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let _js_runtime = crate::JsRuntime::initialize();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("script barrier server should bind");
                let addr = listener
                    .local_addr()
                    .expect("script barrier server should expose address");
                let server = tokio::spawn(async move {
                    let mut accepted = Vec::new();
                    for _ in 0..2 {
                        let (mut stream, _) = listener
                            .accept()
                            .await
                            .expect("two speculative requests should connect");
                        let path = read_http_request_path(&mut stream)
                            .await
                            .expect("script barrier server should read request");
                        accepted.push((stream, path));
                    }
                    let body = "window.cspConcurrentPreload = true;";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    for (stream, _) in &mut accepted {
                        stream
                            .write_all(response.as_bytes())
                            .await
                            .expect("script barrier server should release request");
                    }
                    accepted
                        .into_iter()
                        .map(|(_, path)| path)
                        .collect::<Vec<_>>()
                });

                let final_url =
                    Url::parse(&format!("http://{addr}/page.html")).expect("document url");
                let html = r#"
                    <meta http-equiv="content-security-policy" content="script-src 'self'">
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                "#;
                let loader = ResourceRequestClient::new(&FetchConfig::default())
                    .expect("default loader");
                let mut state = ParseTimeDriverState::new(final_url.clone());
                let parser_dom_host = state
                    .parser_session
                    .stream_handle()
                    .borrow_mut()
                    .take_parser_stream_dom_host();
                let local_executor = JsLocalExecutor::new();
                let runtime_hooks =
                    PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
                state.buffered_document_preloads.bind_resource_runtime(
                    runtime_hooks.owner_wake(),
                    runtime_hooks.resource_task_runner(),
                );
                let mut page_vm = PageVm::new(
                    PageId::new_for_testing(91),
                    local_executor,
                    &loader,
                    &default_test_page_vm_env_config(),
                    runtime_hooks,
                    parser_dom_host,
                    Instant::now(),
                )
                .expect("page vm");
                activate_standalone_main_parser_continuation_for_test(&mut page_vm);
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(&final_url, html, &loader);

                let mut driver = ParserDriver {
                    loader: &loader,
                    final_url: &state.final_url,
                    parser_session: &mut state.parser_session,
                    scheduler: &mut state.scheduler,
                    pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                    buffered_document_preloads: &mut state.buffered_document_preloads,
                    service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                    input_closed: &state.input_closed,
                };
                let _ = driver
                    .advance_parser_step(&mut page_vm, html, None)
                    .await
                    .expect("parser should stop at its first pending external source");

                let mut paths = tokio::time::timeout(std::time::Duration::from_secs(2), server)
                    .await
                    .expect("the second preload must arrive before the first response settles")
                    .expect("script barrier server should finish");
                paths.sort();
                assert_eq!(paths, vec!["/first.js", "/second.js"]);
            }));
        });
    }

    #[test]
    fn parser_meta_csp_owner_admission_blocks_without_starting_preload() {
        run_phase_one_large_stack_test("parser-meta-csp-blocked-preload", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let html = r#"
                    <meta http-equiv="content-security-policy" content="script-src 'none'">
                    <script src="/blocked.js"></script>
                "#;
                let PhaseOnePageVmHarness {
                    mut page_vm,
                    loader,
                    state,
                } = new_phase_one_page_vm_harness_for_test();
                activate_standalone_main_parser_continuation_for_test(&mut page_vm);
                let final_url = state.final_url.clone();
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(&final_url, html, loader);

                let mut driver = ParserDriver {
                    loader,
                    final_url: &state.final_url,
                    parser_session: &mut state.parser_session,
                    scheduler: &mut state.scheduler,
                    pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                    buffered_document_preloads: &mut state.buffered_document_preloads,
                    service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                    input_closed: &state.input_closed,
                };
                let _ = driver
                    .advance_parser_step(&mut page_vm, html, None)
                    .await
                    .expect("CSP-blocked parser script should remain a normal parser decision");

                assert_eq!(
                    driver.buffered_document_preloads.meta_csp_counts_for_test(),
                    (1, 1)
                );
                assert!(
                    driver.buffered_document_preloads.entries.is_empty(),
                    "silent speculative admission must not start a CSP-blocked physical request"
                );
                assert_eq!(
                    driver
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (0, 0)
                );
            }));
        });
    }

    #[test]
    fn response_csp_owner_admission_allows_self_and_blocks_none() {
        run_phase_one_large_stack_test("response-csp-preload-admission", || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let PhaseOnePageVmHarness {
                    mut page_vm,
                    loader,
                    state,
                } = new_phase_one_page_vm_harness_for_test();
                state
                    .buffered_document_preloads
                    .set_response_csp_requires_parser_admission(true);
                let final_url = state.final_url.clone();
                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(
                        &final_url,
                        r#"<script src="/allowed.js"></script>"#,
                        loader,
                    );
                page_vm
                    .vm_mut()
                    .set_response_content_security_policies(&["script-src 'self'".to_owned()]);
                admit_pending_preloads(
                    &mut page_vm,
                    &mut state.buffered_document_preloads,
                    loader,
                    None,
                );
                assert!(
                    state
                        .buffered_document_preloads
                        .entries
                        .contains_key(&classic_preload_key("https://example.test/allowed.js")),
                    "an allowed response-CSP request should enter the shared preload map"
                );

                state
                    .buffered_document_preloads
                    .append_to_main_document_scan(
                        &final_url,
                        r#"<script src="/blocked.js"></script>"#,
                        loader,
                    );
                page_vm
                    .vm_mut()
                    .set_response_content_security_policies(&["script-src 'none'".to_owned()]);
                admit_pending_preloads(
                    &mut page_vm,
                    &mut state.buffered_document_preloads,
                    loader,
                    None,
                );
                assert!(
                    !state
                        .buffered_document_preloads
                        .entries
                        .contains_key(&classic_preload_key("https://example.test/blocked.js")),
                    "a blocked response-CSP descriptor must not start a physical preload"
                );
                assert_eq!(
                    state
                        .buffered_document_preloads
                        .pending_preload_counts_for_test(),
                    (0, 0),
                    "the real parser remains authoritative after a blocked descriptor is discarded"
                );
            }));
        });
    }

    #[test]
    fn incremental_html_preload_scanner_ignores_empty_script_src() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        assert!(
            scanner
                .scan_script_chunk("<script src=\"\"></script>")
                .is_empty()
        );
        assert!(
            scanner
                .scan_script_chunk("<script src=\"  \"></script>")
                .is_empty()
        );
        assert!(scanner.finish_script_scan().is_empty());
    }

    #[test]
    fn incremental_html_preload_scanner_dedupes_urls_across_multiple_appends() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        assert_eq!(
            preload_request_urls(scanner.scan_script_chunk("<script src=\"/dup.js\"></script>")),
            vec![Url::parse("https://example.test/dup.js").expect("dup url")]
        );
        assert!(
            scanner
                .scan_script_chunk("<script src=\"/dup.js\"></script>")
                .is_empty()
        );
        assert!(scanner.finish_script_scan().is_empty());
    }

    #[test]
    fn incremental_html_preload_scanner_keeps_classic_and_module_requests_distinct() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script src="/shared.js"></script>
                <script type="module" src="/shared.js"></script>
            "#,
        );

        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].cache_key(),
            BufferedScriptPreloadKey::new(
                Url::parse("https://example.test/shared.js").expect("shared url"),
                crate::types::ScriptKind::Classic,
                &crate::planning::ScriptFetchMetadata::default(),
            )
            .expect("classic key")
        );
        assert_eq!(
            requests[1].cache_key(),
            BufferedScriptPreloadKey::new(
                Url::parse("https://example.test/shared.js").expect("shared url"),
                crate::types::ScriptKind::Module,
                &crate::planning::ScriptFetchMetadata::default(),
            )
            .expect("module key")
        );
    }

    #[test]
    fn incremental_html_preload_scanner_keeps_crossorigin_requests_distinct() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script src="/shared.js" crossorigin="anonymous"></script>
                <script src="/shared.js" crossorigin="use-credentials"></script>
            "#,
        );

        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0].cache_key(), requests[1].cache_key());
        assert_eq!(
            requests[0].request_metadata_for_testing().0,
            Some("anonymous")
        );
        assert_eq!(
            requests[1].request_metadata_for_testing().0,
            Some("use-credentials")
        );
    }

    #[test]
    fn incremental_html_preload_scanner_dedupes_fetchpriority_variants() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script src="/shared.js" fetchpriority="low"></script>
                <script src="/shared.js" fetchpriority="high"></script>
            "#,
        );

        assert_eq!(
            requests.len(),
            1,
            "fetchpriority is a scheduling hint, not a preload identity key"
        );
        assert_eq!(requests[0].request_metadata_for_testing().5, Some("low"));
    }

    #[test]
    fn buffered_script_preload_cache_reuses_load_with_different_fetchpriority() {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut preloaded = prepared_external_classic("https://example.test/vendor.js");
        preloaded.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
            None,
            None,
            None,
            None,
            None,
            Some("low"),
        );
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&preloaded).expect("preload key"),
            ready_preload_entry_for_script(&preloaded, "window.vendor = 1;"),
        );

        let mut consumer = prepared_external_classic("https://example.test/vendor.js");
        consumer.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
            None,
            None,
            None,
            None,
            None,
            Some("high"),
        );

        assert!(
            cache.shared_preload_for_script(&consumer).is_some(),
            "same request identity should reuse the preload even when fetchpriority differs"
        );
    }

    #[test]
    fn incremental_html_preload_scanner_preserves_script_raw_text_state_across_chunks() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut scanner = IncrementalHtmlPreloadScanner::new(final_url);

        assert!(
            scanner
                .scan_script_chunk("<script>window.fake = \"<script src='/bad.js'></script>")
                .is_empty()
        );
        assert_eq!(
            preload_request_urls(
                scanner.scan_script_chunk("\";</script><script src=\"/real.js\"></script>")
            ),
            vec![Url::parse("https://example.test/real.js").expect("real url")]
        );
        assert!(scanner.finish_script_scan().is_empty());
    }

    #[test]
    fn html_preload_scanner_carries_script_request_metadata() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let mut requests = collect_preloadable_external_script_requests_from_html(
            &final_url,
            r#"
                <script
                    type="module"
                    src="/module.mjs"
                    crossorigin="anonymous"
                    referrerpolicy="no-referrer"
                    charset="utf-8"
                    integrity="sha256-test"
                    nonce="nonce-1"
                    fetchpriority="high">
                </script>
            "#,
        );

        assert_eq!(requests.len(), 1);
        let request = requests.pop().expect("request");
        assert_eq!(
            request.url,
            Url::parse("https://example.test/module.mjs").expect("module url")
        );
        assert_eq!(request.kind_hint, crate::types::ScriptKind::Module);
        assert_eq!(request.mode_hint, crate::types::ScriptMode::ModuleDefer);
        assert_eq!(
            request.request_metadata_for_testing(),
            (
                Some("anonymous"),
                Some("no-referrer"),
                Some("utf-8"),
                Some("sha256-test"),
                Some("nonce-1"),
                Some("high"),
            )
        );
    }

    #[test]
    fn buffered_script_preload_cache_starts_loads_during_scan_before_handoff() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader =
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut cache = BufferedDocumentPreloadState::default();
            bind_preload_state_to_current_test_runtime(&mut cache);

            cache.append_to_main_document_scan(
                &final_url,
                r#"
                    <script src="/first.js"></script>
                    <script src="/second.js"></script>
                "#,
                &loader,
            );

            let first = prepared_external_classic("https://example.test/first.js");
            let second = prepared_external_classic("https://example.test/second.js");

            assert!(
                cache.shared_preload_for_script(&first).is_some(),
                "scanner must create the first script's shared load before parser handoff"
            );
            assert!(
                cache.shared_preload_for_script(&second).is_some(),
                "scanner must create later script shared loads before parser reaches them"
            );
        });
    }

    #[test]
    fn buffered_html_preload_scan_ignores_script_like_text_inside_script_bodies() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let urls = collect_preloadable_external_script_urls_from_html(
            &final_url,
            r#"
                <script>
                    window.fake = "<script src='/should-not-preload.js'></script>";
                </script>
                <script src="/real.js"></script>
            "#,
        );

        assert_eq!(
            urls,
            vec![Url::parse("https://example.test/real.js").expect("real url")]
        );
    }

    #[test]
    fn buffered_html_preload_scan_ignores_data_url_scripts() {
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let urls = collect_preloadable_external_script_urls_from_html(
            &final_url,
            r#"
                <script src="data:text/javascript,window.data=1"></script>
                <script src="/real.js"></script>
            "#,
        );

        assert_eq!(
            urls,
            vec![Url::parse("https://example.test/real.js").expect("real url")]
        );
    }

    #[test]
    fn buffered_script_preload_cache_applies_ready_source_to_matching_script() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/vendor.js");
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                ready_preload_entry_for_script(&script, "window.vendor = 1;"),
            );

            assert!(
                cache
                    .apply_preloaded_source_to_script_if_available(&mut script, false)
                    .await
                    .is_some()
            );
            assert!(matches!(
                &script.source,
                ScriptSource::Loaded(source) if source == "window.vendor = 1;"
            ));
        });
    }

    #[test]
    fn buffered_script_preload_cache_keeps_spawn_time_source_after_late_document_charset() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/legacy.js");
            let source_bytes = encoding_rs::GBK
                .encode("window.legacyMarker = '家居';")
                .0
                .into_owned();
            let stale_utf8_source = String::from_utf8_lossy(&source_bytes).into_owned();
            assert!(
                !stale_utf8_source.contains("家居"),
                "test fixture should be non-UTF-8 bytes"
            );

            let response = crate::protocol_types::NavigationResponse::from_text_body(
                script.url.clone(),
                200,
                vec![("Content-Type".to_owned(), "text/javascript".to_owned())],
                String::new(),
            );
            let response = crate::protocol_types::NavigationResponse::from_head_and_body(
                response.head(),
                stale_utf8_source.clone(),
                source_bytes,
            );
            let request = BufferedScriptPreloadRequest {
                url: script.url.clone(),
                initiator_url: script.initiator_url.clone(),
                kind_hint: script.kind,
                mode_hint: script.mode,
                resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
                fetch_metadata: script.fetch_metadata.clone(),
            };
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request,
                    load: SharedScriptSourceLoad::ready_outcome(
                        Ok(stale_utf8_source.clone()),
                        Some(std::sync::Arc::new(Ok(response))),
                    ),
                },
            );

            cache.set_document_character_set("GBK");
            assert!(
                cache
                    .apply_preloaded_source_to_script_if_available(&mut script, false)
                    .await
                    .is_some()
            );
            assert!(matches!(
                &script.source,
                ScriptSource::Loaded(source) if source == &stale_utf8_source
            ));
        });
    }

    #[test]
    fn buffered_script_preload_cache_keeps_source_when_late_document_charset_is_utf8() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/app.js");
            let source = "window.predecoded = true;".to_owned();
            let raw_bytes = encoding_rs::GBK
                .encode("window.predecoded = '家居';")
                .0
                .into_owned();
            let response = crate::protocol_types::NavigationResponse::from_text_body(
                script.url.clone(),
                200,
                vec![("Content-Type".to_owned(), "text/javascript".to_owned())],
                String::new(),
            );
            let response = crate::protocol_types::NavigationResponse::from_head_and_body(
                response.head(),
                String::from_utf8_lossy(&raw_bytes).into_owned(),
                raw_bytes,
            );
            let request = BufferedScriptPreloadRequest {
                url: script.url.clone(),
                initiator_url: script.initiator_url.clone(),
                kind_hint: script.kind,
                mode_hint: script.mode,
                resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
                fetch_metadata: script.fetch_metadata.clone(),
            };
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request,
                    load: SharedScriptSourceLoad::ready_outcome(
                        Ok(source.clone()),
                        Some(std::sync::Arc::new(Ok(response))),
                    ),
                },
            );

            cache.set_document_character_set("UTF-8");
            assert!(
                cache
                    .apply_preloaded_source_to_script_if_available(&mut script, false)
                    .await
                    .is_some()
            );
            assert!(matches!(
                &script.source,
                ScriptSource::Loaded(loaded) if loaded == &source
            ));
        });
    }

    #[test]
    fn buffered_script_preload_cache_does_not_reuse_load_with_different_crossorigin() {
        let mut cache = BufferedDocumentPreloadState::default();
        let mut preloaded = prepared_external_classic("https://example.test/vendor.js");
        preloaded.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
            Some("anonymous"),
            None,
            None,
            None,
            None,
            None,
        );
        cache.entries.insert(
            BufferedScriptPreloadKey::from_script(&preloaded).expect("preload key"),
            ready_preload_entry_for_script(&preloaded, "window.vendor = 1;"),
        );

        let mut consumer = prepared_external_classic("https://example.test/vendor.js");
        consumer.fetch_metadata = crate::planning::ScriptFetchMetadata::from_script_attributes(
            Some("use-credentials"),
            None,
            None,
            None,
            None,
            None,
        );

        assert!(
            cache.shared_preload_for_script(&consumer).is_none(),
            "same URL with different crossorigin cannot reuse the preload handle"
        );
    }

    #[test]
    fn buffered_script_preload_cache_can_await_pending_source_for_blocking_script() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/vendor.js");
            let load = SharedScriptSourceLoad::spawn_for_test(async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok("window.vendor = 2;".to_owned())
            });
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: BufferedScriptPreloadRequest {
                        url: script.url.clone(),
                        initiator_url: script.initiator_url.clone(),
                        kind_hint: script.kind,
                        mode_hint: script.mode,
                        resource_type_hint: moli_fetch::RequestResourceType::ParserBlockingScript,
                        fetch_metadata: script.fetch_metadata.clone(),
                    },
                    load,
                },
            );

            assert!(
                cache
                    .apply_preloaded_source_to_script_if_available(&mut script, true)
                    .await
                    .is_some()
            );
            assert!(matches!(
                &script.source,
                ScriptSource::Loaded(source) if source == "window.vendor = 2;"
            ));
        });
    }

    #[test]
    fn buffered_script_preload_cache_does_not_wait_for_pending_late_parser_blocking_preload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/late.js");
            let load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &script,
                        moli_fetch::RequestResourceType::LatePreloadScript,
                    ),
                    load,
                },
            );

            let applied = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                cache.apply_preloaded_source_to_script_if_available(&mut script, true),
            )
            .await
            .expect("pending late preload should not hold the parser-blocking consumer");

            assert!(applied.is_none());
            assert!(matches!(script.source, ScriptSource::External));
        });
    }

    #[test]
    fn buffered_script_preload_cache_classifies_parser_blocking_preload_states() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut missing_cache = BufferedDocumentPreloadState::default();
            let mut missing = prepared_external_classic("https://example.test/missing.js");
            assert!(matches!(
                missing_cache.parser_blocking_preload_disposition_for_script(&mut missing),
                ParserBlockingPreloadDisposition::Missing
            ));

            let mut ready_cache = BufferedDocumentPreloadState::default();
            let mut ready = prepared_external_classic("https://example.test/ready.js");
            ready_cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&ready).expect("preload key"),
                ready_preload_entry_for_script(&ready, "window.readyPreload = 1;"),
            );
            assert!(matches!(
                ready_cache.parser_blocking_preload_disposition_for_script(&mut ready),
                ParserBlockingPreloadDisposition::Ready(_)
            ));
            assert!(matches!(
                &ready.source,
                ScriptSource::Loaded(source) if source == "window.readyPreload = 1;"
            ));

            let mut ready_error_cache = BufferedDocumentPreloadState::default();
            let mut ready_error = prepared_external_classic("https://example.test/ready-error.js");
            ready_error_cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&ready_error).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &ready_error,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::ready_err("failed preload"),
                },
            );
            let ready_error_load = match ready_error_cache
                .parser_blocking_preload_disposition_for_script(&mut ready_error)
            {
                ParserBlockingPreloadDisposition::ReusableSourceLoad(load) => load,
                _ => panic!("completed preload failure must remain attached to PendingScript"),
            };
            assert!(
                ready_error_load
                    .try_outcome()
                    .is_some_and(|outcome| outcome.source_result.is_err()),
                "completed preload failure must retain its terminal source result"
            );
            assert!(matches!(ready_error.source, ScriptSource::External));

            let mut pending_cache = BufferedDocumentPreloadState::default();
            let mut pending = prepared_external_classic("https://example.test/pending.js");
            pending_cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&pending).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &pending,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );
            assert!(matches!(
                pending_cache.parser_blocking_preload_disposition_for_script(&mut pending),
                ParserBlockingPreloadDisposition::ReusableSourceLoad(_)
            ));

            let mut pending_late_cache = BufferedDocumentPreloadState::default();
            let mut pending_late =
                prepared_external_classic("https://example.test/pending-late.js");
            pending_late_cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&pending_late).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &pending_late,
                        moli_fetch::RequestResourceType::LatePreloadScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );
            assert!(matches!(
                pending_late_cache
                    .parser_blocking_preload_disposition_for_script(&mut pending_late),
                ParserBlockingPreloadDisposition::ExistingButNotReusable
            ));
            assert!(matches!(pending_late.source, ScriptSource::External));
        });
    }

    #[test]
    fn parser_blocking_pending_preload_returns_streaming_boundary_without_wait() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &blocking_script,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                driver.advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><script src="/blocking.js"></script><script defer src="/later.js"></script></head></html>"#,
                    None,
                ),
            )
            .await
            .expect("pending parser-blocking source load must not hold the streaming boundary")
            .expect("parser step should succeed");

            let ParserStepAdvanceOutcome::BlockedOnExternalSource(pending) = outcome else {
                panic!("pending parser-blocking preload should return a streaming boundary");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url,
                blocking_script.url
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(
                parser_blocking_classic_source_load_for_test(pending_script).is_some(),
                "streaming driver needs the pending source load as a wake interest"
            );
            assert!(matches!(
                parser_blocking_classic_source_load_for_test(pending_script),
                Some(PendingParserBlockingSourceLoad::ReusablePreload(_))
            ));
        }));
    }

    #[test]
    fn full_body_phase_one_parks_on_pending_parser_blocking_source_load() {
        run_phase_one_large_stack_test(
            "phase-one-pending-parser-blocking-source-park",
            full_body_phase_one_parks_on_pending_parser_blocking_source_load_inner,
        );
    }

    fn full_body_phase_one_parks_on_pending_parser_blocking_source_load_inner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            state.parser_session.queue_arrived_chunk(
                r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#
                    .to_owned(),
            );
            state.input_closed = true;

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &blocking_script,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );

            let local_executor = JsLocalExecutor::new();
            let run_executor = local_executor.clone();
            let page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                loader.clone(),
                PageVmInitStage::Load,
                state,
                page_vm,
            );

            let creation = Box::pin(async move {
                super::scaffold::finish_phase_one_creation_on_execution_context(
                    runtime,
                    Instant::now(),
                )
                .await
            });
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                super::access::run_named_owner_local_task(
                    run_executor,
                    "phase-one pending source-load creation test channel closed",
                    creation,
                ),
            )
            .await
            .expect("full-body phase one must not await a pending source load")
            .expect("phase-one creation should park instead of failing");

            assert!(matches!(
                outcome,
                ParseTimePageVmCreationOutcome::PendingPhaseOne(
                    PendingPhaseOneResidence::ParserBlockingSourceLoad { .. }
                )
            ));
        }));
    }

    #[test]
    fn full_body_phase_one_parks_async_subresource_terminal_for_page_owner() {
        run_phase_one_large_stack_test(
            "phase-one-async-subresource-before-source-park",
            full_body_phase_one_parks_async_subresource_terminal_for_page_owner_inner,
        );
    }

    fn full_body_phase_one_parks_async_subresource_terminal_for_page_owner_inner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            state.parser_session.queue_arrived_chunk(
                r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#
                    .to_owned(),
            );
            state.input_closed = true;

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &blocking_script,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );

            let local_executor = JsLocalExecutor::new();
            let run_executor = local_executor.clone();
            let continuation_executor = local_executor.clone();
            let page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            page_vm
                .vm()
                .resource_completion_sender_for_test()
                .send_async_subresource_event(
                    crate::types::AsyncSubresourceFetchEvent::ObservedNetworkRecord(Box::new(
                        crate::types::SubresourceNetworkRecord::failure(
                            None,
                            Url::parse("https://example.test/").unwrap(),
                            Url::parse("https://example.test/preflight").unwrap(),
                            "OPTIONS".to_owned(),
                            Vec::new(),
                            None,
                            crate::types::SubresourceResourceType::Fetch,
                            "phase-one typed terminal".to_owned(),
                        ),
                    )),
                )
                .expect("async-subresource terminal should enqueue on Networking");
            let runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                loader.clone(),
                PageVmInitStage::Load,
                state,
                page_vm,
            );

            let creation = Box::pin(async move {
                super::scaffold::finish_phase_one_creation_on_execution_context(
                    runtime,
                    Instant::now(),
                )
                .await
            });
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                super::access::run_named_owner_local_task(
                    run_executor,
                    "phase-one pending source-load runtime work test channel closed",
                    creation,
                ),
            )
            .await
            .expect("full-body phase one must not await the pending source load")
            .expect("phase-one creation should park after seeing pending source load");

            let ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::ClosedInputPageWork {
                    mut runtime,
                    started,
                },
            ) = outcome
            else {
                panic!("ready typed resource work must hand control to the Page owner");
            };
            assert!(
                runtime
                    .page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion(),
                "phase one must leave the typed terminal for the stable Page consumer"
            );

            let mut source = runtime.page_vm.page_resource_completion_queue();
            let turn_executor = runtime.page_vm.local_executor.clone();
            runtime = super::access::run_named_owner_local_task(
                turn_executor,
                "phase-one typed resource owner turn channel closed",
                async move {
                    let outcome = runtime
                        .page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut source)?
                        .expect("selected typed resource terminal must execute");
                    assert_eq!(
                        outcome.action.source(),
                        RendererOwnerResourceActivitySource::AsyncSubresource,
                        "phase one must return the async terminal to the shared Networking owner"
                    );
                    Ok(runtime)
                },
            )
            .await
            .expect("stable Page owner turn should consume the typed terminal");

            let creation = Box::pin(async move {
                (*runtime)
                    .continue_creation_from_phase_one_runtime(started)
                    .await
            });
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                super::access::run_named_owner_local_task(
                    continuation_executor,
                    "phase-one pending source-load runtime work continuation test channel closed",
                    creation,
                ),
            )
            .await
            .expect("pending phase-one continuation must not await the pending source load")
            .expect("phase-one continuation should park after one page-creation runtime turn");

            let ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::ParserBlockingSourceLoad { runtime, .. },
            ) = outcome
            else {
                panic!("pending parser-blocking source load should still park after child work");
            };
            assert!(
                !runtime
                    .page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion(),
                "ready runtime work must run before parking again on the parent parser-blocking source"
            );
        }));
    }

    #[test]
    fn parser_blocking_ready_preload_executes_without_source_boundary() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    "document.body.setAttribute('data-ready-preload', 'used');",
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                super::access::run_named_owner_local_task(
                    local_executor,
                    "phase-one ready preload parser-blocking test channel closed",
                    async move {
                        let page_vm = unsafe { &mut *page_vm_ptr };
                        let driver = unsafe { &mut *driver_ptr };
                        driver
                            .advance_parser_step(
                                page_vm,
                                r#"<!doctype html><html><body><script src="/blocking.js"></script><div id="after-ready-preload"></div></body></html>"#,
                                None,
                            )
                            .await
                    },
                ),
            )
            .await
            .expect("ready parser-blocking preload must not wait on a source boundary")
            .expect("ready preload parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preloads must be consumed, not replaced by ParserDiscovered"
            );
            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let ready_marker = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-ready-preload"));
            assert_eq!(ready_marker, Some("used"));
            assert!(
                native_dom_has_element_id(&snapshot, "after-ready-preload"),
                "parser should continue after executing the ready preloaded blocking script"
            );
        }));
    }

    #[test]
    fn main_parser_blocking_completion_dispatches_load_before_later_inline_script() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    "window.__mainParserClassicCompletionEvents = ['external'];",
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "main parser classic completion ordering test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script id="external" src="/blocking.js" onload="window.__mainParserClassicCompletionEvents.push('load:' + (document.currentScript === null))"></script>
<script>window.__mainParserClassicCompletionEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic completion ordering test should run");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preload should execute without a source boundary"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicCompletionEvents.join('|')")
                    .expect("main parser classic completion events should evaluate"),
                "external|load:true|later-inline",
                "PendingScript completion must dispatch external load with currentScript cleared before the parser executes the later inline script"
            );
        }));
    }

    #[test]
    fn main_parser_blocking_completion_settles_script_reactions_before_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    r#"
window.__mainParserClassicCheckpointEvents = ['script'];
queueMicrotask(() => window.__mainParserClassicCheckpointEvents.push('script-microtask'));
"#,
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "main parser classic checkpoint ordering test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script src="/blocking.js" onload="window.__mainParserClassicCheckpointEvents.push('load'); queueMicrotask(() => window.__mainParserClassicCheckpointEvents.push('load-microtask'))"></script>
<script>window.__mainParserClassicCheckpointEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic checkpoint ordering test should run");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "ready parser-blocking preload should execute without a source boundary"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicCheckpointEvents.join('|')")
                    .expect("main parser classic checkpoint events should evaluate"),
                "script|script-microtask|load|load-microtask|later-inline",
                "classic-script evaluation reactions must settle before the element load body, whose reactions must settle before parser continuation"
            );
        }));
    }

    #[test]
    fn main_parser_blocking_load_event_document_open_is_ignored_during_parser_execution() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            let initial_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("initial main document owner");
            let blocking_script = prepared_external_classic("https://example.test/blocking.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&blocking_script).expect("preload key"),
                ready_preload_entry_for_script(
                    &blocking_script,
                    "window.__mainParserClassicReplacementEvents = ['external'];",
                ),
            );

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "main parser classic completion replacement test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><body>
<script src="/blocking.js" onload="window.__mainParserClassicReplacementEvents.push('load'); document.open();"></script>
<script>window.__mainParserClassicReplacementEvents.push('later-inline');</script>
</body></html>"#,
                            None,
                        )
                        .await
                },
            )
            .await
            .expect("main parser classic completion replacement test should run");

            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "document.open() from a parser-blocking load event must be ignored while the parser script nesting scope is active"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicReplacementEvents.join('|')")
                    .expect("replacement completion events should evaluate"),
                "external|load|later-inline",
                "the parser must continue after ignoring document.open() from the parser-blocking load event"
            );
            assert_eq!(
                page_vm.vm().current_main_document_task_owner(),
                Some(initial_owner),
                "ignored document.open() must not rotate the main Document owner"
            );
        }));
    }

    #[test]
    fn main_parser_blocking_source_failure_uses_the_shared_completion_event_flow() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader: _,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let script_node = page_vm
                .vm_mut()
                .document_runtime
                .dom_host_mut()
                .create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
            {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                assert!(dom_host.set_attribute(script_node, "src", "/missing.js"));
                assert!(dom_host.set_attribute(
                    script_node,
                    "onerror",
                    "window.__mainParserClassicFailureEvents.push('error:' + (document.currentScript === null)); queueMicrotask(() => window.__mainParserClassicFailureEvents.push('error-microtask'))"
                ));
                assert!(dom_host.append_child(body, script_node));
            }
            let _host_handle = page_vm
                .vm_mut()
                .document_runtime
                .bind_parser_owned_script_handle_for_node(script_node);
            page_vm
                .vm_mut()
                .eval("window.__mainParserClassicFailureEvents = []")
                .expect("source failure event state should initialize");

            let task_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("main document task owner should exist");
            let target = crate::document_script_scheduler::MainDocumentClassicScriptTarget::new(
                task_owner,
                script_node,
            );
            let failure: super::parser_blocking_task::MainParserBlockingClassicScriptSourceFailureAction =
                crate::parser_script::action::ParserClassicScriptSourceFailureAction::new(
                    target,
                    crate::parser_script::payload::ParserClassicScriptSourceFailure {
                        metadata: crate::parser_script::payload::ParserClassicScriptMetadata::new(
                            script_node,
                            1,
                        ),
                        script_url: Url::parse("https://example.test/missing.js")
                            .expect("script URL"),
                        error: "network failure".to_owned(),
                        prepared_script: None,
                        source_network_result: None,
                    },
                    None,
                );
            let mut pending_runner =
                PendingParsingBlockingClassicScriptRunner::new_parser_blocking(Vec::new());
            let parser_insertion_controller =
                crate::document_runtime::ParserInsertionController::for_session(
                    &state.parser_session,
                );
            let mut owner = super::parser_blocking_document_script::MainParserBlockingDocumentScriptOwner::new(
                &mut page_vm,
                &mut pending_runner,
                parser_insertion_controller,
                "test source failure",
            );

            let outcome = crate::document_script_scheduler::ParserClassicDocumentScriptExecutionOwner::new(
                &mut owner,
            )
            .run_source_failure(failure)
            .await
            .expect("main parser classic source failure should complete");

            assert_eq!(
                outcome,
                crate::document_script_scheduler::DocumentScriptExecutionOutcome::Progressed,
                "source failure should be consumed by the shared completion owner"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("__mainParserClassicFailureEvents.join('|')")
                    .expect("source failure events should evaluate"),
                "error:true|error-microtask",
                "source failure must dispatch error with currentScript cleared and settle its reactions before parser continuation"
            );
        }));
    }

    #[test]
    fn parser_blocking_external_script_without_preload_starts_source_load_after_input_close() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            state.input_closed = true;
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                driver.advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><script src="/blocking.js"></script><script defer src="/later.js"></script></head></html>"#,
                    None,
                ),
            )
            .await
            .expect("parser-discovered source load must not hold the streaming boundary")
            .expect("parser step should succeed");

            let ParserStepAdvanceOutcome::BlockedOnExternalSource(pending) = outcome else {
                panic!("parser-discovered parser-blocking load should return a streaming boundary");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url
                    .as_str(),
                "https://example.test/blocking.js"
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(matches!(
                parser_blocking_classic_source_load_for_test(pending_script),
                Some(PendingParserBlockingSourceLoad::ParserDiscovered(_))
            ));
        }));
    }

    #[test]
    fn parser_blocking_pending_late_preload_starts_parser_discovered_source_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);
            let mut script = prepared_external_classic("https://example.test/late.js");
            state.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &script,
                        moli_fetch::RequestResourceType::LatePreloadScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(std::future::pending()),
                },
            );

            let decision = prepare_main_parser_blocking_source_load(
                &mut page_vm,
                loader,
                &mut state.buffered_document_preloads,
                &mut script,
            );

            assert!(matches!(
                decision.disposition,
                MainParserBlockingSourceDisposition::Pending(
                    PendingParserBlockingSourceLoad::ParserDiscovered(_)
                )
            ));
            assert!(
                decision.applied_preload.is_none(),
                "a pending late preload must not become the parser-blocking request fact source"
            );
        }));
    }

    #[test]
    fn parser_blocking_source_boundary_scans_later_preloads_without_parsing_later_dom() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><script src="/blocking.js"></script><script defer src="/later.js"></script></head><body><div id="after-blocking">after</div></body></html>"#,
                    None,
                )
                .await
                .expect("parser step should reach the external source boundary");

            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "external parser-blocking source should yield a streaming boundary"
            );
            assert!(
                driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/later.js")),
                "future defer scripts should be visible to the preload scanner at the parser-blocking boundary"
            );
            assert!(
                !native_dom_has_element_id(
                    &page_vm.vm().snapshot_live_document(),
                    "after-blocking"
                ),
                "parser-visible DOM must not advance past the parser-blocking script"
            );
        }));
    }

    #[test]
    fn parser_blocking_stylesheet_gate_retains_external_source_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);

            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    r#"<!doctype html><html><head><link rel="stylesheet" href="/app.css"><script src="/blocking.js"></script><script defer src="/later.js"></script></head></html>"#,
                    None,
                )
                .await
                .expect("parser step should reach the stylesheet gate");

            let ParserStepAdvanceOutcome::BlockedOnStylesheet(pending) = outcome else {
                panic!("stylesheet gate should be checked before parser-blocking source loading");
            };
            let pending_script = pending.script();
            assert_eq!(
                parser_blocking_classic_script_for_test(pending_script)
                    .expect("pending script")
                    .url
                    .as_str(),
                "https://example.test/blocking.js"
            );
            assert_eq!(
                parser_blocking_classic_metadata_for_test(pending_script)
                    .expect("pending metadata")
                    .start_line(),
                1
            );
            assert!(
                matches!(
                    parser_blocking_classic_source_load_for_test(pending_script),
                    Some(PendingParserBlockingSourceLoad::ParserDiscovered(_))
                ),
                "stylesheet-gated parser-blocking scripts must retain their preparation-time source load"
            );
        }));
    }

    #[test]
    fn stylesheet_gated_parser_blocking_script_reuses_completed_preload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let script_body = "document.documentElement.setAttribute('data-preload-used', 'yes');";
            let (script_url, script_requests, stylesheet_release, server) =
                spawn_counting_parser_asset_server(script_body).await;
            let stylesheet_url = script_url.join("app.css").expect("stylesheet url");
            let PhaseOnePageVmHarness {
                mut page_vm,
                loader,
                state,
            } = new_phase_one_page_vm_harness_for_test();

            let final_url = state.final_url.clone();
            state.buffered_document_preloads.append_to_main_document_scan(
                &final_url,
                &format!(r#"<script src="{script_url}"></script>"#),
                loader,
            );
            let preload = state
                .buffered_document_preloads
                .entries
                .load_for_key(&classic_preload_key(script_url.as_str()))
                .expect("preload scanner should start the parser-blocking script");
            let preload_outcome = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                preload.wait_outcome(),
            )
            .await
            .expect("script preload should complete before the stylesheet gate is released");
            assert_eq!(
                preload_outcome
                    .source_result
                    .expect("script preload should load source"),
                script_body
            );
            assert_eq!(script_requests.load(Ordering::SeqCst), 1);

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    &format!(
                        r#"<!doctype html><html><head><link rel="stylesheet" href="{stylesheet_url}"><script src="{script_url}"></script></head></html>"#
                    ),
                    None,
                )
                .await
                .expect("parser step should reach the stylesheet gate");
            let ParserStepAdvanceOutcome::BlockedOnStylesheet(mut pending) = outcome else {
                panic!("parser-blocking script should remain gated on the stylesheet");
            };

            pending
                .script_mut()
                .context_mut()
                .blocking_signatures_before
                .clear();
            driver
                .pending_parsing_blocking_script
                .install_parser_blocking_script_blocked_on_execution(*pending);
            stylesheet_release.notify_one();
            let mut owner = ParseTimeOwner::Parser;
            let mut parser_step_ready = true;
            let mut pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
            let parser_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("parser document owner should be current");
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let owner_ptr: *mut ParseTimeOwner = &mut owner;
            let parser_step_ready_ptr: *mut bool = &mut parser_step_ready;
            let pending_parsing_blocking_wait_ptr: *mut PendingParsingBlockingWait =
                &mut pending_parsing_blocking_wait;
            let progress = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                super::access::run_named_owner_local_task(
                    local_executor,
                    "stylesheet-gated preload reuse test channel closed",
                    async move {
                        let page_vm = unsafe { &mut *page_vm_ptr };
                        let driver = unsafe { &mut *driver_ptr };
                        let owner = unsafe { &mut *owner_ptr };
                        let parser_step_ready = unsafe { &mut *parser_step_ready_ptr };
                        let pending_parsing_blocking_wait =
                            unsafe { &mut *pending_parsing_blocking_wait_ptr };
                        driver
                            .drive_owner_step(
                                owner,
                                parser_step_ready,
                                pending_parsing_blocking_wait,
                                parser_document_owner,
                                page_vm,
                            )
                            .await
                    },
                ),
            )
            .await
            .expect("stylesheet-unblocked parser script should finish")
            .expect("stylesheet-unblocked parser step should run on the owner lane");
            let observed_script_requests = script_requests.load(Ordering::SeqCst);
            let snapshot = page_vm.vm().snapshot_live_document();
            let document_element = snapshot
                .document_element_handle()
                .expect("parser should create a document element");
            let preload_marker = snapshot
                .node(document_element)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-preload-used"));
            server.abort();
            let _ = server.await;

            assert_eq!(progress, OwnerStepProgress::Continue);
            assert_eq!(preload_marker, Some("yes"));
            assert_eq!(
                observed_script_requests, 1,
                "the completed speculative preload must satisfy the stylesheet-unblocked parser script"
            );
        }));
    }

    #[test]
    fn parser_blocking_script_disabled_does_not_start_parser_discovered_source_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let env = default_test_page_vm_env_config_with(|env| {
                env.script_execution_disabled = true;
            });
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor.clone(),
                loader,
                &env,
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one script-disabled parser-blocking test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        driver.advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#,
                            None,
                        ),
                    )
                    .await
                    .expect("script-disabled parser-blocking handoff must not wait on network")
                },
            )
            .await
            .expect("script-disabled parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "disabled script execution must not create a parser-discovered source-load boundary"
            );
            assert!(
                !driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/blocking.js")),
                "the parser-discovered blocking script should not be inserted into the preload cache when script execution is disabled"
            );
        }));
    }

    #[test]
    fn parser_blocking_csp_block_does_not_start_parser_discovered_source_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let env = default_test_page_vm_env_config_with(|env| {
                env.response_content_security_policies = vec!["script-src 'none'".to_owned()];
            });
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor.clone(),
                loader,
                &env,
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one csp-blocked parser-blocking test channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        driver.advance_parser_step(
                            page_vm,
                            r#"<!doctype html><html><head><script src="/blocking.js"></script></head></html>"#,
                            None,
                        ),
                    )
                    .await
                    .expect("CSP-blocked parser-blocking handoff must not wait on network")
                },
            )
            .await
            .expect("CSP-blocked parser-blocking test should run on owner lane");

            assert!(
                !matches!(outcome, ParserStepAdvanceOutcome::BlockedOnExternalSource(_)),
                "CSP-blocked parser-blocking scripts must not create a source-load boundary"
            );
            assert!(
                !driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/blocking.js")),
                "the CSP-blocked parser-discovered script should not be inserted into the preload cache"
            );
        }));
    }

    #[test]
    fn parser_blocking_strict_dynamic_matching_integrity_can_start_source_load() {
        let mut page_vm = new_phase_one_page_vm_for_test();
        let integrity = "sha256-wIc3KtqOuTFEu6t17sIBuOswgkV406VJvhSk79Gw6U0=";
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&[format!(
                "script-src 'strict-dynamic' '{integrity}'"
            )]);
        let mut script = prepared_external_classic("https://example.test/external-script.js");

        assert!(
            !parser_blocking_script_can_start_external_source_load(&page_vm, &script),
            "strict-dynamic must block a parser-inserted host source without trusted metadata"
        );

        script.fetch_metadata.integrity = Some(integrity.to_owned());
        assert!(
            parser_blocking_script_can_start_external_source_load(&page_vm, &script),
            "a matching integrity hash source must authorize the parser-blocking source load"
        );
    }

    #[test]
    fn parser_connected_document_write_inline_script_obeys_csp() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let env = default_test_page_vm_env_config_with(|env| {
                env.response_content_security_policies =
                    vec!["script-src 'nonce-outer'".to_owned()];
            });
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test_with_env(
                r#"<!doctype html><html><head><script nonce="outer">
globalThis.__blockedDocumentWriteRan = false;
globalThis.__documentWriteViolations = 0;
document.addEventListener("securitypolicyviolation", () => {
  globalThis.__documentWriteViolations += 1;
});
document.write('<script>globalThis.__blockedDocumentWriteRan = true;<\/script>');
globalThis.__outerDocumentWriteScriptContinued = true;
</script></head><body></body></html>"#,
                env,
            )
            .await;

            let before_dispatch = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  blockedScriptRan: globalThis.__blockedDocumentWriteRan ?? "missing",
  outerScriptContinued: globalThis.__outerDocumentWriteScriptContinued ?? "missing",
  violations: globalThis.__documentWriteViolations ?? "missing"
})"#,
                )
                .expect("pre-dispatch document.write CSP state should evaluate");
            assert_eq!(
                before_dispatch
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"blockedScriptRan":false,"outerScriptContinued":true,"violations":0}"#),
                "the inner script must be blocked before its violation task dispatches"
            );
            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let violation_task_count = super::access::run_named_owner_local_task(
                local_executor,
                "document.write CSP violation test task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    page_vm.page_task_queue.accept_ready_parse_time_wakes();
                    let mut violation_task_count = 0;
                    while let Some(task) = page_vm.page_task_queue.parse_time_pop_front() {
                        if matches!(
                            &task,
                            crate::page_task_queue::PageTask::DispatchContentSecurityPolicyViolation(
                                _
                            )
                        ) {
                            violation_task_count += 1;
                        }
                        let work = PostParsePageOwnedWork::lifecycle_work(
                            crate::page_task_queue::PostParseLifecycleWork::from_parse_time_page_task(
                                task,
                            ),
                        );
                        execute_page_owned_work_turn_on_local_task(page_vm, work).await?;
                    }
                    Ok(violation_task_count)
                },
            )
            .await
            .expect("document.write parser-boundary tasks should dispatch");
            assert_eq!(
                violation_task_count, 1,
                "the blocked document.write script should queue exactly one violation task"
            );
            let after_dispatch = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  blockedScriptRan: globalThis.__blockedDocumentWriteRan,
  outerScriptContinued: globalThis.__outerDocumentWriteScriptContinued,
  violations: globalThis.__documentWriteViolations
})"#,
                )
                .expect("document.write CSP state should evaluate");
            assert_eq!(
                after_dispatch
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"blockedScriptRan":false,"outerScriptContinued":true,"violations":1}"#)
            );
        }));
    }

    #[test]
    fn buffered_script_preload_cache_reuses_ready_late_parser_blocking_preload() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let mut cache = BufferedDocumentPreloadState::default();
            let mut script = prepared_external_classic("https://example.test/late-ready.js");
            cache.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("preload key"),
                ready_preload_entry_for_script_with_resource_type(
                    &script,
                    "window.lateReady = 1;",
                    moli_fetch::RequestResourceType::LatePreloadScript,
                ),
            );

            assert!(
                cache
                    .apply_preloaded_source_to_script_if_available(&mut script, true)
                    .await
                    .is_some()
            );
            assert!(matches!(
                &script.source,
                ScriptSource::Loaded(source) if source == "window.lateReady = 1;"
            ));
        });
    }

    #[test]
    fn parser_driver_finish_parser_blocking_pause_scans_document_write_preloads() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            bind_preload_state_to_current_test_runtime(&mut state.buffered_document_preloads);
            let session = state.parser_session.stream_handle().borrow().script_input_session();
            session.enqueue_script_input_preload_html("<script sr".to_owned());
            session.enqueue_script_input_preload_html("c=\"/write.js\"></script>".to_owned());

            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            driver.finish_parser_blocking_pause();

            assert!(
                driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/write.js")),
                "document.write insertions during a parser pause should feed the insertion preload scanner"
            );
            assert!(
                driver.buffered_document_preloads.insertion_scanner.is_none(),
                "Chromium resets the insertion preload scanner after resuming from a parser-blocking pause"
            );
            assert!(
                driver.parser_session.stream_handle().borrow_mut().take_next_insertion_preload_input().is_none(),
                "queued insertion preload html should be fully drained when the pause completes"
            );
        });
    }

    #[tokio::test]
    async fn buffered_script_preload_cache_uses_bound_owner_wake() {
        let script_body = "window.bufferedPreloadWake = true;";
        let (script_url, server) = spawn_single_script_server(script_body).await;
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut cache = BufferedDocumentPreloadState::default();
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let wake_page_id = PageId::new_for_testing(89);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(wake_page_id),
        );
        cache.bind_resource_runtime(
            Some(owner_wake),
            Some(
                crate::network::RendererResourceTaskRunner::from_current_tokio()
                    .expect("Tokio test should expose its resource task runner"),
            ),
        );
        cache.append_to_main_document_scan(
            &final_url,
            &format!(r#"<script defer src="{script_url}"></script>"#),
            &loader,
        );

        let preload = cache
            .entries
            .load_for_key(&classic_preload_key(script_url.as_str()))
            .expect("main-document scan should create script preload");
        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(2), preload.wait_outcome())
                .await
                .expect("ordinary buffered preload should finish");
        assert_eq!(
            outcome
                .source_result
                .expect("ordinary buffered preload should load source"),
            script_body
        );
        let wake = tokio::time::timeout(std::time::Duration::from_secs(1), wake_rx.recv())
            .await
            .expect("ordinary buffered preload should signal owner wake")
            .expect("owner wake channel should remain open");
        assert_eq!(wake.page_id(), wake_page_id);
        assert!(matches!(
            wake,
            crate::page_task_queue::RendererOwnerWake::Page {
                source:
                    crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork,
                ..
            }
        ));
        server.await.expect("test script server should finish");
    }

    #[tokio::test]
    async fn parser_driver_finish_parser_blocking_pause_uses_service_worker_preload_context() {
        let script_body = "window.documentWritePreload = true;";
        let (script_url, server) = spawn_single_script_server(script_body).await;
        let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url.clone());
        let browser_context_owner = crate::runtime::RendererBrowserContextRuntime::new();
        let browser_context_runtime = browser_context_owner.handle();
        let completion_queue = crate::page_task_queue::RendererPageServiceWorkerTestHarness::new();
        let client_id = browser_context_runtime.register_service_worker_client(
            final_url.clone(),
            moli_storage_key::MoliStorageKey::first_party_from_url(&final_url, None)
                .serialized_storage_key(),
            crate::service_worker_runtime::ServiceWorkerClientFrameType::TopLevel,
            Some(crate::native_bridge::WindowDocumentOwner::for_test(1)),
            completion_queue.sender(),
        );
        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let wake_page_id = PageId::new_for_testing(88);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(wake_page_id),
        );
        state.buffered_document_preloads.bind_resource_runtime(
            Some(owner_wake.clone()),
            Some(
                crate::network::RendererResourceTaskRunner::from_current_tokio()
                    .expect("service-worker preload test requires its Tokio runtime"),
            ),
        );
        state.service_worker_preload_context = Some(ServiceWorkerScriptPreloadContext::new(
            browser_context_runtime,
            client_id,
            final_url.clone(),
            Some(owner_wake),
        ));
        let session = state
            .parser_session
            .stream_handle()
            .borrow()
            .script_input_session();
        session
            .enqueue_script_input_preload_html(format!(r#"<script src="{script_url}"></script>"#));

        let mut driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };
        driver.finish_parser_blocking_pause();

        let preload = driver
            .buffered_document_preloads
            .entries
            .load_for_key(&classic_preload_key(script_url.as_str()))
            .expect("document.write insertion should create script preload");
        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(2), preload.wait_outcome())
                .await
                .expect("document.write insertion preload should finish");
        assert_eq!(
            outcome
                .source_result
                .expect("document.write insertion preload should load source"),
            script_body
        );
        let wake = tokio::time::timeout(std::time::Duration::from_secs(1), wake_rx.recv())
            .await
            .expect("service-worker-aware insertion preload should signal owner wake")
            .expect("owner wake channel should remain open");
        assert_eq!(wake.page_id(), wake_page_id);
        assert!(matches!(
            wake,
            crate::page_task_queue::RendererOwnerWake::Page {
                source:
                    crate::page_task_queue::RendererOwnerWakeSource::ParseTimeDocumentScriptWork,
                ..
            }
        ));
        server.await.expect("test script server should finish");
        drop(completion_queue);
    }

    #[test]
    fn parser_driver_finish_parser_blocking_pause_resets_insertion_scanner_state() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/docs/page.html").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            bind_preload_state_to_current_test_runtime(&mut state.buffered_document_preloads);
            let session = state.parser_session.stream_handle().borrow().script_input_session();

            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            session.enqueue_script_input_preload_html("<script sr".to_owned());
            driver.finish_parser_blocking_pause();
            session.enqueue_script_input_preload_html("c=\"/write.js\"></script>".to_owned());
            driver.finish_parser_blocking_pause();

            assert!(
                !driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key("https://example.test/write.js")),
                "partial insertion scanner state must not leak across separate parser-blocking pauses"
            );
        });
    }

    fn blocking_classic_is_stylesheet_gated_for_testing(
        live_runtime: &mut DocumentRuntime,
        discovered_blocking_stylesheet_inputs: &[DocumentOwnedBlockingStylesheetDiscoveryInput],
        blocking_signatures_before: &HashSet<DocumentBlockingStylesheetSignature>,
    ) -> bool {
        live_runtime.note_discovered_document_owned_blocking_stylesheet_inputs(
            discovered_blocking_stylesheet_inputs.iter(),
        );
        live_runtime.has_pending_parser_script_blocking_stylesheet_signatures(
            blocking_signatures_before.iter(),
        )
    }

    #[test]
    fn parse_time_driver_state_can_select_live_stream_backend_for_testing() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let state = ParseTimeDriverState::new(final_url);

        assert!(
            state
                .parser_session
                .stream_handle()
                .borrow()
                .is_parser_stream_backend_for_testing()
        );
    }

    #[test]
    fn parser_owner_boundary_with_live_backend_queues_document_turn_before_runtime_work() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url);
        let mut owner = ParseTimeOwner::Parser;
        let mut parser_step_ready = false;
        let mut pending_parsing_blocking_wait = PendingParsingBlockingWait::None;
        let _js_runtime = crate::JsRuntime::initialize();

        state.parser_session.queue_arrived_chunk(
            "<!doctype html><html><body><div>ok</div></body></html>".to_owned(),
        );
        state.input_closed = true;

        let mut driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let local_executor = JsLocalExecutor::new();
        let mut page_vm = PageVm::new(
            PageId::new_for_testing(1),
            local_executor,
            &loader,
            &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                root_frame_id: None,
                main_document_commit: None,
                top_level_storage_key: None,
                document_start_scripts: vec![],
                runtime_bindings: vec![],
                runtime_inspector_session_restore_snapshots: vec![],
                runtime_isolated_worlds: vec![],
                permission_overrides: vec![],
                extra_http_headers: vec![],
                document_content_security_policies: Vec::new(),
                response_content_security_policies: Vec::new(),
                response_content_security_report_only_policies: Vec::new(),
                response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                document_default_language: None,
                document_last_modified: None,
                locale_override: None,
                timezone_override: None,
                script_execution_disabled: false,
                bypass_content_security_policy: false,
                cpu_throttling_rate: 1.0,
                emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                idle_override: None,
                viewport_surface: None,
                network_offline: false,
                blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                fetch_subresource_interception_enabled: false,
                fetch_subresource_interception_resource_type: None,
                layout_policy: moli_page_types::LayoutPolicy::default(),
                wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
            },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
            crate::dom::native::DomHost::from_dom(NativeDom::new(
                Url::parse("https://example.test/").expect("test url"),
            )),
            Instant::now(),
        )
        .expect("page vm");
        let parser_document_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("parser owner-step test requires a main document owner");
        let progress = runtime
            .block_on(async {
                driver
                    .drive_owner_step(
                        &mut owner,
                        &mut parser_step_ready,
                        &mut pending_parsing_blocking_wait,
                        parser_document_owner,
                        &mut page_vm,
                    )
                    .await
            })
            .expect("initial parser owner step should stop at the document-turn boundary");

        assert_eq!(progress, OwnerStepProgress::Continue);
        assert_eq!(owner, ParseTimeOwner::Document);
        assert!(
            !state
                .pending_parsing_blocking_script
                .has_parser_blocking_script(),
            "initial parser boundary should not invent a pending parser-blocking script"
        );
        assert!(
            !state.parser_session.is_empty() || state.parser_session.has_script_input(),
            "initial parser boundary should preserve staged parser work"
        );
        assert!(
            !pending_parsing_blocking_wait.is_pending(),
            "parser owner should hand off a before-parser-step document turn, not a blocking wait"
        );
        assert!(
            state
                .parser_session
                .current_chunk_is_non_empty_for_testing(),
            "planning the parser boundary should stage the first parser chunk"
        );
        assert!(
            state
                .parser_session
                .stream_handle()
                .borrow()
                .is_parser_stream_backend_for_testing()
        );
    }

    #[test]
    fn document_wait_does_not_block_on_parse_visible_async_credit() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let mut async_script =
                prepared_external_classic("https://example.test/slow-async.js");
            async_script.mode = crate::types::ScriptMode::Async;
            let pending_load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());
            assert!(
                state
                    .scheduler
                    .on_parser_discovered_async_candidate_with_shared_load_and_document_character_set(
                        async_script.clone(),
                        Some(pending_load),
                        None,
                    )
            );
            assert!(
                state
                    .scheduler
                    .claim_existing_parse_time_async_handoff(async_script.node_id)
            );
            let _ = state.scheduler.grant_parse_visible_reevaluation_credit();
            assert!(
                state
                    .scheduler
                    .has_outstanding_parse_visible_reevaluation_credit()
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let scheduler_ptr = &mut state.scheduler as *mut _;
            let parser_session_ptr = &state.parser_session as *const DocumentParserSession;
            let result = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one async credit document drain local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let scheduler = unsafe { &mut *scheduler_ptr };
                    let parser_session = unsafe { &*parser_session_ptr };
                    let mut context = DocumentTurnContext {
                        scheduler,
                        parser_session,
                    };
                    let result = tokio::time::timeout(
                        std::time::Duration::from_millis(50),
                        context.drain_parse_time_turns_until_idle(page_vm, true),
                    )
                    .await
                    .expect("parse-time document drain must not wait for a slow async fetch")
                    .expect("document drain should succeed");
                    Ok(result)
                },
            )
            .await
            .expect("document drain should run on the named owner lane");

            assert!(
                matches!(result, PageTaskTurnResult::NoTask),
                "parse-visible async credit is a parser wake interest, not document-processing work"
            );
            assert!(
                state
                    .scheduler
                    .has_outstanding_parse_visible_reevaluation_credit(),
                "the streaming/parser boundary still owns the async wake interest"
            );
        }));
    }

    #[test]
    fn parser_step_without_script_handoff_consumes_live_backend_dom() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url);
        let driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_modulepreload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .pump_parser_step(
                "<!doctype html><html><body><main>live parser step</main></body></html>",
            );
        let parser_stream_snapshot = state
            .parser_session
            .stream_handle()
            .borrow()
            .snapshot_parser_stream_document();
        assert!(
            matches!(result, ParserPumpStep::InputDrained),
            "expected non-script html to drain without a parser handoff"
        );

        assert!(
            parser_stream_snapshot.parse_errors().is_empty(),
            "plain html parser step should not record parse errors"
        );
        let body = parser_stream_snapshot.document_body_handle().expect("body");
        assert_eq!(
            parser_stream_snapshot.text_content(body).as_deref(),
            Some("live parser step")
        );
        assert!(
            state
                .parser_session
                .stream_handle()
                .borrow()
                .is_parser_stream_backend_for_testing()
        );
    }

    #[test]
    fn parser_step_with_inline_script_surfaces_handoff_on_live_backend() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url);
        let driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_modulepreload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            "<!doctype html><html><head><script>window.answer = 42;</script></head><body><div>late</div></body></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at inline script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id: handle,
            start_line: _,
            start_column: _,
            blocking_signatures_before: _,
            script: _script,
        } = *handoff
        else {
            panic!("expected parser step to stop at inline blocking classic handoff");
        };
        let parser_stream_snapshot = state
            .parser_session
            .stream_handle()
            .borrow()
            .snapshot_parser_stream_document();

        assert!(
            parser_stream_snapshot.node_is_parser_created(handle),
            "handoff script should still be parser-created on the parser-stream backend"
        );
        assert_eq!(
            parser_stream_snapshot.script_text(handle).as_deref(),
            Some("window.answer = 42;")
        );
        assert!(
            parser_stream_snapshot.document_body_handle().is_none(),
            "later body content should remain hidden at the script handoff boundary"
        );
        assert!(
            state
                .parser_session
                .stream_handle()
                .borrow()
                .is_parser_stream_backend_for_testing()
        );
    }

    #[test]
    fn parser_step_with_inline_svg_script_surfaces_shared_script_handoff() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url);
        let driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_modulepreload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
            "<!doctype html><html><body><svg><script>window.svgAnswer = 42;</script></svg><div>late</div></body></html>",
        );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to stop at inline SVG script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id: handle,
            start_line: _,
            start_column: _,
            blocking_signatures_before: _,
            script: _,
        } = *handoff
        else {
            panic!("expected inline SVG script to use the blocking classic handoff");
        };
        let parser_stream_snapshot = state
            .parser_session
            .stream_handle()
            .borrow()
            .snapshot_parser_stream_document();
        let script = parser_stream_snapshot
            .node(handle)
            .and_then(Node::as_element)
            .expect("SVG script element at handoff");

        assert!(script.is_script_element());
        assert_eq!(script.wrapper_prototype_name(), "SVGScriptElement");
        assert!(parser_stream_snapshot.node_is_parser_created(handle));
        assert_eq!(
            parser_stream_snapshot.script_text(handle).as_deref(),
            Some("window.svgAnswer = 42;")
        );
        assert!(
            state
                .parser_session
                .stream_handle()
                .borrow()
                .is_parser_stream_backend_for_testing()
        );
    }

    #[test]
    fn phase_one_parser_inserted_connected_stylesheet_queues_load_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><head>
<link rel="stylesheet" href="data:text/css,body%20%7B%20color%3A%20green%3B%20%7D">
</head><body></body></html>"#;
            let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let head = snapshot.document_head_handle().expect("head handle");
            let link = snapshot
                .child_nodes(head)
                .expect("head children")
                .into_iter()
                .find(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("link"))
                })
                .expect("parser-created stylesheet link");
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "parser insertion should queue connected style/link processing from the runtime mutation owner"
            );
        }));
    }

    #[test]
    fn phase_one_template_stylesheets_remain_inert() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><head>
<template>
  <link rel="stylesheet" href="/inside.css">
  <style>@import url("/inside-import.css");</style>
</template>
</head><body></body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let inert_stylesheet_owners = snapshot
                .nodes()
                .filter_map(|node| {
                    node.as_element()
                        .filter(|element| {
                            element.is_html_element("link") || element.is_html_element("style")
                        })
                        .map(|_| node.id())
                })
                .collect::<Vec<_>>();
            assert_eq!(inert_stylesheet_owners.len(), 2);
            for owner in inert_stylesheet_owners {
                let node = snapshot.node(owner).expect("stylesheet owner node");
                assert!(
                    !node.is_connected(),
                    "a stylesheet owner in template contents must remain disconnected"
                );
                assert!(
                    !page_vm
                        .vm()
                        .document_runtime
                        .connected_style_load_is_queued_for_test(owner),
                    "a disconnected stylesheet owner must not start a connected load"
                );
            }
        }));
    }

    #[test]
    fn js_document_fragment_insertion_queues_connected_stylesheet_loads_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function fragmentWithStylesheet(id) {
  const fragment = document.createDocumentFragment();
  const link = document.createElement('link');
  link.id = id;
  link.rel = 'stylesheet';
  link.href = 'data:text/css,body%20%7B%20color%3A%20green%3B%20%7D';
  fragment.appendChild(link);
  return fragment;
}

document.body.appendChild(fragmentWithStylesheet('js-fragment-style-append'));

const reference = document.createElement('span');
reference.id = 'js-fragment-style-reference';
document.body.appendChild(reference);
document.body.insertBefore(
  fragmentWithStylesheet('js-fragment-style-before'),
  reference
);

const oldChild = document.createElement('span');
oldChild.id = 'js-fragment-style-old';
document.body.appendChild(oldChild);
document.body.replaceChild(
  fragmentWithStylesheet('js-fragment-style-replace'),
  oldChild
);
"#,
                )
                .expect("fragment stylesheet insertion JS setup should evaluate");

            let expected = {
                let runtime = &page_vm.vm().document_runtime;
                vec![
                    runtime
                        .get_element_by_id("js-fragment-style-append")
                        .expect("append fragment stylesheet should exist"),
                    runtime
                        .get_element_by_id("js-fragment-style-before")
                        .expect("insertBefore fragment stylesheet should exist"),
                    runtime
                        .get_element_by_id("js-fragment-style-replace")
                        .expect("replaceChild fragment stylesheet should exist"),
                ]
            };

            let runtime = &page_vm.vm().document_runtime;
            assert!(
                expected
                    .into_iter()
                    .all(|handle| runtime.connected_style_load_is_queued_for_test(handle)),
                "JS DocumentFragment insertion should queue stylesheet loads for hoisted children"
            );
        }));
    }

    #[test]
    fn parser_fragment_append_child_queues_stylesheet_load_from_hoisted_roots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let (fragment, link) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                let link = dom_host.create_parser_element_without_attributes(
                    "link".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(link, "id", "parser-fragment-style-append-link"));
                assert!(dom_host.set_attribute(link, "rel", "stylesheet"));
                assert!(dom_host.set_attribute(
                    link,
                    "href",
                    "data:text/css,body%20%7B%20color%3A%20green%3B%20%7D"
                ));
                assert!(dom_host.append_child(fragment, link));
                (fragment, link)
            };
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "disconnected parser fragment stylesheet setup should not queue style loads"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment stylesheet appendChild should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain stylesheet fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment stylesheet append should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![link],
                "parser DocumentFragment stylesheet append should append the hoisted link"
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "parser DocumentFragment append should queue stylesheet loads for hoisted children"
            );
        }));
    }

    #[test]
    fn parser_fragment_insert_before_queues_stylesheet_load_from_hoisted_roots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let (fragment, link, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-style-ref"));
                assert!(dom_host.append_child(body, reference));

                let fragment = dom_host.create_document_fragment();
                let link = dom_host.create_parser_element_without_attributes(
                    "link".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(link, "id", "parser-fragment-style-link"));
                assert!(dom_host.set_attribute(link, "rel", "stylesheet"));
                assert!(dom_host.set_attribute(
                    link,
                    "href",
                    "data:text/css,body%20%7B%20color%3A%20green%3B%20%7D"
                ));
                assert!(dom_host.append_child(fragment, link));
                (fragment, link, reference)
            };
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "disconnected parser fragment stylesheet setup should not queue style loads"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment stylesheet insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain stylesheet fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment stylesheet insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![link, reference],
                "parser DocumentFragment stylesheet insertion should place the hoisted link before the reference child"
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .connected_style_load_is_queued_for_test(link),
                "parser DocumentFragment insertion should queue stylesheet loads for hoisted children"
            );
        }));
    }

    #[test]
    fn js_replace_child_inserted_image_queues_load_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            page_vm
                .evaluate_expression(
                    r#"
const oldImageSlot = document.createElement('span');
oldImageSlot.id = 'js-replace-old-image-slot';
document.body.appendChild(oldImageSlot);

const image = document.createElement('img');
image.id = 'js-replace-inserted-image';
image.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
window.jsReplaceInsertedImage = image;
"#,
                )
                .expect("replaceChild inserted image setup should evaluate");

            page_vm
                .evaluate_expression(
                    r#"
document.body.replaceChild(
  window.jsReplaceInsertedImage,
  document.getElementById('js-replace-old-image-slot')
);
"#,
                )
                .expect("replaceChild inserted image should evaluate");

            let image = {
                let snapshot = page_vm.vm().snapshot_live_document();
                let body = snapshot.document_body_handle().expect("body handle");
                snapshot
                    .child_nodes(body)
                    .expect("body children")
                    .into_iter()
                    .find(|handle| {
                        snapshot
                            .node(*handle)
                            .and_then(Node::as_element)
                            .is_some_and(|element| {
                                element.attribute("id") == Some("js-replace-inserted-image")
                            })
                    })
                    .expect("connected replacement image should exist")
            };
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "JS replaceChild insertion should queue image load events from the mutation owner"
            );
        }));
    }

    #[test]
    fn js_replace_child_inserted_default_track_queues_load_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const video = document.createElement('video');
video.id = 'js-replace-track-video';
const oldTrackSlot = document.createElement('span');
oldTrackSlot.id = 'js-replace-old-track-slot';
video.appendChild(oldTrackSlot);
document.body.appendChild(video);

const track = document.createElement('track');
track.id = 'js-replace-inserted-track';
document.body.appendChild(track);
window.jsReplaceInsertedTrack = track;
track.remove();
"#,
                )
                .expect("replaceChild inserted track setup should evaluate");

            let track = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("js-replace-inserted-track")
                .or_else(|| {
                    let dom_host = page_vm.vm().document_runtime.dom_host();
                    dom_host.dom().nodes().iter().enumerate().find_map(|(index, node)| {
                        node.as_element()
                            .is_some_and(|element| {
                                element.attribute("id") == Some("js-replace-inserted-track")
                            })
                            .then_some(DomHandle::new(index))
                    })
                })
                .expect("detached replacement track handle should exist");
            {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                assert!(dom_host.set_attribute(track, "default", ""));
                assert!(dom_host.set_attribute(track, "src", "captions/en.vtt"));
            }
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "native detached default track setup should not queue text-track timers before replaceChild insertion"
            );

            page_vm
                .evaluate_expression(
                    r#"
document.getElementById('js-replace-track-video').replaceChild(
  window.jsReplaceInsertedTrack,
  document.getElementById('js-replace-old-track-slot')
);
"#,
                )
                .expect("replaceChild inserted track should evaluate");

            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "default text-track mode selection must not acquire a PageTimer descriptor"
            );
            let task = take_next_dom_manipulation_task_for_test(&page_vm);
            assert!(
                matches!(
                    task,
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "mutation-owned default-mode work should share the DOM-manipulation source"
            );
        }));
    }

    #[test]
    fn parser_mutation_owner_syncs_inserted_child_browsing_context_without_driver_resync() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "test setup should start without pending child frame attachments"
            );
            let (body, iframe) = {
                let body = create_connected_html_body_for_test(&mut page_vm);
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                (body, iframe)
            };
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "creating a disconnected parser iframe should not create a child context"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: iframe,
                    },
                    "parser DOM mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain iframe insertion should not queue custom element reactions"
            );
            let attachments = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                attachments.len(),
                1,
                "parser insertion followup should attach the inserted iframe immediately"
            );
            assert!(matches!(
                &attachments[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.parent_frame_id.is_none()
            ));
        }));
    }

    #[test]
    fn parser_document_fragment_append_child_syncs_child_browsing_context_subtree() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "test setup should start without pending child frame attachments"
            );
            let (fragment, container, iframe) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                let container = dom_host.create_parser_element_without_attributes(
                    "section".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(container, "id", "parser-fragment-iframe-root"));
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(iframe, "id", "parser-fragment-iframe"));
                assert!(dom_host.append_child(container, iframe));
                assert!(dom_host.append_child(fragment, container));
                (fragment, container, iframe)
            };
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "creating a disconnected parser fragment iframe subtree should not create a child context"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment iframe appendChild should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain iframe fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment iframe append should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container],
                "parser DocumentFragment iframe append should append the hoisted root"
            );
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser DocumentFragment append followup should register iframe subtrees under hoisted roots"
            );
            let attachments = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                attachments.len(),
                1,
                "parser DocumentFragment append followup should attach the iframe subtree immediately"
            );
            assert!(matches!(
                &attachments[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.parent_frame_id.is_none()
            ));
        }));
    }

    #[test]
    fn parser_document_fragment_insert_before_syncs_child_browsing_context_subtree() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "test setup should start without pending child frame attachments"
            );
            let (fragment, container, iframe, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-iframe-ref"));
                assert!(dom_host.append_child(body, reference));

                let fragment = dom_host.create_document_fragment();
                let container = dom_host.create_parser_element_without_attributes(
                    "section".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(container, "id", "parser-fragment-iframe-root"));
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(iframe, "id", "parser-fragment-iframe"));
                assert!(dom_host.append_child(container, iframe));
                assert!(dom_host.append_child(fragment, container));
                (fragment, container, iframe, reference)
            };
            assert!(
                context_host
                    .borrow_mut()
                    .take_pending_child_frame_tree_events()
                    .is_empty(),
                "creating a disconnected parser fragment iframe subtree should not create a child context"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment iframe insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain iframe fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment iframe insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container, reference],
                "parser DocumentFragment iframe insertion should place the hoisted root before the reference child"
            );
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser DocumentFragment insertion followup should register iframe subtrees under hoisted roots"
            );
            let attachments = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                attachments.len(),
                1,
                "parser DocumentFragment insertion followup should attach the iframe subtree immediately"
            );
            assert!(matches!(
                &attachments[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.parent_frame_id.is_none()
            ));
        }));
    }

    #[test]
    fn parser_document_fragment_append_child_clears_disconnected_shadow_roots_in_subtree() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let connected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  const host = document.createElement('div');
  host.id = 'parser-fragment-shadow-append-host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const target = document.createElement('span');
  target.id = 'parser-fragment-shadow-append-target';
  target.className = 'target';
  shadow.appendChild(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.target { color: rgb(0, 128, 0); }');
  shadow.adoptedStyleSheets = [sheet];
  window.parserFragmentShadowAppendHost = host;
  window.parserFragmentShadowAppendTarget = target;
  return getComputedStyle(target).color;
})()
"#,
                )
                .expect("shadow append host setup should evaluate");
            assert_eq!(
                connected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "connected shadow tree style should apply before removal"
            );

            let host = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-fragment-shadow-append-host")
                .expect("connected shadow host should exist");

            let disconnected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  window.parserFragmentShadowAppendHost.remove();
  const style = getComputedStyle(window.parserFragmentShadowAppendTarget);
  return JSON.stringify({
    color: style.color,
    length: style.length
  });
})()
"#,
                )
                .expect("shadow append host removal should evaluate");
            assert_eq!(
                disconnected
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"color":"","length":0}"#),
                "removed shadow tree style should be unavailable while disconnected"
            );

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, host));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment shadow host appendChild should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain shadow host fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment shadow append should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![host],
                "parser DocumentFragment shadow append should append the hoisted host"
            );

            let reconnected = page_vm
                .evaluate_expression(
                    "getComputedStyle(window.parserFragmentShadowAppendTarget).color",
                )
                .expect("shadow append host reconnected style should evaluate");
            assert_eq!(
                reconnected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "parser DocumentFragment append should clear disconnected shadow-root style markers for hoisted subtrees"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_insert_before_clears_disconnected_shadow_roots_in_subtree() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            let connected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  const host = document.createElement('div');
  host.id = 'parser-fragment-shadow-host';
  document.body.appendChild(host);
  const shadow = host.attachShadow({ mode: 'open' });
  const target = document.createElement('span');
  target.id = 'parser-fragment-shadow-target';
  target.className = 'target';
  shadow.appendChild(target);
  const sheet = new CSSStyleSheet();
  sheet.replaceSync('.target { color: rgb(0, 128, 0); }');
  shadow.adoptedStyleSheets = [sheet];
  window.parserFragmentShadowHost = host;
  window.parserFragmentShadowTarget = target;
  return getComputedStyle(target).color;
})()
"#,
                )
                .expect("shadow host setup should evaluate");
            assert_eq!(
                connected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "connected shadow tree style should apply before removal"
            );

            let host = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-fragment-shadow-host")
                .expect("connected shadow host should exist");

            let disconnected = page_vm
                .evaluate_expression(
                    r#"
(() => {
  window.parserFragmentShadowHost.remove();
  const style = getComputedStyle(window.parserFragmentShadowTarget);
  return JSON.stringify({
    color: style.color,
    length: style.length
  });
})()
"#,
                )
                .expect("shadow host removal should evaluate");
            assert_eq!(
                disconnected
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"{"color":"","length":0}"#),
                "removed shadow tree style should be unavailable while disconnected"
            );

            let (fragment, reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-shadow-ref"));
                assert!(dom_host.append_child(body, reference));
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, host));
                (fragment, reference)
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment shadow host insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain shadow host fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment shadow insertion should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![host, reference],
                "parser DocumentFragment shadow insertion should place the hoisted host before the reference child"
            );

            let reconnected = page_vm
                .evaluate_expression("getComputedStyle(window.parserFragmentShadowTarget).color")
                .expect("shadow host reconnected style should evaluate");
            assert_eq!(
                reconnected.get("value").and_then(serde_json::Value::as_str),
                Some("rgb(0, 128, 0)"),
                "parser DocumentFragment insertion should clear disconnected shadow-root style markers for hoisted subtrees"
            );
        }));
    }

    #[test]
    fn parser_mutation_owner_drops_removed_child_browsing_context_without_driver_resync() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (body, iframe) = {
                let body = create_connected_html_body_for_test(&mut page_vm);
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                (body, iframe)
            };

            {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: iframe,
                    },
                    "parser append mutation should apply",
                );
            }
            let frame_id = context_host
                .borrow()
                .child_browsing_context_frame_id_by_owner_node_id(iframe)
                .expect("parser append followup should register the iframe child context");
            assert!(
                !frame_id.is_empty(),
                "registered child frame id should be observable before removal"
            );

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: body,
                    child: iframe,
                },
                "parser remove mutation should apply",
                "parser removal reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "parser iframe removal should defer removed-subtree lifecycle followups"
            );
            assert_eq!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe),
                None,
                "parser removal followup should drop the iframe child context registry entry"
            );
            let frame_tree_events = context_host
                .borrow_mut()
                .take_pending_child_frame_tree_events();
            assert_eq!(
                frame_tree_events.len(),
                2,
                "an iframe inserted and removed before a protocol drain must preserve both tree events"
            );
            assert!(matches!(
                &frame_tree_events[0],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Attached(attachment)
                    if attachment.frame_id == frame_id
            ));
            assert!(matches!(
                &frame_tree_events[1],
                crate::protocol_types::ChildFrameTreeEventSnapshot::Detached(detachment)
                    if detachment.frame_id == frame_id
            ));
        }));
    }

    #[test]
    fn parser_reparent_to_disconnected_parent_drops_child_browsing_context() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (body, detached_parent, iframe) = {
                let body = create_connected_html_body_for_test(&mut page_vm);
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let iframe = dom_host.create_parser_element_without_attributes(
                    "iframe".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                (body, detached_parent, iframe)
            };

            {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: iframe,
                    },
                    "parser append mutation should apply",
                );
            }
            assert!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe)
                    .is_some(),
                "parser append followup should register the iframe child context before reparent"
            );

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: detached_parent,
                    child: iframe,
                    reference_child: None,
                },
                "parser reparent mutation should apply",
                "parser reparent lifecycle followups should dispatch",
            );
            assert!(
                had_pending_work,
                "parser reparent to a disconnected parent should defer removed-subtree lifecycle followups"
            );
            assert_eq!(
                context_host
                    .borrow()
                    .child_browsing_context_frame_id_by_owner_node_id(iframe),
                None,
                "parser reparent to a disconnected parent should drop the iframe child context"
            );
        }));
    }

    #[test]
    fn parser_insert_nested_custom_element_to_connected_parent_matches_js_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedInsertEvents = [];
class ParserNestedInsertedElement extends HTMLElement {
  connectedCallback() {
    window.parserNestedInsertEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-nested-inserted-element', ParserNestedInsertedElement);
const jsOuter = document.createElement('parser-nested-inserted-element');
jsOuter.id = 'js-inserted-outer';
const jsInner = document.createElement('parser-nested-inserted-element');
jsInner.id = 'js-inserted-inner';
jsOuter.appendChild(jsInner);
const parserOuter = document.createElement('parser-nested-inserted-element');
parserOuter.id = 'parser-inserted-outer';
const parserInner = document.createElement('parser-nested-inserted-element');
parserInner.id = 'parser-inserted-inner';
parserOuter.appendChild(parserInner);
document.body.append(jsOuter, parserOuter);
window.parserNestedInsertedJsOuter = jsOuter;
window.parserNestedInsertedOuter = parserOuter;
window.parserNestedInsertEvents.length = 0;
"#,
                )
                .expect("nested custom-element insertion setup should evaluate");

            let target = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-inserted-outer")
                    .expect("parser nested custom element should exist before insertion")
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedInsertedJsOuter.remove();
window.parserNestedInsertedOuter.remove();
window.parserNestedInsertEvents.length = 0;
document.body.appendChild(window.parserNestedInsertedJsOuter);
window.parserNestedInsertJsEvents = window.parserNestedInsertEvents.slice();
window.parserNestedInsertEvents.length = 0;
"#,
                )
                .expect("nested custom-element detached baseline should evaluate");

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: target,
                },
                "parser nested insertion mutation should apply",
                "parser nested connected reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "inserting a disconnected upgraded custom-element subtree should defer connected reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserNestedInsertJsEvents,
  parserEvents: window.parserNestedInsertEvents,
  same: window.parserNestedInsertJsEvents.map(event => event.replace('js-inserted', 'target')).join('|') ===
    window.parserNestedInsertEvents.map(event => event.replace('parser-inserted', 'target')).join('|'),
  parserConnected: window.parserNestedInsertedOuter.isConnected,
  parserParent: window.parserNestedInsertedOuter.parentNode && window.parserNestedInsertedOuter.parentNode.nodeName
})"#,
                )
                .expect("nested custom-element insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-inserted-outer:connected:true","js-inserted-inner:connected:true"],"parserEvents":["parser-inserted-outer:connected:true","parser-inserted-inner:connected:true"],"same":true,"parserConnected":true,"parserParent":"BODY"}"#
                ),
                "parser insertion should match JS appendChild connected callback preorder for upgraded subtrees"
            );
        }));
    }

    #[test]
    fn parser_insert_document_fragment_custom_elements_matches_js_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentInsertEvents = [];
class ParserFragmentInsertedElement extends HTMLElement {
  connectedCallback() {
    window.parserFragmentInsertEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-fragment-inserted-element', ParserFragmentInsertedElement);
const jsFragment = document.createDocumentFragment();
const jsOuter = document.createElement('parser-fragment-inserted-element');
jsOuter.id = 'js-fragment-outer';
const jsInner = document.createElement('parser-fragment-inserted-element');
jsInner.id = 'js-fragment-inner';
jsOuter.appendChild(jsInner);
const jsSecond = document.createElement('parser-fragment-inserted-element');
jsSecond.id = 'js-fragment-second';
const parserOuter = document.createElement('parser-fragment-inserted-element');
parserOuter.id = 'parser-fragment-outer';
const parserInner = document.createElement('parser-fragment-inserted-element');
parserInner.id = 'parser-fragment-inner';
parserOuter.appendChild(parserInner);
const parserSecond = document.createElement('parser-fragment-inserted-element');
parserSecond.id = 'parser-fragment-second';
document.body.append(jsOuter, jsSecond, parserOuter, parserSecond);
window.parserFragmentJsFragment = jsFragment;
window.parserFragmentJsOuter = jsOuter;
window.parserFragmentJsSecond = jsSecond;
window.parserFragmentParserOuter = parserOuter;
window.parserFragmentParserSecond = parserSecond;
window.parserFragmentInsertEvents.length = 0;
"#,
                )
                .expect("fragment custom-element insertion setup should evaluate");

            let (parser_outer, parser_second) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-fragment-outer")
                        .expect("parser fragment outer custom element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-second")
                        .expect("parser fragment second custom element should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentJsOuter.remove();
window.parserFragmentJsSecond.remove();
window.parserFragmentParserOuter.remove();
window.parserFragmentParserSecond.remove();
window.parserFragmentInsertEvents.length = 0;
window.parserFragmentJsFragment.append(window.parserFragmentJsOuter, window.parserFragmentJsSecond);
document.body.appendChild(window.parserFragmentJsFragment);
window.parserFragmentJsEvents = window.parserFragmentInsertEvents.slice();
window.parserFragmentInsertEvents.length = 0;
"#,
                )
                .expect("fragment custom-element JS baseline should evaluate");

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_outer));
                assert!(dom_host.append_child(fragment, parser_second));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment insertion mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "inserting a fragment with upgraded custom-element roots should defer connected reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment insertion should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser fragment connected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFragmentJsEvents,
  parserEvents: window.parserFragmentInsertEvents,
  same: window.parserFragmentJsEvents.map(event => event.replace('js-fragment', 'target')).join('|') ===
    window.parserFragmentInsertEvents.map(event => event.replace('parser-fragment', 'target')).join('|'),
  jsFragmentEmpty: window.parserFragmentJsFragment.childNodes.length,
  parserOuterParent: window.parserFragmentParserOuter.parentNode && window.parserFragmentParserOuter.parentNode.nodeName,
  parserSecondParent: window.parserFragmentParserSecond.parentNode && window.parserFragmentParserSecond.parentNode.nodeName
})"#,
                )
                .expect("fragment custom-element insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-fragment-outer:connected:true","js-fragment-inner:connected:true","js-fragment-second:connected:true"],"parserEvents":["parser-fragment-outer:connected:true","parser-fragment-inner:connected:true","parser-fragment-second:connected:true"],"same":true,"jsFragmentEmpty":0,"parserOuterParent":"BODY","parserSecondParent":"BODY"}"#
                ),
                "parser DocumentFragment insertion should match JS appendChild connected callback preorder across hoisted roots"
            );
        }));
    }

    #[test]
    fn parser_insert_before_document_fragment_custom_elements_matches_js_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentBeforeEvents = [];
class ParserFragmentBeforeElement extends HTMLElement {
  connectedCallback() {
    window.parserFragmentBeforeEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
}
customElements.define('parser-fragment-before-element', ParserFragmentBeforeElement);
const jsFragment = document.createDocumentFragment();
const jsOuter = document.createElement('parser-fragment-before-element');
jsOuter.id = 'js-before-outer';
const jsInner = document.createElement('parser-fragment-before-element');
jsInner.id = 'js-before-inner';
jsOuter.appendChild(jsInner);
const jsSecond = document.createElement('parser-fragment-before-element');
jsSecond.id = 'js-before-second';
const jsReference = document.createElement('span');
jsReference.id = 'js-before-reference';
const parserOuter = document.createElement('parser-fragment-before-element');
parserOuter.id = 'parser-before-outer';
const parserInner = document.createElement('parser-fragment-before-element');
parserInner.id = 'parser-before-inner';
parserOuter.appendChild(parserInner);
const parserSecond = document.createElement('parser-fragment-before-element');
parserSecond.id = 'parser-before-second';
const parserReference = document.createElement('span');
parserReference.id = 'parser-before-reference';
document.body.append(jsOuter, jsSecond, jsReference, parserOuter, parserSecond, parserReference);
window.parserFragmentBeforeJsFragment = jsFragment;
window.parserFragmentBeforeJsOuter = jsOuter;
window.parserFragmentBeforeJsSecond = jsSecond;
window.parserFragmentBeforeJsReference = jsReference;
window.parserFragmentBeforeParserOuter = parserOuter;
window.parserFragmentBeforeParserSecond = parserSecond;
window.parserFragmentBeforeParserReference = parserReference;
window.parserFragmentBeforeEvents.length = 0;
"#,
                )
                .expect("fragment insert-before setup should evaluate");

            let (parser_outer, parser_second, parser_reference) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-before-outer")
                        .expect("parser fragment outer custom element should exist"),
                    runtime
                        .get_element_by_id("parser-before-second")
                        .expect("parser fragment second custom element should exist"),
                    runtime
                        .get_element_by_id("parser-before-reference")
                        .expect("parser fragment reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentBeforeJsOuter.remove();
window.parserFragmentBeforeJsSecond.remove();
window.parserFragmentBeforeParserOuter.remove();
window.parserFragmentBeforeParserSecond.remove();
window.parserFragmentBeforeEvents.length = 0;
window.parserFragmentBeforeJsFragment.append(
  window.parserFragmentBeforeJsOuter,
  window.parserFragmentBeforeJsSecond
);
document.body.insertBefore(window.parserFragmentBeforeJsFragment, window.parserFragmentBeforeJsReference);
window.parserFragmentBeforeJsEvents = window.parserFragmentBeforeEvents.slice();
window.parserFragmentBeforeEvents.length = 0;
"#,
                )
                .expect("fragment insert-before JS baseline should evaluate");

            let fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_outer));
                assert!(dom_host.append_child(fragment, parser_second));
                fragment
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(parser_reference),
                    },
                    "parser fragment insert-before mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "insertBefore with a fragment of upgraded custom elements should defer connected reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment insertBefore should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser fragment insert-before reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFragmentBeforeJsEvents,
  parserEvents: window.parserFragmentBeforeEvents,
  same: window.parserFragmentBeforeJsEvents.map(event => event.replace('js-before', 'target')).join('|') ===
    window.parserFragmentBeforeEvents.map(event => event.replace('parser-before', 'target')).join('|'),
  jsFragmentEmpty: window.parserFragmentBeforeJsFragment.childNodes.length,
  parserReferencePrevious: window.parserFragmentBeforeParserReference.previousSibling &&
    window.parserFragmentBeforeParserReference.previousSibling.id,
  parserSecondNext: window.parserFragmentBeforeParserSecond.nextSibling &&
    window.parserFragmentBeforeParserSecond.nextSibling.id
})"#,
                )
                .expect("fragment insert-before result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-before-outer:connected:true","js-before-inner:connected:true","js-before-second:connected:true"],"parserEvents":["parser-before-outer:connected:true","parser-before-inner:connected:true","parser-before-second:connected:true"],"same":true,"jsFragmentEmpty":0,"parserReferencePrevious":"parser-before-second","parserSecondNext":"parser-before-reference"}"#
                ),
                "parser DocumentFragment insertBefore should match JS connected callback preorder and insert before the reference child"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_face_insertion_dispatches_form_reactions_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentFaceEvents = [];
class ParserFragmentFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:form:${form && form.dataset.role}`);
  }
  formDisabledCallback(disabled) {
    window.parserFragmentFaceEvents.push(`${this.dataset.role}:${this.dataset.name}:disabled:${disabled}`);
  }
}
customElements.define('parser-fragment-face-element', ParserFragmentFaceElement);

function makeDisabledFieldset(idPrefix, role) {
  const form = document.createElement('form');
  form.id = `${idPrefix}-form`;
  form.dataset.role = role;
  const fieldset = document.createElement('fieldset');
  fieldset.id = `${idPrefix}-fieldset`;
  fieldset.disabled = true;
  form.appendChild(fieldset);
  document.body.appendChild(form);
  return fieldset;
}

function makeFace(id, role, name) {
  const element = document.createElement('parser-fragment-face-element');
  element.id = id;
  element.dataset.role = role;
  element.dataset.name = name;
  return element;
}

window.parserFragmentFaceJsAppendFieldset = makeDisabledFieldset('js-fragment-face-append', 'append');
window.parserFragmentFaceParserAppendFieldset = makeDisabledFieldset('parser-fragment-face-append', 'append');
window.parserFragmentFaceJsBeforeFieldset = makeDisabledFieldset('js-fragment-face-before', 'before');
window.parserFragmentFaceParserBeforeFieldset = makeDisabledFieldset('parser-fragment-face-before', 'before');
window.parserFragmentFaceJsBeforeReference = document.createElement('span');
window.parserFragmentFaceJsBeforeReference.id = 'js-fragment-face-before-reference';
window.parserFragmentFaceParserBeforeReference = document.createElement('span');
window.parserFragmentFaceParserBeforeReference.id = 'parser-fragment-face-before-reference';
window.parserFragmentFaceJsBeforeFieldset.appendChild(window.parserFragmentFaceJsBeforeReference);
window.parserFragmentFaceParserBeforeFieldset.appendChild(window.parserFragmentFaceParserBeforeReference);

window.parserFragmentFaceJsAppendA = makeFace('js-fragment-face-append-a', 'append', 'a');
window.parserFragmentFaceJsAppendB = makeFace('js-fragment-face-append-b', 'append', 'b');
window.parserFragmentFaceParserAppendA = makeFace('parser-fragment-face-append-a', 'append', 'a');
window.parserFragmentFaceParserAppendB = makeFace('parser-fragment-face-append-b', 'append', 'b');
window.parserFragmentFaceJsBeforeA = makeFace('js-fragment-face-before-a', 'before', 'a');
window.parserFragmentFaceJsBeforeB = makeFace('js-fragment-face-before-b', 'before', 'b');
window.parserFragmentFaceParserBeforeA = makeFace('parser-fragment-face-before-a', 'before', 'a');
window.parserFragmentFaceParserBeforeB = makeFace('parser-fragment-face-before-b', 'before', 'b');
document.body.append(
  window.parserFragmentFaceJsAppendA,
  window.parserFragmentFaceJsAppendB,
  window.parserFragmentFaceParserAppendA,
  window.parserFragmentFaceParserAppendB,
  window.parserFragmentFaceJsBeforeA,
  window.parserFragmentFaceJsBeforeB,
  window.parserFragmentFaceParserBeforeA,
  window.parserFragmentFaceParserBeforeB
);
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE setup should evaluate");

            let (
                parser_append_fieldset,
                parser_append_a,
                parser_append_b,
                parser_before_fieldset,
                parser_before_a,
                parser_before_b,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-fragment-face-append-fieldset")
                        .expect("parser fragment FACE append fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-append-a")
                        .expect("parser fragment FACE append first element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-append-b")
                        .expect("parser fragment FACE append second element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-fieldset")
                        .expect("parser fragment FACE before fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-a")
                        .expect("parser fragment FACE before first element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-b")
                        .expect("parser fragment FACE before second element should exist"),
                    runtime
                        .get_element_by_id("parser-fragment-face-before-reference")
                        .expect("parser fragment FACE before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentFaceJsAppendA.remove();
window.parserFragmentFaceJsAppendB.remove();
window.parserFragmentFaceParserAppendA.remove();
window.parserFragmentFaceParserAppendB.remove();
window.parserFragmentFaceJsBeforeA.remove();
window.parserFragmentFaceJsBeforeB.remove();
window.parserFragmentFaceParserBeforeA.remove();
window.parserFragmentFaceParserBeforeB.remove();
window.parserFragmentFaceEvents.length = 0;

const jsAppendFragment = document.createDocumentFragment();
jsAppendFragment.append(window.parserFragmentFaceJsAppendA, window.parserFragmentFaceJsAppendB);
window.parserFragmentFaceJsAppendFieldset.appendChild(jsAppendFragment);
window.parserFragmentFaceJsAppendEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;

const jsBeforeFragment = document.createDocumentFragment();
jsBeforeFragment.append(window.parserFragmentFaceJsBeforeA, window.parserFragmentFaceJsBeforeB);
window.parserFragmentFaceJsBeforeFieldset.insertBefore(
  jsBeforeFragment,
  window.parserFragmentFaceJsBeforeReference
);
window.parserFragmentFaceJsBeforeEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE JS baseline should evaluate");

            let parser_append_fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_append_a));
                assert!(dom_host.append_child(fragment, parser_append_b));
                fragment
            };
            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_fieldset,
                    child: parser_append_fragment,
                },
                "parser fragment FACE append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser fragment FACE append should defer connected/form reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_append_fragment)
                    .count(),
                0,
                "parser fragment FACE append should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser fragment FACE append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentFaceParserAppendEvents = window.parserFragmentFaceEvents.slice();
window.parserFragmentFaceEvents.length = 0;
"#,
                )
                .expect("fragment FACE parser append events should snapshot");

            let parser_before_fragment = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let fragment = dom_host.create_document_fragment();
                assert!(dom_host.append_child(fragment, parser_before_a));
                assert!(dom_host.append_child(fragment, parser_before_b));
                fragment
            };
            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_fieldset,
                    child: parser_before_fragment,
                    reference_child: Some(parser_before_reference),
                },
                "parser fragment FACE insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser fragment FACE insertBefore should defer connected/form reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_before_fragment)
                    .count(),
                0,
                "parser fragment FACE insertBefore should hoist and empty the fragment"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser fragment FACE insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserFragmentFaceEvents.slice();
  const appendSame = JSON.stringify(window.parserFragmentFaceJsAppendEvents) ===
    JSON.stringify(window.parserFragmentFaceParserAppendEvents);
  const beforeSame = JSON.stringify(window.parserFragmentFaceJsBeforeEvents) ===
    JSON.stringify(parserBeforeEvents);
  return JSON.stringify({
    jsAppendEvents: window.parserFragmentFaceJsAppendEvents,
    parserAppendEvents: window.parserFragmentFaceParserAppendEvents,
    appendSame,
    jsBeforeEvents: window.parserFragmentFaceJsBeforeEvents,
    parserBeforeEvents,
    beforeSame,
    parserBeforeReferencePrevious: window.parserFragmentFaceParserBeforeReference.previousSibling &&
      window.parserFragmentFaceParserBeforeReference.previousSibling.id,
    parserBeforeBNext: window.parserFragmentFaceParserBeforeB.nextSibling &&
      window.parserFragmentFaceParserBeforeB.nextSibling.id
  });
})()"#,
                )
                .expect("fragment FACE result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["append:a:connected:true","append:a:form:append","append:a:disabled:true","append:b:connected:true","append:b:form:append","append:b:disabled:true"],"parserAppendEvents":["append:a:connected:true","append:a:form:append","append:a:disabled:true","append:b:connected:true","append:b:form:append","append:b:disabled:true"],"appendSame":true,"jsBeforeEvents":["before:a:connected:true","before:a:form:before","before:a:disabled:true","before:b:connected:true","before:b:form:before","before:b:disabled:true"],"parserBeforeEvents":["before:a:connected:true","before:a:form:before","before:a:disabled:true","before:b:connected:true","before:b:form:before","before:b:disabled:true"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-fragment-face-before-b","parserBeforeBNext":"parser-fragment-face-before-reference"}"#
                ),
                "parser DocumentFragment insertion should dispatch FACE connected/form callbacks like JS fragment insertion for append and insertBefore"
            );
        }));
    }

    #[test]
    fn parser_nested_face_insertion_dispatches_form_reactions_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedFaceEvents = [];
class ParserNestedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:form:${form && form.dataset.mode}`);
  }
  formDisabledCallback(disabled) {
    window.parserNestedFaceEvents.push(`${this.dataset.mode}:${this.dataset.name}:disabled:${disabled}`);
  }
}
customElements.define('parser-nested-face-element', ParserNestedFaceElement);

function makeDisabledFieldset(idPrefix, mode) {
  const form = document.createElement('form');
  form.id = `${idPrefix}-form`;
  form.dataset.mode = mode;
  const fieldset = document.createElement('fieldset');
  fieldset.id = `${idPrefix}-fieldset`;
  fieldset.disabled = true;
  form.appendChild(fieldset);
  document.body.appendChild(form);
  return fieldset;
}

function makeNestedFace(prefix, mode) {
  const outer = document.createElement('parser-nested-face-element');
  outer.id = `${prefix}-outer`;
  outer.dataset.mode = mode;
  outer.dataset.name = 'outer';
  const inner = document.createElement('parser-nested-face-element');
  inner.id = `${prefix}-inner`;
  inner.dataset.mode = mode;
  inner.dataset.name = 'inner';
  outer.appendChild(inner);
  return outer;
}

window.parserNestedFaceJsAppendFieldset = makeDisabledFieldset('js-nested-face-append', 'append');
window.parserNestedFaceParserAppendFieldset = makeDisabledFieldset('parser-nested-face-append', 'append');
window.parserNestedFaceJsBeforeFieldset = makeDisabledFieldset('js-nested-face-before', 'before');
window.parserNestedFaceParserBeforeFieldset = makeDisabledFieldset('parser-nested-face-before', 'before');
window.parserNestedFaceJsBeforeReference = document.createElement('span');
window.parserNestedFaceJsBeforeReference.id = 'js-nested-face-before-reference';
window.parserNestedFaceParserBeforeReference = document.createElement('span');
window.parserNestedFaceParserBeforeReference.id = 'parser-nested-face-before-reference';
window.parserNestedFaceJsBeforeFieldset.appendChild(window.parserNestedFaceJsBeforeReference);
window.parserNestedFaceParserBeforeFieldset.appendChild(window.parserNestedFaceParserBeforeReference);

window.parserNestedFaceJsAppendOuter = makeNestedFace('js-nested-face-append', 'append');
window.parserNestedFaceParserAppendOuter = makeNestedFace('parser-nested-face-append', 'append');
window.parserNestedFaceJsBeforeOuter = makeNestedFace('js-nested-face-before', 'before');
window.parserNestedFaceParserBeforeOuter = makeNestedFace('parser-nested-face-before', 'before');
document.body.append(
  window.parserNestedFaceJsAppendOuter,
  window.parserNestedFaceParserAppendOuter,
  window.parserNestedFaceJsBeforeOuter,
  window.parserNestedFaceParserBeforeOuter
);
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE setup should evaluate");

            let (
                parser_append_fieldset,
                parser_append_outer,
                parser_before_fieldset,
                parser_before_outer,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-nested-face-append-fieldset")
                        .expect("parser nested FACE append fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-append-outer")
                        .expect("parser nested FACE append outer should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-fieldset")
                        .expect("parser nested FACE before fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-outer")
                        .expect("parser nested FACE before outer should exist"),
                    runtime
                        .get_element_by_id("parser-nested-face-before-reference")
                        .expect("parser nested FACE before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedFaceJsAppendOuter.remove();
window.parserNestedFaceParserAppendOuter.remove();
window.parserNestedFaceJsBeforeOuter.remove();
window.parserNestedFaceParserBeforeOuter.remove();
window.parserNestedFaceEvents.length = 0;

window.parserNestedFaceJsAppendFieldset.appendChild(window.parserNestedFaceJsAppendOuter);
window.parserNestedFaceJsAppendEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;

window.parserNestedFaceJsBeforeFieldset.insertBefore(
  window.parserNestedFaceJsBeforeOuter,
  window.parserNestedFaceJsBeforeReference
);
window.parserNestedFaceJsBeforeEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE JS baseline should evaluate");

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_fieldset,
                    child: parser_append_outer,
                },
                "parser nested FACE append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser nested FACE append should defer connected/form reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser nested FACE append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedFaceParserAppendEvents = window.parserNestedFaceEvents.slice();
window.parserNestedFaceEvents.length = 0;
"#,
                )
                .expect("nested FACE parser append events should snapshot");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_fieldset,
                    child: parser_before_outer,
                    reference_child: Some(parser_before_reference),
                },
                "parser nested FACE insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser nested FACE insertBefore should defer connected/form reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser nested FACE insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserNestedFaceEvents.slice();
  return JSON.stringify({
    jsAppendEvents: window.parserNestedFaceJsAppendEvents,
    parserAppendEvents: window.parserNestedFaceParserAppendEvents,
    appendSame: JSON.stringify(window.parserNestedFaceJsAppendEvents) ===
      JSON.stringify(window.parserNestedFaceParserAppendEvents),
    jsBeforeEvents: window.parserNestedFaceJsBeforeEvents,
    parserBeforeEvents,
    beforeSame: JSON.stringify(window.parserNestedFaceJsBeforeEvents) ===
      JSON.stringify(parserBeforeEvents),
    parserBeforeReferencePrevious: window.parserNestedFaceParserBeforeReference.previousSibling &&
      window.parserNestedFaceParserBeforeReference.previousSibling.id,
    parserBeforeOuterNext: window.parserNestedFaceParserBeforeOuter.nextSibling &&
      window.parserNestedFaceParserBeforeOuter.nextSibling.id
  });
})()"#,
                )
                .expect("nested FACE result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["append:outer:connected:true","append:outer:form:append","append:outer:disabled:true","append:inner:connected:true","append:inner:form:append","append:inner:disabled:true"],"parserAppendEvents":["append:outer:connected:true","append:outer:form:append","append:outer:disabled:true","append:inner:connected:true","append:inner:form:append","append:inner:disabled:true"],"appendSame":true,"jsBeforeEvents":["before:outer:connected:true","before:outer:form:before","before:outer:disabled:true","before:inner:connected:true","before:inner:form:before","before:inner:disabled:true"],"parserBeforeEvents":["before:outer:connected:true","before:outer:form:before","before:outer:disabled:true","before:inner:connected:true","before:inner:form:before","before:inner:disabled:true"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-nested-face-before-outer","parserBeforeOuterNext":"parser-nested-face-before-reference"}"#
                ),
                "parser nested FACE insertion should dispatch connected/form callbacks like JS insertion for append and insertBefore"
            );
        }));
    }

    #[test]
    fn parser_shadow_including_insertion_dispatches_connected_reactions_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowInsertionEvents = [];
class ParserShadowInsertionElement extends HTMLElement {
  connectedCallback() {
    const root = this.getRootNode();
    window.parserShadowInsertionEvents.push(
      `${this.id}:connected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
  disconnectedCallback() {
    const root = this.getRootNode();
    window.parserShadowInsertionEvents.push(
      `${this.id}:disconnected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
}
customElements.define('parser-shadow-insertion-element', ParserShadowInsertionElement);

function makeParent(prefix) {
  const parent = document.createElement('section');
  parent.id = `${prefix}-parent`;
  document.body.appendChild(parent);
  return parent;
}

function makeBeforeParent(prefix) {
  const parent = makeParent(prefix);
  const reference = document.createElement('span');
  reference.id = `${prefix}-reference`;
  parent.appendChild(reference);
  return { parent, reference };
}

function makeShadowHost(prefix) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowChild = document.createElement('parser-shadow-insertion-element');
  shadowChild.id = `${prefix}-shadow-child`;
  shadow.appendChild(shadowChild);
  return host;
}

window.parserShadowJsAppendParent = makeParent('js-shadow-append');
window.parserShadowParserAppendParent = makeParent('parser-shadow-append');
const jsBefore = makeBeforeParent('js-shadow-before');
const parserBefore = makeBeforeParent('parser-shadow-before');
window.parserShadowJsBeforeParent = jsBefore.parent;
window.parserShadowJsBeforeReference = jsBefore.reference;
window.parserShadowParserBeforeParent = parserBefore.parent;
window.parserShadowParserBeforeReference = parserBefore.reference;

window.parserShadowJsAppendHost = makeShadowHost('js-shadow-append');
window.parserShadowParserAppendHost = makeShadowHost('parser-shadow-append');
window.parserShadowJsBeforeHost = makeShadowHost('js-shadow-before');
window.parserShadowParserBeforeHost = makeShadowHost('parser-shadow-before');
document.body.append(
  window.parserShadowJsAppendHost,
  window.parserShadowParserAppendHost,
  window.parserShadowJsBeforeHost,
  window.parserShadowParserBeforeHost
);
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including insertion setup should evaluate");

            let (
                parser_append_parent,
                parser_append_host,
                parser_before_parent,
                parser_before_host,
                parser_before_reference,
            ) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-shadow-append-parent")
                        .expect("parser shadow append parent should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-append-host")
                        .expect("parser shadow append host should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-parent")
                        .expect("parser shadow before parent should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-host")
                        .expect("parser shadow before host should exist"),
                    runtime
                        .get_element_by_id("parser-shadow-before-reference")
                        .expect("parser shadow before reference should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowJsAppendHost.remove();
window.parserShadowParserAppendHost.remove();
window.parserShadowJsBeforeHost.remove();
window.parserShadowParserBeforeHost.remove();
window.parserShadowInsertionEvents.length = 0;

window.parserShadowJsAppendParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowJsAppendEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;

window.parserShadowJsBeforeParent.insertBefore(
  window.parserShadowJsBeforeHost,
  window.parserShadowJsBeforeReference
);
window.parserShadowJsBeforeEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including JS baseline should evaluate");

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_parent,
                    child: parser_append_host,
                },
                "parser shadow-including append should apply",
            );
            assert!(
                !append_reaction_roots.is_empty(),
                "parser shadow-including append should defer connected reactions for shadow descendants"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(append_reaction_roots)
                .expect("parser shadow-including append reactions should dispatch");
            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowParserAppendEvents = window.parserShadowInsertionEvents.slice();
window.parserShadowInsertionEvents.length = 0;
"#,
                )
                .expect("shadow-including parser append events should snapshot");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_parent,
                    child: parser_before_host,
                    reference_child: Some(parser_before_reference),
                },
                "parser shadow-including insertBefore should apply",
            );
            assert!(
                !before_reaction_roots.is_empty(),
                "parser shadow-including insertBefore should defer connected reactions for shadow descendants"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(before_reaction_roots)
                .expect("parser shadow-including insertBefore reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const parserBeforeEvents = window.parserShadowInsertionEvents.slice();
  const normalize = (events, prefix) =>
    events.map(event => event.replaceAll(prefix, 'target'));
  return JSON.stringify({
    jsAppendEvents: window.parserShadowJsAppendEvents,
    parserAppendEvents: window.parserShadowParserAppendEvents,
    appendSame: JSON.stringify(normalize(window.parserShadowJsAppendEvents, 'js-shadow-append')) ===
      JSON.stringify(normalize(window.parserShadowParserAppendEvents, 'parser-shadow-append')),
    jsBeforeEvents: window.parserShadowJsBeforeEvents,
    parserBeforeEvents,
    beforeSame: JSON.stringify(normalize(window.parserShadowJsBeforeEvents, 'js-shadow-before')) ===
      JSON.stringify(normalize(parserBeforeEvents, 'parser-shadow-before')),
    parserBeforeReferencePrevious: window.parserShadowParserBeforeReference.previousSibling &&
      window.parserShadowParserBeforeReference.previousSibling.id,
    parserBeforeHostNext: window.parserShadowParserBeforeHost.nextSibling &&
      window.parserShadowParserBeforeHost.nextSibling.id
  });
})()"#,
                )
                .expect("shadow-including insertion result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppendEvents":["js-shadow-append-shadow-child:connected:true:true:js-shadow-append-host"],"parserAppendEvents":["parser-shadow-append-shadow-child:connected:true:true:parser-shadow-append-host"],"appendSame":true,"jsBeforeEvents":["js-shadow-before-shadow-child:connected:true:true:js-shadow-before-host"],"parserBeforeEvents":["parser-shadow-before-shadow-child:connected:true:true:parser-shadow-before-host"],"beforeSame":true,"parserBeforeReferencePrevious":"parser-shadow-before-host","parserBeforeHostNext":"parser-shadow-before-reference"}"#
                ),
                "parser connected insertion should dispatch connectedCallback for upgraded shadow-including descendants like JS insertion"
            );
        }));
    }

    #[test]
    fn parser_shadow_including_remove_reparent_dispatches_disconnected_reactions_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserShadowDisconnectEvents = [];
class ParserShadowDisconnectedElement extends HTMLElement {
  connectedCallback() {
    const root = this.getRootNode();
    window.parserShadowDisconnectEvents.push(
      `${this.id}:connected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
  disconnectedCallback() {
    const root = this.getRootNode();
    window.parserShadowDisconnectEvents.push(
      `${this.id}:disconnected:${this.isConnected}:${root instanceof ShadowRoot}:${root.host && root.host.id}`
    );
  }
}
customElements.define('parser-shadow-disconnected-element', ParserShadowDisconnectedElement);

function makeConnectedParent(prefix) {
  const parent = document.createElement('section');
  parent.id = `${prefix}-parent`;
  document.body.appendChild(parent);
  return parent;
}

function makeShadowHost(prefix) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  const shadowChild = document.createElement('parser-shadow-disconnected-element');
  shadowChild.id = `${prefix}-shadow-child`;
  shadow.appendChild(shadowChild);
  return host;
}

window.parserShadowJsRemoveParent = makeConnectedParent('js-shadow-remove');
window.parserShadowParserRemoveParent = makeConnectedParent('parser-shadow-remove');
window.parserShadowJsAppendParent = makeConnectedParent('js-shadow-append-disconnect');
window.parserShadowParserAppendParent = makeConnectedParent('parser-shadow-append-disconnect');
window.parserShadowJsBeforeParent = makeConnectedParent('js-shadow-before-disconnect');
window.parserShadowParserBeforeParent = makeConnectedParent('parser-shadow-before-disconnect');

window.parserShadowJsRemoveHost = makeShadowHost('js-shadow-remove');
window.parserShadowParserRemoveHost = makeShadowHost('parser-shadow-remove');
window.parserShadowJsAppendHost = makeShadowHost('js-shadow-append-disconnect');
window.parserShadowParserAppendHost = makeShadowHost('parser-shadow-append-disconnect');
window.parserShadowJsBeforeHost = makeShadowHost('js-shadow-before-disconnect');
window.parserShadowParserBeforeHost = makeShadowHost('parser-shadow-before-disconnect');

window.parserShadowJsRemoveParent.appendChild(window.parserShadowJsRemoveHost);
window.parserShadowParserRemoveParent.appendChild(window.parserShadowParserRemoveHost);
window.parserShadowJsAppendParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowParserAppendParent.appendChild(window.parserShadowParserAppendHost);
window.parserShadowJsBeforeParent.appendChild(window.parserShadowJsBeforeHost);
window.parserShadowParserBeforeParent.appendChild(window.parserShadowParserBeforeHost);

window.parserShadowJsAppendDetachedParent = document.createElement('div');
window.parserShadowJsBeforeDetachedParent = document.createElement('div');
window.parserShadowJsBeforeReference = document.createElement('span');
window.parserShadowJsBeforeReference.id = 'js-shadow-before-disconnect-reference';
window.parserShadowJsBeforeDetachedParent.appendChild(window.parserShadowJsBeforeReference);

window.parserShadowDisconnectEvents.length = 0;
window.parserShadowJsRemoveParent.removeChild(window.parserShadowJsRemoveHost);
window.parserShadowJsRemoveEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;

window.parserShadowJsAppendDetachedParent.appendChild(window.parserShadowJsAppendHost);
window.parserShadowJsAppendEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;

window.parserShadowJsBeforeDetachedParent.insertBefore(
  window.parserShadowJsBeforeHost,
  window.parserShadowJsBeforeReference
);
window.parserShadowJsBeforeEvents = window.parserShadowDisconnectEvents.slice();
window.parserShadowDisconnectEvents.length = 0;
"#,
                )
                .expect("shadow-including disconnected setup should evaluate");

            let (
                parser_remove_parent,
                parser_remove_host,
                parser_append_host,
                parser_before_host,
                append_detached_parent,
                before_detached_parent,
                before_reference,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let parser_remove_parent = dom_host
                    .element_handle_by_id("parser-shadow-remove-parent")
                    .expect("parser shadow remove parent should exist");
                let parser_remove_host = dom_host
                    .element_handle_by_id("parser-shadow-remove-host")
                    .expect("parser shadow remove host should exist");
                let parser_append_host = dom_host
                    .element_handle_by_id("parser-shadow-append-disconnect-host")
                    .expect("parser shadow append host should exist");
                let parser_before_host = dom_host
                    .element_handle_by_id("parser-shadow-before-disconnect-host")
                    .expect("parser shadow before host should exist");
                let append_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let before_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let before_reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    before_reference,
                    "id",
                    "parser-shadow-before-disconnect-reference"
                ));
                assert!(dom_host.append_child(before_detached_parent, before_reference));
                (
                    parser_remove_parent,
                    parser_remove_host,
                    parser_append_host,
                    parser_before_host,
                    append_detached_parent,
                    before_detached_parent,
                    before_reference,
                )
            };

            let reaction_roots = ParserPostStepRuntimeWorkForTest::merge_for_test([
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: parser_remove_parent,
                        child: parser_remove_host,
                    },
                    "parser shadow-including remove should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: append_detached_parent,
                        child: parser_append_host,
                    },
                    "parser shadow-including append to detached parent should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: before_detached_parent,
                        child: parser_before_host,
                        reference_child: Some(before_reference),
                    },
                    "parser shadow-including insertBefore into detached parent should apply",
                ),
            ]);
            assert!(
                !reaction_roots.is_empty(),
                "parser shadow-including remove/reparent should defer disconnected reactions for shadow descendants"
            );

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host
                        .child_handles(before_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![parser_before_host, before_reference],
                    "parser shadow-including insertBefore should move the host before the native reference"
                );
                assert_eq!(
                    dom_host
                        .child_handles(append_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![parser_append_host],
                    "parser shadow-including AppendChild should move the host under the native detached parent"
                );
            }

            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser shadow-including disconnected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const normalize = (events) =>
    events.map(event => event.replaceAll('js-shadow-', '').replaceAll('parser-shadow-', ''));
  const parserEvents = window.parserShadowDisconnectEvents.slice();
  return JSON.stringify({
    jsEvents: [
      ...window.parserShadowJsRemoveEvents,
      ...window.parserShadowJsAppendEvents,
      ...window.parserShadowJsBeforeEvents
    ],
    parserEvents,
    same: JSON.stringify(normalize([
      ...window.parserShadowJsRemoveEvents,
      ...window.parserShadowJsAppendEvents,
      ...window.parserShadowJsBeforeEvents
    ])) === JSON.stringify(normalize(parserEvents)),
    parserRemoveParent: window.parserShadowParserRemoveHost.parentNode && window.parserShadowParserRemoveHost.parentNode.id,
    parserAppendParentConnected: window.parserShadowParserAppendHost.parentNode &&
      window.parserShadowParserAppendHost.parentNode.isConnected,
    parserBeforeParentConnected: window.parserShadowParserBeforeHost.parentNode &&
      window.parserShadowParserBeforeHost.parentNode.isConnected,
    parserBeforeHostNext: window.parserShadowParserBeforeHost.nextSibling &&
      window.parserShadowParserBeforeHost.nextSibling.id
  });
})()"#,
                )
                .expect("shadow-including disconnected result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-shadow-remove-shadow-child:disconnected:false:true:js-shadow-remove-host","js-shadow-append-disconnect-shadow-child:disconnected:false:true:js-shadow-append-disconnect-host","js-shadow-before-disconnect-shadow-child:disconnected:false:true:js-shadow-before-disconnect-host"],"parserEvents":["parser-shadow-remove-shadow-child:disconnected:false:true:parser-shadow-remove-host","parser-shadow-append-disconnect-shadow-child:disconnected:false:true:parser-shadow-append-disconnect-host","parser-shadow-before-disconnect-shadow-child:disconnected:false:true:parser-shadow-before-disconnect-host"],"same":true,"parserRemoveParent":null,"parserAppendParentConnected":false,"parserBeforeParentConnected":false,"parserBeforeHostNext":"parser-shadow-before-disconnect-reference"}"#
                ),
                "parser remove/reparent to a disconnected parent should dispatch disconnectedCallback for upgraded shadow-including descendants like JS mutation"
            );
        }));
    }

    #[test]
    fn parser_reparent_nested_custom_element_to_disconnected_parent_matches_js_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserNestedMoveEvents = [];
class ParserNestedMovedElement extends HTMLElement {
  connectedCallback() {
    window.parserNestedMoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserNestedMoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-nested-moved-element', ParserNestedMovedElement);
function createNestedMoved(prefix) {
  const outer = document.createElement('parser-nested-moved-element');
  outer.id = `${prefix}-outer`;
  const inner = document.createElement('parser-nested-moved-element');
  inner.id = `${prefix}-inner`;
  outer.appendChild(inner);
  return outer;
}
const connectedParent = document.createElement('div');
const jsBeforeDetachedParent = document.createElement('div');
const jsBeforeReference = document.createElement('span');
jsBeforeDetachedParent.appendChild(jsBeforeReference);
const jsAppendDetachedParent = document.createElement('div');
const jsBeforeOuter = createNestedMoved('js-before-nested');
const jsAppendOuter = createNestedMoved('js-append-nested');
const parserBeforeOuter = createNestedMoved('parser-before-nested');
const parserAppendOuter = createNestedMoved('parser-append-nested');
connectedParent.append(jsBeforeOuter, jsAppendOuter, parserBeforeOuter, parserAppendOuter);
document.body.appendChild(connectedParent);
window.parserBeforeNestedOuter = parserBeforeOuter;
window.parserAppendNestedOuter = parserAppendOuter;
window.parserNestedMoveEvents.length = 0;
jsBeforeDetachedParent.insertBefore(jsBeforeOuter, jsBeforeReference);
jsAppendDetachedParent.appendChild(jsAppendOuter);
"#,
                )
                .expect("nested custom-element reparent setup should evaluate");

            let (
                before_detached_parent,
                before_reference,
                before_target,
                append_detached_parent,
                append_target,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let before_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let before_reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(before_detached_parent, before_reference));
                let before_target = dom_host
                    .element_handle_by_id("parser-before-nested-outer")
                    .expect("parser insertBefore nested custom element should exist before reparent");
                let append_detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let append_target = dom_host
                    .element_handle_by_id("parser-append-nested-outer")
                    .expect("parser append nested custom element should exist before reparent");
                (
                    before_detached_parent,
                    before_reference,
                    before_target,
                    append_detached_parent,
                    append_target,
                )
            };

            let before_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: before_detached_parent,
                        child: before_target,
                        reference_child: Some(before_reference),
                    },
                    "parser nested insertBefore reparent mutation should apply",
                )
            };
            assert!(
                !before_reaction_roots.is_empty(),
                "parser insertBefore moving a connected custom-element subtree to a disconnected parent should defer disconnected reactions"
            );
            let append_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: append_detached_parent,
                        child: append_target,
                    },
                    "parser nested append reparent mutation should apply",
                )
            };
            assert!(
                !append_reaction_roots.is_empty(),
                "parser AppendChild moving a connected custom-element subtree to a disconnected parent should defer disconnected reactions"
            );
            let custom_element_reaction_roots =
                ParserPostStepRuntimeWorkForTest::merge_for_test([
                    before_reaction_roots,
                    append_reaction_roots,
                ]);

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host
                        .child_handles(before_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![before_target, before_reference],
                    "parser insertBefore should move the nested custom element before the native reference"
                );
                assert_eq!(
                    dom_host
                        .child_handles(append_detached_parent)
                        .collect::<Vec<_>>(),
                    vec![append_target],
                    "parser AppendChild should move the nested custom element under the native detached parent"
                );
            }

            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving connected custom-element subtrees to disconnected parents should defer disconnected reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser nested disconnected reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserNestedMoveEvents,
  parserBeforeParentConnected: window.parserBeforeNestedOuter.parentNode && window.parserBeforeNestedOuter.parentNode.isConnected,
  parserAppendParentConnected: window.parserAppendNestedOuter.parentNode && window.parserAppendNestedOuter.parentNode.isConnected
})"#,
                )
                .expect("nested custom-element reparent result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-before-nested-outer:disconnected:false","js-before-nested-inner:disconnected:false","js-append-nested-outer:disconnected:false","js-append-nested-inner:disconnected:false","parser-before-nested-outer:disconnected:false","parser-before-nested-inner:disconnected:false","parser-append-nested-outer:disconnected:false","parser-append-nested-inner:disconnected:false"],"parserBeforeParentConnected":false,"parserAppendParentConnected":false}"#
                ),
                "parser reparent to a disconnected parent should match JS disconnected callback preorder for insertBefore and appendChild"
            );
        }));
    }

    #[test]
    fn parser_mutation_owner_dispatches_removed_custom_element_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserRemoveEvents = [];
class ParserRemovedElement extends HTMLElement {
  connectedCallback() {
    window.parserRemoveEvents.push(`connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserRemoveEvents.push(`disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-removed-element', ParserRemovedElement);
const parserRemoveTarget = document.createElement('parser-removed-element');
parserRemoveTarget.id = 'parser-remove-target';
window.parserRemoveTarget = parserRemoveTarget;
document.body.appendChild(parserRemoveTarget);
"#,
                )
                .expect("custom element setup should evaluate");

            let target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-remove-target")
                .expect("custom element should be connected before parser removal");

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: body,
                        child: target,
                    },
                    "parser remove mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "parser removal should defer disconnected reactions until the parser pump returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser removal reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserRemoveEvents,
  connected: window.parserRemoveTarget.isConnected,
  parent: window.parserRemoveTarget.parentNode && window.parserRemoveTarget.parentNode.nodeName
})"#,
                )
                .expect("custom element removal result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"events":["connected:true","disconnected:false"],"connected":false,"parent":null}"#),
                "parser remove should run the same disconnected lifecycle reaction as JS removeChild"
            );
        }));
    }

    #[test]
    fn parser_reparent_connected_custom_element_does_not_reconnect_callback() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserMoveEvents = [];
class ParserMovedElement extends HTMLElement {
  connectedCallback() {
    window.parserMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  disconnectedCallback() {
    window.parserMoveEvents.push(`${this.id}:disconnected:${this.parentNode && this.parentNode.id}`);
  }
}
customElements.define('parser-moved-element', ParserMovedElement);
const a = document.createElement('div');
a.id = 'parser-move-a';
const b = document.createElement('div');
b.id = 'parser-move-b';
const parserTarget = document.createElement('parser-moved-element');
parserTarget.id = 'parser-target';
const jsTarget = document.createElement('parser-moved-element');
jsTarget.id = 'js-target';
window.parserTarget = parserTarget;
window.jsTarget = jsTarget;
a.append(parserTarget, jsTarget);
document.body.append(a, b);
b.appendChild(jsTarget);
"#,
                )
                .expect("custom element move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-move-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-target")
                        .expect("custom element should be connected before parser reparent"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser reparent mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "moving an already-connected custom element should not defer a connected reaction"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("empty parser move reactions should dispatch as a no-op");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserMoveEvents,
  parserParent: window.parserTarget.parentNode && window.parserTarget.parentNode.id,
  jsParent: window.jsTarget.parentNode && window.jsTarget.parentNode.id
})"#,
                )
                .expect("custom element move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["parser-target:connected:parser-move-a","js-target:connected:parser-move-a"],"parserParent":"parser-move-b","jsParent":"parser-move-b"}"#
                ),
                "parser reparent should match JS insertBefore and avoid reconnecting an already-connected custom element"
            );
        }));
    }

    #[test]
    fn parser_reparent_connected_custom_element_does_not_dispatch_atomic_move_callback() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserAtomicMoveEvents = [];
class ParserAtomicMoveElement extends HTMLElement {
  connectedCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  disconnectedCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:disconnected:${this.parentNode && this.parentNode.id}`);
  }
  connectedMoveCallback() {
    window.parserAtomicMoveEvents.push(`${this.id}:move:${this.parentNode && this.parentNode.id}`);
  }
}
customElements.define('parser-atomic-move-element', ParserAtomicMoveElement);
const a = document.createElement('div');
a.id = 'parser-atomic-a';
const b = document.createElement('div');
b.id = 'parser-atomic-b';
const c = document.createElement('div');
c.id = 'parser-atomic-c';
const parserTarget = document.createElement('parser-atomic-move-element');
parserTarget.id = 'parser-atomic-target';
const jsInsertTarget = document.createElement('parser-atomic-move-element');
jsInsertTarget.id = 'js-insert-target';
const jsMoveTarget = document.createElement('parser-atomic-move-element');
jsMoveTarget.id = 'js-move-target';
window.parserAtomicTarget = parserTarget;
window.jsInsertTarget = jsInsertTarget;
window.jsMoveTarget = jsMoveTarget;
a.append(parserTarget, jsInsertTarget, jsMoveTarget);
document.body.append(a, b, c);
window.parserAtomicMoveEvents.length = 0;
b.insertBefore(jsInsertTarget, null);
c.moveBefore(jsMoveTarget, null);
"#,
                )
                .expect("custom element atomic move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-atomic-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-atomic-target")
                        .expect("parser custom element should be connected before reparent"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser atomic-move comparison reparent mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "parser ordinary reparent should not defer connected or atomic move reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("empty parser atomic move comparison reactions should dispatch as a no-op");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserAtomicMoveEvents,
  parserParent: window.parserAtomicTarget.parentNode && window.parserAtomicTarget.parentNode.id,
  jsInsertParent: window.jsInsertTarget.parentNode && window.jsInsertTarget.parentNode.id,
  jsMoveParent: window.jsMoveTarget.parentNode && window.jsMoveTarget.parentNode.id
})"#,
                )
                .expect("custom element atomic move comparison result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-move-target:move:parser-atomic-c"],"parserParent":"parser-atomic-b","jsInsertParent":"parser-atomic-b","jsMoveParent":"parser-atomic-c"}"#
                ),
                "parser reparent should match JS insertBefore and must not dispatch connectedMoveCallback"
            );
        }));
    }

    #[test]
    fn parser_reparent_focused_subtree_resets_focus_after_parser_step() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFocusMoveEvents = [];
const a = document.createElement('div');
a.id = 'parser-focus-move-a';
const b = document.createElement('div');
b.id = 'parser-focus-move-b';
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-focus-target';
const jsTarget = document.createElement('input');
jsTarget.id = 'js-focus-target';
window.parserFocusTarget = parserTarget;
window.jsFocusTarget = jsTarget;
for (const target of [parserTarget, jsTarget]) {
  target.addEventListener('blur', () => {
    window.parserFocusMoveEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserFocusMoveEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}
a.append(parserTarget, jsTarget);
document.body.append(a, b);
jsTarget.focus();
b.insertBefore(jsTarget, null);
parserTarget.focus();
"#,
                )
                .expect("focused move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-focus-move-b")
                        .expect("target parser parent should exist"),
                    runtime
                        .get_element_by_id("parser-focus-target")
                        .expect("focused parser target should exist"),
                )
            };

            let focus_reset_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser focused reparent mutation should apply",
                )
            };
            assert!(
                !focus_reset_roots.is_empty(),
                "moving a focused connected subtree should defer focus reset until the parser step returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused reparent followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFocusMoveEvents,
  parserParent: window.parserFocusTarget.parentNode && window.parserFocusTarget.parentNode.id,
  jsParent: window.jsFocusTarget.parentNode && window.jsFocusTarget.parentNode.id,
  parserFocused: document.activeElement === window.parserFocusTarget
})"#,
                )
                .expect("focused move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-target:blur:false","js-focus-target:focusout:false","parser-focus-target:blur:false","parser-focus-target:focusout:false"],"parserParent":"parser-focus-move-b","jsParent":"parser-focus-move-b","parserFocused":false}"#
                ),
                "parser reparent should match JS insertBefore focus reset for focused moved subtrees"
            );
        }));
    }

    #[test]
    fn parser_append_child_focused_subtree_to_disconnected_parent_resets_focus_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFocusAppendDetachEvents = [];
function observeFocusAppendDetach(target) {
  target.addEventListener('blur', () => {
    window.parserFocusAppendDetachEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserFocusAppendDetachEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const jsSource = document.createElement('div');
const jsDetachedParent = document.createElement('div');
const jsTarget = document.createElement('input');
jsTarget.id = 'js-focus-append-detach-target';
observeFocusAppendDetach(jsTarget);
jsSource.appendChild(jsTarget);

const parserSource = document.createElement('div');
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-focus-append-detach-target';
window.parserFocusAppendDetachTarget = parserTarget;
observeFocusAppendDetach(parserTarget);
parserSource.appendChild(parserTarget);

document.body.append(jsSource, parserSource);

jsTarget.focus();
jsDetachedParent.appendChild(jsTarget);
window.parserFocusAppendDetachJsFocused = document.activeElement === jsTarget;

parserTarget.focus();
"#,
                )
                .expect("focused append-to-detached setup should evaluate");

            let (detached_parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-focus-append-detach-target")
                    .expect("focused parser append-to-detached target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    detached_parent,
                    "id",
                    "parser-focus-append-detach-parent"
                ));
                (detached_parent, target)
            };

            let focus_reset_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: detached_parent,
                    child: target,
                },
                "parser focused append-to-detached mutation should apply",
            );
            assert!(
                !focus_reset_roots.is_empty(),
                "parser AppendChild moving a focused connected subtree to a disconnected parent should defer focus reset"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused append-to-detached followups should dispatch");

            {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                assert_eq!(
                    dom_host.child_handles(detached_parent).collect::<Vec<_>>(),
                    vec![target],
                    "parser AppendChild should move the focused target under the native detached parent"
                );
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFocusAppendDetachEvents,
  jsFocused: window.parserFocusAppendDetachJsFocused,
  parserFocused: document.activeElement === window.parserFocusAppendDetachTarget
})"#,
                )
                .expect("focused append-to-detached result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-append-detach-target:blur:false","js-focus-append-detach-target:focusout:false","parser-focus-append-detach-target:blur:false","parser-focus-append-detach-target:focusout:false"],"jsFocused":false,"parserFocused":false}"#
                ),
                "parser AppendChild to a disconnected parent should match JS appendChild focus reset for focused moved subtrees"
            );
        }));
    }

    #[test]
    fn js_append_and_replace_child_reparent_focused_subtree_reset_focus_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let result = page_vm
                .evaluate_expression(
                    r#"
window.jsFocusedReparentEvents = [];
function observe(target) {
  target.addEventListener('blur', () => {
    window.jsFocusedReparentEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.jsFocusedReparentEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const appendSource = document.createElement('div');
appendSource.id = 'js-focus-append-source';
const appendDest = document.createElement('div');
appendDest.id = 'js-focus-append-dest';
const appendTarget = document.createElement('input');
appendTarget.id = 'js-focus-append-target';
observe(appendTarget);
appendSource.appendChild(appendTarget);

const replaceSource = document.createElement('div');
replaceSource.id = 'js-focus-replace-source';
const replaceDest = document.createElement('div');
replaceDest.id = 'js-focus-replace-dest';
const replaceSlot = document.createElement('span');
replaceSlot.id = 'js-focus-replace-slot';
const replaceTarget = document.createElement('input');
replaceTarget.id = 'js-focus-replace-target';
observe(replaceTarget);
replaceSource.appendChild(replaceTarget);
replaceDest.appendChild(replaceSlot);

document.body.append(appendSource, appendDest, replaceSource, replaceDest);

appendTarget.focus();
appendDest.appendChild(appendTarget);
const appendFocused = document.activeElement === appendTarget;

replaceTarget.focus();
replaceDest.replaceChild(replaceTarget, replaceSlot);
const replaceFocused = document.activeElement === replaceTarget;

JSON.stringify({
  events: window.jsFocusedReparentEvents,
  appendFocused,
  replaceFocused,
  appendParent: appendTarget.parentNode && appendTarget.parentNode.id,
  replaceParent: replaceTarget.parentNode && replaceTarget.parentNode.id
})
"#,
                )
                .expect("focused append/replace reparent should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-focus-append-target:blur:false","js-focus-append-target:focusout:false","js-focus-replace-target:blur:false","js-focus-replace-target:focusout:false"],"appendFocused":false,"replaceFocused":false,"appendParent":"js-focus-append-dest","replaceParent":"js-focus-replace-dest"}"#
                ),
                "appendChild and replaceChild should reset focused moved subtrees like insertBefore"
            );
        }));
    }

    #[test]
    fn js_and_parser_remove_focused_subtree_reset_focus_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserRemoveFocusEvents = [];
function observe(target) {
  target.addEventListener('blur', () => {
    window.parserRemoveFocusEvents.push(`${target.id}:blur:${document.activeElement === target}`);
  });
  target.addEventListener('focusout', () => {
    window.parserRemoveFocusEvents.push(`${target.id}:focusout:${document.activeElement === target}`);
  });
}

const jsParent = document.createElement('div');
jsParent.id = 'js-remove-focus-parent';
const jsTarget = document.createElement('input');
jsTarget.id = 'js-remove-focus-target';
observe(jsTarget);
jsParent.appendChild(jsTarget);

const parserParent = document.createElement('div');
parserParent.id = 'parser-remove-focus-parent';
const parserTarget = document.createElement('input');
parserTarget.id = 'parser-remove-focus-target';
observe(parserTarget);
parserParent.appendChild(parserTarget);
window.parserRemoveFocusParserTarget = parserTarget;

document.body.append(jsParent, parserParent);
jsTarget.focus();
jsParent.removeChild(jsTarget);
window.parserRemoveFocusJsState = {
  focused: document.activeElement === jsTarget,
  parent: jsTarget.parentNode && jsTarget.parentNode.id
};
parserTarget.focus();
"#,
                )
                .expect("focused remove setup should evaluate");

            let (parser_parent, parser_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-remove-focus-parent")
                        .expect("focused parser remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-remove-focus-target")
                        .expect("focused parser remove target should exist"),
                )
            };

            let focus_reset_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: parser_parent,
                        child: parser_target,
                    },
                    "parser focused remove mutation should apply",
                )
            };
            assert!(
                !focus_reset_roots.is_empty(),
                "removing a focused connected subtree should defer focus reset until the parser step returns"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(focus_reset_roots)
                .expect("parser focused remove followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserRemoveFocusEvents,
  js: window.parserRemoveFocusJsState,
  parser: {
    focused: document.activeElement === window.parserRemoveFocusParserTarget,
    parent: window.parserRemoveFocusParserTarget.parentNode &&
      window.parserRemoveFocusParserTarget.parentNode.id
  }
})"#,
                )
                .expect("focused remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-remove-focus-target:blur:false","js-remove-focus-target:focusout:false","parser-remove-focus-target:blur:false","parser-remove-focus-target:focusout:false"],"js":{"focused":false,"parent":null},"parser":{"focused":false,"parent":null}}"#
                ),
                "JS and parser removeChild should reset focused removed subtrees"
            );
        }));
    }

    #[test]
    fn parser_remove_pending_pointer_capture_target_clears_like_js_remove() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            page_vm.vm_mut().force_fresh_layout_reads_for_test();

            page_vm
                .evaluate_expression(
                    r#"
window.parserPointerCaptureLog = [];
function installPendingPointerCaptureTarget(prefix, removeWithJs) {
  const button = document.createElement('div');
  button.setAttribute('id', `${prefix}-button`);
  button.textContent = `${prefix} button`;
  const target = document.createElement('div');
  target.setAttribute('id', `${prefix}-target`);
  target.textContent = `${prefix} target`;
  button.addEventListener('pointerdown', event => {
    window.parserPointerCaptureLog.push(`${prefix}:pointerdown`);
    target.setPointerCapture(event.pointerId);
    window.parserPointerCaptureLog.push(
      `${prefix}:has-before:${target.hasPointerCapture(event.pointerId)}`
    );
    if (removeWithJs) {
      target.remove();
      window.parserPointerCaptureLog.push(
        `${prefix}:has-after-js-remove:${target.hasPointerCapture(event.pointerId)}`
      );
    }
  });
  button.addEventListener('pointerup', () => {
    window.parserPointerCaptureLog.push(`${prefix}:pointerup`);
  });
  target.addEventListener('gotpointercapture', () => {
    window.parserPointerCaptureLog.push(`${prefix}:gotpointercapture`);
  });
  target.addEventListener('lostpointercapture', () => {
    window.parserPointerCaptureLog.push(`${prefix}:lostpointercapture`);
  });
  document.body.append(button, target);
  return { button, target };
}
window.jsPendingPointerCapture = installPendingPointerCaptureTarget('js-pointer', true);
window.parserPendingPointerCapture = installPendingPointerCaptureTarget('parser-pointer', false);
"#,
                )
                .expect("pending pointer capture parser mutation setup should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mousedown should set then clear pending capture");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mouseup should not use removed pending capture");
            let js_log = page_vm
                .evaluate_expression(
                    r#"(() => {
  const log = window.parserPointerCaptureLog.splice(0).join('|');
  window.jsPendingPointerCapture.button.remove();
  return log;
})()"#,
                )
                .expect("JS pending pointer capture remove baseline should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mousedown should set pending capture");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-pointer-target")
                .expect("parser pending pointer capture target should exist");
            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: body,
                    child: parser_target,
                },
                "parser pending pointer capture target removal should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser pending pointer capture followups should dispatch");
            page_vm
                .evaluate_expression(
                    r#"window.parserPointerCaptureLog.push(
  `parser-pointer:has-after-parser-remove:${
    window.parserPendingPointerCapture.target.hasPointerCapture(1)
  }`
)"#,
                )
                .expect("parser pending pointer capture state should evaluate");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mouseup should not use removed pending capture");
            let parser_log = page_vm
                .evaluate_expression("window.parserPointerCaptureLog.join('|')")
                .expect("parser pending pointer capture remove log should evaluate");

            assert_eq!(
                js_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "js-pointer:pointerdown|js-pointer:has-before:true|js-pointer:has-after-js-remove:false|js-pointer:pointerup"
                ),
                "JS remove baseline should clear pending pointer capture immediately"
            );
            assert_eq!(
                parser_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "parser-pointer:pointerdown|parser-pointer:has-before:true|parser-pointer:has-after-parser-remove:false|parser-pointer:pointerup"
                ),
                "parser remove should clear pending pointer capture before the next pointer event"
            );
        }));
    }

    #[test]
    fn parser_reparent_pending_pointer_capture_target_to_disconnected_parent_clears_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            page_vm.vm_mut().force_fresh_layout_reads_for_test();

            page_vm
                .evaluate_expression(
                    r#"
window.parserPointerCaptureReparentLog = [];
function installPendingPointerCaptureReparentTarget(prefix, reparentWithJs) {
  const button = document.createElement('div');
  button.setAttribute('id', `${prefix}-button`);
  button.textContent = `${prefix} button`;
  const target = document.createElement('div');
  target.setAttribute('id', `${prefix}-target`);
  target.textContent = `${prefix} target`;
  const detachedParent = document.createElement('div');
  detachedParent.setAttribute('id', `${prefix}-detached-parent`);
  button.addEventListener('pointerdown', event => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:pointerdown`);
    target.setPointerCapture(event.pointerId);
    window.parserPointerCaptureReparentLog.push(
      `${prefix}:has-before:${target.hasPointerCapture(event.pointerId)}`
    );
    if (reparentWithJs) {
      detachedParent.appendChild(target);
      window.parserPointerCaptureReparentLog.push(
        `${prefix}:has-after-js-reparent:${target.hasPointerCapture(event.pointerId)}`
      );
    }
  });
  button.addEventListener('pointerup', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:pointerup`);
  });
  target.addEventListener('gotpointercapture', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:gotpointercapture`);
  });
  target.addEventListener('lostpointercapture', () => {
    window.parserPointerCaptureReparentLog.push(`${prefix}:lostpointercapture`);
  });
  document.body.append(button, target);
  return { button, target, detachedParent };
}
window.jsPendingPointerCaptureReparent =
  installPendingPointerCaptureReparentTarget('js-pointer-reparent', true);
window.parserPendingPointerCaptureReparent =
  installPendingPointerCaptureReparentTarget('parser-pointer-reparent', false);
"#,
                )
                .expect("pending pointer capture reparent setup should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mousedown should set then clear pending capture by reparent");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("JS baseline mouseup should not use reparented pending capture");
            let js_log = page_vm
                .evaluate_expression(
                    r#"(() => {
  const log = window.parserPointerCaptureReparentLog.splice(0).join('|');
  window.jsPendingPointerCaptureReparent.button.remove();
  return log;
})()"#,
                )
                .expect("JS pending pointer capture reparent baseline should evaluate");

            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mousedown",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mousedown should set pending capture");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-pointer-reparent-target")
                .expect("parser pending pointer capture reparent target should exist");
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                )
            };
            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_target,
                },
                "parser pending pointer capture target reparent should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser pending pointer capture reparent followups should dispatch");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_target)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the pending capture target under the native detached parent"
            );
            page_vm
                .evaluate_expression(
                    r#"window.parserPointerCaptureReparentLog.push(
  `parser-pointer-reparent:has-after-parser-reparent:${
    window.parserPendingPointerCaptureReparent.target.hasPointerCapture(1)
  }`
)"#,
                )
                .expect("parser pending pointer capture reparent state should evaluate");
            page_vm
                .dispatch_mouse_event_at_point_with_pointer(
                    10.0,
                    11.0,
                    "mouseup",
                    0,
                    None,
                    0,
                    0.0,
                    0.0,
                    RendererPointerEventProperties::default(),
                    0,
                )
                .expect("parser baseline mouseup should not use reparented pending capture");
            let parser_log = page_vm
                .evaluate_expression("window.parserPointerCaptureReparentLog.join('|')")
                .expect("parser pending pointer capture reparent log should evaluate");

            assert_eq!(
                js_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "js-pointer-reparent:pointerdown|js-pointer-reparent:has-before:true|js-pointer-reparent:has-after-js-reparent:false|js-pointer-reparent:pointerup"
                ),
                "JS reparent to a disconnected parent should clear pending pointer capture immediately"
            );
            assert_eq!(
                parser_log.get("value").and_then(serde_json::Value::as_str),
                Some(
                    "parser-pointer-reparent:pointerdown|parser-pointer-reparent:has-before:true|parser-pointer-reparent:has-after-parser-reparent:false|parser-pointer-reparent:pointerup"
                ),
                "parser reparent to a disconnected parent should clear pending pointer capture before the next pointer event"
            );
        }));
    }

    #[test]
    fn parser_remove_applies_scroll_anchor_adjustment_like_js_remove() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            page_vm.vm_mut().force_fresh_layout_reads_for_test();

            page_vm
                .evaluate_expression(
                    r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-anchor-remove';
jsTarget.style.height = '20px';
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-anchor-remove';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
document.body.removeChild(jsTarget);
window.parserScrollAnchorJsOffset = window.pageYOffset;
window.scrollTo(0, 30);
window.parserScrollAnchorTarget = parserTarget;
"#,
                )
                .expect("scroll-anchor remove setup should evaluate");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-scroll-anchor-remove")
                .expect("parser scroll-anchor target should exist");
            let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: body,
                    child: parser_target,
                },
                "parser scroll-anchor remove mutation should apply",
            );
            if !custom_element_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                    .expect("parser scroll-anchor remove reactions should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsOffset: window.parserScrollAnchorJsOffset,
  parserOffset: window.pageYOffset,
  parserParent: window.parserScrollAnchorTarget.parentNode
})"#,
                )
                .expect("scroll-anchor remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"jsOffset":10,"parserOffset":10,"parserParent":null}"#),
                "parser RemoveChild should apply the same scroll-anchor adjustment as JS removeChild"
            );
        }));
    }

    #[test]
    fn parser_reparent_applies_scroll_anchor_adjustment_like_js_reparent_to_disconnected_parent() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            page_vm.vm_mut().force_fresh_layout_reads_for_test();

            page_vm
                .evaluate_expression(
                    r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-anchor-reparent';
jsTarget.style.height = '20px';
const jsDetachedParent = document.createElement('div');
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-anchor-reparent';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
jsDetachedParent.appendChild(jsTarget);
window.parserScrollAnchorReparentJsState = {
  offset: window.pageYOffset,
  parentIsDetached: jsTarget.parentNode === jsDetachedParent
};
window.scrollTo(0, 30);
window.parserScrollAnchorReparentTarget = parserTarget;
"#,
                )
                .expect("scroll-anchor reparent setup should evaluate");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-scroll-anchor-reparent")
                .expect("parser scroll-anchor reparent target should exist");
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                )
            };
            let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_target,
                },
                "parser scroll-anchor reparent mutation should apply",
            );
            if !custom_element_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                    .expect("parser scroll-anchor reparent reactions should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  js: window.parserScrollAnchorReparentJsState,
  parserOffset: window.pageYOffset
})"#,
                )
                .expect("scroll-anchor reparent result should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_target)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the scroll-anchor target under the native detached parent"
            );
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"js":{"offset":10,"parentIsDetached":true},"parserOffset":10}"#),
                "parser reparent to a disconnected parent should apply the same scroll-anchor adjustment as JS reparent"
            );
        }));
    }

    #[test]
    fn parser_insert_before_reparent_applies_scroll_anchor_adjustment_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            page_vm.vm_mut().force_fresh_layout_reads_for_test();

            page_vm
                .evaluate_expression(
                    r#"
const jsTarget = document.createElement('div');
jsTarget.id = 'js-scroll-anchor-insert-before';
jsTarget.style.height = '20px';
const jsDetachedParent = document.createElement('div');
const jsReference = document.createElement('span');
jsDetachedParent.appendChild(jsReference);
const parserTarget = document.createElement('div');
parserTarget.id = 'parser-scroll-anchor-insert-before';
parserTarget.style.height = '20px';
document.body.style.minHeight = '2000px';
document.body.append(jsTarget, parserTarget);
window.scrollTo(0, 30);
jsDetachedParent.insertBefore(jsTarget, jsReference);
window.parserScrollAnchorInsertBeforeJsState = {
  offset: window.pageYOffset,
  parentIsDetached: jsTarget.parentNode === jsDetachedParent,
  nextIsReference: jsTarget.nextSibling === jsReference
};
window.scrollTo(0, 30);
"#,
                )
                .expect("scroll-anchor insertBefore setup should evaluate");

            let parser_target = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("parser-scroll-anchor-insert-before")
                .expect("parser scroll-anchor insertBefore target should exist");
            let (parser_detached_parent, parser_reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(detached_parent, reference));
                (detached_parent, reference)
            };
            let custom_element_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_detached_parent,
                    child: parser_target,
                    reference_child: Some(parser_reference),
                },
                "parser scroll-anchor insertBefore reparent mutation should apply",
            );
            if !custom_element_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                    .expect("parser scroll-anchor insertBefore reparent reactions should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  js: window.parserScrollAnchorInsertBeforeJsState,
  parserOffset: window.pageYOffset
})"#,
                )
                .expect("scroll-anchor insertBefore result should evaluate");
            let parser_children = page_vm
                .vm()
                .document_runtime
                .dom_host()
                .child_handles(parser_detached_parent)
                .collect::<Vec<_>>();
            assert_eq!(
                parser_children,
                vec![parser_target, parser_reference],
                "parser insertBefore should move the scroll-anchor target before the native reference child"
            );
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"js":{"offset":10,"parentIsDetached":true,"nextIsReference":true},"parserOffset":10}"#
                ),
                "parser InsertBefore reparent to a disconnected parent should apply the same scroll-anchor adjustment as JS insertBefore"
            );
        }));
    }

    #[test]
    fn parser_remove_and_reparent_update_child_list_style_invalidation_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
const style = document.createElement('style');
style.textContent = [
  '.parser-style-parent { color: rgb(1, 2, 3); }',
  '.parser-style-parent:empty { color: rgb(10, 20, 30); }',
  '.parser-style-old > .parser-style-target { color: rgb(4, 5, 6); }',
  '.parser-style-new > .parser-style-target { color: rgb(40, 50, 60); }'
].join('\n');
const head = document.head || document.documentElement.insertBefore(
  document.createElement('head'),
  document.body
);
head.appendChild(style);

function installStyleRemove(prefix) {
  const parent = document.createElement('div');
  parent.setAttribute('id', `${prefix}-parent`);
  parent.setAttribute('class', 'parser-style-parent');
  const target = document.createElement('span');
  target.setAttribute('id', `${prefix}-target`);
  target.setAttribute('class', 'parser-style-target');
  parent.appendChild(target);
  document.body.appendChild(parent);
  return { parent, target };
}

function installStyleReparent(prefix) {
  const oldParent = document.createElement('div');
  oldParent.setAttribute('id', `${prefix}-old-parent`);
  oldParent.setAttribute('class', 'parser-style-parent parser-style-old');
  const newParent = document.createElement('div');
  newParent.setAttribute('id', `${prefix}-new-parent`);
  newParent.setAttribute('class', 'parser-style-parent parser-style-new');
  const target = document.createElement('span');
  target.setAttribute('id', `${prefix}-target`);
  target.setAttribute('class', 'parser-style-target');
  oldParent.appendChild(target);
  document.body.append(oldParent, newParent);
  return { oldParent, newParent, target };
}

window.parserStyleRemoveSummary = pair => [
  getComputedStyle(pair.parent).color,
  pair.target.parentNode && pair.target.parentNode.id
].join('|');
window.parserStyleReparentSummary = pair => [
  getComputedStyle(pair.oldParent).color,
  getComputedStyle(pair.newParent).color,
  getComputedStyle(pair.target).color,
  pair.target.parentNode && pair.target.parentNode.id
].join('|');

window.jsStyleRemove = installStyleRemove('js-style-remove');
window.parserStyleRemove = installStyleRemove('parser-style-remove');
window.jsStyleReparent = installStyleReparent('js-style-reparent');
window.parserStyleReparent = installStyleReparent('parser-style-reparent');
"#,
                )
                .expect("style invalidation parser mutation setup should evaluate");

            let js_remove_result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const before = window.parserStyleRemoveSummary(window.jsStyleRemove);
  window.jsStyleRemove.parent.removeChild(window.jsStyleRemove.target);
  return `${before}=>${window.parserStyleRemoveSummary(window.jsStyleRemove)}`;
})()"#,
                )
                .expect("JS style remove baseline should evaluate");

            let parser_remove_before = page_vm
                .evaluate_expression("window.parserStyleRemoveSummary(window.parserStyleRemove)")
                .expect("parser style remove precondition should evaluate");
            let (parser_remove_parent, parser_remove_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-style-remove-parent")
                        .expect("parser style remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-style-remove-target")
                        .expect("parser style remove target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove_parent,
                    child: parser_remove_child,
                },
                "parser style remove mutation should apply",
            );
            let parser_remove_after = page_vm
                .evaluate_expression("window.parserStyleRemoveSummary(window.parserStyleRemove)")
                .expect("parser style remove result should evaluate");

            let js_reparent_result = page_vm
                .evaluate_expression(
                    r#"(() => {
  const before = window.parserStyleReparentSummary(window.jsStyleReparent);
  window.jsStyleReparent.newParent.appendChild(window.jsStyleReparent.target);
  return `${before}=>${window.parserStyleReparentSummary(window.jsStyleReparent)}`;
})()"#,
                )
                .expect("JS style reparent baseline should evaluate");

            let parser_reparent_before = page_vm
                .evaluate_expression("window.parserStyleReparentSummary(window.parserStyleReparent)")
                .expect("parser style reparent precondition should evaluate");
            let (parser_reparent_new_parent, parser_reparent_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-style-reparent-new-parent")
                        .expect("parser style reparent new parent should exist"),
                    runtime
                        .get_element_by_id("parser-style-reparent-target")
                        .expect("parser style reparent target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_reparent_new_parent,
                    child: parser_reparent_target,
                    reference_child: None,
                },
                "parser style reparent mutation should apply",
            );
            let parser_reparent_after = page_vm
                .evaluate_expression("window.parserStyleReparentSummary(window.parserStyleReparent)")
                .expect("parser style reparent result should evaluate");

            assert_eq!(
                js_remove_result
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|js-style-remove-parent=>rgb(10, 20, 30)|"
                ),
                "JS remove baseline should invalidate :empty style"
            );
            assert_eq!(
                parser_remove_before
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("rgb(1, 2, 3)|parser-style-remove-parent"),
                "parser remove precondition should populate the non-empty style cache"
            );
            assert_eq!(
                parser_remove_after
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("rgb(10, 20, 30)|"),
                "parser remove should invalidate child-list-dependent :empty style"
            );
            assert_eq!(
                js_reparent_result
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|rgb(10, 20, 30)|rgb(4, 5, 6)|js-style-reparent-old-parent=>rgb(10, 20, 30)|rgb(1, 2, 3)|rgb(40, 50, 60)|js-style-reparent-new-parent"
                ),
                "JS reparent baseline should invalidate removed and inserted child-list style"
            );
            assert_eq!(
                parser_reparent_before
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(1, 2, 3)|rgb(10, 20, 30)|rgb(4, 5, 6)|parser-style-reparent-old-parent"
                ),
                "parser reparent precondition should populate old-parent, new-parent, and target style caches"
            );
            assert_eq!(
                parser_reparent_after
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(
                    "rgb(10, 20, 30)|rgb(1, 2, 3)|rgb(40, 50, 60)|parser-style-reparent-new-parent"
                ),
                "parser reparent should invalidate removed and inserted child-list style like JS"
            );
        }));
    }

    #[test]
    fn parser_remove_and_reparent_slotted_nodes_queue_slotchange_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserSlotchangeLog = [];
function installSlottedTarget(prefix, label) {
  const host = document.createElement('div');
  host.id = `${prefix}-host`;
  const shadow = host.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="a"></slot>';
  const slot = shadow.querySelector('slot');
  slot.addEventListener('slotchange', () => {
    window.parserSlotchangeLog.push(`${label}:${slot.assignedNodes().length}`);
  });
  const target = document.createElement('span');
  target.id = `${prefix}-target`;
  target.slot = 'a';
  host.appendChild(target);
  document.body.appendChild(host);
  return { host, target };
}
function installReparentPair(prefix) {
  const oldPair = installSlottedTarget(`${prefix}-old`, 'old');
  const newHost = document.createElement('div');
  newHost.id = `${prefix}-new-host`;
  const shadow = newHost.attachShadow({ mode: 'open' });
  shadow.innerHTML = '<slot name="a"></slot>';
  const newSlot = shadow.querySelector('slot');
  newSlot.addEventListener('slotchange', () => {
    window.parserSlotchangeLog.push(`new:${newSlot.assignedNodes().length}`);
  });
  document.body.appendChild(newHost);
  return { oldHost: oldPair.host, newHost, target: oldPair.target };
}

window.jsSlotRemove = installSlottedTarget('js-slot-remove', 'remove');
window.parserSlotRemove = installSlottedTarget('parser-slot-remove', 'remove');
window.jsSlotReparent = installReparentPair('js-slot-reparent');
window.parserSlotReparent = installReparentPair('parser-slot-reparent');
"#,
                )
                .expect("slotchange parser mutation setup should evaluate");

            page_vm
                .evaluate_expression("undefined")
                .expect("initial slotchange microtasks should drain");
            page_vm
                .evaluate_expression("window.parserSlotchangeLog = []")
                .expect("slotchange log reset should evaluate");

            page_vm
                .evaluate_expression(
                    r#"window.jsSlotRemove.host.removeChild(window.jsSlotRemove.target)"#,
                )
                .expect("JS slot remove baseline should evaluate");
            page_vm
                .evaluate_expression("undefined")
                .expect("JS slot remove slotchange microtask should drain");
            let js_remove_log = page_vm
                .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
                .expect("JS slot remove log should evaluate");

            let (parser_remove_host, parser_remove_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-slot-remove-host")
                        .expect("parser slot remove host should exist"),
                    runtime
                        .get_element_by_id("parser-slot-remove-target")
                        .expect("parser slot remove target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove_host,
                    child: parser_remove_target,
                },
                "parser slot remove mutation should apply",
            );
            page_vm
                .evaluate_expression("undefined")
                .expect("parser slot remove slotchange microtask should drain");
            let parser_remove_log = page_vm
                .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
                .expect("parser slot remove log should evaluate");

            page_vm
                .evaluate_expression(
                    r#"window.jsSlotReparent.newHost.appendChild(window.jsSlotReparent.target)"#,
                )
                .expect("JS slot reparent baseline should evaluate");
            page_vm
                .evaluate_expression("undefined")
                .expect("JS slot reparent slotchange microtask should drain");
            let js_reparent_log = page_vm
                .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
                .expect("JS slot reparent log should evaluate");

            let (parser_reparent_new_host, parser_reparent_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-slot-reparent-new-host")
                        .expect("parser slot reparent new host should exist"),
                    runtime
                        .get_element_by_id("parser-slot-reparent-old-target")
                        .expect("parser slot reparent target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_reparent_new_host,
                    child: parser_reparent_target,
                    reference_child: None,
                },
                "parser slot reparent mutation should apply",
            );
            page_vm
                .evaluate_expression("undefined")
                .expect("parser slot reparent slotchange microtask should drain");
            let parser_reparent_log = page_vm
                .evaluate_expression("JSON.stringify(window.parserSlotchangeLog.splice(0))")
                .expect("parser slot reparent log should evaluate");

            assert_eq!(
                js_remove_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["remove:0"]"#),
                "JS remove baseline should queue one slotchange for removed slotted node"
            );
            assert_eq!(
                parser_remove_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_remove_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser remove should queue the same slotchange signal as JS remove"
            );
            assert_eq!(
                js_reparent_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["old:0","new:1"]"#),
                "JS reparent baseline should queue old then new slotchange signals"
            );
            assert_eq!(
                parser_reparent_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_reparent_log
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser reparent should queue the same slotchange sequence as JS reparent"
            );
        }));
    }

    #[test]
    fn parser_remove_and_reparent_queue_mutation_observer_records_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function parserMoRole(node) {
  if (!node) return '';
  return node.dataset && node.dataset.role ? node.dataset.role : node.nodeName;
}
window.parserMoSummarize = function(records) {
  return records.map(record => [
    parserMoRole(record.target),
    Array.from(record.addedNodes).map(parserMoRole).join(','),
    Array.from(record.removedNodes).map(parserMoRole).join(','),
    parserMoRole(record.previousSibling),
    parserMoRole(record.nextSibling)
  ].join(':')).join('|');
};
function installMoRemove(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-parent`;
  parent.dataset.role = 'remove-parent';
  const child = document.createElement('span');
  child.id = `${prefix}-target`;
  child.dataset.role = 'target';
  parent.appendChild(child);
  document.body.appendChild(parent);
  const observer = new MutationObserver(() => {});
  observer.observe(parent, { childList: true });
  return { parent, child, observer };
}
function installMoReparent(prefix) {
  const oldParent = document.createElement('div');
  oldParent.id = `${prefix}-old-parent`;
  oldParent.dataset.role = 'old-parent';
  const newParent = document.createElement('div');
  newParent.id = `${prefix}-new-parent`;
  newParent.dataset.role = 'new-parent';
  const target = document.createElement('span');
  target.id = `${prefix}-target`;
  target.dataset.role = 'target';
  oldParent.appendChild(target);
  document.body.append(oldParent, newParent);
  const oldObserver = new MutationObserver(() => {});
  const newObserver = new MutationObserver(() => {});
  oldObserver.observe(oldParent, { childList: true });
  newObserver.observe(newParent, { childList: true });
  return { oldParent, newParent, target, oldObserver, newObserver };
}
window.jsMoRemove = installMoRemove('js-mo-remove');
window.parserMoRemove = installMoRemove('parser-mo-remove');
window.jsMoReparent = installMoReparent('js-mo-reparent');
window.parserMoReparent = installMoReparent('parser-mo-reparent');
"#,
                )
                .expect("mutation observer parser mutation setup should evaluate");

            let js_remove_records = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsMoRemove.parent.removeChild(window.jsMoRemove.child);
  return window.parserMoSummarize(window.jsMoRemove.observer.takeRecords());
})()"#,
                )
                .expect("JS mutation observer remove baseline should evaluate");

            let (parser_remove_parent, parser_remove_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-mo-remove-parent")
                        .expect("parser mutation observer remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-mo-remove-target")
                        .expect("parser mutation observer remove target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove_parent,
                    child: parser_remove_child,
                },
                "parser mutation observer remove mutation should apply",
            );
            let parser_remove_records = page_vm
                .evaluate_expression(
                    r#"window.parserMoSummarize(window.parserMoRemove.observer.takeRecords())"#,
                )
                .expect("parser mutation observer remove records should evaluate");

            let js_reparent_records = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsMoReparent.newParent.appendChild(window.jsMoReparent.target);
  return JSON.stringify([
    window.parserMoSummarize(window.jsMoReparent.oldObserver.takeRecords()),
    window.parserMoSummarize(window.jsMoReparent.newObserver.takeRecords())
  ]);
})()"#,
                )
                .expect("JS mutation observer reparent baseline should evaluate");

            let (parser_reparent_parent, parser_reparent_child) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-mo-reparent-new-parent")
                        .expect("parser mutation observer reparent parent should exist"),
                    runtime
                        .get_element_by_id("parser-mo-reparent-target")
                        .expect("parser mutation observer reparent target should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_reparent_parent,
                    child: parser_reparent_child,
                    reference_child: None,
                },
                "parser mutation observer reparent mutation should apply",
            );
            let parser_reparent_records = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserMoSummarize(window.parserMoReparent.oldObserver.takeRecords()),
  window.parserMoSummarize(window.parserMoReparent.newObserver.takeRecords())
])"#,
                )
                .expect("parser mutation observer reparent records should evaluate");

            assert_eq!(
                js_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some("remove-parent::target::"),
                "JS remove baseline should queue one childList removal record"
            );
            assert_eq!(
                parser_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_remove_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser remove should queue the same MutationObserver childList record as JS remove"
            );
            assert_eq!(
                js_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["old-parent::target::","new-parent:target:::"]"#),
                "JS reparent baseline should queue old removal and new insertion records"
            );
            assert_eq!(
                parser_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                js_reparent_records
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                "parser reparent should queue the same MutationObserver childList records as JS reparent"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_insertion_queues_mutation_observer_records_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function parserFragmentMoRole(node) {
  if (!node) return '';
  return node.id || node.nodeName;
}
window.parserFragmentMoSummarize = function(records) {
  return records.map(record => [
    parserFragmentMoRole(record.target),
    Array.from(record.addedNodes).map(parserFragmentMoRole).join(','),
    Array.from(record.removedNodes).map(parserFragmentMoRole).join(','),
    parserFragmentMoRole(record.previousSibling),
    parserFragmentMoRole(record.nextSibling)
  ].join(':')).join('|');
};
function parserFragmentMoParent(id) {
  const parent = document.createElement('div');
  parent.id = id;
  document.body.appendChild(parent);
  return parent;
}
const jsAppendFragment = document.createDocumentFragment();
jsAppendFragment.append(
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-append-a' }),
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-append-b' })
);
const jsAppendParent = parserFragmentMoParent('js-fragment-mo-append-parent');
const jsAppendObserver = new MutationObserver(() => {});
jsAppendObserver.observe(jsAppendParent, { childList: true });
jsAppendParent.appendChild(jsAppendFragment);
window.parserFragmentMoJsAppend = window.parserFragmentMoSummarize(jsAppendObserver.takeRecords());
window.parserFragmentMoJsAppendEmpty = jsAppendFragment.childNodes.length;

const parserAppendParent = parserFragmentMoParent('parser-fragment-mo-append-parent');
window.parserFragmentMoParserAppendObserver = new MutationObserver(() => {});
window.parserFragmentMoParserAppendObserver.observe(parserAppendParent, { childList: true });

const jsBeforeParent = parserFragmentMoParent('js-fragment-mo-before-parent');
const jsBeforeReference = Object.assign(document.createElement('span'), {
  id: 'js-fragment-mo-before-reference'
});
jsBeforeParent.appendChild(jsBeforeReference);
const jsBeforeFragment = document.createDocumentFragment();
jsBeforeFragment.append(
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-before-a' }),
  Object.assign(document.createElement('span'), { id: 'js-fragment-mo-before-b' })
);
const jsBeforeObserver = new MutationObserver(() => {});
jsBeforeObserver.observe(jsBeforeParent, { childList: true });
jsBeforeParent.insertBefore(jsBeforeFragment, jsBeforeReference);
window.parserFragmentMoJsBefore = window.parserFragmentMoSummarize(jsBeforeObserver.takeRecords());
window.parserFragmentMoJsBeforeEmpty = jsBeforeFragment.childNodes.length;

const parserBeforeParent = parserFragmentMoParent('parser-fragment-mo-before-parent');
const parserBeforeReference = Object.assign(document.createElement('span'), {
  id: 'parser-fragment-mo-before-reference'
});
parserBeforeParent.appendChild(parserBeforeReference);
"#,
                )
                .expect("fragment MutationObserver setup should evaluate");

            let (
                parser_append_parent,
                parser_append_fragment,
                parser_before_parent,
                parser_before_fragment,
                parser_before_reference,
            ) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let parser_append_parent = dom_host
                    .element_handle_by_id("parser-fragment-mo-append-parent")
                    .expect("parser fragment MutationObserver append parent should exist");
                let parser_append_fragment = dom_host.create_document_fragment();
                let parser_append_a = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_append_a,
                    "id",
                    "parser-fragment-mo-append-a"
                ));
                let parser_append_b = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_append_b,
                    "id",
                    "parser-fragment-mo-append-b"
                ));
                assert!(dom_host.append_child(parser_append_fragment, parser_append_a));
                assert!(dom_host.append_child(parser_append_fragment, parser_append_b));

                let parser_before_parent = dom_host
                    .element_handle_by_id("parser-fragment-mo-before-parent")
                    .expect("parser fragment MutationObserver before parent should exist");
                let parser_before_fragment = dom_host.create_document_fragment();
                let parser_before_a = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_before_a,
                    "id",
                    "parser-fragment-mo-before-a"
                ));
                let parser_before_b = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    parser_before_b,
                    "id",
                    "parser-fragment-mo-before-b"
                ));
                assert!(dom_host.append_child(parser_before_fragment, parser_before_a));
                assert!(dom_host.append_child(parser_before_fragment, parser_before_b));

                let parser_before_reference = dom_host
                    .element_handle_by_id("parser-fragment-mo-before-reference")
                    .expect("parser fragment MutationObserver reference should exist");
                (
                    parser_append_parent,
                    parser_append_fragment,
                    parser_before_parent,
                    parser_before_fragment,
                    parser_before_reference,
                )
            };

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append_parent,
                    child: parser_append_fragment,
                },
                "parser fragment MutationObserver append should apply",
            );
            assert!(
                append_reaction_roots.is_empty(),
                "plain fragment append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_append_fragment)
                    .count(),
                0,
                "parser fragment append should hoist and empty the fragment"
            );
            let parser_append_records = page_vm
                .evaluate_expression(
                    r#"window.parserFragmentMoSummarize(
  window.parserFragmentMoParserAppendObserver.takeRecords()
)"#,
                )
                .expect("parser fragment MutationObserver append records should evaluate");

            page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentMoParserBeforeObserver = new MutationObserver(() => {});
window.parserFragmentMoParserBeforeObserver.observe(
  document.getElementById('parser-fragment-mo-before-parent'),
  { childList: true }
);
"#,
                )
                .expect("parser fragment MutationObserver before observer should install");

            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before_parent,
                    child: parser_before_fragment,
                    reference_child: Some(parser_before_reference),
                },
                "parser fragment MutationObserver insertBefore should apply",
            );
            assert!(
                before_reaction_roots.is_empty(),
                "plain fragment insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_before_fragment)
                    .count(),
                0,
                "parser fragment insertBefore should hoist and empty the fragment"
            );
            let parser_before_records = page_vm
                .evaluate_expression(
                    r#"window.parserFragmentMoSummarize(
  window.parserFragmentMoParserBeforeObserver.takeRecords()
)"#,
                )
                .expect("parser fragment MutationObserver insertBefore records should evaluate");

            let js_records = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsAppend: window.parserFragmentMoJsAppend,
  jsAppendEmpty: window.parserFragmentMoJsAppendEmpty,
  jsBefore: window.parserFragmentMoJsBefore,
  jsBeforeEmpty: window.parserFragmentMoJsBeforeEmpty
})"#,
                )
                .expect("JS fragment MutationObserver records should evaluate");

            let parser_append = parser_append_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("parser append records should be string");
            let parser_before = parser_before_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("parser insertBefore records should be string");
            let js_records = js_records
                .get("value")
                .and_then(serde_json::Value::as_str)
                .expect("JS fragment records should be string");
            let js_records: serde_json::Value =
                serde_json::from_str(js_records).expect("JS fragment records should parse");
            let normalize = |value: &str| {
                value
                    .replace("js-fragment-mo", "fragment-mo")
                    .replace("parser-fragment-mo", "fragment-mo")
            };
            assert_eq!(
                js_records.get("jsAppendEmpty").and_then(serde_json::Value::as_u64),
                Some(0),
                "JS append baseline should empty the inserted DocumentFragment"
            );
            assert_eq!(
                js_records.get("jsBeforeEmpty").and_then(serde_json::Value::as_u64),
                Some(0),
                "JS insertBefore baseline should empty the inserted DocumentFragment"
            );
            assert_eq!(
                normalize(
                    js_records
                        .get("jsAppend")
                        .and_then(serde_json::Value::as_str)
                        .expect("JS append records should be string")
                ),
                normalize(parser_append),
                "parser DocumentFragment append should queue MutationObserver records like JS appendChild"
            );
            assert_eq!(
                normalize(
                    js_records
                        .get("jsBefore")
                        .and_then(serde_json::Value::as_str)
                        .expect("JS insertBefore records should be string")
                ),
                normalize(parser_before),
                "parser DocumentFragment insertBefore should queue MutationObserver records like JS insertBefore"
            );
        }));
    }

    #[test]
    fn parser_remove_open_popover_dispatches_forced_close_events_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverRemoveEvents = [];
function installOpenPopover(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-parent`;
  const popover = document.createElement('div');
  popover.id = `${prefix}-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverRemoveEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverRemoveEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, popover };
}
window.jsPopoverRemove = installOpenPopover('js');
window.parserPopoverRemove = installOpenPopover('parser');
JSON.stringify([
  window.jsPopoverRemove.popover.matches(':popover-open'),
  window.parserPopoverRemove.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover removal parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover removal targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser popover show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverRemoveEvents = []")
                .expect("popover removal event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverRemove.parent.removeChild(window.jsPopoverRemove.popover);
  return JSON.stringify([
    window.parserPopoverRemoveEvents.splice(0),
    window.jsPopoverRemove.popover.matches(':popover-open')
  ]);
})()"#,
                )
                .expect("JS popover removal baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover removal toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverRemoveEvents.splice(0))")
                .expect("JS popover removal task events should evaluate");

            let (parser_parent, parser_popover) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-parent")
                        .expect("parser popover parent should exist"),
                    runtime
                        .get_element_by_id("parser-popover")
                        .expect("parser popover should exist"),
                )
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_parent,
                    child: parser_popover,
                },
                "parser popover removal mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverRemoveEvents.splice(0),
  window.parserPopoverRemove.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover removal sync events should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover removal toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverRemoveEvents.splice(0))")
                .expect("parser popover removal task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false]"#),
                "JS removal baseline should synchronously dispatch beforetoggle and clear open state"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser removal should synchronously dispatch beforetoggle and clear open state"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS removal baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser removal should queue the same close toggle event"
            );
        }));
    }

    #[test]
    fn parser_reparent_open_popover_to_disconnected_parent_dispatches_forced_close_events_like_js()
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverReparentEvents = [];
function installOpenPopoverReparent(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-reparent-parent`;
  const detachedParent = document.createElement('div');
  detachedParent.id = `${prefix}-reparent-detached-parent`;
  const popover = document.createElement('div');
  popover.id = `${prefix}-reparent-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverReparentEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverReparentEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, detachedParent, popover };
}
window.jsPopoverReparent = installOpenPopoverReparent('js');
window.parserPopoverReparent = installOpenPopoverReparent('parser');
JSON.stringify([
  window.jsPopoverReparent.popover.matches(':popover-open'),
  window.parserPopoverReparent.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover reparent parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover reparent targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser popover reparent show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverReparentEvents = []")
                .expect("popover reparent event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverReparent.detachedParent.appendChild(window.jsPopoverReparent.popover);
  return JSON.stringify([
    window.parserPopoverReparentEvents.splice(0),
    window.jsPopoverReparent.popover.matches(':popover-open'),
    window.jsPopoverReparent.popover.parentNode === window.jsPopoverReparent.detachedParent
  ]);
})()"#,
                )
                .expect("JS popover reparent baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover reparent toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverReparentEvents.splice(0))")
                .expect("JS popover reparent task events should evaluate");

            let parser_popover = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-reparent-popover")
                    .expect("parser popover should exist")
            };
            let parser_detached_parent = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    detached_parent,
                    "id",
                    "parser-reparent-native-detached-parent"
                ));
                detached_parent
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_detached_parent,
                    child: parser_popover,
                },
                "parser popover reparent mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverReparentEvents.splice(0),
  window.parserPopoverReparent.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover reparent sync events should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_popover)
                    .and_then(Node::parent_node),
                Some(parser_detached_parent),
                "parser reparent should move the popover under the native detached parent"
            );
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover reparent toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverReparentEvents.splice(0))")
                .expect("parser popover reparent task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false,true]"#),
                "JS reparent to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser reparent to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS reparent baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser reparent should queue the same close toggle event"
            );
        }));
    }

    #[test]
    fn parser_insert_before_open_popover_reparent_forces_close_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            let setup = page_vm
                .evaluate_expression(
                    r#"
window.parserPopoverBeforeEvents = [];
function installOpenPopoverBefore(prefix) {
  const parent = document.createElement('div');
  parent.id = `${prefix}-before-parent`;
  const detachedParent = document.createElement('div');
  detachedParent.id = `${prefix}-before-detached-parent`;
  const reference = document.createElement('span');
  reference.id = `${prefix}-before-reference`;
  detachedParent.appendChild(reference);
  const popover = document.createElement('div');
  popover.id = `${prefix}-before-popover`;
  popover.popover = 'manual';
  popover.addEventListener('beforetoggle', event => {
    window.parserPopoverBeforeEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  popover.addEventListener('toggle', event => {
    window.parserPopoverBeforeEvents.push(
      `${prefix}:${event.type}:${event.oldState}->${event.newState}:${event.cancelable}:${popover.matches(':popover-open')}`
    );
  });
  parent.appendChild(popover);
  document.body.appendChild(parent);
  popover.showPopover();
  return { parent, detachedParent, reference, popover };
}
window.jsPopoverBefore = installOpenPopoverBefore('js');
window.parserPopoverBefore = installOpenPopoverBefore('parser');
JSON.stringify([
  window.jsPopoverBefore.popover.matches(':popover-open'),
  window.parserPopoverBefore.popover.matches(':popover-open')
])
"#,
                )
                .expect("popover insertBefore parser mutation setup should evaluate");
            assert_eq!(
                setup.get("value").and_then(serde_json::Value::as_str),
                Some("[true,true]"),
                "both JS and parser popover insertBefore targets should start open"
            );

            let loader = page_vm.main_document_resource_loader();
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                2,
                "initial JS/parser insertBefore popover show tasks should run",
            )
            .await;
            page_vm
                .evaluate_expression("window.parserPopoverBeforeEvents = []")
                .expect("popover insertBefore event log reset should evaluate");

            let js_sync = page_vm
                .evaluate_expression(
                    r#"(() => {
  window.jsPopoverBefore.detachedParent.insertBefore(
    window.jsPopoverBefore.popover,
    window.jsPopoverBefore.reference
  );
  return JSON.stringify([
    window.parserPopoverBeforeEvents.splice(0),
    window.jsPopoverBefore.popover.matches(':popover-open'),
    window.jsPopoverBefore.popover.parentNode === window.jsPopoverBefore.detachedParent,
    window.jsPopoverBefore.popover.nextSibling === window.jsPopoverBefore.reference
  ]);
})()"#,
                )
                .expect("JS popover insertBefore baseline should evaluate");
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "JS popover insertBefore toggle task should run",
            )
            .await;
            let js_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverBeforeEvents.splice(0))")
                .expect("JS popover insertBefore task events should evaluate");

            let parser_popover = {
                let runtime = &page_vm.vm().document_runtime;
                runtime
                    .get_element_by_id("parser-before-popover")
                    .expect("parser popover should exist")
            };
            let (parser_detached_parent, parser_reference) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(detached_parent, reference));
                (detached_parent, reference)
            };
            let _ = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_detached_parent,
                    child: parser_popover,
                    reference_child: Some(parser_reference),
                },
                "parser popover insertBefore mutation should apply",
            );
            let parser_sync = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
  window.parserPopoverBeforeEvents.splice(0),
  window.parserPopoverBefore.popover.matches(':popover-open')
])"#,
                )
                .expect("parser popover insertBefore sync events should evaluate");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(parser_detached_parent)
                    .collect::<Vec<_>>(),
                vec![parser_popover, parser_reference],
                "parser insertBefore should move the popover before the native reference child"
            );
            run_element_toggle_tasks_for_test(
                &mut page_vm,
                loader.request_client(),
                1,
                "parser popover insertBefore toggle task should run",
            )
            .await;
            let parser_after_task = page_vm
                .evaluate_expression("JSON.stringify(window.parserPopoverBeforeEvents.splice(0))")
                .expect("parser popover insertBefore task events should evaluate");

            assert_eq!(
                js_sync.get("value").and_then(serde_json::Value::as_str),
                Some(r#"[["js:beforetoggle:open->closed:false:false"],false,true,true]"#),
                "JS insertBefore to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                parser_sync
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"[["parser:beforetoggle:open->closed:false:false"],false]"#),
                "parser insertBefore to a disconnected parent should synchronously force-close the popover"
            );
            assert_eq!(
                js_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["js:toggle:open->closed:false:false"]"#),
                "JS insertBefore baseline should queue a close toggle event"
            );
            assert_eq!(
                parser_after_task
                    .get("value")
                    .and_then(serde_json::Value::as_str),
                Some(r#"["parser:toggle:open->closed:false:false"]"#),
                "parser insertBefore should queue the same close toggle event"
            );
        }));
    }

    #[test]
    fn parser_remove_and_reparent_selected_subtrees_match_js_selection_ranges() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserSelectionStates = {};
function selectionState(targetText) {
  const selection = getSelection();
  const range = selection.rangeCount ? selection.getRangeAt(0) : null;
  const nodeLabel = node => {
    if (node === null) return null;
    if (node === targetText) return 'target-text';
    if (node.nodeType === Node.TEXT_NODE) {
      return `${node.parentNode && node.parentNode.id}#text`;
    }
    return node.id || node.nodeName;
  };
  return {
    rangeCount: selection.rangeCount,
    anchor: nodeLabel(selection.anchorNode),
    anchorOffset: selection.anchorOffset,
    focus: nodeLabel(selection.focusNode),
    focusOffset: selection.focusOffset,
    start: range ? nodeLabel(range.startContainer) : null,
    startOffset: range ? range.startOffset : null,
    end: range ? nodeLabel(range.endContainer) : null,
    endOffset: range ? range.endOffset : null,
    text: selection.toString()
  };
}

const removeParent = document.createElement('div');
removeParent.id = 'parser-selection-remove-parent';
const removeJs = document.createElement('span');
removeJs.id = 'js-selection-remove-target';
removeJs.textContent = 'remove';
const removeParser = document.createElement('span');
removeParser.id = 'parser-selection-remove-target';
removeParser.textContent = 'remove';
removeParent.append(removeJs, removeParser);

const moveParent = document.createElement('div');
moveParent.id = 'parser-selection-move-parent';
const moveDest = document.createElement('div');
moveDest.id = 'parser-selection-move-dest';
const moveJs = document.createElement('span');
moveJs.id = 'js-selection-move-target';
moveJs.textContent = 'move';
const moveParser = document.createElement('span');
moveParser.id = 'parser-selection-move-target';
moveParser.textContent = 'move';
moveParent.append(moveJs, moveParser);
document.body.append(removeParent, moveParent, moveDest);
window.parserSelectionRemoveText = removeParser.firstChild;
window.parserSelectionMoveText = moveParser.firstChild;

const selection = getSelection();
selection.setBaseAndExtent(removeJs.firstChild, 0, removeJs.firstChild, removeJs.firstChild.data.length);
removeParent.removeChild(removeJs);
window.parserSelectionStates.jsRemove = selectionState(removeJs.firstChild);

selection.setBaseAndExtent(moveJs.firstChild, 0, moveJs.firstChild, moveJs.firstChild.data.length);
moveDest.insertBefore(moveJs, null);
window.parserSelectionStates.jsMove = selectionState(moveJs.firstChild);

selection.setBaseAndExtent(removeParser.firstChild, 0, removeParser.firstChild, removeParser.firstChild.data.length);
"#,
                )
                .expect("selection setup should evaluate");

            let (remove_parent, remove_target, move_parent, move_target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-selection-remove-parent")
                        .expect("parser remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-selection-remove-target")
                        .expect("parser remove target should exist"),
                    runtime
                        .get_element_by_id("parser-selection-move-dest")
                        .expect("parser move destination should exist"),
                    runtime
                        .get_element_by_id("parser-selection-move-target")
                        .expect("parser move target should exist"),
                )
            };

            let remove_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: remove_parent,
                        child: remove_target,
                    },
                    "parser remove selected subtree should apply",
                )
            };
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(remove_roots)
                .expect("parser remove followups should dispatch");
            page_vm
                .evaluate_expression(
                    r#"(() => {
window.parserSelectionStates.parserRemove = selectionState(window.parserSelectionRemoveText);
const parserMove = document.getElementById('parser-selection-move-target');
const selection = getSelection();
selection.setBaseAndExtent(parserMove.firstChild, 0, parserMove.firstChild, parserMove.firstChild.data.length);
})()"#,
                )
                .expect("parser remove selection state should evaluate");

            let move_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: move_parent,
                        child: move_target,
                        reference_child: None,
                    },
                    "parser reparent selected subtree should apply",
                )
            };
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(move_roots)
                .expect("parser move followups should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserSelectionStates.parserMove = selectionState(window.parserSelectionMoveText);
JSON.stringify(window.parserSelectionStates)
"#,
                )
                .expect("selection comparison result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsRemove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":6,"start":"parser-selection-remove-parent","startOffset":0,"end":"parser-selection-remove-parent","endOffset":0,"text":""},"jsMove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":4,"start":"parser-selection-move-parent","startOffset":0,"end":"parser-selection-move-parent","endOffset":0,"text":""},"parserRemove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":6,"start":"parser-selection-remove-parent","startOffset":0,"end":"parser-selection-remove-parent","endOffset":0,"text":""},"parserMove":{"rangeCount":1,"anchor":"target-text","anchorOffset":0,"focus":"target-text","focusOffset":4,"start":"parser-selection-move-parent","startOffset":0,"end":"parser-selection-move-parent","endOffset":0,"text":""}}"#
                ),
                "parser remove/reparent should update the selected live range like JS DOM mutation"
            );
        }));
    }

    #[test]
    fn parser_textarea_child_list_mutations_reset_selection_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserTextareaSelectionStates = {};
function textareaState(textarea) {
  return {
    value: textarea.value,
    text: textarea.textContent,
    start: textarea.selectionStart,
    end: textarea.selectionEnd
  };
}
function makeTextarea(id, text, start, end) {
  const textarea = document.createElement('textarea');
  textarea.id = id;
  textarea.textContent = text;
  document.body.appendChild(textarea);
  textarea.setSelectionRange(start, end);
  return textarea;
}

const jsAppend = makeTextarea('js-textarea-append', 'abc', 2, 3);
const parserAppend = makeTextarea('parser-textarea-append', 'abc', 2, 3);
jsAppend.appendChild(document.createTextNode('d'));
window.parserTextareaSelectionStates.jsAppend = textareaState(jsAppend);

const jsBefore = makeTextarea('js-textarea-before', 'bc', 1, 2);
const parserBefore = makeTextarea('parser-textarea-before', 'bc', 1, 2);
jsBefore.insertBefore(document.createTextNode('a'), jsBefore.firstChild);
window.parserTextareaSelectionStates.jsInsertBefore = textareaState(jsBefore);

const jsRemove = makeTextarea('js-textarea-remove', 'abc', 2, 3);
const parserRemove = makeTextarea('parser-textarea-remove', 'abc', 2, 3);
jsRemove.removeChild(jsRemove.firstChild);
window.parserTextareaSelectionStates.jsRemove = textareaState(jsRemove);
"#,
                )
                .expect("textarea selection setup should evaluate");

            let (parser_append, parser_before, parser_remove) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-textarea-append")
                        .expect("parser append textarea should exist"),
                    runtime
                        .get_element_by_id("parser-textarea-before")
                        .expect("parser insertBefore textarea should exist"),
                    runtime
                        .get_element_by_id("parser-textarea-remove")
                        .expect("parser remove textarea should exist"),
                )
            };
            let (parser_before_reference, parser_remove_child) = {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                (
                    dom_host
                        .child_handles(parser_before)
                        .next()
                        .expect("parser insertBefore textarea should have a text child"),
                    dom_host
                        .child_handles(parser_remove)
                        .next()
                        .expect("parser remove textarea should have a text child"),
                )
            };

            let (parser_append_text, parser_before_text) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                (
                    dom_host.create_text_node("d"),
                    dom_host.create_text_node("a"),
                )
            };

            let append_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: parser_append,
                    child: parser_append_text,
                },
                "parser textarea append should apply",
            );
            assert!(
                append_reaction_roots.is_empty(),
                "plain textarea text append should not queue custom element reactions"
            );
            let before_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: parser_before,
                    child: parser_before_text,
                    reference_child: Some(parser_before_reference),
                },
                "parser textarea insertBefore should apply",
            );
            assert!(
                before_reaction_roots.is_empty(),
                "plain textarea text insertBefore should not queue custom element reactions"
            );
            let remove_reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent: parser_remove,
                    child: parser_remove_child,
                },
                "parser textarea remove should apply",
            );
            if !remove_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(remove_reaction_roots)
                    .expect("parser textarea remove followups should dispatch");
            }

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserTextareaSelectionStates.parserAppend =
  textareaState(document.getElementById('parser-textarea-append'));
window.parserTextareaSelectionStates.parserInsertBefore =
  textareaState(document.getElementById('parser-textarea-before'));
window.parserTextareaSelectionStates.parserRemove =
  textareaState(document.getElementById('parser-textarea-remove'));
JSON.stringify({
  jsAppend: window.parserTextareaSelectionStates.jsAppend,
  parserAppend: window.parserTextareaSelectionStates.parserAppend,
  appendSame: JSON.stringify(window.parserTextareaSelectionStates.jsAppend) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserAppend),
  jsInsertBefore: window.parserTextareaSelectionStates.jsInsertBefore,
  parserInsertBefore: window.parserTextareaSelectionStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserTextareaSelectionStates.jsInsertBefore) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserInsertBefore),
  jsRemove: window.parserTextareaSelectionStates.jsRemove,
  parserRemove: window.parserTextareaSelectionStates.parserRemove,
  removeSame: JSON.stringify(window.parserTextareaSelectionStates.jsRemove) ===
    JSON.stringify(window.parserTextareaSelectionStates.parserRemove)
})
"#,
                )
                .expect("textarea selection result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"value":"abcd","text":"abcd","start":0,"end":0},"parserAppend":{"value":"abcd","text":"abcd","start":0,"end":0},"appendSame":true,"jsInsertBefore":{"value":"abc","text":"abc","start":0,"end":0},"parserInsertBefore":{"value":"abc","text":"abc","start":0,"end":0},"insertBeforeSame":true,"jsRemove":{"value":"","text":"","start":0,"end":0},"parserRemove":{"value":"","text":"","start":0,"end":0},"removeSame":true}"#
                ),
                "parser textarea append/insertBefore/remove should reset non-dirty selection like JS child-list mutations"
            );
        }));
    }

    #[test]
    fn parser_reparent_selected_option_preserves_selectedness_like_js() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserOptionSelectedStates = {};
function optionMoveState(select, fallback, chosen) {
  return {
    selectedIndex: select.selectedIndex,
    value: select.value,
    fallbackSelected: fallback.selected,
    chosenSelected: chosen.selected,
    chosenParent: chosen.parentNode && chosen.parentNode.nodeName
  };
}
function optionInsertBeforeState(select, fallback, chosen, reference) {
  const state = optionMoveState(select, fallback, chosen);
  state.chosenNextIsReference = chosen.nextSibling === reference;
  return state;
}

const jsSelect = document.createElement('select');
jsSelect.id = 'js-option-move-select';
const jsFallback = new Option('fallback', 'fallback');
jsFallback.id = 'js-option-move-fallback';
const jsChosen = new Option('chosen', 'chosen');
jsChosen.id = 'js-option-move-chosen';
jsSelect.append(jsFallback, jsChosen);

const parserSelect = document.createElement('select');
parserSelect.id = 'parser-option-move-select';
const parserFallback = new Option('fallback', 'fallback');
parserFallback.id = 'parser-option-move-fallback';
const parserChosen = new Option('chosen', 'chosen');
parserChosen.id = 'parser-option-move-chosen';
parserSelect.append(parserFallback, parserChosen);

const jsBeforeSelect = document.createElement('select');
jsBeforeSelect.id = 'js-option-before-select';
const jsBeforeFallback = new Option('fallback', 'fallback');
jsBeforeFallback.id = 'js-option-before-fallback';
const jsBeforeChosen = new Option('chosen', 'chosen');
jsBeforeChosen.id = 'js-option-before-chosen';
jsBeforeSelect.append(jsBeforeFallback, jsBeforeChosen);
const jsBeforeReference = document.createElement('span');
jsBeforeReference.id = 'js-option-before-reference';

const parserBeforeSelect = document.createElement('select');
parserBeforeSelect.id = 'parser-option-before-select';
const parserBeforeFallback = new Option('fallback', 'fallback');
parserBeforeFallback.id = 'parser-option-before-fallback';
const parserBeforeChosen = new Option('chosen', 'chosen');
parserBeforeChosen.id = 'parser-option-before-chosen';
parserBeforeSelect.append(parserBeforeFallback, parserBeforeChosen);
const parserBeforeReference = document.createElement('span');
parserBeforeReference.id = 'parser-option-before-reference';

document.body.append(
  jsSelect,
  parserSelect,
  jsBeforeSelect,
  jsBeforeReference,
  parserBeforeSelect,
  parserBeforeReference
);
jsSelect.selectedIndex = 1;
parserSelect.selectedIndex = 1;
jsBeforeSelect.selectedIndex = 1;
parserBeforeSelect.selectedIndex = 1;
window.parserOptionMoveSelect = parserSelect;
window.parserOptionMoveFallback = parserFallback;
window.parserOptionMoveChosen = parserChosen;
window.parserOptionBeforeSelect = parserBeforeSelect;
window.parserOptionBeforeFallback = parserBeforeFallback;
window.parserOptionBeforeChosen = parserBeforeChosen;
window.parserOptionBeforeReference = parserBeforeReference;

document.body.appendChild(jsChosen);
document.body.insertBefore(jsBeforeChosen, jsBeforeReference);
window.parserOptionSelectedStates.jsAppend = optionMoveState(jsSelect, jsFallback, jsChosen);
window.parserOptionSelectedStates.jsInsertBefore =
  optionInsertBeforeState(jsBeforeSelect, jsBeforeFallback, jsBeforeChosen, jsBeforeReference);
"#,
                )
                .expect("option selectedness setup should evaluate");

            let (parser_chosen, parser_before_chosen, parser_before_reference) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-option-move-chosen")
                        .expect("parser selected option should exist"),
                    runtime
                        .get_element_by_id("parser-option-before-chosen")
                        .expect("parser insertBefore selected option should exist"),
                    runtime
                        .get_element_by_id("parser-option-before-reference")
                        .expect("parser insertBefore reference should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_chosen,
                    },
                    "parser selected option reparent should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain option reparent should not queue custom element reactions"
            );
            let insert_before_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: parser_before_chosen,
                        reference_child: Some(parser_before_reference),
                    },
                    "parser selected option insertBefore reparent should apply",
                )
            };
            assert!(
                insert_before_reaction_roots.is_empty(),
                "plain option insertBefore reparent should not queue custom element reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserOptionSelectedStates.parserAppend = optionMoveState(
  window.parserOptionMoveSelect,
  window.parserOptionMoveFallback,
  window.parserOptionMoveChosen
);
window.parserOptionSelectedStates.parserInsertBefore = optionInsertBeforeState(
  window.parserOptionBeforeSelect,
  window.parserOptionBeforeFallback,
  window.parserOptionBeforeChosen,
  window.parserOptionBeforeReference
);
JSON.stringify({
  jsAppend: window.parserOptionSelectedStates.jsAppend,
  parserAppend: window.parserOptionSelectedStates.parserAppend,
  appendSame: JSON.stringify(window.parserOptionSelectedStates.jsAppend) ===
    JSON.stringify(window.parserOptionSelectedStates.parserAppend),
  jsInsertBefore: window.parserOptionSelectedStates.jsInsertBefore,
  parserInsertBefore: window.parserOptionSelectedStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserOptionSelectedStates.jsInsertBefore) ===
    JSON.stringify(window.parserOptionSelectedStates.parserInsertBefore)
})
"#,
                )
                .expect("option selectedness comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY"},"parserAppend":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY"},"appendSame":true,"jsInsertBefore":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY","chosenNextIsReference":true},"parserInsertBefore":{"selectedIndex":0,"value":"fallback","fallbackSelected":true,"chosenSelected":true,"chosenParent":"BODY","chosenNextIsReference":true},"insertBeforeSame":true}"#
                ),
                "parser reparent should preserve selected option state like JS appendChild and insertBefore"
            );
        }));
    }

    #[test]
    fn parser_merged_root_attributes_hide_nonce_content_values() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<body nonce="body-secret">
<html nonce="html-secret">
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let html = snapshot
                .document_element_handle()
                .expect("document element");
            let body = snapshot.document_body_handle().expect("body");
            let html = snapshot
                .node(html)
                .and_then(Node::as_element)
                .expect("html element");
            let body = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");

            assert_eq!(html.attribute("nonce"), Some(""));
            assert_eq!(html.cryptographic_nonce(), Some("html-secret"));
            assert_eq!(body.attribute("nonce"), Some(""));
            assert_eq!(body.cryptographic_nonce(), Some("body-secret"));
        }));
    }

    #[test]
    fn parser_inserted_nonce_attribute_is_hidden_like_js_insertion() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function nonceState(id, referenceId) {
  const element = document.getElementById(id);
  return {
    attr: element.getAttribute('nonce'),
    nonce: element.nonce,
    parent: element.parentNode && element.parentNode.nodeName,
    beforeReference: referenceId ? element.nextSibling === document.getElementById(referenceId) : null
  };
}

const jsAppend = document.createElement('script');
jsAppend.id = 'js-nonce-append';
jsAppend.setAttribute('nonce', 'nonce-secret');
document.body.appendChild(jsAppend);

const jsBefore = document.createElement('script');
jsBefore.id = 'js-nonce-before';
jsBefore.setAttribute('nonce', 'nonce-secret');
const jsReference = document.createElement('span');
jsReference.id = 'js-nonce-reference';
document.body.appendChild(jsReference);
document.body.insertBefore(jsBefore, jsReference);

const parserReference = document.createElement('span');
parserReference.id = 'parser-nonce-reference';
document.body.appendChild(parserReference);

window.parserNonceInsertionStates = {
  jsAppend: nonceState('js-nonce-append', null),
  jsInsertBefore: nonceState('js-nonce-before', 'js-nonce-reference')
};
"#,
                )
                .expect("nonce insertion JS baseline should evaluate");

            let (parser_append, parser_before, parser_external, parser_reference) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let parser_reference = runtime
                    .get_element_by_id("parser-nonce-reference")
                    .expect("parser nonce reference should exist");
                let dom_host = runtime.dom_host_mut();
                let parser_append = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_append, "id", "parser-nonce-append"));
                assert!(dom_host.set_attribute(parser_append, "nonce", "nonce-secret"));
                let parser_before = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_before, "id", "parser-nonce-before"));
                assert!(dom_host.set_attribute(parser_before, "nonce", "nonce-secret"));
                let parser_external = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(parser_external, "id", "parser-nonce-external"));
                assert!(dom_host.set_attribute(parser_external, "nonce", "nonce-secret"));
                assert!(dom_host.set_attribute(parser_external, "src", "/parser-nonce.js"));
                (
                    parser_append,
                    parser_before,
                    parser_external,
                    parser_reference,
                )
            };

            let append_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_append,
                    },
                    "parser nonce append should apply",
                )
            };
            assert!(
                append_roots.is_empty(),
                "plain script nonce append should not queue custom element reactions"
            );
            let insert_before_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: parser_before,
                        reference_child: Some(parser_reference),
                    },
                    "parser nonce insertBefore should apply",
                )
            };
            assert!(
                insert_before_roots.is_empty(),
                "plain script nonce insertBefore should not queue custom element reactions"
            );
            let external_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: parser_external,
                    },
                    "parser external nonce append should apply",
                )
            };
            assert!(
                external_roots.is_empty(),
                "plain external script nonce append should not queue custom element reactions"
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .node(parser_external)
                    .and_then(|node| node.as_element())
                    .is_some_and(|element| !element.script_already_started()),
                "parser-created external nonce script should remain startable for parser handoff"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserNonceInsertionStates.parserAppend =
  nonceState('parser-nonce-append', null);
window.parserNonceInsertionStates.parserInsertBefore =
  nonceState('parser-nonce-before', 'parser-nonce-reference');
JSON.stringify({
  jsAppend: window.parserNonceInsertionStates.jsAppend,
  parserAppend: window.parserNonceInsertionStates.parserAppend,
  appendSame: JSON.stringify(window.parserNonceInsertionStates.jsAppend) ===
    JSON.stringify(window.parserNonceInsertionStates.parserAppend),
  jsInsertBefore: window.parserNonceInsertionStates.jsInsertBefore,
  parserInsertBefore: window.parserNonceInsertionStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserNonceInsertionStates.jsInsertBefore) ===
    JSON.stringify(window.parserNonceInsertionStates.parserInsertBefore)
})
"#,
                )
                .expect("nonce insertion comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":null},"parserAppend":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":null},"appendSame":true,"jsInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":true},"parserInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"BODY","beforeReference":true},"insertBeforeSame":true}"#
                ),
                "parser insertion should hide nonce content attributes like JS insertion"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_inserted_nonce_subtree_is_hidden_like_js_insertion() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
function nonceSubtreeState(scriptId, containerId, referenceId) {
  const script = document.getElementById(scriptId);
  const container = document.getElementById(containerId);
  return {
    attr: script.getAttribute('nonce'),
    nonce: script.nonce,
    parent: script.parentNode && script.parentNode.nodeName,
    containerParent: container.parentNode && container.parentNode.nodeName,
    containerBeforeReference: referenceId ? container.nextSibling === document.getElementById(referenceId) : null
  };
}
function nonceFragment(containerId, scriptId) {
  const fragment = document.createDocumentFragment();
  const container = document.createElement('div');
  container.id = containerId;
  const script = document.createElement('script');
  script.id = scriptId;
  script.setAttribute('nonce', 'nonce-secret');
  container.appendChild(script);
  fragment.appendChild(container);
  return fragment;
}

const jsAppendFragment = nonceFragment('js-fragment-nonce-append-container', 'js-fragment-nonce-append');
document.body.appendChild(jsAppendFragment);

const jsBeforeReference = document.createElement('span');
jsBeforeReference.id = 'js-fragment-nonce-reference';
document.body.appendChild(jsBeforeReference);
const jsBeforeFragment = nonceFragment('js-fragment-nonce-before-container', 'js-fragment-nonce-before');
document.body.insertBefore(jsBeforeFragment, jsBeforeReference);

const parserBeforeReference = document.createElement('span');
parserBeforeReference.id = 'parser-fragment-nonce-reference';
document.body.appendChild(parserBeforeReference);

window.parserFragmentNonceStates = {
  jsAppend: nonceSubtreeState(
    'js-fragment-nonce-append',
    'js-fragment-nonce-append-container',
    null
  ),
  jsInsertBefore: nonceSubtreeState(
    'js-fragment-nonce-before',
    'js-fragment-nonce-before-container',
    'js-fragment-nonce-reference'
  )
};
"#,
                )
                .expect("fragment nonce JS baseline should evaluate");

            let (append_fragment, before_fragment, before_reference) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let before_reference = runtime
                    .get_element_by_id("parser-fragment-nonce-reference")
                    .expect("parser fragment nonce reference should exist");
                let dom_host = runtime.dom_host_mut();

                let append_fragment = dom_host.create_document_fragment();
                let append_container = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    append_container,
                    "id",
                    "parser-fragment-nonce-append-container"
                ));
                let append_script = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    append_script,
                    "id",
                    "parser-fragment-nonce-append"
                ));
                assert!(dom_host.set_attribute(append_script, "nonce", "nonce-secret"));
                assert!(dom_host.append_child(append_container, append_script));
                assert!(dom_host.append_child(append_fragment, append_container));

                let before_fragment = dom_host.create_document_fragment();
                let before_container = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    before_container,
                    "id",
                    "parser-fragment-nonce-before-container"
                ));
                let before_script = dom_host.create_parser_element_without_attributes(
                    "script".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(
                    before_script,
                    "id",
                    "parser-fragment-nonce-before"
                ));
                assert!(dom_host.set_attribute(before_script, "nonce", "nonce-secret"));
                assert!(dom_host.append_child(before_container, before_script));
                assert!(dom_host.append_child(before_fragment, before_container));

                (append_fragment, before_fragment, before_reference)
            };

            let append_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent: body,
                    child: append_fragment,
                },
                "parser fragment nonce append should apply",
            );
            assert!(
                append_roots.is_empty(),
                "plain fragment nonce append should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(append_fragment)
                    .count(),
                0,
                "parser fragment nonce append should hoist and empty the fragment"
            );

            let before_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent: body,
                    child: before_fragment,
                    reference_child: Some(before_reference),
                },
                "parser fragment nonce insertBefore should apply",
            );
            assert!(
                before_roots.is_empty(),
                "plain fragment nonce insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(before_fragment)
                    .count(),
                0,
                "parser fragment nonce insertBefore should hoist and empty the fragment"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"
window.parserFragmentNonceStates.parserAppend = nonceSubtreeState(
  'parser-fragment-nonce-append',
  'parser-fragment-nonce-append-container',
  null
);
window.parserFragmentNonceStates.parserInsertBefore = nonceSubtreeState(
  'parser-fragment-nonce-before',
  'parser-fragment-nonce-before-container',
  'parser-fragment-nonce-reference'
);
JSON.stringify({
  jsAppend: window.parserFragmentNonceStates.jsAppend,
  parserAppend: window.parserFragmentNonceStates.parserAppend,
  appendSame: JSON.stringify(window.parserFragmentNonceStates.jsAppend) ===
    JSON.stringify(window.parserFragmentNonceStates.parserAppend),
  jsInsertBefore: window.parserFragmentNonceStates.jsInsertBefore,
  parserInsertBefore: window.parserFragmentNonceStates.parserInsertBefore,
  insertBeforeSame: JSON.stringify(window.parserFragmentNonceStates.jsInsertBefore) ===
    JSON.stringify(window.parserFragmentNonceStates.parserInsertBefore)
})
"#,
                )
                .expect("fragment nonce parser comparison should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsAppend":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":null},"parserAppend":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":null},"appendSame":true,"jsInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":true},"parserInsertBefore":{"attr":"","nonce":"nonce-secret","parent":"DIV","containerParent":"BODY","containerBeforeReference":true},"insertBeforeSame":true}"#
                ),
                "parser DocumentFragment insertion should hide nonce-bearing subtree content attributes like JS insertion"
            );
        }));
    }

    #[test]
    fn parser_reparent_form_associated_custom_element_dispatches_form_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFaceMoveEvents = [];
class ParserMovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceMoveEvents.push(`${this.id}:connected:${this.parentNode && this.parentNode.id}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceMoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceMoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-moved-face-element', ParserMovedFaceElement);
const formA = document.createElement('form');
formA.id = 'parser-face-form-a';
const formB = document.createElement('form');
formB.id = 'parser-face-form-b';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-fieldset';
fieldset.disabled = true;
formB.appendChild(fieldset);
const parserTarget = document.createElement('parser-moved-face-element');
parserTarget.id = 'parser-face-target';
const jsTarget = document.createElement('parser-moved-face-element');
jsTarget.id = 'js-face-target';
window.parserFaceTarget = parserTarget;
window.jsFaceTarget = jsTarget;
formA.append(parserTarget, jsTarget);
document.body.append(formA, formB);
window.parserFaceMoveEvents.length = 0;
fieldset.appendChild(jsTarget);
"#,
                )
                .expect("FACE move setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-face-fieldset")
                        .expect("target parser fieldset should exist"),
                    runtime
                        .get_element_by_id("parser-face-target")
                        .expect("parser FACE target should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser FACE reparent mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving an already-connected FACE should defer form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE reparent reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserFaceMoveEvents,
  parserParent: window.parserFaceTarget.parentNode && window.parserFaceTarget.parentNode.id,
  jsParent: window.jsFaceTarget.parentNode && window.jsFaceTarget.parentNode.id
})"#,
                )
                .expect("FACE move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-face-target:form:parser-face-form-b","js-face-target:disabled:true","parser-face-target:form:parser-face-form-b","parser-face-target:disabled:true"],"parserParent":"parser-face-fieldset","jsParent":"parser-face-fieldset"}"#
                ),
                "parser FACE reparent should match JS insertion form association and disabled callbacks without reconnecting"
            );
        }));
    }

    #[test]
    fn parser_remove_form_associated_custom_element_matches_js_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFaceRemoveEvents = [];
class ParserRemovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceRemoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserFaceRemoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceRemoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceRemoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-removed-face-element', ParserRemovedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-remove-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-remove-fieldset';
fieldset.disabled = true;
const jsTarget = document.createElement('parser-removed-face-element');
jsTarget.id = 'js-face-remove-target';
const parserTarget = document.createElement('parser-removed-face-element');
parserTarget.id = 'parser-face-remove-target';
window.parserFaceRemoveTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
window.parserFaceRemoveEvents.length = 0;
fieldset.removeChild(jsTarget);
window.parserFaceRemoveJsEvents = window.parserFaceRemoveEvents.slice();
window.parserFaceRemoveEvents.length = 0;
"#,
                )
                .expect("FACE remove setup should evaluate");

            let (parent, target) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("parser-face-remove-fieldset")
                        .expect("parser FACE remove parent should exist"),
                    runtime
                        .get_element_by_id("parser-face-remove-target")
                        .expect("parser FACE remove target should exist"),
                )
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent,
                        child: target,
                    },
                    "parser FACE remove mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "removing a connected FACE should defer lifecycle and form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE remove reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFaceRemoveJsEvents,
  parserEvents: window.parserFaceRemoveEvents,
  same: window.parserFaceRemoveJsEvents.map(event => event.replace('js-face-remove-target', 'target')).join('|') ===
    window.parserFaceRemoveEvents.map(event => event.replace('parser-face-remove-target', 'target')).join('|'),
  parserConnected: window.parserFaceRemoveTarget.isConnected,
  parserParent: window.parserFaceRemoveTarget.parentNode && window.parserFaceRemoveTarget.parentNode.id
})"#,
                )
                .expect("FACE remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-remove-target:disconnected:false","js-face-remove-target:form:null","js-face-remove-target:disabled:false"],"parserEvents":["parser-face-remove-target:disconnected:false","parser-face-remove-target:form:null","parser-face-remove-target:disabled:false"],"same":true,"parserConnected":false,"parserParent":null}"#
                ),
                "parser FACE remove should match JS removeChild lifecycle, form association, and disabled callbacks"
            );
        }));
    }

    #[test]
    fn disconnected_remove_form_associated_custom_element_matches_js_disabled_state_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserDisconnectedFaceRemoveEvents = [];
class ParserDisconnectedRemovedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserDisconnectedFaceRemoveEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-disconnected-removed-face-element', ParserDisconnectedRemovedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-disconnected-remove-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-disconnected-remove-fieldset';
fieldset.disabled = true;
const jsTarget = document.createElement('parser-disconnected-removed-face-element');
jsTarget.id = 'js-face-disconnected-remove-target';
const parserTarget = document.createElement('parser-disconnected-removed-face-element');
parserTarget.id = 'parser-face-disconnected-remove-target';
window.parserDisconnectedFaceRemoveFieldset = fieldset;
window.parserDisconnectedFaceRemoveJsTarget = jsTarget;
window.parserDisconnectedFaceRemoveTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
"#,
                )
                .expect("disconnected FACE remove setup should evaluate");

            let (parent, target) = {
                let dom_host = page_vm.vm().document_runtime.dom_host();
                (
                    dom_host
                        .element_handle_by_id("parser-face-disconnected-remove-fieldset")
                        .expect("parser disconnected FACE remove parent should exist"),
                    dom_host
                        .element_handle_by_id("parser-face-disconnected-remove-target")
                        .expect("parser disconnected FACE remove target should exist"),
                )
            };

            page_vm
                .evaluate_expression(
                    r#"
document.body.querySelector('#parser-face-disconnected-remove-form')
  .removeChild(window.parserDisconnectedFaceRemoveFieldset);
window.parserDisconnectedFaceRemoveEvents.length = 0;
window.parserDisconnectedFaceRemoveFieldset
  .removeChild(window.parserDisconnectedFaceRemoveJsTarget);
window.parserDisconnectedFaceRemoveJsEvents =
  window.parserDisconnectedFaceRemoveEvents.slice();
window.parserDisconnectedFaceRemoveEvents.length = 0;
"#,
                )
                .expect("disconnected FACE remove JS baseline should evaluate");

            let reaction_roots = apply_parser_dom_mutation_for_test(
                &mut page_vm,
                ParserDomMutation::RemoveChild {
                    parent,
                    child: target,
                },
                "parser disconnected FACE remove mutation should apply",
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("parser disconnected FACE remove reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserDisconnectedFaceRemoveJsEvents,
  parserEvents: window.parserDisconnectedFaceRemoveEvents,
  same: window.parserDisconnectedFaceRemoveJsEvents.map(event => event.replace('js-face-disconnected-remove-target', 'target')).join('|') ===
    window.parserDisconnectedFaceRemoveEvents.map(event => event.replace('parser-face-disconnected-remove-target', 'target')).join('|'),
  parserConnected: window.parserDisconnectedFaceRemoveTarget.isConnected,
  parserParent: window.parserDisconnectedFaceRemoveTarget.parentNode && window.parserDisconnectedFaceRemoveTarget.parentNode.id
})"#,
                )
                .expect("disconnected FACE remove result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-disconnected-remove-target:disabled:false"],"parserEvents":["parser-face-disconnected-remove-target:disabled:false"],"same":true,"parserConnected":false,"parserParent":null}"#
                ),
                "tree mutation removal from a disabled disconnected fieldset should match JS formDisabledCallback behavior"
            );
        }));
    }

    #[test]
    fn parser_reparent_form_associated_custom_element_to_disconnected_parent_matches_js_reactions()
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserFaceDetachEvents = [];
class ParserDetachedFaceElement extends HTMLElement {
  static formAssociated = true;
  connectedCallback() {
    window.parserFaceDetachEvents.push(`${this.id}:connected:${this.isConnected}`);
  }
  disconnectedCallback() {
    window.parserFaceDetachEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceDetachEvents.push(`${this.id}:form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceDetachEvents.push(`${this.id}:disabled:${disabled}`);
  }
}
customElements.define('parser-detached-face-element', ParserDetachedFaceElement);
const form = document.createElement('form');
form.id = 'parser-face-detach-form';
const fieldset = document.createElement('fieldset');
fieldset.id = 'parser-face-detach-fieldset';
fieldset.disabled = true;
const jsDetachedParent = document.createElement('div');
const jsTarget = document.createElement('parser-detached-face-element');
jsTarget.id = 'js-face-detach-target';
const parserTarget = document.createElement('parser-detached-face-element');
parserTarget.id = 'parser-face-detach-target';
window.parserFaceDetachTarget = parserTarget;
fieldset.append(jsTarget, parserTarget);
form.appendChild(fieldset);
document.body.appendChild(form);
window.parserFaceDetachEvents.length = 0;
jsDetachedParent.insertBefore(jsTarget, null);
window.parserFaceDetachJsEvents = window.parserFaceDetachEvents.slice();
window.parserFaceDetachEvents.length = 0;
"#,
                )
                .expect("FACE detached reparent setup should evaluate");

            let (detached_parent, target) = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let detached_parent = dom_host.create_parser_element_without_attributes(
                    "div".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                let target = dom_host
                    .element_handle_by_id("parser-face-detach-target")
                    .expect("parser FACE detach target should exist before reparent");
                (detached_parent, target)
            };

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: detached_parent,
                        child: target,
                        reference_child: None,
                    },
                    "parser FACE detached reparent mutation should apply",
                )
            };
            assert!(
                !custom_element_reaction_roots.is_empty(),
                "moving a connected FACE to a disconnected parent should defer lifecycle and form state reactions"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(custom_element_reaction_roots)
                .expect("parser FACE detached reparent reactions should dispatch");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  jsEvents: window.parserFaceDetachJsEvents,
  parserEvents: window.parserFaceDetachEvents,
  same: window.parserFaceDetachJsEvents.map(event => event.replace('js-face-detach-target', 'target')).join('|') ===
    window.parserFaceDetachEvents.map(event => event.replace('parser-face-detach-target', 'target')).join('|'),
  parserConnected: window.parserFaceDetachTarget.isConnected,
  parserParentConnected: window.parserFaceDetachTarget.parentNode && window.parserFaceDetachTarget.parentNode.isConnected
})"#,
                )
                .expect("FACE detached reparent result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"jsEvents":["js-face-detach-target:disconnected:false","js-face-detach-target:form:null","js-face-detach-target:disabled:false"],"parserEvents":["parser-face-detach-target:disconnected:false","parser-face-detach-target:form:null","parser-face-detach-target:disabled:false"],"same":true,"parserConnected":false,"parserParentConnected":false}"#
                ),
                "parser FACE reparent to a disconnected parent should match JS insertBefore lifecycle, form association, and disabled callbacks"
            );
        }));
    }

    #[test]
    fn parser_reparent_custom_element_across_documents_dispatches_adoption_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserAdoptEvents = [];
class ParserAdoptedElement extends HTMLElement {
  connectedCallback() {
    window.parserAdoptEvents.push(`${this.id}:connected`);
  }
  disconnectedCallback() {
    window.parserAdoptEvents.push(`${this.id}:disconnected`);
  }
  adoptedCallback(oldDocument, newDocument) {
    window.parserAdoptEvents.push(`${this.id}:adopted:${oldDocument === document}:${newDocument === document}`);
  }
}
customElements.define('parser-adopted-element', ParserAdoptedElement);
const jsAdoptDoc = document.implementation.createHTMLDocument("");
const jsTarget = document.createElement('parser-adopted-element');
jsTarget.id = 'js-adopt-target';
const parserTarget = document.createElement('parser-adopted-element');
parserTarget.id = 'parser-adopt-target';
window.jsAdoptTarget = jsTarget;
window.parserAdoptTarget = parserTarget;
document.body.append(jsTarget, parserTarget);
window.parserAdoptEvents.length = 0;
jsAdoptDoc.documentElement.appendChild(jsTarget);
"#,
                )
                .expect("custom element adoption setup should evaluate");

            let (parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-adopt-target")
                    .expect("parser adoption target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before parser adoption"
                );
                (detached_root, target)
            };

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::InsertBefore {
                    parent,
                    child: target,
                    reference_child: None,
                },
                "parser cross-document reparent mutation should apply",
                "parser adoption reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "cross-document parser reparent should defer adoption reactions"
            );

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  events: window.parserAdoptEvents,
  parserOwnerIsMain: window.parserAdoptTarget.ownerDocument === document,
  jsOwnerIsMain: window.jsAdoptTarget.ownerDocument === document
})"#,
                )
                .expect("custom element adoption result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["js-adopt-target:disconnected","js-adopt-target:adopted:true:false","js-adopt-target:connected","parser-adopt-target:disconnected","parser-adopt-target:adopted:true:false","parser-adopt-target:connected"],"parserOwnerIsMain":false,"jsOwnerIsMain":false}"#
                ),
                "parser cross-document reparent should match JS insertion adoption reactions"
            );
        }));
    }

    #[test]
    fn parser_cross_document_reparent_without_adopted_callback_still_dispatches_lifecycle_reactions()
     {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserNoAdoptCallbackEvents = [];
class ParserNoAdoptCallbackElement extends HTMLElement {
  connectedCallback() {
    window.parserNoAdoptCallbackEvents.push(`${this.id}:connected:${this.ownerDocument === document}`);
  }
  disconnectedCallback() {
    window.parserNoAdoptCallbackEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
}
customElements.define('parser-no-adopt-callback-element', ParserNoAdoptCallbackElement);
const jsDocWithoutAdopt = document.implementation.createHTMLDocument("");
const jsTarget = document.createElement('parser-no-adopt-callback-element');
jsTarget.id = 'js-no-adopt-callback-target';
const parserTarget = document.createElement('parser-no-adopt-callback-element');
parserTarget.id = 'parser-no-adopt-callback-target';
window.jsNoAdoptCallbackTarget = jsTarget;
window.parserNoAdoptCallbackTarget = parserTarget;
document.body.append(jsTarget, parserTarget);
window.parserNoAdoptCallbackEvents.length = 0;
jsDocWithoutAdopt.documentElement.appendChild(jsTarget);
"#,
                )
                .expect("custom element no-adopt cross-document setup should evaluate");

            let (parent, target) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let target = runtime
                    .get_element_by_id("parser-no-adopt-callback-target")
                    .expect("parser no-adopt target should exist");
                let dom_host = runtime.dom_host_mut();
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before parser no-adopt mutation"
                );
                (detached_root, target)
            };

            let had_pending_work = apply_parser_dom_mutation_and_run_post_step_work_for_test(
                &mut page_vm,
                ParserDomMutation::AppendChild {
                    parent,
                    child: target,
                },
                "parser cross-document reparent without adoptedCallback should apply",
                "parser no-adopt lifecycle reactions should dispatch",
            );
            assert!(
                had_pending_work,
                "cross-document parser reparent must defer lifecycle reactions even without adoptedCallback"
            );

            let result = page_vm
                .evaluate_expression("JSON.stringify(window.parserNoAdoptCallbackEvents)")
                .expect("custom element no-adopt result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"["js-no-adopt-callback-target:disconnected:true","js-no-adopt-callback-target:connected:false","parser-no-adopt-callback-target:disconnected:true","parser-no-adopt-callback-target:connected:false"]"#
                ),
                "parser cross-document reparent should match JS lifecycle reactions even when adoptedCallback is absent"
            );
        }));
    }

    #[test]
    fn parser_adoption_reactions_disconnect_only_preconnected_roots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);

            page_vm
                .evaluate_expression(
                    r#"
window.parserMixedAdoptEvents = [];
class ParserMixedAdoptElement extends HTMLElement {
  disconnectedCallback() {
    window.parserMixedAdoptEvents.push(`${this.id}:disconnected:${this.isConnected}`);
  }
  adoptedCallback() {
    window.parserMixedAdoptEvents.push(`${this.id}:adopted`);
  }
}
customElements.define('parser-mixed-adopt-element', ParserMixedAdoptElement);
const connected = document.createElement('parser-mixed-adopt-element');
connected.id = 'parser-mixed-adopt-connected';
const disconnected = document.createElement('parser-mixed-adopt-element');
disconnected.id = 'parser-mixed-adopt-disconnected';
document.body.append(connected, disconnected);
window.parserMixedAdoptConnected = connected;
window.parserMixedAdoptDisconnected = disconnected;
"#,
                )
                .expect("mixed adoption reaction setup should evaluate");

            let (parent, connected, disconnected) = {
                let runtime = &mut page_vm.vm_mut().document_runtime;
                let dom_host = runtime.dom_host_mut();
                let connected = dom_host
                    .element_handle_by_id("parser-mixed-adopt-connected")
                    .expect("connected mixed adoption target should exist");
                let disconnected = dom_host
                    .element_handle_by_id("parser-mixed-adopt-disconnected")
                    .expect("disconnected mixed adoption target should exist");
                let detached_document = dom_host.create_detached_html_document();
                let detached_root = dom_host.create_parser_element_without_attributes_for_document(
                    detached_document,
                    "html".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(
                    dom_host.append_child(detached_document, detached_root),
                    "detached document root should be attached before mixed adoption"
                );
                (detached_root, connected, disconnected)
            };

            page_vm
                .evaluate_expression(
                    r#"
document.body.removeChild(window.parserMixedAdoptDisconnected);
window.parserMixedAdoptEvents.length = 0;
"#,
                )
                .expect("mixed adoption disconnected setup should evaluate");

            let reaction_roots = ParserPostStepRuntimeWorkForTest::merge_for_test([
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent,
                        child: connected,
                    },
                    "connected mixed adoption parser mutation should apply",
                ),
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent,
                        child: disconnected,
                    },
                    "disconnected mixed adoption parser mutation should apply",
                ),
            ]);
            assert!(
                !reaction_roots.is_empty(),
                "mixed cross-document parser reparent should request adoption reaction checkpoint"
            );
            page_vm
                .vm_mut()
                .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(reaction_roots)
                .expect("mixed adoption reactions should dispatch");

            let result = page_vm
                .evaluate_expression("JSON.stringify(window.parserMixedAdoptEvents)")
                .expect("mixed adoption reaction result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"["parser-mixed-adopt-connected:disconnected:true","parser-mixed-adopt-connected:adopted","parser-mixed-adopt-disconnected:adopted"]"#
                ),
                "parser adoption reactions should dispatch disconnectedCallback only for roots that were lifecycle-connected before insertion"
            );
        }));
    }

    #[test]
    fn parser_mutation_owner_queues_inserted_default_text_track_without_getter() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "test setup should start without pending timers"
            );

            let (video, track) = {
                let body = create_connected_html_body_for_test(&mut page_vm);
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let video = dom_host.create_parser_element_without_attributes(
                    "video".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.append_child(body, video));
                let track = dom_host.create_parser_element_without_attributes(
                    "track".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(track, "default", ""));
                assert!(dom_host.set_attribute(track, "src", "captions/en.vtt"));
                (video, track)
            };
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "creating a disconnected parser track should not queue text-track timers"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: video,
                        child: track,
                    },
                    "parser DOM mutation should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain track insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "parser insertion must not represent default-mode work as a timer"
            );
            let task = take_next_dom_manipulation_task_for_test(&page_vm);
            assert!(
                matches!(
                    task,
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(
                        _
                    )
                ),
                "parser-owned default-mode work should share the DOM-manipulation source"
            );
        }));
    }

    #[test]
    fn phase_one_parser_inserted_image_queues_load_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><body>
<img src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">
</body></html>"#;
            let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body handle");
            let image = snapshot
                .child_nodes(body)
                .expect("body children")
                .into_iter()
                .find(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("img"))
                })
                .expect("parser-created image");
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            let pending = context_host
                .borrow()
                .pending_image_load_event(image)
                .expect("parser insertion should queue an image request sequence");
            assert_eq!(
                pending.request_initiator_type(),
                crate::types::SubresourceRequestInitiatorType::Parser,
                "parser insertion should preserve its request initiator through the image owner"
            );
        }));
    }

    #[test]
    fn phase_one_parser_inserted_lazy_media_registers_candidate_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><body>
<video controls loading="lazy" src="data:video/mp4;base64,AAAA"></video>
</body></html>"#;
            let page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body handle");
            let video = snapshot
                .child_nodes(body)
                .expect("body children")
                .into_iter()
                .find(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("video"))
                })
                .expect("parser-created video");
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "parser insertion should register lazy media candidates from the runtime mutation owner"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_insertion_queues_resource_followups_from_hoisted_roots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (fragment, _container, image, video) =
                create_parser_resource_fragment_for_test(&mut page_vm, "parser-fragment-resource");
            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "disconnected fragment setup should not queue image load events"
            );
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "disconnected fragment setup should not register lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "disconnected fragment setup should not queue text-track timers"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::AppendChild {
                        parent: body,
                        child: fragment,
                    },
                    "parser fragment resource insertion should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain resource fragment insertion should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment resource insertion should hoist and empty the fragment"
            );
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser DocumentFragment insertion should queue image load events for hoisted subtree children"
            );
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "parser DocumentFragment insertion should register hoisted lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "parser DocumentFragment insertion must not represent text-track default-mode work as a timer"
            );
            assert!(matches!(
                take_next_dom_manipulation_task_for_test(&page_vm),
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(_)
            ));
            assert!(
                matches!(
                    take_next_dom_manipulation_task_for_test(&page_vm),
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "hoisted text track should follow the earlier image in the shared DOM FIFO"
            );
        }));
    }

    #[test]
    fn parser_document_fragment_insert_before_queues_resource_followups_from_hoisted_roots() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");
            let reference = {
                let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
                let reference = dom_host.create_parser_element_without_attributes(
                    "span".to_owned(),
                    "http://www.w3.org/1999/xhtml".to_owned(),
                    None,
                );
                assert!(dom_host.set_attribute(reference, "id", "parser-fragment-resource-ref"));
                assert!(dom_host.append_child(body, reference));
                reference
            };

            let (fragment, container, image, video) = create_parser_resource_fragment_for_test(
                &mut page_vm,
                "parser-fragment-resource-before",
            );
            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "disconnected fragment setup should not queue image load events"
            );
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "disconnected fragment setup should not register lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "disconnected fragment setup should not queue text-track timers"
            );

            let custom_element_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::InsertBefore {
                        parent: body,
                        child: fragment,
                        reference_child: Some(reference),
                    },
                    "parser fragment resource insertBefore should apply",
                )
            };
            assert!(
                custom_element_reaction_roots.is_empty(),
                "plain resource fragment insertBefore should not queue custom element reactions"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(fragment)
                    .count(),
                0,
                "parser DocumentFragment resource insertBefore should hoist and empty the fragment"
            );
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .dom_host()
                    .child_handles(body)
                    .collect::<Vec<_>>(),
                vec![container, reference],
                "parser DocumentFragment resource insertBefore should place the hoisted root before the reference child"
            );
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser DocumentFragment insertBefore should queue image load events for hoisted subtree children"
            );
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "parser DocumentFragment insertBefore should register hoisted lazy media candidates"
            );
            assert_eq!(
                page_vm.vm().ms_to_next_timeout(),
                None,
                "parser DocumentFragment insertBefore must not represent text-track default-mode work as a timer"
            );
            assert!(matches!(
                take_next_dom_manipulation_task_for_test(&page_vm),
                crate::page_task_queue::RendererPageDomManipulationTask::ImageLoadEvent(_)
            ));
            assert!(
                matches!(
                    take_next_dom_manipulation_task_for_test(&page_vm),
                    crate::page_task_queue::RendererPageDomManipulationTask::TextTrackDefaultMode(_)
                ),
                "insertBefore-hoisted text track should follow the image in the shared DOM FIFO"
            );
        }));
    }

    #[test]
    fn js_insert_before_inserted_lazy_media_registers_candidate_from_mutation_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            page_vm
                .evaluate_expression(
                    r#"
const anchor = document.createElement('div');
anchor.id = 'js-lazy-media-anchor';
const video = document.createElement('video');
video.id = 'js-lazy-media-video';
video.setAttribute('controls', '');
video.setAttribute('loading', 'lazy');
video.setAttribute('src', 'data:video/mp4;base64,AAAA');
window.jsLazyMediaAnchor = anchor;
window.jsLazyMediaVideo = video;
document.body.append(anchor, video);
"#,
                )
                .expect("lazy media insertBefore setup should evaluate");

            let video = page_vm
                .vm()
                .document_runtime
                .get_element_by_id("js-lazy-media-video")
                .expect("connected lazy video should exist");
            context_host
                .borrow_mut()
                .remove_lazy_media_load_candidate(video);
            assert!(
                !context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "test setup should clear candidate registered by src attribute or initial append"
            );

            page_vm
                .evaluate_expression(
                    r#"
document.body.removeChild(window.jsLazyMediaVideo);
document.body.insertBefore(window.jsLazyMediaVideo, window.jsLazyMediaAnchor);
"#,
                )
                .expect("lazy media insertBefore mutation should evaluate");

            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&video),
                "JS insertBefore should register inserted lazy media candidates from the runtime mutation owner"
            );
        }));
    }

    #[test]
    fn lazy_media_stale_candidates_are_cleared_after_js_and_parser_remove() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = new_phase_one_page_vm_for_test();
            let body = create_connected_html_body_for_test(&mut page_vm);
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            page_vm
                .evaluate_expression(
                    r#"
function makeLazyVideo(id) {
  const video = document.createElement('video');
  video.id = id;
  video.setAttribute('controls', '');
  video.setAttribute('loading', 'lazy');
  video.setAttribute('src', 'data:video/mp4;base64,AAAA');
  return video;
}
window.jsRemovedLazyMedia = makeLazyVideo('js-removed-lazy-media');
window.parserRemovedLazyMedia = makeLazyVideo('parser-removed-lazy-media');
document.body.append(window.jsRemovedLazyMedia, window.parserRemovedLazyMedia);
"#,
                )
                .expect("lazy media stale cleanup setup should evaluate");

            let (js_video, parser_video) = {
                let runtime = &page_vm.vm().document_runtime;
                (
                    runtime
                        .get_element_by_id("js-removed-lazy-media")
                        .expect("JS removed lazy video should exist"),
                    runtime
                        .get_element_by_id("parser-removed-lazy-media")
                        .expect("parser removed lazy video should exist"),
                )
            };
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&js_video),
                "JS-created lazy media should be registered before removal"
            );
            assert!(
                context_host
                    .borrow()
                    .lazy_media_load_candidates()
                    .contains(&parser_video),
                "parser-remove target lazy media should be registered before removal"
            );

            page_vm
                .evaluate_expression("document.body.removeChild(window.jsRemovedLazyMedia);")
                .expect("JS lazy media removal should evaluate");

            let parser_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: body,
                        child: parser_video,
                    },
                    "parser lazy media removal should apply",
                )
            };
            if !parser_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                        parser_reaction_roots,
                    )
                    .expect("parser lazy media removal reactions should dispatch");
            }

            page_vm
                .evaluate_expression("window.scrollTo(0, 1);")
                .expect("lazy media reveal scan should evaluate");

            let candidates = context_host.borrow().lazy_media_load_candidates();
            assert!(
                !candidates.contains(&js_video),
                "lazy media reveal scan should clear stale JS-removed candidates"
            );
            assert!(
                !candidates.contains(&parser_video),
                "lazy media reveal scan should clear stale parser-removed candidates"
            );
        }));
    }

    #[test]
    fn parser_removed_image_pending_load_event_clears_when_dom_task_runs() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let html = r#"<!doctype html><html><body>
<img id="parser-removed-pending-image" src="data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7">
</body></html>"#;
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(html).await;
            let context_host = page_vm
                .vm()
                .context_host_weak_for_test()
                .upgrade()
                .expect("context host should be alive");

            let (body, image) = {
                let snapshot = page_vm.vm().snapshot_live_document();
                let runtime = &page_vm.vm().document_runtime;
                (
                    snapshot
                        .document_body_handle()
                        .expect("parser-created body should exist"),
                    runtime
                        .get_element_by_id("parser-removed-pending-image")
                        .expect("parser-created pending image should exist"),
                )
            };
            assert!(
                context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "parser insertion should leave an image load event pending before its DOM turn"
            );

            let parser_reaction_roots = {
                apply_parser_dom_mutation_for_test(
                    &mut page_vm,
                    ParserDomMutation::RemoveChild {
                        parent: body,
                        child: image,
                    },
                    "parser image removal should apply",
                )
            };
            if !parser_reaction_roots.is_empty() {
                page_vm
                    .vm_mut()
                    .queue_and_run_pending_parser_post_step_runtime_work_in_default_context_for_test(
                        parser_reaction_roots,
                    )
                    .expect("parser image removal reactions should dispatch");
            }

            let loader = page_vm.main_document_resource_loader();
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        crate::runtime::page_vm::PageSelectedTaskTestSelector::DomManipulation(
                            crate::runtime::page_vm::PageDomManipulationTestFamily::ImageLoadEvent,
                        ),
                        loader.request_client(),
                    )
                    .await
                    .expect("parser image DOM-manipulation task should run")
            );

            assert!(
                !context_host
                    .borrow()
                    .has_pending_image_load_event_for_test(image),
                "queued image load callback should clear pending state after parser removal"
            );
        }));
    }

    #[test]
    fn parser_blocking_script_sees_parser_created_style_sources() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let _js_runtime = crate::JsRuntime::initialize();
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><style>div { color: red } .foo { color: lime }</style><body>
<div id="target"></div>
<script>
const style = getComputedStyle(target);
const before = style.color;
target.classList.add('foo');
document.body.setAttribute('data-result', `${before}|${style.color}`);
</script>
</body>"#;
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(101),
                local_executor.clone(),
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser-created style sync script handoff channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("script handoff should complete");
            assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(result, Some("rgb(255, 0, 0)|rgb(0, 255, 0)"));
        }));
    }

    #[test]
    fn parser_defined_autonomous_custom_element_reaches_parser_handoff() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let _js_runtime = crate::JsRuntime::initialize();
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                r#"<!doctype html><script>customElements.define("x-sync", class extends HTMLElement {});</script>"#,
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline customElements.define handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(103),
                local_executor.clone(),
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element definition handoff channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("customElements.define handoff should complete");
            assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

            let parser_document_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("custom-element parser test requires a main document owner");
            let result = driver.pump_parse_step_with_signals(
                &mut page_vm,
                parser_document_owner,
                "<body><x-sync id='candidate' data-probe='yes'></x-sync></body>",
            );
            let crate::live_document_parser::LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(
                handoff,
            ) = result
            else {
                panic!("expected parser-created custom element handoff after define()");
            };
            let handoff = &*handoff;
            assert_eq!(handoff.local_name, "x-sync");
            assert_eq!(handoff.namespace, "http://www.w3.org/1999/xhtml");
            assert_eq!(handoff.prefix, None);
            assert_eq!(handoff.parent_at_creation, None);
            assert_eq!(handoff.owner_document.index(), 0);
            assert!(
                handoff
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name() == "id" && attribute.value() == "candidate")
            );
            assert!(handoff.attributes.iter().any(|attribute| {
                attribute.name() == "data-probe" && attribute.value() == "yes"
            }));
        }));
    }

    #[test]
    fn parser_custom_element_constructor_runs_before_following_siblings() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url =
                Url::parse("https://parser-sync-custom-element.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let html = r#"<!doctype html><script>
window.__containerChildNodesInConstructor = [];
window.__containerNextSiblingInConstructor = "unset";
window.__attributeCountInConstructor = -1;
class MyCustomElement extends HTMLElement {
  constructor() {
    super();
    window.__attributeCountInConstructor = this.attributes.length;
    const container = document.getElementById('custom-element-container');
    for (let i = 0; i < container.childNodes.length; i++)
      window.__containerChildNodesInConstructor.push(container.childNodes[i]);
    window.__containerNextSiblingInConstructor = container.nextSibling;
  }
}
customElements.define('my-custom-element', MyCustomElement);
</script><div id="custom-element-container">
    <span id="custom-element-previous-element"></span>
    <my-custom-element id="candidate"></my-custom-element>
    <div id="custom-element-next-element"></div>
</div><script>
const instance = document.querySelector('my-custom-element');
document.body.setAttribute('data-result', [
  window.__containerChildNodesInConstructor.length,
  window.__containerChildNodesInConstructor[0] === instance.parentNode.firstChild,
  window.__containerChildNodesInConstructor[1] === document.getElementById('custom-element-previous-element'),
  window.__containerChildNodesInConstructor[2] === instance.previousSibling,
  window.__containerNextSiblingInConstructor === null,
  window.__attributeCountInConstructor,
  instance.getAttribute('id')
].join('|'));
</script>"#;
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at initial inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(104),
                local_executor.clone(),
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor.clone(),
                "phase-one parser custom element sync setup handoff channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("initial customElements.define handoff should complete");
            assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element continuation channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser continuation should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(result, Some("3|true|true|true|true|0|candidate"));
        }));
    }

    #[test]
    fn parser_custom_element_inserts_constructor_returned_element() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url =
                Url::parse("https://parser-returned-custom-element.test/").expect("test url");
            let loader: &'static ResourceRequestClient =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };
            let html = r#"<!doctype html><script>
let anotherElementCreatedBeforeSuperCall = undefined;
let elementCreatedBySuperCall = undefined;
let shouldCreateElementBeforeSuperCall = true;
class InstantiatesItselfBeforeSuper extends HTMLElement {
  constructor() {
    if (shouldCreateElementBeforeSuperCall) {
      shouldCreateElementBeforeSuperCall = false;
      anotherElementCreatedBeforeSuperCall = new InstantiatesItselfBeforeSuper();
    }
    super();
    elementCreatedBySuperCall = this;
  }
}
customElements.define('instantiates-itself-before-super', InstantiatesItselfBeforeSuper);

let shouldCreateAnotherInstance = true;
let anotherInstance = undefined;
let firstInstance = undefined;
class ReturnsAnotherInstance extends HTMLElement {
  constructor() {
    super();
    if (shouldCreateAnotherInstance) {
      shouldCreateAnotherInstance = false;
      firstInstance = this;
      anotherInstance = new ReturnsAnotherInstance();
      return anotherInstance;
    }
    return this;
  }
}
customElements.define('returns-another-instance', ReturnsAnotherInstance);
</script>
<instantiates-itself-before-super id="a"><span id="child-a"></span></instantiates-itself-before-super>
<returns-another-instance id="b"></returns-another-instance>
<script>
const instanceA = document.querySelector('instantiates-itself-before-super');
const instanceB = document.querySelector('returns-another-instance');
document.body.setAttribute('data-result', [
  instanceA instanceof InstantiatesItselfBeforeSuper,
  instanceA === elementCreatedBySuperCall,
  instanceA !== anotherElementCreatedBeforeSuperCall,
  anotherElementCreatedBeforeSuperCall.parentNode === null,
  instanceA.getAttribute('id'),
  instanceA.firstElementChild.id,
  instanceB instanceof ReturnsAnotherInstance,
  instanceB === anotherInstance,
  instanceB !== firstInstance,
  firstInstance.parentNode === null,
  instanceB.getAttribute('id')
].join('|'));
</script>"#;
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at initial inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(105),
                local_executor.clone(),
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor.clone(),
                "phase-one parser custom element return setup handoff channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("initial customElements.define handoff should complete");
            assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element return continuation channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser continuation should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("true|true|true|true|a|child-a|true|true|true|true|b")
            );
        }));
    }

    #[test]
    fn phase_transition_syncs_parser_created_style_sources_for_later_reads() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let state = ParseTimeDriverState::new(final_url.clone());
            let _js_runtime = crate::JsRuntime::initialize();
            let html =
                r#"<!doctype html><style>div { color: red } .foo { color: lime }</style><body><div id="target"></div></body>"#;
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = state.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            assert!(
                matches!(result, ParserPumpStep::InputDrained),
                "non-script parser input should drain"
            );
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let page_vm = PageVm::new(
                PageId::new_for_testing(102),
                local_executor.clone(),
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                loader.clone(),
                crate::renderer::PageVmInitStage::Load,
                state,
                page_vm,
            );
            let (mut page_vm, _, _, _) = super::scaffold::run_phase_one_local_task(
                &local_executor,
                "phase-one parser-created final style sync handoff",
                async move {
                    runtime
                        .into_phase_two_execution(
                            Instant::now(),
                            super::loop_protocol::ParseTimePhaseTransitionReason::ParserCompleted,
                        )
                        .await
                },
            )
            .await
            .expect("phase transition should complete");

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify([
                        getComputedStyle(target).color,
                        (target.classList.add('foo'), getComputedStyle(target).color)
                    ])"#,
                )
                .expect("style read should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"["rgb(255, 0, 0)","rgb(0, 255, 0)"]"#)
            );
        }));
    }

    #[test]
    fn parser_connected_head_script_does_not_push_later_head_tokens_into_body() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = "<!doctype html><html><head><script>const body = document.createElement('body');document.documentElement.appendChild(body);const early = document.createElement('div');early.id = 'early';body.appendChild(early);</script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>";
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one inline script handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("inline script handoff should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "inline parser-connected script should not navigate in this fixture"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one inline script continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the inline script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive parser-connected script execution: {serialized}"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
    }

    #[test]
    fn parser_connected_external_head_script_with_live_head_and_body_mutation_keeps_later_head_tokens_in_head()
     {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let external_script = "globalThis.__runtimeHeadMutationStage='start';const body = document.createElement('body');globalThis.__runtimeHeadMutationStage='body-created';document.documentElement.appendChild(body);globalThis.__runtimeHeadMutationStage='body-appended';const early = document.createElement('div');early.id = 'early';body.appendChild(early);const style = document.createElement('style');style.textContent = '.runtime-style { color: red; }';document.head.appendChild(style);const script = document.createElement('script');script.textContent = 'window.__runtimeHeadMutation = true;';document.head.appendChild(script);globalThis.__runtimeHeadMutationStage='complete';";
            let encoded_script = url::form_urlencoded::byte_serialize(external_script.as_bytes())
                .collect::<String>()
                .replace('+', "%20");
            let html = format!(
                "<!doctype html><html><head><script src=\"data:text/javascript,{encoded_script}\"></script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>"
            );
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(&html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at external script handoff");
            };
            let ParserScriptHandoff::BlockingClassic { script, .. } = handoff.as_ref() else {
                panic!("expected external parser-blocking classic handoff");
            };
            driver.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(script).expect("preload key"),
                ready_preload_entry_for_script(script, external_script),
            );

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one external script handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("external parser-connected script should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "external parser-connected script should not navigate in this fixture"
            );
            let before_resume = page_vm.vm().snapshot_live_document();
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__runtimeHeadMutationStage)")
                    .expect("runtime head mutation stage should evaluate"),
                "complete",
                "external parser-blocking script should complete before parser resume"
            );
            let before_resume_head = before_resume
                .document_head_handle()
                .expect("head should exist before parser resume");
            let before_resume_body = before_resume
                .document_body_handle()
                .expect("runtime body should exist before parser resume");
            assert!(
                before_resume
                    .child_ids(before_resume_head)
                    .any(|handle| before_resume
                        .node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))),
                "runtime style should exist before parser resume: {}",
                before_resume.serialize_document()
            );
            assert!(
                before_resume
                    .child_ids(before_resume_body)
                    .any(|handle| before_resume
                        .node(handle)
                        .and_then(Node::as_element)
                        .and_then(Element::id)
                        .is_some_and(|id| id == "early")),
                "runtime body node should exist before parser resume: {}",
                before_resume.serialize_document()
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.__runtimeHeadMutation === true)")
                    .expect("runtime head script marker should evaluate"),
                "true",
                "runtime-inserted script should execute before parser resume"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one external script continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the external script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive external parser-connected script execution: {serialized}"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))
                }),
                "runtime-inserted <style> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .and_then(Element::id)
                        .is_some_and(|id| id == "early")
                }),
                "runtime-inserted body node should stay under <body>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
    }

    #[test]
    fn parser_connected_head_document_write_keeps_later_head_tokens_in_head() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = Box::leak(Box::new(
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader"),
            ));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url.clone())));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = "<!doctype html><html><head><script>document.write('<style>.runtime-style{color:red}</style>');document.write('<script>window.__docWriteHeadMutation=true;<\\/script>');</script><meta charset='utf-8'><title>x</title></head><body><main>late</main></body></html>";
            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(html);
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline script handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("inline document.write handoff should execute");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "inline document.write fixture should not navigate"
            );

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write continuation local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, "", None).await
                },
            )
            .await
            .expect("parser should continue after the inline script");
            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::Continue),
                "parser should finish the remaining buffered html after the script"
            );

            let snapshot = page_vm.vm().snapshot_live_document();
            let serialized = snapshot.serialize_document();
            assert!(
                serialized.to_ascii_lowercase().contains("<!doctype html>"),
                "doctype should survive parser-connected document.write execution: {serialized}"
            );

            let inserted_script = snapshot
                .script_handles()
                .into_iter()
                .find(|handle| {
                    snapshot
                        .direct_text_content(*handle)
                        .is_some_and(|source| {
                            source
                                .trim_start()
                                .starts_with("window.__docWriteHeadMutation")
                        })
                })
                .expect("document.write-inserted script should remain in the live document");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .parser_script_start_position(inserted_script),
                Some(crate::document_runtime::ParserScriptStartPosition {
                    line: 0,
                    column: 0,
                }),
                "document.write-generated script source positions are intentionally unknown"
            );

            let head = snapshot.document_head_handle().expect("head should exist");
            let body = snapshot.document_body_handle().expect("body should exist");
            let head_children = snapshot.child_ids(head).collect::<Vec<_>>();
            let body_children = snapshot.child_ids(body).collect::<Vec<_>>();

            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("meta"))
                }),
                "later <meta> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("title"))
                }),
                "later <title> should stay under <head>: {serialized}"
            );
            assert!(
                head_children.iter().any(|handle| {
                    snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.is_html_element("style"))
                }),
                "document.write-inserted <style> should stay under <head>: {serialized}"
            );
            assert!(
                body_children.iter().all(|handle| {
                    !snapshot
                        .node(*handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| {
                            element.is_html_element("meta") || element.is_html_element("title")
                        })
                }),
                "<body> should not receive later head-only tokens: {serialized}"
            );
        }));
    }

    #[test]
    fn empty_parser_blocking_script_drains_observers_before_declarative_shadow() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
let gotHost = false;
new MutationObserver((records) => {
  for (const record of records) {
    for (const node of record.addedNodes) {
      if (node.id === 'host') {
        gotHost = true;
        node.attachShadow({ mode: 'closed' });
      }
    }
  }
}).observe(document.body, { childList: true, subtree: true });
</script>
<div id="host"><script></script><template shadowrootmode="open"><span>Content</span></template></div>
<script>
const host = document.querySelector('#host');
document.body.setAttribute('data-result', [
  gotHost,
  !!host.querySelector('template'),
  !host.shadowRoot
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one empty script parser step local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("true|true|true"),
                "empty parser-blocking scripts still create a microtask checkpoint before later declarative shadow parsing"
            );
            let empty_script_remained_startable = snapshot
                .script_handles()
                .into_iter()
                .filter(|handle| {
                    snapshot
                        .direct_text_content(*handle)
                        .is_some_and(|text| text.is_empty())
                })
                .any(|handle| {
                    snapshot
                        .node(handle)
                        .and_then(Node::as_element)
                        .is_some_and(|element| !element.script_already_started())
                });
            assert!(
                empty_script_remained_startable,
                "empty parser-blocking scripts must not commit already-started; later text insertion can still start them"
            );
        }));
    }

    #[test]
    fn move_before_does_not_prepare_empty_parser_scripts_from_moved_text() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let mut page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script id="html-move"></script>
<svg><script id="svg-move"></script></svg>
<script id="html-append"></script>
<svg><script id="svg-append"></script></svg>
<script>
globalThis.__htmlMoveRan = false;
globalThis.__svgMoveRan = false;
globalThis.__htmlAppendRan = false;
globalThis.__svgAppendRan = false;

for (const [id, flag] of [
  ['html-move', '__htmlMoveRan'],
  ['svg-move', '__svgMoveRan'],
]) {
  const text = document.createTextNode(`globalThis.${flag} = true;`);
  document.body.appendChild(text);
  document.getElementById(id).moveBefore(text, null);
}
for (const [id, flag] of [
  ['html-append', '__htmlAppendRan'],
  ['svg-append', '__svgAppendRan'],
]) {
  document.getElementById(id).appendChild(
    document.createTextNode(`globalThis.${flag} = true;`)
  );
}
</script>
</body></html>"#,
            )
            .await;

            let result = page_vm
                .evaluate_expression(
                    r#"JSON.stringify({
  moved: [globalThis.__htmlMoveRan, globalThis.__svgMoveRan],
  appended: [globalThis.__htmlAppendRan, globalThis.__svgAppendRan],
})"#,
                )
                .expect("atomic script move result should evaluate");
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"moved":[false,false],"appended":[true,true]}"#),
                "moveBefore must suppress script preparation without changing ordinary child insertion"
            );
        }));
    }

    #[test]
    fn empty_script_inside_template_does_not_crash_observer_delivery() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
new MutationObserver(() => {}).observe(document.body, { childList: true });
</script>
<div id="host"><template><span>Content</span><script></script></template></div>
<script>
document.body.setAttribute('data-result', 'done');
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one template script parser step local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("done"),
                "template-contained scripts are inert and must not crash observer delivery before the following parser script"
            );
        }));
    }

    #[test]
    fn declarative_shadow_respects_custom_element_disabled_shadow_feature() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
class ShadowDisabledElement extends HTMLElement {
  static get disabledFeatures() { return ['shadow']; }
}
customElements.define('shadow-disabled', ShadowDisabledElement);
</script>
<shadow-disabled><template shadowrootmode="open"><span>Content</span></template></shadow-disabled>
<script>
const element = document.querySelector('shadow-disabled');
document.body.setAttribute('data-result', [
  element instanceof ShadowDisabledElement,
  !!element.querySelector('template'),
  !element.shadowRoot
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one disabled shadow parser step local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let result = snapshot
                .node(body)
                .and_then(Node::as_element)
                .and_then(|element| element.attribute("data-result"));
            assert_eq!(
                result,
                Some("true|true|true"),
                "custom elements with disabledFeatures containing shadow should reject parser-created declarative shadow roots"
            );
        }));
    }

    #[test]
    fn parser_created_custom_element_direct_constructs_before_token_attributes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.ceEvents = [];
window.WptTokenTiming = class extends HTMLElement {
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.ceEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after'),
      this.isConnected,
      writeResult
    ].join('|'));
  }
  connectedCallback() {
    window.ceEvents.push([
      'connected',
      this.getAttribute('data-token'),
      !!document.getElementById('after'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-token-timing', window.WptTokenTiming);
</script>
<wpt-token-timing data-token="owned"></wpt-token-timing><span id="after"></span>
<script>
const element = document.querySelector('wpt-token-timing');
document.body.setAttribute('data-first-event', window.ceEvents[0] || '');
document.body.setAttribute('data-second-event', window.ceEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after')));
document.body.setAttribute('data-instance', String(element instanceof window.WptTokenTiming));
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false|InvalidStateError"),
                "constructor must run before parser token attributes and following siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|false|true"),
                "connectedCallback must run after parser insertion but before following parser tokens are appended"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
    }

    #[test]
    fn parser_created_custom_element_uses_constructor_returned_element() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
let shouldCreateAnotherInstance = true;
let anotherInstance = undefined;
let firstInstance = undefined;
class ReturnsAnotherInstance extends HTMLElement {
  constructor() {
    super();
    if (shouldCreateAnotherInstance) {
      shouldCreateAnotherInstance = false;
      firstInstance = this;
      anotherInstance = new ReturnsAnotherInstance();
      return anotherInstance;
    }
  }
}
customElements.define('returns-another-instance', ReturnsAnotherInstance);
</script>
<returns-another-instance></returns-another-instance>
<script>
const instance = document.querySelector('returns-another-instance');
document.body.setAttribute('data-parser-returned', [
  instance instanceof ReturnsAnotherInstance,
  instance === anotherInstance,
  instance !== firstInstance,
  firstInstance.parentNode === null,
  anotherInstance.parentNode === document.body
].join('|'));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-parser-returned"),
                Some("true|true|true|true|true"),
                "parser synchronous construction must insert the element returned by the constructor"
            );
        }));
    }

    #[test]
    fn parser_created_custom_element_token_attributes_queue_initial_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.ceEvents = [];
window.ceCallbackState = {};
window.WptTokenAttributes = class extends HTMLElement {
  static get observedAttributes() { return ['data-token', 'data-extra']; }
  constructor() {
    super();
    window.ceEvents.push('ctor-has=' + this.hasAttribute('data-token'));
    new MutationObserver((records) => {
      for (const record of records) {
        window.ceEvents.push(
          'mo:' + record.attributeName + ':' +
          this.getAttribute(record.attributeName)
        );
      }
    }).observe(this, { attributes: true });
  }
  attributeChangedCallback(name, oldValue, newValue) {
    window.ceCallbackState[name] = {
      token: this.getAttribute('data-token'),
      extra: this.getAttribute('data-extra')
    };
    window.ceEvents.push('attr:' + name + ':' + oldValue + ':' + newValue);
    Promise.resolve().then(() => {
      window.ceEvents.push('promise:' + this.getAttribute(name));
    });
  }
  connectedCallback() {
    window.ceEvents.push('connected:' + this.getAttribute('data-token'));
  }
};
customElements.define('wpt-token-attributes', window.WptTokenAttributes);
</script>
<wpt-token-attributes data-token="owned" data-extra="extra"></wpt-token-attributes>
<script>
document.body.setAttribute('data-events', window.ceEvents.join('|'));
document.body.setAttribute(
  'data-token-callback-saw-extra',
  window.ceCallbackState['data-token'].extra || ''
);
document.body.setAttribute(
  'data-extra-callback-saw-token',
  window.ceCallbackState['data-extra'].token || ''
);
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element token attrs reaction channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-token-callback-saw-extra"),
                Some("extra"),
                "all parser token attributes must be appended before the first queued attribute reaction is flushed"
            );
            assert_eq!(
                body_element.attribute("data-extra-callback-saw-token"),
                Some("owned"),
                "later parser token attribute reactions should observe earlier token attributes"
            );
        }));
    }

    #[test]
    fn parser_nonce_hiding_queues_attribute_reaction_after_connected() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
window.nonceEvents = [];
class ParserNonceElement extends HTMLElement {
  static get observedAttributes() { return ['nonce']; }
  attributeChangedCallback(name, oldValue, newValue) {
    window.nonceEvents.push(`attribute:${name}:${oldValue}:${newValue}`);
  }
  connectedCallback() {
    window.nonceEvents.push('connected');
  }
}
customElements.define('parser-nonce-element', ParserNonceElement);
</script>
<parser-nonce-element nonce="secret"></parser-nonce-element>
<script>
const element = document.querySelector('parser-nonce-element');
document.body.setAttribute('data-events', window.nonceEvents.join('|'));
document.body.setAttribute('data-content-nonce', element.getAttribute('nonce'));
document.body.setAttribute('data-idl-nonce', element.nonce);
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-events"),
                Some("attribute:nonce:null:secret|connected|attribute:nonce:secret:"),
                "nonce hiding must enqueue its attribute reaction after the parser connection reaction"
            );
            assert_eq!(body_element.attribute("data-content-nonce"), Some(""));
            assert_eq!(body_element.attribute("data-idl-nonce"), Some("secret"));
        }));
    }

    #[test]
    fn parser_created_customized_builtin_direct_constructs_from_is_attribute() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.builtInEvents = [];
window.WptParserButton = class extends HTMLButtonElement {
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-built-in-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.builtInEvents.push([
      'constructor',
      this.localName,
      this.hasAttribute('is'),
      this.hasAttribute('data-token'),
      !!document.getElementById('after-button'),
      writeResult
    ].join('|'));
  }
  connectedCallback() {
    window.builtInEvents.push([
      'connected',
      this.getAttribute('is'),
      this.getAttribute('data-token'),
      !!document.getElementById('after-button'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-parser-button', window.WptParserButton, { extends: 'button' });
</script>
<button is="wpt-parser-button" data-token="owned"></button><span id="after-button"></span>
<script>
const element = document.querySelector('button');
document.body.setAttribute('data-events', window.builtInEvents.join('||'));
document.body.setAttribute('data-instance', String(element instanceof window.WptParserButton));
document.body.setAttribute('data-local-name', element.localName);
document.body.setAttribute('data-is', element.getAttribute('is') || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-built-in-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser customized built-in direct channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-events"),
                Some("constructor|button|false|false|false|InvalidStateError||connected|wpt-parser-button|owned|false|true"),
                "parser-created customized built-ins should synchronously construct from the token is= attribute"
            );
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-local-name"), Some("button"));
            assert_eq!(body_element.attribute("data-is"), Some("wpt-parser-button"));
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
    }

    #[test]
    fn parser_created_customized_builtin_uses_existing_element_validation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
window.onerror = () => true;

class MyCustomParagraph extends HTMLParagraphElement {
  constructor() {
    super();
    this.textContent = 'PASS';
  }
}
customElements.define('custom-p', MyCustomParagraph, { extends: 'p' });
</script>
<p id="targetp" is="custom-p"></p>
<script>
const targetp = document.getElementById('targetp');
document.body.setAttribute('data-p', [
  !!targetp,
  targetp.localName,
  targetp instanceof MyCustomParagraph,
  targetp instanceof HTMLParagraphElement,
  targetp.childNodes.length,
  targetp.textContent
].join('|'));

class MyCustomVideo extends HTMLVideoElement {
  constructor() {
    super();
    throw new Error('boom');
  }
}
customElements.define('custom-video', MyCustomVideo, { extends: 'video' });
</script>
<video id="targetvideo" is="custom-video"> <source></source> </video>
<script>
const targetvideo = document.getElementById('targetvideo');
document.body.setAttribute('data-video', [
  !!targetvideo,
  targetvideo.localName,
  targetvideo instanceof MyCustomVideo,
  targetvideo instanceof HTMLVideoElement,
  targetvideo.children.length
].join('|'));

class MyCustomForm extends HTMLFormElement {
  constructor() {
    super();
    throw new Error('boom');
  }
}
customElements.define('custom-form', MyCustomForm, { extends: 'form' });
</script>
<form id="targetform" is="custom-form"> <label></label><input> </form>
<script>
const targetform = document.getElementById('targetform');
document.body.setAttribute('data-form', [
  !!targetform,
  targetform.localName,
  targetform instanceof MyCustomForm,
  targetform instanceof HTMLFormElement,
  targetform.children.length
].join('|'));

class MyInputAttrs extends HTMLInputElement {
  constructor() {
    super();
    this.setAttribute('foo', 'bar');
  }
}
customElements.define('my-input-attr', MyInputAttrs, { extends: 'input' });
</script>
<input id="customized-input-attr" is="my-input-attr">
<script>
const input = document.getElementById('customized-input-attr');
document.body.setAttribute('data-input', [
  input instanceof MyInputAttrs,
  input instanceof HTMLInputElement,
  input.getAttribute('foo')
].join('|'));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-p"),
                Some("true|p|true|true|1|PASS"),
                "parser-created customized built-ins should allow constructor child mutations on the existing element"
            );
            assert_eq!(
                body_element.attribute("data-video"),
                Some("true|video|true|true|1"),
                "throwing parser-created customized built-ins should preserve the specialized custom prototype"
            );
            assert_eq!(
                body_element.attribute("data-form"),
                Some("true|form|true|true|2"),
                "throwing parser-created customized form built-ins should keep parser children on the existing element"
            );
            assert_eq!(
                body_element.attribute("data-input"),
                Some("true|true|bar"),
                "parser-created customized built-ins should allow constructor attribute mutations on the existing element"
            );
        }));
    }

    #[test]
    fn parser_created_custom_element_direct_constructs_before_declarative_shadow() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.shadowTiming = [];
class ShadowHostElement extends HTMLElement {
  constructor() {
    super();
    window.shadowTiming.push([
      'constructor',
      !!this.shadowRoot,
      this.childNodes.length,
      !!this.querySelector('template'),
      !!document.getElementById('after-shadow-host')
    ].join('|'));
  }
  connectedCallback() {
    window.shadowTiming.push([
      'connected',
      !!this.shadowRoot,
      this.childNodes.length,
      !!this.querySelector('template'),
      !!document.getElementById('after-shadow-host')
    ].join('|'));
  }
}
customElements.define('shadow-host-element', ShadowHostElement);
</script>
<shadow-host-element><template shadowrootmode="open"><span>Shadow Content</span></template><p>Light Content</p></shadow-host-element><span id="after-shadow-host"></span>
<script>
const element = document.querySelector('shadow-host-element');
document.body.setAttribute('data-constructor-event', window.shadowTiming[0] || '');
document.body.setAttribute('data-connected-event', window.shadowTiming[1] || '');
document.body.setAttribute('data-shadow-ready', [
  element instanceof ShadowHostElement,
  !!element.shadowRoot,
  element.shadowRoot && element.shadowRoot.textContent.trim(),
  !!element.querySelector('template'),
  element.textContent.trim(),
  !!document.getElementById('after-shadow-host')
].join('|'));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser custom element DSD timing local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-constructor-event"),
                Some("constructor|false|0|false|false"),
                "constructor must run before declarative shadow template contents and following siblings are parsed"
            );
            assert_eq!(
                body_element.attribute("data-connected-event"),
                Some("connected|false|0|false|false"),
                "connectedCallback is delivered before declarative shadow and child tokens are appended"
            );
            assert_eq!(
                body_element.attribute("data-shadow-ready"),
                Some("true|true|Shadow Content|false|Light Content|true"),
                "declarative shadow root should attach after construction while light DOM and following siblings continue parsing"
            );
        }));
    }

    #[test]
    fn parser_connected_form_associated_custom_element_dispatches_form_reactions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let page_vm = parse_phase_one_html_into_page_vm_for_test(
                r#"<!doctype html><html><body>
<script>
window.parserFaceEvents = [];
class ParserFaceElement extends HTMLElement {
  static formAssociated = true;
  constructor() {
    super();
    this.internals = this.attachInternals();
  }
  connectedCallback() {
    window.parserFaceEvents.push(`connected:${this.isConnected}`);
  }
  formAssociatedCallback(form) {
    window.parserFaceEvents.push(`form:${form && form.id}`);
  }
  formDisabledCallback(disabled) {
    window.parserFaceEvents.push(`disabled:${disabled}`);
  }
}
customElements.define('parser-face-element', ParserFaceElement);
</script>
<form id="parser-form"><fieldset disabled><parser-face-element id="parser-face"></parser-face-element></fieldset></form>
<script>
const face = document.getElementById('parser-face');
document.body.setAttribute('data-face-events', window.parserFaceEvents.join('|'));
document.body.setAttribute('data-face-form', face.internals.form && face.internals.form.id);
document.body.setAttribute('data-face-disabled', String(face.matches(':disabled')));
</script>
</body></html>"#,
            )
            .await;

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-face-events"),
                Some("connected:true|form:parser-form|disabled:true"),
                "parser insertion should dispatch connected, form-associated, and form-disabled reactions after the parser step"
            );
            assert_eq!(
                body_element.attribute("data-face-form"),
                Some("parser-form")
            );
            assert_eq!(body_element.attribute("data-face-disabled"), Some("true"));
        }));
    }

    #[test]
    fn parser_created_custom_element_direct_survives_table_foster_parenting() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.tableCeEvents = [];
window.WptTableTiming = class extends HTMLElement {
  constructor() {
    super();
    window.tableCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after-table'),
      this.isConnected
    ].join('|'));
  }
  connectedCallback() {
    window.tableCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.parentElement && this.parentElement.localName,
      this.nextElementSibling && this.nextElementSibling.id,
      !!document.getElementById('after-table'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-table-timing', window.WptTableTiming);
</script>
<table id="table"><wpt-table-timing data-token="owned"></wpt-table-timing><tr><td>cell</td></tr></table><span id="after-table"></span>
<script>
const element = document.querySelector('wpt-table-timing');
document.body.setAttribute('data-first-event', window.tableCeEvents[0] || '');
document.body.setAttribute('data-second-event', window.tableCeEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-parent', element.parentElement && element.parentElement.localName);
document.body.setAttribute('data-next', element.nextElementSibling && element.nextElementSibling.id);
document.body.setAttribute('data-instance', String(element instanceof window.WptTableTiming));
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after-table')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser table custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false"),
                "constructor must run before token attributes and later table siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|body|table|false|true"),
                "table foster parenting must connect the same parser-created handle before later parser siblings"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-parent"), Some("body"));
            assert_eq!(body_element.attribute("data-next"), Some("table"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
        }));
    }

    #[test]
    fn parser_created_custom_element_direct_waits_for_head_reprocess_body() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><head>
<script>
window.headCeEvents = [];
window.WptHeadTiming = class extends HTMLElement {
  constructor() {
    super();
    window.headCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.body,
      !!document.getElementById('after-head'),
      this.isConnected
    ].join('|'));
  }
  connectedCallback() {
    window.headCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.parentElement && this.parentElement.localName,
      !!document.body,
      !!document.getElementById('after-head'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-head-timing', window.WptHeadTiming);
</script>
<wpt-head-timing data-token="owned"></wpt-head-timing><meta id="after-head">
</head><body><span id="after-body"></span>
<script>
const element = document.querySelector('wpt-head-timing');
document.body.setAttribute('data-first-event', window.headCeEvents[0] || '');
document.body.setAttribute('data-second-event', window.headCeEvents[1] || '');
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-parent', element.parentElement && element.parentElement.localName);
document.body.setAttribute('data-instance', String(element instanceof window.WptHeadTiming));
document.body.setAttribute('data-after-head-visible', String(!!document.getElementById('after-head')));
document.body.setAttribute('data-after-body-visible', String(!!document.getElementById('after-body')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser head reprocess custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|true|false|false"),
                "head reprocess must create body before direct construction but before token attributes and following siblings"
            );
            assert_eq!(
                body_element.attribute("data-second-event"),
                Some("connected|owned|body|true|false|true"),
                "reprocessed custom element must connect under body before following parser tokens"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-parent"), Some("body"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(
                body_element.attribute("data-after-head-visible"),
                Some("true")
            );
            assert_eq!(
                body_element.attribute("data-after-body-visible"),
                Some("true")
            );
        }));
    }

    #[test]
    fn parser_created_custom_element_direct_skips_template_contents() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.templateConstructed = 0;
window.WptTemplateTiming = class extends HTMLElement {
  constructor() {
    super();
    window.templateConstructed += 1;
  }
};
customElements.define('wpt-template-timing', window.WptTemplateTiming);
</script>
<template id="template-root"><wpt-template-timing data-token="owned"></wpt-template-timing></template>
<script>
const instance = document.getElementById('template-root').content.firstElementChild;
document.body.setAttribute('data-constructed', String(window.templateConstructed));
document.body.setAttribute('data-html-element', String(instance instanceof HTMLElement));
document.body.setAttribute('data-instance', String(instance instanceof window.WptTemplateTiming));
document.body.setAttribute('data-token', instance.getAttribute('data-token') || '');
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one parser template custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-constructed"),
                Some("0"),
                "parser must not instantiate custom elements inside template contents"
            );
            assert_eq!(body_element.attribute("data-html-element"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("false"));
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
        }));
    }

    #[test]
    fn document_write_custom_element_direct_constructs_before_token_attributes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r#"<!doctype html><html><body>
<script>
window.writeCeEvents = [];
window.WptWrittenTiming = class extends HTMLElement {
  static get observedAttributes() { return ['data-token']; }
  constructor() {
    super();
    let writeResult = 'missing';
    try {
      document.write('<b id="bad-write">bad</b>');
      writeResult = 'ok';
    } catch (error) {
      writeResult = error.name;
    }
    window.writeCeEvents.push([
      this.hasAttribute('data-token'),
      !!document.getElementById('after-write'),
      this.isConnected,
      writeResult
    ].join('|'));
    new MutationObserver((records) => {
      for (const record of records) {
        window.writeCeEvents.push(
          'mo:' + record.attributeName + ':' +
          this.getAttribute(record.attributeName)
        );
      }
    }).observe(this, { attributes: true });
  }
  attributeChangedCallback(name, oldValue, newValue) {
    window.writeCeEvents.push('attr:' + name + ':' + oldValue + ':' + newValue);
    Promise.resolve().then(() => {
      window.writeCeEvents.push('promise:' + this.getAttribute(name));
    });
  }
  connectedCallback() {
    window.writeCeEvents.push([
      'connected',
      this.getAttribute('data-token'),
      this.childNodes.length,
      !!document.getElementById('after-write'),
      this.isConnected
    ].join('|'));
  }
};
customElements.define('wpt-written-timing', window.WptWrittenTiming);
document.write('<wpt-written-timing data-token="owned"></wpt-written-timing><span id="after-write"></span>');
</script>
<script>
const element = document.querySelector('wpt-written-timing');
document.body.setAttribute('data-events', window.writeCeEvents.join('|'));
document.body.setAttribute('data-first-event', window.writeCeEvents[0] || '');
document.body.setAttribute(
  'data-connected-event',
  window.writeCeEvents.find((event) => event.startsWith('connected')) || ''
);
document.body.setAttribute('data-token', element.getAttribute('data-token') || '');
document.body.setAttribute('data-after-visible', String(!!document.getElementById('after-write')));
document.body.setAttribute('data-instance', String(element instanceof window.WptWrittenTiming));
document.body.setAttribute('data-bad-write', String(!!document.getElementById('bad-write')));
</script>
</body></html>"#;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write custom element direct regression local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-first-event"),
                Some("false|false|false|InvalidStateError"),
                "document.write constructor must run before token attributes and following siblings are visible"
            );
            assert_eq!(
                body_element.attribute("data-events"),
                Some("false|false|false|InvalidStateError|attr:data-token:null:owned|mo:data-token:owned|promise:owned|connected|owned|0|false|true"),
                "document.write token attributes should queue initial attribute reactions and run the resulting microtask checkpoint before connectedCallback, then flush connected before child or following sibling tokens"
            );
            assert_eq!(
                body_element.attribute("data-connected-event"),
                Some("connected|owned|0|false|true"),
                "document.write connectedCallback must run before child and following sibling tokens"
            );
            assert_eq!(body_element.attribute("data-token"), Some("owned"));
            assert_eq!(body_element.attribute("data-after-visible"), Some("true"));
            assert_eq!(body_element.attribute("data-instance"), Some("true"));
            assert_eq!(body_element.attribute("data-bad-write"), Some("false"));
        }));
    }

    #[test]
    fn document_write_fostered_text_updates_live_range_boundaries() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                input_closed: &state.input_closed,
            };

            let html = r##"<!doctype html><html><body><table id="t"><script>
window.fosterRange = document.createRange();
window.fosterRange.setStart(document.body, document.body.childNodes.length);
window.fosterRange.setEnd(document.body, document.body.childNodes.length);
window.fosterBefore = document.body.childNodes.length;
document.write("hello");
</script></table><script>
document.body.setAttribute("data-range", [
  window.fosterBefore,
  window.fosterRange.startOffset,
  window.fosterRange.endOffset,
  Array.from(document.body.childNodes).map(node => node.nodeName + (node.id ? "#" + node.id : "")).join(",")
].join("|"));
</script></body></html>"##;

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one document.write fostered text live range local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver.advance_parser_step(page_vm, html, None).await
                },
            )
            .await
            .expect("parser step should complete");
            assert!(matches!(outcome, ParserStepAdvanceOutcome::Continue));

            let snapshot = page_vm.vm().snapshot_live_document();
            let body = snapshot.document_body_handle().expect("body");
            let body_element = snapshot
                .node(body)
                .and_then(Node::as_element)
                .expect("body element");
            assert_eq!(
                body_element.attribute("data-range"),
                Some("1|2|2|#text,TABLE#t,SCRIPT"),
                "parser-stream document.write foster parenting must update live Range boundaries like browser DOM mutation"
            );
        }));
    }

    #[test]
    fn inline_script_handoff_is_classified_as_blocking_classic_on_live_backend() {
        let final_url = Url::parse("https://example.test/").expect("test url");
        let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
        let mut state = ParseTimeDriverState::new(final_url.clone());
        let driver = ParserDriver {
            loader: &loader,
            final_url: &state.final_url,
            parser_session: &mut state.parser_session,
            scheduler: &mut state.scheduler,
            pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
            buffered_document_preloads: &mut state.buffered_document_preloads,
            service_worker_preload_context: state.service_worker_preload_context.as_ref(),
            input_closed: &state.input_closed,
        };

        let crate::parser::ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts: _,
            discovered_modulepreload_link_candidates: _,
            discovered_blocking_stylesheet_inputs: _,
        } = driver
            .parser_session
            .stream_handle()
            .borrow_mut()
            .pump_parser_step(
                "<!doctype html><html><head><script>window.answer = 42;</script></head></html>",
            );
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
            panic!("expected parser step to surface an inline script handoff");
        };
        let ParserScriptHandoff::BlockingClassic {
            node_id: handle,
            start_line: _,
            start_column: _,
            blocking_signatures_before: _,
            script,
        } = *handoff
        else {
            panic!("inline parser-visible classic should already be prepared as blocking classic");
        };

        assert_eq!(script.node_id, NodeId::new(handle.index()));
        assert_eq!(script.kind, crate::types::ScriptKind::Classic);
        assert_eq!(script.mode, crate::types::ScriptMode::Normal);
        assert_eq!(script.source_kind, crate::types::ScriptSourceKind::Inline);
        assert_eq!(script.url, final_url);
        assert_eq!(script.initiator_url, final_url);
    }

    #[test]
    fn no_execution_datablock_handoff_consumes_parser_prepare_state_on_live_backend() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_blocking_stylesheet_inputs: _,
                discovered_modulepreload_link_candidates: _,
            } = driver
                .parser_session
                .stream_handle()
                .borrow_mut()
                .pump_parser_step(
                "<!doctype html><html><head><script type=\"text/plain\" src=\"/data.txt\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at data-block script handoff");
            };
            let ParserScriptHandoff::NoExecution {
                node_id: handle, ..
            } = handoff.as_ref()
            else {
                panic!("data-block parser script should surface as no-execution handoff");
            };
            let handle = *handle;

            let parser_dom_host = driver
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(104),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let before = page_vm.vm().snapshot_live_document();
            let before_element = before
                .node(handle)
                .and_then(Node::as_element)
                .expect("script element should exist before handoff");
            assert!(before_element.script_parser_inserted_for_prepare());
            assert!(
                !before_element.script_async(),
                "DOM should not classify data-block scripts before prepare"
            );

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("data-block handoff should resolve without executing V8");
            assert!(matches!(outcome, ScriptHandoffOutcome::NoNavigation));

            let after = page_vm.vm().snapshot_live_document();
            let after_element = after
                .node(handle)
                .and_then(Node::as_element)
                .expect("script element should exist after handoff");
            assert!(
                !after_element.script_parser_inserted_for_prepare(),
                "inert data-block prepare should consume parser-inserted state"
            );
            assert!(
                after_element.script_async(),
                "inert data-block prepare should expose force-async for later reactivation"
            );
            assert!(
                !after_element.script_already_started(),
                "inert data-block prepare must leave the script startable"
            );
        }));
    }

    #[test]
    fn external_async_handoff_marks_parser_stream_already_started_on_live_backend() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state =
                ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts,
            discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script async src=\"/async.js\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at external async script handoff");
            };
            let handle = match handoff.as_ref() {
                ParserScriptHandoff::AsyncPostParse { node_id, .. }
                | ParserScriptHandoff::NonAsyncPostParse { node_id, .. }
                | ParserScriptHandoff::ImportMap { node_id, .. }
                | ParserScriptHandoff::NoExecution { node_id, .. }
                | ParserScriptHandoff::PreparationFailure { node_id, .. } => *node_id,
                ParserScriptHandoff::BlockingClassic { .. } => {
                    panic!("non-blocking async handoff should not carry a blocking-classic payload")
                }
            };

            // In the single-DOM model, the PageVm owns the parser's DomHost.
            // Simulate bootstrap by giving the parser's DomHost to the PageVm.
            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            for mut script in discovered_async_prefetch_scripts {
                bind_parser_owned_script_handle(&mut page_vm, &mut script);
                let _ = driver
                    .scheduler
                    .on_parser_discovered_async_candidate_with_shared_load(
                        script,
                        Some(SharedScriptSourceLoad::ready_err(
                            "synthetic async source terminal",
                        )),
                    );
            }
            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff.clone(), None)
                .await
                .expect("external async handoff should resolve without executing V8");

            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "external async handoff should resolve without navigation (credit granted internally)"
            );

            // In the single-DOM model, the already-started bit is on the runtime's DomHost.
            let runtime_snapshot = page_vm.vm().snapshot_live_document();
            assert!(
                runtime_snapshot
                    .node(handle)
                    .and_then(|node| node.as_element())
                    .is_some_and(|element| element.script_already_started()),
                "runtime live document should record async handoff ownership after mutation"
            );
            let expected_handle = format!("parser-script-native-{}", handle.index());
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .resolve_host_script_handle(&expected_handle),
                Some(handle),
                "async parser-owned handoff should register a parser-owned host script handle"
            );
        });
    }

    #[test]
    fn blocking_classic_handoff_registers_parser_owned_handle_on_live_backend() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/app.css\"><script src=\"/app.js\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at blocking classic handoff");
            };
            let ParserScriptHandoff::BlockingClassic {
                node_id: handle, ..
            } = handoff.as_ref()
            else {
                panic!("expected parser-blocking classic to classify as blocking classic");
            };
            let handle = *handle;

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            activate_standalone_main_parser_continuation_for_test(&mut page_vm);
            page_vm
                .vm_mut()
                .document_runtime
                .note_discovered_document_owned_blocking_stylesheet_inputs(
                    discovered_blocking_stylesheet_inputs.iter(),
                );

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("blocking classic handoff should resolve on the live backend");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::BlockedOnStylesheet(_)),
                "blocking classic handoff should stay stylesheet-gated in this fixture"
            );
            let expected_handle = format!("parser-script-native-{}", handle.index());
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .resolve_host_script_handle(&expected_handle),
                Some(handle),
                "blocking classic handoff should register a parser-owned host script handle"
            );
            let runtime_snapshot = page_vm.vm().snapshot_live_document();
            assert!(
                runtime_snapshot
                    .node(handle)
                    .and_then(|node| node.as_element())
                    .is_some_and(|element| element.script_already_started()),
                "blocking classic handoff should mark already-started on the live runtime DOM"
            );
        });
    }

    #[test]
    fn non_async_post_parse_handoff_registers_pending_before_source_and_seals_without_waiting() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script defer src=\"/defer.js\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at defer handoff");
            };
            let ParserScriptHandoff::NonAsyncPostParse {
                node_id: handle,
                script,
                ..
            } = handoff.as_ref()
            else {
                panic!("expected defer script to classify as non-async post-parse");
            };
            let handle = *handle;
            let script = script.clone();
            let (source_tx, source_rx) = tokio::sync::oneshot::channel();
            driver.buffered_document_preloads.entries.insert(
                BufferedScriptPreloadKey::from_script(&script).expect("defer preload key"),
                BufferedScriptPreloadEntry {
                    request: preload_request_for_script(
                        &script,
                        moli_fetch::RequestResourceType::ParserBlockingScript,
                    ),
                    load: SharedScriptSourceLoad::spawn_for_test(async move {
                        source_rx.await.expect("defer source result")
                    }),
                },
            );

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("defer handoff should resolve without executing V8");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "defer handoff should remain a scheduling-only step"
            );
            let expected_handle = format!("parser-script-native-{}", handle.index());
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .resolve_host_script_handle(&expected_handle),
                Some(handle),
                "non-async parser-owned handoff should register a parser-owned host script handle"
            );
            let task_owner = page_vm
                .vm()
                .current_main_document_task_owner()
                .expect("main parser-deferred document owner");
            let parser_owner =
                crate::module_script_continuation::MainParserDocumentOwner::new(task_owner);
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .parser_module_document_scripts()
                    .has_load_blocking_document_script_work(parser_owner),
                "handoff must install the PendingScript before its pending source finishes"
            );
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .parser_module_document_scripts()
                    .has_after_parsing_script(parser_owner),
                "parser-deferred work cannot execute before EOF"
            );
            assert!(
                page_vm
                    .seal_main_parser_deferred_scripts(task_owner)
                    .is_some(),
                "EOF seal should synchronously arm parser-deferred work"
            );
            assert!(
                !page_vm
                    .vm()
                    .document_runtime
                    .parser_module_document_scripts()
                    .next_after_parsing_script_is_ready(parser_owner),
                "EOF must not wait for or inline-apply the source result"
            );

            source_tx
                .send(Ok("globalThis.__mainDeferred = 1;".to_owned()))
                .expect("defer source receiver should remain alive");
            if !page_vm.page_resource_completion_queue().has_ready_completion() {
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    page_vm.wait_for_page_resource_completion_for_test(),
                )
                .await
                .expect("defer source completion should arrive");
            }
            let mut source = page_vm.page_resource_completion_queue();
            let completion = page_vm
                .apply_one_page_resource_terminal_owner_admission_for_test(&mut source)
                .expect("defer source completion should apply")
                .expect("defer source completion should make progress");
            assert_eq!(
                completion.action.source(),
                RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .parser_module_document_scripts()
                    .next_after_parsing_script_is_ready(parser_owner),
                "typed source completion should release the original PendingScript"
            );
        });
    }

    #[test]
    fn parser_owned_external_module_handoff_starts_pending_script_tree_root_fetch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script type=\"module\" src=\"/module.mjs\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at module handoff");
            };
            let ParserScriptHandoff::NonAsyncPostParse {
                node_id: handle,
                ..
            } = handoff.as_ref()
            else {
                panic!("expected parser-owned external module to classify as non-async post-parse");
            };
            let handle = *handle;

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("module handoff should resolve without executing V8");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "module handoff should remain a scheduling-only step"
            );
            let expected_handle = format!("parser-script-native-{}", handle.index());
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .resolve_host_script_handle(&expected_handle),
                Some(handle),
                "parser-owned external module handoff should register a parser-owned host script handle"
            );
            let root_key = crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/module.mjs").expect("root url"),
            );
            let root_entry = page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&root_key)
                .expect("parser handoff should eagerly start module tree root fetch");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_state(root_entry),
                crate::module_runtime::ModuleMapEntryState::Fetching
            );
        });
    }

    #[test]
    fn parser_owned_module_handoff_starts_external_root_pending_tree_without_source_load() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script type=\"module\" src=\"/pending.mjs\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at module handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("module handoff should start root pending script tree");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "module handoff should remain a scheduling-only step"
            );

            let pending_root_key = crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/pending.mjs").expect("root url"),
            );
            let pending_dep_key = crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/pending-dep.mjs").expect("dependency url"),
            );
            let pending_root_entry = page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&pending_root_key)
                .expect("parser handoff should start external root graph fetch immediately");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_state(pending_root_entry),
                crate::module_runtime::ModuleMapEntryState::Fetching
            );
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_id(&pending_dep_key)
                    .is_none(),
                "dependency cannot be discovered until the root graph fetch completes"
            );

            tokio::task::yield_now().await;
            assert!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_id(&pending_dep_key)
                    .is_none(),
                "module graph dependency discovery must wait for native module fetch completion"
            );
        });
    }

    #[test]
    fn parser_owned_module_handoff_starts_loaded_source_pending_tree_dependencies() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script type=\"module\" src=\"/ready.mjs\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at module handoff");
            };

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            page_vm.vm_mut().document_runtime.insert_native_module_source(
                crate::module_runtime::ModuleMapKey::java_script(
                    Url::parse("https://example.test/ready.mjs").expect("root url"),
                ),
                crate::module_runtime::ModuleSource::text(
                    "import './ready-dep.mjs'; globalThis.readyModuleShouldNotRunYet = true;"
                        .to_owned(),
                ),
            );

            let outcome = driver
                .handle_parse_time_script_handoff(&mut page_vm, *handoff, None)
                .await
                .expect("module handoff should start loaded-source pending script tree");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "module handoff should remain a scheduling-only step"
            );

            let root_key = crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/ready.mjs").expect("root url"),
            );
            let dep_key = crate::module_runtime::ModuleMapKey::java_script(
                Url::parse("https://example.test/ready-dep.mjs").expect("dependency url"),
            );
            let root_entry = page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&root_key)
                .expect("loaded module source should install the root module entry");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_state(root_entry),
                crate::module_runtime::ModuleMapEntryState::Compiled
            );
            let dep_entry = page_vm
                .vm()
                .document_runtime
                .native_module_entry_id(&dep_key)
                .expect("loaded module handoff should discover static dependencies immediately");
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .native_module_entry_state(dep_entry),
                crate::module_runtime::ModuleMapEntryState::Fetching
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval("String(globalThis.readyModuleShouldNotRunYet)")
                    .expect("read module side effect before page task"),
                "undefined",
                "prestarted loaded-source graph must not evaluate before its ordered page task"
            );
        });
    }

    #[test]
    fn parser_owned_inline_importmap_handoff_registers_parser_owned_handle_on_live_backend() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader =
                Box::leak(Box::new(ResourceRequestClient::new(&FetchConfig::default()).expect("default loader")));
            let state = Box::leak(Box::new(ParseTimeDriverState::new(final_url)));
            let mut driver = ParserDriver {
                loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs: _,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><script type=\"importmap\">{\"imports\":{\"fixture\":\"/module.mjs\"}}</script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at inline importmap handoff");
            };
            let ParserScriptHandoff::ImportMap { node_id: handle, .. } = handoff.as_ref() else {
                panic!("expected parser-owned inline importmap registration handoff");
            };
            let handle = *handle;

            let parser_dom_host = driver.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");

            let local_executor = page_vm.local_executor.clone();
            let page_vm_ptr: *mut PageVm = &mut page_vm;
            let driver_ptr: *mut ParserDriver<'_, '_> = &mut driver;
            let handoff = handoff.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "phase-one inline importmap handoff local task channel closed",
                async move {
                    let page_vm = unsafe { &mut *page_vm_ptr };
                    let driver = unsafe { &mut *driver_ptr };
                    driver
                        .handle_parse_time_script_handoff(page_vm, *handoff, None)
                        .await
                },
            )
            .await
            .expect("inline importmap handoff should resolve without executing external fetch");
            assert!(
                matches!(outcome, ScriptHandoffOutcome::NoNavigation),
                "inline importmap handoff should remain a current-turn scheduling step"
            );
            let expected_handle = format!("parser-script-native-{}", handle.index());
            assert_eq!(
                page_vm
                    .vm()
                    .document_runtime
                    .resolve_host_script_handle(&expected_handle),
                Some(handle),
                "parser-owned inline importmap handoff should register a parser-owned host script handle before current-turn execution"
            );
            assert_eq!(
                page_vm
                    .vm_mut()
                    .document_runtime
                    .resolve_module_specifier(
                        "fixture",
                        &Url::parse("https://example.test/").expect("base url"),
                    )
                    .expect("registered import map should resolve fixture"),
                Url::parse("https://example.test/module.mjs").expect("mapped url"),
                "dedicated import-map handoff should register before later module preparation"
            );
        }));
    }

    #[test]
    fn external_blocking_classic_handoff_is_stylesheet_gated_on_live_backend() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(async move {
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state =
                ParseTimeDriverState::new(final_url);
            let driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let crate::parser::ParserPumpOutcome {
                result,
                discovered_async_prefetch_scripts: _,
                discovered_modulepreload_link_candidates: _,
                discovered_blocking_stylesheet_inputs,
            } = driver.parser_session.stream_handle().borrow_mut().pump_parser_step(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"/app.css\"><script src=\"/app.js\"></script></head></html>",
            );
            let ParserPumpStep::Yield(ParserYield::Script(handoff)) = result else {
                panic!("expected parser step to stop at external classic script handoff");
            };
            let ParserScriptHandoff::BlockingClassic {
                node_id: _handle,
                start_line: _,
                start_column: _,
                blocking_signatures_before,
                script: _script,
            } = *handoff
            else {
                panic!("external parser-blocking classic should already be prepared");
            };
            assert!(
                !blocking_signatures_before.is_empty(),
                "parser-owned blocking classic handoff should carry blocking stylesheet signatures discovered before the script"
            );
            let parser_stream_snapshot = state.parser_session.stream_handle().borrow().snapshot_parser_stream_document();

            let mut live_runtime = DocumentRuntime::new_networked(
                &parser_stream_snapshot.clone().into_document(),
                &loader,
            );
            let stylesheet_gated =
                blocking_classic_is_stylesheet_gated_for_testing(
                &mut live_runtime,
                &discovered_blocking_stylesheet_inputs,
                &blocking_signatures_before,
            );

            assert!(
                stylesheet_gated,
                "stylesheet discovered before a parser-blocking classic script should gate execution on the parser-stream backend"
            );
        });
    }

    #[test]
    fn parser_owner_style_import_handoff_is_stylesheet_gated_on_live_page_vm() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state.parser_session.stream_handle().borrow_mut().take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(1),
                local_executor,
                &loader,
                &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                cross_origin_embedder_policy: Default::default(),
                document_isolation_policy: Default::default(),
                cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                indexed_db_manager: None,
            storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
                },
            PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    "<!doctype html><html><head><style>@import url('/style.css');</style><script>window.afterStyle = true;</script></head></html>",
                    None,
                )
                .await
                .expect("parser step should complete");

            assert!(
                matches!(outcome, ParserStepAdvanceOutcome::BlockedOnStylesheet(_)),
                "parser-created style import should gate parser-blocking script on live PageVm"
            );
        }));
    }

    #[test]
    fn parser_owner_body_stylesheet_pause_preserves_unconsumed_tail_on_live_page_vm() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let mut state = ParseTimeDriverState::new(final_url);
            let parser_dom_host = state
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let mut page_vm = PageVm::new(
                PageId::new_for_testing(2),
                local_executor,
                &loader,
                &PageVmEnvConfig {
                    root_frame_id: None,
                    main_document_commit: None,
                    top_level_storage_key: None,
                    web_storage: crate::RendererWebStorageHandles::ephemeral(),
                    document_start_scripts: vec![],
                    runtime_bindings: vec![],
                    runtime_inspector_session_restore_snapshots: vec![],
                    runtime_isolated_worlds: vec![],
                    permission_overrides: vec![],
                    extra_http_headers: vec![],
                    document_content_security_policies: Vec::new(),
                    response_content_security_policies: Vec::new(),
                    response_content_security_report_only_policies: Vec::new(),
                    response_referrer_policy: None,
                    content_security_reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
                    cross_origin_embedder_policy: Default::default(),
                    document_isolation_policy: Default::default(),
                    cross_origin_isolated: false,
                    document_default_language: None,
                    document_last_modified: None,
                    locale_override: None,
                    timezone_override: None,
                    script_execution_disabled: false,
                    bypass_content_security_policy: false,
                    cpu_throttling_rate: 1.0,
                    emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
                    idle_override: None,
                    viewport_surface: None,
                    network_offline: false,
                    blocked_url_patterns: Vec::new(),
                    indexed_db_manager: None,
                    storage_bucket_store: None,
                    fetch_subresource_interception_enabled: false,
                    fetch_subresource_interception_resource_type: None,
                    layout_policy: moli_page_types::LayoutPolicy::default(),
                    wpt_extensions_enabled: false,
                    navigation_bootstrap_entry: None,
                    reserved_service_worker_client_id: None,
                },
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let mut driver = ParserDriver {
                loader: &loader,
                final_url: &state.final_url,
                parser_session: &mut state.parser_session,
                scheduler: &mut state.scheduler,
                pending_parsing_blocking_script: &mut state.pending_parsing_blocking_script,
                buffered_document_preloads: &mut state.buffered_document_preloads,
                service_worker_preload_context: state.service_worker_preload_context.as_ref(),
                input_closed: &state.input_closed,
            };

            let outcome = driver
                .advance_parser_step(
                    &mut page_vm,
                    concat!(
                        "<!doctype html><html><body>",
                        "<main id=phase-one-before>before</main>",
                        "<link rel=stylesheet href='/style.css'>",
                        "<script src='/after-pause.js'></script>",
                        "<footer id=phase-one-after>after</footer>",
                        "</body></html>"
                    ),
                    None,
                )
                .await
                .expect("parser step should reach its stylesheet boundary");

            assert!(matches!(
                outcome,
                ParserStepAdvanceOutcome::BlockedOnStylesheetParserPause
            ));
            let live_dom = page_vm.vm().document_runtime.dom_host();
            assert!(live_dom.element_handle_by_id("phase-one-before").is_some());
            assert!(
                live_dom.element_handle_by_id("phase-one-after").is_none(),
                "phase-one must retain the parser tail until the body stylesheet settles"
            );
            assert!(
                driver
                    .buffered_document_preloads
                    .entries
                    .contains_key(&classic_preload_key(
                        "https://example.test/after-pause.js"
                    )),
                "a stylesheet parser pause must scan the unconsumed tail for preloadable scripts"
            );
        }));
    }

    #[test]
    fn pending_parsing_blocking_turn_preserves_triggered_navigation_progress() {
        let pending_parsing_blocking_wait = PendingParsingBlockingWait::PageTaskBlockingStylesheet;
        let owner = ParseTimeOwner::Document;
        let result = OwnerStepProgress::TriggeredNavigation;

        assert_eq!(result, OwnerStepProgress::TriggeredNavigation);
        assert!(pending_parsing_blocking_wait.is_pending());
        assert_eq!(owner, ParseTimeOwner::Document);
    }

    #[test]
    fn pending_parsing_blocking_wake_from_task_like_source_prefers_task_drain() {
        assert!(pending_parsing_blocking_wake_prefers_ready_task_drain(
            PendingParsingBlockingWake::Source(
                crate::document_runtime::DocumentProcessingWakeSource::InjectedPageTask,
            ),
        ));
        assert!(pending_parsing_blocking_wake_prefers_ready_task_drain(
            PendingParsingBlockingWake::Source(
                crate::document_runtime::DocumentProcessingWakeSource::TaskSourceLoadCompletion,
            ),
        ));
    }

    #[test]
    fn document_execution_debug_starts_on_parser_without_pending_turn_state() {
        let debug = format!(
            "DocumentExecutionState {{ owner: {:?}, parser_step_ready: false, pending_parsing_blocking_wait: None }}",
            ParseTimeOwner::Parser
        );

        assert!(
            debug.contains("owner: Parser"),
            "new coordinator should start on the parser owner"
        );
        assert!(
            debug.contains("pending_parsing_blocking_wait: None"),
            "new coordinator should not start in stylesheet-blocking wait mode"
        );
    }
}
