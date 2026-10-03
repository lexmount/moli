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
#[cfg(test)]
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

fn script_preloads_require_owner_admission(env: &PageVmEnvConfig) -> bool {
    !main_document_parser_scripting_enabled(env)
        || env.fetch_subresource_interception_enabled
            && env
                .fetch_subresource_interception_resource_type
                .is_none_or(|resource_type| {
                    resource_type.has_same_cdp_fetch_interception_type(
                        crate::types::SubresourceResourceType::Script,
                    )
                })
}

fn main_document_parser_scripting_enabled(env: &PageVmEnvConfig) -> bool {
    !env.script_execution_disabled
        && crate::content_security_policy::content_security_policy_sandbox_allows_scripts(
            &env.document_policy_container
                .response_content_security_policies,
        )
}

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
        let state = Box::leak(Box::new(ParseTimeDriverState::new_with_scripting_enabled(
            final_url,
            main_document_parser_scripting_enabled(&env),
        )));
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

    pub(super) async fn parse_phase_one_html_into_page_vm_for_test_with_env(
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
            extra_http_headers: Default::default(),
            navigator_identity: Default::default(),
            document_policy_container: Default::default(),
            document_default_language: None,
            document_last_modified: None,
            script_execution_disabled: false,
            bypass_content_security_policy: false,
            emulated_media: crate::protocol_types::EmulatedMediaOverrides::default(),
            idle_override: None,
            navigator_overrides: Default::default(),
            viewport_surface: None,
            document_activity: Default::default(),
            network_offline: false,
            blocked_url_patterns: Vec::new(),
            indexed_db_manager: None,
            storage_bucket_store: None,
            fetch_subresource_interception_enabled: false,
            fetch_subresource_interception_resource_type: None,
            layout_policy: moli_page_types::LayoutPolicy::default(),
            scrollbars_hidden: false,
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        }
    }

    pub(super) fn default_test_page_vm_env_config_with(
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

    fn main_parser_finish_waits_for_phase_one_continuation_boundary_inner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");

        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let final_url = Url::parse("https://example.test/").expect("test url");
            let loader_owner =
                ResourceRequestClient::new(&FetchConfig::default()).expect("default loader");
            let loader = loader_owner.clone();
            let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
            state.close_input();
            let parser_bridge =
                crate::document_runtime::ParserConnectedScriptBridge::for_session(
                    &state.parser_session,
                )
                .expect("HTML parser should expose a parser-connected script bridge");
            let resume_permit = parser_bridge.suspend(
                crate::live_document_parser::ParserSuspensionCause::ParserCreatedStylesheet {
                    owner: crate::dom::native::NativeNodeId::new(1),
                },
            );
            assert_eq!(
                state.parser_session.finish_request_state(),
                crate::live_document_parser::DocumentParserFinishRequestState::Delayed
            );
            assert!(parser_bridge.resume(resume_permit));
            let parser_dom_host = state
                .parser_session
                .stream_handle()
                .borrow_mut()
                .take_parser_stream_dom_host();
            let local_executor = JsLocalExecutor::new();
            let page_vm = PageVm::new(
                PageId::new_for_testing(103),
                local_executor,
                &loader,
                &default_test_page_vm_env_config(),
                PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
                parser_dom_host,
                Instant::now(),
            )
            .expect("page vm");
            let phase_runtime = ConcurrentParseTimeRuntime::new_parser_owner(
                loader,
                PageVmInitStage::Load,
                state,
                page_vm,
            );

            let completion = ParseTimePhaseOnePump::new(phase_runtime)
                .run_to_completion()
                .await
                .expect("EOF should park instead of finishing in the requesting turn");
            let super::loop_protocol::ParseTimeOwnerCompletion::PendingPageTask(
                mut phase_runtime,
            ) = completion
            else {
                panic!("EOF should publish one phase-one continuation task");
            };
            assert_eq!(
                phase_runtime.state.parser_session.finish_request_state(),
                crate::live_document_parser::DocumentParserFinishRequestState::Delayed,
                "the requesting turn must record an explicitly delayed finish"
            );
            assert!(
                phase_runtime
                    .page_vm
                    .page_task_executor_sources_for_test()
                    .has_scheduler_task_for_executor_test(|descriptor| matches!(
                        descriptor,
                        crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                            owner:
                                crate::page_task_queue::RendererPageNetworkingOwner::MainParserContinuation(
                                    _
                                ),
                            ..
                        }
                    )),
                "deferred finish must cross a selected Page task boundary"
            );

            let request_client = phase_runtime
                .page_vm
                .main_document_resource_loader()
                .request_client()
                .clone();
            assert!(
                phase_runtime
                    .page_vm
                    .run_exact_selected_page_task_for_test(
                        crate::runtime::page_vm::PageSelectedTaskTestSelector::MainParserContinuation,
                        &request_client,
                    )
                    .await
                    .expect("main parser continuation should execute"),
                "the exact deferred-finish continuation should remain selectable"
            );

            // Keep the insertion bridge alive across finalization. Its weak
            // stream capability must not participate in parser ownership.
            let completion = ParseTimePhaseOnePump::new(phase_runtime)
                .run_to_completion()
                .await
                .expect("admitted parser continuation should finish");
            let super::loop_protocol::ParseTimeOwnerCompletion::AdvancePhase {
                runtime,
                reason: super::loop_protocol::ParseTimePhaseTransitionReason::ParserCompleted,
            } = completion
            else {
                panic!("the admitted delayed finish should advance to phase two");
            };
            assert_eq!(
                runtime.state.parser_session.run_state(),
                crate::live_document_parser::DocumentParserRunState::Finished
            );
            assert_eq!(
                runtime.state.parser_session.finish_request_state(),
                crate::live_document_parser::DocumentParserFinishRequestState::Admitted
            );
            drop(parser_bridge);
        }));
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
            let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
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
            let mut state = ParseTimeDriverState::new_with_scripting_enabled_for_test(final_url);
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
                            Vec::new().into(),
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

    mod extracted;
}
