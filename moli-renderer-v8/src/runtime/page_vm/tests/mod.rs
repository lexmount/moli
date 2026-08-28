use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::document_script_scheduler::{
    DocumentScriptExecutionLane, PageOwnedDocumentScriptWork, ParserPendingScriptId,
    ParserPendingScriptKey,
};
use crate::dom::{NodeId, native::DomHost};
use crate::dynamic_script_owner::{
    DynamicScriptFailureKind, DynamicScriptOwnerId, DynamicScriptRunnable,
};
use crate::host::ModuleFailurePolicy;
use crate::module_runtime::{
    ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleGraphHandle,
    ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage, ModuleMapEntryState,
    ModuleMapKey, ModuleSource, NativeModuleGraphFetchRequest, NativeModuleSingleFetchRequest,
};
use crate::module_script_continuation::{
    MainParserDeferredClassicSourceLoadCompletion, MainParserDocumentOwner,
    ModuleScriptContinuation, NativeModuleOwnerActions,
};
use crate::page_resource_completion::{
    MainModuleFetchNetworkAttribution, MainModulepreloadFetchCompletion,
    MainParserDeferredClassicSourceNetworkAttribution, MainParserModuleGraphFetchCompletion,
    MainParserModuleGraphFetchTarget, PageResourceCompletionBodyActivity,
    PageResourceCompletionDocumentEffect, PageResourceCompletionOutputEffect,
    PageResourceCompletionPostCheckpointEffect, PageResourceCompletionTurnAction,
    PageResourceCompletionTurnOutcome, RendererPageResourceCompletion,
    RendererPageResourceCompletionOwner, RendererPageResourceTerminal,
};
use crate::page_task_queue::{
    PageMainDocumentRuntimeActionKind, PageMainDocumentRuntimeTargetEffect,
    PageModulepreloadStartDocumentEffect, PageTask, RendererPageModulepreloadStartOwner,
    RendererPageNetworkingSource,
};
use crate::page_task_queue::{PostParseLifecycleWork, PostParsePageOwnedWork};
use crate::parser::HtmlParser;
use crate::planning::{
    PreparedScript, PreparedScriptSourceLoadOutcome, ScriptFetchMetadata, ScriptSource,
    SharedScriptSourceLoad,
};
use crate::runtime::{
    RendererOwnerResourceActivitySource, RendererPageCommand, RendererRuntimeObservableSourceItem,
    RendererSharedWorkerTargetEvent,
};
use crate::script_vm::{PostParseLifecycleAdvance, PostParseLifecycleCompletionAction};
use crate::types::{
    ChildBlockingStylesheetLoadCompletion, ChildBlockingStylesheetNetworkResult,
    ChildClassicScriptLoadCompletion, ChildClassicScriptNetworkAttribution,
    ChildDynamicImportFetchCompletion, ChildModuleDependencyFetchCompletion,
    ChildModuleFetchNetworkAttribution, ChildModulepreloadFetchCompletion,
    ChildParserModuleRootFetchCompletion, ModuleGraphFetchCompletion, ModuleGraphFetchOrdering,
    ModuleGraphFetchRequester, ScriptKind, ScriptMode, ScriptNetworkOutput,
    ScriptNetworkOutputItem, ScriptObservableOutput, ScriptObservableOutputItem, ScriptRun,
    ScriptRunOutcome, ScriptSkipReason, ScriptSourceKind, SubresourceBodyFinishedResult,
    SubresourceNetworkRecord, SubresourceRequestInitiatorType, SubresourceResponseWaitCriteria,
    WebSocketLifecycleEvent, WebSocketNetworkEvent,
};
use crate::types::{SubresourceNetworkOutcome, SubresourceResourceType};
use moli_fetch::FetchConfig;
use moli_module_script_tree as module_tree;
use moli_websocket::test_support::{
    header_value, spawn_abrupt_close_after_open_websocket_server,
    spawn_backpressure_websocket_server, spawn_child_document_and_header_capture_websocket_server,
    spawn_close_after_goodbye_websocket_server, spawn_cookie_echo_websocket_server,
    spawn_delayed_passive_close_websocket_server, spawn_dropping_websocket_server,
    spawn_header_capture_websocket_server, spawn_http_connect_proxy,
    spawn_raw_websocket_response_server, spawn_receive_backpressure_websocket_server,
    spawn_send_backpressure_websocket_server, spawn_server_close_websocket_server,
    spawn_server_close_websocket_server_with_frame, spawn_set_cookie_websocket_server,
    spawn_sleeping_handshake_websocket_server, spawn_subprotocol_websocket_server,
    spawn_text_echo_websocket_server, spawn_tls_header_capture_websocket_server,
    spawn_triggered_text_websocket_server,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use url::Url;

use super::super::{PageId, PageVmInitStage, PageVmNavigationTurnOutcome};
use super::{
    IntoPageTaskCompletion, PageDomManipulationTestFamily, PageSelectedTaskTestSelector, PageVm,
    PageVmEnvConfig, PageVmRuntimeHooks, PostParseLifecycleLoopAdvance,
};
use crate::frame_owner_model::{
    ChildDocumentModuleFetchTarget, ChildFrameSemanticTurnKind, DocumentId,
    FrameDocumentModuleClientEntryId, FrameDocumentModuleClientId,
    FrameDocumentModuleClientRegistration, FrameDocumentModuleClientReservation,
    FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleFetchDisposition,
    FrameDocumentModulepreloadFetchTask, FrameDocumentModulepreloadLinkClient,
    FrameDocumentStaticDependencyModuleClient, FrameDocumentTaskOwner, FrameRealmId,
    FrameRequestId,
};
use crate::native_bridge::PendingWindowMessageEndpoint;

mod async_subresource_completion;
mod broadcast_channel_delivery;
mod child_classic_source_load_completion;
mod child_document_completion;
mod child_document_lifecycle;
mod child_document_script_ready;
mod child_dynamic_import_completion;
mod child_dynamic_import_owner_action;
mod child_host_load;
mod child_module_dependency_fetch_start;
mod child_module_document_script_ready;
mod child_module_script_terminal;
mod child_module_script_terminal_completion;
mod child_modulepreload_event_action;
mod child_navigation_commit_completion;
mod child_parser_module_root_start_completion;
mod child_realm_materialization;
mod child_realm_materialization_completion;
mod command_checkpoint;
mod cssom_zoom;
mod dedicated_worker_client_event;
mod document_script_completion;
mod element_toggle_event;
mod fetch_xhr;
mod file_entry_file_callback;
mod file_system_directory_reader;
mod grid_resolved_track_values;
mod hash_change_delivery;
mod history_traversal;
mod image_load_event;
mod image_root_coordinates;
mod indexed_db;
mod inline_svg_paint;
mod internal_loading_completion;
mod intrinsic_percentage_resolution;
mod lifecycle;
mod main_document_post_parse_completion;
mod main_document_runtime;
mod main_dynamic_import_completion;
mod main_modulepreload_completion;
mod main_native_module_completion;
mod main_parser_continuation_task;
mod main_parser_module_completion;
mod main_parser_owned_module_completion;
mod main_runtime_module_completion;
mod main_runtime_script_completion;
mod media_element_event;
mod message_port_delivery;
mod misc_platform_api;
mod module_reaction;
mod modulepreload_start_completion;
mod navigation_api_task;
mod opfs;
mod parser_written_script_residence;
mod popup_document_completion;
mod preferred_aspect_ratio;
mod rendering_update;
mod service_worker;
mod service_worker_client_message;
mod service_worker_internal;
mod shared_worker_client_event;
mod storage_event_delivery;
mod stylesheet_task;
mod text_track_default_mode;
mod text_track_load;
mod timer;
mod user_interaction;
mod view_transition;
mod wait_observer;
mod webcrypto;
mod websocket;
mod window_message;
mod worker;
mod worker_host_bridge;

fn has_ready_runtime_script_continuation_for_test(page_vm: &PageVm) -> bool {
    page_vm
        .page_task_executor_sources_for_test()
        .has_main_document_runtime_action_for_executor_test(
            PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation,
        )
}

#[tokio::test(flavor = "current_thread")]
async fn page_resource_completion_rejects_stale_document_before_application() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .document_runtime
            .set_document_ready_state(crate::dom::native::DocumentReadyState::Complete);
        park_current_document_websocket_for_test(
            &mut page_vm,
            moli_websocket::Event::TextMessage {
                socket_id: 72,
                data: "blocked".to_owned(),
            },
        )
        .await;
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("test page should install a main Document owner");
        let root_document = page_vm.document_lifecycle.identity().document;
        let stale_owner = FrameDocumentTaskOwner::new(
            current_owner.scheduler_lane_id,
            current_owner.local_window_id,
            DocumentId(current_owner.document_id.0 + 1),
        );
        let later_stale_owner = FrameDocumentTaskOwner::new(
            current_owner.scheduler_lane_id,
            current_owner.local_window_id,
            DocumentId(current_owner.document_id.0 + 2),
        );
        let completion_for = |owner, parser_position, node_id, network_error: Option<&str>| {
            MainParserDeferredClassicSourceLoadCompletion::new(
                ParserPendingScriptId::from_key(
                    MainParserDocumentOwner::new(owner),
                    ParserPendingScriptKey::from_parts_for_test(
                        parser_position,
                        NodeId::new(node_id),
                    ),
                ),
                PreparedScriptSourceLoadOutcome {
                    source_result: Ok("globalThis.__staleDeferRan = true".to_owned()),
                    source_bytes: None,
                    network_result: network_error.map(|error| Arc::new(Err(error.to_owned()))),
                },
            )
        };
        let network_attribution_for = |parser_position| {
            MainParserDeferredClassicSourceNetworkAttribution::new(
                Url::parse("https://stale-defer.test/document").unwrap(),
                Url::parse(&format!(
                    "https://stale-defer.test/script-{parser_position}.js"
                ))
                .unwrap(),
            )
        };
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::main_parser_deferred_classic_source(
                root_document,
                completion_for(stale_owner, 1, 9, Some("stale defer request failed")),
                network_attribution_for(1),
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::main_parser_deferred_classic_source(
                root_document,
                completion_for(later_stale_owner, 2, 10, None),
                network_attribution_for(2),
            ),
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale completion arbitration should succeed")
            .expect("typed stale completion must remain runnable beside blocked WebSocket work");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::MainParserDeferredClassicSource,
                owner: RendererPageResourceCompletionOwner::main_document(
                    root_document,
                    stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: Some(RendererPageResourceCompletionOwner::main_document(
                        root_document,
                        current_owner,
                    )),
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
            }
        );

        assert!(queue.has_ready_completion());

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("second stale completion arbitration should succeed")
            .expect("second stale completion should consume its own turn");
        assert_eq!(
            second.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::MainParserDeferredClassicSource,
                owner: RendererPageResourceCompletionOwner::main_document(
                    root_document,
                    later_stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: Some(RendererPageResourceCompletionOwner::main_document(
                        root_document,
                        current_owner,
                    )),
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::None,
            }
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "stale defer Network output must not become current Document activity"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        assert_eq!(
            network_records[0].document_url().as_str(),
            "https://stale-defer.test/document"
        );
        assert_eq!(
            network_records[0].url().as_str(),
            "https://stale-defer.test/script-1.js"
        );
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale defer request failed".to_owned(),
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleDeferRan)")
                .expect("replacement Document should remain observable"),
            "undefined",
            "stale completion must not execute or rediscover the current Document"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stale_child_classic_completion_preserves_network_without_document_activity() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(401);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(4),
            crate::frame_owner_model::LocalWindowId(5),
            DocumentId(6),
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            ChildClassicScriptLoadCompletion {
                owner: stale_owner,
                load_id: 7,
                handle: child_handle,
                script_handle: crate::dom::native::NativeNodeId::new(402),
                result: Ok("globalThis.__staleChildClassicRan = true".to_owned()),
                network_result: Some(Arc::new(Err(
                    "stale child classic request failed".to_owned()
                ))),
                network_attribution: ChildClassicScriptNetworkAttribution {
                    frame_id: Some("stale-child-classic-frame".to_owned()),
                    document_url: Url::parse("https://stale-child.test/classic-document").unwrap(),
                    request_url: Url::parse("https://stale-child.test/classic.js").unwrap(),
                },
            },
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale child classic arbitration should succeed")
            .expect("stale child classic completion should consume one bounded turn");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::ChildClassicScript,
                owner: RendererPageResourceCompletionOwner::child_document(
                    root_document,
                    child_handle,
                    stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: None,
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
            }
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "a retired child classic request must not become current Document activity"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        let network_record = &network_records[0];
        assert_eq!(network_record.frame_id(), Some("stale-child-classic-frame"));
        assert_eq!(
            network_record.document_url().as_str(),
            "https://stale-child.test/classic-document"
        );
        assert_eq!(
            network_record.url().as_str(),
            "https://stale-child.test/classic.js"
        );
        assert_eq!(
            network_record.resource_type(),
            SubresourceResourceType::Script
        );
        assert_eq!(
            network_record.request_initiator_type(),
            SubresourceRequestInitiatorType::Script
        );
        assert_eq!(
            network_record.outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child classic request failed".to_owned(),
            }
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__staleChildClassicRan)")
                .expect("current main Document should remain observable"),
            "undefined"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn child_classic_completion_queue_runs_exactly_one_terminal_per_owner_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(411);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(12),
            crate::frame_owner_model::LocalWindowId(13),
            DocumentId(14),
        );
        let completion = |load_id, script_handle| ChildClassicScriptLoadCompletion {
            owner: stale_owner,
            load_id,
            handle: child_handle,
            script_handle: crate::dom::native::NativeNodeId::new(script_handle),
            result: Ok(format!("globalThis.__staleChildClassic{load_id} = true")),
            network_result: None,
            network_attribution: ChildClassicScriptNetworkAttribution {
                frame_id: Some("stale-child-classic-turn-frame".to_owned()),
                document_url: Url::parse("https://stale-child-turn.test/document").unwrap(),
                request_url: Url::parse(&format!(
                    "https://stale-child-turn.test/script-{load_id}.js"
                ))
                .unwrap(),
            },
        };
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            completion(1, 412),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_classic_script(
            root_document,
            completion(2, 413),
        ));

        let first = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("first child classic turn should arbitrate")
            .expect("first child classic terminal should be consumed");
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: None,
            }
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::None,
            "a stale terminal without Network output must not synthesize an output wake"
        );

        assert!(
            queue.has_ready_completion(),
            "one owner turn must not drain the second typed terminal"
        );

        let second = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("second child classic turn should arbitrate")
            .expect("second child classic terminal should be consumed");
        assert_eq!(
            second.action.output_effect,
            PageResourceCompletionOutputEffect::None
        );

        assert!(!queue.has_ready_completion());
        assert!(page_vm.vm_mut().take_network_output().is_empty());
        for load_id in [1, 2] {
            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval(&format!("String(globalThis.__staleChildClassic{load_id})"))
                    .expect("current Document should remain observable"),
                "undefined"
            );
        }
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stale_child_module_completions_preserve_network_and_run_one_terminal_per_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm.vm_mut().eval(
            "const staleModuleFrame = document.createElement('iframe'); \
             staleModuleFrame.id = 'stale-module-frame'; \
             document.body.appendChild(staleModuleFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("stale-module-frame")
            .expect("same-Page stale module fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "stale-module-frame")?;
        let current_child_owner = page_vm
            .vm()
            .current_child_document_task_owner(child_handle)
            .expect("same-Page stale module fixture should install a child owner");
        let current_module_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("same-Page stale module fixture should install a child realm");
        assert_eq!(current_module_target.task_owner(), current_child_owner);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(
                current_child_owner.scheduler_lane_id.0 + 1,
            ),
            current_child_owner.local_window_id,
            current_child_owner.document_id,
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_parser_module_root_fetch(
                root_document,
                test_child_parser_module_root_completion(
                    child_handle,
                    stale_owner,
                    47,
                    "stale-child-module-root",
                    Some("stale child module root failed"),
                ),
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_module_dependency_fetch(
                root_document,
                test_child_module_dependency_completion(
                    child_handle,
                    stale_owner,
                    53,
                    "stale-child-module-dependency",
                    Some("stale child module dependency failed"),
                ),
            ),
        );
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_modulepreload_fetch(
            root_document,
            test_child_modulepreload_completion_for_target(
                ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
                57,
                "stale-child-modulepreload",
                Some("stale child modulepreload failed"),
            ),
        ));
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_modulepreload_fetch(
            root_document,
            test_child_modulepreload_completion_for_target(
                ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
                59,
                "stale-child-module-no-network",
                None,
            ),
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let expected_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            ChildDocumentModuleFetchTarget::new(child_handle, stale_owner, FrameRealmId(109)),
        );
        let expected_current_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            current_module_target,
        );

        let root = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale root module terminal should consume one owner turn");
        assert_eq!(root.action.owner, expected_owner);
        assert_eq!(
            root.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            root.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let dependency = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale dependency module terminal should consume its own owner turn");
        assert_eq!(dependency.action.owner, expected_owner);
        assert_eq!(
            dependency.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            dependency.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let modulepreload = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale modulepreload terminal should consume its own owner turn");
        assert_eq!(modulepreload.action.owner, expected_owner);
        assert_eq!(
            modulepreload.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            modulepreload.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(queue.has_ready_completion());

        let no_network = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("stale no-Network module terminal should consume its own owner turn");
        assert_eq!(no_network.action.owner, expected_owner);
        assert_eq!(
            no_network.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            no_network.action.output_effect,
            PageResourceCompletionOutputEffect::None,
            "a stale module terminal without a Network fact must not synthesize output"
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "retired child module Network facts must not become current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 3);
        assert_eq!(
            network_records
                .iter()
                .map(|record| record.frame_id())
                .collect::<Vec<_>>(),
            vec![
                Some("stale-child-module-root-frame"),
                Some("stale-child-module-dependency-frame"),
                Some("stale-child-modulepreload-frame"),
            ]
        );
        assert!(network_records.iter().all(|record| {
            record.resource_type() == SubresourceResourceType::Script
                && record.request_initiator_type() == SubresourceRequestInitiatorType::Parser
        }));
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child module root failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[1].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child module dependency failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[2].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "stale child modulepreload failed".to_owned(),
            }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("stale child module completion test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_module_completion_rejects_replaced_realm_with_same_document_owner() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const realmReplacementFrame = document.createElement('iframe'); \
             realmReplacementFrame.id = 'realm-replacement-frame'; \
             document.body.appendChild(realmReplacementFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("realm-replacement-frame")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "realm-replacement-frame",
        )?;
        let retired_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("first child realm should have an exact module target");
        let root_completion = test_child_parser_module_root_completion_for_target(
            retired_target,
            61,
            "stale-child-module-root-realm",
            Some("retired child realm root fetch failed"),
        );
        let dependency_completion = test_child_module_dependency_completion_for_target(
            retired_target,
            67,
            "stale-child-module-dependency-realm",
            Some("retired child realm dependency fetch failed"),
        );
        let modulepreload_completion = test_child_modulepreload_completion_for_target(
            retired_target,
            71,
            "stale-child-modulepreload-realm",
            Some("retired child realm modulepreload failed"),
        );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "realm-replacement-frame",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("replacement child realm should have an exact module target");
        assert_eq!(current_target.child_handle(), retired_target.child_handle());
        assert_eq!(current_target.task_owner(), retired_target.task_owner());
        assert_ne!(
            current_target.realm_id(),
            retired_target.realm_id(),
            "realm replacement must preserve the Document owner while changing execution identity"
        );

        let root_document = page_vm.document_lifecycle.identity().document;
        let expected_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            retired_target,
        );
        let expected_current_owner = RendererPageResourceCompletionOwner::child_module_fetch(
            root_document,
            current_target,
        );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_parser_module_root_fetch(
                root_document,
                root_completion,
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_module_dependency_fetch(
                root_document,
                dependency_completion,
            ),
        );
        queue.enqueue_local_for_test(
            RendererPageResourceCompletion::child_modulepreload_fetch(
                root_document,
                modulepreload_completion,
            ),
        );

        let root_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm terminal should consume exactly one owner turn");
        assert_eq!(root_outcome.action.owner, expected_owner);
        assert_eq!(
            root_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            root_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        let dependency_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm dependency must consume a second owner turn");
        assert_eq!(dependency_outcome.action.owner, expected_owner);
        assert_eq!(
            dependency_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            dependency_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        let modulepreload_outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
            .expect("retired-realm modulepreload must consume a third owner turn");
        assert_eq!(modulepreload_outcome.action.owner, expected_owner);
        assert_eq!(
            modulepreload_outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(expected_current_owner),
            }
        );
        assert_eq!(
            modulepreload_outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "historical Network output from a retired realm must not become current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 3);
        assert_eq!(
            network_records[0].frame_id(),
            Some("stale-child-module-root-realm-frame")
        );
        assert_eq!(
            network_records[0].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm root fetch failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[1].frame_id(),
            Some("stale-child-module-dependency-realm-frame")
        );
        assert_eq!(
            network_records[1].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm dependency fetch failed".to_owned(),
            }
        );
        assert_eq!(
            network_records[2].frame_id(),
            Some("stale-child-modulepreload-realm-frame")
        );
        assert_eq!(
            network_records[2].outcome(),
            &SubresourceNetworkOutcome::Failure {
                error_text: "retired child realm modulepreload failed".to_owned(),
            }
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Document stale child realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_start_rejects_replaced_realm_with_same_document_owner() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'modulepreload-start-realm-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("modulepreload-start-realm-replacement")
            .expect("realm replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-realm-replacement",
        )?;
        let retired_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("first child realm should have an exact module target");
        let root_document = page_vm.document_lifecycle.identity().document;
        page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .enqueue_local_for_test(
                root_document,
                test_child_modulepreload_start_task_for_target(
                    retired_target,
                    "retired-modulepreload-start-realm",
                ),
            );

        page_vm
            .vm_mut()
            .retire_child_frame_realm_for_test(child_handle);
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-realm-replacement",
        )?;
        let current_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("replacement child realm should have an exact module target");
        assert_eq!(current_target.task_owner(), retired_target.task_owner());
        assert_ne!(current_target.realm_id(), retired_target.realm_id());

        let outcome = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("stale-realm start should consume exactly one bounded turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, retired_target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageModulepreloadStartOwner::new(
                    root_document,
                    current_target,
                )),
            }
        );

        assert!(
            !page_vm
                .page_task_executor_sources_for_test()
                .modulepreload_start()
                .has_ready_task(),
            "a stale typed start must consume only its stable source head"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("same-Document stale modulepreload start realm test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn child_modulepreload_start_rejects_reused_local_owner_from_retired_root_document() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const frame = document.createElement('iframe'); \
             frame.id = 'modulepreload-start-root-replacement'; \
             document.body.appendChild(frame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("modulepreload-start-root-replacement")
            .expect("root replacement fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "modulepreload-start-root-replacement",
        )?;
        let reused_local_target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("current PageVm should expose an exact child target");
        let current_root = page_vm.document_lifecycle.identity().document;
        let retired_root = current_root.successor_for_testing();
        page_vm.page_task_executor_sources_for_test().modulepreload_start()
            .enqueue_local_for_test(
                retired_root,
                test_child_modulepreload_start_task_for_target(
                    reused_local_target,
                    "retired-root-modulepreload-start",
                ),
            );

        let outcome = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("retired-root start should consume one stale turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageModulepreloadStartOwner::new(retired_root, reused_local_target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: Some(RendererPageModulepreloadStartOwner::new(
                    current_root,
                    reused_local_target,
                )),
            },
            "PageVm-local owner counters may collide after replacement, so the root token must remain part of authorization"
        );

        assert!(
            !page_vm.page_task_executor_sources_for_test().modulepreload_start()
                .has_ready_task(),
            "a root-stale typed start must consume only its stable source head"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("retired-root modulepreload start test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn modulepreload_start_source_consumes_one_exact_owner_per_turn_in_fifo_order() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let first_target = ChildDocumentModuleFetchTarget::new(
            crate::dom::native::NativeNodeId::new(701),
            FrameDocumentTaskOwner::new(
                crate::frame_owner_model::FrameSchedulerLaneId(703),
                crate::frame_owner_model::LocalWindowId(709),
                DocumentId(719),
            ),
            FrameRealmId(727),
        );
        let second_target = ChildDocumentModuleFetchTarget::new(
            crate::dom::native::NativeNodeId::new(733),
            FrameDocumentTaskOwner::new(
                crate::frame_owner_model::FrameSchedulerLaneId(739),
                crate::frame_owner_model::LocalWindowId(743),
                DocumentId(751),
            ),
            FrameRealmId(757),
        );
        let source = page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start();
        source.enqueue_local_for_test(
            root_document,
            test_child_modulepreload_start_task_for_target(first_target, "first-start-turn"),
        );
        source.enqueue_local_for_test(
            root_document,
            test_child_modulepreload_start_task_for_target(second_target, "second-start-turn"),
        );

        let first = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("first start should consume one turn");
        assert_eq!(
            first.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, first_target)
        );
        assert!(matches!(
            first.action.document_effect,
            PageModulepreloadStartDocumentEffect::DiscardedStaleOwner {
                current_owner: None
            }
        ));

        let second = page_vm
            .run_page_modulepreload_start_body_for_test()
            .expect("second start should consume a separate turn");
        assert_eq!(
            second.action.owner,
            RendererPageModulepreloadStartOwner::new(root_document, second_target)
        );

        assert!(!source.has_ready_task());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("modulepreload start FIFO test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn production_child_module_route_reaches_exact_realm_page_turn() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            "const productionModuleFrame = document.createElement('iframe'); \
             productionModuleFrame.id = 'production-module-frame'; \
             document.body.appendChild(productionModuleFrame);",
        )?;
        let child_handle = page_vm
            .vm()
            .element_handle_by_id_for_test("production-module-frame")
            .expect("production route fixture should install a child handle");
        materialize_child_realm_through_page_turn_for_test(
            &mut page_vm,
            "production-module-frame",
        )?;
        let target = page_vm
            .vm()
            .current_child_document_module_fetch_target(child_handle)
            .expect("production route fixture should expose an exact module target");
        let root_document = page_vm.document_lifecycle.identity().document;

        let (wake_tx, mut wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let token = crate::runtime::RendererPageToken::new_for_testing(root_document.page_id);
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(wake_tx, token);
        let mut page_queue = RendererPageNetworkingSource::new_owner_attached(
            crate::page_task_queue::PageRuntimeWakeSignal::default(),
            owner_wake.clone(),
        );
        let sender =
            crate::page_task_queue::RendererResourceCompletionSender::for_page_resource_test(
                page_queue.sender(),
                root_document,
            );
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
        sender
            .send_child_parser_module_root_fetch(
                test_child_parser_module_root_completion_for_target(
                    target,
                    67,
                    "production-child-module-route",
                    Some("production routed module fetch failed"),
                ),
            )
            .expect("production sender should enqueue the typed child module terminal");

        assert_eq!(
            wake_rx
                .try_recv()
                .expect("accepted production terminal should publish one Page wake")
                .page_id(),
            root_document.page_id
        );
        assert!(
            wake_rx.try_recv().is_err(),
            "one accepted terminal must not publish a duplicate wake"
        );

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_queue)?
            .expect("production-routed terminal should execute through the typed Page turn");
        assert_eq!(
            outcome.action.owner,
            RendererPageResourceCompletionOwner::child_module_fetch(root_document, target)
        );
        assert_eq!(
            outcome.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            outcome.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert!(!page_queue.has_ready_completion());
        assert!(
            page_vm.vm().subresource_activity_epoch() > activity_epoch_before,
            "current-target Network output must be attributed to current Document activity"
        );

        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(network_records.len(), 1);
        assert_eq!(
            network_records[0].frame_id(),
            Some("production-child-module-route-frame")
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("production child module stable-route test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn stale_child_unusable_stylesheet_preserves_physical_output_without_document_effect() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let root_document = page_vm.document_lifecycle.identity().document;
        let child_handle = crate::dom::native::NativeNodeId::new(404);
        let stale_owner = FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(7),
            crate::frame_owner_model::LocalWindowId(8),
            DocumentId(9),
        );
        let request_url = Url::parse("https://stale-child.test/style.css").unwrap();
        let physical_response = crate::protocol_types::NavigationResponse::from_text_body(
            request_url.clone(),
            200,
            vec![("Content-Type".to_owned(), "text/html".to_owned())],
            "<html>not a stylesheet</html>".to_owned(),
        );
        let mut queue = RendererPageNetworkingSource::new_for_test();
        queue.enqueue_local_for_test(RendererPageResourceCompletion::child_blocking_stylesheet(
            root_document,
            ChildBlockingStylesheetLoadCompletion {
                child_handle,
                owner: stale_owner,
                signature: crate::DocumentBlockingStylesheetSignature::ParserCreatedStyleImport {
                    urls: Vec::new(),
                },
                network_results: vec![ChildBlockingStylesheetNetworkResult {
                    frame_id: Some("stale-child-frame".to_owned()),
                    document_url: Url::parse("https://stale-child.test/document").unwrap(),
                    request_url,
                    initiator_type: SubresourceRequestInitiatorType::Parser,
                    terminal:
                        crate::stylesheet_blocking::StylesheetFetchTerminal::unusable_response(
                            physical_response,
                            false,
                            "stylesheet MIME validation failed",
                        ),
                }],
            },
        ));
        let activity_epoch_before = page_vm.vm().subresource_activity_epoch();

        let outcome = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)
            .expect("stale child completion arbitration should succeed")
            .expect("stale child completion should consume one bounded turn");
        assert_eq!(
            outcome.action,
            PageResourceCompletionTurnAction {
                source: RendererOwnerResourceActivitySource::ChildBlockingStylesheet,
                owner: RendererPageResourceCompletionOwner::child_document(
                    root_document,
                    child_handle,
                    stale_owner,
                ),
                document_effect: PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: None,
                },
                body_activity: PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                post_checkpoint_effect: PageResourceCompletionPostCheckpointEffect::None,
                output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
            }
        );

        assert!(!queue.has_ready_completion());
        assert_eq!(
            page_vm.vm().subresource_activity_epoch(),
            activity_epoch_before,
            "a retired Document's Network fact must not become activity of the current Document"
        );
        let (network_records, websocket_events, websocket_lifecycle_events) =
            split_network_output_items(page_vm.vm_mut().take_network_output());
        assert!(websocket_events.is_empty());
        assert!(websocket_lifecycle_events.is_empty());
        assert_eq!(
            network_records.len(),
            1,
            "stale child Document must not suppress or duplicate the completed Network fact"
        );
        let network_record = &network_records[0];
        assert_eq!(network_record.frame_id(), Some("stale-child-frame"));
        assert_eq!(
            network_record.document_url().as_str(),
            "https://stale-child.test/document"
        );
        assert_eq!(
            network_record.url().as_str(),
            "https://stale-child.test/style.css"
        );
        assert_eq!(
            network_record.resource_type(),
            SubresourceResourceType::Stylesheet
        );
        assert_eq!(
            network_record.request_initiator_type(),
            SubresourceRequestInitiatorType::Parser
        );
        assert!(matches!(
            network_record.outcome(),
            SubresourceNetworkOutcome::Success { status: 200, .. }
        ));
    })
    .await;
}

pub(super) fn test_page_vm() -> PageVm {
    test_page_vm_with_config(FetchConfig::default(), Vec::new())
}

async fn park_current_document_websocket_for_test(
    page_vm: &mut PageVm,
    event: moli_websocket::Event,
) {
    assert!(
        page_vm
            .vm()
            .websocket_sender_for_test()
            .event_sender()
            .send(event)
            .await,
        "test WebSocket ingress should retain its typed Page route"
    );
    let task = page_vm
        .page_task_executor_sources_for_test()
        .take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::WebSocket { .. }
            )
        })
        .expect("test WebSocket ingress should publish one typed source head");
    let crate::page_task_queue::RendererPageSchedulerTask::WebSocket(task) = task else {
        panic!("WebSocket descriptor dequeued a different task variant")
    };
    task.return_backpressured();
    assert!(
        !page_vm.has_ready_page_websocket_task_for_test(),
        "current-Document backpressure must not remain runnable"
    );
}

fn run_next_resource_completion_as_typed_page_turn(
    page_vm: &mut PageVm,
) -> anyhow::Result<PageResourceCompletionTurnOutcome> {
    let mut page_resource_queue = page_vm.page_resource_completion_queue();
    page_vm
        .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
        .ok_or_else(|| anyhow::anyhow!("typed resource completion should consume one owner turn"))
}

async fn wait_for_typed_page_resource_completion(page_vm: &mut PageVm) -> bool {
    page_vm.wait_for_page_resource_completion_for_test().await
}

fn test_child_module_network_attribution(label: &str) -> ChildModuleFetchNetworkAttribution {
    ChildModuleFetchNetworkAttribution::parser(
        Some(format!("{label}-frame")),
        Url::parse(&format!("https://{label}.test/document")).unwrap(),
        Url::parse(&format!("https://{label}.test/module.js")).unwrap(),
    )
}

fn test_child_dynamic_import_network_attribution(
    label: &str,
) -> ChildModuleFetchNetworkAttribution {
    ChildModuleFetchNetworkAttribution::dynamic_import(
        Some(format!("{label}-frame")),
        Url::parse(&format!("https://{label}.test/document")).unwrap(),
        Url::parse(&format!("https://{label}.test/module.js")).unwrap(),
    )
}

fn test_child_parser_module_root_completion(
    child_handle: crate::dom::native::NativeNodeId,
    owner: FrameDocumentTaskOwner,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildParserModuleRootFetchCompletion {
    test_child_parser_module_root_completion_for_target(
        ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(109)),
        request_id,
        label,
        network_error,
    )
}

fn test_child_parser_module_root_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildParserModuleRootFetchCompletion {
    let network_attribution = test_child_module_network_attribution(label);
    ChildParserModuleRootFetchCompletion::new(
        target,
        FrameRequestId(request_id),
        ModuleMapKey::java_script(network_attribution.request_url().clone()),
        Err(format!("{label} root completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        network_attribution,
    )
}

fn test_child_module_dependency_completion(
    child_handle: crate::dom::native::NativeNodeId,
    owner: FrameDocumentTaskOwner,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModuleDependencyFetchCompletion {
    test_child_module_dependency_completion_for_target(
        ChildDocumentModuleFetchTarget::new(child_handle, owner, FrameRealmId(109)),
        request_id,
        label,
        network_error,
    )
}

fn test_child_module_dependency_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModuleDependencyFetchCompletion {
    let network_attribution = test_child_module_network_attribution(label);
    let task = test_child_module_dependency_fetch_task_for_target(target, request_id, label);
    ChildModuleDependencyFetchCompletion::new(
        target.child_handle(),
        FrameRequestId(request_id),
        task,
        Err(format!("{label} dependency completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        network_attribution,
    )
}

fn test_child_module_dependency_fetch_task_for_target(
    target: ChildDocumentModuleFetchTarget,
    request_id: u64,
    label: &str,
) -> FrameDocumentModuleDependencyFetchTask {
    let parent_url = Url::parse(&format!("https://{label}.test/root.js")).unwrap();
    let dependency_url = Url::parse(&format!("https://{label}.test/module.js")).unwrap();
    let parent_key = ModuleMapKey::java_script(parent_url.clone());
    let dependency_key = ModuleMapKey::java_script(dependency_url.clone());
    let parent_entry_id = ModuleEntryId::from_raw(113);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(127),
        sequence: request_id,
    };
    let client = FrameDocumentStaticDependencyModuleClient::new(
        parent_entry_id,
        parent_key.clone(),
        "./module.js".to_owned(),
        ModuleImportPhase::Evaluation,
        tree_client,
    );
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(131);
    let reservation = FrameDocumentModuleClientReservation::new(
        target.task_owner().document_owner(),
        dependency_key.clone(),
        FrameDocumentModuleClientRegistration::new(
            entry_id,
            FrameDocumentModuleClientId::from_raw(request_id),
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
        ),
    );
    FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
        target.task_owner(),
        target.realm_id(),
        dependency_key.clone(),
        client,
        reservation,
        NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
            dependency_url,
            parent_url,
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
            tree_client,
            dependency_key,
            parent_key,
            parent_entry_id,
            "./module.js".to_owned(),
            ModuleImportPhase::Evaluation,
        ),
    )
}

fn test_child_modulepreload_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    load_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildModulepreloadFetchCompletion {
    ChildModulepreloadFetchCompletion::new(
        target,
        load_id,
        Err(format!("{label} modulepreload completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        test_child_module_network_attribution(label),
    )
}

fn test_child_dynamic_import_completion_for_target(
    target: ChildDocumentModuleFetchTarget,
    load_id: u64,
    label: &str,
    network_error: Option<&str>,
) -> ChildDynamicImportFetchCompletion {
    ChildDynamicImportFetchCompletion::new(
        target,
        load_id,
        Err(format!("{label} dynamic import completion")),
        network_error.map(|error| Arc::new(Err(error.to_owned()))),
        test_child_dynamic_import_network_attribution(label),
    )
}

fn test_child_modulepreload_start_task_for_target(
    target: ChildDocumentModuleFetchTarget,
    label: &str,
) -> FrameDocumentModulepreloadFetchTask {
    let source_url = Url::parse(&format!("https://{label}.test/modulepreload.js"))
        .expect("modulepreload start URL");
    FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        target.realm_id(),
        FrameDocumentModulepreloadLinkClient::new(
            target.child_handle(),
            target.task_owner(),
            crate::dom::native::NativeNodeId::new(997),
        ),
        NativeModuleSingleFetchRequest::new(
            source_url.clone(),
            source_url.clone(),
            source_url.clone(),
            ModuleMapKey::java_script(source_url),
            ModuleFetchMetadata::default(),
        ),
    )
}

fn test_page_vm_with_config(
    config: FetchConfig,
    extra_http_headers: Vec<(String, String)>,
) -> PageVm {
    let loader_owner = crate::network::ResourceRequestClient::new(&config).expect("loader");
    let mut page_vm = test_page_vm_with_loader(&loader_owner, extra_http_headers);
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_loader(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
) -> PageVm {
    test_page_vm_with_loader_and_document_url(
        loader,
        extra_http_headers,
        Url::parse("https://example.com/").unwrap(),
    )
}

fn test_page_vm_with_document_url(document_url: Url) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm =
        test_page_vm_with_loader_and_document_url(&loader_owner, Vec::new(), document_url);
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_root_frame_id(root_frame_id: &str) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let dom_host = DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
        Url::parse("https://example.com/").expect("test Document URL"),
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    ));
    let mut page_vm = test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        &loader_owner,
        Vec::new(),
        dom_host,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        None,
        Some(root_frame_id.to_owned()),
    );
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn test_page_vm_with_response_referrer_policy(
    document_url: Url,
    response_referrer_policy: impl Into<String>,
) -> PageVm {
    let loader_owner =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut page_vm = test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
        &loader_owner,
        Vec::new(),
        document_url,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        Some(response_referrer_policy.into()),
    );
    page_vm.retain_standalone_request_client_owner_for_test(loader_owner);
    page_vm
}

fn dns_failure_fetch_config() -> FetchConfig {
    let mut config = FetchConfig::default();
    config.set_http_no_proxy(Some("*".to_owned()));
    config.set_request_timeout_ms(2_000);
    config.set_connect_timeout_ms(Some(500));
    config
}

fn test_page_vm_with_loader_and_document_url(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
) -> PageVm {
    test_page_vm_with_loader_document_url_and_hooks(
        loader,
        extra_http_headers,
        document_url,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
    )
}

fn test_page_vm_with_loader_document_url_and_hooks(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
    runtime_hooks: PageVmRuntimeHooks,
) -> PageVm {
    test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
        loader,
        extra_http_headers,
        document_url,
        runtime_hooks,
        None,
    )
}

/// Builds a low-level PageVm executor fixture with the production Page task
/// sources and wake route bound.
///
/// It deliberately has no `RendererOwnerLocalPageSlot` or scheduler
/// residence. Tests using it may prove producer routing, exact-owner
/// arbitration, and one-task execution, but not owner admission, fairness, or
/// autonomous liveness.
fn page_vm_with_bound_task_sources_and_owner_wake(
    loader: &crate::network::ResourceRequestClient,
    document_url: Url,
) -> (
    PageVm,
    crate::page_task_queue::RendererPageResourceCompletionTestSource,
    tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let (wake_tx, wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
    );
    let hooks = PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
        owner_wake,
    );
    let page_vm =
        test_page_vm_with_loader_document_url_and_hooks(loader, Vec::new(), document_url, hooks);
    let queue = page_vm.page_resource_completion_queue();
    (page_vm, queue, wake_rx)
}

/// Materialize the only child fixture through the production producer,
/// stable source, exact-owner arbiter, and one-turn executor.
fn materialize_child_realm_through_page_turn_for_test(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<crate::page_task_queue::PageChildRealmMaterializationTurnOutcome> {
    let child_handle = page_vm
        .vm()
        .element_handle_by_id_for_test(element_id)
        .expect("child realm fixture should retain its iframe handle");
    page_vm.vm_mut().eval(&format!(
        "void document.getElementById({element_id:?}).contentWindow.Function; 'queued'"
    ))?;
    let outcome = page_vm
        .run_child_realm_materialization_body_for_test()?
        .expect("child Window exposure should enqueue one typed realm turn");
    assert_eq!(
        outcome.action.owner.target().child_handle(),
        Some(child_handle)
    );
    assert_eq!(
        outcome.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
    );
    Ok(outcome)
}

fn run_expected_pending_child_realm_materialization_turn(
    page_vm: &mut PageVm,
    label: &str,
) -> anyhow::Result<crate::page_task_queue::PageChildRealmMaterializationTurnOutcome> {
    let outcome = page_vm
        .run_child_realm_materialization_body_for_test()?
        .unwrap_or_else(|| panic!("{label} should consume one typed child-realm turn"));
    assert_eq!(
        outcome.action.target_effect,
        crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript,
        "{label} must materialize the exact current child Document"
    );
    Ok(outcome)
}

fn materialize_only_child_realm_execution_context_through_page_turn_for_test(
    page_vm: &mut PageVm,
    element_id: &str,
) -> anyhow::Result<i64> {
    let _ = materialize_child_realm_through_page_turn_for_test(page_vm, element_id)?;
    let realms = page_vm
        .vm_mut()
        .live_child_default_runtime_realm_inventory();
    assert_eq!(
        realms.len(),
        1,
        "single-child PageVm fixture should expose exactly one live child realm"
    );
    Ok(realms[0].context_id)
}

fn test_page_vm_with_loader_document_url_hooks_and_response_referrer_policy(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    document_url: Url,
    runtime_hooks: PageVmRuntimeHooks,
    response_referrer_policy: Option<String>,
) -> PageVm {
    let dom_host = DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
        document_url,
        "<!doctype html><html><head></head><body></body></html>".to_owned(),
    ));
    test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        loader,
        extra_http_headers,
        dom_host,
        runtime_hooks,
        response_referrer_policy,
        None,
    )
}

fn test_page_vm_with_loader_and_dom_host(
    loader: &crate::network::ResourceRequestClient,
    dom_host: DomHost,
) -> PageVm {
    test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
        loader,
        Vec::new(),
        dom_host,
        PageVmRuntimeHooks::standalone_without_owner_reservation_for_test(),
        None,
        None,
    )
}

fn test_page_vm_with_loader_dom_host_hooks_and_response_referrer_policy(
    loader: &crate::network::ResourceRequestClient,
    extra_http_headers: Vec<(String, String)>,
    dom_host: DomHost,
    runtime_hooks: PageVmRuntimeHooks,
    response_referrer_policy: Option<String>,
    root_frame_id: Option<String>,
) -> PageVm {
    let _js_runtime = crate::JsRuntime::initialize();
    let local_executor = crate::local_executor::JsLocalExecutor::new();
    PageVm::new(
        PageId::new_for_testing(1),
        local_executor,
        loader,
        &PageVmEnvConfig {
            web_storage: crate::RendererWebStorageHandles::ephemeral(),
            root_frame_id,
            main_document_commit: None,
            top_level_storage_key: None,
            document_start_scripts: vec![],
            runtime_bindings: vec![],
            runtime_inspector_session_restore_snapshots: vec![],
            runtime_isolated_worlds: vec![],
            permission_overrides: vec![],
            extra_http_headers,
            document_policy_container: crate::document_runtime::DocumentPolicyContainer {
                referrer_policy: response_referrer_policy,
                ..Default::default()
            },
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
            layout_policy: crate::real_layout_test_policy(),
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        },
        runtime_hooks,
        dom_host,
        Instant::now(),
    )
    .expect("page vm")
}

fn prepared_external_module_for_page_vm_test(page_vm: &PageVm, url: Url) -> PreparedScript {
    prepared_external_module_for_page_vm_test_with_node(page_vm, 9001, url)
}

fn prepared_external_module_for_page_vm_test_with_node(
    page_vm: &PageVm,
    node: u32,
    url: Url,
) -> PreparedScript {
    PreparedScript {
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleDefer,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::External,
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: url.clone(),
        url,
        host_script_handle: None,
    }
}

async fn drive_child_frame_task_sources_until_resource_completion_ready(
    page_vm: &mut PageVm,
    max_turns: usize,
) -> Vec<ChildFrameSemanticTurnKind> {
    let mut sources = Vec::new();
    for _ in 0..max_turns {
        if page_vm.has_ready_page_websocket_task_for_test() {
            break;
        }
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        sources.push(source);
    }
    sources
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildModulepreloadStartupTurn {
    TypedModulepreloadStart,
    ChildSemanticTurn(ChildFrameSemanticTurnKind),
}

async fn run_expected_child_modulepreload_event_action_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    assert!(
        page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                loader,
            )
            .await
            .unwrap_or_else(|error| panic!("{label} selected task should succeed: {error:#}")),
        "{label} should consume one typed modulepreload event action through the selected-task dispatcher"
    );
}

/// Adjacent module-map helper that preserves distinct modulepreload and child
/// semantic turns. The modulepreload start enters the production selected-task
/// dispatcher; the returned sequence still must not be used as owner-scheduler
/// or cross-source fairness evidence.
async fn drive_child_modulepreload_startup_until_resource_completion_ready(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    max_turns: usize,
) -> Vec<ChildModulepreloadStartupTurn> {
    let mut turns = Vec::new();
    for _ in 0..max_turns {
        if page_vm.has_ready_page_websocket_task_for_test() {
            break;
        }
        if page_vm
            .page_task_executor_sources_for_test()
            .modulepreload_start()
            .has_ready_task()
        {
            assert!(
                page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ModulepreloadStart,
                        loader,
                    )
                    .await
                    .expect("selected child modulepreload start should succeed"),
                "typed child modulepreload start should consume one source head through the production dispatcher",
            );
            turns.push(ChildModulepreloadStartupTurn::TypedModulepreloadStart);
            continue;
        }
        let Some(source) = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
        else {
            break;
        };
        turns.push(ChildModulepreloadStartupTurn::ChildSemanticTurn(source));
    }
    turns
}

/// Settle at most one exact realm-materialization prerequisite before running
/// the requested child-family turn. The production semantic helper remains a
/// strict one-turn operation.
async fn run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
    page_vm: &mut PageVm,
    expected: impl Into<ChildFrameSemanticTurnKind>,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    let expected = expected.into();
    if expected != ChildFrameSemanticTurnKind::RealmMaterialization
        && page_vm.has_ready_child_frame_semantic_turn_for_test(
            ChildFrameSemanticTurnKind::RealmMaterialization,
        )
    {
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "{label} should first materialize the exact child realm"
        );
    }
    let Some(source) = page_vm
        .run_next_child_frame_task_source_for_semantic_test()
        .await
    else {
        panic!("{label} should produce {expected:?}, but no child frame task work was ready");
    };
    assert_eq!(
        source, expected,
        "{label} should run the expected child frame task source"
    );
    source
}

async fn run_expected_child_realm_materialization_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    let source = page_vm
        .run_next_child_frame_task_source_for_semantic_test()
        .await;
    assert_eq!(
        source,
        Some(ChildFrameSemanticTurnKind::RealmMaterialization),
        "{label} should materialize its exact child realm in one explicit turn"
    );
    ChildFrameSemanticTurnKind::RealmMaterialization
}

async fn run_expected_child_module_script_terminal_turn(page_vm: &mut PageVm, label: &str) {
    let loader = page_vm.request_client.clone();
    let claimed = page_vm
        .claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::ChildModuleScriptTerminal,
        )
        .unwrap_or_else(|| panic!("{label} should produce one selected module-terminal task"));
    let owner = claimed
        .child_module_script_terminal_owner()
        .expect("exact terminal selector must retain its typed owner");
    assert_eq!(
        owner.root_document(),
        page_vm.document_lifecycle.identity().document,
        "{label} must belong to the current root Document",
    );
    assert_eq!(
        page_vm
            .vm()
            .current_child_module_script_terminal_owner(owner.document_owner(), owner.realm_id(),),
        Some(owner.document_owner()),
        "{label} must belong to the current child Document/realm",
    );
    page_vm
        .run_claimed_selected_page_task_for_test(claimed, &loader)
        .await
        .unwrap_or_else(|error| panic!("{label} selected task failed: {error:#}"));
}

async fn run_child_domcontentloaded_then_host_load_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        &format!("{label} DOMContentLoaded transition"),
    )
    .await;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        &format!("{label} complete transition"),
    )
    .await;
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::HostLoad,
        label,
    )
    .await
}

async fn run_child_interactive_domcontentloaded_then_host_load_for_wait(
    page_vm: &mut PageVm,
    label: &str,
) -> ChildFrameSemanticTurnKind {
    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
        page_vm,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        &format!("{label} interactive transition"),
    )
    .await;
    run_child_domcontentloaded_then_host_load_for_wait(page_vm, label).await
}

fn prepared_inline_module_for_page_vm_test(
    page_vm: &PageVm,
    node: u32,
    source: &str,
) -> PreparedScript {
    PreparedScript {
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Module,
        mode: ScriptMode::ModuleDefer,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::Inline(source.to_owned()),
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: page_vm.vm().document_runtime.document_url().clone(),
        url: page_vm.vm().document_runtime.document_url().clone(),
        host_script_handle: None,
    }
}

fn prepared_loaded_classic_for_page_vm_test(
    page_vm: &PageVm,
    node: u32,
    source: &str,
) -> PreparedScript {
    PreparedScript {
        position: node as usize,
        node_id: NodeId::new(node as usize),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Normal,
        source_kind: ScriptSourceKind::Inline,
        fetch_metadata: ScriptFetchMetadata::default(),
        source: ScriptSource::Loaded(source.to_owned()),
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: page_vm.vm().document_runtime.document_url().clone(),
        url: page_vm.vm().document_runtime.document_url().clone(),
        host_script_handle: None,
    }
}

fn append_parser_owned_external_classic_defer_for_page_vm_test(
    page_vm: &mut PageVm,
    position: usize,
    element_id: &str,
    script_url: Url,
    source: ScriptSource,
    completion_attribute: (&str, &str),
) -> PreparedScript {
    let body = page_vm
        .vm()
        .snapshot_live_document()
        .document_body_handle()
        .expect("test document body");
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
        assert!(dom_host.set_attribute(script_node, "id", element_id));
        assert!(dom_host.set_attribute(script_node, "src", script_url.as_str()));
        assert!(dom_host.set_attribute(
            script_node,
            completion_attribute.0,
            completion_attribute.1,
        ));
        assert!(dom_host.append_child(body, script_node));
    }
    let host_script_handle = page_vm
        .vm_mut()
        .document_runtime
        .bind_parser_owned_script_handle_for_node(script_node);
    PreparedScript {
        position,
        node_id: NodeId::new(script_node.index()),
        kind: ScriptKind::Classic,
        mode: ScriptMode::Defer,
        source_kind: ScriptSourceKind::External,
        fetch_metadata: ScriptFetchMetadata::default(),
        source,
        initiator_url: page_vm.vm().document_runtime.document_url().clone(),
        base_url: script_url.clone(),
        url: script_url,
        host_script_handle: Some(host_script_handle),
    }
}

fn enqueue_parser_owned_module_script_fetch_completion_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    source: &str,
) {
    let target = page_vm
        .vm()
        .current_main_parser_module_graph_fetch_target(load_id)
        .unwrap_or_else(|| panic!("parser module fetch {load_id} must retain its exact target"));
    enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
        page_vm, target, module_url, source,
    );
}

fn enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
    page_vm: &mut PageVm,
    target: MainParserModuleGraphFetchTarget,
    module_url: &Url,
    source: &str,
) {
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
            target,
            Ok(ModuleGraphFetchedSource::new(
                module_url.clone(),
                false,
                ModuleSource::text(source.to_owned()),
            )),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("module graph completion should enqueue");
}

fn enqueue_parser_owned_module_script_fetch_error_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    message: &str,
) {
    let target = page_vm
        .vm()
        .current_main_parser_module_graph_fetch_target(load_id)
        .unwrap_or_else(|| panic!("parser module fetch {load_id} must retain its exact target"));
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
            target,
            Err(message.to_owned()),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("module graph error completion should enqueue");
}

fn run_next_main_module_fetch_terminal_for_test(
    page_vm: &mut PageVm,
) -> anyhow::Result<Option<RendererOwnerResourceActivitySource>> {
    let outcome = run_next_resource_completion_as_typed_page_turn(page_vm)?;
    anyhow::ensure!(
        outcome.action.source == RendererOwnerResourceActivitySource::ModuleGraphFetch,
        "expected a typed main module fetch terminal, got {:?}",
        outcome.action.source
    );
    Ok(Some(outcome.action.source))
}

async fn run_next_native_module_owner_event_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let outcome = page_vm
        .run_page_main_document_runtime_body_for_test(loader)
        .await
        .unwrap_or_else(|error| panic!("{label} owner event should run: {error}"))
        .unwrap_or_else(|| panic!("{label} should retain one native module owner-event turn"));
    assert_eq!(
        outcome.action.kind(),
        crate::page_task_queue::PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent,
        "{label} must resume joined module-map clients through their typed owner source"
    );
}

fn enqueue_main_modulepreload_fetch_error_for_test(
    page_vm: &mut PageVm,
    load_id: u64,
    module_url: &Url,
    message: &str,
) {
    let target = page_vm
        .vm()
        .current_main_modulepreload_fetch_target(load_id)
        .unwrap_or_else(|| panic!("modulepreload fetch {load_id} must retain its exact target"));
    let document_url = page_vm.vm().document_runtime.document_url().clone();
    page_vm
        .vm()
        .resource_completion_sender_for_test()
        .send_main_modulepreload_fetch(MainModulepreloadFetchCompletion::new(
            target,
            Err(message.to_owned()),
            None,
            MainModuleFetchNetworkAttribution::new(document_url, module_url.clone()),
        ))
        .expect("modulepreload graph completion should enqueue");
}

async fn run_parser_module_completion_turns_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    expected_ready_turns: usize,
    label: &str,
) {
    page_vm
        .vm_mut()
        .perform_script_task_checkpoint(None)
        .expect("module evaluation reaction checkpoint should run");
    while page_vm
        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, loader)
        .await
        .expect("module reactions should run")
    {}
    for turn in 0..expected_ready_turns {
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(page_vm, loader)
                .await
                .expect("module evaluation completion should run"),
            "{label} should run parser-owned ready turn {} of {expected_ready_turns}",
            turn + 1
        );
    }
    assert!(
        !page_vm.has_ready_parser_owned_document_script_action(),
        "{label} should not leave another concrete parser-owned module action ready"
    );
}

async fn run_one_parser_owned_main_document_runtime_turn_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<bool> {
    page_vm.admit_ready_parser_owned_document_script_action();
    page_vm
        .run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::MainDocumentRuntime(
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation,
            ),
            loader,
        )
        .await
}

fn post_parse_document_script_work(
    lane: DocumentScriptExecutionLane,
    script: PreparedScript,
) -> PostParsePageOwnedWork {
    PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::script(lane, script))
}

fn install_parser_module_defer_work(
    page_vm: &mut PageVm,
    script: PreparedScript,
) -> PostParsePageOwnedWork {
    let task_owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("parser module test requires a current document owner");
    assert!(
        page_vm
            .vm_mut()
            .claim_main_parser_deferred_script(task_owner, script, None, None, Default::default(),)
            .expect("parser module PendingScript acceptance should start its graph")
    );
    page_vm
        .seal_main_parser_deferred_scripts(task_owner)
        .expect("parser module PendingScript should install into after-parsing order")
}

/// Execute only the parser-deferred domain body.
///
/// This helper intentionally does not prove selected-task checkpoint or exact
/// DCL handoff behavior. Tests asserting a complete parser task must use
/// `run_and_finish_ready_parser_deferred_task_for_test()` instead.
async fn run_ready_parser_deferred_body_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let action = poll_post_parse_document_processing_action_for_test(page_vm)
        .unwrap_or_else(|| panic!("{label} should expose a ready parser-deferred owner action"));
    let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) = action
    else {
        panic!("{label} should produce page-owned parser-deferred work");
    };
    assert!(
        work.main_parser_deferred_scripts_owner().is_some(),
        "{label} should produce the armed parser-deferred marker, got {work:?}"
    );
    page_vm
        .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
        .await
        .unwrap_or_else(|error| panic!("{label} parser-deferred turn should run: {error}"));
}

async fn run_and_finish_ready_parser_deferred_task_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    label: &str,
) {
    let action = poll_post_parse_document_processing_action_for_test(page_vm)
        .unwrap_or_else(|| panic!("{label} should expose a ready parser-deferred owner action"));
    let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) = action
    else {
        panic!("{label} should produce page-owned parser-deferred work");
    };
    assert!(
        work.main_parser_deferred_scripts_owner().is_some(),
        "{label} should produce the armed parser-deferred marker, got {work:?}"
    );
    let completion = page_vm
        .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
        .await
        .unwrap_or_else(|error| panic!("{label} parser-deferred body should run: {error}"));
    let super::parser_completion::SelectedPostParsePageOwnedCompletion::MainParser(completion) =
        completion
    else {
        panic!("{label} must retain its main-parser completion authority");
    };
    page_vm
        .finish_parse_time_main_parser_boundary(completion)
        .await
        .unwrap_or_else(|error| panic!("{label} parser task completion should run: {error}"));
}

fn poll_post_parse_document_processing_action_for_test(
    page_vm: &mut PageVm,
) -> Option<crate::document_runtime::DocumentProcessingAction> {
    let (vm, task_queue) = (&mut page_vm.vm, &mut page_vm.page_task_queue);
    vm.as_mut()
        .expect("test PageVm should retain ScriptVm")
        .document_runtime
        .poll_document_processing_action(task_queue, Option::<&crate::dom::native::NativeDom>::None)
}

async fn run_main_async_script_load_delay_settlement_for_test(
    page_vm: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
    owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    label: &str,
) {
    for turn in 0..2 {
        if let Some(outcome) = page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await
            .unwrap_or_else(|error| panic!("{label} follow-up turn should run: {error}"))
        {
            assert_eq!(
                outcome.action.kind(),
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork
            );
            assert_eq!(outcome.action.owner().document_owner(), owner);
            assert_eq!(
                outcome.action.target_effect(),
                crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner
            );
        } else {
            // Parse-time async classic work can finish before DCL. Its event
            // and settlement therefore remain lifecycle-owned rather than
            // entering the post-DCL MainDocumentRuntime source.
            let action = page_vm
                .vm_mut()
                .document_runtime
                .pop_parser_owned_pre_domcontentloaded_action()
                .or_else(|| poll_post_parse_document_processing_action_for_test(page_vm))
                .unwrap_or_else(|| panic!("{label} should enqueue an explicit follow-up"));
            let crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work) =
                action
            else {
                panic!("{label} async follow-up must remain page-owned work");
            };
            if let Some(PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(binding)) =
                work.as_lifecycle_work()
            {
                assert_eq!(binding.owner(), owner);
            }
            page_vm
                .execute_post_parse_page_owned_task_on_named_owner_lane(loader, *work)
                .await
                .unwrap_or_else(|error| panic!("{label} lifecycle follow-up should run: {error}"));
        }
        if page_vm
            .vm()
            .current_main_document_has_async_script_load_delay(owner)
            == Some(false)
        {
            return;
        }
        assert_eq!(
            turn, 0,
            "{label} must settle after at most one observable event turn"
        );
    }
    panic!("{label} left its exact main-Document load delay unsettled");
}

fn classic_defer_work(script: PreparedScript) -> PostParsePageOwnedWork {
    post_parse_document_script_work(DocumentScriptExecutionLane::ClassicDefer, script)
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_classic_owns_load_delay_from_discovery_through_settlement() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async.html").expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async test requires an owner");
        let script_url =
            Url::parse("https://example.com/main-parser-async.js").expect("script URL");
        let mut script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            9060,
            "main-parser-async-classic",
            script_url,
            ScriptSource::External,
            ("async", ""),
        );
        script.mode = ScriptMode::Async;
        assert!(
            page_vm
                .vm_mut()
                .document_runtime
                .dom_host_mut()
                .set_script_already_started(script.node_id, true)
        );
        let shared_load =
            SharedScriptSourceLoad::ready_ok("globalThis.__mainParserAsyncClassicExecuted = 1;");
        let mut scheduler = crate::document_script_scheduler::DocumentScriptScheduler::new();
        let resource_task_runner = page_vm.resource_task_runner();

        assert!(scheduler.accept_parser_discovered_async_candidate(
            script.clone(),
            &loader,
            resource_task_runner,
            Some(shared_load),
            None,
            |_| {
                page_vm
                    .vm_mut()
                    .accept_main_document_script_load_delay_binding(
                        owner,
                        crate::frame_owner_model::MainDocumentScriptLoadDelayKind::Classic,
                    )
                    .expect("parser discovery should bind classic lifecycle ownership")
            },
        ));
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "acceptance must delay load before source readiness is observed"
        );
        assert!(scheduler.claim_existing_parse_time_async_handoff(script.node_id));
        let ready = scheduler
            .parse_time_turn(
                crate::document_script_scheduler::ParseTimeTurnTrigger::BeforeParserStep {
                    default_chunk_bytes: 4096,
                },
            )
            .ready_task
            .expect("ready shared source should produce a parser async task");
        let crate::document_script_scheduler::ParseTimeDocumentScriptTask::ClassicAsyncScript(
            ready,
        ) = ready
        else {
            panic!("ready shared source should produce classic execution work");
        };
        let (script, binding) = ready.into_parts();
        let binding = binding.expect("parse-time work must retain its discovery binding");
        assert_eq!(binding.owner(), owner);

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::document_script_work(
                    PageOwnedDocumentScriptWork::parser_async_script(
                        DocumentScriptExecutionLane::ParseTimeAsync,
                        script,
                        Some(binding),
                    ),
                ),
            )
            .await
            .expect("parser async classic execution turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncClassicExecuted)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "execution must enqueue, not inline-apply, lifecycle settlement"
        );
        page_vm
            .drain_deferred_page_tasks_on_named_owner_local_task()
            .await?;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "parser progress may publish follow-ups but must not apply settlement inline"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async classic",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async classic lifecycle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_module_owns_load_delay_until_evaluation_start() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-module.html").expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async module test requires an owner");
        let module_url =
            Url::parse("https://example.com/main-parser-async-module.mjs").expect("module URL");
        let mut script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9061, module_url.clone());
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "module acceptance must bind lifecycle ownership before graph work"
        );
        page_vm.target_stage = PageVmInitStage::DomContentLoaded;
        assert!(
            !page_vm.has_pending_module_script_for_target_stage(),
            "a watched parser-origin async module must not enter the parser-deferred DCL gate"
        );
        assert!(
            page_vm.has_pending_module_fetch_for_target_stage(),
            "the pending graph should remain visible to resource-completion observation"
        );
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__mainParserAsyncModuleExecuted = 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)?.is_some(),
            "async module graph completion should apply"
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "watching a completed async graph should queue owner work"
        );
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader).await?,
            "async module graph-ready work should start evaluation"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncModuleExecuted)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "evaluation start must leave settlement for its own lifecycle turn"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async module",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async module lifecycle test should run");
}

#[test]
fn runtime_module_graph_wait_does_not_block_domcontentloaded_on_load_target() {
    let mut page_vm = test_page_vm();
    page_vm.target_stage = PageVmInitStage::Load;
    let mut script = prepared_inline_module_for_page_vm_test(
        &page_vm,
        9062,
        "import { value } from 'late'; globalThis.__lateValue = value;",
    );
    script.mode = ScriptMode::Async;
    page_vm
        .vm_mut()
        .document_runtime
        .runtime_script_work_mut()
        .dynamic_scripts
        .requeue_failed_script_front(
            DynamicScriptOwnerId::from_u64(1),
            script,
            "failed to resolve module specifier `late`".to_owned(),
            DynamicScriptFailureKind::ModuleResolve,
            Some(ModuleFailurePolicy::GraphFailure),
            None,
            None,
        );

    assert!(
        page_vm
            .vm_mut()
            .has_pending_runtime_owned_module_script_graph(),
        "the terminal graph failure must remain pending until its error owner turn"
    );
    assert!(
        !page_vm.has_pending_module_script_for_target_stage(),
        "runtime async module work must not become a DCL wait merely because the final target is Load"
    );

    page_vm
        .vm_mut()
        .document_runtime
        .note_dom_content_loaded_dispatched();
    assert!(
        page_vm.has_pending_module_script_for_target_stage(),
        "after DCL the same terminal graph work must delay Load until error dispatch settles it"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_inline_module_is_watched_before_graph_start() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-inline-module.html")
                .expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async inline module test requires an owner");
        let mut script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9063,
            "globalThis.__mainParserAsyncInlineModuleExecuted = 1; export const value = 1;",
        );
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        assert!(
            page_vm.has_ready_parser_owned_document_script_action(),
            "an immediately-ready graph must notify the PendingScript watch installed before graph start"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true)
        );
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader)
                .await?
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserAsyncInlineModuleExecuted)")?,
            "1"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async inline module",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async inline module lifecycle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_async_module_graph_failure_settles_exact_load_delay() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-parser-async-module-failure.html")
                .expect("document URL"),
        );
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main parser async module failure test requires an owner");
        let module_url =
            Url::parse("https://example.com/main-parser-async-failure.mjs").expect("module URL");
        let mut script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9062, module_url.clone());
        script.mode = ScriptMode::Async;

        assert!(
            page_vm
                .vm_mut()
                .accept_main_parser_async_module_script(owner, &script)?
        );
        enqueue_parser_owned_module_script_fetch_error_for_test(
            &mut page_vm,
            0,
            &module_url,
            "async module graph failed",
        );
        assert!(run_next_main_module_fetch_terminal_for_test(&mut page_vm)?.is_some());
        assert!(
            run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader).await?,
            "graph failure should run through the watched PendingScript owner"
        );
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(true),
            "failure reporting must be ordered before lifecycle settlement"
        );

        run_main_async_script_load_delay_settlement_for_test(
            &mut page_vm,
            &loader,
            owner,
            "main parser async module graph failure",
        )
        .await;
        assert_eq!(
            page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner),
            Some(false)
        );
        assert!(
            page_vm.report.runs.iter().any(|run| {
                run.url() == &module_url && matches!(run.outcome(), ScriptRunOutcome::Failed(_))
            }),
            "graph failure should remain an observable failed script run"
        );
        Ok::<(), anyhow::Error>(())
    })
    .await
    .expect("main parser async module failure lifecycle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_lifecycle_sets_interactive_before_defer_and_loads_later() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-lifecycle.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__mainLifecycleEvents = [];
                document.addEventListener("readystatechange", () => {
                  __mainLifecycleEvents.push("ready:" + document.readyState);
                });
                document.addEventListener("DOMContentLoaded", () => {
                  __mainLifecycleEvents.push("dcl:" + document.readyState);
                });
                window.addEventListener("load", () => {
                  __mainLifecycleEvents.push("load:" + document.readyState);
                });
                "installed";
                "#,
            )
            .expect("main lifecycle listeners should install");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main lifecycle test requires an owner");
        let defer = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            7001,
            "main-lifecycle-defer",
            Url::parse("https://example.com/main-lifecycle-defer.js").expect("defer script URL"),
            ScriptSource::Loaded(
                "__mainLifecycleEvents.push('defer:' + document.readyState);".to_owned(),
            ),
            ("data-lifecycle-test", "defer"),
        );
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(owner, defer, None, None, Default::default(),)
                .expect("parser-deferred PendingScript acceptance should succeed")
        );
        let defer_marker = page_vm
            .seal_main_parser_deferred_scripts(owner)
            .expect("parser-deferred work should install into after-parsing order");
        let interactive = page_vm
            .vm_mut()
            .finish_current_main_document_parsing(owner)
            .expect("parser EOF should produce one interactive action");

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_interactive(interactive),
            )
            .await
            .expect("interactive lifecycle turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("interactive events"),
            "ready:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, defer_marker)
            .await
            .expect("main defer script should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("defer events"),
            "ready:interactive|defer:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_domcontentloaded(owner),
            )
            .await
            .expect("main DOMContentLoaded turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("DOMContentLoaded events"),
            "ready:interactive|defer:interactive|dcl:interactive"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_window_load(owner),
            )
            .await
            .expect("main complete/load turn should run");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainLifecycleEvents.join('|')")
                .expect("complete/load events"),
            "ready:interactive|defer:interactive|dcl:interactive|ready:complete|load:complete"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_document_replacement_during_interactive_retires_old_lifecycle_actions() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-lifecycle-replacement.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                r#"
                globalThis.__mainReplacementLifecycleEvents = [];
                document.addEventListener("readystatechange", () => {
                  __mainReplacementLifecycleEvents.push("ready:" + document.readyState);
                  if (document.readyState === "interactive") {
                    document.open();
                  }
                });
                "installed";
                "#,
            )
            .expect("replacement listener should install");
        let retired_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main lifecycle test requires an owner");
        let interactive = page_vm
            .vm_mut()
            .finish_current_main_document_parsing(retired_owner)
            .expect("parser EOF should produce interactive action");

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_interactive(interactive),
            )
            .await
            .expect("interactive replacement turn should run");
        let current_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement must install a current owner");
        assert_ne!(retired_owner, current_owner);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainReplacementLifecycleEvents.join('|') + ':' + document.readyState")
                .expect("replacement state"),
            "ready:interactive:loading"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_domcontentloaded(retired_owner),
            )
            .await
            .expect("stale DOMContentLoaded work should be consumed");
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                PostParsePageOwnedWork::main_document_window_load(retired_owner),
            )
            .await
            .expect("stale load work should be consumed");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainReplacementLifecycleEvents.join('|') + ':' + document.readyState")
                .expect("stale lifecycle state"),
            "ready:interactive:loading",
            "retired owner work must not mutate or dispatch lifecycle events on the replacement"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn empty_main_parser_deferred_seal_does_not_arm_owner_source() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/empty-parser-deferred.html").expect("document URL"),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("empty parser-deferred test requires a document owner");

        assert!(
            page_vm
                .seal_main_parser_deferred_scripts(task_owner)
                .is_none(),
            "empty EOF seal must not create a parser-deferred marker"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "empty EOF seal must not arm a lifecycle source that can block DCL"
        );
    })
    .await;
}

#[test]
fn default_runtime_hooks_reject_direct_no_owner_page_vm_construction() {
    let _js_runtime = crate::JsRuntime::initialize();
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let local_executor = crate::local_executor::JsLocalExecutor::new();
    let error = match PageVm::new(
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
            extra_http_headers: Vec::new(),
            document_policy_container: Default::default(),
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
            layout_policy: crate::real_layout_test_policy(),
            wpt_extensions_enabled: false,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        },
        PageVmRuntimeHooks::default(),
        DomHost::from_dom(HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://example.com/").unwrap(),
            "<!doctype html><html><head></head><body></body></html>".to_owned(),
        )),
        Instant::now(),
    ) {
        Ok(_) => panic!("default runtime hooks must not create standalone document isolates"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("PageVmRuntimeHooks::standalone_without_owner_reservation_for_test()"),
        "unexpected direct no-owner construction error: {error}"
    );
}

#[tokio::test]
async fn page_vm_child_classic_resource_completion_queues_child_frame_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicWaitEvents.push("external:" + (globalThis === self));
parent.__childClassicWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicWaitValue = 91;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_source,
            blocked_host_load_before_completion,
            blocked_host_load_pump_before_completion,
            events_after_blocked_host_load_before_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicWaitEvents.push("load");
  frame.srcdoc = `
    <script id="external-classic" src="{script_url}"><\/script>
    <script id="after-external-classic">
      parent.__childClassicWaitEvents.push(
        "inline:" + globalThis.__childClassicWaitValue
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "child classic srcdoc navigation commit",
                )
                .await;
                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "child classic exact realm",
                )
                .await;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "",
                    "pending child external classic script should block inline script and load"
                );
                let blocked_host_load_before_completion = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let blocked_host_load_pump_before_completion =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_blocked_host_load_before_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;

                let page_resource_queue = page_vm.page_resource_completion_queue();
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        page_vm.page_task_queue.wait_for_page_runtime_wake(),
                    )
                    .await
                    .expect("child classic completion should arrive before timeout");
                }
                let (_, queued_completion) = page_resource_queue
                    .pop_front()
                    .expect("child classic completion should remain queued");
                let completion_owner = queued_completion.owner();
                let completion = match queued_completion.into_terminal() {
                    RendererPageResourceTerminal::ChildClassicScript { completion } => completion,
                    other => panic!("expected child classic completion, got {other:?}"),
                };
                let root_document = page_vm.document_lifecycle.identity().document;
                assert!(
                    completion.network_result.is_some(),
                    "loader-backed child classic completion must retain its Network fact"
                );
                let expected_network_frame_id = completion.network_attribution.frame_id.clone();
                let expected_network_document_url =
                    completion.network_attribution.document_url.clone();
                let expected_network_request_url =
                    completion.network_attribution.request_url.clone();
                let _ = page_vm.vm_mut().take_network_output();
                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let first = page_vm
                    .apply_selected_page_resource_completion_turn(
                        RendererPageResourceCompletion::child_classic_script(
                            root_document,
                            completion,
                        ),
                    )?;
                assert_eq!(first.action.owner, completion_owner);
                assert_eq!(
                    first.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    first.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_completion,
                    "a current child classic Network terminal must advance current Document activity"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                let network_record = &network_records[0];
                assert_eq!(
                    network_record.frame_id(),
                    expected_network_frame_id.as_deref(),
                    "consumer must use producer-captured child frame attribution"
                );
                assert_eq!(
                    network_record.document_url(),
                    &expected_network_document_url
                );
                assert_eq!(network_record.url(), &expected_network_request_url);
                assert_eq!(network_record.resource_type(), SubresourceResourceType::Script);
                assert_eq!(
                    network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Script
                );
                assert!(matches!(
                    network_record.outcome(),
                    SubresourceNetworkOutcome::Success { .. }
                ));
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child classic external execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "external:true|current:external-classic",
                    "first DocumentScriptReady should execute the external classic script without parser continuation or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child classic parser continuation",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicWaitEvents.join('|')")?,
                    "external:true|current:external-classic|inline:91",
                    "second DocumentScriptReady should run parser continuation without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic parser EOF interactive transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_source,
                    blocked_host_load_before_completion,
                    blocked_host_load_pump_before_completion,
                    events_after_blocked_host_load_before_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child classic deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            first_events, "",
            "resource completion turn should not inline-run child classic script"
        );
        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad),
            "child classic bootstrap should start the external source load with one explicit source turn"
        );
        assert!(
            !blocked_host_load_before_completion,
            "HostLoad source should report no progress while parser-blocking classic source is pending"
        );
        assert_eq!(
            blocked_host_load_pump_before_completion, None,
            "blocked lifecycle must not produce a HostLoad task in the stable child-frame family"
        );
        assert_eq!(
            events_after_blocked_host_load_before_completion, "",
            "no HostLoad delivery may exist before classic source completion"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child classic completion should progress through script, parser continuation, interactive, DOMContentLoaded, complete, and HostLoad turns"
        );
        assert_eq!(
            final_events,
            "external:true|current:external-classic|inline:91|load",
            "explicit later wait turns should run queued child classic work"
        );

        server
            .await
            .expect("child classic wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_parser_blocking_classic_waits_for_preceding_stylesheet() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![(
            "/parser-blocking.css",
            "HTTP/1.1 200 OK",
            ":root { --child-parser-style-gate: stylesheet-ready; }".to_owned(),
            Duration::from_millis(180),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            navigation_source,
            completion,
            events_after_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childParserStylesheetEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childParserStylesheetEvents.push("load");
  frame.srcdoc = `
    <link rel="stylesheet" href="{base_url}/parser-blocking.css">
    <script>
      parent.__childParserStylesheetEvents.push(
        "script:" + getComputedStyle(document.documentElement)
          .getPropertyValue("--child-parser-style-gate").trim() + ":" + document.readyState
      );
    <\/script>
    <body></body>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let navigation_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "stylesheet-gated child srcdoc navigation commit",
                )
                .await;
                let materialization = page_vm
                    .run_child_realm_materialization_body_for_test()?
                    .expect("child parser stylesheet fixture should queue one typed realm turn");
                assert_eq!(
                    materialization.action.target_effect,
                    crate::page_task_queue::PageChildRealmMaterializationTargetEffect::MaterializedCurrentOwnerWithoutDocumentStartScript
                );

                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childParserStylesheetEvents.join('|')")?,
                    "",
                    "parser-blocking classic must remain pending while its preparation-time stylesheet snapshot is unresolved"
                );
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must not bypass a stylesheet-blocked parser"
                );

                let page_resource_queue = page_vm.page_resource_completion_queue();
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        page_vm.page_task_queue.wait_for_page_runtime_wake(),
                    )
                    .await
                    .expect("child stylesheet completion should arrive");
                }
                let (_, queued_completion) = page_resource_queue
                    .pop_front()
                    .expect("child stylesheet completion should remain queued");
                let completion_owner = queued_completion.owner();
                let completion = match queued_completion.into_terminal() {
                    RendererPageResourceTerminal::ChildBlockingStylesheet { completion } => {
                        completion
                    }
                    other => panic!("expected child stylesheet completion, got {other:?}"),
                };
                let root_document = page_vm.document_lifecycle.identity().document;
                let completion = page_vm
                    .apply_selected_page_resource_completion_turn(
                        RendererPageResourceCompletion::child_blocking_stylesheet(
                            root_document,
                            completion,
                        ),
                    )?;
                assert_eq!(completion.action.owner, completion_owner);
                let events_after_completion = page_vm
                    .vm_mut()
                    .eval("__childParserStylesheetEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child parser-blocking classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childParserStylesheetEvents.join('|')")?,
                    "script:stylesheet-ready:complete",
                    "stylesheet source must be installed before the parser-blocking script executes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child interactive transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-gated child complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "stylesheet-gated child iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childParserStylesheetEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    navigation_source,
                    completion,
                    events_after_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("child parser stylesheet test should run");

        assert_eq!(navigation_source, ChildFrameSemanticTurnKind::NavigationCommit);
        assert_eq!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );
        assert_eq!(
            events_after_completion, "",
            "stylesheet resource completion must not inline-run child script"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(final_events, "script:stylesheet-ready:complete|load");
        server.await.expect("child parser stylesheet server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_mixed_parser_defer_waits_for_preceding_stylesheet() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/mixed-defer.css",
                "HTTP/1.1 200 OK",
                ":root { --child-defer-style-gate: stylesheet-ready; }".to_owned(),
                Duration::from_millis(600),
            ),
            (
                "/stylesheet-classic-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDeferStylesheetEvents.push(
  "classic:" + getComputedStyle(document.documentElement)
    .getPropertyValue("--child-defer-style-gate").trim()
);
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/stylesheet-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDeferStylesheetEvents.push(
  "module:" + getComputedStyle(document.documentElement)
    .getPropertyValue("--child-defer-style-gate").trim()
);
"#
                .to_owned(),
                Duration::from_millis(20),
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            mut script_completion_sources,
            stylesheet_completion,
            execution_sources,
            lifecycle_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDeferStylesheetEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDeferStylesheetEvents.push("load");
  frame.srcdoc = `
    <link rel="stylesheet" href="{base_url}/mixed-defer.css">
    <script defer src="{base_url}/stylesheet-classic-defer.js"><\/script>
    <script type="module" src="{base_url}/stylesheet-module-defer.js"><\/script>
    <body></body>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    if bootstrap_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                    {
                        break;
                    }
                }
                assert!(
                    bootstrap_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
                    "child parser must start the module root fetch and reach interactive while classic defer fetches directly: {bootstrap_sources:?}"
                );

                let mut script_completion_sources = Vec::new();
                while script_completion_sources.len() != 2 {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("child deferred script completion should arrive");
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    assert_ne!(
                        completion.action.source(),
                        RendererOwnerResourceActivitySource::ChildBlockingStylesheet,
                        "delayed stylesheet must remain pending while both script sources become terminal"
                    );
                    script_completion_sources.push(completion.action.source());
                    if completion.action.source()
                        == RendererOwnerResourceActivitySource::ModuleGraphFetch
                    {
                        run_expected_child_module_script_terminal_turn(
                            &mut page_vm,
                            "stylesheet-blocked child module terminal",
                        )
                        .await;
                    }
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("__childDeferStylesheetEvents.join('|')")?,
                        "",
                        "terminal script sources must remain retained behind the stylesheet snapshot"
                    );
                }
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "no script or HostLoad turn should be runnable before stylesheet completion"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("delayed child stylesheet completion should arrive");
                }
                let stylesheet_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childDeferStylesheetEvents.join('|')")?,
                    "",
                    "stylesheet completion must not inline-execute retained deferred scripts"
                );

                let execution_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child classic defer",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "stylesheet-released child module defer",
                    )
                    .await,
                ];
                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-released child DOMContentLoaded",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-released child complete transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "stylesheet-released child iframe load",
                    )
                    .await,
                ];
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childDeferStylesheetEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    script_completion_sources,
                    stylesheet_completion,
                    execution_sources,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("mixed child defer stylesheet test should run");

        script_completion_sources.sort_by_key(|source| match source {
            RendererOwnerResourceActivitySource::ChildClassicScript => 0,
            RendererOwnerResourceActivitySource::ModuleGraphFetch => 1,
            _ => 2,
        });
        assert_eq!(
            script_completion_sources,
            vec![
                RendererOwnerResourceActivitySource::ChildClassicScript,
                RendererOwnerResourceActivitySource::ModuleGraphFetch,
            ]
        );
        assert_eq!(
            stylesheet_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            execution_sources,
            vec![ChildFrameSemanticTurnKind::DocumentScriptReady; 2]
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "classic:stylesheet-ready|module:stylesheet-ready|load",
            "mixed parser-deferred scripts must preserve document order and observe the installed stylesheet"
        );
        server
            .await
            .expect("mixed child defer stylesheet server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_failed_child_stylesheet_installs_empty_sheet_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![(
            "/load-only.css",
            "HTTP/1.1 404 Not Found",
            ".failed-body { color: rgb(11, 12, 13); }".to_owned(),
            Duration::from_millis(220),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (completion, child_stylesheet_surface, lifecycle_sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childStylesheetHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childStylesheetHostLoadEvents.push("load");
  frame.srcdoc = `<link rel="stylesheet" href="{base_url}/load-only.css"><body class="failed-body"></body>`;
  body.appendChild(frame);
}})()
"#
                ))?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "stylesheet-only child navigation commit",
                )
                .await;
                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-only child interactive transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "stylesheet-only child DOMContentLoaded transition",
                    )
                    .await,
                ];
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must remain blocked after DCL while a child stylesheet is pending"
                );
                assert_eq!(
                    page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await,
                    None,
                    "blocked stylesheet lifecycle must not produce HostLoad progress"
                );

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("failed stylesheet-only completion should arrive");
                }
                let completion = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let child_context_id = page_vm
                    .live_child_default_runtime_realm_inventory()
                    .into_iter()
                    .map(|realm| realm.context_id)
                    .next()
                    .expect("stylesheet child should have one materialized default realm");
                let child_stylesheet_surface = page_vm.vm_mut().eval_in_child_default_context(
                    child_context_id,
                    r#"
(() => {
  const link = document.querySelector("link");
  return JSON.stringify({
    sheet: link.sheet !== null,
    owner: link.sheet && link.sheet.ownerNode === link,
    count: document.styleSheets.length,
    color: getComputedStyle(document.body).color,
  });
})()
"#,
                )?;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childStylesheetHostLoadEvents.join('|')")?,
                    "",
                    "stylesheet completion must not inline-dispatch iframe load"
                );
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "stylesheet-terminal child complete transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::HostLoad,
                    "stylesheet-terminal child iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childStylesheetHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion,
                    child_stylesheet_surface,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("stylesheet-only child HostLoad test should run");

        assert_eq!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert_eq!(
            child_stylesheet_surface,
            r#"{"sheet":true,"owner":true,"count":1,"color":"rgb(0, 0, 0)"}"#,
            "a failed child stylesheet must expose the same empty sheet surface as Chromium and the main document"
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
            ]
        );
        assert_eq!(final_events, "load");
        server
            .await
            .expect("stylesheet-only child HostLoad server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_stylesheet_fonts_block_complete_until_the_exact_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/blocking-resources.css",
                "HTTP/1.1 200 OK",
                r#"
body { background-image: url('/css-image.png'); }
@font-face { font-family: Demo; src: url('/demo.woff2') format('woff2'); }
"#
                .to_owned(),
                Duration::from_millis(80),
            ),
            (
                "/demo.woff2",
                "HTTP/1.1 200 OK",
                "font-body".to_owned(),
                Duration::from_millis(420),
            ),
        ])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        loader.set_optional_resource_fetch_mask(
            crate::protocol_types::OptionalResourceFetchMask::IMAGE
                | crate::protocol_types::OptionalResourceFetchMask::FONT,
        );
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (stylesheet_outcome, lifecycle_ready_after_font, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childCssResourceEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childCssResourceEvents.push("load");
  frame.srcdoc = `<link rel="stylesheet" href="{base_url}/blocking-resources.css"><body></body>`;
  body.appendChild(frame);
}})()
"#
                ))?;

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "CSS-resource child navigation commit",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child interactive transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child DOMContentLoaded transition",
                )
                .await;

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("stylesheet completion should arrive");
                }
                let stylesheet_outcome =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert!(
                    !page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildHostLoad,
                            &loader,
                        )
                        .await?,
                    "HostLoad must remain blocked while stylesheet subresources are pending"
                );

                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("stylesheet font terminal should arrive");
                }
                let outcome = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    outcome.action.source(),
                    RendererOwnerResourceActivitySource::AsyncSubresource,
                    "the stylesheet font terminal must use Networking"
                );
                let lifecycle_ready_after_font = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childCssResourceEvents.join('|')")?,
                    "",
                    "the font terminal must not inline-dispatch iframe load"
                );

                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "CSS-resource child complete transition",
                )
                .await;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::HostLoad,
                    "CSS-resource child iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childCssResourceEvents.join('|')")?;
                assert_eq!(
                    page_vm.vm().css_image_resource_observability_for_test(),
                    (0, 0, 0, 0, Vec::<String>::new()),
                    "raw stylesheet text must not eagerly admit CSS image slots before paint layout"
                );
                Ok::<_, anyhow::Error>((
                    stylesheet_outcome,
                    lifecycle_ready_after_font,
                    final_events,
                ))
            })
            .await
            .expect("stylesheet subresource lifecycle test should run");

        assert_eq!(
            stylesheet_outcome.action.source(),
            RendererOwnerResourceActivitySource::ChildBlockingStylesheet
        );
        assert!(
            lifecycle_ready_after_font,
            "the exact stylesheet font token must make complete runnable"
        );
        assert_eq!(final_events, "load");
        server
            .await
            .expect("stylesheet subresource server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_defer_classic_runs_between_interactive_and_domcontentloaded() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-defer-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicDeferWaitEvents.push("defer:" + (globalThis === self));
parent.__childClassicDeferWaitEvents.push("defer-ready:" + document.readyState);
parent.__childClassicDeferWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicDeferWaitValue = 73;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-defer-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferWaitEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-defer-classic" defer src="{script_url}"><\/script>
    <script>
      document.getElementById("external-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferWaitEvents.push("script-load");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__childClassicDeferWaitEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicDeferWaitEvents.push("dcl:" + document.readyState);
      }});
      parent.__childClassicDeferWaitEvents.push(
        "after:" + String(globalThis.__childClassicDeferWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?;
                    if bootstrap_events == "before:true|after:undefined|ready:interactive" {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined|ready:interactive",
                    "parser EOF should dispatch interactive before the deferred script or load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }

                let first =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load",
                    "DocumentScriptReady should run defer after interactive and before DOMContentLoaded or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child defer classic DOMContentLoaded transition",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive",
                    "DOMContentLoaded should run on its own lifecycle turn after defer and before complete/load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child defer classic complete transition",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferWaitEvents.join('|')")?,
                    "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive|ready:complete",
                    "document complete must be a lifecycle turn after defer and DOMContentLoaded"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child defer classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child defer classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child defer classic deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            first_events, "before:true|after:undefined|ready:interactive",
            "source completion turn should not inline-run child defer classic script"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child defer classic bootstrap should end with the document-owned interactive turn"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child defer classic should progress through script, DCL, complete, and later HostLoad turns"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|ready:interactive|defer:true|defer-ready:interactive|current:external-defer-classic|script-load|dcl:interactive|ready:complete|load",
            "defer must run after interactive and before the later HostLoad DCL/complete/load delivery"
        );

        server
            .await
            .expect("child defer classic wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_defer_classic_source_failure_releases_parser_order_slot() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/missing-child-defer.js",
            "HTTP/1.1 404 Not Found",
            "missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (completion, events_after_completion, followup_sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/missing-child-defer.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferFailureEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferFailureEvents.push("before");<\/script>
    <script id="failed-defer" defer src="{script_url}"><\/script>
    <script>
      document.getElementById("failed-defer").addEventListener("error", () => {{
        parent.__childClassicDeferFailureEvents.push("script-error");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__childClassicDeferFailureEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicDeferFailureEvents.push("dcl");
      }});
      parent.__childClassicDeferFailureEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                for _ in 0..8 {
                    let Some(_) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    if page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?
                        == "before|after|ready:interactive"
                    {
                        break;
                    }
                }
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?,
                    "before|after|ready:interactive",
                    "failed defer must keep DCL gated while its source completion is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("failed child defer completion should arrive");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferFailureEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "failed child defer source terminal",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferFailureEvents.join('|')")?,
                    "before|after|ready:interactive|script-error",
                    "source failure must dispatch error and release defer ordering without resuming the parser"
                );
                for (source, label) in [
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "failed child defer DOMContentLoaded",
                    ),
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "failed child defer complete transition",
                    ),
                    (
                        ChildFrameSemanticTurnKind::HostLoad,
                        "failed child defer iframe load",
                    ),
                ] {
                    followup_sources.push(
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(&mut page_vm, source, label)
                            .await,
                    );
                }
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferFailureEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    completion,
                    events_after_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("child defer source-failure order test should run");

        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            events_after_completion, "before|after|ready:interactive",
            "resource completion must not report or finalize the failed script inline"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "before|after|ready:interactive|script-error|dcl|ready:complete|load"
        );
        server
            .await
            .expect("failed child defer source server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_moved_child_defer_disposes_in_flight_slot_before_later_module() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/moved-child-defer.js",
                "HTTP/1.1 200 OK",
                "parent.__movedChildDeferEvents.push('classic-ran');".to_owned(),
                Duration::from_millis(80),
            ),
            (
                "/later-moved-module.js",
                "HTTP/1.1 200 OK",
                "parent.__movedChildDeferEvents.push('module-ran');".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_after_dispose, sources, final_events) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/moved-child-defer.js");
                let module_url = format!("{base_url}/later-moved-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__movedChildDeferEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "moved-defer-frame";
  frame.onload = () => globalThis.__movedChildDeferEvents.push("load");
  frame.srcdoc = `
    <script>parent.__movedChildDeferEvents.push("before");<\/script>
    <script id="move-defer" defer src="{script_url}"><\/script>
    <script id="later-module" type="module" src="{module_url}"><\/script>
    <script>
      document.getElementById("move-defer").addEventListener("load", () => {{
        parent.__movedChildDeferEvents.push("classic-load");
      }});
      document.getElementById("later-module").addEventListener("load", () => {{
        parent.__movedChildDeferEvents.push("module-load");
      }});
      document.addEventListener("readystatechange", () => {{
        parent.__movedChildDeferEvents.push("ready:" + document.readyState);
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__movedChildDeferEvents.push("dcl");
      }});
      parent.__movedChildDeferEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                for _ in 0..12 {
                    let Some(_) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                }
                assert_eq!(
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?,
                    "before|after|ready:interactive",
                    "later module must remain retained behind the unresolved classic defer"
                );

                let mut classic_completion = None;
                for _ in 0..4 {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("moved child defer completion should arrive");
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    if matches!(
                        completion.action.source(),
                        RendererOwnerResourceActivitySource::ChildClassicScript
                    ) {
                        classic_completion = Some(completion);
                        break;
                    }
                    assert!(
                        matches!(
                            completion.action.source(),
                            RendererOwnerResourceActivitySource::ModuleGraphFetch
                        ),
                        "the only completion allowed ahead of the moved classic defer is its later module root"
                    );
                    run_expected_child_module_script_terminal_turn(
                        &mut page_vm,
                        "module terminal retained behind the moved classic defer",
                    )
                    .await;
                }
                classic_completion
                    .expect("classic source completion must arrive after retained module terminals");
                page_vm.vm_mut().eval(
                    r#"
(() => {
  const frame = document.getElementById("moved-defer-frame");
  document.body.appendChild(frame.contentDocument.getElementById("move-defer"));
  __movedChildDeferEvents.push("moved");
})()
"#,
                )?;

                let mut sources = Vec::new();
                sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "moved classic defer disposal",
                    )
                    .await,
                );
                let events_after_dispose =
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?;
                assert_eq!(
                    events_after_dispose, "before|after|ready:interactive|moved",
                    "disposed classic defer must not execute or dispatch load"
                );
                sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "module after moved classic defer",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?,
                    "before|after|ready:interactive|moved|module-ran|module-load",
                    "disposing the exact classic slot must release the later module-defer"
                );
                for (source, label) in [
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "moved defer DOMContentLoaded",
                    ),
                    (
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "moved defer complete transition",
                    ),
                    (ChildFrameSemanticTurnKind::HostLoad, "moved defer iframe load"),
                ] {
                    sources.push(
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(&mut page_vm, source, label)
                            .await,
                    );
                }
                let final_events = page_vm.vm_mut().eval("__movedChildDeferEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);
                Ok::<_, anyhow::Error>((events_after_dispose, sources, final_events))
            })
            .await
            .expect("moved child defer cancellation test should run");

        assert_eq!(events_after_dispose, "before|after|ready:interactive|moved");
        assert_eq!(
            sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ]
        );
        assert_eq!(
            final_events,
            "before|after|ready:interactive|moved|module-ran|module-load|dcl|ready:complete|load"
        );
        server.await.expect("moved child defer server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_async_classic_handoff_queues_document_script_ready_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-async-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childClassicAsyncWaitEvents.push("async:" + (globalThis === self));
parent.__childClassicAsyncWaitEvents.push("current:" + document.currentScript.id);
globalThis.__childClassicAsyncWaitValue = 41;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-async-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicAsyncWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicAsyncWaitEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicAsyncWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-async-classic" async src="{script_url}"><\/script>
    <script>
      document.getElementById("external-async-classic").addEventListener("load", () => {{
        parent.__childClassicAsyncWaitEvents.push("script-load");
      }});
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childClassicAsyncWaitEvents.push("dcl:" + document.readyState);
      }});
      parent.__childClassicAsyncWaitEvents.push(
        "after:" + String(globalThis.__childClassicAsyncWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicAsyncWaitEvents.join('|')")?;
                    if bootstrap_events == "before:true|after:undefined|dcl:interactive" {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined|dcl:interactive",
                    "async classic must not block the document-owned DOMContentLoaded transition"
                );
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "the document-owned async classic delay must keep HostLoad blocked until source completion is applied"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child async classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child async classic completion sender should remain open"
                    );
                }

                let first =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childClassicAsyncWaitEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child async classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicAsyncWaitEvents.join('|')")?,
                    "before:true|after:undefined|dcl:interactive|async:true|current:external-async-classic|script-load",
                    "DocumentScriptReady should execute child async classic and dispatch its script load without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child async classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child async classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicAsyncWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child async classic follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child async classic deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            first_events, "before:true|after:undefined|dcl:interactive",
            "source completion turn should not inline-run child async classic script"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child async bootstrap should dispatch interactive and DCL before the later async completion"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "child async classic source completion should progress through script execution, complete, and later HostLoad turns"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|dcl:interactive|async:true|current:external-async-classic|script-load|load",
            "explicit later wait turns should run queued child async classic work"
        );

        server
            .await
            .expect("child async classic wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_defer_classic_preserves_document_order_when_second_source_finishes_first() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-defer-classic.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childClassicDeferOrderEvents.push("first:" + (globalThis === self));
parent.__childClassicDeferOrderEvents.push("current:" + document.currentScript.id);
"#
                .to_owned(),
                Duration::from_millis(150),
            ),
            (
                "/second-defer-classic.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childClassicDeferOrderEvents.push("second:" + (globalThis === self));
parent.__childClassicDeferOrderEvents.push("current:" + document.currentScript.id);
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            second_completion,
            events_after_second_completion,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-defer-classic.js");
                let second_script_url = format!("{base_url}/second-defer-classic.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childClassicDeferOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childClassicDeferOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childClassicDeferOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-defer-classic" defer src="{first_script_url}"><\/script>
    <script id="second-defer-classic" defer src="{second_script_url}"><\/script>
    <script>
      document.getElementById("first-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferOrderEvents.push("first-load");
      }});
      document.getElementById("second-defer-classic").addEventListener("load", () => {{
        parent.__childClassicDeferOrderEvents.push("second-load");
      }});
      parent.__childClassicDeferOrderEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?;
                    if source == ChildFrameSemanticTurnKind::DocumentLifecycle {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after",
                    "child parser should continue past both defer classics without firing load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child defer classic completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child defer classic completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?,
                    "before:true|after|first:true|current:first-defer-classic|first-load",
                    "first ordered DocumentScriptReady should execute the earlier defer classic without second defer or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second child defer classic execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childClassicDeferOrderEvents.join('|')")?,
                    "before:true|after|first:true|current:first-defer-classic|first-load|second:true|current:second-defer-classic|second-load",
                    "second ordered DocumentScriptReady should run only after the earlier defer classic completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child defer classic DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child defer classic complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child defer classic iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childClassicDeferOrderEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child defer ordering sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child defer classic ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            second_completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            second_completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "child defer ordering bootstrap should reach interactive before either source completion"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after",
            "faster second defer source completion must not run before earlier defer"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ChildClassicScript
        ));
        assert_eq!(
            first_completion.action.document_effect,
            PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
        );
        assert_eq!(
            first_completion.action.output_effect,
            PageResourceCompletionOutputEffect::CaptureRequired
        );

        assert_eq!(
            events_after_first_completion, "before:true|after",
            "first defer source completion turn should still not inline-run script"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "ordered child defer classics should execute through two script-ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after|first:true|current:first-defer-classic|first-load|second:true|current:second-defer-classic|second-load|load",
            "defer classics should execute in document order even when the second source finishes first"
        );

        server
            .await
            .expect("child defer classic ordering server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_defer_preserves_document_order_when_second_graph_finishes_first() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferOrderEvents.push("first-module:" + (globalThis === self));
globalThis.__childModuleDeferOrderFirst = 1;
"#
                .to_owned(),
                Duration::from_millis(350),
            ),
            (
                "/second-module-defer.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferOrderEvents.push("second-module:" + (globalThis === self));
globalThis.__childModuleDeferOrderSecond = 2;
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            second_completion,
            events_after_second_completion,
            events_after_second_module_owner,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            events_after_first_module_owner,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-module-defer.js");
                let second_script_url = format!("{base_url}/second-module-defer.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDeferOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDeferOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childModuleDeferOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-module-defer" type="module" src="{first_script_url}"><\/script>
    <script id="second-module-defer" type="module" src="{second_script_url}"><\/script>
    <script>
      document.getElementById("first-module-defer").addEventListener("load", () => {{
        parent.__childModuleDeferOrderEvents.push("first-load");
      }});
      document.getElementById("second-module-defer").addEventListener("load", () => {{
        parent.__childModuleDeferOrderEvents.push("second-load");
      }});
      parent.__childModuleDeferOrderEvents.push(
        "after:" + String(globalThis.__childModuleDeferOrderFirst) + ":" +
          String(globalThis.__childModuleDeferOrderSecond)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..10 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?;
                    if bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count()
                        == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true|after:undefined:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true|after:undefined:undefined",
                    "child parser should continue past both parser module-defer scripts without evaluating them"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child module-defer completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "later child module-defer terminal",
                )
                .await;
                let events_after_second_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child module-defer completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "earlier child module-defer terminal",
                )
                .await;
                let events_after_first_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?,
                    "before:true|after:undefined:undefined|first-module:true|first-load",
                    "first ordered DocumentScriptReady should execute the earlier module-defer without second module or iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferOrderEvents.join('|')")?,
                    "before:true|after:undefined:undefined|first-module:true|first-load|second-module:true|second-load",
                    "second ordered DocumentScriptReady should run only after the earlier module-defer completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child module-defer iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferOrderEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module-defer ordering sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    events_after_second_module_owner,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    events_after_first_module_owner,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module-defer ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            bootstrap_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "child module-defer ordering bootstrap should start both parser-module roots explicitly: {bootstrap_sources:?}"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after:undefined:undefined",
            "faster second module graph completion must not evaluate before earlier parser module"
        );
        assert_eq!(
            events_after_second_module_owner, "before:true|after:undefined:undefined",
            "later terminal source must only retain the terminal behind the earlier parser module"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_first_completion, "before:true|after:undefined:undefined",
            "first module graph completion turn should still not inline-run script"
        );
        assert_eq!(
            events_after_first_module_owner, "before:true|after:undefined:undefined",
            "ModuleScriptTerminal should only enqueue ordered document-script work"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "ordered child module-defer scripts should execute through two script-ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined:undefined|first-module:true|first-load|second-module:true|second-load|load",
            "module-defer scripts should execute in document order even when the second graph finishes first"
        );

        server
            .await
            .expect("child module-defer ordering server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_parser_defer_preserves_cross_kind_document_order() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("first-module");"#.to_owned(),
                Duration::from_millis(220),
            ),
            (
                "/second-classic.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("second-classic");"#.to_owned(),
                Duration::ZERO,
            ),
            (
                "/third-classic.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("third-classic");"#.to_owned(),
                Duration::from_millis(420),
            ),
            (
                "/fourth-module.js",
                "HTTP/1.1 200 OK",
                r#"parent.__childMixedDeferEvents.push("fourth-module");"#.to_owned(),
                Duration::from_millis(20),
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            execution_sources,
            lifecycle_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childMixedDeferEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childMixedDeferEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childMixedDeferEvents.push("before");<\/script>
    <script id="first-module" type="module" src="{base_url}/first-module.js"><\/script>
    <script id="second-classic" defer src="{base_url}/second-classic.js"><\/script>
    <script id="third-classic" defer src="{base_url}/third-classic.js"><\/script>
    <script id="fourth-module" type="module" src="{base_url}/fourth-module.js"><\/script>
    <script>
      for (const id of ["first-module", "second-classic", "third-classic", "fourth-module"]) {{
        document.getElementById(id).addEventListener("load", () => {{
          parent.__childMixedDeferEvents.push(id + "-load");
        }});
      }}
      parent.__childMixedDeferEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                for _ in 0..12 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    if source == ChildFrameSemanticTurnKind::DocumentLifecycle
                        && bootstrap_sources
                            .iter()
                            .filter(|source| {
                                **source == ChildFrameSemanticTurnKind::ParserModuleRootStart
                            })
                            .count()
                            == 2
                    {
                        break;
                    }
                }
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after",
                    "parser EOF must not execute any mixed parser-deferred script"
                );
                assert!(
                    bootstrap_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
                    "mixed parser-deferred document should reach interactive"
                );
                assert_eq!(
                    bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count(),
                    2,
                    "both module roots should start before terminal ordering is tested"
                );

                let mut completion_sources = Vec::new();
                let mut terminal_turn_count = 0;
                for completion_index in 0..3 {
                    if !page_vm.page_resource_completion_queue().has_ready_completion() {
                        tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .unwrap_or_else(|_| {
                            panic!(
                                "mixed parser-deferred completion {completion_index} should arrive"
                            )
                        });
                    }
                    let completion =
                        run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    completion_sources.push(completion.action.source());
                    if completion.action.source()
                        == RendererOwnerResourceActivitySource::ModuleGraphFetch
                    {
                        run_expected_child_module_script_terminal_turn(
                            &mut page_vm,
                            "mixed child module terminal",
                        )
                        .await;
                        terminal_turn_count += 1;
                    }
                    assert_eq!(
                        page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                        "before|after",
                        "out-of-order completions and retained terminals must not execute a later parser-deferred script"
                    );
                }

                assert_eq!(
                    completion_sources,
                    vec![
                        RendererOwnerResourceActivitySource::ChildClassicScript,
                        RendererOwnerResourceActivitySource::ModuleGraphFetch,
                        RendererOwnerResourceActivitySource::ModuleGraphFetch,
                    ],
                    "network timing should make later classic and module work terminal before the first module"
                );
                assert_eq!(terminal_turn_count, 2);

                let mut execution_sources = Vec::new();
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first mixed parser-deferred module",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load",
                    "earlier module must execute before the already-ready later classic"
                );
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second mixed parser-deferred classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load|second-classic|second-classic-load",
                    "second classic should execute only after the first module finishes"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("third classic completion should arrive");
                }
                let third_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                completion_sources.push(third_completion.action.source());

                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "third mixed parser-deferred classic",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?,
                    "before|after|first-module|first-module-load|second-classic|second-classic-load|third-classic|third-classic-load",
                    "third classic must execute before the already-terminal fourth module"
                );
                execution_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "fourth mixed parser-deferred module",
                    )
                    .await,
                );

                let lifecycle_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "mixed parser-deferred DOMContentLoaded",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "mixed parser-deferred complete transition",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "mixed parser-deferred iframe load",
                    )
                    .await,
                ];
                let final_events = page_vm.vm_mut().eval("__childMixedDeferEvents.join('|')")?;
                assert_eq!(page_vm.run_next_child_frame_task_source_for_semantic_test().await, None);

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    execution_sources,
                    lifecycle_sources,
                    final_events,
                ))
            })
            .await
            .expect("mixed child parser-deferred ordering test should run");

        assert_eq!(
            completion_sources.last(),
            Some(&RendererOwnerResourceActivitySource::ChildClassicScript)
        );
        assert_eq!(
            execution_sources,
            vec![ChildFrameSemanticTurnKind::DocumentScriptReady; 4]
        );
        assert_eq!(
            lifecycle_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad,
            ],
            "DCL, complete, and iframe load must remain later lifecycle turns"
        );
        assert_eq!(
            final_events,
            "before|after|first-module|first-module-load|second-classic|second-classic-load|third-classic|third-classic-load|fourth-module|fourth-module-load|load",
            "mixed parser-deferred scripts must execute in one cross-kind document order"
        );

        server
            .await
            .expect("mixed child parser-deferred ordering server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_defer_retains_later_graph_failure_behind_earlier_graph() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_concurrent_path_response_http_server(vec![
            (
                "/first-module-defer-success.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDeferFailureOrderEvents.push("first-module:" + (globalThis === self));
globalThis.__childModuleDeferFailureOrderFirst = 1;
"#
                .to_owned(),
                Duration::from_millis(350),
            ),
            (
                "/second-module-defer-failure.js",
                "HTTP/1.1 200 OK",
                "import {".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            second_completion,
            events_after_second_completion,
            events_after_second_module_owner,
            source_after_second_module_owner,
            events_after_blocked_second_terminal,
            first_completion,
            events_after_first_completion,
            bootstrap_sources,
            events_after_first_module_owner,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let first_script_url = format!("{base_url}/first-module-defer-success.js");
                let second_script_url = format!("{base_url}/second-module-defer-failure.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDeferFailureOrderEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDeferFailureOrderEvents.push("load");
  frame.srcdoc = `
    <script>parent.__childModuleDeferFailureOrderEvents.push("before:" + (globalThis === self));<\/script>
    <script id="first-module-defer-success" type="module" src="{first_script_url}"><\/script>
    <script id="second-module-defer-failure" type="module" src="{second_script_url}"><\/script>
    <script>
      document.getElementById("first-module-defer-success").addEventListener("load", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("first-load");
      }});
      document.getElementById("first-module-defer-success").addEventListener("error", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("first-error");
      }});
      document.getElementById("second-module-defer-failure").addEventListener("load", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("second-load");
      }});
      document.getElementById("second-module-defer-failure").addEventListener("error", () => {{
        parent.__childModuleDeferFailureOrderEvents.push("second-error");
      }});
      parent.__childModuleDeferFailureOrderEvents.push(
        "after:" + String(globalThis.__childModuleDeferFailureOrderFirst)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..10 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                    if bootstrap_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        .count()
                        == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true|after:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events, "before:true|after:undefined",
                    "child parser should continue past success/failure module-defer siblings without evaluating them"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("second child module-defer failure should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let second_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_second_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "later failing child module-defer terminal",
                )
                .await;
                let events_after_second_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                let source_after_second_module_owner =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_blocked_second_terminal = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("first child module-defer success should arrive before timeout");
                    assert!(
                        arrived,
                        "child module-defer completion sender should remain open"
                    );
                }
                let first_completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let events_after_first_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "earlier successful child module-defer terminal",
                )
                .await;
                let events_after_first_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                let mut followup_sources = Vec::new();
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "first successful child module-defer execution",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?,
                    "before:true|after:undefined|first-module:true|first-load",
                    "first ordered DocumentScriptReady should execute the earlier successful module before later error dispatch"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "second failing child module-defer dispatch",
                    )
                    .await,
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDeferFailureOrderEvents.join('|')")?,
                    "before:true|after:undefined|first-module:true|first-load|second-error",
                    "later graph failure should dispatch only after the earlier module-defer completes"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer graph-failure DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "ordered child module-defer graph-failure complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "ordered child module-defer graph-failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDeferFailureOrderEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    second_completion,
                    events_after_second_completion,
                    events_after_second_module_owner,
                    source_after_second_module_owner,
                    events_after_blocked_second_terminal,
                    first_completion,
                    events_after_first_completion,
                    bootstrap_sources,
                    events_after_first_module_owner,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module-defer graph-failure ordering test should run");

        assert!(matches!(
            second_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            bootstrap_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "child module-defer failure ordering bootstrap should start both roots explicitly: {bootstrap_sources:?}"
        );
        assert_eq!(
            events_after_second_completion, "before:true|after:undefined",
            "faster second graph failure completion must not dispatch script error before earlier parser module"
        );
        assert_eq!(
            events_after_second_module_owner, "before:true|after:undefined",
            "later graph failure terminal must be retained behind the earlier parser module"
        );
        assert_eq!(
            source_after_second_module_owner, None,
            "retained later graph failure should not wake DocumentScriptReady before the earlier graph finishes"
        );
        assert_eq!(
            events_after_blocked_second_terminal, "before:true|after:undefined",
            "blocked later graph failure should not dispatch script error or iframe load"
        );
        assert!(matches!(
            first_completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_first_completion, "before:true|after:undefined",
            "first graph completion turn should not inline-run script"
        );
        assert_eq!(
            events_after_first_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should only enqueue ordered document-script work"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "ordered child module-defer success/failure should finish through ready turns, DOMContentLoaded, and complete before HostLoad"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|first-module:true|first-load|second-error|load",
            "later graph failure should preserve parser module document order and keep iframe load on HostLoad"
        );

        server
            .await
            .expect("child module-defer graph-failure ordering server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_parser_module_root_completion_queues_module_script_terminal_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-parser-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childParserModuleWaitEvents.push("module:" + (globalThis === self));
globalThis.__childParserModuleWaitValue = 188;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(1)),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                owner_wake,
            );
        let page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            document_url,
            runtime_hooks,
        );
        let page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            bootstrap_sources,
            events_after_module_owner,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let script_url = format!("{base_url}/child-parser-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childParserModuleWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childParserModuleWaitEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childParserModuleWaitEvents.push(
        "before:" + (globalThis === self) + ":" + document.currentScript.isConnected
      );
    <\/script>
    <script id="external-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childParserModuleWaitEvents.push("script-load");
      }});
      parent.__childParserModuleWaitEvents.push(
        "after:" + String(globalThis.__childParserModuleWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let mut bootstrap_sources = Vec::new();
                let mut bootstrap_events = String::new();
                for _ in 0..8 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    bootstrap_sources.push(source);
                    bootstrap_events = page_vm
                        .vm_mut()
                        .eval("__childParserModuleWaitEvents.join('|')")?;
                    if bootstrap_sources
                        .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart)
                        && bootstrap_sources
                            .iter()
                            .filter(|source| {
                                **source == ChildFrameSemanticTurnKind::DocumentScriptReady
                            })
                            .count()
                            == 2
                        && bootstrap_sources
                            .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                        && bootstrap_events == "before:true:true|after:undefined"
                    {
                        break;
                    }
                }
                assert_eq!(
                    bootstrap_events,
                    "before:true:true|after:undefined",
                    "parser should continue past child module-defer script while root fetch is pending; sources={bootstrap_sources:?}"
                );

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner wake route should remain open");
                        }
                    })
                    .await
                    .expect("child parser module completion should arrive before timeout");
                }

                let _ = page_vm.vm_mut().take_network_output();
                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let first = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("production child module completion should consume one typed turn");
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "the production root terminal must be consumed exactly once"
                );
                assert_eq!(
                    first.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    first.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_completion,
                    "a current child module Network terminal must advance current Document activity"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                let network_record = &network_records[0];
                assert!(
                    network_record.frame_id().is_some(),
                    "producer must retain the child frame attribution"
                );
                assert_eq!(network_record.document_url().as_str(), "about:srcdoc");
                assert_eq!(network_record.url().as_str(), script_url);
                assert_eq!(
                    network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child parser-module root terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child parser-module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childParserModuleWaitEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    bootstrap_sources,
                    events_after_module_owner,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child parser module deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            first_events, "before:true:true|after:undefined",
            "resource completion turn should not inline-run child parser module graph"
        );
        assert_eq!(
            bootstrap_sources,
            vec![
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::RealmMaterialization,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::ParserModuleRootStart,
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "the typed root fetch-start must preserve parser discovery FIFO, then the parser should run the following inline script and reach interactive"
        );
        assert_eq!(
            events_after_module_owner, "before:true:true|after:undefined",
            "ModuleScriptTerminal should not execute the module-defer script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module-defer graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true:true|after:undefined|module:true|script-load",
            "DocumentScriptReady should run the module-defer script and dispatch its script load without iframe load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after module-defer execution"
        );
        assert_eq!(
            final_events,
            "before:true:true|after:undefined|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load only after module-defer execution completes"
        );

        server
            .await
            .expect("child parser module wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_joined_parser_roots_batch_one_module_script_terminal() {
    run_page_vm_async_test(async move {
        let (base_url, shutdown_server_tx, server) = spawn_shutdown_path_response_http_server(vec![(
            "/child-shared-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childJoinedParserRootEvents.push("module:" + (globalThis === self));
globalThis.__childJoinedParserRootValue = 501;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            first_ready_source,
            events_after_first_ready,
            second_ready_source,
            events_after_second_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-shared-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childJoinedParserRootEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childJoinedParserRootEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childJoinedParserRootEvents.push("before:" + (globalThis === self));<\/script>
    <script id="joined-module-a" type="module" src="{script_url}"><\/script>
    <script id="joined-module-b" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("joined-module-a").addEventListener("load", () => {{
        parent.__childJoinedParserRootEvents.push("script-a-load");
      }});
      document.getElementById("joined-module-b").addEventListener("load", () => {{
        parent.__childJoinedParserRootEvents.push("script-b-load");
      }});
      parent.__childJoinedParserRootEvents.push(
        "after:" + String(globalThis.__childJoinedParserRootValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childJoinedParserRootEvents.join('|')")?;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    if source.is_none()
                        || page_vm.has_ready_page_websocket_task_for_test()
                        || events_before_completion.contains("frame-load")
                    {
                        break;
                    }
                }
                let resource_ready_before_wait = page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion();

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("joined child module completion should arrive before timeout");
                    assert!(
                        arrived,
                        "joined child module completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "joined parser-root terminal fanout",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let first_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_ready = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let second_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_second_ready = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "joined child parser-module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childJoinedParserRootEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    first_ready_source,
                    events_after_first_ready,
                    second_ready_source,
                    events_after_second_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm joined child parser roots test should run");

        assert_eq!(
            pre_completion_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::ParserModuleRootStart)
                .count(),
            2,
            "each parser root should get a reserve/join root-fetch source turn: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed shared module response should leave a window to prove joined roots wait for one completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "joined child parser roots should not evaluate before the shared fetch completes"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "module owner event should only enqueue document-script ready work"
        );
        assert_eq!(
            first_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "first joined parser root should execute on a document-script ready turn"
        );
        assert_eq!(
            events_after_first_ready,
            "before:true|after:undefined|module:true|script-a-load",
            "first ready turn should evaluate the shared module and dispatch the first script load"
        );
        assert_eq!(
            second_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "second joined parser root should execute on a later document-script ready turn"
        );
        assert_eq!(
            events_after_second_ready,
            "before:true|after:undefined|module:true|script-a-load|script-b-load",
            "second ready turn should dispatch the second script load without re-evaluating the module"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after joined parser roots"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module:true|script-a-load|script-b-load|frame-load",
            "HostLoad should dispatch iframe load after both joined parser roots complete"
        );

        shutdown_server_tx
            .send(())
            .expect("joined child parser roots server shutdown should send");
        let requested_paths = server
            .await
            .expect("joined child parser roots server should finish");
        assert_eq!(
            requested_paths,
            vec!["/child-shared-module.js"],
            "joined parser roots should reserve twice but share one network module fetch"
        );
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_script_blocks_host_load_until_evaluation_finishes() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModuleHostLoadEvents.push("module:" + (globalThis === self));
globalThis.__childModuleHostLoadValue = 203;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            initial_host_load_before_root_fetch,
            navigation_commit_source,
            first_document_script_ready_before_root_fetch,
            host_load_attempt_before_root_fetch,
            events_after_pre_root_host_load_attempt,
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-load-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="load-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("load-module").addEventListener("load", () => {{
        parent.__childModuleHostLoadEvents.push("script-load");
      }});
      parent.__childModuleHostLoadEvents.push(
        "after:" + String(globalThis.__childModuleHostLoadValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let initial_host_load_before_root_fetch = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let navigation_commit_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "child parser-module exact realm",
                )
                .await;
                let first_document_script_ready_before_root_fetch =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await
                        == Some(ChildFrameSemanticTurnKind::DocumentScriptReady);
                let host_load_attempt_before_root_fetch = page_vm
                    .run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::ChildHostLoad,
                        &loader,
                    )
                    .await?;
                let events_after_pre_root_host_load_attempt = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childModuleHostLoadEvents.join('|')")?;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    if source.is_none()
                        || page_vm.has_ready_page_websocket_task_for_test()
                        || events_before_completion.contains("frame-load")
                    {
                        break;
                    }
                }
                let resource_ready_before_wait = page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion();

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child module completion should arrive before timeout");
                    assert!(arrived, "child module completion sender should remain open");
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module HostLoad-gate terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    initial_host_load_before_root_fetch,
                    navigation_commit_source,
                    first_document_script_ready_before_root_fetch,
                    host_load_attempt_before_root_fetch,
                    events_after_pre_root_host_load_attempt,
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module HostLoad gate test should run");

        assert!(
            !initial_host_load_before_root_fetch,
            "HostLoad source should not progress while initial document-script ready work is pending"
        );
        assert_eq!(
            navigation_commit_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "child navigation must install the active Document before its parser-script turns"
        );
        assert!(
            first_document_script_ready_before_root_fetch,
            "the initial child DocumentScriptReady turn should enqueue parser-module root fetch work"
        );
        assert!(
            !host_load_attempt_before_root_fetch,
            "HostLoad source should make no progress while parser-module root fetch work is still queued"
        );
        assert!(
            !events_after_pre_root_host_load_attempt.contains("frame-load"),
            "queued parser-module root fetch work must block iframe load before source priority runs it"
        );
        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !pre_completion_sources.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "blocked bootstrap HostLoad should not re-ready itself before module source completion: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should not execute the module or dispatch iframe load inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|module:true|script-load",
            "module evaluation should dispatch script load but not iframe load inline"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load only after module evaluation finishes"
        );

        server
            .await
            .expect("child module HostLoad gate server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_async_module_allows_dcl_before_source_completion() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-async-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childAsyncModuleEvents.push("module:" + (globalThis === self));
globalThis.__childAsyncModuleValue = 307;
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_at_dcl,
            resource_ready_at_dcl,
            post_dcl_sources_before_completion,
            completion_source,
            document_script_source,
            events_after_module,
            complete_source,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-async-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childAsyncModuleEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childAsyncModuleEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childAsyncModuleEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childAsyncModuleEvents.push("dcl");
      }});
    <\/script>
    <script id="async-module" type="module" async src="{script_url}"><\/script>
    <script>
      document.getElementById("async-module").addEventListener("load", () => {{
        parent.__childAsyncModuleEvents.push("script-load");
      }});
      parent.__childAsyncModuleEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_at_dcl = String::new();
                for _ in 0..12 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    events_at_dcl = page_vm
                        .vm_mut()
                        .eval("__childAsyncModuleEvents.join('|')")?;
                    if events_at_dcl.contains("dcl") || source.is_none() {
                        break;
                    }
                }
                let resource_ready_at_dcl =
                    page_vm.has_ready_page_websocket_task_for_test();
                let mut post_dcl_sources_before_completion = Vec::new();
                for _ in 0..4 {
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    post_dcl_sources_before_completion.push(source);
                }

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child async module completion should arrive before timeout");
                    assert!(arrived, "child async module completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child async-module terminal",
                )
                .await;
                let document_script_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_module = page_vm
                    .vm_mut()
                    .eval("__childAsyncModuleEvents.join('|')")?;
                let complete_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childAsyncModuleEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_at_dcl,
                    resource_ready_at_dcl,
                    post_dcl_sources_before_completion,
                    completion_source,
                    document_script_source,
                    events_after_module,
                    complete_source,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child async module lifecycle test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::DocumentLifecycle),
            "the child document should reach an explicit lifecycle turn before async module completion: {pre_completion_sources:?}"
        );
        assert!(
            !pre_completion_sources.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "the async module load-delay token must keep HostLoad unavailable before source completion: {pre_completion_sources:?}"
        );
        assert_eq!(events_at_dcl, "before|after|dcl");
        assert!(
            !resource_ready_at_dcl,
            "DCL should run while the delayed async module source is still pending"
        );
        assert!(
            pre_completion_sources
                .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "the async root fetch must start when the parser encounters the module, before DCL observes parser completion: {pre_completion_sources:?}"
        );
        assert!(
            !post_dcl_sources_before_completion
                .contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "one async parser module must publish exactly one root-start task: before={pre_completion_sources:?}, after={post_dcl_sources_before_completion:?}"
        );
        assert!(
            !post_dcl_sources_before_completion
                .contains(&ChildFrameSemanticTurnKind::DocumentLifecycle)
                && !post_dcl_sources_before_completion.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "the async-module token should block complete and HostLoad without blocking DCL: {post_dcl_sources_before_completion:?}"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            document_script_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady)
        );
        assert_eq!(
            events_after_module,
            "before|after|dcl|module:true|script-load",
            "async module execution should release complete without redispatching DCL"
        );
        assert_eq!(
            complete_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle)
        );
        assert_eq!(host_load_source, Some(ChildFrameSemanticTurnKind::HostLoad));
        assert_eq!(
            final_events,
            "before|after|dcl|module:true|script-load|frame-load"
        );

        server
            .await
            .expect("child async module lifecycle server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_tla_releases_lifecycle_after_evaluation_starts() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-tla-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModuleTlaHostLoadEvents.push("module-start:" + (globalThis === self));
await new Promise(resolve => {
  parent.__resolveChildModuleTlaHostLoad = resolve;
  parent.__childModuleTlaHostLoadEvents.push("module-pending");
});
globalThis.__childModuleTlaHostLoadValue = 409;
parent.__childModuleTlaHostLoadEvents.push("module-after");
"#
            .to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            graph_ready_source,
            events_after_pending_evaluation,
            host_load_source_while_tla_pending,
            events_after_host_load,
            parent_completion_recheck_source,
            events_after_module_reaction,
            evaluation_completion_source,
            final_events,
        ) = local_executor
            // Keep this large, stateful scenario off nextest's comparatively
            // small test-thread stack; pinning does not add a scheduler turn.
            .run(Box::pin(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-tla-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleTlaHostLoadEvents = [];
  delete globalThis.__resolveChildModuleTlaHostLoad;
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleTlaHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleTlaHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="tla-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("tla-module").addEventListener("load", () => {{
        parent.__childModuleTlaHostLoadEvents.push("script-load");
      }});
      document.getElementById("tla-module").addEventListener("error", () => {{
        parent.__childModuleTlaHostLoadEvents.push("script-error");
      }});
      parent.__childModuleTlaHostLoadEvents.push(
        "after:" + String(globalThis.__childModuleTlaHostLoadValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childModuleTlaHostLoadEvents.join('|')")?;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    if source.is_none()
                        || page_vm.has_ready_page_websocket_task_for_test()
                        || events_before_completion.contains("frame-load")
                    {
                        break;
                    }
                }
                let resource_ready_before_wait = page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion();

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child TLA module completion should arrive before timeout");
                    assert!(arrived, "child TLA module completion sender should remain open");
                }

                let loader = page_vm.main_document_resource_loader();
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child TLA module terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;
                let graph_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_pending_evaluation = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                let host_load_source_while_tla_pending = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module TLA iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;
                let parent_completion_recheck = page_vm
                    .run_page_main_document_runtime_body_for_test(loader.request_client())
                    .await?
                    .expect("child HostLoad must publish one typed parent completion recheck");

                page_vm
                    .vm_mut()
                    .eval("__resolveChildModuleTlaHostLoad(); 'ok'")?;
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ModuleReaction,
                            loader.request_client(),
                        )
                        .await?,
                    "resolved child TLA should enqueue one typed module reaction"
                );
                let events_after_module_reaction = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                let evaluation_completion_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleTlaHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    graph_ready_source,
                    events_after_pending_evaluation,
                    host_load_source_while_tla_pending,
                    events_after_host_load,
                    parent_completion_recheck.action,
                    events_after_module_reaction,
                    evaluation_completion_source,
                    final_events,
                ))
            }))
            .await
            .expect("page vm child TLA module HostLoad gate test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed TLA module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending TLA module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after:undefined",
            "ModuleScriptTerminal should not start TLA evaluation or dispatch iframe load inline"
        );
        assert_eq!(
            graph_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "module graph-ready work should start TLA evaluation through DocumentScriptReady"
        );
        assert_eq!(
            events_after_pending_evaluation,
            "before:true|after:undefined|module-start:true|module-pending|script-load",
            "starting TLA should complete the static module script and dispatch its load event"
        );
        assert_eq!(
            host_load_source_while_tla_pending,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "the evaluation promise must not keep the child lifecycle blocked"
        );
        assert_eq!(
            events_after_host_load,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load",
            "iframe load should run after the static module execution turn while TLA remains pending"
        );
        assert_eq!(
            parent_completion_recheck_source.kind(),
            crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork,
            "child HostLoad completion should publish one concrete parent completion recheck task"
        );
        assert_eq!(
            parent_completion_recheck_source.target_effect(),
            crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
            "the exact parent completion recheck must apply through the main-Document arbiter"
        );
        assert_eq!(
            events_after_module_reaction,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load|module-after",
            "module reaction should finish evaluation after document load without redispatching lifecycle events"
        );
        assert_eq!(
            evaluation_completion_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "fulfilled TLA continuation should re-enter DocumentScriptReady"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|module-start:true|module-pending|script-load|frame-load|module-after",
            "TLA completion must not dispatch duplicate script or iframe load events"
        );

        server
            .await
            .expect("child TLA module HostLoad gate server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_dynamic_import_does_not_block_host_load() {
    run_page_vm_async_test(async move {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum PostLoadProgressSource {
            TypedMainDocumentRuntime {
                kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind,
                effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect,
            },
            TypedDynamicImportOwnerAction,
        }

        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-dynamic-root.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDynamicImportHostLoadEvents.push("module-start:" + (globalThis === self));
import("./child-dynamic-leaf.js").then(
  () => parent.__childDynamicImportHostLoadEvents.push(
    "dynamic-fulfilled:" + String(globalThis.__childDynamicImportLeafValue)
  ),
  () => parent.__childDynamicImportHostLoadEvents.push("dynamic-rejected")
);
parent.__childDynamicImportHostLoadEvents.push("module-after");
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-dynamic-leaf.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childDynamicImportHostLoadEvents.push("dynamic-module:" + (globalThis === self));
globalThis.__childDynamicImportLeafValue = 701;
"#
                .to_owned(),
                Duration::from_millis(500),
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(2)),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                owner_wake,
            );
        let page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            document_url,
            runtime_hooks,
        );
        let page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            startup_sources,
            events_before_completion,
            completion_source,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
            post_load_progress_sources,
            events_after_dynamic_fetch_scheduled,
            dynamic_fetch_completion_source,
            host_load_pending_after_dynamic_fetch_completion,
            dynamic_owner_action_applied,
            events_after_dynamic_owner_action_body,
            events_after_dynamic_owner_action,
            host_load_pending_after_dynamic_owner_action,
        ) = local_executor
            // Keep this large, stateful scenario off nextest's comparatively
            // small test-thread stack; pinning does not add a scheduler turn.
            .run(Box::pin(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let root_url = format!("{base_url}/child-dynamic-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDynamicImportHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDynamicImportHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <base href="{base_url}/">
    <script>parent.__childDynamicImportHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="dynamic-root" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("dynamic-root").addEventListener("load", () => {{
        parent.__childDynamicImportHostLoadEvents.push("script-load");
      }});
      document.getElementById("dynamic-root").addEventListener("error", () => {{
        parent.__childDynamicImportHostLoadEvents.push("script-error");
      }});
      parent.__childDynamicImportHostLoadEvents.push(
        "after:" + String(globalThis.__childDynamicImportLeafValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut startup_sources = Vec::new();
                for _ in 0..8 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    startup_sources.push(source);
                }
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner-attached dynamic-import route should remain open");
                        }
                    })
                    .await
                    .expect("child dynamic root completion should arrive before timeout");
                }

                let completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("typed child root completion should consume one Page turn");
                let completion_source = completion.action.source();
                let (root_network_records, root_websocket_events, root_websocket_lifecycle) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(root_websocket_events.is_empty());
                assert!(root_websocket_lifecycle.is_empty());
                assert_eq!(root_network_records.len(), 1);
                assert_eq!(
                    root_network_records[0].request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dynamic import root terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child dynamic import root module execution",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "child dynamic import iframe load before dynamic fetch",
                )
                .await;
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childDynamicImportHostLoadEvents.join('|')")?;

                let mut post_load_progress_sources = Vec::new();
                let mut events_after_dynamic_fetch_scheduled = String::new();
                for _ in 0..8 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    if let Some(outcome) = page_vm
                        .run_page_main_document_runtime_body_for_test(&loader)
                        .await?
                    {
                        post_load_progress_sources.push(
                            PostLoadProgressSource::TypedMainDocumentRuntime {
                                kind: outcome.action.kind(),
                                effect: outcome.action.target_effect(),
                            },
                        );
                        events_after_dynamic_fetch_scheduled = page_vm
                            .vm_mut()
                            .eval("__childDynamicImportHostLoadEvents.join('|')")?;
                        continue;
                    }
                    if page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                        .has_ready_task()
                    {
                        let action = page_vm
                            .run_page_dynamic_import_owner_action_body_for_test()
                            .expect("typed dynamic-import owner action should win its Page turn");
                        let crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::AppliedToCurrentOwner {
                            outcome,
                        } = action.action.document_effect
                        else {
                            panic!("current dynamic-import owner action must not be stale: {action:?}");
                        };
                        assert!(
                            outcome.waiting_fetch_was_scheduled(),
                            "the first typed owner action should schedule the dynamic fetch"
                        );
                        page_vm
                            .finish_selected_page_task_completion(
                                action.action.into_page_task_completion(),
                                &loader,
                            )
                            .await?;
                        post_load_progress_sources
                            .push(PostLoadProgressSource::TypedDynamicImportOwnerAction);
                        events_after_dynamic_fetch_scheduled = page_vm
                            .vm_mut()
                            .eval("__childDynamicImportHostLoadEvents.join('|')")?;
                        continue;
                    }
                    break;
                }

                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx.recv().await.expect(
                                "owner-attached dynamic-import completion route should remain open",
                            );
                        }
                    })
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "child dynamic import completion should arrive before timeout: progress={post_load_progress_sources:?} events={events_after_dynamic_fetch_scheduled}"
                        )
                    });
                }
                let activity_epoch_before_dynamic_completion =
                    page_vm.vm().subresource_activity_epoch();
                let dynamic_fetch_completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("child dynamic import fetch completion should be ready");
                assert_eq!(
                    dynamic_fetch_completion.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dynamic_fetch_completion.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );
                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_dynamic_completion,
                    "a current dynamic-import Network fact must advance current Document activity"
                );
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "one producer terminal must be consumed exactly once"
                );
                let (dynamic_network_records, dynamic_websocket_events, dynamic_websocket_lifecycle) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(dynamic_websocket_events.is_empty());
                assert!(dynamic_websocket_lifecycle.is_empty());
                assert_eq!(dynamic_network_records.len(), 1);
                assert_eq!(
                    dynamic_network_records[0].url().as_str(),
                    format!("{base_url}/child-dynamic-leaf.js")
                );
                assert_eq!(
                    dynamic_network_records[0].request_initiator_type(),
                    SubresourceRequestInitiatorType::Script
                );
                let dynamic_fetch_completion_source = dynamic_fetch_completion.action.source();
                let host_load_pending_after_dynamic_fetch_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                let dynamic_owner_action = page_vm
                    .run_page_dynamic_import_owner_action_body_for_test()
                    .expect("dynamic-import ready action should consume one typed Page turn");
                let crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::AppliedToCurrentOwner {
                    outcome: dynamic_owner_action_outcome,
                } = dynamic_owner_action.action.document_effect
                else {
                    panic!("current dynamic-import ready action must not be stale: {dynamic_owner_action:?}");
                };
                let dynamic_owner_action_applied =
                    dynamic_owner_action_outcome.evaluation_import_was_resolved();
                let events_after_dynamic_owner_action_body = page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "__childDynamicImportHostLoadEvents.join('|')",
                    )?;
                page_vm
                    .finish_selected_page_task_completion(
                        dynamic_owner_action.action.into_page_task_completion(),
                        &loader,
                    )
                    .await?;
                let events_after_dynamic_owner_action = page_vm
                    .vm_mut()
                    .eval_without_microtask_checkpoint_for_test(
                        "__childDynamicImportHostLoadEvents.join('|')",
                    )?;
                let host_load_pending_after_dynamic_owner_action = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion_source,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                    post_load_progress_sources,
                    events_after_dynamic_fetch_scheduled,
                    dynamic_fetch_completion_source,
                    host_load_pending_after_dynamic_fetch_completion,
                    dynamic_owner_action_applied,
                    events_after_dynamic_owner_action_body,
                    events_after_dynamic_owner_action,
                    host_load_pending_after_dynamic_owner_action,
                ))
            }))
            .await
            .expect("page vm child dynamic import HostLoad proof should run");

        assert!(
            startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "child dynamic import startup should start the root module fetch from a typed source: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "pending root module should not run dynamic import or dispatch iframe load before completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined",
            "ModuleScriptTerminal should not execute the root module inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "root module graph-ready work should execute through DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|module-start:true|module-after|script-load",
            "DocumentScriptReady should execute the root module and dispatch script load without iframe load or dynamic import completion"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a HostLoad turn after the root module script queues dynamic import"
        );
        assert_eq!(
            events_after_host_load,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load",
            "HostLoad should dispatch iframe load before dynamic import fetch scheduling or completion"
        );
        assert!(
            post_load_progress_sources.contains(
                &PostLoadProgressSource::TypedMainDocumentRuntime {
                    kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob,
                    effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
                },
            ),
            "dynamic import should advance through its concrete exact-main-Document task: {post_load_progress_sources:?}"
        );
        let dynamic_module_job = PostLoadProgressSource::TypedMainDocumentRuntime {
            kind: crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob,
            effect: crate::page_task_queue::PageMainDocumentRuntimeTargetEffect::AppliedToCurrentOwner,
        };
        let materialize_position = post_load_progress_sources
            .iter()
            .position(|source| *source == dynamic_module_job)
            .expect("dynamic import should expose its concrete graph-advance turn");
        let owner_action_position = post_load_progress_sources
            .iter()
            .position(|source| *source == PostLoadProgressSource::TypedDynamicImportOwnerAction)
            .expect("dynamic import graph advance should publish an exact child owner action");
        assert!(
            owner_action_position > materialize_position,
            "the exact child owner action must follow its graph advance without imposing a cross-source adjacency rule: {post_load_progress_sources:?}"
        );
        assert_eq!(
            events_after_dynamic_fetch_scheduled,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load",
            "dynamic import runtime work should schedule fetch without executing the dynamic module after iframe load"
        );
        assert_eq!(
            dynamic_fetch_completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert!(
            !host_load_pending_after_dynamic_fetch_completion,
            "dynamic import fetch completion should not queue HostLoad after the iframe has already loaded"
        );
        assert!(
            dynamic_owner_action_applied,
            "dynamic import ready action should resolve from the typed Page source"
        );
        assert_eq!(
            events_after_dynamic_owner_action_body,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load|dynamic-module:true",
            "DynamicImportOwnerAction body must leave the user import reaction for selected-task completion"
        );
        assert_eq!(
            events_after_dynamic_owner_action,
            "before:true|after:undefined|module-start:true|module-after|script-load|frame-load|dynamic-module:true|dynamic-fulfilled:701",
            "selected-task completion should fulfill the child dynamic import after the body returns"
        );
        assert!(
            !host_load_pending_after_dynamic_owner_action,
            "dynamic import ready action should not requeue HostLoad"
        );

        server
            .await
            .expect("child dynamic import HostLoad proof server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_graph_failure_blocks_host_load_until_error_dispatches() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-bad-module.js",
            "HTTP/1.1 200 OK",
            "import {".to_owned(),
            Duration::from_millis(200),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            pre_completion_sources,
            events_before_completion,
            resource_ready_before_wait,
            completion_source,
            events_after_module_owner,
            graph_failure_source,
            events_after_graph_failure,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-bad-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleFailureHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleFailureHostLoadEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleFailureHostLoadEvents.push("before:" + (globalThis === self));<\/script>
    <script id="bad-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("bad-module").addEventListener("load", () => {{
        parent.__childModuleFailureHostLoadEvents.push("script-load");
      }});
      document.getElementById("bad-module").addEventListener("error", () => {{
        parent.__childModuleFailureHostLoadEvents.push("script-error");
      }});
      parent.__childModuleFailureHostLoadEvents.push("after");
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut pre_completion_sources = Vec::new();
                let mut events_before_completion = String::new();
                for _ in 0..8 {
                    let source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                    events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childModuleFailureHostLoadEvents.join('|')")?;
                    if let Some(source) = source {
                        pre_completion_sources.push(source);
                    }
                    if source.is_none()
                        || page_vm.has_ready_page_websocket_task_for_test()
                        || events_before_completion.contains("frame-load")
                    {
                        break;
                    }
                }
                let resource_ready_before_wait = page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion();

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child bad module completion should arrive before timeout");
                    assert!(arrived, "child bad module completion sender should remain open");
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child graph-failure module terminal",
                )
                .await;
                let events_after_module_owner = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                let graph_failure_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_graph_failure = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child module graph-failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleFailureHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    pre_completion_sources,
                    events_before_completion,
                    resource_ready_before_wait,
                    completion_source,
                    events_after_module_owner,
                    graph_failure_source,
                    events_after_graph_failure,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child bad module HostLoad gate test should run");

        assert!(
            pre_completion_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
            "pre-completion turns should start the module root fetch from its typed source: {pre_completion_sources:?}"
        );
        assert!(
            !resource_ready_before_wait,
            "delayed bad module response should leave a window to prove HostLoad is gated before completion"
        );
        assert_eq!(
            events_before_completion, "before:true|after",
            "pending bad module script must block iframe load before resource completion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            events_after_module_owner, "before:true|after",
            "module owner event should not dispatch script error or iframe load inline"
        );
        assert_eq!(
            graph_failure_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "graph failure should dispatch through DocumentScriptReady"
        );
        assert_eq!(
            events_after_graph_failure, "before:true|after|script-error",
            "graph failure should dispatch script error without iframe load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a later HostLoad source after graph failure"
        );
        assert_eq!(
            final_events, "before:true|after|script-error|frame-load",
            "HostLoad should dispatch iframe load only after graph failure finalizes"
        );

        server
            .await
            .expect("child bad module HostLoad gate server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_script_event_created_nested_navigation_precedes_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-event-module.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childScriptEventNestedEvents.push("module:" + (globalThis === self));
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_source,
            module_followup_sources,
            events_after_script_event,
            nested_script_ready_source,
            events_after_nested_script,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let script_url = format!("{base_url}/child-event-module.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childScriptEventNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <script>parent.__childScriptEventNestedEvents.push("before:" + (globalThis === self));<\/script>
    <script id="event-module" type="module" src="{script_url}"><\/script>
    <script>
      document.getElementById("event-module").addEventListener("load", () => {{
        parent.__childScriptEventNestedEvents.push("script-load");
        const nested = document.createElement("iframe");
        nested.srcdoc = "<script>parent.parent.__childScriptEventNestedEvents.push('nested-script:' + (globalThis === self));<\\/script>";
        document.body.appendChild(nested);
      }});
      parent.__childScriptEventNestedEvents.push("after:" + String(globalThis.__childScriptEventNestedValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module startup should start the parser module root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childScriptEventNestedEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past the child module while the root fetch is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module completion sender should remain open"
                    );
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child script-event module terminal fanout",
                )
                .await;
                let mut module_followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childScriptEventNestedEvents.join('|')")?,
                    "before:true|after:undefined",
                    "ModuleScriptTerminal should queue graph-ready work without running the module or nested iframe"
                );
                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child script-event module execution",
                    )
                    .await,
                );
                let events_after_script_event = page_vm
                    .vm_mut()
                    .eval("__childScriptEventNestedEvents.join('|')")?;

                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "nested iframe navigation created during child script event",
                    )
                    .await,
                );
                module_followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child script-event DOMContentLoaded transition",
                    )
                    .await,
                );
                let nested_script_ready_source = Some(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "nested iframe ready work created during child script event",
                    )
                    .await,
                );
                let events_after_nested_script = page_vm
                    .vm_mut()
                    .eval("__childScriptEventNestedEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion_source,
                    module_followup_sources,
                    events_after_script_event,
                    nested_script_ready_source,
                    events_after_nested_script,
                ))
            })
            .await
            .expect("child script event nested ready-work source test should run");

        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        );
        assert_eq!(
            module_followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::NavigationCommit,
                ChildFrameSemanticTurnKind::DocumentLifecycle
            ],
            "module execution, nested navigation commit, and DOMContentLoaded should remain separate child-frame turns after the typed terminal fanout"
        );
        assert_eq!(
            events_after_script_event, "before:true|after:undefined|module:true|script-load",
            "module evaluation should dispatch the script load event before nested parser work"
        );
        assert_eq!(
            nested_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "nested parser ready work should enter DocumentScriptReady after its owner transaction commits"
        );
        assert_eq!(
            events_after_nested_script,
            "before:true|after:undefined|module:true|script-load|nested-script:true"
        );

        server
            .await
            .expect("child script event nested ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_dependency_completion_queues_module_script_terminal_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-root-module.js",
                "HTTP/1.1 200 OK",
                r#"
import { depValue } from "./child-dependency.js";
parent.__childModuleDependencyWaitEvents.push("root:" + depValue);
globalThis.__childModuleDependencyWaitValue = depValue;
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-dependency.js",
                "HTTP/1.1 200 OK",
                r#"
parent.__childModuleDependencyWaitEvents.push("dep:" + (globalThis === self));
export const depValue = "dep-value";
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            dependency,
            events_after_dependency_completion,
            followup_sources,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/child-root-module.js");
                let dependency_url = format!("{base_url}/child-dependency.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDependencyWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDependencyWaitEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleDependencyWaitEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModuleDependencyWaitEvents.push("script-load");
      }});
      parent.__childModuleDependencyWaitEvents.push(
        "after:" + String(globalThis.__childModuleDependencyWaitValue)
      );
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module dependency startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module dependency startup should start the root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyWaitEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past child module script while dependency fetches are pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module root completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module root completion sender should remain open"
                    );
                }

                let root = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    root.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    root.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    root.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dependency root terminal",
                )
                .await;
                let (root_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(root_network_records.len(), 1);
                assert_eq!(root_network_records[0].url().as_str(), root_url);
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart,
                            &loader,
                        )
                        .await?,
                    "module terminal should queue one selected dependency-start task",
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child module dependency completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child module dependency completion sender should remain open"
                    );
                }

                let activity_epoch_before_dependency =
                    page_vm.vm().subresource_activity_epoch();
                let dependency =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    dependency.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dependency.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_dependency,
                    "a current child dependency Network terminal must advance current Document activity"
                );
                let (dependency_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(dependency_network_records.len(), 1);
                let dependency_network_record = &dependency_network_records[0];
                assert_eq!(dependency_network_record.url().as_str(), dependency_url);
                assert!(dependency_network_record.frame_id().is_some());
                assert_eq!(
                    dependency_network_record.request_initiator_type(),
                    SubresourceRequestInitiatorType::Parser
                );
                let events_after_dependency_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module dependency terminal fanout",
                )
                .await;
                let mut followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyWaitEvents.join('|')")?,
                    "before:true|after:undefined",
                    "dependency terminal fanout should not execute the module graph inline"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child module dependency graph-ready execution",
                )
                .await,
                );
                let events_after_graph_ready = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                assert_eq!(
                    events_after_graph_ready,
                    "before:true|after:undefined|dep:true|root:dep-value|script-load",
                    "dependency graph-ready execution should dispatch script load without iframe load"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child module dependency iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module dependency follow-up sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    dependency,
                    events_after_dependency_completion,
                    followup_sources,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module dependency deferred completion test should run");

        assert!(matches!(
            dependency.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_dependency_completion, "before:true|after:undefined",
            "dependency completion turn should not inline-run child module graph"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "graph-ready, DOMContentLoaded, complete, and HostLoad must remain separate child turns after typed terminal fanout"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|dep:true|root:dep-value|script-load|frame-load",
            "explicit later turns should run queued child module graph work before iframe load"
        );

        server
            .await
            .expect("child module dependency wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_module_dependency_failure_queues_graph_failed_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-root-module.js",
                "HTTP/1.1 200 OK",
                r#"
import { depValue } from "./missing-child-dependency.js";
parent.__childModuleDependencyFailureEvents.push("root:" + depValue);
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/missing-child-dependency.js",
                "HTTP/1.1 404 Not Found",
                "missing dependency".to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            dependency,
            events_after_dependency_completion,
            followup_sources,
            events_after_graph_failure,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/child-root-module.js");
                let dependency_url = format!("{base_url}/missing-child-dependency.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModuleDependencyFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModuleDependencyFailureEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModuleDependencyFailureEvents.push("before:" + (globalThis === self));<\/script>
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModuleDependencyFailureEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModuleDependencyFailureEvents.push("script-error");
      }});
      parent.__childModuleDependencyFailureEvents.push("after:" + String(globalThis.__childModuleDependencyFailureValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::DocumentScriptReady),
                    "child module dependency failure startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
                );
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::ParserModuleRootStart),
                    "child module dependency failure startup should start the root fetch from its typed source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyFailureEvents.join('|')")?,
                    "before:true|after:undefined",
                    "parser should continue past child module script while dependency fetch is pending"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child parser module root completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child parser module root completion sender should remain open"
                    );
                }

                run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child dependency-failure root terminal",
                )
                .await;
                let _ = page_vm.vm_mut().take_network_output();
                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModuleDependencyFetchStart,
                            &loader,
                        )
                        .await?,
                    "module terminal should queue one selected dependency-start task",
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect(
                        "child module dependency failure completion should arrive before timeout",
                    );
                    assert!(
                        arrived,
                        "child module dependency failure completion sender should remain open"
                    );
                }

                let dependency =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                assert_eq!(
                    dependency.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    dependency.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                let (dependency_network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert_eq!(dependency_network_records.len(), 1);
                assert_eq!(
                    dependency_network_records[0].url().as_str(),
                    dependency_url
                );
                assert!(matches!(
                    dependency_network_records[0].outcome(),
                    SubresourceNetworkOutcome::Success { status: 404, .. }
                ));
                let events_after_dependency_completion = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "child module dependency failure terminal fanout",
                )
                .await;
                let mut followup_sources = Vec::new();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childModuleDependencyFailureEvents.join('|')")?,
                    "before:true|after:undefined",
                    "dependency failure terminal fanout should not dispatch script error or iframe load inline"
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "child module dependency graph-failed dispatch",
                    )
                    .await,
                );
                let events_after_graph_failure = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency failure DOMContentLoaded transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                        "child module dependency failure complete transition",
                    )
                    .await,
                );
                followup_sources.push(
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::HostLoad,
                        "child module dependency failure iframe load",
                    )
                    .await,
                );
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModuleDependencyFailureEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child module dependency failure sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    dependency,
                    events_after_dependency_completion,
                    followup_sources,
                    events_after_graph_failure,
                    final_events,
                ))
            })
            .await
            .expect("page vm child module dependency failure test should run");

        assert!(matches!(
            dependency.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_dependency_completion, "before:true|after:undefined",
            "dependency failure completion turn should not inline-dispatch script error"
        );
        assert_eq!(
            followup_sources,
            vec![
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                ChildFrameSemanticTurnKind::HostLoad
            ],
            "graph-failed dispatch, DOMContentLoaded, complete, and HostLoad must remain separate child turns after typed terminal fanout"
        );
        assert_eq!(
            events_after_graph_failure, "before:true|after:undefined|script-error",
            "graph-failed work should dispatch script error without iframe load"
        );
        assert_eq!(
            final_events, "before:true|after:undefined|script-error|frame-load",
            "HostLoad should dispatch iframe load only after dependency failure finalizes"
        );

        server
            .await
            .expect("child module dependency failure server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_modulepreload_terminal_event_does_not_delay_complete() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (
            lifecycle_turns,
            events_before_preload_event,
            ready_state_before_preload_event,
            events_after_preload_event,
            ready_state_after_preload_event,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childModulepreloadTerminalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadTerminalEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childModulepreloadTerminalEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {
        parent.__childModulepreloadTerminalEvents.push("dcl:" + document.readyState);
      });
    <\/script>
    <link rel="modulepreload" href="/invalid-modulepreload.bin" as="image" onerror="parent.__childModulepreloadTerminalEvents.push('preload-error')">
    <script>parent.__childModulepreloadTerminalEvents.push("after");<\/script>
  `;
  body.appendChild(frame);
})()
"#,
                )?;

                let mut lifecycle_turns = Vec::new();
                while let Some(source) = page_vm
                    .run_next_child_frame_task_source_for_semantic_test()
                    .await
                {
                    lifecycle_turns.push(source);
                }

                let events_before_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadTerminalEvents.join('|')")?;
                let ready_state_before_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument?.readyState || 'missing'",
                )?;

                assert!(
                    page_vm
                        .run_exact_selected_page_task_for_test(
                            PageSelectedTaskTestSelector::ChildModulepreloadEventAction,
                            &loader,
                        )
                        .await?,
                    "the deliberately withheld modulepreload error action must remain queued"
                );
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadTerminalEvents.join('|')")?;
                let ready_state_after_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument?.readyState || 'missing'",
                )?;

                Ok::<_, anyhow::Error>((
                    lifecycle_turns,
                    events_before_preload_event,
                    ready_state_before_preload_event,
                    events_after_preload_event,
                    ready_state_after_preload_event,
                ))
            })
            .await
            .expect("terminal child modulepreload lifecycle test should run");

        assert!(
            lifecycle_turns
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::DocumentLifecycle)
                .count()
                >= 3,
            "interactive, DOMContentLoaded, and complete must advance while the terminal link event remains queued: {lifecycle_turns:?}"
        );
        assert!(
            lifecycle_turns.contains(&ChildFrameSemanticTurnKind::HostLoad),
            "iframe load must remain independently runnable while the terminal link event is withheld: {lifecycle_turns:?}"
        );
        assert_eq!(
            events_before_preload_event,
            "before|after|dcl:interactive|frame-load",
            "child complete and iframe load must not wait for the queued modulepreload error"
        );
        assert_eq!(
            ready_state_before_preload_event, "complete",
            "a terminal modulepreload event must not hold the child Document load gate"
        );
        assert_eq!(
            events_after_preload_event,
            "before|after|dcl:interactive|frame-load|preload-error",
            "the independently queued modulepreload event must still dispatch afterward"
        );
        assert_eq!(
            ready_state_after_preload_event, "complete",
            "post-complete modulepreload dispatch must not regress readyState"
        );
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_modulepreload_fetch_does_not_delay_iframe_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/standalone-preload.js",
            "HTTP/1.1 200 OK",
            "export const preloaded = true;".to_owned(),
            Duration::from_millis(100),
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(crate::PageId::new_for_testing(2)),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                owner_wake,
            );
        let page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            document_url,
            runtime_hooks,
        );
        let page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            typed_modulepreload_start_ran,
            startup_sources,
            events_before_completion,
            ready_state_before_completion,
            completion,
            events_after_preload_event,
            ready_state_after_preload_event,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut page_resource_queue = page_resource_queue;
                let mut owner_wake_rx = owner_wake_rx;
                let preload_url = format!("{base_url}/standalone-preload.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childStandaloneModulepreloadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childStandaloneModulepreloadEvents.push("frame-load");
  frame.srcdoc = `
    <script>
      parent.__childStandaloneModulepreloadEvents.push("before");
      document.addEventListener("DOMContentLoaded", () => {{
        parent.__childStandaloneModulepreloadEvents.push("dcl:" + document.readyState);
      }});
    <\/script>
    <link rel="modulepreload" href="{preload_url}" onload="parent.__childStandaloneModulepreloadEvents.push('preload-load')" onerror="parent.__childStandaloneModulepreloadEvents.push('preload-error')">
    <script>parent.__childStandaloneModulepreloadEvents.push("after");<\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let mut typed_modulepreload_start_ran = false;
                let mut startup_sources = Vec::new();
                for _ in 0..10 {
                    if page_resource_queue.has_ready_completion() {
                        break;
                    }
                    if page_vm.page_task_executor_sources_for_test().modulepreload_start()
                        .has_ready_task()
                    {
                        assert!(
                            page_vm
                                .run_exact_selected_page_task_for_test(
                                    PageSelectedTaskTestSelector::ModulepreloadStart,
                                    &loader,
                                )
                                .await?,
                            "typed modulepreload start should enter the production selected dispatcher",
                        );

                        assert!(
                            !typed_modulepreload_start_ran,
                            "one parser-discovered link must publish one typed start"
                        );
                        typed_modulepreload_start_ran = true;
                        continue;
                    }
                    let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                        break;
                    };
                    startup_sources.push(source);
                }
                if !page_resource_queue.has_ready_completion() {
                    tokio::time::timeout(Duration::from_secs(2), async {
                        while !page_resource_queue.has_ready_completion() {
                            owner_wake_rx
                                .recv()
                                .await
                                .expect("owner-attached modulepreload wake route should remain open");
                        }
                    })
                    .await
                    .expect("typed modulepreload completion should arrive before timeout");
                }
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childStandaloneModulepreloadEvents.join('|')")?;
                let ready_state_before_completion = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument.readyState",
                )?;

                let activity_epoch_before_completion =
                    page_vm.vm().subresource_activity_epoch();
                let completion = page_vm
                    .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                    .expect("typed modulepreload completion should consume one Page owner turn");
                assert_eq!(
                    completion.action.document_effect,
                    PageResourceCompletionDocumentEffect::AppliedToCurrentOwner
                );
                assert_eq!(
                    completion.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );
                assert!(
                    page_vm.vm().subresource_activity_epoch()
                        > activity_epoch_before_completion,
                    "current modulepreload Network output must advance current Document activity"
                );
                assert!(
                    !page_resource_queue.has_ready_completion(),
                    "one modulepreload producer terminal must be consumed exactly once"
                );
                let (network_records, websocket_events, websocket_lifecycle_events) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                assert!(websocket_events.is_empty());
                assert!(websocket_lifecycle_events.is_empty());
                assert_eq!(network_records.len(), 1);
                assert!(network_records[0].frame_id().is_some());
                assert_eq!(network_records[0].document_url().as_str(), "about:srcdoc");
                assert_eq!(network_records[0].url().as_str(), preload_url);
                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "owner-scheduled child modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childStandaloneModulepreloadEvents.join('|')")?;
                let ready_state_after_preload_event = page_vm.vm_mut().eval(
                    "document.querySelector('iframe').contentDocument.readyState",
                )?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "owner-scheduled modulepreload sequence should not leave child frame work"
                );

                Ok::<_, anyhow::Error>((
                    typed_modulepreload_start_ran,
                    startup_sources,
                    events_before_completion,
                    ready_state_before_completion,
                    completion,
                    events_after_preload_event,
                    ready_state_after_preload_event,
                ))
            })
            .await
            .expect("owner-scheduled child modulepreload lifecycle test should run");

        assert!(
            typed_modulepreload_start_ran,
            "parser-discovered modulepreload should start from its typed source: {startup_sources:?}"
        );
        assert!(
            startup_sources
                .iter()
                .filter(|source| **source == ChildFrameSemanticTurnKind::DocumentLifecycle)
                .count()
                >= 3,
            "interactive, DOMContentLoaded and complete should run while modulepreload fetch is pending: {startup_sources:?}"
        );
        assert!(startup_sources.contains(&ChildFrameSemanticTurnKind::HostLoad));
        assert_eq!(
            events_before_completion,
            "before|after|dcl:interactive|frame-load"
        );
        assert_eq!(
            ready_state_before_completion, "complete",
            "fetching modulepreload must not delay document complete or iframe load"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event,
            "before|after|dcl:interactive|frame-load|preload-load",
            "a slow modulepreload link event may dispatch after iframe load"
        );
        assert_eq!(
            ready_state_after_preload_event, "complete",
            "post-complete modulepreload event dispatch must not regress readyState"
        );

        server
            .await
            .expect("standalone child modulepreload server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_modulepreload_fetch_runs_before_joined_module_root() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preloaded-root.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childModulepreloadJoinEvents.push("module:" + (globalThis === self));
globalThis.__childModulepreloadJoinValue = 551;
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            startup_sources,
            events_before_completion,
            completion,
            events_after_preload_event,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/preloaded-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModulepreloadJoinEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadJoinEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModulepreloadJoinEvents.push("before:" + (globalThis === self));<\/script>
    <link id="preload" rel="modulepreload" href="{root_url}" onload="parent.__childModulepreloadJoinEvents.push('preload-load')" onerror="parent.__childModulepreloadJoinEvents.push('preload-error')">
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModulepreloadJoinEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModulepreloadJoinEvents.push("script-error");
      }});
      parent.__childModulepreloadJoinEvents.push("after:" + String(globalThis.__childModulepreloadJoinValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let startup_sources =
                    drive_child_modulepreload_startup_until_resource_completion_ready(
                        &mut page_vm,
                        &loader,
                        8,
                    )
                    .await;
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child modulepreload completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child modulepreload completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;

                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "child modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "same-URL modulepreload terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "same-URL modulepreload graph-ready execution",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "same-URL modulepreload iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadJoinEvents.join('|')")?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "same-URL modulepreload sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion,
                    events_after_preload_event,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child modulepreload joined root test should run");

        assert!(
            startup_sources.contains(&ChildModulepreloadStartupTurn::ChildSemanticTurn(
                ChildFrameSemanticTurnKind::DocumentScriptReady,
            )),
            "child modulepreload startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
        );
        let modulepreload_fetch_position = startup_sources
            .iter()
            .position(|turn| *turn == ChildModulepreloadStartupTurn::TypedModulepreloadStart)
            .expect("parser-discovered modulepreload should start a fetch before completion");
        let parser_root_position = startup_sources
            .iter()
            .position(|turn| {
                *turn
                    == ChildModulepreloadStartupTurn::ChildSemanticTurn(
                        ChildFrameSemanticTurnKind::ParserModuleRootStart,
                    )
            })
            .expect("same-URL parser module root should still run and join the module map entry");
        assert!(
            modulepreload_fetch_position < parser_root_position,
            "parser-discovered modulepreload should start before same-URL parser module root joins it: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "modulepreload fetch start should not execute module or dispatch link/frame events"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event, "before:true|after:undefined|preload-load",
            "modulepreload link load should dispatch before module script execution"
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined|preload-load",
            "ModuleScriptTerminal should not execute the joined module inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "joined module graph-ready work should execute on DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready,
            "before:true|after:undefined|preload-load|module:true|script-load",
            "DocumentScriptReady should execute the joined module without iframe load"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "before:true|after:undefined|preload-load|module:true|script-load|frame-load",
            "HostLoad should dispatch iframe load after joined modulepreload execution"
        );

        server
            .await
            .expect("child modulepreload joined root server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_modulepreload_failure_wakes_joined_root_before_host_load() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/preloaded-root.js",
            "HTTP/1.1 404 Not Found",
            "modulepreload missing".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            startup_sources,
            events_before_completion,
            completion,
            events_after_preload_event,
            events_after_module_terminal,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let root_url = format!("{base_url}/preloaded-root.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childModulepreloadFailureEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childModulepreloadFailureEvents.push("frame-load");
  frame.srcdoc = `
    <script>parent.__childModulepreloadFailureEvents.push("before:" + (globalThis === self));<\/script>
    <link id="preload" rel="modulepreload" href="{root_url}" onload="parent.__childModulepreloadFailureEvents.push('preload-load')" onerror="parent.__childModulepreloadFailureEvents.push('preload-error')">
    <script id="external-module" type="module" src="{root_url}"><\/script>
    <script>
      document.getElementById("external-module").addEventListener("load", () => {{
        parent.__childModulepreloadFailureEvents.push("script-load");
      }});
      document.getElementById("external-module").addEventListener("error", () => {{
        parent.__childModulepreloadFailureEvents.push("script-error");
      }});
      parent.__childModulepreloadFailureEvents.push("after:" + String(globalThis.__childModulepreloadFailureValue));
    <\/script>
  `;
  body.appendChild(frame);
}})()
"#
                ))?;

                let startup_sources =
                    drive_child_modulepreload_startup_until_resource_completion_ready(
                        &mut page_vm,
                        &loader,
                        8,
                    )
                    .await;
                let events_before_completion = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child failed modulepreload completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child failed modulepreload completion sender should remain open"
                    );
                }

                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;

                run_expected_child_modulepreload_event_action_for_test(
                    &mut page_vm,
                    &loader,
                    "child failed modulepreload event dispatch",
                )
                .await;
                let events_after_preload_event = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                run_expected_child_module_script_terminal_turn(
                    &mut page_vm,
                    "failed same-URL modulepreload terminal fanout",
                )
                .await;
                let events_after_module_terminal = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                let script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "failed same-URL modulepreload graph-failed dispatch",
                )
                .await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "failed same-URL modulepreload iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childModulepreloadFailureEvents.join('|')")?;

                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "failed same-URL modulepreload sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    startup_sources,
                    events_before_completion,
                    completion,
                    events_after_preload_event,
                    events_after_module_terminal,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child failed modulepreload joined root test should run");

        assert!(
            startup_sources.contains(&ChildModulepreloadStartupTurn::ChildSemanticTurn(
                ChildFrameSemanticTurnKind::DocumentScriptReady,
            )),
            "child failed modulepreload startup should reach DocumentScriptReady without test drain: {startup_sources:?}"
        );
        let modulepreload_fetch_position = startup_sources
            .iter()
            .position(|turn| *turn == ChildModulepreloadStartupTurn::TypedModulepreloadStart)
            .expect("parser-discovered modulepreload should start a fetch before failure");
        let parser_root_position = startup_sources
            .iter()
            .position(|turn| {
                *turn
                    == ChildModulepreloadStartupTurn::ChildSemanticTurn(
                        ChildFrameSemanticTurnKind::ParserModuleRootStart,
                    )
            })
            .expect("same-URL parser module root should still run and join the failed module map entry");
        assert!(
            modulepreload_fetch_position < parser_root_position,
            "parser-discovered modulepreload should start before same-URL parser module root joins it: {startup_sources:?}"
        );
        assert_eq!(
            events_before_completion, "before:true|after:undefined",
            "modulepreload fetch start should not dispatch link/script/frame events before failure"
        );
        assert!(matches!(
            completion.action.source(),
            RendererOwnerResourceActivitySource::ModuleGraphFetch
        ));
        assert_eq!(
            events_after_preload_event, "before:true|after:undefined|preload-error",
            "modulepreload link error should dispatch before joined module script failure"
        );
        assert_eq!(
            events_after_module_terminal, "before:true|after:undefined|preload-error",
            "ModuleScriptTerminal should not dispatch the joined script error inline"
        );
        assert_eq!(
            script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "joined module graph-failed work should dispatch on DocumentScriptReady"
        );
        assert_eq!(
            events_after_script_ready, "before:true|after:undefined|preload-error|script-error",
            "DocumentScriptReady should dispatch the joined module script error without iframe load"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source after modulepreload failure"
        );
        assert_eq!(
            final_events, "before:true|after:undefined|preload-error|script-error|frame-load",
            "HostLoad should dispatch iframe load after joined modulepreload failure finalizes"
        );

        server
            .await
            .expect("child failed modulepreload joined root server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_document_completion_queues_document_script_ready_work() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-document.html",
            "HTTP/1.1 200 OK",
            r#"
<!doctype html>
<script>
parent.__childDocumentLoadWaitEvents.push("child-script:" + (globalThis === self));
globalThis.__childDocumentLoadWaitValue = 42;
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            first,
            first_events,
            realm_pending_after_first,
            host_load_pending_after_completion,
            first_followup_source,
            events_after_first_followup,
            lifecycle_ready_after_first_followup,
            interactive_source,
            host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-document.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childDocumentLoadWaitEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childDocumentLoadWaitEvents.push("frameload");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;
                let startup_sources =
                    drive_child_frame_task_sources_until_resource_completion_ready(
                        &mut page_vm,
                        8,
                    )
                    .await;
                assert!(
                    startup_sources.contains(&ChildFrameSemanticTurnKind::NavigationCommit),
                    "external child document startup should begin from an explicit NavigationCommit source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childDocumentLoadWaitEvents.join('|')")?,
                    "",
                    "child document script and frame load should wait for document completion"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child document completion sender should remain open"
                    );
                }

                let first = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let first_events = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                let realm_pending_after_first = page_vm
                    .vm()
                    .has_pending_child_frame_realm_materialization();
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child document parser realm prerequisite",
                )?;
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

                let first_followup_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child document parser script execution",
                )
                .await;
                let events_after_first_followup = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                let lifecycle_ready_after_first_followup = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );

                let interactive_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentLifecycle,
                    "child document parser EOF interactive transition",
                )
                .await;
                let host_load_source = run_child_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "child document iframe load",
                )
                .await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__childDocumentLoadWaitEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "child document completion sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    first,
                    first_events,
                    realm_pending_after_first,
                    host_load_pending_after_completion,
                    first_followup_source,
                    events_after_first_followup,
                    lifecycle_ready_after_first_followup,
                    interactive_source,
                    host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm child document deferred completion test should run");

        assert!(matches!(
            first.action.source(),
            RendererOwnerResourceActivitySource::ChildDocument
        ));
        assert_eq!(
            first_events, "",
            "child document completion turn should not inline-run child document script or frame load"
        );
        assert!(
            realm_pending_after_first,
            "child document completion must leave a typed realm prerequisite queued"
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not wake HostLoad while initial parser work is pending"
        );
        assert_eq!(
            first_followup_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "first child frame task turn should run document-script ready work"
        );
        assert_eq!(
            events_after_first_followup, "child-script:true",
            "first child frame task turn should run document-script ready work, not iframe load"
        );
        assert!(
            lifecycle_ready_after_first_followup,
            "document-script ready should make the later lifecycle turn runnable"
        );
        assert_eq!(
            interactive_source,
            ChildFrameSemanticTurnKind::DocumentLifecycle,
            "parser EOF should become interactive before HostLoad"
        );
        assert_eq!(
            host_load_source,
            ChildFrameSemanticTurnKind::HostLoad,
            "iframe load should remain a later HostLoad source"
        );
        assert_eq!(
            final_events,
            "child-script:true|frameload",
            "explicit later HostLoad turn should dispatch iframe load"
        );

        server
            .await
            .expect("child document wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_document_ready_work_runs_one_item_per_turn() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/child-a.html",
                "HTTP/1.1 200 OK",
                r#"
<script>
parent.__multiChildDocumentEvents.push("child-a-script:" + (globalThis === self));
</script>
"#
                .to_owned(),
                Duration::ZERO,
            ),
            (
                "/child-b.html",
                "HTTP/1.1 200 OK",
                r#"
<script>
parent.__multiChildDocumentEvents.push("child-b-script:" + (globalThis === self));
</script>
"#
                .to_owned(),
                Duration::ZERO,
            ),
        ])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
            owner_wake_tx,
            crate::runtime::RendererPageToken::new_for_testing(PageId::new_for_testing(1)),
        );
        let runtime_hooks =
            PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                owner_wake,
            );
        let page_vm = test_page_vm_with_loader_document_url_and_hooks(
            &loader,
            Vec::new(),
            document_url,
            runtime_hooks,
        );
        let mut page_resource_queue = page_vm.page_resource_completion_queue();
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_completions,
            first_script_ready_source,
            events_after_first_script_ready,
            second_script_ready_source,
            events_after_second_script_ready,
            lifecycle_ready_after_second_script,
            lifecycle_sources,
            first_host_load_source,
            events_after_first_host_load,
            second_host_load_source,
            final_events,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_a_url = format!("{base_url}/child-a.html");
                let child_b_url = format!("{base_url}/child-b.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__multiChildDocumentEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  for (const [name, url] of [["a", "{child_a_url}"], ["b", "{child_b_url}"]]) {{
    const frame = document.createElement("iframe");
    frame.onload = () => globalThis.__multiChildDocumentEvents.push("frame-" + name + "-load");
    frame.src = url;
    body.appendChild(frame);
  }}
}})()
"#
                ))?;
                let startup_sources = vec![
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "first external child navigation start",
                    )
                    .await,
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "second external child navigation start",
                    )
                    .await,
                ];
                assert_eq!(
                    startup_sources
                        .iter()
                        .filter(|source| **source == ChildFrameSemanticTurnKind::NavigationCommit)
                        .count(),
                    2,
                    "each child document startup should consume one NavigationCommit source: {startup_sources:?}"
                );
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__multiChildDocumentEvents.join('|')")?,
                    "",
                    "child documents should wait for external document completions"
                );

                let mut completion_sources = Vec::new();
                for label in ["first child document", "second child document"] {
                    child_document_completion::wait_for_page_resource_completion(
                        &mut page_resource_queue,
                        &mut owner_wake_rx,
                        label,
                    )
                    .await;
                    let completion = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)?
                        .expect("child document completion should consume one typed Page turn");
                    completion_sources.push(completion.action.source);
                }
                for label in ["first child realm", "second child realm"] {
                    run_expected_pending_child_realm_materialization_turn(
                        &mut page_vm,
                        label,
                    )?;
                }
                let events_after_completions = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;

                let first_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_script_ready = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                let second_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_second_script_ready = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                let lifecycle_ready_after_second_script = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );
                let lifecycle_sources = vec![
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                ];
                let first_host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_first_host_load = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;

                let second_host_load_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let final_events = page_vm
                    .vm_mut()
                    .eval("__multiChildDocumentEvents.join('|')")?;
                assert_eq!(
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await,
                    None,
                    "multi-child document completion sequence should not leave extra child frame task work"
                );

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_completions,
                    first_script_ready_source,
                    events_after_first_script_ready,
                    second_script_ready_source,
                    events_after_second_script_ready,
                    lifecycle_ready_after_second_script,
                    lifecycle_sources,
                    first_host_load_source,
                    events_after_first_host_load,
                    second_host_load_source,
                    final_events,
                ))
            })
            .await
            .expect("page vm multi child document ready-work test should run");

        assert_eq!(
            completion_sources,
            vec![
                RendererOwnerResourceActivitySource::ChildDocument,
                RendererOwnerResourceActivitySource::ChildDocument,
            ]
        );
        assert_eq!(
            events_after_completions, "",
            "document completion turns should not inline-run either child document"
        );
        assert_eq!(
            first_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "first child frame task turn should run only one document-script ready item"
        );
        let first_turn_ran_a = events_after_first_script_ready.contains("child-a-script:true");
        let first_turn_ran_b = events_after_first_script_ready.contains("child-b-script:true");
        assert_ne!(
            first_turn_ran_a, first_turn_ran_b,
            "first document-script ready turn should run exactly one child script; events: {events_after_first_script_ready}"
        );
        assert_eq!(
            second_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "the stable child-frame family must preserve the already-enqueued second script ahead of lifecycle work produced by the first"
        );
        for expected in ["child-a-script:true", "child-b-script:true"] {
            assert!(
                events_after_second_script_ready.contains(expected),
                "second document-script ready turn should run both child scripts across two turns; events: {events_after_second_script_ready}"
            );
        }
        for unexpected in ["frame-a-load", "frame-b-load"] {
            assert!(
                !events_after_second_script_ready.contains(unexpected),
                "second document-script ready turn should still not dispatch iframe load inline; events: {events_after_second_script_ready}"
            );
        }
        assert!(
            lifecycle_ready_after_second_script,
            "both scripts should leave the oldest exact-Document lifecycle action at the family head"
        );
        assert_eq!(
            lifecycle_sources,
            vec![Some(ChildFrameSemanticTurnKind::DocumentLifecycle); 6],
            "interactive, DOMContentLoaded and complete must each consume one lifecycle turn per child"
        );
        assert_eq!(
            first_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "first iframe load should remain a later HostLoad source"
        );
        let first_host_load_dispatched_a =
            events_after_first_host_load.contains("frame-a-load");
        let first_host_load_dispatched_b =
            events_after_first_host_load.contains("frame-b-load");
        assert_ne!(
            first_host_load_dispatched_a,
            first_host_load_dispatched_b,
            "first HostLoad turn should dispatch exactly one iframe load; events: {events_after_first_host_load}"
        );
        assert_eq!(
            second_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "second iframe load should remain a separate HostLoad source"
        );
        for expected in [
            "child-a-script:true",
            "child-b-script:true",
            "frame-a-load",
            "frame-b-load",
        ] {
            assert!(
                final_events.contains(expected),
                "explicit script-ready and HostLoad turns should finish both child documents; final events: {final_events}"
            );
        }

        server
            .await
            .expect("multi child document wait-path server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_document_script_ready_leaves_complete_and_load_for_later_sources() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__childReadyHostLoadEvents.push("child-script:" + (globalThis === self));
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            bootstrap_source,
            completion_source,
            host_load_pending_after_completion,
            script_ready_source,
            events_after_script_ready,
            lifecycle_ready_after_script,
            interactive_source,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childReadyHostLoadEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => globalThis.__childReadyHostLoadEvents.push("frame-load");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("__childReadyHostLoadEvents.join('|')")?,
                    "",
                    "NavigationCommit should start the child document load without dispatching load"
                );

                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child script-ready realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childReadyHostLoadEvents.join('|')")?;
                let lifecycle_ready_after_script = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(
                        ChildFrameSemanticTurnKind::DocumentLifecycle,
                    );

                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child document iframe load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childReadyHostLoadEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    host_load_pending_after_completion,
                    script_ready_source,
                    events_after_script_ready,
                    lifecycle_ready_after_script,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("page vm child script-ready host-load boundary test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not leave HostLoad pending before document-script ready work runs"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "child document completion follow-up should run document-script ready work"
        );
        assert_eq!(
            events_after_script_ready, "child-script:true",
            "DocumentScriptReady should run the child script but not dispatch iframe load inline"
        );
        assert!(
            lifecycle_ready_after_script,
            "DocumentScriptReady should make the later lifecycle turn runnable"
        );
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "parser EOF should dispatch interactive before HostLoad"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "HostLoad should be the source that dispatches iframe load"
        );
        assert_eq!(
            events_after_host_load, "child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("child document host-load boundary server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_resync_child_commits_navigation_before_document_script_ready() {
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();

    let resync = run_on_page_vm_local_executor(local_executor, async move {
        let mut page_vm = page_vm;
        page_vm.vm_mut().eval_with_child_record_sync(
            r#"
(() => {
  globalThis.__resyncReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __resyncReadyEvents.push("frame-load");
  frame.srcdoc = `<script>parent.__resyncReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
  body.appendChild(frame);
})()
"#,
        )?;
        let host_load_pending_after_resync = page_vm
            .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);

        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "resynced child navigation commit",
        )
        .await;
        run_expected_child_realm_materialization_for_wait(&mut page_vm, "resynced child realm")
            .await;
        let script_ready_source = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await;
        let events_after_script_ready = page_vm.vm_mut().eval("__resyncReadyEvents.join('|')")?;
        let host_load_source = Some(
            run_child_interactive_domcontentloaded_then_host_load_for_wait(
                &mut page_vm,
                "resynced child iframe load",
            )
            .await,
        );
        let events_after_host_load = page_vm.vm_mut().eval("__resyncReadyEvents.join('|')")?;

        Ok::<_, anyhow::Error>((
            host_load_pending_after_resync,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ))
    });
    let (
        host_load_pending_after_resync,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = run_page_vm_async_test(resync)
        .await
        .expect("resync ready-work source test should run");

    assert!(
        !host_load_pending_after_resync,
        "ScriptVm resync should not wake HostLoad while parser-ready work is pending"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "ScriptVm resync should expose parser-ready work after the child navigation commits"
    );
    assert_eq!(
        events_after_script_ready, "child-script:true",
        "DocumentScriptReady should run the child script without dispatching iframe load"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a later HostLoad source"
    );
    assert_eq!(
        events_after_host_load, "child-script:true|frame-load",
        "HostLoad should dispatch iframe load after the script-ready turn"
    );
}

#[tokio::test]
async fn page_vm_input_event_commits_child_navigation_before_document_script_ready() {
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();

    let input = run_on_page_vm_local_executor(local_executor, async move {
        let mut page_vm = page_vm;
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__inputReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const button = document.createElement("button");
  button.textContent = "create child";
  button.style.cssText = "display:block;width:100px;height:20px";
  button.addEventListener("click", () => {
    __inputReadyEvents.push("click");
    const frame = document.createElement("iframe");
    frame.onload = () => __inputReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__inputReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  });
  body.appendChild(button);
})()
"#,
        )?;

        let mouse_down_handled = page_vm
            .vm_mut()
            .dispatch_mouse_event_at_point(10.0, 10.0, "mousedown", 0, None, 0.0, 0.0)?
            .handled;
        let mouse_up_handled = page_vm
            .vm_mut()
            .dispatch_mouse_event_at_point(10.0, 10.0, "mouseup", 0, Some(0), 0.0, 0.0)?
            .handled;
        let events_after_click = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "input-created child navigation commit",
        )
        .await;
        run_expected_child_realm_materialization_for_wait(
            &mut page_vm,
            "input-created child realm",
        )
        .await;
        let script_ready_source = page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await;
        let events_after_script_ready = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;
        let host_load_source = Some(
            run_child_interactive_domcontentloaded_then_host_load_for_wait(
                &mut page_vm,
                "input-created child iframe load",
            )
            .await,
        );
        let events_after_host_load = page_vm.vm_mut().eval("__inputReadyEvents.join('|')")?;

        Ok::<_, anyhow::Error>((
            mouse_down_handled,
            mouse_up_handled,
            events_after_click,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ))
    });
    let (
        mouse_down_handled,
        mouse_up_handled,
        events_after_click,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = run_page_vm_async_test(input)
        .await
        .expect("input ready-work source test should run");

    assert!(mouse_down_handled, "mousedown should hit the button");
    assert!(
        mouse_up_handled,
        "mouseup should hit the button and dispatch click"
    );
    assert_eq!(
        events_after_click, "click",
        "input click should create the child frame without running its parser script inline"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "input-created child parser work should follow its navigation commit"
    );
    assert_eq!(
        events_after_script_ready, "click|child-script:true",
        "child parser work should run on the later DocumentScriptReady turn"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a separate HostLoad turn after input dispatch"
    );
    assert_eq!(
        events_after_host_load, "click|child-script:true|frame-load",
        "iframe load should dispatch only on the HostLoad turn"
    );
}

#[tokio::test]
async fn page_vm_message_port_event_commits_child_navigation_before_document_script_ready() {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let document_url = Url::parse("https://example.com/message-port-child-navigation").unwrap();
    let (page_vm, _resource_source, mut owner_wake_rx) =
        page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
    let local_executor = page_vm.local_executor.clone();
    let selected_task_local_set = tokio::task::LocalSet::new();

    let (
        completion_sources,
        message_port_turns,
        events_after_message,
        script_ready_source,
        events_after_script_ready,
        host_load_source,
        events_after_host_load,
    ) = selected_task_local_set
        .run_until(local_executor.run(async move {
            let mut page_vm = page_vm;
            page_vm.vm_mut().eval(
                r#"
(() => {
  globalThis.__messagePortReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const workerSource = `
    onmessage = (event) => {
      if (event.data !== "connect") {
        return;
      }
      const port = event.ports[0];
      port.postMessage("go");
    };
  `;
  globalThis.__messagePortReadyWorker = new Worker(
    "data:text/javascript," + encodeURIComponent(workerSource)
  );
  globalThis.__messagePortReadyChannel = new MessageChannel();
  const channel = globalThis.__messagePortReadyChannel;
  channel.port1.onmessage = (event) => {
    __messagePortReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __messagePortReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__messagePortReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  };
  globalThis.__messagePortReadyWorker.postMessage("connect", [channel.port2]);
})()
"#,
            )?;

            let mut completion_sources = Vec::new();
            let mut message_port_turns = 0;
            let events_after_message = loop {
                if page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                        &loader,
                    )
                    .await?
                {
                    completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                } else if page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::MessagePortDelivery, &loader)
                    .await?
                {
                    message_port_turns += 1;
                } else if page_vm.has_ready_page_websocket_task_for_test() {
                    let completion_source = page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .expect("ready Worker completion should remain available");
                    completion_sources.push(completion_source);
                } else {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        async {
                            tokio::select! {
                                wake = owner_wake_rx.recv() => wake.is_some(),
                                arrived = page_vm.wait_for_page_websocket_task_for_test() => arrived,
                            }
                        },
                    )
                    .await
                    .expect("MessagePort/worker completion should arrive before timeout");
                    assert!(
                        arrived,
                        "Worker completion sender should remain open"
                    );
                }
                let events = page_vm
                    .vm_mut()
                    .eval("__messagePortReadyEvents.join('|')")?;
                if events == "message:go" {
                    break events;
                }
                assert!(
                    completion_sources.len() + message_port_turns < 16,
                    "MessagePort handler should run after bounded turns; sources: {completion_sources:?}, message_port_turns={message_port_turns}, events: {events}"
                );
            };
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                "MessagePort-created child navigation commit",
            )
            .await;
            run_expected_child_realm_materialization_for_wait(
                &mut page_vm,
                "MessagePort-created child realm",
            )
            .await;
            let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
            let events_after_script_ready = page_vm
                .vm_mut()
                .eval("__messagePortReadyEvents.join('|')")?;
            let host_load_source = Some(
                run_child_interactive_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "MessagePort-created child iframe load",
                )
                .await,
            );
            let events_after_host_load = page_vm
                .vm_mut()
                .eval("__messagePortReadyEvents.join('|')")?;

            Ok::<_, anyhow::Error>((
                completion_sources,
                message_port_turns,
                events_after_message,
                script_ready_source,
                events_after_script_ready,
                host_load_source,
                events_after_host_load,
            ))
        }))
        .await
        .expect("MessagePort ready-work source test should run");

    assert_eq!(
        message_port_turns, 1,
        "one Worker reply should be consumed by one typed MessagePort turn; completion sources: {completion_sources:?}"
    );
    assert_eq!(
        events_after_message, "message:go",
        "MessagePort handler should create the child frame without running its parser script inline"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "MessagePort-created child parser work should follow its navigation commit"
    );
    assert_eq!(
        events_after_script_ready, "message:go|child-script:true",
        "child parser work should run on the later DocumentScriptReady turn"
    );
    assert_eq!(
        host_load_source,
        Some(ChildFrameSemanticTurnKind::HostLoad),
        "iframe load should remain a separate HostLoad turn after MessagePort dispatch"
    );
    assert_eq!(
        events_after_host_load, "message:go|child-script:true|frame-load",
        "iframe load should dispatch only on the HostLoad turn"
    );
}

#[tokio::test]
async fn page_vm_broadcast_channel_commits_child_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/broadcast-ready-worker.js",
            "HTTP/1.1 200 OK",
            r#"
const sender = new BroadcastChannel("broadcast-ready-work");
sender.postMessage("go");
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let (page_vm, _typed_resource_source, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(&loader, document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            completion_sources,
            events_after_broadcast,
            script_ready_source,
            events_after_script_ready,
            host_load_source,
            events_after_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let worker_url = format!("{base_url}/broadcast-ready-worker.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__broadcastReadyEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  globalThis.__broadcastReadyReceiver = new BroadcastChannel("broadcast-ready-work");
  __broadcastReadyReceiver.onmessage = (event) => {{
    __broadcastReadyEvents.push("message:" + event.data);
    const frame = document.createElement("iframe");
    frame.onload = () => __broadcastReadyEvents.push("frame-load");
    frame.srcdoc = `<script>parent.__broadcastReadyEvents.push("child-script:" + (globalThis === self));<\/script>`;
    body.appendChild(frame);
  }};
  globalThis.__broadcastReadyWorker = new Worker("{worker_url}");
}})()
"#
                ))?;

                let mut completion_sources = Vec::new();
                let events_after_broadcast = loop {
                    if page_vm
                        .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::DomManipulation(PageDomManipulationTestFamily::BroadcastChannel),
                            &loader,
                        )
                        .await?
                    {
                        completion_sources
                            .push(RendererOwnerResourceActivitySource::BroadcastChannel);
                        let events = page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                        if events == "message:go" {
                            break events;
                        }
                        continue;
                    }
                    if page_vm
                        .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::DedicatedWorkerClientEvent,
                            &loader,
                        )
                        .await?
                    {
                        completion_sources.push(RendererOwnerResourceActivitySource::Worker);
                        continue;
                    }
                    if !page_vm.has_ready_page_websocket_task_for_test() {
                        let websocket_arrival = page_vm.wait_for_page_websocket_task_for_test();
                        tokio::pin!(websocket_arrival);
                        tokio::time::timeout(Duration::from_secs(2), async {
                            tokio::select! {
                                arrived = &mut websocket_arrival => {
                                    assert!(
                                        arrived,
                                        "BroadcastChannel/worker completion sender should remain open"
                                    );
                                }
                                wake = owner_wake_rx.recv() => {
                                    assert!(
                                        wake.is_some(),
                                        "typed BroadcastChannel owner wake sender should remain open"
                                    );
                                }
                            }
                        })
                        .await
                        .expect("BroadcastChannel/worker work should arrive before timeout");
                        continue;
                    }
                    let completion_source = page_vm
                        .run_exact_page_websocket_selected_task_for_test().await?
                        .expect("BroadcastChannel/worker completion should be ready");
                    completion_sources.push(completion_source);
                    let events = page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                    if events == "message:go" {
                        break events;
                    }
                    assert!(
                        completion_sources.len() < 16,
                        "BroadcastChannel handler should run after bounded completions; sources: {completion_sources:?}, events: {events}"
                    );
                };
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "BroadcastChannel-created child navigation commit",
                )
                .await;
                run_expected_child_realm_materialization_for_wait(
                    &mut page_vm,
                    "BroadcastChannel-created child realm",
                )
                .await;
                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready =
                    page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;
                let host_load_source = Some(
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "BroadcastChannel-created child iframe load",
                    )
                    .await,
                );
                let events_after_host_load =
                    page_vm.vm_mut().eval("__broadcastReadyEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    completion_sources,
                    events_after_broadcast,
                    script_ready_source,
                    events_after_script_ready,
                    host_load_source,
                    events_after_host_load,
                ))
            })
            .await
            .expect("BroadcastChannel ready-work source test should run");

        assert!(
            completion_sources.contains(&RendererOwnerResourceActivitySource::BroadcastChannel),
            "BroadcastChannel handler should be driven by a BroadcastChannel completion: {completion_sources:?}"
        );
        assert_eq!(
            events_after_broadcast, "message:go",
            "BroadcastChannel handler should create the child frame without running its parser script inline"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "BroadcastChannel-created child parser work should follow its navigation commit"
        );
        assert_eq!(
            events_after_script_ready, "message:go|child-script:true",
            "child parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "iframe load should remain a separate HostLoad turn after BroadcastChannel dispatch"
        );
        assert_eq!(
            events_after_host_load, "message:go|child-script:true|frame-load",
            "iframe load should dispatch only on the HostLoad turn"
        );

        server
            .await
            .expect("BroadcastChannel ready-work worker server should finish");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn page_vm_realm_materialization_created_ready_work_enters_document_script_ready_directly() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval_with_child_record_sync(
            r#"
(() => {
  globalThis.__realmMaterializationNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <iframe srcdoc="&lt;script&gt;parent.parent.__realmMaterializationNestedEvents.push('nested-script:' + (globalThis === self));&lt;/script&gt;"></iframe>
    <script>parent.__realmMaterializationNestedEvents.push("outer-script:" + (globalThis === self));<\/script>
  `;
  body.appendChild(frame);
})()
"#,
        )?;

        for label in ["outer", "nested"] {
            run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                &mut page_vm,
                ChildFrameSemanticTurnKind::NavigationCommit,
                &format!("{label} child navigation commit"),
            )
            .await;
        }
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "outer child parser work should run on the DocumentScriptReady source"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__realmMaterializationNestedEvents.join('|')")?,
            "outer-script:true",
            "outer DocumentScriptReady turn should not inline-run nested parser work"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::RealmMaterialization),
            "nested child realm must be materialized in its own visible family turn"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "outer parser EOF should become interactive before nested document work"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "ready work produced by child realm materialization should enter DocumentScriptReady directly"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__realmMaterializationNestedEvents.join('|')")?,
            "outer-script:true|nested-script:true",
            "nested parser work should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "outer DOMContentLoaded should remain a later FIFO turn after nested work already admitted by realm materialization"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("realm materialization nested ready-work source test should run");
}

#[tokio::test]
async fn child_dynamic_inline_script_runs_on_document_script_ready_source() {
    let (events_before_script_ready, script_ready_source, events_after_script_ready, next_source) =
        run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            page_vm.vm_mut().eval(
                r#"
(() => {
  globalThis.__childDynamicReadySourceEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __childDynamicReadySourceEvents.push("frame-load");
  body.appendChild(frame);
  const script = document.createElement("script");
  script.textContent = `
    parent.__childDynamicReadySourceEvents.push(
      "dynamic:" + document.currentScript.tagName.toLowerCase()
    );
  `;
  frame.contentDocument.body.appendChild(script);
})()
"#,
            )?;

            let events_before_script_ready = page_vm
                .vm_mut()
                .eval("__childDynamicReadySourceEvents.join('|')")?;
            run_expected_child_realm_materialization_for_wait(
                &mut page_vm,
                "dynamic child inline-script realm",
            )
            .await;
            let script_ready_source = page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await;
            let events_after_script_ready = page_vm
                .vm_mut()
                .eval("__childDynamicReadySourceEvents.join('|')")?;
            let next_source = page_vm
                .run_next_child_frame_task_source_for_semantic_test()
                .await;

            Ok::<_, anyhow::Error>((
                events_before_script_ready,
                script_ready_source,
                events_after_script_ready,
                next_source,
            ))
        })
        .await
        .expect("child dynamic inline script source test should run");

    assert_eq!(
        events_before_script_ready, "frame-load",
        "the initial about:blank load must dispatch synchronously when the frame is connected"
    );
    assert_eq!(
        script_ready_source,
        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
        "dynamic child inline script should run from the document-script ready source"
    );
    assert_eq!(
        events_after_script_ready, "frame-load|dynamic:script",
        "DocumentScriptReady should execute the dynamic child script after the synchronous initial load"
    );
    assert_eq!(
        next_source, None,
        "synchronous initial about:blank delivery must leave no HostLoad source"
    );
}

#[tokio::test]
async fn child_dynamic_external_classic_script_uses_child_resource_and_ready_owners() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-dynamic-classic.js",
            "HTTP/1.1 200 OK",
            r#"
parent.__childDynamicExternalEvents.push("external:" + (globalThis === self));
parent.__childDynamicExternalEvents.push("current:" + document.currentScript.id);
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_before_completion, completion_source, ready_source, final_events) =
            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let script_url = format!("{base_url}/child-dynamic-classic.js");
                    page_vm.vm_mut().eval(
                        r#"
(() => {
  globalThis.__childDynamicExternalEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.id = "dynamic-external-frame";
  frame.onload = () => __childDynamicExternalEvents.push("frame-load");
  frame.srcdoc = "<!doctype html><html><head><title>child</title></head><body></body></html>";
  body.appendChild(frame);
})()
"#,
                    )?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "dynamic external child navigation commit",
                    )
                    .await;
                    run_expected_child_realm_materialization_for_wait(
                        &mut page_vm,
                        "dynamic external child realm",
                    )
                    .await;
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "dynamic external child initial load",
                    )
                    .await;

                    page_vm.vm_mut().eval(&format!(
                        r#"
(() => {{
  const frame = document.getElementById("dynamic-external-frame");
  const script = frame.contentDocument.createElement("script");
  script.id = "dynamic-external";
  script.onload = () => __childDynamicExternalEvents.push("script-load");
  script.onerror = () => __childDynamicExternalEvents.push("script-error");
  script.src = "{script_url}";
  frame.contentDocument.body.appendChild(script);
}})()
"#,
                    ))?;
                    let events_before_completion = page_vm
                        .vm_mut()
                        .eval("__childDynamicExternalEvents.join('|')")?;
                    if !page_vm
                        .page_resource_completion_queue()
                        .has_ready_completion()
                    {
                        let arrived = tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("child dynamic external completion should arrive before timeout");
                        assert!(
                            arrived,
                            "child dynamic external completion sender should remain open"
                        );
                    }
                    let completion = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    let completion_source = completion.action.source();
                    let ready_source =
                        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                            &mut page_vm,
                            ChildFrameSemanticTurnKind::DocumentScriptReady,
                            "child dynamic external classic execution",
                        )
                        .await;
                    let final_events = page_vm
                        .vm_mut()
                        .eval("__childDynamicExternalEvents.join('|')")?;

                    Ok::<_, anyhow::Error>((
                        events_before_completion,
                        completion_source,
                        ready_source,
                        final_events,
                    ))
                })
                .await
                .expect("child dynamic external script test should run");

        assert_eq!(
            events_before_completion, "frame-load",
            "external script fetch and execution must not run inline with insertion"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildClassicScript,
            "the child classic resource owner should publish the network terminal"
        );
        assert_eq!(
            ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "the loaded external classic script should execute on the child ready source"
        );
        assert_eq!(
            final_events, "frame-load|external:true|current:dynamic-external|script-load",
            "the current child realm should execute once and dispatch load on its script element"
        );

        server
            .await
            .expect("child dynamic external script server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_document_write_nested_external_classic_blocks_domcontentloaded() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-written-classic.js",
            "HTTP/1.1 200 OK",
            "parent.__childWrittenEvents.push('external');".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (events_after_outer, source_load_source, events_after_external, final_events) =
            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let script_url = format!("{base_url}/child-written-classic.js");
                    page_vm.vm_mut().eval(&format!(
                        r#"
(() => {{
  globalThis.__childWrittenEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.onload = () => __childWrittenEvents.push("load");
  frame.srcdoc = `<script>
    parent.__childWrittenEvents.push("outer");
    document.write("<script>parent.__childWrittenEvents.push('nested');<\\/script>");
    document.write("<script src='{script_url}'><\\/script>");
    document.addEventListener("DOMContentLoaded", () => parent.__childWrittenEvents.push("dcl"));
  <\/script>`;
  body.appendChild(frame);
}})()
"#,
                    ))?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::NavigationCommit,
                        "nested document.write child navigation commit",
                    )
                    .await;
                    run_expected_child_realm_materialization_for_wait(
                        &mut page_vm,
                        "nested document.write child realm",
                    )
                    .await;
                    assert_eq!(
                        page_vm
                            .run_next_child_frame_task_source_for_semantic_test()
                            .await,
                        Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
                        "outer parser script should run before its written external source starts"
                    );
                    let events_after_outer =
                        page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;
                    let source_load_source = page_vm
                        .run_next_child_frame_task_source_for_semantic_test()
                        .await;
                    if !page_vm
                        .page_resource_completion_queue()
                        .has_ready_completion()
                    {
                        let arrived = tokio::time::timeout(
                            Duration::from_secs(2),
                            wait_for_typed_page_resource_completion(&mut page_vm),
                        )
                        .await
                        .expect("written external completion should arrive before timeout");
                        assert!(
                            arrived,
                            "written external completion sender should remain open"
                        );
                    }
                    let _ = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::DocumentScriptReady,
                        "written external classic execution",
                    )
                    .await;
                    let events_after_external =
                        page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;
                    run_child_interactive_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "nested document.write child load",
                    )
                    .await;
                    let final_events = page_vm.vm_mut().eval("__childWrittenEvents.join('|')")?;

                    Ok::<_, anyhow::Error>((
                        events_after_outer,
                        source_load_source,
                        events_after_external,
                        final_events,
                    ))
                })
                .await
                .expect("nested child document.write test should run");

        assert_eq!(events_after_outer, "outer|nested");
        assert_eq!(
            source_load_source,
            Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad),
            "the written external script should start before the parser resumes"
        );
        assert_eq!(events_after_external, "outer|nested|external");
        assert_eq!(final_events, "outer|nested|external|dcl|load");

        server
            .await
            .expect("nested child document.write server should finish");
    })
    .await;
}

#[tokio::test]
async fn child_document_close_defers_while_written_external_classic_blocks_parser() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-close-blocker.js",
            "HTTP/1.1 200 OK",
            "parent.__childCloseEvents.push('external:' + Boolean(document.getElementById('after-blocker')));"
                .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            events_after_close,
            source_load_source,
            events_after_external,
            final_events,
            tail_exists,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                page_vm.vm_mut().eval(
                    r#"
(() => {
  globalThis.__childCloseEvents = [];
  const frame = document.createElement("iframe");
  frame.id = "child-close-frame";
  document.body.appendChild(frame);
})()
"#,
                )?;
                materialize_child_realm_through_page_turn_for_test(
                    &mut page_vm,
                    "child-close-frame",
                )?;

                let script_url = format!("{base_url}/child-close-blocker.js");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  const frame = document.getElementById("child-close-frame");
  frame.onload = () => __childCloseEvents.push("load");
  const childDocument = frame.contentDocument;
  childDocument.open();
  childDocument.addEventListener("DOMContentLoaded", () => __childCloseEvents.push("dcl"));
  childDocument.write(`<script src="{script_url}"><\/script><main id="after-blocker">tail</main>`);
  childDocument.close();
  __childCloseEvents.push("after-close");
}})()
"#,
                ))?;
                let events_after_close = page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                let source_load_source =
                    run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                        &mut page_vm,
                        ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
                        "document.close delayed external classic source load",
                    )
                    .await;
                if !page_vm
                    .page_resource_completion_queue()
                    .has_ready_completion()
                {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child close blocker completion should arrive before timeout");
                    assert!(
                        arrived,
                        "child close blocker completion sender should remain open"
                    );
                }
                let _ = run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "document.close delayed external classic execution",
                )
                .await;
                let events_after_external =
                    page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                run_child_interactive_domcontentloaded_then_host_load_for_wait(
                    &mut page_vm,
                    "document.close delayed child load",
                )
                .await;
                let final_events = page_vm.vm_mut().eval("__childCloseEvents.join('|')")?;
                let tail_exists = page_vm.vm_mut().eval(
                    "String(Boolean(document.getElementById('child-close-frame').contentDocument.getElementById('after-blocker')))",
                )?;

                Ok::<_, anyhow::Error>((
                    events_after_close,
                    source_load_source,
                    events_after_external,
                    final_events,
                    tail_exists,
                ))
            })
            .await
            .expect("child document.close delayed EOF test should run");

        assert_eq!(
            events_after_close, "after-close",
            "document.close() should return without executing or bypassing the blocker"
        );
        assert_eq!(
            source_load_source,
            ChildFrameSemanticTurnKind::ClassicScriptSourceLoad
        );
        assert_eq!(
            events_after_external, "after-close|external:false",
            "the external parser-blocking script must run before future markup is parsed"
        );
        assert_eq!(final_events, "after-close|external:false|dcl|load");
        assert_eq!(tail_exists, "true");

        server
            .await
            .expect("child close blocker server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_nested_frame_finish_releases_parent_document_lifecycle() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        page_vm.vm_mut().eval(
            r#"
(() => {
  globalThis.__nestedLifecycleEvents = [];
  const outer = document.createElement("iframe");
  outer.onload = () => __nestedLifecycleEvents.push("outer-load");
  outer.srcdoc = `<!doctype html><body><script>
    parent.__nestedLifecycleEvents.push("outer-script");
    const nested = document.createElement("iframe");
    nested.onload = () => parent.parent.__nestedLifecycleEvents.push("nested-load");
    nested.srcdoc = \`<!doctype html><script>parent.parent.__nestedLifecycleEvents.push("nested-script")<\\/script>\`;
    document.body.appendChild(nested);
  <\/script></body>`;
  document.body.appendChild(outer);
})()
"#,
        )?;

        let mut turns = Vec::new();
        for _ in 0..32 {
            let Some(source) = page_vm.run_next_child_frame_task_source_for_semantic_test().await else {
                break;
            };
            let events = page_vm
                .vm_mut()
                .eval("__nestedLifecycleEvents.join('|')")?;
            let outer_loaded = events.contains("outer-load");
            turns.push((source, events));
            if outer_loaded {
                break;
            }
        }

        let nested_load_turn = turns
            .iter()
            .position(|(_, events)| events.contains("nested-load"))
            .expect("nested frame should finish");
        let outer_load_turn = turns
            .iter()
            .position(|(_, events)| events.contains("outer-load"))
            .expect("parent frame should finish after its descendant");
        assert_eq!(turns[nested_load_turn].0, ChildFrameSemanticTurnKind::HostLoad);
        assert_eq!(turns[outer_load_turn].0, ChildFrameSemanticTurnKind::HostLoad);
        assert!(
            nested_load_turn < outer_load_turn,
            "parent load must remain blocked until the exact descendant frame finishes: {turns:?}"
        );
        assert!(
            !turns[nested_load_turn].1.contains("outer-load"),
            "descendant HostLoad must only enqueue the parent lifecycle wake: {turns:?}"
        );
        assert!(
            turns[nested_load_turn + 1..outer_load_turn]
                .iter()
                .any(|(source, _)| *source == ChildFrameSemanticTurnKind::DocumentLifecycle),
            "parent complete must be a later DocumentLifecycle turn before its HostLoad: {turns:?}"
        );
        assert_eq!(
            turns[outer_load_turn].1,
            "outer-script|nested-script|nested-load|outer-load"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("nested lifecycle source-turn proof should run");
}

#[tokio::test]
async fn page_vm_host_load_commits_nested_navigation_before_document_script_ready() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-creates-nested.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__hostLoadNestedEvents.push("child-script");
onload = () => {
  parent.__hostLoadNestedEvents.push("child-load");
  const nested = document.createElement("iframe");
  nested.onload = () => parent.__hostLoadNestedEvents.push("nested-load");
  nested.srcdoc = `<script>parent.parent.__hostLoadNestedEvents.push("nested-script:" + (globalThis === self));<\/script>`;
  document.body.appendChild(nested);
};
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let local_executor = page_vm.local_executor.clone();

        let (
            bootstrap_source,
            completion_source,
            host_load_pending_after_completion,
            script_ready_source,
            events_after_script_ready,
            interactive_source,
            host_load_source,
            events_after_host_load,
            nested_script_ready_source,
            events_after_nested_script_ready,
            nested_interactive_source,
            nested_host_load_source,
            events_after_nested_host_load,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-load-creates-nested.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__hostLoadNestedEvents = [];
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                let host_load_pending_after_completion = page_vm
                    .has_ready_child_frame_semantic_turn_for_test(ChildFrameSemanticTurnKind::HostLoad);
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child host-load realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child window load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "nested child navigation created during HostLoad",
                )
                .await;
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "nested child realm prerequisite",
                )?;
                let nested_script_ready_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_nested_script_ready = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;
                let nested_interactive_source =
                    page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let nested_host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "nested child iframe load",
                    )
                    .await,
                );
                let events_after_nested_host_load = page_vm
                    .vm_mut()
                    .eval("__hostLoadNestedEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    host_load_pending_after_completion,
                    script_ready_source,
                    events_after_script_ready,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                    nested_script_ready_source,
                    events_after_nested_script_ready,
                    nested_interactive_source,
                    nested_host_load_source,
                    events_after_nested_host_load,
                ))
            })
            .await
            .expect("host-load nested ready-work source test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert!(
            !host_load_pending_after_completion,
            "child document completion should not leave HostLoad pending before document-script ready work runs"
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "child document completion follow-up should run document-script ready work"
        );
        assert_eq!(events_after_script_ready, "child-script");
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "child parser EOF should become interactive before window load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "child window load should dispatch on HostLoad"
        );
        assert_eq!(
            events_after_host_load, "child-script|child-load",
            "HostLoad should create the nested frame without running its script inline"
        );
        assert_eq!(
            nested_script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady),
            "nested parser work created during HostLoad should follow its navigation commit"
        );
        assert_eq!(
            events_after_nested_script_ready, "child-script|child-load|nested-script:true",
            "nested script should run on the later DocumentScriptReady turn"
        );
        assert_eq!(
            nested_interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "nested parser EOF should become interactive before nested HostLoad"
        );
        assert_eq!(
            nested_host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "nested iframe load should still remain a separate HostLoad turn"
        );
        assert_eq!(
            events_after_nested_host_load,
            "child-script|child-load|nested-script:true|nested-load"
        );

        server
            .await
            .expect("host-load nested ready-work server should finish");
    })
    .await;
}

#[tokio::test]
async fn page_vm_child_window_load_navigation_uses_navigation_commit_followup() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/child-load-navigate.html",
            "HTTP/1.1 200 OK",
            r#"
<script>
parent.__childLoadNavigationEvents.push("child-script");
const clickOrder = [];
addEventListener("click", event => {
  clickOrder.push("before");
  onclick = () => {
    clickOrder.push("property");
    return false;
  };
  event.stopPropagation();
});
onclick = () => clickOrder.push("stale");
addEventListener("click", () => clickOrder.push("after"));
const clickDispatchResult = document.dispatchEvent(new Event("click", {
  bubbles: true,
  cancelable: true
}));
parent.__childLoadNavigationEvents.push("click:" + clickOrder.join(",") + ":" + clickDispatchResult);
onerror = function(message, source, line, column, error, extra) {
  parent.__childLoadNavigationEvents.push([
    "child-error",
    arguments.length,
    message,
    source.endsWith("child-load-navigate.html"),
    line > 0,
    column > 0,
    error && error.message,
    extra === undefined
  ].join(":"));
  return true;
};
const marker = new Error("child-error-marker");
const errorDispatchResult = dispatchEvent(new ErrorEvent("error", {
  cancelable: true,
  message: marker.message,
  filename: location.href,
  lineno: 7,
  colno: 9,
  error: marker
}));
parent.__childLoadNavigationEvents.push("error-dispatch:" + errorDispatchResult);
addEventListener("load", () => parent.__childLoadNavigationEvents.push("load-listener-before"));
onload = () => {
  parent.__childLoadNavigationEvents.push("child-load");
  location.href = "data:text/html,<!doctype html><script>parent.postMessage('can navigate', '*')<\/script>";
};
addEventListener("load", () => parent.__childLoadNavigationEvents.push("load-listener-after"));
</script>
"#
            .to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse(&format!("{base_url}/page")).expect("page url");
        let page_vm = test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url);
        let loader_for_task = loader.clone();
        let local_executor = page_vm.local_executor.clone();

        let (
            bootstrap_source,
            completion_source,
            script_ready_source,
            events_after_script_ready,
            interactive_source,
            host_load_source,
            events_after_host_load,
            pending_after_host_load,
            navigation_commit_source,
            events_after_navigation_commit,
            navigation_script_ready_source,
            events_after_navigation_script_ready,
            message_ready_after_navigation_script,
            message_endpoints_after_navigation_script,
            events_after_window_message,
        ) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let child_url = format!("{base_url}/child-load-navigate.html");
                page_vm.vm_mut().eval(&format!(
                    r#"
(() => {{
  globalThis.__childLoadNavigationEvents = [];
  addEventListener("message", event => __childLoadNavigationEvents.push(event.data));
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.src = "{child_url}";
  body.appendChild(frame);
}})()
"#
                ))?;

                let bootstrap_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                if !page_vm.page_resource_completion_queue().has_ready_completion() {
                    let arrived = tokio::time::timeout(
                        Duration::from_secs(2),
                        wait_for_typed_page_resource_completion(&mut page_vm),
                    )
                    .await
                    .expect("child document completion should arrive before timeout");
                    assert!(arrived, "child document completion sender should remain open");
                }
                let completion =
                    run_next_resource_completion_as_typed_page_turn(&mut page_vm)?;
                let completion_source = completion.action.source();
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child window-load realm prerequisite",
                )?;

                let script_ready_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let events_after_script_ready = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;

                let interactive_source = page_vm.run_next_child_frame_task_source_for_semantic_test().await;
                let host_load_source = Some(
                    run_child_domcontentloaded_then_host_load_for_wait(
                        &mut page_vm,
                        "child window-load navigation initial load",
                    )
                    .await,
                );
                let events_after_host_load = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                let pending_after_host_load = page_vm
                    .vm()
                    .has_pending_child_navigation_commit_for_test();
                let navigation_commit_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "child window load navigation commit",
                )
                .await;
                let events_after_navigation_commit = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                run_expected_pending_child_realm_materialization_turn(
                    &mut page_vm,
                    "child window-load navigation realm prerequisite",
                )?;
                let navigation_script_ready_source = run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::DocumentScriptReady,
                    "child window load navigation script",
                )
                .await;
                let events_after_navigation_script_ready = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;
                let message_endpoints_after_navigation_script =
                    page_vm.vm().pending_window_message_endpoints_for_test();
                let message_ready_after_navigation_script = page_vm
                    .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WindowMessage, &loader_for_task)
                    .await?;
                let events_after_window_message = page_vm
                    .vm_mut()
                    .eval("__childLoadNavigationEvents.join('|')")?;

                Ok::<_, anyhow::Error>((
                    bootstrap_source,
                    completion_source,
                    script_ready_source,
                    events_after_script_ready,
                    interactive_source,
                    host_load_source,
                    events_after_host_load,
                    pending_after_host_load,
                    navigation_commit_source,
                    events_after_navigation_commit,
                    navigation_script_ready_source,
                    events_after_navigation_script_ready,
                    message_ready_after_navigation_script,
                    message_endpoints_after_navigation_script,
                    events_after_window_message,
                ))
            })
            .await
            .expect("child load navigation host-load follow-up test should run");

        assert_eq!(
            bootstrap_source,
            Some(ChildFrameSemanticTurnKind::NavigationCommit),
            "bootstrap child frame turn should commit navigation before lifecycle delivery"
        );
        assert_eq!(
            completion_source,
            RendererOwnerResourceActivitySource::ChildDocument
        );
        assert_eq!(
            script_ready_source,
            Some(ChildFrameSemanticTurnKind::DocumentScriptReady)
        );
        assert_eq!(
            events_after_script_ready,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false",
            "DocumentScriptReady should not dispatch child window load inline"
        );
        assert_eq!(
            interactive_source,
            Some(ChildFrameSemanticTurnKind::DocumentLifecycle),
            "child document should become interactive before its window load"
        );
        assert_eq!(
            host_load_source,
            Some(ChildFrameSemanticTurnKind::HostLoad),
            "child window load should dispatch on a HostLoad turn"
        );
        assert_eq!(
            events_after_host_load,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "HostLoad should invoke one registration-ordered child window load dispatch without finishing navigation inline"
        );
        assert!(
            pending_after_host_load,
            "child window load navigation should leave follow-up child frame work"
        );
        assert_eq!(
            navigation_commit_source,
            ChildFrameSemanticTurnKind::NavigationCommit,
            "navigation queued by child window load should commit through NavigationCommit"
        );
        assert_eq!(
            events_after_navigation_commit,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "NavigationCommit should not deliver the child postMessage inline"
        );
        assert_eq!(
            navigation_script_ready_source,
            ChildFrameSemanticTurnKind::DocumentScriptReady,
            "data-url navigation script should run from DocumentScriptReady"
        );
        assert_eq!(
            events_after_navigation_script_ready,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after",
            "navigation script should queue a parent window-message task without delivering it inline"
        );
        assert!(
            message_ready_after_navigation_script,
            "child navigation script should queue a ready parent window-message task"
        );
        assert_eq!(
            message_endpoints_after_navigation_script.len(),
            1,
            "child navigation script should queue exactly one window-message task: {message_endpoints_after_navigation_script:?}"
        );
        assert!(
            matches!(
                message_endpoints_after_navigation_script.as_slice(),
                [(
                    PendingWindowMessageEndpoint::TopWindow,
                    PendingWindowMessageEndpoint::ChildWindow(_)
                )]
            ),
            "child navigation script should target the top window from the child: {message_endpoints_after_navigation_script:?}"
        );
        assert_eq!(
            events_after_window_message,
            "child-script|click:before,property,after:false|child-error:5:child-error-marker:true:true:true:child-error-marker:true|error-dispatch:false|load-listener-before|child-load|load-listener-after|can navigate",
            "advancing the ready window-message task should deliver the child navigation postMessage"
        );

        server
            .await
            .expect("child load navigation host-load follow-up server should finish");
    })
    .await;
}

#[tokio::test]
async fn stream_declared_controller_and_writer_surface_ignores_reflection_and_spoofing() {
    run_page_vm_async_test(async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        const methodShape = (object, name) => {
                            const descriptor = Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(object),
                                name
                            );
                            return [
                                !!descriptor,
                                descriptor && descriptor.enumerable,
                                descriptor && descriptor.configurable,
                                descriptor && descriptor.writable,
                                descriptor && typeof descriptor.value,
                                descriptor && descriptor.value.length,
                                descriptor && descriptor.value.name
                            ].join(":");
                        };
                        const accessorShape = (object, name) => {
                            const descriptor = Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(object),
                                name
                            );
                            return [
                                !!descriptor,
                                descriptor && descriptor.enumerable,
                                descriptor && descriptor.configurable,
                                descriptor && ("writable" in descriptor),
                                descriptor && typeof descriptor.get,
                                descriptor && descriptor.get.name,
                                descriptor && descriptor.get.length,
                                descriptor && typeof descriptor.set
                            ].join(":");
                        };
                        const internals = [
                            "__moliStreamControllerStream",
                            "__moliWritableStreamWriterStream"
                        ];
                        const reflectedInternals = object => Object.getOwnPropertyNames(object)
                            .filter(name => internals.includes(name))
                            .join(",");
                        const throwsTypeError = callback => {
                            try {
                                callback();
                                return "no-throw";
                            } catch (error) {
                                return `throw:${error.name}`;
                            }
                        };

                        let readableController;
                        const readable = new ReadableStream({
                            start(controller) {
                                readableController = controller;
                            }
                        }, { highWaterMark: 2 });
                        if (!readableController) {
                            throw new Error("ReadableStream start controller was not captured");
                        }
                        const readableReflectedBefore = reflectedInternals(readableController);
                        const readableDesiredGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(readableController),
                                "desiredSize"
                            ).get;
                        const readableBefore = readableController.desiredSize;
                        readableController.enqueue("one");
                        const readableAfter = readableController.desiredSize;
                        const readableFake = throwsTypeError(() => readableDesiredGetter.call({
                            __moliStreamControllerStream: readable
                        }));
                        readableController.__moliStreamControllerStream = new ReadableStream(
                            {},
                            { highWaterMark: 99 }
                        );
                        const readableSpoofed = readableController.desiredSize;

                        let transformController;
                        const transform = new TransformStream({
                            start(controller) {
                                transformController = controller;
                            }
                        }, undefined, { highWaterMark: 3 });
                        if (!transformController) {
                            throw new Error("TransformStream transform controller was not captured");
                        }
                        transformController.enqueue("chunk");
                        const transformReflectedBefore = reflectedInternals(transformController);
                        const transformBeforeSpoof = transformController.desiredSize;
                        transformController.__moliStreamControllerStream = new ReadableStream(
                            {},
                            { highWaterMark: 99 }
                        );
                        const transformSpoofed = transformController.desiredSize;

                        const writable = new WritableStream();
                        const writer = writable.getWriter();
                        const writerReflectedBefore = reflectedInternals(writer);
                        const writerReadyGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(writer),
                                "ready"
                            ).get;
                        const writerDesiredGetter =
                            Object.getOwnPropertyDescriptor(
                                Object.getPrototypeOf(writer),
                                "desiredSize"
                            ).get;
                        const writerDesiredBefore = writer.desiredSize;
                        const writerReadyIsPromise = writer.ready instanceof Promise;
                        const writerClosedIsPromise = writer.closed instanceof Promise;
                        const writerFakeReady = writerReadyGetter.call({
                            __moliWritableStreamWriterStream: writable
                        }) instanceof Promise;
                        const writerFakeDesired = throwsTypeError(() =>
                            writerDesiredGetter.call({
                                __moliWritableStreamWriterStream: writable
                            })
                        );
                        writer.__moliWritableStreamWriterStream = new WritableStream();
                        const writerDesiredSpoofed = writer.desiredSize;

                        return [
                            readableReflectedBefore,
                            methodShape(readableController, "enqueue"),
                            methodShape(readableController, "close"),
                            accessorShape(readableController, "desiredSize"),
                            `${readableBefore}:${readableAfter}:${readableFake}:${readableSpoofed}`,
                            transformReflectedBefore,
                            methodShape(transformController, "enqueue"),
                            String(Object.hasOwn(
                                Object.getPrototypeOf(transformController),
                                "close"
                            )),
                            accessorShape(transformController, "desiredSize"),
                            `${transformBeforeSpoof}:${transformSpoofed}`,
                            writerReflectedBefore,
                            accessorShape(writer, "ready"),
                            accessorShape(writer, "closed"),
                            accessorShape(writer, "desiredSize"),
                            `${writerDesiredBefore}:${writerReadyIsPromise}:${writerClosedIsPromise}:${writerFakeReady}:${writerFakeDesired}:${writerDesiredSpoofed}`
                        ].join("\n");
                    })()
                    "#,
                )
            })
            .await
            .expect("stream declared surface test should run on owner lane");

        assert_eq!(
            result,
            [
                "",
                "true:true:true:true:function:0:enqueue",
                "true:true:true:true:function:0:close",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "2:1:throw:TypeError:1",
                "",
                "true:true:true:true:function:0:enqueue",
                "false",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "2:2",
                "",
                "true:true:true:false:function:get ready:0:undefined",
                "true:true:true:false:function:get closed:0:undefined",
                "true:true:true:false:function:get desiredSize:0:undefined",
                "1:true:true:true:throw:TypeError:1",
            ]
            .join("\n")
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_inline_module_runs_from_parser_after_parsing_order() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let script = prepared_inline_module_for_page_vm_test(
            &page_vm,
            9002,
            "globalThis.__inlineParserModuleExecuted = (globalThis.__inlineParserModuleExecuted ?? 0) + 1; export const value = 1;",
        );

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__inlineParserModuleExecuted)")
                .expect("read module side effect before page task"),
            "undefined",
            "prewarmed inline graph must not evaluate before parser after-parsing release"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release inline graph");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "inline parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__inlineParserModuleExecuted)")
                .expect("read module side effect after page task"),
            "1",
            "parser after-parsing release should evaluate the prewarmed inline graph once"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_data_url_module_runs_from_parser_after_parsing_order() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse(
            "data:text/javascript,globalThis.__dataParserModuleExecuted%3D1%3Bexport%20const%20value%3D1%3B",
        )
        .expect("data module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url);

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dataParserModuleExecuted)")
                .expect("read data module side effect before page task"),
            "undefined",
            "prepared data graph must not evaluate before parser after-parsing release"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release the data module graph");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "data parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dataParserModuleExecuted)")
                .expect("read data module side effect after page task"),
            "1",
            "parser after-parsing release should evaluate the prepared data graph once"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_pending_module_tree_fetch_completion_waits_for_after_parsing_release() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__pendingTreeExecuted = (globalThis.__pendingTreeExecuted ?? 0) + 1; export const value = 1;",
        );

        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some(),
            "module graph completion should be consumed"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeExecuted)")
                .expect("read module side effect before page task"),
            "undefined",
            "completed fetch tree must not evaluate before the parser script task watches it"
        );

        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should release the pending tree");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "completed parser module after-parsing release",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeExecuted)")
                .expect("read module side effect after page task"),
            "1",
            "parser script task watch should evaluate the completed pending tree once"
        );
        assert!(
            page_vm
                .report
                .runs
                .iter()
                .any(|run| run.url() == &module_url),
            "module script run should be reported after watch: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn watched_loading_parser_pending_module_tree_runs_after_fetch_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/late-module.mjs").expect("module URL");
        let script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser after-parsing owner should retain the loading pending tree");
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeLateExecuted)")
                .expect("read module side effect before fetch completion"),
            "undefined",
            "loading watch must wait for the fetch tree completion"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__pendingTreeLateExecuted = (globalThis.__pendingTreeLateExecuted ?? 0) + 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some(),
            "module graph completion should be consumed"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "after-parsing terminal must stay in its PendingScript instead of the broad ready queue"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "late parser module graph completion",
        )
        .await;
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "late parser module graph completion",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__pendingTreeLateExecuted)")
                .expect("read module side effect after fetch completion"),
            "1",
            "fetch completion after watch should evaluate the pending tree once"
        );
        assert!(
            page_vm
                .report
                .runs
                .iter()
                .any(|run| run.url() == &module_url),
            "module script run should be reported after late completion: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_module_tla_releases_lifecycle_at_evaluation_start() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-tla.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/main-parser-tla.mjs").expect("module URL");
        let script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9050, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "the accepted module PendingScript must own a lifecycle token before fetch starts"
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("parser module owner should wait for its graph");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            concat!(
                "globalThis.__mainParserTlaStarted = 1;",
                "await new Promise(resolve => { globalThis.__resolveMainParserTla = resolve; });",
                "globalThis.__mainParserTlaFinished = 1; export const value = 1;",
            ),
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("TLA module graph completion should apply")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "main parser TLA evaluation start",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaStarted)")
                .expect("read TLA start"),
            "1"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaFinished)")
                .expect("read suspended TLA body"),
            "undefined"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "the TLA promise must not retain parser-deferred lifecycle ownership"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "starting module evaluation must apply script completion exactly once"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__resolveMainParserTla(); 'resolved'")
            .expect("resolve main parser TLA");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            1,
            "main parser TLA fulfillment",
        )
        .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__mainParserTlaFinished)")
                .expect("read fulfilled TLA body"),
            "1"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "TLA fulfillment must not redispatch script completion"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_module_tla_rejection_reports_without_duplicate_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/main-tla-rejection.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval(
                "globalThis.__mainParserTlaErrors = []; addEventListener('error', event => { __mainParserTlaErrors.push((event.error?.constructor?.name ?? 'none') + ':' + event.message); event.preventDefault(); });",
            )
            .expect("install main TLA error observer");
        let module_url = Url::parse("https://example.com/main-parser-tla-rejection.mjs")
            .expect("module URL");
        let script = prepared_external_module_for_page_vm_test_with_node(
            &page_vm,
            9051,
            module_url.clone(),
        );
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("parser module owner should wait for its graph");
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            concat!(
                "await new Promise((_, reject) => {",
                "  globalThis.__rejectMainParserTla = reject;",
                "}); export const value = 1;",
            ),
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("TLA rejection graph completion should apply")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "main parser TLA rejection evaluation start",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "pending rejection must not retain parser lifecycle ownership"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__rejectMainParserTla(new TypeError('main TLA rejected')); 'rejected'")
            .expect("reject main parser TLA");
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            1,
            "main parser TLA rejection",
        )
        .await;
        let error = page_vm
            .vm_mut()
            .eval("__mainParserTlaErrors.join('|')")
            .expect("read TLA rejection event");
        assert!(
            error.contains("TypeError:") && error.contains("main TLA rejected"),
            "TLA rejection must retain typed window-error metadata: {error}"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url() == &module_url)
                .count(),
            1,
            "TLA rejection must not redispatch or rerecord script completion"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_runtime_module_tla_releases_lifecycle_at_evaluation_start() {
    #[derive(Debug)]
    enum RuntimeModuleTurn {
        RuntimeScriptAdmission,
        ParserAsyncModuleAdmission,
        ModuleReaction,
        NativeModuleOwner,
        DynamicModuleJob,
        RuntimeScriptContinuation,
        RuntimeOwnedModuleContinuation,
        ParserOwnedModuleContinuation,
        PostParseWork,
    }

    async fn run_one_runtime_module_turn(
        page_vm: &mut PageVm,
        loader: &crate::network::ResourceRequestClient,
    ) -> anyhow::Result<Option<RuntimeModuleTurn>> {
        if page_vm
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ModuleReaction,
                loader,
            )
            .await?
        {
            return Ok(Some(RuntimeModuleTurn::ModuleReaction));
        }
        if let Some(outcome) = page_vm
            .run_page_main_document_runtime_body_for_test(loader)
            .await?
        {
            let turn = match outcome.action.kind() {
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptAdmission => {
                    RuntimeModuleTurn::RuntimeScriptAdmission
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserAsyncModuleAdmission => {
                    RuntimeModuleTurn::ParserAsyncModuleAdmission
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeScriptContinuation => {
                    RuntimeModuleTurn::RuntimeScriptContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::DynamicModuleJob => {
                    RuntimeModuleTurn::DynamicModuleJob
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::RuntimeOwnedModuleContinuation => {
                    RuntimeModuleTurn::RuntimeOwnedModuleContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::ParserOwnedModuleContinuation => {
                    RuntimeModuleTurn::ParserOwnedModuleContinuation
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::NativeModuleOwnerEvent => {
                    RuntimeModuleTurn::NativeModuleOwner
                }
                crate::page_task_queue::PageMainDocumentRuntimeActionKind::PostParseWork => {
                    RuntimeModuleTurn::PostParseWork
                }
            };
            return Ok(Some(turn));
        }
        Ok(None)
    }

    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/runtime-tla.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .document_runtime
            .note_dom_content_loaded_dispatched();
        page_vm
            .vm_mut()
            .eval(
                r#"
(() => {
  globalThis.__runtimeTlaEvents = [];
  const script = document.createElement("script");
  script.type = "module";
  script.onload = () => __runtimeTlaEvents.push("script-load");
  script.onerror = () => __runtimeTlaEvents.push("script-error");
  script.textContent = `
    globalThis.__runtimeTlaStarted = 1;
    await new Promise(resolve => { globalThis.__resolveRuntimeTla = resolve; });
    globalThis.__runtimeTlaFinished = 1;
  `;
  document.body.appendChild(script);
})()
"#,
            )
            .expect("install runtime TLA module");
        let owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main document owner");

        let mut reached_evaluation_start = false;
        let mut progress_actions = Vec::new();
        for _ in 0..32 {
            let progressed = run_one_runtime_module_turn(&mut page_vm, &loader).await?;
            let started = page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaStarted)")?;
            let load_delay = page_vm
                .vm()
                .current_main_document_has_async_script_load_delay(owner);
            let events = page_vm.vm_mut().eval("__runtimeTlaEvents.join('|')")?;
            if started == "1" && load_delay == Some(false) {
                assert_eq!(
                    events, "",
                    "inline module evaluation must not dispatch script load or error"
                );
                reached_evaluation_start = true;
                break;
            }
            let Some(progressed) = progressed else {
                let ready_module_continuation =
                    page_vm.has_ready_runtime_owned_module_script_continuation_work();
                let ready_dynamic_module_job = page_vm.vm_mut().has_ready_dynamic_module_job();
                let runnable_runtime_work = page_vm
                    .vm_mut()
                    .has_runnable_runtime_script_work_now();
                panic!(
                    "runtime module owner loop stalled before evaluation-start completion: started={started} load_delay={load_delay:?} events={events:?} progress_actions={progress_actions:?} ready_module_continuation={ready_module_continuation} ready_dynamic_module_job={ready_dynamic_module_job} runnable_runtime_work={runnable_runtime_work}",
                );
            };
            progress_actions.push(progressed);
        }
        assert!(
            reached_evaluation_start,
            "runtime TLA evaluation start must complete the script and release its lifecycle binding"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?,
            "undefined",
            "the module body should still be suspended on its TLA promise"
        );
        let run_count_at_evaluation_start = page_vm
            .report
            .runs
            .iter()
            .filter(|run| run.url().as_str().contains("runtime-tla"))
            .count();
        assert_eq!(
            run_count_at_evaluation_start, 1,
            "evaluation start must apply runtime module script completion exactly once"
        );

        page_vm
            .vm_mut()
            .eval("globalThis.__resolveRuntimeTla(); 'resolved'")?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::ModuleReaction, &loader)
                .await?,
            "runtime TLA fulfillment should enqueue one typed module reaction"
        );
        for _ in 0..32 {
            let progressed = run_one_runtime_module_turn(&mut page_vm, &loader).await?;
            if page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?
                == "1"
            {
                break;
            }
            assert!(
                progressed.is_some(),
                "runtime module owner loop stalled after TLA resolve"
            );
        }
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__runtimeTlaFinished)")?,
            "1"
        );
        assert_eq!(
            page_vm
                .report
                .runs
                .iter()
                .filter(|run| run.url().as_str().contains("runtime-tla"))
                .count(),
            run_count_at_evaluation_start,
            "TLA fulfillment must not redispatch runtime module script completion"
        );
        assert_eq!(
            page_vm.vm_mut().eval("__runtimeTlaEvents.join('|')")?,
            "",
            "inline module fulfillment must not dispatch script load or error"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("runtime TLA lifecycle test should run");
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_dispatches_load_before_releasing_next_pending_script() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-order.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferEvents = []")
            .expect("defer event state should initialize");
        let first = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "first-defer",
            Url::parse("https://example.com/first-defer.js").expect("first script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferEvents.push('first-exec:' + document.currentScript.id);"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferEvents.push('first-load:' + (document.currentScript === null))",
            ),
        );
        let second = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            2,
            "second-defer",
            Url::parse("https://example.com/second-defer.js").expect("second script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferEvents.push('second-exec:' + document.currentScript.id);"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferEvents.push('second-load:' + (document.currentScript === null))",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer test requires a document owner");
        for script in [first, second] {
            assert!(
                page_vm
                    .vm_mut()
                    .claim_main_parser_deferred_script(
                        task_owner,
                        script,
                        None,
                        None,
                        Default::default(),
                    )
                    .expect("loaded classic defer should be accepted")
            );
        }
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "accepted classic-defer PendingScripts must own the main lifecycle gate"
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");
        assert!(
            !page_vm.has_pending_module_script_for_target_stage(),
            "classic defer lifecycle ownership must not enter the runtime-owned module lane"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "first classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferEvents.join('|')")
                .expect("first defer events should evaluate"),
            "first-exec:first-defer|first-load:true",
            "the first PendingScript must dispatch load with currentScript cleared before the next parser-deferred slot is released"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "the second PendingScript must retain its own lifecycle token"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "second classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferEvents.join('|')")
                .expect("all defer events should evaluate"),
            "first-exec:first-defer|first-load:true|second-exec:second-defer|second-load:true"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "consuming the final PendingScript must release the lifecycle gate"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_settles_script_reactions_before_load() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-checkpoint-order.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferCheckpointEvents = []")
            .expect("defer checkpoint state should initialize");
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "checkpoint-defer",
            Url::parse("https://example.com/checkpoint-defer.js").expect("script URL"),
            ScriptSource::Loaded(
                r#"
globalThis.__mainParserDeferCheckpointEvents.push('script');
queueMicrotask(() => globalThis.__mainParserDeferCheckpointEvents.push('script-microtask'));
"#
                .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferCheckpointEvents.push('load'); queueMicrotask(() => globalThis.__mainParserDeferCheckpointEvents.push('load-microtask'))",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer checkpoint test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    None,
                    None,
                    Default::default(),
                )
                .expect("loaded classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");

        run_and_finish_ready_parser_deferred_task_for_test(
            &mut page_vm,
            &loader,
            "classic defer checkpoint ordering",
        )
        .await;

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferCheckpointEvents.join('|')")
                .expect("defer checkpoint events should evaluate"),
            "script|script-microtask|load|load-microtask",
            "classic-defer evaluation reactions must settle before load and the selected parser task must then settle load reactions"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn dcl_lifecycle_yields_parser_deferred_source_wait_to_page_vm_resource_queue() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-arrival.html").expect("document URL"),
        );
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "deferred-arrival",
            Url::parse("https://example.com/deferred-arrival.js").expect("script URL"),
            ScriptSource::External,
            ("onload", "globalThis.__deferredArrivalLoaded = true"),
        );
        let (source_ready_tx, source_ready_rx) = tokio::sync::oneshot::channel();
        let source_load = SharedScriptSourceLoad::spawn_for_test(async move {
            source_ready_rx
                .await
                .expect("test should release the deferred source");
            Ok("globalThis.__deferredArrivalExecuted = true;".to_owned())
        });
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("defer test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    Some(source_load),
                    None,
                    Default::default(),
                )
                .expect("pending classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("defer queue should seal");
        let lifecycle_driver = page_vm
            .vm()
            .resume_post_parse_lifecycle_driver_for_existing_queue(
                PageVmInitStage::DomContentLoaded,
            );

        let advance = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut page_vm;
            vm.as_mut()
                .expect("page vm must retain a ScriptVm")
                .advance_post_parse_lifecycle(
                    &loader,
                    page_task_queue,
                    report,
                    lifecycle_driver,
                    None,
                )
                .await
                .expect("DCL driver should report its blocked state")
        };
        assert!(
            matches!(advance, PostParseLifecycleAdvance::AwaitProgress),
            "the document driver must yield future resource progress to PageVm arbitration"
        );

        page_vm
            .vm_mut()
            .eval(
                "globalThis.__dclBlockedTimerRan = false; \
                 setTimeout(() => { globalThis.__dclBlockedTimerRan = true; }, 0);",
            )
            .expect("ready timer should be registered");
        assert!(
            page_vm.vm().has_ready_timeout(),
            "the regression requires a ready timer competing with the deferred source"
        );
        let progress_arrived = {
            let progress_wait =
                page_vm.wait_for_lifecycle_blocking_page_work_arrival_without_timeout(false);
            tokio::pin!(progress_wait);
            tokio::select! {
                biased;
                result = &mut progress_wait => {
                    panic!("a DCL-ineligible timer must not satisfy the lifecycle wait: {result}")
                }
                _ = std::future::ready(()) => {}
            }
            source_ready_tx
                .send(())
                .expect("deferred source receiver should remain alive");
            progress_wait.await
        };
        assert!(
            progress_arrived,
            "deferred source terminal must wake the PageVm resource queue"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__dclBlockedTimerRan)")
                .expect("timer marker should remain readable"),
            "false",
            "waiting for DCL progress must not execute a generic timer task"
        );
        let deferred_source = run_next_resource_completion_as_typed_page_turn(&mut page_vm)
            .expect("deferred source terminal should apply");
        assert_eq!(
            deferred_source.action.source,
            RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
        );

        let advance = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut page_vm;
            vm.as_mut()
                .expect("page vm must retain a ScriptVm")
                .advance_post_parse_lifecycle(
                    &loader,
                    page_task_queue,
                    report,
                    lifecycle_driver,
                    None,
                )
                .await
                .expect("resource completion should resume the DCL driver")
        };
        let PostParseLifecycleAdvance::PageOwnedTask(mut task) = advance else {
            panic!("ready parser-deferred work should become a page-owned task");
        };
        assert!(
            task.take_work_for_execution()
                .main_parser_deferred_scripts_owner()
                .is_some(),
            "the resumed owner turn must execute the exact parser-deferred queue"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_load_replacement_retires_old_pending_queue() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-replacement.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferReplacementEvents = []")
            .expect("replacement event state should initialize");
        let first = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "replacement-defer",
            Url::parse("https://example.com/replacement-defer.js")
                .expect("replacement script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferReplacementEvents.push('execute');".to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferReplacementEvents.push('load'); document.open(); document.write(\"<!doctype html><html><body><main id='replacement'>replacement</main></body></html>\"); document.close();",
            ),
        );
        let stale = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            2,
            "stale-defer",
            Url::parse("https://example.com/stale-defer.js").expect("stale script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferReplacementEvents.push('stale-execute');".to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferReplacementEvents.push('stale-load')",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement test requires a document owner");
        for script in [first, stale] {
            assert!(
                page_vm
                    .vm_mut()
                    .claim_main_parser_deferred_script(
                        task_owner,
                        script,
                        None,
                        None,
                        Default::default(),
                    )
                    .expect("classic defer should be accepted")
            );
        }
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("replacement defer queue should seal");

        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "replacement classic defer",
        )
        .await;
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(task_owner),
            "document.open from the load handler must rotate the document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferReplacementEvents.join('|')")
                .expect("replacement events should evaluate"),
            "execute|load",
            "old parser-deferred work must stop after its completion event replaces the document"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.getElementById('replacement') !== null)")
                .expect("replacement document should evaluate"),
            "true"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "replacement must disarm the retired parser-deferred owner source"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_execution_can_replace_the_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/defer-execution-replacement.html")
                .expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferExecutionReplacementEvents = []")
            .expect("replacement event state should initialize");
        let script = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "replacement-defer",
            Url::parse("https://example.com/replacement-defer.js")
                .expect("replacement script URL"),
            ScriptSource::Loaded(
                "globalThis.__mainParserDeferExecutionReplacementEvents.push('execute'); document.open(); document.write(\"<!doctype html><html><body><main id='replacement'>replacement</main></body></html>\"); document.close(); globalThis.__mainParserDeferExecutionReplacementEvents.push('after-open');"
                    .to_owned(),
            ),
            (
                "onload",
                "globalThis.__mainParserDeferExecutionReplacementEvents.push('load')",
            ),
        );
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("replacement test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    script,
                    None,
                    None,
                    Default::default(),
                )
                .expect("classic defer should be accepted")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("replacement defer queue should seal");

        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "replacement classic defer execution",
        )
        .await;
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(task_owner),
            "document.open from deferred execution must rotate the document owner"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferExecutionReplacementEvents.join('|')")
                .expect("replacement events should evaluate"),
            "execute|after-open",
            "replacement must prevent the retired script element load event from dispatching"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(document.getElementById('replacement') !== null)")
                .expect("replacement document should evaluate"),
            "true"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_classic_defer_source_failure_dispatches_typed_error_without_refetch() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let (mut page_vm, mut page_resource_queue, mut owner_wake_rx) =
            page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/defer-failure.html").expect("document URL"),
        );
        page_vm
            .vm_mut()
            .eval("globalThis.__mainParserDeferFailureEvents = []")
            .expect("failure event state should initialize");
        let failed = append_parser_owned_external_classic_defer_for_page_vm_test(
            &mut page_vm,
            1,
            "failed-defer",
            Url::parse("https://defer-failure.test/missing.js").expect("failed script URL"),
            ScriptSource::External,
            (
                "onerror",
                "globalThis.__mainParserDeferFailureEvents.push('error:' + (document.currentScript === null))",
            ),
        );
        let failed_node_id = failed.node_id;
        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("failure test requires a document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    failed,
                    Some(crate::planning::SharedScriptSourceLoad::ready_err(
                        "prepared source failure",
                    )),
                    None,
                    Default::default(),
                )
                .expect("failed classic defer should be accepted before source start")
        );
        page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("failed defer queue should seal without waiting");
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "source failure must retain lifecycle ownership until its owner turn dispatches error"
        );
        loop {
            let wake = owner_wake_rx
                .recv()
                .await
                .expect("ready source failure must keep the Page owner wake route open");
            if wake.source_for_test()
                == crate::page_task_queue::RendererOwnerWakeSource::NetworkingTask
            {
                break;
            }
        }
        let source_failure = page_vm
            .apply_one_page_resource_terminal_owner_admission_for_test(&mut page_resource_queue)
            .expect("source terminal should arbitrate")
            .expect("source terminal should apply");
        assert_eq!(
            source_failure.action.source,
            RendererOwnerResourceActivitySource::MainParserDeferredClassicSource
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferFailureEvents.join('|')")
                .expect("pre-turn failure events should evaluate"),
            "",
            "resource completion must only update PendingScript state"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "failed classic defer")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("__mainParserDeferFailureEvents.join('|')")
                .expect("failure events should evaluate"),
            "error:true"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "source-failure completion must release its exact lifecycle token"
        );
        let run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.node_id() == failed_node_id)
            .expect("source failure owner turn should record one failed run");
        assert!(matches!(run.outcome(), ScriptRunOutcome::Failed(_)));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn main_parser_after_parsing_queue_orders_module_before_ready_classic_defer() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/ordered-module.mjs").expect("module URL");
        let module =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9101, module_url.clone());
        let (classic_node, classic_handle) = {
            let runtime = &mut page_vm.vm_mut().document_runtime;
            let body = runtime
                .snapshot_document()
                .document_body_handle()
                .expect("test document body");
            let script_node = runtime.dom_host_mut().create_element("script");
            assert!(runtime.dom_host_mut().append_child(body, script_node));
            let handle = runtime.bind_document_write_owned_script_handle_for_node(script_node);
            (script_node, handle)
        };
        let mut classic = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9102,
            "globalThis.__orderedClassicDefer = 1;",
        );
        classic.node_id = classic_node;
        classic.host_script_handle = Some(classic_handle);
        classic.mode = ScriptMode::Defer;

        let task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("mixed parser-deferred test requires a current document owner");
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    classic.clone(),
                    None,
                    None,
                    Default::default(),
                )
                .expect("classic defer PendingScript should be accepted")
        );
        assert!(
            page_vm
                .vm_mut()
                .claim_main_parser_deferred_script(
                    task_owner,
                    module.clone(),
                    None,
                    None,
                    Default::default(),
                )
                .expect("module defer PendingScript should start its graph after acceptance")
        );
        let _marker = page_vm
            .seal_main_parser_deferred_scripts(task_owner)
            .expect("mixed parser-deferred batch should install");
        page_vm
            .page_task_queue
            .extend_post_parse_work([PostParsePageOwnedWork::lifecycle_work(
                PostParseLifecycleWork::test_domcontentloaded(),
            )]);
        assert!(
            poll_post_parse_document_processing_action_for_test(&mut page_vm).is_none(),
            "pending earlier module must hold DCL without exposing later classic work"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic side effect while module is pending"),
            "undefined",
            "later ready classic defer must stay behind the earlier module PendingScript"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "mixed after-parsing order must not materialize broad ready work"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &module_url,
            "globalThis.__orderedModuleDefer = 1; export const value = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("module graph completion should run")
                .is_some()
        );
        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "earlier module release")
            .await;
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedModuleDefer)")
                .expect("read earlier module side effect"),
            "1"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic before its owner turn"),
            "undefined",
            "one parser owner turn must release only the current document-order slot"
        );

        run_ready_parser_deferred_body_for_test(&mut page_vm, &loader, "later classic release")
            .await;
        let classic_run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.node_id() == classic.node_id)
            .unwrap_or_else(|| {
                panic!(
                    "later classic owner turn should report a run: {:?}",
                    page_vm.report.runs
                )
            });
        assert!(
            matches!(classic_run.outcome(), ScriptRunOutcome::Executed),
            "later classic owner turn should execute, got {classic_run:?}"
        );
        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__orderedClassicDefer)")
                .expect("read later classic side effect"),
            "1"
        );
        assert!(
            page_vm
                .vm()
                .document_runtime
                .main_parser_deferred_scripts_owner()
                .is_none(),
            "consuming the final ordered slot must disarm the document owner source"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "mixed classic/module order must release every lifecycle token"
        );
        assert!(matches!(
            poll_post_parse_document_processing_action_for_test(&mut page_vm),
            Some(crate::document_runtime::DocumentProcessingAction::PostParsePageOwnedWork(work))
                if work.is_domcontentloaded_task()
        ));
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_graph_failure_leaves_shared_sibling_fetch_for_joined_script_waiter() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let first_root_url =
            Url::parse("https://example.com/first-root.mjs").expect("first root URL");
        let second_root_url =
            Url::parse("https://example.com/second-root.mjs").expect("second root URL");
        let bad_url = Url::parse("https://example.com/bad.mjs").expect("bad module URL");
        let shared_url = Url::parse("https://example.com/shared.mjs").expect("shared module URL");
        let shared_key = ModuleMapKey::java_script(shared_url.clone());
        let first_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9201, first_root_url.clone());
        let second_script = prepared_external_module_for_page_vm_test_with_node(
            &page_vm,
            9202,
            second_root_url.clone(),
        );

        let first_parser_module_work =
            install_parser_module_defer_work(&mut page_vm, first_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                first_parser_module_work,
            )
            .await
            .expect("first module defer page task should watch loading tree");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            0,
            &first_root_url,
            r#"
import "./bad.mjs";
import "./shared.mjs";
export const first = 1;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("first root completion should run")
                .is_some(),
            "first root completion should be consumed"
        );

        let second_parser_module_work =
            install_parser_module_defer_work(&mut page_vm, second_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                second_parser_module_work,
            )
            .await
            .expect("second module defer page task should watch loading tree");
        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            3,
            &second_root_url,
            r#"
import "./shared.mjs";
export const second = 2;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("second root completion should run")
                .is_some(),
            "second root completion should be consumed"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "second graph should be waiting on the first graph's shared sibling fetch"
        );
        let shared_fetch_target = page_vm
            .vm()
            .current_main_parser_module_graph_fetch_target(2)
            .expect("shared fetch must retain its producer-captured target before owner failure");

        enqueue_parser_owned_module_script_fetch_error_for_test(
            &mut page_vm,
            1,
            &bad_url,
            "bad dependency failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("bad dependency completion should run")
                .is_some(),
            "bad dependency failure should be consumed"
        );

        let shared_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&shared_key)
            .expect("cancelled shared sibling fetch should remain in the module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(shared_entry),
            ModuleMapEntryState::Fetching,
            "owner failure must not globally fail a shared module map entry while its network fetch is still pending"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should keep waiting for the shared fetch network completion"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "ordered graph failure must stay in the parser PendingScript instead of the broad ready lane"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "bad dependency failure",
        )
        .await;
        let first_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &first_root_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("first parser module script should fail after bad dependency");
        assert!(
            first_failure.contains("bad dependency failed"),
            "first script should report the real dependency failure: {first_failure}"
        );

        enqueue_parser_owned_module_script_fetch_completion_for_target_for_test(
            &mut page_vm,
            shared_fetch_target,
            &shared_url,
            "export const shared = 1;",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("shared dependency completion should run")
                .is_some(),
            "shared dependency completion should be consumed"
        );
        assert!(
            page_vm.vm_mut().has_ready_native_module_owner_actions(),
            "shared module-map terminal must publish its joined-client owner event"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "shared parser module-map completion",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should resume through the native module owner turn"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "joined parser module graph completion",
        )
        .await;
        run_parser_module_completion_turns_for_test(
            &mut page_vm,
            &loader,
            0,
            "joined parser module graph completion",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph should no longer be pending after its ready script executes"
        );
        let second_run = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &second_root_url)
            .expect("second parser module script should run after joined fetch wakeup");
        assert!(
            matches!(second_run.outcome(), ScriptRunOutcome::Executed),
            "second parser module script should execute after joined fetch wakeup: {:?}",
            second_run.outcome()
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn modulepreload_fetch_failure_wakes_joined_parser_script_waiter() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let root_url = Url::parse("https://example.com/root.mjs").expect("root URL");
        let shared_url = Url::parse("https://cdn.example.com/shared.mjs").expect("shared URL");
        let shared_key = ModuleMapKey::java_script(shared_url.clone());
        let modulepreload = NativeModuleSingleFetchRequest::new(
            shared_url.clone(),
            shared_url.clone(),
            document_url,
            shared_key.clone(),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        let root_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9301, root_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, root_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("module defer page task should watch loading tree");

        enqueue_parser_owned_module_script_fetch_completion_for_test(
            &mut page_vm,
            1,
            &root_url,
            r#"
import "https://cdn.example.com/shared.mjs";
export const root = 1;
"#,
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("root completion should run")
                .is_some(),
            "root completion should be consumed"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "parser graph should wait on the in-flight modulepreload entry"
        );

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &shared_url,
            "modulepreload fetch failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "joined modulepreload failure",
        )
        .await;
        let shared_entry = page_vm
            .vm()
            .document_runtime
            .native_module_entry_id(&shared_key)
            .expect("failed modulepreload should remain in module map");
        assert_eq!(
            page_vm
                .vm()
                .document_runtime
                .native_module_entry_state(shared_entry),
            ModuleMapEntryState::Failed,
            "failed modulepreload must become terminal"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "joined parser graph should be woken by the modulepreload failure"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph failure should wait for the document-owned ready dispatch"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "joined modulepreload graph failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "joined parser graph failure should leave no pending parser-owned module script"
        );
        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &root_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail after joined modulepreload wakeup");
        assert!(
            root_failure.contains("modulepreload fetch failed"),
            "joined script should observe the terminal modulepreload failure: {root_failure}"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_script_observes_prior_modulepreload_failure() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let modulepreload = NativeModuleSingleFetchRequest::new(
            module_url.clone(),
            module_url.clone(),
            document_url,
            ModuleMapKey::java_script(module_url.clone()),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &module_url,
            "modulepreload integrity failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );

        let module_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9401, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(&loader, parser_module_work)
            .await
            .expect("module defer page task should watch failed tree");
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "the parser-deferred owner turn should consume the already-terminal failure"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "sticky failure must not materialize broad ready work"
        );

        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &module_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail from sticky modulepreload failure");
        assert!(
            root_failure.contains("modulepreload integrity failed"),
            "module script should report sticky modulepreload failure: {root_failure}"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn parser_module_script_joins_inflight_same_url_modulepreload_failure() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let document_url = Url::parse("https://example.com/page.html").expect("document URL");
        let mut page_vm =
            test_page_vm_with_loader_and_document_url(&loader, Vec::new(), document_url.clone());

        let module_url = Url::parse("https://example.com/module.mjs").expect("module URL");
        let modulepreload = NativeModuleSingleFetchRequest::new(
            module_url.clone(),
            module_url.clone(),
            document_url,
            ModuleMapKey::java_script(module_url.clone()),
            ModuleFetchMetadata::default(),
        );
        let start = page_vm
            .vm_mut()
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload)
            .expect("modulepreload registration should succeed");
        let crate::module_runtime::NativeModulepreloadFetchStart::Started(modulepreload) = start
        else {
            panic!("new modulepreload should start a single-module fetch");
        };
        let modulepreload_load_id = page_vm
            .vm_mut()
            .document_runtime
            .suspend_native_modulepreload_fetch(*modulepreload);

        let module_script =
            prepared_external_module_for_page_vm_test_with_node(&page_vm, 9501, module_url.clone());
        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("module defer page task should watch joined modulepreload");
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "parser root graph should wait on the in-flight same-URL modulepreload entry"
        );

        enqueue_main_modulepreload_fetch_error_for_test(
            &mut page_vm,
            modulepreload_load_id,
            &module_url,
            "modulepreload integrity failed",
        );
        assert!(
            run_next_main_module_fetch_terminal_for_test(&mut page_vm)
                .expect("modulepreload completion should run")
                .is_some(),
            "modulepreload completion should be consumed"
        );
        run_next_native_module_owner_event_for_test(
            &mut page_vm,
            &loader,
            "same-URL joined modulepreload failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "same-URL joined parser graph should be woken by modulepreload failure"
        );
        assert!(
            page_vm.has_pending_parser_owned_module_script(),
            "same-URL joined parser graph failure should wait for the document-owned ready dispatch"
        );
        run_ready_parser_deferred_body_for_test(
            &mut page_vm,
            &loader,
            "same-URL joined modulepreload failure",
        )
        .await;
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "same-URL joined parser graph failure should leave no pending parser-owned module script"
        );

        let root_failure = page_vm
            .report
            .runs
            .iter()
            .find(|run| run.url() == &module_url)
            .and_then(|run| match run.outcome() {
                ScriptRunOutcome::Failed(message) => Some(message),
                _ => None,
            })
            .expect("parser module script should fail from same-URL modulepreload failure");
        assert!(
            root_failure.contains("modulepreload integrity failed"),
            "module script should report joined modulepreload failure: {root_failure}"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn document_replacement_drops_parser_pending_module_tree_owner() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let module_url = Url::parse("https://example.com/replaced-module.mjs")
            .expect("module URL");
        let module_script = prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let parser_module_work = install_parser_module_defer_work(&mut page_vm, module_script);
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                parser_module_work,
            )
            .await
            .expect("module defer page task should watch loading pending tree");
        assert!(
            page_vm.has_pending_parser_owned_module_fetch(),
            "module tree should be waiting for its root fetch before replacement"
        );
        let stale_target = page_vm
            .vm()
            .current_main_parser_module_graph_fetch_target(0)
            .expect("loading parser root must expose its exact terminal target");

        let replacement_script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9100,
            "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(replacement_script),
            )
            .await
            .expect("replacement script task should run");

        assert!(
            !page_vm.has_pending_parser_owned_module_fetch(),
            "document replacement must drop parser-owned module fetch continuations"
        );
        assert!(
            !page_vm.has_pending_parser_owned_module_script(),
            "document replacement must drop parser pending module scripts"
        );

        let replacement_document_url = page_vm.vm().document_runtime.document_url().clone();
        page_vm
            .vm()
            .resource_completion_sender_for_test()
            .send_main_parser_module_graph_fetch(MainParserModuleGraphFetchCompletion::new(
                stale_target,
                Ok(ModuleGraphFetchedSource::new(
                    module_url.clone(),
                    false,
                    ModuleSource::text(
                        "globalThis.__oldModuleAfterReplacement = true; export const value = 1;"
                            .to_owned(),
                    ),
                )),
                None,
                MainModuleFetchNetworkAttribution::new(
                    replacement_document_url,
                    module_url.clone(),
                ),
            ))
            .expect("stale exact parser terminal should still enter the stable Page source");
        let outcome = run_next_resource_completion_as_typed_page_turn(&mut page_vm)
            .expect("old module graph completion should consume one typed turn");
        assert!(
            matches!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::DiscardedStaleOwner { .. }
            ),
            "old exact parser target must be rejected before module-map application: {outcome:?}"
        );

        assert_eq!(
            page_vm
                .vm_mut()
                .eval("String(globalThis.__oldModuleAfterReplacement)")
                .expect("read old module side effect"),
            "undefined",
            "old parser module tree must not execute after document replacement"
        );
        assert!(
            page_vm
                .capture_page_state()
                .expect("page state capture")
                .report
                .lifecycle_errors()
                .is_empty(),
            "typed stale-owner rejection is an expected terminal outcome, not a lifecycle error"
        );
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn stale_main_parser_module_terminal_drops_after_pending_script_retirement() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/page.html").expect("document URL"),
        );
        let old_task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("original main document task owner should exist");
        let old_owner = MainParserDocumentOwner::new(old_task_owner);
        let module_url =
            Url::parse("https://example.com/stale-ready-module.mjs").expect("module URL");
        let module_script =
            prepared_external_module_for_page_vm_test(&page_vm, module_url.clone());

        let replacement_script = prepared_loaded_classic_for_page_vm_test(
            &page_vm,
            9200,
            "document.open(); document.write('<!doctype html><p>replacement</p>'); document.close();",
        );
        page_vm
            .execute_post_parse_page_owned_task_on_named_owner_lane(
                &loader,
                classic_defer_work(replacement_script),
            )
            .await
            .expect("replacement script task should run");
        let stale_pending_script_id =
            crate::document_script_scheduler::ParserPendingScriptId::new(
                old_owner,
                &module_script,
            );
        let stale_continuation = ModuleScriptContinuation::new_parser(
            module_script,
            stale_pending_script_id,
        )
        .with_completed_graph(ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(1),
            entries: vec![ModuleEntryId::for_test(1)],
        });
        let main_task_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("main document task owner should exist");
        let main_owner = MainParserDocumentOwner::new(main_task_owner);
        assert_ne!(main_task_owner.document_id, old_task_owner.document_id);
        assert_eq!(
            main_task_owner.scheduler_lane_id,
            old_task_owner.scheduler_lane_id,
            "main document replacement should retain the browsing-context scheduler lane"
        );
        assert_eq!(
            main_task_owner.local_window_id,
            old_task_owner.local_window_id,
            "document.open() should retain the main LocalWindow"
        );
        assert_ne!(
            main_owner, old_owner,
            "document replacement should install a new document-scoped parser owner"
        );
        let stale_work = stale_continuation.into_main_document_graph_ready_work();
        let accepted = page_vm
            .vm_mut()
            .document_runtime
            .parser_module_document_scripts_mut()
            .notify_module_script_graph_ready_work(stale_work);
        assert!(
            !accepted,
            "terminal must fail closed after its original PendingScript is retired"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_load_blocking_document_script_work(old_owner),
            "retired owner must not be rematerialized by a stale terminal"
        );
        assert!(
            !page_vm
                .vm()
                .document_runtime
                .parser_module_document_scripts()
                .has_load_blocking_document_script_work(main_owner),
            "stale terminal must not enter the replacement document scheduler"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "stale terminal must not synthesize a ready action without its PendingScript"
        );

        assert!(
            !run_one_parser_owned_main_document_runtime_turn_for_test(&mut page_vm, &loader)
                .await
                .expect("stale parser module ready lane should remain idle"),
            "retired PendingScript terminal must not reach owner-specific execution"
        );
        assert!(
            !page_vm.has_ready_parser_owned_document_script_action(),
            "stale ready action should be removed from the ready lane"
        );
        assert!(
            page_vm.report.runs.iter().all(|run| run.url() != &module_url),
            "stale main parser module ready action must not produce a script run: {:?}",
            page_vm.report.runs
        );
    })
    .await;
}

#[test]
fn module_graph_network_result_records_staged_response_started_with_cache_state() {
    let mut page_vm = test_page_vm_with_document_url(
        Url::parse("https://example.com/page.html").expect("document URL"),
    );
    let request_url = Url::parse("https://example.com/module.js").expect("request URL");
    let response = crate::types::NavigationResponse::from_head_and_text_body(
        moli_fetch::ResponseHead {
            final_url: request_url.clone(),
            status: 200,
            headers: vec![("content-type".to_owned(), "text/javascript".to_owned())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: true,
            negotiated_http_version: None,
        },
        "export default 1;".to_owned(),
    );
    let completion = ModuleGraphFetchCompletion {
        load_id: 7,
        requester: ModuleGraphFetchRequester::DynamicImport,
        ordering: ModuleGraphFetchOrdering::Runtime,
        request_url: request_url.clone(),
        result: Err("source is not used by this test".to_owned()),
        network_result: None,
    };

    page_vm
        .vm_mut()
        .record_module_graph_subresource_network_result(&completion, &Ok(response));

    let items: Vec<_> = page_vm
        .vm_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(
        items.len(),
        3,
        "module fetch should record staged network output"
    );
    let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = &items[0] else {
        panic!("first item should be requestStarted: {items:?}");
    };
    assert_eq!(request.url(), &request_url);
    assert_eq!(request.resource_type(), SubresourceResourceType::Script);

    let ScriptNetworkOutputItem::SubresourceResponseStarted(response) = &items[1] else {
        panic!("second item should be responseStarted: {items:?}");
    };
    assert_eq!(response.final_url(), &request_url);
    assert!(
        response.from_cache(),
        "module responseStarted must preserve fetch cache state"
    );

    let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = &items[2] else {
        panic!("third item should be bodyFinished: {items:?}");
    };
    assert!(matches!(
        body.result(),
        SubresourceBodyFinishedResult::Ready(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn direct_page_vm_wait_commands_fail_closed() {
    run_page_vm_async_test(async move {
        let loader_owner =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let loader = loader_owner.handle();

        let commands = [
            (
                RendererPageCommand::WaitForSelector {
                    selector: "#never".to_owned(),
                    timeout_ms: 1_000,
                    loader: loader.clone(),
                },
                "wait-for-selector must be routed through the renderer owner continuation",
            ),
            (
                RendererPageCommand::WaitForScriptTruthy {
                    expression: "false".to_owned(),
                    timeout_ms: 1_000,
                    loader: loader.clone(),
                },
                "wait-for-script-truthy must be routed through the renderer owner continuation",
            ),
            (
                RendererPageCommand::WaitForSubresourceResponse {
                    criteria: SubresourceResponseWaitCriteria::default(),
                    timeout_ms: 1_000,
                    loader,
                },
                "wait-for-subresource-response must be routed through the renderer owner continuation",
            ),
        ];

        for (command, expected_error) in commands {
            let mut page_vm = test_page_vm();
            let error = match page_vm
                .dispatch_renderer_page_command_async(command)
                .await
            {
                Ok(_) => panic!("direct wait command should fail closed"),
                Err(error) => error,
            };
            assert!(
                error.to_string().contains(expected_error),
                "unexpected error: {error}"
            );
        }
    })
    .await;
}

fn split_network_output_items(
    output: ScriptNetworkOutput,
) -> (
    Vec<SubresourceNetworkRecord>,
    Vec<WebSocketNetworkEvent>,
    Vec<WebSocketLifecycleEvent>,
) {
    let mut records = Vec::new();
    let mut frame_events = Vec::new();
    let mut lifecycle_events = Vec::new();
    for item in output.into_items() {
        match item {
            ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => records.push(*record),
            ScriptNetworkOutputItem::WebSocketNetworkEvent(event) => frame_events.push(event),
            ScriptNetworkOutputItem::WebSocketLifecycleEvent(event) => {
                lifecycle_events.push(event);
            }
            ScriptNetworkOutputItem::SubresourceRequestStarted(_)
            | ScriptNetworkOutputItem::SubresourceResponseStarted(_)
            | ScriptNetworkOutputItem::SubresourceDataReceived(_)
            | ScriptNetworkOutputItem::SubresourceEventSourceMessageReceived(_)
            | ScriptNetworkOutputItem::SubresourceBodyFinished(_) => {}
        }
    }
    (records, frame_events, lifecycle_events)
}

fn detached_test_run() -> ScriptRun {
    ScriptRun::skipped(
        NodeId::new(99),
        ScriptKind::Classic,
        ScriptMode::Async,
        ScriptSourceKind::External,
        Url::parse("https://example.com/detached.js").unwrap(),
        ScriptSkipReason::NotInMainDocument,
    )
}

async fn run_page_vm_async_test<F, R>(future: F) -> R
where
    F: std::future::Future<Output = R> + 'static,
    R: 'static,
{
    tokio::task::LocalSet::new()
        .run_until(async move {
            tokio::task::spawn_local(future)
                .await
                .expect("page_vm async test task should finish")
        })
        .await
}

async fn run_on_page_vm_local_executor<F, R>(
    local_executor: crate::local_executor::JsLocalExecutor,
    future: F,
) -> R
where
    F: std::future::Future<Output = R> + 'static,
    R: 'static,
{
    local_executor.run(future).await
}

fn run_page_vm_local_runtime_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build_local(tokio::runtime::LocalOptions::default())
                .expect("local-runtime page_vm test runtime should build")
                .block_on(build());
        })
        .expect("local-runtime page_vm test thread should spawn")
        .join()
        .expect("local-runtime page_vm test thread should finish");
}

fn run_page_vm_local_runtime_async_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    run_page_vm_local_runtime_test(thread_name, || async move {
        run_page_vm_async_test(build()).await;
    });
}

fn run_page_vm_large_stack_async_test<F, Fut>(thread_name: &'static str, build: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("large-stack page_vm test runtime should build")
                .block_on(run_page_vm_async_test(build()));
        })
        .expect("large-stack page_vm test thread should spawn")
        .join()
        .expect("large-stack page_vm test thread should finish");
}

async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<String> {
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
            return Ok(String::from_utf8_lossy(&request).into_owned());
        }
    }
}

async fn read_http_request_with_body(
    stream: &mut tokio::net::TcpStream,
) -> std::io::Result<String> {
    let head = read_http_request_head(stream).await?;
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).await?;
    }
    Ok(format!("{head}{}", String::from_utf8_lossy(&body)))
}

async fn spawn_single_response_http_server(
    status_line: &'static str,
    body: String,
    delay: Duration,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept worker script request");
        read_http_request_head(&mut stream)
            .await
            .expect("read worker script request");
        if !delay.is_zero() {
            sleep(delay).await;
        }
        let response = format!(
            "{status_line}\r\nContent-Type: text/javascript; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write worker script response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let server = tokio::spawn(async move {
        let mut response_queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            response_queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        while response_queues.values().any(|queue| !queue.is_empty()) {
            let (mut stream, _) = listener.accept().await.expect("accept worker request");
            let request = read_http_request_head(&mut stream)
                .await
                .expect("read worker request");
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .expect("request path");
            let (status_line, body, delay) = response_queues
                .get_mut(path)
                .and_then(|queue| queue.pop_front())
                .unwrap_or_else(|| panic!("unexpected worker test request path: {path}"));
            if !delay.is_zero() {
                sleep(delay).await;
            }
            let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
            let content_type =
                if path_without_query.ends_with(".js") || path_without_query.ends_with(".mjs") {
                    "text/javascript"
                } else if path_without_query.ends_with(".css") {
                    "text/css"
                } else {
                    "text/html"
                };
            let response = format!(
                "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write worker test response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_concurrent_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local concurrent multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let request_count = response_specs.len();
    let response_queues = {
        let mut queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        std::sync::Arc::new(tokio::sync::Mutex::new(queues))
    };
    let server = tokio::spawn(async move {
        let mut response_tasks = Vec::with_capacity(request_count);
        for _ in 0..request_count {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept concurrent worker request");
            let response_queues = response_queues.clone();
            response_tasks.push(tokio::spawn(async move {
                let request = read_http_request_head(&mut stream)
                    .await
                    .expect("read concurrent worker request");
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("request path");
                let (status_line, body, delay) = {
                    let mut response_queues = response_queues.lock().await;
                    response_queues
                        .get_mut(path)
                        .and_then(|queue| queue.pop_front())
                        .unwrap_or_else(|| {
                            panic!("unexpected concurrent worker test request path: {path}")
                        })
                };
                if !delay.is_zero() {
                    sleep(delay).await;
                }
                let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
                let content_type =
                    if path_without_query.ends_with(".js") || path_without_query.ends_with(".mjs")
                    {
                        "text/javascript"
                    } else if path_without_query.ends_with(".css") {
                        "text/css"
                    } else {
                        "text/html"
                    };
                let response = format!(
                    "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write concurrent worker test response");
            }));
        }
        for task in response_tasks {
            task.await.expect("concurrent worker response task");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_shutdown_path_response_http_server(
    response_specs: Vec<(&'static str, &'static str, String, Duration)>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<Vec<String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local shutdown multi-response worker test server");
    let addr = listener.local_addr().expect("server local addr");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut response_queues = std::collections::HashMap::<
            String,
            std::collections::VecDeque<(&'static str, String, Duration)>,
        >::new();
        for (path, status, body, delay) in response_specs {
            response_queues
                .entry(path.to_owned())
                .or_default()
                .push_back((status, body, delay));
        }
        let mut requested_paths = Vec::new();
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.expect("accept worker request");
                    let request = read_http_request_head(&mut stream)
                        .await
                        .expect("read worker request");
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .expect("request path");
                    requested_paths.push(path.to_owned());
                    let (status_line, body, delay) = response_queues
                        .get_mut(path)
                        .and_then(|queue| queue.pop_front())
                        .unwrap_or_else(|| panic!("unexpected worker test request path: {path}"));
                    if !delay.is_zero() {
                        sleep(delay).await;
                    }
                    let path_without_query = path.split_once('?').map_or(path, |(path, _)| path);
                    let content_type = if path_without_query.ends_with(".js")
                        || path_without_query.ends_with(".mjs")
                    {
                        "text/javascript"
                    } else {
                        "text/html"
                    };
                    let response = format!(
                        "{status_line}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write worker test response");
                }
            }
        }
        requested_paths
    });
    (format!("http://{addr}"), shutdown_tx, server)
}

async fn spawn_connection_drop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind connection-drop http server");
    let addr = listener.local_addr().expect("connection-drop server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept connection-drop request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read connection-drop request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("connection-drop request path");
        assert_eq!(request_path, path);
        drop(stream);
    });
    (format!("http://{addr}"), server)
}

async fn spawn_redirect_loop_http_server(path: &'static str) -> (String, JoinHandle<()>) {
    const REDIRECT_LOOP_REQUESTS: usize = 11;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect-loop http server");
    let addr = listener.local_addr().expect("redirect-loop server addr");
    let server = tokio::spawn(async move {
        for _ in 0..REDIRECT_LOOP_REQUESTS {
            let (mut stream, _) = listener
                .accept()
                .await
                .expect("accept redirect-loop request");
            read_http_request_head(&mut stream)
                .await
                .expect("read redirect-loop request");
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write redirect-loop response");
        }
    });
    (format!("http://{addr}"), server)
}

async fn spawn_single_redirect_http_server(
    path: &'static str,
    location: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind single-redirect http server");
    let addr = listener.local_addr().expect("single-redirect server addr");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept single-redirect request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read single-redirect request");
        let request_path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("single-redirect request path");
        assert_eq!(request_path, path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write single-redirect response");
    });
    (format!("http://{addr}"), server)
}

async fn spawn_cross_origin_redirect_without_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target without CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("redirect target without CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept redirect target request");
        read_http_request_head(&mut stream)
            .await
            .expect("read redirect target request");
        let body = "cors-denied-target";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect target without CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect source server");
    let source_addr = source_listener.local_addr().expect("redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read redirect source request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("redirect source request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect source response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_cross_origin_redirect_with_cors_http_servers(
    source_path: &'static str,
    target_path: &'static str,
    target_body: &'static str,
) -> (String, String, JoinHandle<()>, JoinHandle<()>) {
    let target_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind redirect target with CORS server");
    let target_addr = target_listener
        .local_addr()
        .expect("redirect target with CORS addr");
    let target_base_url = format!("http://{target_addr}");
    let target_server = tokio::spawn(async move {
        let (mut stream, _) = target_listener
            .accept()
            .await
            .expect("accept redirect target request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read redirect target request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("redirect target request path");
        assert_eq!(path, target_path);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            target_body.len(),
            target_body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write redirect target with CORS response");
    });

    let source_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind CORS redirect source server");
    let source_addr = source_listener
        .local_addr()
        .expect("CORS redirect source addr");
    let source_base_url = format!("http://{source_addr}");
    let target_location = format!("{target_base_url}{target_path}");
    let source_server = tokio::spawn(async move {
        let (mut stream, _) = source_listener
            .accept()
            .await
            .expect("accept CORS redirect source request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read CORS redirect source request");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("CORS redirect source request path");
        assert_eq!(path, source_path);
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write CORS redirect source response");
    });

    (
        source_base_url,
        target_base_url,
        source_server,
        target_server,
    )
}

async fn spawn_header_capture_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local header-capture http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept captured http request");
        let request = read_http_request_head(&mut stream)
            .await
            .expect("read captured http request");
        let _ = request_tx.send(request);
        let body = "ok";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write captured http response");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_request_capture_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local request-capture http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_tx, request_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept captured http request");
        let request = read_http_request_with_body(&mut stream)
            .await
            .expect("read captured http request");
        let _ = request_tx.send(request);
        let response = "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write captured http response");
    });
    (format!("http://{addr}"), request_rx, server)
}

async fn spawn_disconnect_observing_http_server()
-> (String, tokio::sync::oneshot::Receiver<bool>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local disconnect-observing http server");
    let addr = listener.local_addr().expect("server local addr");
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept disconnect-observing http request");
        read_http_request_head(&mut stream)
            .await
            .expect("read disconnect-observing http request");
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nhello",
                )
                .await
                .expect("write disconnect-observing http response");
        sleep(Duration::from_millis(150)).await;
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });
    (format!("http://{addr}"), disconnect_rx, server)
}

async fn spawn_request_seen_disconnect_observing_http_server() -> (
    String,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Receiver<bool>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local request-seen disconnect-observing http server");
    let addr = listener.local_addr().expect("server local addr");
    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept request-seen disconnect-observing http request");
        read_http_request_head(&mut stream)
            .await
            .expect("read request-seen disconnect-observing http request");
        let _ = request_seen_tx.send(());
        let mut tail = [0u8; 1];
        let disconnected = matches!(
            tokio::time::timeout(Duration::from_secs(3), stream.read(&mut tail)).await,
            Ok(Ok(0))
        );
        let _ = disconnect_tx.send(disconnected);
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        disconnect_rx,
        server,
    )
}

async fn drive_page_work_until_done_with_explicit_producer_admission(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
    mut admit_external_producer_work: impl FnMut(&PageVm),
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        admit_external_producer_work(page_vm);
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        if page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {
            continue;
        }
        if page_vm
            .run_next_child_frame_task_source_for_semantic_test()
            .await
            .is_some()
        {
            continue;
        }
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        page_vm
            .advance_timers_until_deadline_for_test(loader.request_client())
            .await?;
        if page_vm.vm_mut().eval(done_expression)? == "true" {
            return Ok(());
        }
        let arrived = tokio::time::timeout(
            Duration::from_secs(1),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await
        .unwrap_or(false);
        if !arrived {
            while page_vm
                .run_exact_page_websocket_selected_task_for_test()
                .await?
                .is_some()
            {}
            let loader = page_vm.main_document_resource_loader();
            page_vm
                .advance_timers_until_deadline_for_test(loader.request_client())
                .await?;
            if page_vm.vm_mut().eval(done_expression)? == "true" {
                return Ok(());
            }
        }
    }
    admit_external_producer_work(page_vm);
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    page_vm
        .advance_timers_until_deadline_for_test(loader.request_client())
        .await?;
    if page_vm.vm_mut().eval(done_expression)? != "true" {
        let events = page_vm
            .vm_mut()
            .eval("JSON.stringify(globalThis.__wsEvents ?? globalThis.__wsStreamEvents ?? null)")
            .unwrap_or_else(|error| {
                format!("<failed to read __wsEvents/__wsStreamEvents: {error}>")
            });
        let done = page_vm
            .vm_mut()
            .eval("String(globalThis.__wsDone ?? globalThis.__wsStreamDone)")
            .unwrap_or_else(|error| format!("<failed to read __wsDone/__wsStreamDone: {error}>"));
        let stream_events = page_vm
            .vm_mut()
            .eval("JSON.stringify(globalThis.__wsStreamEvents ?? null)")
            .unwrap_or_else(|error| format!("<failed to read __wsStreamEvents: {error}>"));
        let stream_done = page_vm
            .vm_mut()
            .eval("String(globalThis.__wsStreamDone ?? null)")
            .unwrap_or_else(|error| format!("<failed to read __wsStreamDone: {error}>"));
        panic!(
            "{context}; events={events}; done={done}; stream_events={stream_events}; stream_done={stream_done}"
        );
    }
    Ok(())
}

pub(super) async fn drive_websocket_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_page_work_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |_| {},
    )
    .await
}

/// Drive a direct PageVm SharedWorker workflow without borrowing WebSocket
/// execution as an implicit browser-context producer pump.
///
/// Production performs this admission in the render-owner service turn. This
/// fixture has no owner loop, so it names and performs that responsibility
/// before selecting already-resident Page tasks.
pub(super) async fn drive_shared_worker_until_done(
    page_vm: &mut PageVm,
    done_expression: &str,
    context: &str,
) -> anyhow::Result<()> {
    drive_page_work_until_done_with_explicit_producer_admission(
        page_vm,
        done_expression,
        context,
        |page_vm| {
            page_vm
                .runtime_hooks
                .browser_context_runtime
                .drain_shared_worker_service_lane();
        },
    )
    .await
}

pub(super) async fn drain_page_work_until_no_pending_subresources(
    page_vm: &mut PageVm,
    context: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        while page_vm
            .run_exact_page_websocket_selected_task_for_test()
            .await?
            .is_some()
        {}
        let loader = page_vm.main_document_resource_loader();
        while page_vm
            .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
            .await?
        {}
        if page_vm.pending_subresource_request_count() == 0 {
            return Ok(());
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            page_vm.wait_for_page_work_arrival_without_timeout(false),
        )
        .await;
    }
    while page_vm
        .run_exact_page_websocket_selected_task_for_test()
        .await?
        .is_some()
    {}
    let loader = page_vm.main_document_resource_loader();
    while page_vm
        .run_one_oldest_ready_page_task_on_owner_lane_for_test(loader.request_client())
        .await?
    {}
    anyhow::bail!(
        "{context}; pending={}",
        page_vm.pending_subresource_request_count()
    )
}
