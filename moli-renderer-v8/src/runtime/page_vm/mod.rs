use super::access::{is_on_script_execution_lane_for, run_named_owner_local_task};
use super::owner_local_store::{
    RendererDocumentIsolateAllocator, RendererDocumentIsolateReservation,
};
use super::page::{
    LoadedFollowedLocationNavigation, bootstrap_committed_followed_location_navigation,
    load_followed_location_navigation,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::{
    PageOwnedDocumentScriptBodyActivity, PageOwnedDocumentScriptBodyExecution,
};
use crate::dom::native::DomHost;
use crate::dynamic_script_owner::{DynamicScriptOwnerId, DynamicScriptPageTaskClaim};
use crate::frame_owner_model::DocumentId;
use crate::live_document_parser::DocumentParserSession;
use crate::local_executor::is_on_named_owner_execution_lane_for;
use crate::module_script_continuation::{
    ModuleScriptCompletionOwner, ModuleScriptContinuation, ModuleScriptContinuationGraphAdvance,
    ModuleScriptEvaluationContinuation, NativeModuleOwnerActions,
};
use crate::page_task_queue::{
    MainDocumentPostParseExecution, MainDocumentPostParseOwner, MainDocumentPostParseWork,
    PostParseLifecycleWork, PostParsePageOwnedWork, RendererOwnerWakeSender,
    RendererResourceCompletionSender,
};
use crate::script_vm::{
    MainDocumentLifecycleBody, MainDocumentLifecycleCallbackEffect,
    MainDocumentLifecycleCompletion, MainDocumentLifecycleFollowup,
    MainDocumentLifecycleTargetEffect, PostParseLifecycleCompletionAction,
};
use crate::script_vm::{
    ParserModuleTerminalDisposition, ParserOwnedClassicScriptCompletion,
    ParserOwnedClassicScriptCompletionApplication, ParserOwnedClassicScriptExecutionContext,
    ScriptVmBootstrapError, ScriptVmDefaultWorldBootstrap,
};
#[cfg(test)]
use crate::script_vm::{
    PostParseLifecycleAdvance, PostParsePageOwnedTask, RendererDocumentIsolateHandle,
};
use crate::script_vm::{PreparedScriptExecutionOutcome, RendererDocumentIsolateBootstrap};
use crate::types::ScriptErrorConstructorKind;
use crate::types::ScriptSkipReason;
use moli_page_types::{
    ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType, InspectorIssueSnapshot,
    InspectorSourceCodeLocationSnapshot, LayoutPolicy, QuirksModeIssueSnapshot,
    ScriptObservableOutput, ScriptObservableOutputItem, V8InspectorSessionAttach,
};
use percent_encoding::percent_decode_str;
use std::{
    collections::{BTreeMap, HashMap},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
    time::Instant,
};
use url::Url;

pub(crate) mod backend_node_registry;
mod command_turn_output;
mod css_agent_state;
mod document_lifecycle_turn;
pub(crate) mod dom_agent_state;
mod dom_mutations;
mod dom_search;
mod frontend_node_bindings;
mod javascript_navigation_lifecycle;
mod main_document_lifecycle_completion;
mod page_action_window;
mod page_broadcast_channel_delivery;
mod page_callback_task_completion;
mod page_child_classic_script_source_load_task_completion;
#[cfg(test)]
mod page_child_document_lifecycle_body_test_support;
mod page_child_document_lifecycle_task_completion;
#[cfg(test)]
mod page_child_document_script_ready_body_test_support;
mod page_child_document_script_ready_task_completion;
mod page_child_frame_task;
#[cfg(test)]
mod page_child_host_load_body_test_support;
mod page_child_host_load_task_completion;
mod page_child_module_dependency_fetch_start;
mod page_child_module_dependency_fetch_start_task_completion;
mod page_child_module_script_terminal;
mod page_child_module_script_terminal_task_completion;
mod page_child_modulepreload_event_action;
mod page_child_navigation_commit;
mod page_child_parser_module_root_start_task_completion;
mod page_child_realm_materialization_task_completion;
mod page_dedicated_worker_client_event;
#[cfg(test)]
mod page_dedicated_worker_client_event_body_test_support;
mod page_dom_manipulation;
mod page_dom_manipulation_task_completion;
#[cfg(test)]
mod page_dom_manipulation_test_support;
mod page_dynamic_import_owner_action;
mod page_element_toggle_event;
mod page_file_entry_file_callback;
mod page_file_reading;
mod page_hash_change_delivery;
mod page_history_traversal;
#[cfg(test)]
mod page_history_traversal_body_test_support;
mod page_image_load_event;
mod page_indexed_db_task;
mod page_internal_loading;
mod page_internal_loading_task_completion;
mod page_main_document_post_parse;
mod page_main_document_runtime;
mod page_main_native_module_task;
mod page_main_parser_continuation;
mod page_media_element_event;
#[cfg(test)]
mod page_media_element_event_body_test_support;
mod page_message_port_delivery;
mod page_misc_platform_api;
mod page_module_reaction;
#[cfg(test)]
mod page_module_reaction_body_test_support;
mod page_modulepreload_start;
mod page_modulepreload_start_task_completion;
mod page_navigation_and_traversal;
mod page_navigation_api_task;
#[cfg(test)]
mod page_navigation_api_task_body_test_support;
mod page_networking;
mod page_networking_task_completion;
mod page_opfs_task;
mod page_owned_document_script;
mod page_owned_document_script_completion;
mod page_owned_document_script_hooks;
mod page_parser_async_module_admission;
mod page_parser_owned_module_continuation;
mod page_popup_load_event;
mod page_rendering_update;
#[cfg(test)]
mod page_rendering_update_body_test_support;
mod page_resource_completion;
mod page_resource_completion_task_completion;
#[cfg(test)]
mod page_selected_task_test_harness;
mod page_service_worker_client_message;
#[cfg(test)]
mod page_service_worker_client_message_body_test_support;
mod page_service_worker_internal;
#[cfg(test)]
mod page_service_worker_internal_body_test_support;
mod page_shared_worker_client_event;
#[cfg(test)]
mod page_shared_worker_client_event_body_test_support;
mod page_storage_event_delivery;
mod page_stylesheet_task;
#[cfg(test)]
mod page_stylesheet_task_body_test_support;
mod page_task_checkpoint;
mod page_task_completion;

use backend_node_registry::new_shared_renderer_backend_node_registry;
use css_agent_state::RendererCssAgentSessionState;
use dom_agent_state::RendererDomAgentState;
#[cfg(test)]
pub(crate) use page_dom_manipulation_test_support::PageDomManipulationTestFamily;
#[cfg(test)]
pub(crate) use page_selected_task_test_harness::{
    ClaimedPageSelectedTaskForTest, PageSelectedTaskTestSelector,
};
pub(crate) use page_task_completion::{IntoPageTaskCompletion, PageTaskCompletion};
mod page_text_track_default_mode;
mod page_text_track_default_mode_task_completion;
mod page_text_track_load;
mod page_text_track_load_task_completion;
mod page_timer;
mod page_typed_immediate_source;
mod page_user_interaction;
mod page_v8_foreground_task;
mod page_view_transition_update;
mod page_webcrypto_task;
mod page_websocket;
mod page_window_message;
mod page_worker_host_bridge;
#[cfg(test)]
mod page_worker_host_bridge_body_test_support;
mod parser_completion;
pub(in crate::runtime) use parser_completion::ParseTimeMainParserBoundaryOutcome;
mod parser_continuation;
mod parser_deferred_classic;
mod parser_module_ready;
mod parser_owned_document_script;
mod parser_owned_module_completion;
mod parser_task_completion;
mod selected_page_task;

pub(crate) use page_broadcast_channel_delivery::AuthorizedCurrentBroadcastChannelDelivery;
pub(crate) use page_child_frame_task::{
    AuthorizedCurrentPageChildClassicScriptSourceLoad, AuthorizedCurrentPageChildDocumentLifecycle,
    AuthorizedCurrentPageChildDocumentScriptReady, AuthorizedCurrentPageChildHostLoad,
    AuthorizedCurrentPageChildParserModuleRootStart,
    AuthorizedCurrentPageChildRealmMaterialization,
};
pub(crate) use page_child_module_dependency_fetch_start::AuthorizedCurrentChildModuleDependencyFetchStart;
pub(crate) use page_child_module_script_terminal::AuthorizedCurrentChildModuleScriptTerminal;
pub(crate) use page_child_modulepreload_event_action::AuthorizedCurrentChildModulepreloadEventAction;
pub(crate) use page_child_navigation_commit::AuthorizedCurrentPageChildNavigationCommit;
pub(crate) use page_dedicated_worker_client_event::AuthorizedCurrentPageDedicatedWorkerClientEvent;
pub(crate) use page_dynamic_import_owner_action::AuthorizedCurrentChildDynamicImportOwnerAction;
pub(crate) use page_element_toggle_event::AuthorizedCurrentPageElementToggleEvent;
pub(crate) use page_file_entry_file_callback::AuthorizedCurrentPageFileEntryFileCallback;
pub(crate) use page_file_reading::AuthorizedCurrentPageFileReadingTask;
pub(crate) use page_hash_change_delivery::AuthorizedCurrentPageHashChangeDelivery;
pub(crate) use page_history_traversal::AuthorizedCurrentPageHistoryTraversal;
pub(crate) use page_image_load_event::AuthorizedCurrentPageImageLoadEvent;
pub(crate) use page_indexed_db_task::AuthorizedCurrentPageIndexedDbTask;
pub(crate) use page_media_element_event::AuthorizedCurrentPageMediaElementEvent;
pub(crate) use page_message_port_delivery::AuthorizedCurrentPageMessagePortDelivery;
pub(crate) use page_misc_platform_api::AuthorizedCurrentPageMiscPlatformApiTask;
pub(crate) use page_module_reaction::AuthorizedCurrentPageModuleReaction;
pub(crate) use page_modulepreload_start::AuthorizedCurrentChildModulepreloadStartTask;
pub(crate) use page_navigation_api_task::AuthorizedCurrentPageNavigationApiTask;
pub(crate) use page_opfs_task::AuthorizedCurrentPageOpfsTask;
pub(crate) use page_popup_load_event::AuthorizedCurrentPagePopupLoadEvent;
pub(crate) use page_rendering_update::AuthorizedCurrentPageRenderingUpdate;
pub(crate) use page_resource_completion::{
    AuthorizedCurrentChildDocumentLoadCompletion, AuthorizedCurrentChildModuleFetchCompletion,
    AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion,
    AuthorizedCurrentMainDynamicImportGraphFetchCompletion,
    AuthorizedCurrentMainParserModuleGraphFetchCompletion,
    AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion,
    AuthorizedCurrentPopupClassicScriptLoadCompletion,
    AuthorizedCurrentPopupDocumentLoadCompletion, AuthorizedLiveMainModulepreloadFetchCompletion,
    CurrentChildDocumentLoadApplication,
};
pub(crate) use page_service_worker_client_message::AuthorizedCurrentPageServiceWorkerClientMessage;
pub(crate) use page_service_worker_internal::AuthorizedCurrentPageServiceWorkerInternalTask;
pub(crate) use page_shared_worker_client_event::AuthorizedCurrentPageSharedWorkerClientEvent;
pub(crate) use page_storage_event_delivery::AuthorizedCurrentPageStorageEventDelivery;
pub(crate) use page_text_track_default_mode::AuthorizedCurrentPageTextTrackDefaultMode;
pub(crate) use page_text_track_load::AuthorizedCurrentPageTextTrackLoad;
pub(crate) use page_typed_immediate_source::AuthorizedCurrentWindowDocumentTask;
pub(crate) use page_user_interaction::AuthorizedCurrentPageUserInteractionTask;
pub(crate) use page_view_transition_update::AuthorizedCurrentPageViewTransitionUpdate;
pub(crate) use page_webcrypto_task::AuthorizedCurrentPageWebCryptoTask;
pub(crate) use page_window_message::AuthorizedCurrentPageWindowMessage;

pub(in crate::runtime) use super::document_lifecycle_turn::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, DocumentLifecycleTurnReadiness,
};

#[cfg(test)]
async fn wait_for_page_timer_deadline(deadline: Option<Instant>) {
    let Some(deadline) = deadline else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PageVmRuntimeCommandLifecycleAdvance {
    Pending,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PageVmRuntimeCommandOutputScopeId(pub(super) u64);

pub(super) enum PageVmRuntimeCommandLifecycleTarget {
    AwaitingExplicitDocumentReplacement,
    Exact(RendererDocumentLifecycleIdentity),
}

pub(super) struct PageVmRuntimeCommandOutputScope {
    pub(super) id: PageVmRuntimeCommandOutputScopeId,
    pub(super) inspector_session_id: Option<String>,
    pub(super) protocol_configuration_command:
        Option<RendererInspectorProtocolConfigurationCommand>,
    pub(super) recorder: RendererRuntimeCommandOutputRecorder,
    pub(super) lifecycle_target: PageVmRuntimeCommandLifecycleTarget,
}

struct PageOwnedScriptExecutionOutcome {
    run: ScriptRun,
    activity: PageOwnedDocumentScriptBodyActivity,
}

impl PageOwnedScriptExecutionOutcome {
    fn without_page_code_or_event_dispatch(run: ScriptRun) -> Self {
        Self {
            run,
            activity: PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch,
        }
    }

    fn with_page_code_or_event_dispatch(run: ScriptRun) -> Self {
        Self {
            run,
            activity: PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch,
        }
    }

    fn from_prepared_script_activity(
        run: ScriptRun,
        activity: crate::script_vm::PreparedScriptBodyActivity,
    ) -> Self {
        match activity {
            crate::script_vm::PreparedScriptBodyActivity::NotEntered => {
                Self::without_page_code_or_event_dispatch(run)
            }
            crate::script_vm::PreparedScriptBodyActivity::Entered => {
                Self::with_page_code_or_event_dispatch(run)
            }
        }
    }

    fn note_terminal_activity(
        mut self,
        activity: crate::script_vm::ScriptTerminalBodyActivity,
    ) -> Self {
        if activity == crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted {
            self.activity = PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch;
        }
        self
    }

    fn into_run(self) -> ScriptRun {
        self.run
    }

    fn into_parts(self) -> (ScriptRun, PageOwnedDocumentScriptBodyActivity) {
        (self.run, self.activity)
    }

    fn into_body_execution(self) -> PageOwnedDocumentScriptBodyExecution {
        match self.activity {
            PageOwnedDocumentScriptBodyActivity::NoPageCodeOrEventDispatch => {
                PageOwnedDocumentScriptBodyExecution::without_page_code_or_event_dispatch(self.run)
            }
            PageOwnedDocumentScriptBodyActivity::PageCodeOrEventDispatch => {
                PageOwnedDocumentScriptBodyExecution::with_page_code_or_event_dispatch(self.run)
            }
        }
    }
}

struct MainDocumentLifecycleTaskRun {
    completion: MainDocumentLifecycleCompletion,
    checkpoint_elapsed_ms: u128,
    lifecycle_task_elapsed_ms: u128,
    lifecycle_elapsed_ms: u128,
}

struct PageOwnedMainDocumentPostParseBodyRun {
    execution: MainDocumentPostParseExecution,
    body_elapsed_ms: u128,
}

struct PageOwnedConnectedStyleLoadTaskRun {
    dispatch_elapsed_ms: u128,
    lifecycle_elapsed_ms: u128,
}

struct AwaitedOwnerLocalPageVm {
    ptr: NonNull<PageVm>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl AwaitedOwnerLocalPageVm {
    fn new(page_vm: &mut PageVm) -> Self {
        Self {
            ptr: NonNull::from(page_vm),
            _not_send_or_sync: PhantomData,
        }
    }

    fn get_mut(&mut self) -> &mut PageVm {
        // SAFETY: this handle is only created immediately before a
        // `run_named_owner_local_task` spawn, and every caller awaits that task
        // before touching the borrowed `PageVm` again. The Rc marker keeps the
        // handle !Send/!Sync so it remains an owner-local task boundary.
        unsafe { self.ptr.as_mut() }
    }
}

enum ModuleScriptEvaluationStart {
    Completed(crate::script_vm::PreparedScriptBodyActivity),
    Pending {
        root_entry: crate::module_runtime::ModuleEntryId,
        reaction_id: u64,
    },
}

struct ModuleScriptEvaluationStartFailure {
    error: crate::module_runtime::ModuleLoadError,
    body_activity: crate::script_vm::PreparedScriptBodyActivity,
}

impl ModuleScriptEvaluationStartFailure {
    fn new(
        error: crate::module_runtime::ModuleLoadError,
        body_activity: crate::script_vm::PreparedScriptBodyActivity,
    ) -> Self {
        Self {
            error,
            body_activity,
        }
    }

    fn into_parts(
        self,
    ) -> (
        crate::module_runtime::ModuleLoadError,
        crate::script_vm::PreparedScriptBodyActivity,
    ) {
        (self.error, self.body_activity)
    }
}

fn wrap_native_esm_module_load_error(
    prefix: &str,
    error: crate::module_runtime::ModuleLoadError,
) -> crate::module_runtime::ModuleLoadError {
    let wrapped = crate::module_runtime::ModuleLoadError::new(
        error.stage(),
        format!("{prefix}: {}", error.message()),
    );
    match error.error_constructor() {
        Some(error_constructor) => wrapped.with_error_constructor(error_constructor),
        None => wrapped,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScriptExecutionReportSnapshotSignature {
    runs: usize,
    globals: usize,
    globals_snapshot_state: crate::types::ScriptGlobalsSnapshotState,
    observable_outputs: usize,
    network_outputs: usize,
}

impl ScriptExecutionReportSnapshotSignature {
    fn from_report(report: &ScriptExecutionReport) -> Self {
        Self {
            runs: report.runs().len(),
            globals: report.globals().len(),
            globals_snapshot_state: report.globals_snapshot_state(),
            observable_outputs: report.observable_output_items().len(),
            network_outputs: report.network_output_items().len(),
        }
    }
}

fn content_security_policy_violation_type(
    violation: &crate::content_security_policy::ContentSecurityPolicyUrlViolation,
) -> ContentSecurityPolicyViolationType {
    match violation.blocked_uri.as_str() {
        "inline" => ContentSecurityPolicyViolationType::Inline,
        "eval" => ContentSecurityPolicyViolationType::Eval,
        "wasm-eval" => ContentSecurityPolicyViolationType::WasmEval,
        "trusted-types-sink" => ContentSecurityPolicyViolationType::TrustedTypesSink,
        _ if violation.effective_directive == "trusted-types" => {
            ContentSecurityPolicyViolationType::TrustedTypesPolicy
        }
        _ => ContentSecurityPolicyViolationType::Url,
    }
}

enum RuntimeOwnedModuleScriptContinuation {
    Graph(ModuleScriptContinuation),
    Evaluation(ModuleScriptEvaluationContinuation),
}

enum PageOwnedScriptFailureClassification {
    LegacyMessageText,
    Typed {
        dynamic_kind: crate::dynamic_script_owner::DynamicScriptFailureKind,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    },
}

impl PageOwnedScriptFailureClassification {
    fn from_prepared_script_error(
        script: &PreparedScript,
        error: &crate::script_vm::PreparedScriptExecutionError,
    ) -> Self {
        error.module_load_stage().map_or(
            PageOwnedScriptFailureClassification::LegacyMessageText,
            |stage| PageOwnedScriptFailureClassification::Typed {
                dynamic_kind:
                    crate::dynamic_script_owner::DynamicScriptOwner::module_load_failure_kind(
                        script, stage,
                    ),
                module_failure_policy: error.module_failure_policy(),
                error_constructor: error.error_constructor(),
            },
        )
    }

    fn from_module_load_error(
        script: &PreparedScript,
        error: &crate::module_runtime::ModuleLoadError,
    ) -> Self {
        let module_failure_policy = crate::host::ModuleFailurePolicy::for_module_load_error(error);
        PageOwnedScriptFailureClassification::Typed {
            dynamic_kind: crate::dynamic_script_owner::DynamicScriptOwner::module_load_failure_kind(
                script,
                error.stage(),
            ),
            module_failure_policy: Some(module_failure_policy),
            error_constructor: error.error_constructor(),
        }
    }
}

fn complete_prepared_script_execution_success(
    script: PreparedScript,
) -> PageOwnedScriptExecutionOutcome {
    PageOwnedScriptExecutionOutcome::without_page_code_or_event_dispatch(ScriptRun::executed(
        script.node_id,
        script.kind,
        script.mode,
        script.source_kind,
        script.url,
    ))
}

fn complete_prepared_script_execution_success_with_activity(
    script: PreparedScript,
    activity: crate::script_vm::PreparedScriptBodyActivity,
) -> PageOwnedScriptExecutionOutcome {
    PageOwnedScriptExecutionOutcome::from_prepared_script_activity(
        ScriptRun::executed(
            script.node_id,
            script.kind,
            script.mode,
            script.source_kind,
            script.url,
        ),
        activity,
    )
}

fn complete_prepared_script_execution_failure_report(
    script: PreparedScript,
    error: String,
) -> PageOwnedScriptExecutionOutcome {
    PageOwnedScriptExecutionOutcome::without_page_code_or_event_dispatch(ScriptRun::failed(
        script.node_id,
        script.kind,
        script.mode,
        script.source_kind,
        script.url,
        error,
    ))
}

fn complete_prepared_script_execution_failure_report_with_activity(
    script: PreparedScript,
    error: String,
    activity: crate::script_vm::PreparedScriptBodyActivity,
) -> PageOwnedScriptExecutionOutcome {
    PageOwnedScriptExecutionOutcome::from_prepared_script_activity(
        ScriptRun::failed(
            script.node_id,
            script.kind,
            script.mode,
            script.source_kind,
            script.url,
            error,
        ),
        activity,
    )
}

fn complete_prepared_script_execution_failure(
    vm: &mut ScriptVm,
    script: PreparedScript,
    completion_owner: ModuleScriptCompletionOwner,
    dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
    error: String,
    failure_classification: PageOwnedScriptFailureClassification,
) -> PageOwnedScriptExecutionOutcome {
    if completion_owner.is_runtime_owned() {
        match failure_classification {
            PageOwnedScriptFailureClassification::Typed {
                dynamic_kind,
                module_failure_policy,
                error_constructor,
            } => vm.finish_runtime_owned_script_failure_with_kind(
                dynamic_script_owner_id,
                &script,
                &error,
                dynamic_kind,
                module_failure_policy,
                error_constructor,
            ),
            PageOwnedScriptFailureClassification::LegacyMessageText => {
                vm.finish_runtime_owned_script_failure(dynamic_script_owner_id, &script, &error);
            }
        }
    } else if vm.parser_owned_inline_importmap_reports_window_error_immediately(&script) {
        vm.report_parser_import_map_registration_failure_and_finish_algorithm_best_effort(
            &error,
            Some(script.url.as_str()),
        );
    } else if vm.parser_owned_module_reports_failure_immediately(&script) {
        let (module_failure_policy, error_constructor) = match failure_classification {
            PageOwnedScriptFailureClassification::Typed {
                module_failure_policy,
                error_constructor,
                ..
            } => (module_failure_policy, error_constructor),
            PageOwnedScriptFailureClassification::LegacyMessageText => (None, None),
        };
        vm.dispatch_parser_owned_module_failure_and_finish_settlement_best_effort(
            &script,
            &error,
            module_failure_policy,
            error_constructor,
        );
    } else {
        let (module_failure_policy, error_constructor) = match failure_classification {
            PageOwnedScriptFailureClassification::Typed {
                module_failure_policy,
                error_constructor,
                ..
            } => (module_failure_policy, error_constructor),
            PageOwnedScriptFailureClassification::LegacyMessageText => (None, None),
        };
        vm.enqueue_script_failure_lifecycle_work_best_effort(
            &script,
            &error,
            module_failure_policy,
            error_constructor,
        );
    }
    complete_prepared_script_execution_failure_report(script, error)
}

fn complete_page_owned_prepared_script_execution_failure_body(
    vm: &mut ScriptVm,
    script: PreparedScript,
    completion_owner: ModuleScriptCompletionOwner,
    dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
    error: String,
    failure_classification: PageOwnedScriptFailureClassification,
    prepared_script_activity: crate::script_vm::PreparedScriptBodyActivity,
) -> PageOwnedScriptExecutionOutcome {
    let terminal_activity = if completion_owner.is_runtime_owned() {
        let (dynamic_kind, module_failure_policy, error_constructor) = match failure_classification
        {
            PageOwnedScriptFailureClassification::Typed {
                dynamic_kind,
                module_failure_policy,
                error_constructor,
            } => (dynamic_kind, module_failure_policy, error_constructor),
            PageOwnedScriptFailureClassification::LegacyMessageText => (
                crate::dynamic_script_owner::DynamicScriptOwner::legacy_message_failure_kind(
                    &script, &error,
                ),
                None,
                None,
            ),
        };
        vm.finish_runtime_owned_script_failure_body_with_kind(
            dynamic_script_owner_id,
            &script,
            &error,
            dynamic_kind,
            module_failure_policy,
            error_constructor,
        )
    } else if vm.parser_owned_inline_importmap_reports_window_error_immediately(&script) {
        vm.report_window_error_body_best_effort(&error, Some(script.url.as_str()), None);
        crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
    } else if vm.parser_owned_module_reports_failure_immediately(&script) {
        let (module_failure_policy, error_constructor) = match failure_classification {
            PageOwnedScriptFailureClassification::Typed {
                module_failure_policy,
                error_constructor,
                ..
            } => (module_failure_policy, error_constructor),
            PageOwnedScriptFailureClassification::LegacyMessageText => (None, None),
        };
        vm.dispatch_current_prepared_script_error_body_best_effort(
            &script,
            &error,
            module_failure_policy,
            error_constructor,
        )
    } else {
        let (module_failure_policy, error_constructor) = match failure_classification {
            PageOwnedScriptFailureClassification::Typed {
                module_failure_policy,
                error_constructor,
                ..
            } => (module_failure_policy, error_constructor),
            PageOwnedScriptFailureClassification::LegacyMessageText => (None, None),
        };
        vm.enqueue_script_failure_lifecycle_work_best_effort(
            &script,
            &error,
            module_failure_policy,
            error_constructor,
        );
        crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch
    };

    complete_prepared_script_execution_failure_report_with_activity(
        script,
        error,
        prepared_script_activity,
    )
    .note_terminal_activity(terminal_activity)
}

async fn execute_prepared_script_on_script_execution_lane(
    local_executor: &JsLocalExecutor,
    loader: &ResourceRequestClient,
    vm: &mut ScriptVm,
    script: PreparedScript,
    mut runtime_script_claim: Option<DynamicScriptPageTaskClaim>,
    script_execution_disabled: bool,
) -> PageOwnedScriptExecutionOutcome {
    debug_assert!(
        is_on_script_execution_lane_for(local_executor),
        "prepared script execution must stay on the current script execution lane"
    );
    let dynamic_script_owner_id = runtime_script_claim
        .as_ref()
        .map(DynamicScriptPageTaskClaim::id);
    if script_execution_disabled {
        if let Some(claim) = runtime_script_claim.take() {
            vm.cancel_claimed_runtime_owned_script_load_delay_body(claim, &script);
        }
        return PageOwnedScriptExecutionOutcome::without_page_code_or_event_dispatch(
            ScriptRun::skipped(
                script.node_id,
                script.kind,
                script.mode,
                script.source_kind,
                script.url,
                ScriptSkipReason::ScriptExecutionDisabled,
            ),
        );
    }
    if script.kind == crate::types::ScriptKind::Module
        && let Some(claim) = runtime_script_claim.take()
    {
        vm.restore_runtime_owned_script_page_task_claim(claim);
    }
    let completion_owner = if vm.prepared_script_uses_runtime_owned_page_task_execution(&script) {
        ModuleScriptCompletionOwner::Runtime
    } else {
        ModuleScriptCompletionOwner::Parser
    };
    let execution_result = vm
        .run_prepared_script(loader, &script, dynamic_script_owner_id)
        .await;

    match execution_result {
        Ok(PreparedScriptExecutionOutcome::Completed(activity)) => {
            let mut outcome =
                complete_prepared_script_execution_success_with_activity(script.clone(), activity);
            if completion_owner.is_runtime_owned() {
                let terminal_activity = if let Some(claim) = runtime_script_claim.take() {
                    vm.finish_claimed_runtime_owned_script_success_body(claim, &script)
                } else if let Some(owner_id) = dynamic_script_owner_id {
                    vm.finish_runtime_owned_script_success_body(owner_id, &script)
                } else {
                    crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch
                };
                outcome = outcome.note_terminal_activity(terminal_activity);
            }
            outcome
        }
        Ok(PreparedScriptExecutionOutcome::DeferredModuleCompletion) => {
            complete_prepared_script_execution_success(script)
        }
        Ok(PreparedScriptExecutionOutcome::Dropped(activity)) => {
            if let Some(claim) = runtime_script_claim.take() {
                vm.cancel_claimed_runtime_owned_script_load_delay_body(claim, &script);
            } else if let Some(owner_id) = dynamic_script_owner_id {
                vm.cancel_runtime_owned_script_load_delay_body(&script, owner_id);
            }
            complete_prepared_script_execution_success_with_activity(script, activity)
        }
        Err(error) => {
            let prepared_script_activity = error.body_activity();
            let failure_classification =
                PageOwnedScriptFailureClassification::from_prepared_script_error(&script, &error);
            if let Some(claim) = runtime_script_claim.take() {
                let (module_failure_policy, error_constructor) = match failure_classification {
                    PageOwnedScriptFailureClassification::LegacyMessageText => (None, None),
                    PageOwnedScriptFailureClassification::Typed {
                        module_failure_policy,
                        error_constructor,
                        ..
                    } => (module_failure_policy, error_constructor),
                };
                let message = error.into_message();
                let terminal_activity = vm.finish_claimed_runtime_owned_script_failure_body(
                    claim,
                    &script,
                    &message,
                    module_failure_policy,
                    error_constructor,
                );
                return complete_prepared_script_execution_failure_report_with_activity(
                    script,
                    message,
                    prepared_script_activity,
                )
                .note_terminal_activity(terminal_activity);
            }
            complete_page_owned_prepared_script_execution_failure_body(
                vm,
                script,
                completion_owner,
                dynamic_script_owner_id,
                error.into_message(),
                failure_classification,
                prepared_script_activity,
            )
        }
    }
}

async fn execute_main_document_post_parse_body_on_owner_local_task(
    page_vm: &mut PageVm,
    selected_owner: MainDocumentPostParseOwner,
    work: MainDocumentPostParseWork,
) -> Result<PageOwnedMainDocumentPostParseBodyRun> {
    // CSP and script terminal work can synchronously enter page callbacks.
    // Keep body entry on a fresh owner-local task so V8 is not invoked from a
    // deep phase-one poll stack. The task-end checkpoint deliberately remains
    // outside this body-only boundary.
    let local_executor = page_vm.local_executor.clone();
    let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(page_vm);
    run_named_owner_local_task(
        local_executor,
        "main-document post-parse body local task channel closed",
        async move {
            let page_vm = page_vm_ref.get_mut();
            page_vm.ensure_document_replacement_lifecycle_journal_is_valid()?;
            page_vm.request_pending_cross_document_navigation_termination();
            let body_started = Instant::now();
            let execution = page_vm
                .vm_mut()
                .execute_main_document_post_parse_body(selected_owner, work);
            page_vm.request_pending_cross_document_navigation_termination();
            Ok(PageOwnedMainDocumentPostParseBodyRun {
                execution,
                body_elapsed_ms: body_started.elapsed().as_millis(),
            })
        },
    )
    .await
}

async fn execute_connected_style_load_task_on_owner_local_task(
    loader: &ResourceRequestClient,
    page_vm: &mut PageVm,
    ready: crate::document_runtime::ReadyConnectedStyleLoad,
) -> Result<PageOwnedConnectedStyleLoadTaskRun> {
    // Style load/error handlers are page callbacks too. Dispatch them from a
    // fresh owner-local task so V8 does not inherit deep lifecycle poll stacks.
    let local_executor = page_vm.local_executor.clone();
    let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(page_vm);
    let loader = loader.clone();
    run_named_owner_local_task(
        local_executor,
        "page-owned connected style load local task channel closed",
        async move {
            let page_vm = page_vm_ref.get_mut();
            let dispatch_started = Instant::now();
            let binding = ready.load_event_binding();
            let dispatched = page_vm.vm_mut().dispatch_connected_style_load(ready);
            page_vm.vm_mut().settle_connected_style_load(binding);
            if dispatched {
                page_vm.finish_selected_page_callback_task(&loader).await?;
            } else {
                page_vm.finish_selected_page_task_checkpoint()?;
            }
            let dispatch_elapsed_ms = dispatch_started.elapsed().as_millis();

            let lifecycle_started = Instant::now();
            page_vm
                .vm_mut()
                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
            let lifecycle_elapsed_ms = lifecycle_started.elapsed().as_millis();

            Ok(PageOwnedConnectedStyleLoadTaskRun {
                dispatch_elapsed_ms,
                lifecycle_elapsed_ms,
            })
        },
    )
    .await
}

async fn execute_page_owned_work_on_script_execution_lane(
    loader: &ResourceRequestClient,
    page_vm: &mut PageVm,
    work: PostParsePageOwnedWork,
) -> Result<parser_continuation::PostParsePageOwnedExecution> {
    debug_assert!(
        is_on_script_execution_lane_for(&page_vm.local_executor),
        "page-owned tasks must execute on the current script execution lane"
    );
    let lifecycle_work = match work {
        PostParsePageOwnedWork::Lifecycle(work) => *work,
        PostParsePageOwnedWork::DocumentScript(work)
        | PostParsePageOwnedWork::DocumentScriptWithStylesheetSnapshot { work, .. } => {
            let execution =
                page_owned_document_script::MainPageOwnedDocumentScriptOwner::new(page_vm, loader)
                    .run_work(*work)
                    .await?;
            return Ok(
                parser_continuation::PostParsePageOwnedExecution::document_script(execution),
            );
        }
    };
    let task_phase = lifecycle_work.phase_label();
    let cdp_nav_timing_enabled = moli_trace::cdp_nav_timing_enabled();
    if let Some(body) = MainDocumentLifecycleBody::from_post_parse_work(&lifecycle_work) {
        let task_started = Instant::now();
        let run = main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task(
            page_vm,
            body,
        )
        .await?;
        let execution = run.completion;
        let target: MainDocumentLifecycleTargetEffect = execution.target();
        let callback: MainDocumentLifecycleCallbackEffect = execution.callback();
        tracing::debug!(
            kind = ?execution.kind(),
            owner = ?execution.owner(),
            ?target,
            ?callback,
            "reconciled typed main-document lifecycle execution"
        );
        match execution.into_followup() {
            MainDocumentLifecycleFollowup::None => {}
            MainDocumentLifecycleFollowup::ScheduleInternalLoading { task, ready_at } => {
                page_vm
                    .vm()
                    .schedule_page_internal_loading_task(task, ready_at)?;
            }
        }
        tracing::debug!(
            phase = task_phase,
            total_elapsed_ms = task_started.elapsed().as_millis(),
            checkpoint_elapsed_ms = run.checkpoint_elapsed_ms,
            lifecycle_task_elapsed_ms = run.lifecycle_task_elapsed_ms,
            lifecycle_elapsed_ms = run.lifecycle_elapsed_ms,
            "main-document lifecycle task completed"
        );
        return Ok(parser_continuation::PostParsePageOwnedExecution::ordinary(
            None,
        ));
    }

    let lifecycle_work = match lifecycle_work {
        PostParseLifecycleWork::AdvanceMainParserDeferredScripts {
            owner,
            initial_count: _,
        } => {
            let execution = page_vm
                .run_next_main_parser_deferred_script(loader, owner)
                .await?;
            return Ok(
                parser_continuation::PostParsePageOwnedExecution::main_parser_continuation(
                    execution,
                ),
            );
        }
        work => work,
    };
    if let PostParseLifecycleWork::DispatchConnectedStyleLoad(ready) = &lifecycle_work {
        let task_started = Instant::now();
        tracing::debug!(
            phase = task_phase,
            "executing page-owned connected style load task"
        );
        let run =
            execute_connected_style_load_task_on_owner_local_task(loader, page_vm, ready.clone())
                .await?;
        tracing::debug!(
            phase = task_phase,
            total_elapsed_ms = task_started.elapsed().as_millis(),
            dispatch_elapsed_ms = run.dispatch_elapsed_ms,
            lifecycle_elapsed_ms = run.lifecycle_elapsed_ms,
            "page-owned connected style load task completed"
        );
        if cdp_nav_timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = task_phase,
                total_elapsed_ms = task_started.elapsed().as_millis(),
                dispatch_elapsed_ms = run.dispatch_elapsed_ms,
                lifecycle_elapsed_ms = run.lifecycle_elapsed_ms,
                stage = "page_owned_connected_style_load_task_completed",
            );
        }
        Ok(parser_continuation::PostParsePageOwnedExecution::ordinary(
            None,
        ))
    } else {
        let task_started = Instant::now();
        tracing::debug!(phase = task_phase, "executing typed post-parse task body");
        let work = match MainDocumentPostParseWork::try_from_lifecycle(lifecycle_work) {
            Ok(work) => work,
            Err(work) => anyhow::bail!("post-parse work escaped its dedicated executor: {work:?}"),
        };
        let Some(selected_owner) = page_vm.vm().current_main_document_post_parse_owner() else {
            return Ok(
                parser_continuation::PostParsePageOwnedExecution::main_document_post_parse(
                    work.discarded_stale(None),
                ),
            );
        };
        let run = execute_main_document_post_parse_body_on_owner_local_task(
            page_vm,
            selected_owner,
            work,
        )
        .await?;
        tracing::debug!(
            phase = task_phase,
            total_elapsed_ms = task_started.elapsed().as_millis(),
            body_elapsed_ms = run.body_elapsed_ms,
            kind = run.execution.kind(),
            target = ?run.execution.target(),
            "typed post-parse task body completed"
        );
        if cdp_nav_timing_enabled {
            tracing::info!(
                target: "moli_cdp_nav_timing",
                phase = task_phase,
                total_elapsed_ms = task_started.elapsed().as_millis(),
                body_elapsed_ms = run.body_elapsed_ms,
                stage = "page_owned_post_parse_task_body_completed",
            );
        }
        Ok(
            parser_continuation::PostParsePageOwnedExecution::main_document_post_parse(
                run.execution,
            ),
        )
    }
}

/// Environment configuration shared across all `PageVm` construction paths.
///
/// Groups the script-visible CDP/network tunables forwarded to the underlying
/// `ScriptVm`. Runtime scheduling hooks live in [`PageVmRuntimeHooks`] so
/// environment setup does not grow owner-loop wiring fields.
#[derive(Clone)]
pub(crate) struct PageVmEnvConfig {
    pub(crate) root_frame_id: Option<String>,
    pub(crate) main_document_commit: Option<super::RendererMainDocumentCommit>,
    pub(crate) top_level_storage_key: Option<moli_storage_key::MoliStorageKey>,
    pub(crate) web_storage: crate::RendererWebStorageHandles,
    pub(crate) document_start_scripts: Vec<DocumentStartScript>,
    pub(crate) runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub(crate) runtime_inspector_session_restore_snapshots:
        Vec<RendererInspectorSessionRestoreSnapshot>,
    pub(crate) runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub(crate) permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub(crate) extra_http_headers: Vec<(String, String)>,
    pub(crate) document_content_security_policies: Vec<String>,
    pub(crate) response_content_security_policies: Vec<String>,
    pub(crate) response_content_security_report_only_policies: Vec<String>,
    pub(crate) response_referrer_policy: Option<String>,
    pub(crate) content_security_reporting_endpoints:
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    pub(crate) cross_origin_embedder_policy:
        crate::cross_origin_isolation::CrossOriginEmbedderPolicy,
    pub(crate) document_isolation_policy: crate::cross_origin_isolation::DocumentIsolationPolicy,
    pub(crate) cross_origin_isolated: bool,
    pub(crate) document_default_language: Option<String>,
    pub(crate) document_last_modified: Option<f64>,
    pub(crate) locale_override: Option<String>,
    pub(crate) timezone_override: Option<String>,
    pub(crate) script_execution_disabled: bool,
    pub(crate) bypass_content_security_policy: bool,
    pub(crate) cpu_throttling_rate: f64,
    pub(crate) emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub(crate) idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub(crate) viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub(crate) network_offline: bool,
    pub(crate) blocked_url_patterns: Vec<String>,
    pub(crate) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(crate) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(crate) fetch_subresource_interception_enabled: bool,
    pub(crate) fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    pub(crate) layout_policy: LayoutPolicy,
    pub(crate) wpt_extensions_enabled: bool,
    pub(crate) navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
    pub(crate) reserved_service_worker_client_id:
        Option<crate::service_worker_runtime::ServiceWorkerClientId>,
}

/// Runtime wiring for a live `PageVm`.
///
/// Keep this separate from [`PageVmEnvConfig`]: env config describes the page
/// environment that scripts observe, while hooks describe how the outer render
/// owner schedules and wakes the page.
#[derive(Clone)]
pub(crate) struct PageVmRuntimeHooks {
    owner_wake: Option<RendererOwnerWakeSender>,
    javascript_dialog_runtime: RendererJavaScriptDialogRuntime,
    resource_task_runner: Option<crate::network::RendererResourceTaskRunner>,
    pub(crate) browser_context_runtime: super::RendererBrowserContextRuntime,
    document_lifecycle: Option<RendererDocumentLifecycleJournalHandle>,
    document_lifecycle_install: PageVmDocumentLifecycleInstall,
    renderer_document_isolate_allocator: Option<RendererDocumentIsolateAllocator>,
    renderer_page_script_environment: Option<crate::script_vm::RendererPageScriptEnvironment>,
    renderer_document_isolate_reservation: Option<RendererDocumentIsolateReservation>,
    prepared_renderer_document_isolate_bootstrap:
        Option<Rc<std::cell::RefCell<Option<RendererDocumentIsolateBootstrap>>>>,
    renderer_document_isolate_mode: PageVmRendererDocumentIsolateMode,
    #[cfg(test)]
    // Standalone PageVm fixtures have no outer Browser/NavigationEngine root,
    // so the hooks retain their explicitly created context owner. Fixtures
    // supplied with an external context leave this empty; their caller owns it.
    _standalone_browser_context_owner: Option<Rc<super::RendererBrowserContextRuntimeOwner>>,
    #[cfg(test)]
    // A few low-level PageVm factories construct a standalone request client
    // before returning the PageVm itself. Retain that exact owner beside the
    // hooks so the cloned request handle cannot outlive its semantic runtime.
    // Production PageVms leave this empty because their BrowserContext owns
    // the registered resource runtime.
    _standalone_request_client_owner: Option<Rc<crate::network::ResourceRequestClientOwner>>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PageVmDocumentLifecycleInstall {
    #[default]
    ReuseOrCreateInitial,
    CrossDocumentCommit,
}

#[derive(Clone, Default)]
enum PageVmRendererDocumentIsolateMode {
    #[default]
    RequireOwnerReservation,
    #[cfg(test)]
    StandaloneWithoutOwnerReservationTest {
        residence: crate::page_task_queue::RendererPageTaskTestResidence,
    },
}

#[cfg(test)]
impl Default for PageVmRuntimeHooks {
    fn default() -> Self {
        Self::standalone_base_for_test()
    }
}

struct PageVmRendererDocumentIsolateBootstrap {
    renderer_document_isolate_bootstrap: RendererDocumentIsolateBootstrap,
}

impl PageVmRuntimeHooks {
    #[cfg(test)]
    fn standalone_base_for_test() -> Self {
        let owner = Rc::new(super::RendererBrowserContextRuntime::new());
        let browser_context_runtime = owner.handle();
        Self::bind_standalone_browser_context_owner_for_test(&browser_context_runtime);
        Self {
            owner_wake: None,
            javascript_dialog_runtime: RendererJavaScriptDialogRuntime::default(),
            resource_task_runner: None,
            browser_context_runtime,
            document_lifecycle: None,
            document_lifecycle_install: PageVmDocumentLifecycleInstall::default(),
            renderer_document_isolate_allocator: None,
            renderer_page_script_environment: None,
            renderer_document_isolate_reservation: None,
            prepared_renderer_document_isolate_bootstrap: None,
            renderer_document_isolate_mode: PageVmRendererDocumentIsolateMode::default(),
            _standalone_browser_context_owner: Some(owner),
            _standalone_request_client_owner: None,
        }
    }

    pub(crate) fn owner_wake(&self) -> Option<RendererOwnerWakeSender> {
        self.owner_wake.clone()
    }

    pub(crate) fn resource_task_runner(
        &self,
    ) -> Option<crate::network::RendererResourceTaskRunner> {
        self.resource_task_runner.clone()
    }

    #[cfg(test)]
    /// Builds a low-level test PageVm with a private document isolate.
    ///
    /// This deliberately bypasses renderer-owner reservation and therefore
    /// must not be used as evidence for production owner wiring, shared
    /// document-isolate attachment, or reservation drop behavior.
    pub(crate) fn standalone_without_owner_reservation_for_test() -> Self {
        let residence = crate::page_task_queue::RendererPageTaskTestResidence::new(None);
        let mut hooks = Self::standalone_base_for_test();
        hooks.resource_task_runner = Some(residence.resource_task_runner());
        hooks.renderer_document_isolate_mode =
            PageVmRendererDocumentIsolateMode::StandaloneWithoutOwnerReservationTest { residence };
        hooks
    }

    #[cfg(test)]
    /// Builds a low-level test PageVm whose production typed sources publish
    /// readiness through the supplied final owner-wake route.
    pub(crate) fn standalone_with_owner_wake_without_owner_reservation_for_test(
        owner_wake: RendererOwnerWakeSender,
    ) -> Self {
        let mut hooks = Self::standalone_base_for_test();
        let residence =
            crate::page_task_queue::RendererPageTaskTestResidence::new(Some(owner_wake.clone()));
        hooks.owner_wake = Some(owner_wake);
        hooks.resource_task_runner = Some(residence.resource_task_runner());
        hooks.renderer_document_isolate_mode =
            PageVmRendererDocumentIsolateMode::StandaloneWithoutOwnerReservationTest { residence };
        hooks
    }

    #[cfg(test)]
    /// Builds the same owner-woken standalone fixture while preserving the
    /// caller's browser-context runtime. ServiceWorker and SharedWorker tests
    /// use this to connect their real browser-context producers to the Page's
    /// typed task routes instead of a parallel test queue.
    pub(crate) fn standalone_with_owner_wake_and_browser_context_without_owner_reservation_for_test(
        owner_wake: RendererOwnerWakeSender,
        browser_context_runtime: super::RendererBrowserContextRuntime,
    ) -> Self {
        Self::bind_standalone_browser_context_owner_for_test(&browser_context_runtime);
        let residence =
            crate::page_task_queue::RendererPageTaskTestResidence::new(Some(owner_wake.clone()));
        Self {
            javascript_dialog_runtime: RendererJavaScriptDialogRuntime::default(),
            owner_wake: Some(owner_wake),
            resource_task_runner: Some(residence.resource_task_runner()),
            browser_context_runtime,
            renderer_document_isolate_mode:
                PageVmRendererDocumentIsolateMode::StandaloneWithoutOwnerReservationTest {
                    residence,
                },
            document_lifecycle: None,
            document_lifecycle_install: PageVmDocumentLifecycleInstall::default(),
            renderer_document_isolate_allocator: None,
            renderer_page_script_environment: None,
            renderer_document_isolate_reservation: None,
            prepared_renderer_document_isolate_bootstrap: None,
            _standalone_request_client_owner: None,
            _standalone_browser_context_owner: None,
        }
    }

    #[cfg(test)]
    fn bind_standalone_browser_context_owner_for_test(
        browser_context_runtime: &super::RendererBrowserContextRuntime,
    ) {
        // A production BrowserContext runtime is attached to exactly one
        // renderer owner before any SharedWorker can be launched. This
        // standalone executor has no RendererOwnerHandle, so model that
        // prerequisite explicitly at the fixture boundary. Individual worker
        // tests must not install ad-hoc owner identities after launch.
        browser_context_runtime.set_shared_worker_owner_local_host_id(
            super::RendererOwnerLocalHostId::new_for_testing(1),
        );
    }

    pub(crate) fn with_owner_wake(
        owner_wake: RendererOwnerWakeSender,
        browser_context_runtime: super::RendererBrowserContextRuntime,
    ) -> Self {
        Self {
            javascript_dialog_runtime: RendererJavaScriptDialogRuntime::default(),
            owner_wake: Some(owner_wake),
            resource_task_runner: Some(
                crate::network::RendererResourceTaskRunner::from_current_tokio()
                    .expect("renderer owner must install its resource task runner"),
            ),
            browser_context_runtime,
            document_lifecycle: None,
            document_lifecycle_install: PageVmDocumentLifecycleInstall::ReuseOrCreateInitial,
            renderer_document_isolate_allocator: None,
            renderer_page_script_environment: None,
            renderer_document_isolate_reservation: None,
            prepared_renderer_document_isolate_bootstrap: None,
            renderer_document_isolate_mode:
                PageVmRendererDocumentIsolateMode::RequireOwnerReservation,
            #[cfg(test)]
            _standalone_browser_context_owner: None,
            #[cfg(test)]
            _standalone_request_client_owner: None,
        }
    }

    pub(crate) fn with_renderer_document_isolate_allocator(
        mut self,
        allocator: RendererDocumentIsolateAllocator,
    ) -> Self {
        self.renderer_document_isolate_allocator = Some(allocator);
        self
    }

    pub(crate) fn with_prepared_renderer_document_isolate(
        mut self,
        bootstrap: RendererDocumentIsolateBootstrap,
        reservation: RendererDocumentIsolateReservation,
    ) -> Result<Self> {
        anyhow::ensure!(
            reservation.is_active(),
            "prepared renderer document isolate reservation is no longer active"
        );
        let environment = bootstrap
            .renderer_page_script_environment()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "prepared renderer document isolate is missing its Page environment"
                )
            })?;
        self.renderer_page_script_environment = Some(environment);
        self.renderer_document_isolate_reservation = Some(reservation);
        self.prepared_renderer_document_isolate_bootstrap =
            Some(Rc::new(std::cell::RefCell::new(Some(bootstrap))));
        Ok(self)
    }

    fn for_cross_document_commit(mut self) -> Self {
        self.document_lifecycle_install = PageVmDocumentLifecycleInstall::CrossDocumentCommit;
        self
    }

    #[cfg(test)]
    fn standalone_page_task_residence(
        &self,
    ) -> Option<&crate::page_task_queue::RendererPageTaskTestResidence> {
        match &self.renderer_document_isolate_mode {
            PageVmRendererDocumentIsolateMode::RequireOwnerReservation => None,
            PageVmRendererDocumentIsolateMode::StandaloneWithoutOwnerReservationTest {
                residence,
            } => Some(residence),
        }
    }

    fn install_document_lifecycle(
        &mut self,
        page_id: PageId,
    ) -> Result<(
        RendererDocumentLifecycleJournalHandle,
        RendererDocumentLifecycleIdentity,
    )> {
        let journal = self
            .document_lifecycle
            .clone()
            .unwrap_or_else(|| RendererDocumentLifecycleJournalHandle::new_initial(page_id));
        let identity = match self.document_lifecycle_install {
            PageVmDocumentLifecycleInstall::ReuseOrCreateInitial => journal.identity(),
            PageVmDocumentLifecycleInstall::CrossDocumentCommit => journal
                .start_cross_document()
                .map_err(|transition| {
                anyhow::anyhow!(
                    "renderer document lifecycle rejected cross-document install: {transition:?}"
                )
            })?,
        };
        anyhow::ensure!(
            identity.document.page_id == page_id,
            "renderer document lifecycle page identity does not match PageVm"
        );
        self.document_lifecycle = Some(journal.clone());
        self.document_lifecycle_install = PageVmDocumentLifecycleInstall::ReuseOrCreateInitial;
        Ok((journal, identity))
    }

    fn has_renderer_page_script_environment(&self) -> bool {
        self.renderer_page_script_environment.is_some()
    }

    fn create_renderer_document_isolate_bootstrap(
        &mut self,
        page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
    ) -> Result<PageVmRendererDocumentIsolateBootstrap> {
        if let Some(prepared) = self.prepared_renderer_document_isolate_bootstrap.take() {
            let bootstrap = prepared.borrow_mut().take().ok_or_else(|| {
                anyhow::anyhow!("prepared renderer document isolate was already consumed")
            })?;
            let environment = self
                .renderer_page_script_environment
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("prepared renderer document isolate lost its Page environment")
                })?;
            anyhow::ensure!(
                environment.page_runtime_task_source().identity_key()
                    == page_runtime_task_source.identity_key(),
                "prepared renderer document isolate did not retain its Page runtime task source"
            );
            return Ok(PageVmRendererDocumentIsolateBootstrap {
                renderer_document_isolate_bootstrap: bootstrap,
            });
        }
        if let Some(environment) = self.renderer_page_script_environment.as_ref() {
            anyhow::ensure!(
                self.renderer_document_isolate_allocator.is_some(),
                "page script environment requires a renderer owner allocator"
            );
            anyhow::ensure!(
                environment.page_runtime_task_source().identity_key()
                    == page_runtime_task_source.identity_key(),
                "replacement PageVm did not retain its page runtime task source"
            );
            let renderer_document_isolate_bootstrap =
                environment.bootstrap_replacement_document_isolate()?;
            return Ok(PageVmRendererDocumentIsolateBootstrap {
                renderer_document_isolate_bootstrap,
            });
        }
        match (
            &self.renderer_document_isolate_allocator,
            &self.renderer_document_isolate_mode,
        ) {
            (Some(allocator), _) => {
                let (renderer_document_isolate_bootstrap, reservation) =
                    allocator.reserve_renderer_document_isolate(page_runtime_task_source)?;
                self.renderer_page_script_environment =
                    renderer_document_isolate_bootstrap.renderer_page_script_environment();
                self.renderer_document_isolate_reservation = Some(reservation);
                Ok(PageVmRendererDocumentIsolateBootstrap {
                    renderer_document_isolate_bootstrap,
                })
            }
            #[cfg(test)]
            (
                None,
                PageVmRendererDocumentIsolateMode::StandaloneWithoutOwnerReservationTest {
                    residence,
                },
            ) => {
                let v8_foreground_task_sender = page_runtime_task_source
                    .v8_foreground_task_sender()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "standalone PageVm test is missing its typed V8 foreground source"
                        )
                    })?;
                let renderer_document_isolate_bootstrap = residence.with_owner_runtime(|| {
                    RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
                        v8_foreground_task_sender,
                    )
                });
                Ok(PageVmRendererDocumentIsolateBootstrap {
                    renderer_document_isolate_bootstrap: renderer_document_isolate_bootstrap?,
                })
            }
            (None, PageVmRendererDocumentIsolateMode::RequireOwnerReservation) => {
                #[cfg(test)]
                {
                    Err(anyhow::anyhow!(
                        "PageVmRuntimeHooks::standalone_without_owner_reservation_for_test() is required for direct no-owner test PageVm construction"
                    ))
                }
                #[cfg(not(test))]
                {
                    Err(anyhow::anyhow!(
                        "production PageVm creation requires an owner-local renderer document isolate reservation"
                    ))
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedStylesheetAdmission {
    Admitted,
    DeferredToParser(ScannedStylesheetDeferral),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedStylesheetDeferral {
    FetchInterception,
    MediaMismatch,
    ContentSecurityPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedImageAdmission {
    Admitted,
    DeferredToParser(ScannedImageDeferral),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedImageDeferral {
    Disabled,
    FetchInterception,
    ContentSecurityPolicy,
    ServiceWorker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedScriptAdmission {
    Admitted,
    DeferredToParser(ScannedScriptDeferral),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScannedScriptDeferral {
    ScriptExecutionDisabled,
    FetchInterception,
    ContentSecurityPolicy,
}

pub(crate) struct PageVm {
    pub(super) page_id: PageId,
    pub(super) creation_id: u64,
    pub(super) document_lifecycle: RendererDocumentLifecycleJournalHandle,
    pub(super) runtime_command_output: RendererRuntimeCommandOutput,
    pub(super) pending_runtime_command_output: Option<PageVmRuntimeCommandOutputScope>,
    pub(super) next_runtime_command_output_scope_id: u64,
    pub(super) vm: Option<ScriptVm>,
    pub(super) report: ScriptExecutionReport,
    report_snapshot_cache: Option<(
        ScriptExecutionReportSnapshotSignature,
        Arc<ScriptExecutionReport>,
    )>,
    dom_agent_state: RendererDomAgentState,
    pending_dom_mutation_event_batches: Vec<RendererDomMutationEventBatch>,
    last_published_document_title: String,
    css_agent_sessions: HashMap<Option<String>, RendererCssAgentSessionState>,
    // Page-owned task queue lives on the page VM itself so parse-time turns and later lifecycle
    // turns share one owner-lane carrier. The runtime still uses it in narrow slices today, but
    // it is now a page resource rather than two unrelated local queues created in different
    // phases.
    pub(super) page_task_queue: PageTaskQueue,
    page_action_window: page_action_window::RendererPageActionWindow,
    next_module_script_evaluation_reaction_id: u64,
    target_stage: PageVmInitStage,
    /// Page-level transport/policy view used by navigation and owner-loop
    /// orchestration.
    ///
    /// This is deliberately not a `DocumentResourceLoader`. The authoritative
    /// committed-Document loader lives only in `ScriptVm`'s exact-owner
    /// registry and may change across `document.open()` or navigation.
    pub(super) request_client: ResourceRequestClient,
    pub(super) runtime_isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
    pub(super) permission_overrides: Vec<crate::protocol_types::PermissionOverrideRegistration>,
    pub(super) document_start_scripts: Vec<DocumentStartScript>,
    pub(super) runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    pub(super) runtime_inspector_protocol_configurations:
        BTreeMap<DevToolsSessionKey, RendererInspectorProtocolConfiguration>,
    pub(super) extra_http_headers: Vec<(String, String)>,
    pub(super) locale_override: Option<String>,
    pub(super) timezone_override: Option<String>,
    pub(super) bypass_content_security_policy: bool,
    pub(super) cpu_throttling_rate: f64,
    pub(super) emulated_media: crate::protocol_types::EmulatedMediaOverrides,
    pub(super) idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    pub(super) viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    pub(super) network_offline: bool,
    pub(super) blocked_url_patterns: Vec<String>,
    pub(super) indexed_db_manager: Option<crate::context_bootstrap::WeakIndexedDbManager>,
    pub(super) storage_bucket_store: Option<crate::context_bootstrap::SharedStorageBucketStore>,
    pub(super) fetch_subresource_interception_enabled: bool,
    pub(super) fetch_subresource_interception_resource_type: Option<crate::SubresourceResourceType>,
    pub(super) layout_policy: LayoutPolicy,
    pub(super) wpt_extensions_enabled: bool,
    pub(crate) runtime_hooks: PageVmRuntimeHooks,
    pub(super) navigation_response: Option<PageVmNavigationResponse>,
    /// Exact cross-document navigation whose response installed this PageVm.
    /// The owner-local publication boundary consumes this identity exactly
    /// once before phase one may resume.
    replacement_document_commit_handoff:
        Option<crate::page_task_queue::RendererTopLevelNavigationHandoff>,
    pub(super) local_executor: JsLocalExecutor,
}

#[cfg(test)]
enum PostParseLifecycleLoopAdvance {
    Continue(Box<Option<PostParsePageOwnedTask>>),
    Complete(PostParseLifecycleCompletionAction),
}

pub(in crate::runtime) fn renderer_document_lifecycle_milestone_for_stage(
    stage: PageVmInitStage,
) -> RendererDocumentLifecycleMilestone {
    match stage {
        PageVmInitStage::DomContentLoaded => RendererDocumentLifecycleMilestone::DomContentLoaded,
        PageVmInitStage::Load => RendererDocumentLifecycleMilestone::Load,
    }
}

pub(super) enum ParseTimeLiveExecution {
    ParserOwnedClassicScript {
        execution_context: ParserOwnedClassicScriptExecutionContext,
        script: Box<PreparedScript>,
    },
    ConnectedStyleLoad {
        ready: crate::document_runtime::ReadyConnectedStyleLoad,
    },
    PageOwnedDocumentScript {
        lane: crate::document_script_scheduler::DocumentScriptExecutionLane,
        script: Box<PreparedScript>,
        load_delay_binding: Option<crate::frame_owner_model::MainDocumentScriptLoadDelayLease>,
    },
    PageOwnedWork {
        work: Box<PostParsePageOwnedWork>,
    },
}

pub(super) struct ParseTimeLiveExecutionOutcome {
    navigation_triggered: bool,
    parser_owned_classic_script_completion: Option<ParserOwnedClassicScriptCompletion>,
    main_parser_completion: Option<parser_continuation::MainParserContinuationCompletion>,
}

impl ParseTimeLiveExecutionOutcome {
    fn new(
        navigation_triggered: bool,
        parser_owned_classic_script_completion: Option<ParserOwnedClassicScriptCompletion>,
        main_parser_completion: Option<parser_continuation::MainParserContinuationCompletion>,
    ) -> Self {
        Self {
            navigation_triggered,
            parser_owned_classic_script_completion,
            main_parser_completion,
        }
    }

    pub(super) fn navigation_triggered(&self) -> bool {
        self.navigation_triggered
    }

    pub(super) fn into_parser_owned_classic_script_completion(
        self,
    ) -> Option<ParserOwnedClassicScriptCompletion> {
        self.parser_owned_classic_script_completion
    }

    pub(super) fn into_main_parser_completion(
        self,
    ) -> Option<parser_continuation::MainParserContinuationCompletion> {
        self.main_parser_completion
    }
}

impl fmt::Debug for PageVm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageVm").finish_non_exhaustive()
    }
}

impl Drop for PageVm {
    fn drop(&mut self) {
        let Some(mut vm) = self.vm.take() else {
            return;
        };

        // Stop accepting foreground work before dropping tasks that V8 had
        // already transferred to this PageVm. The isolate remains alive until
        // its ScriptVm and owner pin finish teardown below.
        vm.unregister_document_isolate_platform_for_context_teardown();
        self.page_task_queue.clear_document_owned_tasks();

        // SharedWorker owner membership is not a V8 disposal detail, so release
        // it as soon as the PageVm leaves the renderer owner. Only legacy
        // standalone test construction can require deferred LIFO teardown;
        // production per-page isolates dispose immediately on their owner lane.
        vm.detach_default_inspector_context_for_context_teardown();
        vm.close_page_context_resources_for_context_teardown();
        if vm.requires_deferred_lifo_drop() {
            defer_page_vm_drop(self.creation_id, vm);
        } else {
            drop(vm);
        }
    }
}

#[derive(Default)]
pub(super) struct PageVmDropTracker {
    next_id: u64,
    creation_order: Vec<u64>,
    pending: HashMap<u64, ScriptVm>,
}

fn mark_followed_navigation_document_commit(
    outcome: &mut PageVmFollowedNavigationBuildOutcome,
    handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
) -> Result<()> {
    match outcome {
        PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle { page_vm, .. }
        | PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, .. } => {
            page_vm.prepare_replacement_document_commit(handoff)
        }
        PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(pending) => pending
            .page_vm_mut()
            .prepare_replacement_document_commit(handoff),
        PageVmFollowedNavigationBuildOutcome::Download(_) => Ok(()),
    }
}

impl PageVm {
    pub(super) async fn run_ready_document_write_stylesheet_blocked_script(
        &mut self,
    ) -> Result<bool> {
        if !self
            .vm()
            .document_runtime
            .has_pending_document_write_stylesheet_blocked_script()
        {
            return Ok(false);
        }

        self.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();

        let mut progressed = false;
        loop {
            let ready = self
                .vm_mut()
                .document_runtime
                .pop_ready_connected_style_load_before_parser_blocking_script();
            let Some(ready) = ready else {
                break;
            };
            progressed = true;
            if self
                .execute_parse_time_on_existing_live_document_on_named_owner_local_task(
                    ParseTimeLiveExecution::ConnectedStyleLoad { ready },
                )
                .await?
                .navigation_triggered()
            {
                return Ok(true);
            }
            if !self
                .vm()
                .document_runtime
                .has_pending_document_write_stylesheet_blocked_script()
            {
                return Ok(true);
            }
        }

        if !self
            .vm_mut()
            .document_runtime
            .document_write_stylesheet_blocked_script_is_ready()
        {
            return Ok(progressed);
        }

        self.vm_mut().sync_live_document_style_sources();
        let resumed = self
            .vm_mut()
            .resume_document_write_stylesheet_blocked_script()?;
        if resumed {
            self.vm_mut()
                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        }
        Ok(progressed || resumed)
    }

    pub(super) fn devtools_agent_token(&self) -> RendererDevToolsAgentToken {
        self.vm().devtools_agent_token()
    }

    pub(super) fn settle_renderer_output_publication(
        &mut self,
    ) -> Option<crate::runtime::RendererOutputPublication> {
        // Every owner turn must freeze its DOM facts into the same ordered
        // stream as its other observations before settlement. Command turns
        // have already moved their causal suffix into command records; this
        // call is therefore a no-op for that suffix and a safety boundary for
        // ordinary, lifecycle, error, and maintenance turns.
        self.absorb_pending_dom_mutations_into_output_journal();
        self.record_document_title_change_if_needed();
        self.vm_mut().settle_renderer_output_publication()
    }

    fn record_document_title_change_if_needed(&mut self) {
        // A PageVm can exist without a DevTools-facing Page residence in
        // standalone embeddings and owner-boundary unit tests. Lifecycle
        // progress must not depend on an observer being installed. Keep the
        // last-published value untouched so a later binding still publishes
        // the current title on its first owner settlement.
        if !self.vm().has_renderer_output_journal() {
            return;
        }
        let title = self.vm().document_runtime.dom_host().dom().document_title();
        if title == self.last_published_document_title {
            return;
        }
        self.last_published_document_title.clone_from(&title);
        self.append_renderer_output_records(vec![PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::DocumentTitleChanged(RendererDocumentTitleChanged {
                source_document: self.document_lifecycle.identity(),
                title,
            }),
        )]);
    }

    pub(super) fn append_renderer_output_records(&self, records: Vec<PendingRendererOutputRecord>) {
        self.vm().append_renderer_output_records(records);
    }

    pub(super) fn renderer_output_tail_cursor(
        &self,
    ) -> Option<crate::runtime::RendererOutputCursor> {
        self.vm().renderer_output_tail_cursor()
    }

    pub(super) fn declare_renderer_output_fence(
        &self,
        cursor: crate::runtime::RendererOutputCursor,
    ) -> crate::runtime::RendererOutputFence {
        self.vm().declare_renderer_output_fence(cursor)
    }

    pub(crate) fn devtools_target(&self) -> crate::devtools::target::RendererDevToolsTargetHandle {
        self.vm().devtools_target()
    }

    pub(super) fn close_for_context_teardown(&mut self) {
        let _ = self.document_lifecycle.request_termination(
            self.document_lifecycle.identity(),
            RendererDocumentTerminationReason::Detached,
        );
        let Some(vm) = self.vm.as_mut() else {
            return;
        };
        vm.detach_default_inspector_context_for_context_teardown();
        vm.close_page_context_resources_for_context_teardown();
    }

    pub(super) fn take_page_creation_artifacts(&self) -> RendererPageCreationArtifacts {
        self.document_lifecycle.take_page_creation_artifacts()
    }

    #[cfg(test)]
    pub(super) fn drain_document_lifecycle_events(&self) -> Vec<RendererDocumentLifecycleEvent> {
        self.document_lifecycle.drain_live_events()
    }

    pub(super) fn stop_document_lifecycle(&self) {
        let _ = self.document_lifecycle.request_termination(
            self.document_lifecycle.identity(),
            RendererDocumentTerminationReason::Stopped,
        );
    }

    pub(crate) fn document_lifecycle_wait_outcome(
        &self,
        milestone: RendererDocumentLifecycleMilestone,
    ) -> RendererDocumentLifecycleWaitOutcome {
        RendererDocumentLifecycleWaiter::from_snapshot(
            self.document_lifecycle.current_snapshot(),
            milestone,
        )
        .outcome()
    }

    fn request_pending_cross_document_navigation_termination(&self) -> bool {
        if !self.vm().has_pending_location_navigation()
            || self
                .vm()
                .pending_location_navigation_scheme_is("javascript")
        {
            return false;
        }
        let transition = self.document_lifecycle.request_termination(
            self.document_lifecycle.identity(),
            RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
        );
        debug_assert!(
            matches!(
                transition,
                RendererDocumentLifecycleTransition::Recorded(_)
                    | RendererDocumentLifecycleTransition::Deferred
                    | RendererDocumentLifecycleTransition::Duplicate
            ),
            "pending cross-document navigation should terminate the active renderer document: {transition:?}"
        );
        true
    }

    fn ensure_document_replacement_lifecycle_journal_is_valid(&self) -> Result<()> {
        if let Some(transition) = self.document_lifecycle.take_pending_document_open_error() {
            anyhow::bail!(
                "renderer document lifecycle rejected document.open restart: {transition:?}"
            );
        }
        Ok(())
    }

    fn pending_runtime_command_lifecycle_document(
        &self,
    ) -> Result<RendererDocumentLifecycleIdentity> {
        self.ensure_document_replacement_lifecycle_journal_is_valid()?;
        let current_document = self.document_lifecycle.identity();
        let scope = self
            .pending_runtime_command_output
            .as_ref()
            .ok_or_else(|| {
                anyhow!(
                    "renderer runtime command lifecycle continuation has no active output scope"
                )
            })?;
        let document = match scope.lifecycle_target {
            PageVmRuntimeCommandLifecycleTarget::Exact(document) => document,
            PageVmRuntimeCommandLifecycleTarget::AwaitingExplicitDocumentReplacement => {
                anyhow::bail!(
                    "runtime command lifecycle observer was not bound at the explicit Document replacement boundary"
                )
            }
        };
        ensure!(
            document == current_document,
            "runtime command lifecycle observer was replaced before command completion"
        );
        Ok(document)
    }

    pub(super) fn complete_pending_runtime_command_lifecycle(&mut self) -> Result<()> {
        self.ensure_document_replacement_lifecycle_journal_is_valid()?;
        let current_document = self.document_lifecycle.identity();
        let completion = self
            .pending_runtime_command_output
            .as_ref()
            .ok_or_else(|| {
                anyhow!(
                    "renderer runtime command lifecycle continuation has no active output scope"
                )
            })
            .and_then(|scope| match scope.lifecycle_target {
                // No explicit `document.open()` replacement was installed.
                // A location navigation requested by the command remains a
                // separate Page-owner action and must not turn the command's
                // already-produced inspector response into an error.
                PageVmRuntimeCommandLifecycleTarget::AwaitingExplicitDocumentReplacement => Ok(()),
                PageVmRuntimeCommandLifecycleTarget::Exact(document) => {
                    ensure!(
                        document == current_document,
                        "runtime command lifecycle observer was replaced before command completion"
                    );
                    Ok(())
                }
            });
        match completion {
            Ok(()) => {
                self.finish_pending_runtime_command_output(None, false);
                Ok(())
            }
            Err(error) => {
                let had_response =
                    self.finish_pending_runtime_command_output(Some(error.to_string()), true);
                if had_response { Ok(()) } else { Err(error) }
            }
        }
    }

    pub(super) async fn advance_pending_runtime_command_lifecycle_one_turn(
        &mut self,
        expected_scope_id: PageVmRuntimeCommandOutputScopeId,
    ) -> Result<PageVmRuntimeCommandLifecycleAdvance> {
        ensure!(
            self.pending_runtime_command_output.is_some(),
            "renderer runtime command lifecycle continuation has no active output scope"
        );
        ensure!(
            self.pending_runtime_command_output
                .as_ref()
                .is_some_and(|scope| scope.id == expected_scope_id),
            "renderer runtime command lifecycle continuation does not own the active output scope"
        );

        let observation: Result<PageVmRuntimeCommandLifecycleAdvance> = (|| {
            let _ = self.pending_runtime_command_lifecycle_document()?;
            match self.document_lifecycle_wait_outcome(RendererDocumentLifecycleMilestone::Load) {
                RendererDocumentLifecycleWaitOutcome::Pending => {
                    Ok(PageVmRuntimeCommandLifecycleAdvance::Pending)
                }
                RendererDocumentLifecycleWaitOutcome::Reached(_) => {
                    Ok(PageVmRuntimeCommandLifecycleAdvance::Completed)
                }
                RendererDocumentLifecycleWaitOutcome::Interrupted(termination) => Err(anyhow!(
                    "runtime command Document was interrupted before Load: {:?}",
                    termination.reason
                )),
            }
        })();

        match observation {
            Ok(PageVmRuntimeCommandLifecycleAdvance::Pending) => {
                Ok(PageVmRuntimeCommandLifecycleAdvance::Pending)
            }
            Ok(PageVmRuntimeCommandLifecycleAdvance::Completed) => {
                self.finish_pending_runtime_command_output(None, false);
                Ok(PageVmRuntimeCommandLifecycleAdvance::Completed)
            }
            Err(error) => {
                let had_response =
                    self.finish_pending_runtime_command_output(Some(error.to_string()), true);
                if had_response {
                    Ok(PageVmRuntimeCommandLifecycleAdvance::Completed)
                } else {
                    Err(error)
                }
            }
        }
    }

    fn repeated_document_lifecycle_load_is_pending(&self) -> bool {
        self.document_lifecycle
            .active_document_replacement_drive_identity()
            == Some(self.document_lifecycle.identity())
            && matches!(
                self.document_lifecycle_wait_outcome(RendererDocumentLifecycleMilestone::Load,),
                RendererDocumentLifecycleWaitOutcome::Pending
            )
    }

    pub(super) fn take_renderer_document_isolate_reservation_for_attach(
        &mut self,
    ) -> Option<RendererDocumentIsolateReservation> {
        self.runtime_hooks
            .renderer_document_isolate_reservation
            .as_ref()
            .filter(|reservation| reservation.is_active())
            .cloned()
    }

    pub(super) fn renderer_page_script_environment(
        &self,
    ) -> Option<crate::script_vm::RendererPageScriptEnvironment> {
        self.runtime_hooks.renderer_page_script_environment.clone()
    }

    pub(super) fn javascript_dialog_broker(&self) -> RendererJavaScriptDialogBroker {
        self.runtime_hooks.javascript_dialog_runtime.broker()
    }

    pub(super) fn has_live_script_vm(&self) -> bool {
        self.vm.is_some()
    }

    /// Low-level parser tests intentionally drive a tokenizer step without
    /// installing the phase-one residence that production uses. Only that
    /// explicit standalone fixture may continue directly when no continuation
    /// producer is active.
    #[cfg(test)]
    pub(in crate::runtime) fn permits_direct_parser_budget_continuation_for_test(&self) -> bool {
        self.runtime_hooks
            .standalone_page_task_residence()
            .is_some()
    }

    pub(super) fn has_page_resource_completion_route(&self) -> bool {
        self.page_task_queue.resource_completion_sender().is_some()
    }

    pub(super) fn has_ready_page_networking_task(&self) -> bool {
        let current_document = self.document_lifecycle.identity().document;
        #[cfg(test)]
        if let Some(residence) = self.runtime_hooks.standalone_page_task_residence() {
            return residence
                .task_sources()
                .has_ready_networking_task_for(current_document);
        }
        self.runtime_hooks.owner_wake().is_some_and(|owner_wake| {
            super::owner_local_store::has_ready_page_networking_task_on_bound_owner_local_store(
                owner_wake.token(),
                current_document,
            )
        })
    }

    #[cfg(test)]
    pub(super) fn page_task_executor_sources_for_test(
        &self,
    ) -> crate::page_task_queue::RendererPageOwnedTaskSourcesTestHarness {
        self.runtime_hooks
            .standalone_page_task_residence()
            .expect("direct PageVm executor tests require an explicit Page task residence")
            .task_sources()
    }

    #[cfg(test)]
    pub(super) fn service_worker_task_sender_for_root_for_test(
        &self,
        root_document: RendererDocumentToken,
    ) -> crate::page_task_queue::RendererPageServiceWorkerTaskSender {
        self.runtime_hooks
            .standalone_page_task_residence()
            .expect("direct PageVm executor tests require an explicit Page task residence")
            .service_worker_task_sender_for_root(root_document)
    }

    #[cfg(test)]
    pub(super) fn has_ready_child_frame_semantic_turn_for_test(
        &self,
        expected: crate::frame_owner_model::ChildFrameSemanticTurnKind,
    ) -> bool {
        use crate::{
            frame_owner_model::ChildFrameSemanticTurnKind,
            page_task_queue::RendererPageChildFrameTaskTarget,
        };

        let Some(target) = self
            .page_task_executor_sources_for_test()
            .next_child_frame_task_target()
        else {
            return false;
        };
        matches!(
            (expected, target),
            (
                ChildFrameSemanticTurnKind::RealmMaterialization,
                RendererPageChildFrameTaskTarget::RealmMaterialization(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentLifecycle,
                RendererPageChildFrameTaskTarget::DocumentLifecycle(_)
            ) | (
                ChildFrameSemanticTurnKind::DocumentScriptReady,
                RendererPageChildFrameTaskTarget::DocumentScriptReady(_)
            ) | (
                ChildFrameSemanticTurnKind::HostLoad,
                RendererPageChildFrameTaskTarget::HostLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ClassicScriptSourceLoad,
                RendererPageChildFrameTaskTarget::ClassicScriptSourceLoad(_)
            ) | (
                ChildFrameSemanticTurnKind::ParserModuleRootStart,
                RendererPageChildFrameTaskTarget::ParserModuleRootStart(_)
            )
        )
    }

    #[cfg(test)]
    pub(super) fn page_resource_completion_queue(
        &self,
    ) -> crate::page_task_queue::RendererPageResourceCompletionTestSource {
        self.page_task_executor_sources_for_test()
            .resource_completion()
    }

    #[cfg(test)]
    pub(in crate::runtime) async fn wait_for_page_resource_completion_for_test(&mut self) -> bool {
        let source = self.page_resource_completion_queue();
        loop {
            if source.has_ready_completion() {
                return true;
            }
            self.page_task_queue.wait_for_page_runtime_wake().await;
        }
    }

    pub(super) async fn wait_for_initial_page_runtime_wake(&mut self) -> bool {
        if !self.has_page_resource_completion_route() {
            return std::future::pending().await;
        }
        self.page_task_queue.wait_for_page_runtime_wake().await;
        true
    }

    pub(super) fn page_task_producer_routes_match(
        &self,
        sources: &crate::page_task_queue::RendererPageOwnedTaskSources,
    ) -> bool {
        self.page_task_queue
            .page_task_producer_routes_match(sources)
    }

    fn commit_main_window_proxy_navigation(&mut self) -> Result<()> {
        let Some(mut vm) = self.vm.take() else {
            return Err(anyhow::anyhow!(
                "main navigation attempted to commit an already retired PageVm"
            ));
        };
        vm.detach_default_inspector_context_for_context_teardown();
        vm.close_page_context_resources_for_context_teardown();
        match vm.detach_main_window_proxy_for_navigation_commit(self.page_id.as_u64()) {
            Ok(()) => {
                tracing::debug!(
                    page_id = self.page_id.as_u64(),
                    "committed main navigation before replacement realm bootstrap"
                );
                drop(vm);
                Ok(())
            }
            Err(error) => {
                self.vm = Some(vm);
                Err(error)
            }
        }
    }

    // Fresh PageVm bootstrap is a special V8-entry boundary.
    //
    // The direct V8 call does not happen here; it happens further down in:
    // - runtime owner / navigation rebuild bootstrap call sites
    // - PageVm::new(...)
    // - ScriptVmDefaultWorldBootstrap standalone test bootstrap / explicit document-isolate bootstrap
    // - ScriptVmContextBootstrap::new_main_default()
    // - v8::Context::new(...)
    //
    // But this seam is where we still know two critical things at once:
    // 1. a fresh bootstrap V8 entry is about to happen
    // 2. which JsLocalExecutor / owner lane must host that entry
    //
    // That makes this the correct layer to enforce the "fresh local task"
    // policy. Pushing the policy down into script_vm would make the V8/bootstrap
    // layer depend on runtime scheduling decisions it should not know about.
    //
    // We also require an already-boxed future here. Bootstrap/navigation paths
    // can recurse back into PageVm construction, so keeping the future behind a
    // Pin<Box<_>> prevents recursive async-future sizing from leaking back into
    // callers. Keep this as the local explanation instead of relying on deleted
    // investigation notes.
    pub(in crate::runtime) async fn run_bootstrap_future_on_fresh_local_task<R>(
        local_executor: JsLocalExecutor,
        closed_message: &'static str,
        future: std::pin::Pin<Box<dyn std::future::Future<Output = Result<R>> + 'static>>,
    ) -> Result<R>
    where
        R: 'static,
    {
        run_named_owner_local_task(local_executor, closed_message, future).await
    }

    fn ensure_named_world_ready_for_document_start_script(
        &mut self,
        world_name: &str,
    ) -> Result<i64> {
        let execution_context_id = self.vm_mut().ensure_isolated_world(world_name, false)?;
        let matching_bindings = self
            .runtime_bindings
            .iter()
            .filter(|binding| binding.execution_context_name.as_deref() == Some(world_name))
            .cloned()
            .collect::<Vec<_>>();
        for binding in &matching_bindings {
            self.vm_mut().install_runtime_binding(
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        Ok(execution_context_id)
    }

    pub(super) fn install_stored_runtime_bindings_on_named_owner_lane(&mut self) -> Result<()> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "runtime binding replay must execute on the matching named owner lane"
        );
        let runtime_bindings = self.runtime_bindings.clone();
        for binding in runtime_bindings {
            self.vm_mut().install_runtime_binding(
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        Ok(())
    }

    pub(super) fn install_stored_runtime_isolated_worlds_on_named_owner_lane(
        &mut self,
    ) -> Result<()> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "isolated world restore must execute on the matching named owner lane"
        );
        let worlds = self.runtime_isolated_worlds.clone();
        for world in worlds {
            self.create_isolated_world(&world.name, world.grant_universal_access)?;
        }
        Ok(())
    }

    pub(super) fn restore_runtime_inspector_sessions_on_named_owner_lane(
        &mut self,
        restores: &[RendererInspectorSessionRestoreSnapshot],
    ) -> Result<()> {
        if restores.is_empty() {
            return Ok(());
        }
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "runtime inspector session restore must execute on the matching named owner lane"
        );
        for restore in restores {
            self.vm_mut().initialize_inspector_session_after_attach(
                restore.inspector_session_id.as_deref(),
                &restore.protocol_configuration,
                &restore.v8_attach,
            )?;
        }
        Ok(())
    }

    fn runtime_inspector_session_restore_snapshots(
        &self,
    ) -> Vec<RendererInspectorSessionRestoreSnapshot> {
        let mut snapshots = self
            .runtime_inspector_protocol_configurations
            .iter()
            .map(|(session_key, protocol_configuration)| {
                (
                    session_key.clone(),
                    RendererInspectorSessionRestoreSnapshot {
                        inspector_session_id: session_key.wire_session_id().map(str::to_owned),
                        protocol_configuration: protocol_configuration.clone(),
                        ..RendererInspectorSessionRestoreSnapshot::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (session_key, state) in self.vm().inspector_v8_session_states() {
            snapshots
                .entry(session_key.clone())
                .or_insert_with(|| RendererInspectorSessionRestoreSnapshot {
                    inspector_session_id: session_key.wire_session_id().map(str::to_owned),
                    ..RendererInspectorSessionRestoreSnapshot::default()
                })
                .v8_attach = V8InspectorSessionAttach::Reattach(state);
        }
        snapshots.into_values().collect()
    }

    pub(super) fn run_document_start_scripts_on_named_owner_lane<F>(
        &mut self,
        document_start_scripts: &[DocumentStartScript],
        mut after_each_script: F,
    ) -> Result<bool>
    where
        F: FnMut(&PageVm),
    {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "document-start scripts must execute on the matching named owner lane"
        );
        if self.script_execution_disabled() {
            return Ok(false);
        }
        for script in document_start_scripts {
            let script_started = Instant::now();
            match script.world_name.as_deref() {
                Some(world_name) => {
                    let execution_context_id =
                        self.ensure_named_world_ready_for_document_start_script(world_name)?;
                    self.vm_mut()
                        .exec_in_execution_context(execution_context_id, &script.source)?;
                }
                None => self.vm_mut().exec_runtime_turn(&script.source, None)?,
            }
            tracing::debug!(
                phase = "document start script",
                elapsed_ms = script_started.elapsed().as_millis(),
                "document start script executed"
            );
            after_each_script(self);
            if self.vm().has_pending_location_navigation() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    #[cfg(test)]
    pub(super) async fn follow_pending_location_navigation_one_turn_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        self.follow_pending_location_navigation_one_turn_at_boundary_async(
            pending_document_lifecycle_turn,
            stage,
            crate::runtime::page::FollowedLocationNavigationBootstrapBoundary::ContinuePhaseOne,
        )
        .await
    }

    pub(super) async fn prepare_pending_location_navigation_document_commit_one_turn_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        self.follow_pending_location_navigation_one_turn_at_boundary_async(
            pending_document_lifecycle_turn,
            stage,
            crate::runtime::page::FollowedLocationNavigationBootstrapBoundary::DocumentCommit,
        )
        .await
    }

    async fn follow_pending_location_navigation_one_turn_at_boundary_async(
        &mut self,
        pending_document_lifecycle_turn: &mut Option<PendingDocumentLifecycleTurn>,
        stage: PageVmInitStage,
        bootstrap_boundary: crate::runtime::page::FollowedLocationNavigationBootstrapBoundary,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let Some(pending) = self.vm_mut().take_pending_location_navigation_with_seed() else {
            return Ok(PageVmFollowNavigationTurnOutcome::Completed);
        };
        let initiator_url = self.vm().document_runtime.document_url().clone();
        let navigation_handoff = pending.handoff;
        let url = pending.url.clone();
        let request_method = pending.request_method.clone();
        let request_body = pending.request_body.clone();
        let request_headers = pending.request_headers.clone();
        let browser_navigation_kind = pending.browser_navigation_kind;
        let reserved_service_worker_client_id = pending
            .reserved_service_worker_client
            .map(|reserved| reserved.release());
        let service_worker_client_navigate = pending.service_worker_client_navigate;
        tracing::debug!(stage = ?stage, %url, "following pending location navigation asynchronously");

        if url.scheme() == "javascript" {
            if let Some(client_id) = reserved_service_worker_client_id {
                self.vm_mut()
                    .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
            }
            let replacement_lifecycle_snapshot =
                self.document_replacement_lifecycle_action_snapshot();
            let source_document = self.document_lifecycle.identity();
            let outcome =
                self.follow_taken_javascript_location_navigation(initiator_url, url, stage);
            let reconciliation = self
                .reconcile_javascript_navigation_lifecycle_after_owner_action(
                    replacement_lifecycle_snapshot,
                    pending_document_lifecycle_turn,
                    source_document,
                )
                .await;
            let outcome = match (outcome, reconciliation) {
                (Ok(PageVmFollowNavigationTurnOutcome::Completed), Ok(reconciliation)) => {
                    Ok(reconciliation.into_follow_outcome_after_completed_javascript_url())
                }
                (Ok(outcome), Ok(_)) => Ok(outcome),
                (Err(navigation_error), Ok(_)) => Err(navigation_error),
                (Ok(_), Err(reconciliation_error)) => Err(reconciliation_error),
                (Err(navigation_error), Err(reconciliation_error)) => Err(anyhow!(
                    "javascript: navigation failed ({navigation_error:#}) and its Document replacement lifecycle reconciliation also failed ({reconciliation_error:#})"
                )),
            };
            if let Some(continuation) = service_worker_client_navigate {
                match &outcome {
                    Ok(PageVmFollowNavigationTurnOutcome::Download(_)) => self
                        .vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            "Cannot navigate to URL.".to_owned(),
                        ),
                    Ok(PageVmFollowNavigationTurnOutcome::Completed)
                    | Ok(PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. })
                    | Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. }) => self
                        .vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation),
                    Ok(PageVmFollowNavigationTurnOutcome::PendingPhaseOne(_)) => {
                        unreachable!(
                            "javascript: location navigation cannot park in asynchronous phase-one creation"
                        )
                    }
                    Err(error) => self
                        .vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            format!("Cannot navigate to URL: {error}"),
                        ),
                }
            }
            return outcome;
        }

        let loaded = match load_followed_location_navigation(
            &self.request_client,
            initiator_url.clone(),
            url,
            request_method,
            request_body,
            request_headers,
            browser_navigation_kind,
        )
        .await
        {
            Ok(loaded) => loaded,
            Err(error) => {
                self.reject_failed_followed_location_navigation(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                    &error,
                );
                return Err(error);
            }
        };
        let loaded = match loaded {
            LoadedFollowedLocationNavigation::NoDocument => {
                self.abort_followed_navigation_without_document(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                );
                return Ok(PageVmFollowNavigationTurnOutcome::Completed);
            }
            LoadedFollowedLocationNavigation::Download(download) => {
                self.abort_followed_navigation_without_document(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                );
                return Ok(PageVmFollowNavigationTurnOutcome::Download(download));
            }
            loaded @ (LoadedFollowedLocationNavigation::StreamingDocument { .. }
            | LoadedFollowedLocationNavigation::ExternalDocument { .. }) => loaded,
        };

        let termination = self.document_lifecycle.request_termination(
            self.document_lifecycle.identity(),
            RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
        );
        debug_assert!(
            matches!(
                termination,
                RendererDocumentLifecycleTransition::Recorded(_)
                    | RendererDocumentLifecycleTransition::Deferred
                    | RendererDocumentLifecycleTransition::Duplicate
            ),
            "cross-document navigation should terminate the active renderer document: {termination:?}"
        );
        *pending_document_lifecycle_turn = None;

        let mut outcome = match self
            .bootstrap_followed_location_navigation(
                loaded,
                pending.entry_seed,
                reserved_service_worker_client_id,
                stage,
                bootstrap_boundary,
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                self.reject_failed_followed_location_navigation(
                    &initiator_url,
                    reserved_service_worker_client_id,
                    service_worker_client_navigate,
                    &error,
                );
                return Err(error);
            }
        };
        if matches!(
            bootstrap_boundary,
            crate::runtime::page::FollowedLocationNavigationBootstrapBoundary::DocumentCommit
        ) {
            mark_followed_navigation_document_commit(&mut outcome, navigation_handoff)?;
        }
        Ok(match outcome {
            PageVmFollowedNavigationBuildOutcome::ContinuePostParseLifecycle {
                page_vm,
                page_tasks,
                stage,
                started,
            } => {
                *self = page_vm;
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation);
                }
                let lifecycle = self
                    .begin_post_parse_lifecycle_on_named_owner_lane(
                        pending_document_lifecycle_turn,
                        page_tasks,
                        stage,
                        started,
                    )
                    .await?;
                PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                    target_stage: stage,
                    outcome: lifecycle,
                }
            }
            PageVmFollowedNavigationBuildOutcome::Download(download) => {
                if let Some(client_id) = reserved_service_worker_client_id {
                    self.vm_mut()
                        .unregister_reserved_service_worker_client_after_navigation_abort(
                            client_id,
                        );
                }
                self.vm_mut()
                    .restore_top_level_location_runtime_state(&initiator_url);
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .reject_pending_service_worker_client_navigate_after_follow(
                            continuation,
                            "Cannot navigate to URL.".to_owned(),
                        );
                }
                PageVmFollowNavigationTurnOutcome::Download(download)
            }
            PageVmFollowedNavigationBuildOutcome::PendingPhaseOne(mut pending) => {
                pending.metadata.service_worker_client_navigate = service_worker_client_navigate;
                pending.metadata.abort_reserved_service_worker_client_id =
                    reserved_service_worker_client_id;
                pending.metadata.abort_navigation_initiator_url = Some(initiator_url);
                PageVmFollowNavigationTurnOutcome::PendingPhaseOne(pending)
            }
            PageVmFollowedNavigationBuildOutcome::TriggeredNavigation { page_vm, stage } => {
                *self = page_vm;
                if let Some(continuation) = service_worker_client_navigate {
                    self.vm_mut()
                        .complete_pending_service_worker_client_navigate_after_follow(continuation);
                }
                PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage }
            }
        })
    }

    fn prepare_replacement_document_commit(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> Result<()> {
        ensure!(
            self.replacement_document_commit_handoff.is_none(),
            "replacement PageVm already owns a pending Document commit identity"
        );
        self.replacement_document_commit_handoff = Some(handoff);
        Ok(())
    }

    fn abort_followed_navigation_without_document(
        &mut self,
        initiator_url: &Url,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        service_worker_client_navigate: Option<
            crate::types::ServiceWorkerClientNavigateContinuation,
        >,
    ) {
        if let Some(client_id) = reserved_service_worker_client_id {
            self.vm_mut()
                .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
        }
        self.vm_mut()
            .restore_top_level_location_runtime_state(initiator_url);
        if let Some(continuation) = service_worker_client_navigate {
            self.vm_mut()
                .reject_pending_service_worker_client_navigate_after_follow(
                    continuation,
                    "Cannot navigate to URL.".to_owned(),
                );
        }
    }

    fn reject_failed_followed_location_navigation(
        &mut self,
        initiator_url: &Url,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        service_worker_client_navigate: Option<
            crate::types::ServiceWorkerClientNavigateContinuation,
        >,
        error: &anyhow::Error,
    ) {
        let navigation_committed = !self.has_live_script_vm();
        if !navigation_committed {
            self.vm_mut()
                .restore_top_level_location_runtime_state(initiator_url);
        }
        if let Some(client_id) = reserved_service_worker_client_id {
            if navigation_committed {
                self.runtime_hooks
                    .browser_context_runtime
                    .unregister_service_worker_client(client_id);
            } else {
                self.vm_mut()
                    .unregister_reserved_service_worker_client_after_navigation_abort(client_id);
            }
        }
        let Some(continuation) = service_worker_client_navigate else {
            return;
        };
        let message = format!("Cannot navigate to URL: {error}");
        if navigation_committed {
            self.runtime_hooks
                .browser_context_runtime
                .service_worker_runtime()
                .enqueue_client_navigate_completed(
                    crate::types::ServiceWorkerClientNavigateCompletion {
                        request_id: continuation.request_id,
                        source_version_id: continuation.source_version_id,
                        source_run: continuation.source_run,
                        result: Err(
                            crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                                message,
                            ),
                        ),
                    },
                );
        } else {
            self.vm_mut()
                .reject_pending_service_worker_client_navigate_after_follow(continuation, message);
        }
    }

    pub(super) fn replacement_document_commit_handoff(
        &self,
    ) -> Option<crate::page_task_queue::RendererTopLevelNavigationHandoff> {
        self.replacement_document_commit_handoff
    }

    pub(super) fn settle_replacement_document_commit(
        &mut self,
        handoff: crate::page_task_queue::RendererTopLevelNavigationHandoff,
    ) -> Result<()> {
        ensure!(
            self.replacement_document_commit_handoff == Some(handoff),
            "replacement Document commit identity changed before publication"
        );
        self.replacement_document_commit_handoff = None;
        Ok(())
    }

    fn follow_pending_javascript_location_navigation_if_present(
        &mut self,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let Some(pending) = self.vm_mut().take_pending_location_navigation_with_seed() else {
            return Ok(PageVmFollowNavigationTurnOutcome::Completed);
        };
        let initiator_url = self.vm().document_runtime.document_url().clone();
        let service_worker_client_navigate = pending.service_worker_client_navigate;
        let outcome =
            self.follow_taken_javascript_location_navigation(initiator_url, pending.url, stage);
        if let Some(continuation) = service_worker_client_navigate {
            match &outcome {
                Ok(PageVmFollowNavigationTurnOutcome::Download(_)) => self
                    .vm_mut()
                    .reject_pending_service_worker_client_navigate_after_follow(
                        continuation,
                        "Cannot navigate to URL.".to_owned(),
                    ),
                Ok(PageVmFollowNavigationTurnOutcome::Completed)
                | Ok(PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. })
                | Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. }) => self
                    .vm_mut()
                    .complete_pending_service_worker_client_navigate_after_follow(continuation),
                Ok(PageVmFollowNavigationTurnOutcome::PendingPhaseOne(_)) => {
                    unreachable!(
                        "javascript: location navigation cannot park in asynchronous phase-one creation"
                    )
                }
                Err(error) => self
                    .vm_mut()
                    .reject_pending_service_worker_client_navigate_after_follow(
                        continuation,
                        format!("Cannot navigate to URL: {error}"),
                    ),
            }
        }
        outcome
    }

    fn follow_taken_javascript_location_navigation(
        &mut self,
        initiator_url: Url,
        url: Url,
        stage: PageVmInitStage,
    ) -> Result<PageVmFollowNavigationTurnOutcome> {
        let source = javascript_location_navigation_source(&url);
        tracing::debug!(
            stage = ?stage,
            %url,
            source_len = source.len(),
            "executing pending javascript location navigation"
        );
        self.vm_mut()
            .restore_top_level_location_runtime_state(&initiator_url);
        let replacement_html = self.vm_mut().eval_javascript_url_runtime_turn(&source)?;
        if let Some(replacement_html) = replacement_html {
            self.document_lifecycle.set_next_document_open_start_reason(
                RendererLifecycleStartReason::JavascriptDocumentReplacement,
            );
            let replacement_html = serde_json::to_string(&replacement_html)?;
            let execution = self.vm_mut().exec_runtime_turn(
                &format!("document.open(); document.write({replacement_html}); document.close();"),
                Some(&url),
            );
            self.document_lifecycle.set_next_document_open_start_reason(
                RendererLifecycleStartReason::ExplicitDocumentOpen,
            );
            execution?;
        }
        if self.vm().has_pending_location_navigation() {
            Ok(PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage })
        } else {
            Ok(PageVmFollowNavigationTurnOutcome::Completed)
        }
    }

    async fn bootstrap_followed_location_navigation(
        &mut self,
        loaded: LoadedFollowedLocationNavigation,
        navigation_bootstrap_entry: Option<crate::native_bridge::NavigationHistoryEntrySeed>,
        reserved_service_worker_client_id: Option<
            crate::service_worker_runtime::ServiceWorkerClientId,
        >,
        stage: PageVmInitStage,
        boundary: crate::runtime::page::FollowedLocationNavigationBootstrapBoundary,
    ) -> Result<PageVmFollowedNavigationBuildOutcome> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "async followed location-navigation rebuild must execute on the matching named owner lane"
        );
        debug_assert!(matches!(
            &loaded,
            LoadedFollowedLocationNavigation::StreamingDocument { .. }
                | LoadedFollowedLocationNavigation::ExternalDocument { .. }
        ));
        let env = PageVmEnvConfig {
            main_document_commit: None,
            web_storage: self.vm().web_storage_handles(),
            document_start_scripts: self.document_start_scripts.clone(),
            runtime_bindings: self.runtime_bindings.clone(),
            runtime_inspector_session_restore_snapshots: self
                .runtime_inspector_session_restore_snapshots(),
            runtime_isolated_worlds: self.runtime_isolated_worlds.clone(),
            permission_overrides: self.permission_overrides.clone(),
            extra_http_headers: self.extra_http_headers.clone(),
            document_content_security_policies: self.vm().document_content_security_policies(),
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
            locale_override: self.locale_override.clone(),
            timezone_override: self.timezone_override.clone(),
            script_execution_disabled: self.script_execution_disabled(),
            bypass_content_security_policy: self.bypass_content_security_policy,
            cpu_throttling_rate: self.cpu_throttling_rate,
            emulated_media: self.emulated_media.clone(),
            idle_override: self.idle_override,
            viewport_surface: self.viewport_surface,
            network_offline: self.network_offline,
            blocked_url_patterns: self.blocked_url_patterns.clone(),
            indexed_db_manager: self.indexed_db_manager.clone(),
            storage_bucket_store: self.storage_bucket_store.clone(),
            fetch_subresource_interception_enabled: self.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: self
                .fetch_subresource_interception_resource_type,
            layout_policy: self.layout_policy,
            wpt_extensions_enabled: self.wpt_extensions_enabled,
            root_frame_id: self.vm().root_frame_id().map(str::to_owned),
            top_level_storage_key: None,
            navigation_bootstrap_entry: None,
            reserved_service_worker_client_id: None,
        };
        let runtime_hooks = self.runtime_hooks.clone().for_cross_document_commit();
        if runtime_hooks.has_renderer_page_script_environment() {
            self.commit_main_window_proxy_navigation()?;
        }
        bootstrap_committed_followed_location_navigation(
            self.page_id,
            self.local_executor.clone(),
            self.request_client.clone(),
            env,
            runtime_hooks,
            navigation_bootstrap_entry,
            reserved_service_worker_client_id,
            stage,
            loaded,
            boundary,
        )
        .await
    }

    pub(super) async fn drain_deferred_page_tasks_on_named_owner_local_task(
        &mut self,
    ) -> Result<()> {
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one deferred page-task local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                page_vm.vm_mut().drain_deferred_page_tasks_best_effort();
                Ok(())
            },
        )
        .await
    }

    pub(super) async fn perform_script_task_checkpoint_on_named_owner_local_task(
        &mut self,
        script_url: Option<Url>,
    ) -> Result<()> {
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one script-task checkpoint local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                page_vm
                    .vm_mut()
                    .perform_script_task_checkpoint(script_url.as_ref())?;
                page_vm.absorb_parser_no_execution_runs();
                Ok(())
            },
        )
        .await
    }

    pub(super) async fn construct_parser_custom_element_handoff_on_named_owner_local_task(
        &mut self,
        handoff: crate::parser::ParserCustomElementConstructionHandoff,
    ) -> Result<()> {
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one parser custom-element construction local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                page_vm
                    .vm_mut()
                    .construct_parser_custom_element_handoff(&handoff)
            },
        )
        .await
    }

    async fn execute_post_parse_page_owned_task_on_named_owner_lane(
        &mut self,
        loader: &ResourceRequestClient,
        work: PostParsePageOwnedWork,
    ) -> Result<parser_completion::SelectedPostParsePageOwnedCompletion> {
        let local_executor = self.local_executor.clone();
        let loader = loader.clone();
        let page_vm_ptr: *mut PageVm = self;
        run_named_owner_local_task(
            local_executor,
            "post-parse page-owned task local task channel closed",
            async move {
                // SAFETY: the caller awaits this local task before touching `page_vm`
                // again, and the task stays on the same render thread/local runtime.
                let page_vm = unsafe { &mut *page_vm_ptr };
                let execution =
                    execute_page_owned_work_on_script_execution_lane(&loader, page_vm, work)
                        .await?;
                let completion = match execution {
                    parser_continuation::PostParsePageOwnedExecution::Ordinary(run) => {
                        if let Some(run) = run {
                            page_vm.report.runs.push(run);
                            page_vm
                                .vm_mut()
                                .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
                        }
                        parser_completion::SelectedPostParsePageOwnedCompletion::Ordinary
                    }
                    parser_continuation::PostParsePageOwnedExecution::DocumentScript(execution) => {
                        let run = page_vm
                            .finish_main_page_owned_document_script_execution(execution)?;
                        page_vm.report.runs.push(run);
                        parser_completion::SelectedPostParsePageOwnedCompletion::Ordinary
                    }
                    parser_continuation::PostParsePageOwnedExecution::MainDocumentPostParse(
                        execution,
                    ) => {
                        parser_completion::SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(
                            execution,
                        )
                    }
                    parser_continuation::PostParsePageOwnedExecution::MainParserContinuation(
                        execution,
                    ) => {
                        let (run, completion) = execution.into_parts();
                        if let Some(run) = run {
                            page_vm.report.runs.push(run);
                        }
                        parser_completion::SelectedPostParsePageOwnedCompletion::MainParser(
                            completion,
                        )
                    }
                };
                Ok(completion)
            },
        )
        .await
    }

    #[cfg(test)]
    async fn execute_ordinary_post_parse_page_owned_task_on_named_owner_lane(
        &mut self,
        loader: &ResourceRequestClient,
        work: PostParsePageOwnedWork,
    ) -> Result<()> {
        match self
            .execute_post_parse_page_owned_task_on_named_owner_lane(loader, work)
            .await?
        {
            parser_completion::SelectedPostParsePageOwnedCompletion::Ordinary => Ok(()),
            parser_completion::SelectedPostParsePageOwnedCompletion::MainDocumentPostParse(
                execution,
            ) => self.finish_main_document_post_parse_execution(execution),
            parser_completion::SelectedPostParsePageOwnedCompletion::MainParser(_) => {
                anyhow::bail!(
                    "main-parser continuation escaped its lifecycle-aware completion boundary"
                )
            }
        }
    }

    fn record_notified_module_script_graph_failure(
        &mut self,
        mut script_continuation: ModuleScriptContinuation,
        error: crate::module_runtime::ModuleLoadError,
    ) -> Option<DynamicScriptOwnerId> {
        let message = error.message().to_owned();
        let failure_classification = PageOwnedScriptFailureClassification::from_module_load_error(
            &script_continuation.script,
            &error,
        );
        let completion_owner = script_continuation.completion_owner();
        let dynamic_script_owner_id = script_continuation.dynamic_script_owner_id();
        let load_delay_binding = script_continuation.take_main_document_load_delay_binding();
        let settlement_script = script_continuation.script.clone();
        let (outcome, selected_runtime_terminal) = match (
            completion_owner.is_runtime_owned(),
            dynamic_script_owner_id,
            failure_classification,
        ) {
            (
                true,
                Some(id),
                PageOwnedScriptFailureClassification::Typed {
                    dynamic_kind,
                    module_failure_policy,
                    error_constructor,
                },
            ) if dynamic_kind.is_deferrable_module() => {
                self.vm_mut()
                    .record_runtime_owned_module_failure_for_selected_action(
                        id,
                        &script_continuation.script,
                        message.clone(),
                        dynamic_kind,
                        module_failure_policy,
                        error_constructor,
                    );
                (
                    complete_prepared_script_execution_failure_report(
                        script_continuation.script,
                        message,
                    ),
                    Some(id),
                )
            }
            (_, _, failure_classification) => (
                complete_prepared_script_execution_failure(
                    self.vm_mut(),
                    script_continuation.script,
                    completion_owner,
                    dynamic_script_owner_id,
                    message,
                    failure_classification,
                ),
                None,
            ),
        };
        if let Some(binding) = load_delay_binding {
            let _ = self
                .vm_mut()
                .enqueue_main_document_script_load_delay_settlement_best_effort(
                    &settlement_script,
                    binding,
                );
        }
        let run = outcome.into_run();
        self.report.runs.push(run);
        selected_runtime_terminal
    }

    /// Applies one native-module owner action as a bounded transaction: first
    /// record every graph failure, then synchronously settle only exact error
    /// terminals from that same action which DynamicScriptOwner says are now
    /// runnable. Remaining ordered failures retain stable residence and get a
    /// normal continuation signal.
    fn record_notified_module_script_graph_failures(
        &mut self,
        failures: impl IntoIterator<
            Item = (
                ModuleScriptContinuation,
                crate::module_runtime::ModuleLoadError,
            ),
        >,
    ) -> (bool, Vec<crate::dynamic_script_owner::DynamicScriptOwnerId>) {
        let mut completed = false;
        let mut selected_runtime_terminals = Vec::new();
        for (script_continuation, error) in failures {
            completed = true;
            if let Some(id) =
                self.record_notified_module_script_graph_failure(script_continuation, error)
            {
                selected_runtime_terminals.push(id);
            }
        }
        (completed, selected_runtime_terminals)
    }

    fn complete_notified_module_script_graph_failures(
        &mut self,
        failures: impl IntoIterator<
            Item = (
                ModuleScriptContinuation,
                crate::module_runtime::ModuleLoadError,
            ),
        >,
    ) -> bool {
        let (completed, selected_runtime_terminals) =
            self.record_notified_module_script_graph_failures(failures);
        if !selected_runtime_terminals.is_empty() {
            let _ = self
                .vm_mut()
                .settle_runtime_owned_module_failures_for_selected_action(
                    &selected_runtime_terminals,
                );
            self.vm_mut()
                .enqueue_immediate_runtime_script_work_if_needed();
        }
        completed
    }

    fn handle_immediate_native_module_owner_actions(&mut self, actions: NativeModuleOwnerActions) {
        let _ = self
            .complete_notified_module_script_graph_failures(actions.into_runtime_module_failures());
    }

    /// Body-only settlement for a graph terminal already selected from the
    /// ResourceCompletion source. Event dispatch and load-gate release are
    /// returned to that task's central completion coordinator.
    fn handle_immediate_native_module_owner_actions_body(
        &mut self,
        actions: NativeModuleOwnerActions,
    ) -> crate::script_vm::RuntimeOwnedModuleFailureBodySettlement {
        let (_, selected_runtime_terminals) = self
            .record_notified_module_script_graph_failures(actions.into_runtime_module_failures());
        if selected_runtime_terminals.is_empty() {
            return crate::script_vm::RuntimeOwnedModuleFailureBodySettlement::none();
        }
        let settlement = self
            .vm_mut()
            .settle_runtime_owned_module_failures_for_selected_action_body(
                &selected_runtime_terminals,
            );
        self.vm_mut()
            .enqueue_immediate_runtime_script_work_if_needed();
        settlement
    }

    fn handle_module_script_continuation_graph_advance(
        &mut self,
        advance: ModuleScriptContinuationGraphAdvance,
    ) -> Result<()> {
        let actions = self
            .vm_mut()
            .handle_module_script_graph_advance_for_owner(advance);
        self.handle_immediate_native_module_owner_actions(actions);
        Ok(())
    }

    fn take_ready_runtime_owned_module_script_continuation(
        &mut self,
    ) -> Option<RuntimeOwnedModuleScriptContinuation> {
        let actions = self
            .vm_mut()
            .drain_ready_runtime_owned_module_owner_actions()
            .ok()?;
        let (mut ready_scripts, mut ready_evaluations, runtime_failures) = actions.into_parts();
        let _ = self.complete_notified_module_script_graph_failures(runtime_failures);
        if let Some(script) = ready_scripts.pop() {
            return Some(RuntimeOwnedModuleScriptContinuation::Graph(script));
        }
        ready_evaluations
            .pop()
            .map(RuntimeOwnedModuleScriptContinuation::Evaluation)
    }

    fn reserve_module_script_evaluation_reaction_id(&mut self) -> u64 {
        let reaction_id = self.next_module_script_evaluation_reaction_id;
        self.next_module_script_evaluation_reaction_id = self
            .next_module_script_evaluation_reaction_id
            .wrapping_add(1);
        reaction_id
    }

    fn push_module_evaluation_continuation(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        self.vm_mut()
            .note_module_script_evaluation_suspended_for_owner(evaluation);
    }

    fn has_ready_runtime_owned_module_script_continuation_work(&mut self) -> bool {
        self.vm_mut().has_ready_runtime_owned_module_owner_actions()
    }

    #[cfg(test)]
    fn has_pending_parser_owned_module_script(&self) -> bool {
        self.vm().has_pending_parser_owned_module_script()
    }

    #[cfg(test)]
    fn has_pending_parser_owned_module_fetch(&self) -> bool {
        self.vm().has_pending_parser_owned_module_fetch()
    }

    #[cfg(test)]
    fn has_pending_runtime_owned_module_graph(&mut self) -> bool {
        self.vm_mut()
            .has_pending_runtime_owned_module_script_graph()
    }

    #[cfg(test)]
    fn has_pending_runtime_owned_module_evaluation(&mut self) -> bool {
        self.vm_mut()
            .has_pending_runtime_owned_module_script_evaluation()
    }

    #[cfg(test)]
    fn has_pending_module_fetch_for_target_stage(&mut self) -> bool {
        self.has_pending_parser_owned_module_fetch()
            || (!matches!(self.target_stage, PageVmInitStage::DomContentLoaded)
                && (self.has_pending_runtime_owned_module_graph()
                    || self.vm().has_inflight_dynamic_module_fetch()))
    }

    #[cfg(test)]
    fn has_pending_module_script_for_target_stage(&mut self) -> bool {
        // Parser-owned defer/module-defer is released by the post-parse
        // PendingScript marker. Its lifecycle token is a DCL gate, not a signal
        // that a command wait owns the next transition. Concrete module tasks
        // remain authoritative.
        self.vm().document_runtime.dom_content_loaded_dispatched()
            && !matches!(self.target_stage, PageVmInitStage::DomContentLoaded)
            && (self.has_pending_runtime_owned_module_graph()
                || self.has_pending_runtime_owned_module_evaluation())
    }

    async fn run_ready_runtime_owned_module_script_continuation(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<bool> {
        let Some(continuation) = self.take_ready_runtime_owned_module_script_continuation() else {
            return Ok(false);
        };

        match continuation {
            RuntimeOwnedModuleScriptContinuation::Graph(script_continuation) => {
                let _ = self
                    .finish_ready_completed_module_script(
                        loader,
                        script_continuation,
                        ParserModuleTerminalDisposition::CompleteWithinModuleSettlement,
                    )
                    .await?;
            }
            RuntimeOwnedModuleScriptContinuation::Evaluation(evaluation) => {
                let _ = self
                    .run_ready_module_evaluation_completion(
                        loader,
                        Some(evaluation),
                        ParserModuleTerminalDisposition::CompleteWithinModuleSettlement,
                    )
                    .await?;
            }
        }
        Ok(true)
    }

    fn start_module_script_graph_evaluation(
        &mut self,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<ModuleScriptEvaluationStart, ModuleScriptEvaluationStartFailure> {
        // Duplicate parser-owned module scripts share the evaluated module map
        // entry, but each script still needs its own load/finalize path below.
        if self
            .vm()
            .document_runtime
            .native_module_entry_state(graph.root_entry)
            == crate::module_runtime::ModuleMapEntryState::Evaluated
        {
            return Ok(ModuleScriptEvaluationStart::Completed(
                crate::script_vm::PreparedScriptBodyActivity::NotEntered,
            ));
        }
        self.vm_mut()
            .instantiate_native_module_graph(graph)
            .map_err(|error| {
                ModuleScriptEvaluationStartFailure::new(
                    wrap_native_esm_module_load_error("NativeEsmInstantiateFailed", error),
                    crate::script_vm::PreparedScriptBodyActivity::NotEntered,
                )
            })?;
        if let Some(promise) = self
            .vm_mut()
            .evaluate_native_module_graph(graph.root_entry)
            .map_err(|error| {
                ModuleScriptEvaluationStartFailure::new(
                    wrap_native_esm_module_load_error("NativeEsmEvaluateFailed", error),
                    crate::script_vm::PreparedScriptBodyActivity::Entered,
                )
            })?
        {
            let reaction_id = self.reserve_module_script_evaluation_reaction_id();
            self.vm_mut()
                .attach_native_module_script_evaluation_reactions(reaction_id, promise)
                .map_err(|error| {
                    ModuleScriptEvaluationStartFailure::new(
                        wrap_native_esm_module_load_error("NativeEsmEvaluateFailed", error),
                        crate::script_vm::PreparedScriptBodyActivity::Entered,
                    )
                })?;
            return Ok(ModuleScriptEvaluationStart::Pending {
                root_entry: graph.root_entry,
                reaction_id,
            });
        }
        Ok(ModuleScriptEvaluationStart::Completed(
            crate::script_vm::PreparedScriptBodyActivity::Entered,
        ))
    }

    #[cfg(test)]
    fn has_ready_page_websocket_task_for_test(&mut self) -> bool {
        let current_document = self.document_lifecycle.identity().document;
        self.page_task_executor_sources_for_test()
            .next_runnable_websocket_at_for_executor_test(current_document)
            .is_some()
    }

    #[cfg(test)]
    async fn wait_for_page_websocket_task_for_test(&mut self) -> bool {
        loop {
            if self.has_ready_page_websocket_task_for_test() {
                return true;
            }
            self.page_task_queue.wait_for_page_runtime_wake().await;
        }
    }

    /// Execute one exact WebSocket task through the production selected-task
    /// dispatcher and return only the output it produced.
    ///
    /// Browser-context producer lanes are deliberately outside this helper.
    /// Direct fixtures that create SharedWorker or ServiceWorker work must
    /// admit those lanes explicitly before selecting the resident WebSocket
    /// task.
    #[cfg(test)]
    pub(super) async fn run_exact_page_websocket_selected_task_for_test(
        &mut self,
    ) -> Result<Option<RendererOwnerResourceActivitySource>> {
        let current_document = self.document_lifecycle.identity().document;
        let sources = self.page_task_executor_sources_for_test();
        if let Some(task) = sources.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::WebSocket {
                    owner,
                    readiness,
                    ..
                } if matches!(
                    readiness,
                    crate::page_task_queue::RendererPageWebSocketReadiness::Ready
                ) || owner.root_document() != current_document
            )
        }) {
            let crate::page_task_queue::RendererPageSchedulerTask::WebSocket(task) = task else {
                unreachable!("WebSocket descriptor must dequeue its own typed source")
            };
            let outcome = self.apply_selected_page_websocket_turn(task)?;
            let action = outcome.action;
            let produced_output = !matches!(
                action.target_effect,
                crate::page_task_queue::PageWebSocketTargetEffect::ParkedForReadableBackpressure
            );
            let loader = self.request_client.clone();
            self.finish_selected_page_task_completion(action.into_page_task_completion(), &loader)
                .await?;
            return Ok(produced_output.then_some(RendererOwnerResourceActivitySource::WebSocket));
        }
        Ok(None)
    }

    #[cfg(test)]
    fn should_wait_for_runtime_owned_work(
        &self,
        include_post_domcontentloaded_runtime_work: bool,
    ) -> bool {
        include_post_domcontentloaded_runtime_work
            || !matches!(self.target_stage, PageVmInitStage::DomContentLoaded)
    }

    /// Execute one due timer through the production selected-task dispatcher.
    ///
    /// This helper belongs only to timer-focused direct PageVm fixtures. It
    /// does not compare timers with another source or participate in lifecycle
    /// task selection.
    #[cfg(test)]
    async fn run_one_due_timer_selected_task_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<bool> {
        let Some(crate::page_task_queue::RendererPageReadyDescriptor::Timer { deadline }) =
            self.due_page_timer_ready_descriptor()
        else {
            return Ok(false);
        };
        Box::pin(self.apply_selected_page_scheduler_task(
            crate::page_task_queue::RendererPageSchedulerTask::Timer { deadline },
            loader,
        ))
        .await?;
        Ok(true)
    }

    /// Advance one child semantic action from the production stable sources in
    /// direct PageVm fixtures that do not own a full RendererOwner loop.
    #[cfg(test)]
    pub(in crate::runtime) async fn run_next_child_frame_task_source_for_semantic_test(
        &mut self,
    ) -> Option<crate::frame_owner_model::ChildFrameSemanticTurnKind> {
        use crate::frame_owner_model::ChildFrameSemanticTurnKind;

        let loader = self.request_client.clone();

        if self
            .run_child_realm_materialization_body_for_test()
            .expect("child realm materialization prerequisite should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::RealmMaterialization);
        }
        if self
            .run_child_navigation_commit_body_for_test()
            .expect("typed child navigation-commit body should succeed")
            .is_some()
        {
            return Some(ChildFrameSemanticTurnKind::NavigationCommit);
        }
        if self
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentLifecycle,
                &loader,
            )
            .await
            .expect("typed child lifecycle executor turn should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::DocumentLifecycle);
        }
        if self
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildDocumentScriptReady,
                &loader,
            )
            .await
            .expect("typed child DocumentScriptReady executor turn should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::DocumentScriptReady);
        }
        if self
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildHostLoad,
                &loader,
            )
            .await
            .expect("typed child HostLoad selected task should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::HostLoad);
        }
        if self
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildParserModuleRootStart,
                &loader,
            )
            .await
            .expect("typed child parser module root selected task should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::ParserModuleRootStart);
        }
        if self
            .run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::ChildClassicScriptSourceLoad,
                &loader,
            )
            .await
            .expect("typed child classic source-load selected task should succeed")
        {
            return Some(ChildFrameSemanticTurnKind::ClassicScriptSourceLoad);
        }
        None
    }

    pub(super) fn replay_pending_owner_wakes_after_attach(
        &mut self,
        has_page_task_source: bool,
    ) -> bool {
        let Some(owner_wake) = self.runtime_hooks.owner_wake.clone() else {
            return false;
        };
        let mut replayed = false;
        if has_page_task_source {
            owner_wake.signal_scheduler_continuation();
            tracing::debug!(
                page_id = self.page_id.as_u64(),
                "replayed durable Page-source wake after page attachment"
            );
            replayed = true;
        }
        replayed
    }

    pub(in crate::runtime) fn child_frame_lifecycle_work_is_complete(&self) -> bool {
        !self.vm().has_pending_location_navigation()
            && !self.vm().has_pending_child_frame_realm_materialization()
            && !self.vm().has_pending_child_document_lifecycle()
            && !self.vm().has_pending_lightweight_popup_resource_loads()
    }

    #[cfg(test)]
    pub(super) async fn wait_for_page_work_arrival_without_timeout(
        &mut self,
        include_post_domcontentloaded_runtime_work: bool,
    ) -> bool {
        self.wait_for_page_work_arrival_without_timeout_with_timer_policy(
            include_post_domcontentloaded_runtime_work,
            true,
        )
        .await
    }

    #[cfg(test)]
    async fn wait_for_lifecycle_blocking_page_work_arrival_without_timeout(
        &mut self,
        include_post_domcontentloaded_runtime_work: bool,
    ) -> bool {
        self.wait_for_page_work_arrival_without_timeout_with_timer_policy(
            include_post_domcontentloaded_runtime_work,
            include_post_domcontentloaded_runtime_work,
        )
        .await
    }

    #[cfg(test)]
    async fn wait_for_document_style_progress_without_timeout(&mut self) -> bool {
        let PageVm {
            vm,
            page_task_queue,
            ..
        } = self;
        vm.as_mut()
            .expect("PageVm must retain a live ScriptVm until drop")
            .document_runtime
            .wait_for_document_processing_wake_source(page_task_queue)
            .await
            .is_some()
    }

    #[cfg(test)]
    async fn wait_for_page_work_arrival_without_timeout_with_timer_policy(
        &mut self,
        include_post_domcontentloaded_runtime_work: bool,
        include_timer_work: bool,
    ) -> bool {
        let wait_for_runtime_owned_module_work =
            self.should_wait_for_runtime_owned_work(include_post_domcontentloaded_runtime_work);
        if include_timer_work && self.vm().has_ready_timeout() {
            return true;
        }
        if self.has_ready_page_networking_task() {
            return true;
        }
        if self.has_ready_parser_owned_document_script_action() {
            return true;
        }
        if wait_for_runtime_owned_module_work
            && self.has_ready_runtime_owned_module_script_continuation_work()
        {
            return true;
        }
        if self.vm().has_ready_dynamic_module_job() {
            return true;
        }
        if wait_for_runtime_owned_module_work && self.vm().has_pending_native_module_job() {
            return true;
        }
        if wait_for_runtime_owned_module_work
            && self.vm_mut().has_runnable_runtime_script_work_now()
        {
            return true;
        }
        if self.page_task_queue.complete_ready_source_loads() {
            return true;
        }
        if self.vm().has_ready_dynamic_module_job() {
            return true;
        }
        if self
            .vm()
            .document_runtime
            .has_pending_document_write_stylesheet_blocked_script()
        {
            return self
                .wait_for_document_style_progress_without_timeout()
                .await;
        }
        if self
            .vm()
            .document_runtime
            .has_pending_document_write_external_script_load()
            || self.vm().has_pending_child_document_lifecycle()
            || self.vm().has_pending_lightweight_popup_resource_loads()
            || self.has_pending_module_script_for_target_stage()
        {
            let timer_deadline = include_timer_work
                .then(|| self.vm().next_timeout_deadline())
                .flatten();
            let PageVm {
                page_task_queue, ..
            } = self;
            let pending_task_source_load = page_task_queue.pending_task_source_load();
            return tokio::select! {
                biased;
                arrived = page_task_queue.wait_for_injected_task_arrival_without_timeout() => arrived,
                arrived = wait_for_page_task_source_load_arrival(pending_task_source_load) => arrived,
                _ = wait_for_page_timer_deadline(timer_deadline) => true,
            };
        }
        let pending_task_source_load = self.page_task_queue.pending_task_source_load();
        let timer_deadline = include_timer_work
            .then(|| self.vm().next_timeout_deadline())
            .flatten();
        tokio::select! {
            biased;
            arrived = self.page_task_queue.wait_for_injected_task_arrival_without_timeout() => arrived,
            arrived = wait_for_page_task_source_load_arrival(pending_task_source_load) => arrived,
            _ = wait_for_page_timer_deadline(timer_deadline) => true,
        }
    }

    #[cfg(test)]
    async fn handle_post_parse_lifecycle_advance_on_named_owner_lane(
        &mut self,
        stage: PageVmInitStage,
        advance: PostParseLifecycleAdvance,
    ) -> Result<PostParseLifecycleLoopAdvance> {
        match advance {
            PostParseLifecycleAdvance::PageOwnedTask(mut task) => {
                let request_client = self.request_client.clone();
                self.execute_ordinary_post_parse_page_owned_task_on_named_owner_lane(
                    &request_client,
                    task.take_work_for_execution(),
                )
                .await?;
                if self.vm().has_pending_location_navigation() {
                    return Ok(PostParseLifecycleLoopAdvance::Complete(
                        PostParseLifecycleCompletionAction::TriggeredNavigation,
                    ));
                }
                Ok(PostParseLifecycleLoopAdvance::Continue(Box::new(Some(
                    *task,
                ))))
            }
            PostParseLifecycleAdvance::NeedsContinuation => {
                Ok(PostParseLifecycleLoopAdvance::Continue(Box::new(None)))
            }
            PostParseLifecycleAdvance::AwaitProgress => {
                let wait_for_runtime_owned_work =
                    !matches!(stage, PageVmInitStage::DomContentLoaded);
                self.admit_ready_parser_owned_document_script_action();
                let request_client = self.request_client.clone();
                if self
                    .run_one_oldest_ready_page_task_on_owner_lane_for_test(&request_client)
                    .await?
                {
                    return Ok(PostParseLifecycleLoopAdvance::Continue(Box::new(None)));
                }
                let _ = self
                    .wait_for_lifecycle_blocking_page_work_arrival_without_timeout(
                        wait_for_runtime_owned_work,
                    )
                    .await;
                Ok(PostParseLifecycleLoopAdvance::Continue(Box::new(None)))
            }
            PostParseLifecycleAdvance::Complete(completion_action) => {
                Ok(PostParseLifecycleLoopAdvance::Complete(completion_action))
            }
        }
    }

    #[cfg(test)]
    async fn drive_post_parse_lifecycle_loop_on_named_owner_lane(
        &mut self,
        stage: PageVmInitStage,
        lifecycle_driver: crate::script_vm::PostParseLifecycleDriver,
    ) -> Result<PostParseLifecycleCompletionAction> {
        let mut completed_task = None;
        let wait_for_runtime_owned_module_work =
            !matches!(stage, PageVmInitStage::DomContentLoaded);
        loop {
            let wait_for_child_browsing_context_work =
                !matches!(stage, PageVmInitStage::DomContentLoaded);
            let pending_module_script_for_target_stage =
                self.has_pending_module_script_for_target_stage();
            let pending_runtime_native_job = wait_for_runtime_owned_module_work
                && self.vm().document_runtime.dom_content_loaded_dispatched()
                && (self.vm_mut().has_pending_native_module_job()
                    || self.vm_mut().has_ready_native_module_owner_actions());
            if pending_module_script_for_target_stage || pending_runtime_native_job {
                let ms_to_next = self.vm().ms_to_next_timeout();
                match ms_to_next {
                    Some(0) => {
                        if self.has_pending_module_fetch_for_target_stage() {
                            let _ = self
                                .wait_for_lifecycle_blocking_page_work_arrival_without_timeout(
                                    wait_for_runtime_owned_module_work,
                                )
                                .await;
                        }
                    }
                    Some(ms_to_next) => {
                        let sleep_for = std::time::Duration::from_millis(ms_to_next);
                        tokio::select! {
                            arrived = self.wait_for_lifecycle_blocking_page_work_arrival_without_timeout(wait_for_runtime_owned_module_work) => {
                                if !arrived {
                                    tokio::time::sleep(sleep_for).await;
                                }
                            }
                            _ = tokio::time::sleep(sleep_for) => {}
                        }
                    }
                    None => {
                        let _ = self
                            .wait_for_lifecycle_blocking_page_work_arrival_without_timeout(
                                wait_for_runtime_owned_module_work,
                            )
                            .await;
                    }
                }
                continue;
            }
            let wait_for_load_event_delaying_subresources = wait_for_runtime_owned_module_work
                && self.vm().document_runtime.dom_content_loaded_dispatched()
                && self
                    .vm()
                    .has_pending_load_event_delaying_subresource_requests();
            if self
                .vm()
                .document_runtime
                .has_pending_document_write_stylesheet_blocked_script()
            {
                let _ = self
                    .wait_for_document_style_progress_without_timeout()
                    .await;
                continue;
            }
            if self
                .vm()
                .document_runtime
                .has_pending_document_write_external_script_load()
                || wait_for_load_event_delaying_subresources
                || (wait_for_child_browsing_context_work
                    && (self.vm().has_pending_child_document_lifecycle()
                        || self.vm().has_pending_lightweight_popup_resource_loads()))
            {
                let _ = self
                    .wait_for_lifecycle_blocking_page_work_arrival_without_timeout(
                        wait_for_runtime_owned_module_work,
                    )
                    .await;
                continue;
            }
            let request_client = self.request_client.clone();
            let advance = {
                let PageVm {
                    vm,
                    page_task_queue,
                    report,
                    ..
                } = self;
                vm.as_mut()
                    .expect("PageVm must retain a live ScriptVm until drop")
                    .advance_post_parse_lifecycle(
                        &request_client,
                        page_task_queue,
                        report,
                        lifecycle_driver,
                        completed_task.take(),
                    )
                    .await
                    .map_err(|message| anyhow::anyhow!(message))?
            };
            match self
                .handle_post_parse_lifecycle_advance_on_named_owner_lane(stage, advance)
                .await?
            {
                PostParseLifecycleLoopAdvance::Continue(task) => {
                    completed_task = *task;
                }
                PostParseLifecycleLoopAdvance::Complete(completion_action) => {
                    break Ok(completion_action);
                }
            }
        }
    }

    async fn finish_post_parse_lifecycle_completion_on_named_owner_lane(
        &mut self,
        stage: PageVmInitStage,
        started: Instant,
        completion_action: PostParseLifecycleCompletionAction,
    ) -> Result<()> {
        #[cfg(test)]
        let triggered_navigation = matches!(
            completion_action,
            PostParseLifecycleCompletionAction::TriggeredNavigation
        );
        #[cfg(not(test))]
        let triggered_navigation = false;
        if !triggered_navigation {
            let milestone = renderer_document_lifecycle_milestone_for_stage(stage);
            match self.document_lifecycle_wait_outcome(milestone) {
                RendererDocumentLifecycleWaitOutcome::Reached(_) => {}
                RendererDocumentLifecycleWaitOutcome::Interrupted(termination) => {
                    anyhow::bail!(
                        "renderer document lifecycle was interrupted before {milestone:?}: {:?}",
                        termination.reason
                    );
                }
                RendererDocumentLifecycleWaitOutcome::Pending => {
                    anyhow::bail!(
                        "post-parse lifecycle completed without renderer {milestone:?} milestone"
                    );
                }
            }
        }
        match completion_action {
            #[cfg(test)]
            PostParseLifecycleCompletionAction::TriggeredNavigation => {}
            PostParseLifecycleCompletionAction::ReturnAtStage(reached_stage) => {
                tracing::debug!(
                    phase = "stage done",
                    stage = ?stage,
                    reached_stage,
                    elapsed_ms = started.elapsed().as_millis(),
                    "returning at lifecycle stage"
                );
            }
            PostParseLifecycleCompletionAction::Finalize(finalization) => {
                tracing::debug!(
                    phase = "post-parse lifecycle queue",
                    defer_count = finalization.defer_count(),
                    async_count = finalization.async_count(),
                    detached_count = finalization.detached_count(),
                    elapsed_ms = finalization.elapsed_ms(),
                    "post-parse lifecycle queue drained"
                );
                tracing::debug!(
                    phase = "detached scripts",
                    elapsed_ms = started.elapsed().as_millis(),
                    "detached scripts recorded"
                );
                tracing::debug!(
                    phase = "completed",
                    stage = ?stage,
                    elapsed_ms = started.elapsed().as_millis(),
                    "page VM creation completed"
                );
            }
        }
        Ok(())
    }

    async fn execute_parser_owned_classic_script_body_on_current_lane(
        &mut self,
        loader: &ResourceRequestClient,
        execution_context: ParserOwnedClassicScriptExecutionContext,
        script: Box<PreparedScript>,
    ) -> parser_deferred_classic::MainParserDeferredClassicBodyExecution {
        let _ = self
            .vm_mut()
            .document_runtime
            .mark_script_already_started_by_node_id(script.node_id);
        let (run, script_element_event, evaluation, body_activity) = if self
            .script_execution_disabled()
        {
            (
                ScriptRun::skipped(
                    script.node_id,
                    script.kind,
                    script.mode,
                    script.source_kind,
                    script.url.clone(),
                    ScriptSkipReason::ScriptExecutionDisabled,
                ),
                None,
                crate::script_vm::ParserOwnedClassicScriptEvaluationSettlement::NotSettled,
                crate::script_vm::PreparedScriptBodyActivity::NotEntered,
            )
        } else {
            let execution_report = self
                .vm_mut()
                .run_parser_owned_classic_script_without_blocker_wait(
                    loader,
                    &script,
                    &execution_context,
                )
                .await;
            let (execution_result, script_element_event, evaluation, body_activity) =
                execution_report.into_parts();
            let run = match execution_result {
                Ok(()) => ScriptRun::executed(
                    script.node_id,
                    script.kind,
                    script.mode,
                    script.source_kind,
                    script.url.clone(),
                ),
                Err(error) => {
                    let error = error.into_message();
                    if script_element_event.is_none() {
                        if self
                            .vm_mut()
                            .parser_owned_inline_importmap_reports_window_error_immediately(&script)
                        {
                            self.vm_mut()
                                .report_parser_import_map_registration_failure_and_finish_algorithm_best_effort(
                                    &error,
                                    Some(script.url.as_str()),
                                );
                        } else {
                            self.vm_mut()
                                .enqueue_script_failure_lifecycle_work_best_effort(
                                    &script, &error, None, None,
                                );
                        }
                    }
                    ScriptRun::failed(
                        script.node_id,
                        script.kind,
                        script.mode,
                        script.source_kind,
                        script.url.clone(),
                        error,
                    )
                }
            };
            (run, script_element_event, evaluation, body_activity)
        };
        let navigation_triggered = self.vm_mut().has_pending_location_navigation();
        let completion = ParserOwnedClassicScriptCompletion::after_execution(
            execution_context,
            script_element_event,
            evaluation,
        );
        parser_deferred_classic::MainParserDeferredClassicBodyExecution::new(
            run,
            navigation_triggered,
            completion,
            body_activity,
        )
    }

    pub(super) async fn execute_parser_owned_classic_script_on_current_lane(
        &mut self,
        loader: &ResourceRequestClient,
        execution_context: ParserOwnedClassicScriptExecutionContext,
        script: Box<PreparedScript>,
    ) -> (ScriptRun, bool, ParserOwnedClassicScriptCompletion) {
        let execution = self
            .execute_parser_owned_classic_script_body_on_current_lane(
                loader,
                execution_context,
                script,
            )
            .await;
        self.vm_mut()
            .prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        let (run, navigation_triggered, completion, _) = execution.into_parts();
        (run, navigation_triggered, completion)
    }

    async fn execute_main_parser_deferred_classic_script_body_on_current_lane(
        &mut self,
        loader: &ResourceRequestClient,
        script: Box<PreparedScript>,
    ) -> parser_deferred_classic::MainParserDeferredClassicBodyExecution {
        self.execute_parser_owned_classic_script_body_on_current_lane(
            loader,
            ParserOwnedClassicScriptExecutionContext::Deferred,
            script,
        )
        .await
    }

    /// Executes a parse-time task directly on the runtime's live document.
    ///
    /// The runtime already owns the DomHost (single-DOM authority). Before
    /// executing any V8 code, we run custom-element upgrades for nodes the
    /// parser may have created since the last script execution.
    pub(super) async fn execute_parse_time_on_existing_live_document_on_named_owner_local_task(
        &mut self,
        execution: ParseTimeLiveExecution,
    ) -> Result<ParseTimeLiveExecutionOutcome> {
        let request_client = self.request_client.clone();
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "parse-time bridge execution must stay on the matching named owner lane"
        );
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one page-vm local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                // Parser-created iframe/frame hosts need to be visible to parse-time inline
                // scripts before the parser hands control back to runtime-wide resync points.
                page_vm.vm_mut().resync_child_browsing_contexts();

                // Run late-definition upgrades before parser-connected V8 execution.
                page_vm
                    .vm_mut()
                    .upgrade_late_defined_custom_elements_after_parser_checkpoint()?;

                let (
                    run,
                    navigation_triggered,
                    parser_owned_classic_script_completion,
                    main_parser_completion,
                ) = match execution {
                    ParseTimeLiveExecution::ParserOwnedClassicScript {
                        execution_context,
                        script,
                    } => {
                        let (run, navigation_triggered, completion) = page_vm
                            .execute_parser_owned_classic_script_on_current_lane(
                                &request_client,
                                execution_context,
                                script,
                            )
                            .await;
                        (
                            Some(run),
                            navigation_triggered,
                            Some(completion),
                            None,
                        )
                    }
                    ParseTimeLiveExecution::ConnectedStyleLoad { ready } => {
                        let binding = ready.load_event_binding();
                        let dispatched =
                            page_vm.vm_mut().dispatch_connected_style_load(ready);
                        page_vm.vm_mut().settle_connected_style_load(binding);
                        if dispatched {
                            page_vm
                                .finish_selected_page_callback_task(&request_client)
                                .await?;
                        } else {
                            page_vm.finish_selected_page_task_checkpoint()?;
                        }
                        let navigation_triggered =
                            page_vm.vm_mut().has_pending_location_navigation();
                        (None, navigation_triggered, None, None)
                    }
                    ParseTimeLiveExecution::PageOwnedDocumentScript {
                        lane,
                        script,
                        load_delay_binding,
                    } => {
                        let execution =
                            page_owned_document_script::MainPageOwnedDocumentScriptOwner::new(
                                page_vm,
                                &request_client,
                            )
                            .run_work(
                                crate::document_script_scheduler::PageOwnedDocumentScriptWork::parser_async_script(
                                    lane,
                                    *script,
                                    load_delay_binding,
                                ),
                            )
                            .await?;
                        let run = page_vm
                            .finish_main_page_owned_document_script_execution(execution)?;
                        let navigation_triggered =
                            page_vm.vm_mut().has_pending_location_navigation();
                        (Some(run), navigation_triggered, None, None)
                    }
                    ParseTimeLiveExecution::PageOwnedWork { work } => {
                        let execution = execute_page_owned_work_on_script_execution_lane(
                            &request_client,
                            page_vm,
                            *work,
                        )
                        .await?;
                        let (run, parser_completion) = match execution {
                            parser_continuation::PostParsePageOwnedExecution::Ordinary(
                                run,
                            ) => (run, None),
                            parser_continuation::PostParsePageOwnedExecution::DocumentScript(
                                execution,
                            ) => {
                                let run = page_vm
                                    .finish_main_page_owned_document_script_execution(execution)?;
                                (Some(run), None)
                            }
                            parser_continuation::PostParsePageOwnedExecution::MainDocumentPostParse(
                                execution,
                            ) => {
                                page_vm.finish_main_document_post_parse_execution(execution)?;
                                (None, None)
                            }
                            parser_continuation::PostParsePageOwnedExecution::MainParserContinuation(
                                execution,
                            ) => {
                                let (run, completion) = execution.into_parts();
                                (run, Some(completion))
                            }
                        };
                        let navigation_triggered =
                            page_vm.vm_mut().has_pending_location_navigation();
                        (
                            run,
                            navigation_triggered,
                            None,
                            parser_completion,
                        )
                    }
                };
                let navigation_triggered = if navigation_triggered
                    && page_vm.vm().pending_location_navigation_scheme_is("javascript")
                {
                    match page_vm.follow_pending_javascript_location_navigation_if_present(
                        PageVmInitStage::Load,
                    )? {
                        PageVmFollowNavigationTurnOutcome::Completed => false,
                        PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. } => {
                            unreachable!(
                                "javascript: location navigation cannot start post-parse lifecycle"
                            )
                        }
                        PageVmFollowNavigationTurnOutcome::Download(_) => true,
                        PageVmFollowNavigationTurnOutcome::PendingPhaseOne(_) => {
                            unreachable!(
                                "javascript: location navigation cannot park in asynchronous phase-one creation"
                            )
                        }
                        PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. } => true,
                    }
                } else {
                    navigation_triggered
                };
                if let Some(run) = run {
                    page_vm.report.runs.push(run);
                }
                // Parse-time scripts and page-owned tasks can retarget iframe/frame hosts inside
                // the live runtime after the pre-execution resync point. Refresh the child
                // browsing-context store again before control returns to later load/harness code
                // so late `contentDocument` reads see the final live document state.
                page_vm.vm_mut().resync_child_browsing_contexts();
                Ok(ParseTimeLiveExecutionOutcome::new(
                    navigation_triggered,
                    parser_owned_classic_script_completion,
                    main_parser_completion,
                ))
            },
        )
        .await
    }

    pub(super) async fn apply_parser_owned_classic_script_completion_on_named_owner_local_task(
        &mut self,
        expected_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        completion: ParserOwnedClassicScriptCompletion,
    ) -> Result<ParserOwnedClassicScriptCompletionApplication> {
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one parser-connected completion local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                let current_owner = page_vm.vm().current_main_document_task_owner();
                if current_owner != Some(expected_owner) {
                    tracing::debug!(
                        ?expected_owner,
                        ?current_owner,
                        event = ?completion.script_element_event().map(|task| task.kind),
                        evaluation = ?completion.evaluation(),
                        "dropping stale main parser-owned classic completion effects"
                    );
                    return Ok(ParserOwnedClassicScriptCompletionApplication::stale_owner());
                }
                tracing::debug!(
                    ?expected_owner,
                    event = ?completion.script_element_event().map(|task| task.kind),
                    evaluation = ?completion.evaluation(),
                    "applying main parser-owned classic completion effects"
                );
                let body = page_vm
                    .vm_mut()
                    .apply_main_parser_blocking_classic_completion_body(
                        expected_owner,
                        completion,
                    )
                    .map_err(anyhow::Error::msg)?;
                let (mut application, terminal_activity) = body.into_parts();
                if terminal_activity
                    == crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
                {
                    page_vm
                        .vm_mut()
                        .finish_main_parser_blocking_classic_terminal_checkpoint()?;
                }
                let current_owner = page_vm.vm().current_main_document_task_owner();
                if current_owner != Some(expected_owner) {
                    tracing::debug!(
                        ?expected_owner,
                        ?current_owner,
                        "skipping parser-blocking continuation admission after terminal replaced the document owner"
                    );
                    application.note_stale_owner();
                    return Ok(application);
                }
                page_vm
                    .vm_mut()
                    .start_pending_main_parser_deferred_scripts()
                    .map_err(anyhow::Error::msg)?;
                Ok(application)
            },
        )
        .await
    }

    pub(super) async fn run_pending_parser_post_step_runtime_work_on_named_owner_local_task(
        &mut self,
    ) -> Result<bool> {
        if !self
            .vm()
            .document_runtime
            .has_pending_parser_post_step_runtime_work()
        {
            return Ok(false);
        }
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "parser tree-mutation runtime work must stay on the matching named owner lane"
        );
        let local_executor = self.local_executor.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            local_executor,
            "phase-one parser post-step runtime work local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                page_vm.vm_mut().resync_child_browsing_contexts();
                page_vm
                    .vm_mut()
                    .run_pending_parser_post_step_runtime_work_in_default_context()?;
                page_vm.vm_mut().resync_child_browsing_contexts();
                let navigation_triggered = page_vm.vm_mut().has_pending_location_navigation();
                let navigation_triggered = if navigation_triggered
                    && page_vm
                        .vm()
                        .pending_location_navigation_scheme_is("javascript")
                {
                    match page_vm.follow_pending_javascript_location_navigation_if_present(
                        PageVmInitStage::Load,
                    )? {
                        PageVmFollowNavigationTurnOutcome::Completed => false,
                        PageVmFollowNavigationTurnOutcome::PostParseLifecycle { .. } => {
                            unreachable!(
                                "javascript: location navigation cannot start post-parse lifecycle"
                            )
                        }
                        PageVmFollowNavigationTurnOutcome::Download(_) => true,
                        PageVmFollowNavigationTurnOutcome::PendingPhaseOne(_) => {
                            unreachable!(
                                "javascript: location navigation cannot park in asynchronous phase-one creation"
                            )
                        }
                        PageVmFollowNavigationTurnOutcome::TriggeredNavigation { .. } => true,
                    }
                } else {
                    navigation_triggered
                };
                Ok(navigation_triggered)
            },
        )
        .await
    }

    #[cfg(test)]
    pub(crate) async fn advance_timers_until_deadline_for_test(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<()> {
        let executor = self.local_executor.clone();
        debug_assert!(
            is_on_named_owner_execution_lane_for(&executor),
            "test deadline timer advance must execute on the matching named owner lane"
        );
        let loader = loader.clone();
        let mut page_vm_ref = AwaitedOwnerLocalPageVm::new(self);
        run_named_owner_local_task(
            executor,
            "test deadline timer advance local task channel closed",
            async move {
                let page_vm = page_vm_ref.get_mut();
                let deadline = std::time::Instant::now()
                    .checked_add(std::time::Duration::from_millis(3_200))
                    .unwrap_or_else(std::time::Instant::now);
                for _ in 0..10_000 {
                    if page_vm
                        .run_one_due_timer_selected_task_for_test(&loader)
                        .await?
                    {
                        continue;
                    }
                    let Some(ms_to_next) = page_vm.vm().ms_to_next_timeout() else {
                        break;
                    };
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        break;
                    }
                    let sleep_for = std::time::Duration::from_millis(ms_to_next)
                        .min(deadline.saturating_duration_since(now));
                    if sleep_for.is_zero() {
                        continue;
                    }
                    tokio::time::sleep(sleep_for).await;
                }
                Ok(())
            },
        )
        .await?;
        Ok(())
    }

    pub(super) fn capture_page_state_on_named_owner_lane(&mut self) -> Result<PageVmStateCapture> {
        self.capture_page_state_on_named_owner_lane_with_policy(
            super::RendererPageStateCapturePolicy::FullReport,
        )
    }

    pub(super) fn capture_page_state_on_named_owner_lane_with_policy(
        &mut self,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> Result<PageVmStateCapture> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "page state capture refresh must execute on the matching named owner lane"
        );
        if self.vm().has_pending_location_navigation() {
            // Snapshot the current document before the queued location change
            // commits; wait/navigation code observes the follow-up page separately.
            tracing::debug!("capturing page state while a location navigation is pending");
        }
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "document URL refresh must execute on the matching named owner lane"
        );
        self.capture_page_state_with_policy(capture_policy)
    }

    #[cfg(test)]
    fn capture_page_state(&mut self) -> Result<PageVmStateCapture> {
        self.capture_page_state_with_policy(super::RendererPageStateCapturePolicy::FullReport)
    }

    fn capture_page_state_with_policy(
        &mut self,
        capture_policy: super::RendererPageStateCapturePolicy,
    ) -> Result<PageVmStateCapture> {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(Instant::now);
        self.absorb_parser_no_execution_runs();
        self.refresh_document_url_from_location();
        let globals_started = profile_enabled.then(Instant::now);
        match capture_policy {
            super::RendererPageStateCapturePolicy::FullReport => {
                if let Some(globals) = self.vm_mut().snapshot_globals()?
                    && self.report.replace_globals_snapshot(globals)
                {
                    self.report_snapshot_cache = None;
                }
            }
            super::RendererPageStateCapturePolicy::ProtocolTurn => {
                if self.report.mark_globals_snapshot_dirty() {
                    self.report_snapshot_cache = None;
                }
            }
        }
        let globals_us = globals_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let output_started = profile_enabled.then(Instant::now);
        self.absorb_pending_inspector_issues();
        let observable_output = self.vm_mut().take_runtime_observable_report_output()?;
        if !observable_output.is_empty() {
            self.report.extend_observable_output(observable_output);
        }
        let network_output = self.vm_mut().take_network_output();
        if !network_output.is_empty() {
            self.report.extend_network_output(network_output);
        }
        let output_us = output_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let final_url = self.vm().document_runtime.host_document().url().clone();
        let document_title = self.vm().document_runtime.dom_host().dom().document_title();
        let report_started = profile_enabled.then(Instant::now);
        let report = self.report_snapshot();
        let report_us = report_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let dedicated_worker_running_worker_isolate_count = self
            .vm()
            .dedicated_worker_running_worker_isolate_count_for_diagnostics();
        let performance_metric_snapshot = self.vm().performance_metric_snapshot_without_script(
            self.document_lifecycle.current_snapshot(),
            report.subresource_network_records().len(),
        );

        if let Some(started) = total_started {
            tracing::info!(
                target: "moli_cpu_profile",
                stage = "page_state_capture",
                page_id = self.page_id.as_u64(),
                node_count = self.vm().document_runtime.dom_host().dom().nodes().len(),
                globals_us,
                output_us,
                report_us,
                total_us = started.elapsed().as_micros(),
            );
        }

        Ok(PageVmStateCapture {
            final_url,
            document_title,
            report,
            navigation_response: self.navigation_response.clone(),
            idle_override: self.idle_override,
            service_worker_client_id: self.vm().service_worker_client_id().as_u64(),
            dedicated_worker_running_worker_isolate_count,
            performance_metric_snapshot,
        })
    }

    /// Move parser-local diagnostics into the externally reported execution
    /// history without manufacturing an HTML task for bookkeeping. Producers
    /// append to the DocumentRuntime side channel while they are inside V8 or
    /// a parser adapter; PageVm absorbs it only at an existing checkpoint or
    /// snapshot boundary.
    pub(super) fn absorb_parser_no_execution_runs(&mut self) {
        let runs = self.vm().document_runtime.take_parser_no_execution_runs();
        if !runs.is_empty() {
            self.report.runs.extend(runs);
        }
    }

    fn take_pending_inspector_issues(&mut self) -> Vec<InspectorIssueSnapshot> {
        let pending = self
            .vm_mut()
            .document_runtime
            .take_pending_inspector_issues();
        pending
            .into_iter()
            .filter_map(|issue| match issue {
                crate::document_runtime::PendingInspectorIssue::QuirksMode {
                    document,
                    is_limited_quirks_mode,
                    url,
                } => {
                    let document_node_id =
                        self.renderer_backend_node_id_for_live_handle(document)?;
                    Some(InspectorIssueSnapshot::QuirksMode(QuirksModeIssueSnapshot::new(
                        is_limited_quirks_mode,
                        document_node_id,
                        url,
                    )))
                }
                crate::document_runtime::PendingInspectorIssue::ContentSecurityPolicy {
                    target,
                    violation,
                } => {
                    let violation_type = content_security_policy_violation_type(&violation);
                    let blocked_url = (violation_type == ContentSecurityPolicyViolationType::Url
                        || violation.effective_directive == "frame-ancestors")
                        .then_some(violation.blocked_uri.clone());
                    let source_code_location = (!violation.source_file.is_empty()).then(|| {
                        InspectorSourceCodeLocationSnapshot::new(
                            violation.source_file.clone(),
                            u32::try_from(violation.line_number)
                                .unwrap_or_default()
                                .saturating_sub(1),
                            u32::try_from(violation.column_number).unwrap_or_default(),
                        )
                    });
                    let violating_node_id = target
                        .and_then(|target| self.renderer_backend_node_id_for_live_handle(target));
                    Some(InspectorIssueSnapshot::ContentSecurityPolicy(
                        ContentSecurityPolicyIssueSnapshot::new(
                            violation.disposition
                                == crate::content_security_policy::ContentSecurityPolicyDisposition::Report,
                            violation.effective_directive.to_owned(),
                            violation_type,
                        )
                        .with_blocked_url(blocked_url)
                        .with_source_code_location(source_code_location)
                        .with_violating_node_id(violating_node_id),
                    ))
                }
            })
            .collect()
    }

    /// Freezes Inspector issues in the current Page stream while retaining a
    /// second copy in the execution report used by CLI and benchmark callers.
    ///
    /// `DocumentRuntime` initially stores DOM handles because the backend node
    /// id is only available from `PageVm`. Resolve that representation at this
    /// existing owner-turn state boundary; protocol delivery must never wait
    /// for a later wake to rediscover the issue from the accumulated report.
    fn absorb_pending_inspector_issues(&mut self) {
        let issues = self.take_pending_inspector_issues();
        if issues.is_empty() {
            return;
        }
        let source_document = self.document_lifecycle.identity();
        self.append_renderer_output_records(
            issues
                .iter()
                .cloned()
                .map(|issue| {
                    PendingRendererOutputRecord::observation(
                        None,
                        RendererProtocolObservation::InspectorIssue {
                            source_document,
                            issue,
                        },
                    )
                })
                .collect(),
        );
        self.report
            .extend_observable_output(ScriptObservableOutput::from_items(
                issues
                    .into_iter()
                    .map(|issue| ScriptObservableOutputItem::InspectorIssue(Box::new(issue))),
            ));
        self.report_snapshot_cache = None;
    }

    fn report_snapshot(&mut self) -> Arc<ScriptExecutionReport> {
        let signature = ScriptExecutionReportSnapshotSignature::from_report(&self.report);
        if let Some((cached_signature, cached_report)) = self.report_snapshot_cache.as_ref()
            && *cached_signature == signature
        {
            return cached_report.clone();
        }
        let report = Arc::new(self.report.clone());
        self.report_snapshot_cache = Some((signature, report.clone()));
        report
    }

    pub(super) fn page_diagnostics_snapshot(&mut self) -> Result<RendererPageDiagnosticsSnapshot> {
        self.request_pending_cross_document_navigation_termination();
        self.ensure_document_replacement_lifecycle_journal_is_valid()?;
        self.drain_network_output_into_report();
        self.absorb_pending_inspector_issues();
        let document_lifecycle_identity = self.document_lifecycle.identity();
        let mut snapshot = self.vm_mut().page_diagnostics_snapshot()?;
        snapshot.set_document_lifecycle_identity(document_lifecycle_identity);
        let default_execution_context_id = self.vm_mut().default_execution_context_id();
        let report_lifecycle_errors = snapshot.append_report_observable_items(
            default_execution_context_id,
            self.report.observable_output_items(),
        );
        snapshot.set_document_input_stream_opened(
            self.vm().document_runtime.document_input_stream_opened(),
        );
        snapshot.diagnostics.runtime_lifecycle_errors += report_lifecycle_errors;
        Ok(snapshot)
    }

    pub(super) fn take_runtime_command_output(&mut self) -> RendererRuntimeCommandOutput {
        std::mem::take(&mut self.runtime_command_output)
    }

    fn refresh_document_url_from_location(&mut self) {
        self.vm_mut()
            .refresh_top_level_document_url_from_world_locations();
    }

    #[cfg(test)]
    pub(super) fn new(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        bootstrap_document: DomHost,
        started: Instant,
    ) -> Result<Self> {
        let mut page_vm = Self::new_with_bootstrap_document_recovery(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            bootstrap_document,
            started,
        )
        .map_err(|error| {
            let (error, _) = *error;
            error
        })?;
        page_vm
            .vm_mut()
            .set_wpt_extensions_enabled(env.wpt_extensions_enabled)?;
        page_vm.restore_runtime_inspector_sessions_on_named_owner_lane(
            &env.runtime_inspector_session_restore_snapshots,
        )?;
        Ok(page_vm)
    }

    fn new_with_bootstrap_document_recovery(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        mut runtime_hooks: PageVmRuntimeHooks,
        bootstrap_document: DomHost,
        started: Instant,
    ) -> std::result::Result<Self, ScriptVmBootstrapError> {
        let (document_lifecycle, document_lifecycle_identity) = runtime_hooks
            .install_document_lifecycle(page_id)
            .map_err(|error| Box::new((error, bootstrap_document.clone())))?;
        let page_runtime_task_source = runtime_hooks
            .renderer_page_script_environment
            .as_ref()
            .map(|environment| environment.page_runtime_task_source());
        #[cfg(test)]
        let page_runtime_task_source = page_runtime_task_source.or_else(|| {
            runtime_hooks
                .standalone_page_task_residence()
                .map(crate::page_task_queue::RendererPageTaskTestResidence::runtime_source)
        });
        let page_runtime_task_source = page_runtime_task_source.unwrap_or_else(|| {
            crate::page_task_queue::PageRuntimeTaskSource::new(runtime_hooks.owner_wake.clone())
        });
        let page_task_queue =
            PageTaskQueue::new_with_page_runtime_task_source(page_runtime_task_source.clone());
        let page_vm_isolate_bootstrap = match runtime_hooks
            .create_renderer_document_isolate_bootstrap(page_runtime_task_source.clone())
        {
            Ok(result) => result,
            Err(error) => return Err(Box::new((error, bootstrap_document))),
        };
        // The owner-reserved isolate bootstrap is allowed to create the Page
        // script environment lazily. Bind lifecycle output only after that
        // boundary, when the concrete Page stream is guaranteed to exist.
        // Binding during `install_document_lifecycle()` would work only for
        // prepared replacements and leave newly created Pages on the legacy
        // in-memory lifecycle queue.
        if let Some(environment) = runtime_hooks.renderer_page_script_environment.as_ref() {
            document_lifecycle.bind_output_journal(environment.output_journal());
        }
        // Initial Page creation binds the stable owner-local producer routes
        // while reserving the document isolate above. Resolve every typed
        // sender only after that boundary; replacements already carry the
        // same bound source through their page script environment.
        let Some(task_producer_senders) = page_runtime_task_source
            .bound_task_producer_senders(document_lifecycle_identity.document)
        else {
            return Err(Box::new((
                anyhow::anyhow!("PageVm is missing its complete typed Page producer route set"),
                bootstrap_document,
            )));
        };
        let (
            page_task_capabilities,
            main_document_runtime,
            resource_completion,
            main_parser_continuation,
            stylesheet,
            service_worker,
        ) = task_producer_senders.into_parts();
        let resource_task_runner = runtime_hooks.resource_task_runner.clone().ok_or_else(|| {
            Box::new((
                anyhow::anyhow!("PageVm is missing its resource task runner"),
                bootstrap_document.clone(),
            ))
        })?;
        let post_domcontentloaded_page_task_sender = page_task_queue
            .owner_attached_post_domcontentloaded_runtime_page_task_sender(
                main_document_runtime,
                main_parser_continuation,
                stylesheet,
                service_worker,
            );
        let script_event_parser_boundary_sender = page_task_queue.parser_boundary_sender();
        let resource_completion_sender = RendererResourceCompletionSender::for_page_scheduler(
            resource_completion,
            document_lifecycle_identity.document,
        );
        let PageVmRendererDocumentIsolateBootstrap {
            renderer_document_isolate_bootstrap,
        } = page_vm_isolate_bootstrap;
        let backend_node_registry = new_shared_renderer_backend_node_registry();
        let initial_document_loader_bootstrap =
            crate::network::context::DocumentResourceLoaderBootstrap::new(
                loader.clone(),
                resource_task_runner,
            );
        let vm_bootstrap = ScriptVmDefaultWorldBootstrap::from_dom_host_with_resource_completion_sender_browser_context_runtime_and_document_isolate(
            bootstrap_document,
            env.bypass_content_security_policy,
            post_domcontentloaded_page_task_sender,
            script_event_parser_boundary_sender,
            resource_completion_sender,
            initial_document_loader_bootstrap,
            runtime_hooks.browser_context_runtime.clone(),
            runtime_hooks.javascript_dialog_runtime.clone(),
            renderer_document_isolate_bootstrap,
            &env.runtime_inspector_session_restore_snapshots,
            backend_node_registry.clone(),
            env.root_frame_id.clone(),
            env.main_document_commit.clone(),
            env.top_level_storage_key.clone(),
            env.reserved_service_worker_client_id,
        )?;
        let mut vm = vm_bootstrap.finish()?;
        vm.set_layout_policy(env.layout_policy);
        vm.install_page_task_capabilities(page_task_capabilities);
        vm.set_root_document_lifecycle(document_lifecycle.clone());
        let dom_agent_state = vm.renderer_dom_agent_state();
        let report = ScriptExecutionReport::default();
        let creation_id = register_page_vm_creation();
        let document_loader = vm
            .current_main_document_resource_loader()
            .expect("PageVm bootstrap must publish the committed Document resource authority");
        let mut page_vm = Self {
            page_id,
            creation_id,
            document_lifecycle,
            runtime_command_output: RendererRuntimeCommandOutput::default(),
            pending_runtime_command_output: None,
            next_runtime_command_output_scope_id: 1,
            vm: Some(vm),
            report,
            report_snapshot_cache: None,
            dom_agent_state,
            pending_dom_mutation_event_batches: Vec::new(),
            last_published_document_title: String::new(),
            css_agent_sessions: HashMap::new(),
            page_task_queue,
            page_action_window: page_action_window::RendererPageActionWindow::default(),
            next_module_script_evaluation_reaction_id: 0,
            target_stage: PageVmInitStage::Load,
            request_client: document_loader.request_client().clone(),
            runtime_isolated_worlds: env.runtime_isolated_worlds.clone(),
            permission_overrides: env.permission_overrides.clone(),
            document_start_scripts: env.document_start_scripts.clone(),
            runtime_bindings: env.runtime_bindings.clone(),
            runtime_inspector_protocol_configurations: env
                .runtime_inspector_session_restore_snapshots
                .iter()
                .filter(|restore| restore.protocol_configuration.requires_restore())
                .map(|restore| {
                    (
                        DevToolsSessionKey::from_wire_session_id(
                            restore
                                .inspector_session_id
                                .as_deref()
                                .filter(|session_id| !session_id.is_empty()),
                        ),
                        restore.protocol_configuration.clone(),
                    )
                })
                .collect(),
            extra_http_headers: env.extra_http_headers.clone(),
            locale_override: env.locale_override.clone(),
            timezone_override: env.timezone_override.clone(),
            bypass_content_security_policy: env.bypass_content_security_policy,
            cpu_throttling_rate: env.cpu_throttling_rate,
            emulated_media: env.emulated_media.clone(),
            idle_override: env.idle_override,
            viewport_surface: env.viewport_surface,
            network_offline: env.network_offline,
            blocked_url_patterns: env.blocked_url_patterns.clone(),
            indexed_db_manager: env.indexed_db_manager.clone(),
            storage_bucket_store: env.storage_bucket_store.clone(),
            fetch_subresource_interception_enabled: env.fetch_subresource_interception_enabled,
            fetch_subresource_interception_resource_type: env
                .fetch_subresource_interception_resource_type,
            layout_policy: env.layout_policy,
            wpt_extensions_enabled: env.wpt_extensions_enabled,
            runtime_hooks,
            navigation_response: None,
            replacement_document_commit_handoff: None,
            local_executor,
        };
        debug!(
            phase = "runtime setup",
            elapsed_ms = started.elapsed().as_millis(),
            "page vm runtime initialized"
        );
        page_vm
            .vm_mut()
            .set_indexed_db_manager(env.indexed_db_manager.clone());
        if let Some(storage_bucket_store) = env.storage_bucket_store.clone() {
            page_vm
                .vm_mut()
                .set_storage_bucket_store(storage_bucket_store);
        }
        page_vm.vm_mut().set_web_storage_handles(&env.web_storage);
        page_vm
            .vm_mut()
            .set_script_execution_disabled(env.script_execution_disabled);
        page_vm
            .vm_mut()
            .set_permission_overrides(&env.permission_overrides);
        page_vm
            .vm_mut()
            .set_extra_http_headers(&env.extra_http_headers);
        page_vm
            .vm_mut()
            .set_document_content_security_policies(&env.document_content_security_policies);
        page_vm
            .vm_mut()
            .set_response_content_security_policies(&env.response_content_security_policies);
        page_vm
            .vm_mut()
            .set_response_content_security_report_only_policies(
                &env.response_content_security_report_only_policies,
            );
        page_vm
            .vm_mut()
            .set_response_referrer_policy(env.response_referrer_policy.clone());
        page_vm.vm_mut().set_content_security_reporting_endpoints(
            env.content_security_reporting_endpoints.clone(),
        );
        page_vm
            .vm_mut()
            .set_cross_origin_embedder_policy(env.cross_origin_embedder_policy);
        page_vm
            .vm_mut()
            .set_document_isolation_policy(env.document_isolation_policy);
        page_vm
            .vm_mut()
            .set_cross_origin_isolated(env.cross_origin_isolated);
        page_vm
            .vm_mut()
            .document_runtime
            .set_document_default_language(env.document_default_language.clone());
        page_vm
            .vm_mut()
            .document_runtime
            .set_document_source_last_modified(env.document_last_modified);
        page_vm
            .vm_mut()
            .set_stored_document_start_scripts(&env.document_start_scripts);
        page_vm
            .vm_mut()
            .set_stored_runtime_bindings(&env.runtime_bindings);
        page_vm
            .vm_mut()
            .set_locale_override(env.locale_override.as_deref());
        page_vm
            .vm_mut()
            .set_timezone_override(env.timezone_override.as_deref());
        page_vm.vm_mut().set_emulated_media(&env.emulated_media);
        page_vm.vm_mut().set_idle_override(env.idle_override);
        page_vm
            .vm_mut()
            .set_viewport_surface_for_bootstrap(env.viewport_surface);
        page_vm.vm_mut().set_network_offline(env.network_offline);
        page_vm
            .vm_mut()
            .set_blocked_url_patterns(&env.blocked_url_patterns);
        page_vm.vm_mut().set_fetch_subresource_interception(
            env.fetch_subresource_interception_enabled,
            env.fetch_subresource_interception_resource_type,
        );
        page_vm
            .vm_mut()
            .install_navigation_bootstrap_entry(env.navigation_bootstrap_entry.clone());
        Ok(page_vm)
    }

    pub(super) fn new_from_parser_stream_and_run_document_start(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        parser_session: &mut DocumentParserSession,
        started: Instant,
        before_document_start: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<(Self, bool)> {
        let mut page_vm = Self::bootstrap_page_vm_from_stream(
            page_id,
            local_executor,
            loader,
            env,
            runtime_hooks,
            parser_session,
            started,
        )?;
        page_vm.install_stored_runtime_isolated_worlds_on_named_owner_lane()?;
        page_vm.install_stored_runtime_bindings_on_named_owner_lane()?;
        let null_custom_element_registry_elements =
            parser_session.take_parser_stream_null_custom_element_registry_elements();
        page_vm
            .vm_mut()
            .apply_parser_created_null_registry_associations_in_default_context(
                &null_custom_element_registry_elements,
            )?;
        before_document_start(&mut page_vm)?;
        // Run document-start scripts directly on the runtime's live document.
        // The DomHost stays in the runtime; the next parser step borrows its
        // runtime DOM consumer instead of taking DOM ownership away.
        let document_start_scripts = page_vm.document_start_scripts.clone();
        page_vm.run_document_start_scripts_on_named_owner_lane(&document_start_scripts, |_| {})?;
        let navigation_triggered = page_vm.vm_mut().has_pending_location_navigation();
        Ok((page_vm, navigation_triggered))
    }

    pub(super) fn admit_scanned_stylesheet_preload(
        &mut self,
        request_url: Url,
        media: Option<&str>,
        options: crate::stylesheet_blocking::StylesheetFetchOptions,
        request_resource_type: moli_fetch::RequestResourceType,
        link_preload: bool,
    ) -> ScannedStylesheetAdmission {
        if self.vm().fetch_subresource_interception_matches(
            crate::types::SubresourceResourceType::Stylesheet,
        ) {
            return ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::FetchInterception,
            );
        }
        if !self.vm().stylesheet_preload_media_matches(media) {
            return ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::MediaMismatch,
            );
        }

        let (_, enforced_violation) = self
            .vm()
            .document_runtime
            .style_element_request_csp_check(
                &request_url,
                crate::content_security_policy::ContentSecurityPolicyStyleElementRequest {
                    nonce: options.nonce(),
                },
            )
            .into_violations();
        if enforced_violation.is_some() {
            return ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::ContentSecurityPolicy,
            );
        }

        match self
            .vm_mut()
            .document_runtime
            .preload_stylesheet_with_request_metadata(
                request_url,
                options,
                request_resource_type,
                link_preload,
            ) {
            Ok(_) => ScannedStylesheetAdmission::Admitted,
            Err(
                crate::document_runtime::OwnerlessStylesheetAdmissionError::ContentSecurityPolicy,
            ) => ScannedStylesheetAdmission::DeferredToParser(
                ScannedStylesheetDeferral::ContentSecurityPolicy,
            ),
        }
    }

    pub(super) fn admit_scanned_image_preload(
        &mut self,
        request_url: Url,
        fetch_priority: Option<moli_fetch::FetchPriorityHint>,
    ) -> ScannedImageAdmission {
        if self
            .vm()
            .fetch_subresource_interception_matches(crate::types::SubresourceResourceType::Image)
        {
            return ScannedImageAdmission::DeferredToParser(
                ScannedImageDeferral::FetchInterception,
            );
        }
        let (_, enforced_violation) = self
            .vm()
            .document_runtime
            .document_subresource_csp_check(
                &request_url,
                crate::document_runtime::DocumentSubresourceCspKind::Image,
            )
            .into_violations();
        if enforced_violation.is_some() {
            return ScannedImageAdmission::DeferredToParser(
                ScannedImageDeferral::ContentSecurityPolicy,
            );
        }
        let start = self
            .vm_mut()
            .start_scanned_image_preload(request_url, fetch_priority);
        match start {
            Ok(crate::network_host::ScannedImagePreloadStart::Admitted) => {
                ScannedImageAdmission::Admitted
            }
            Ok(crate::network_host::ScannedImagePreloadStart::ServiceWorker) => {
                ScannedImageAdmission::DeferredToParser(ScannedImageDeferral::ServiceWorker)
            }
            Ok(crate::network_host::ScannedImagePreloadStart::Disabled) | Err(_) => {
                ScannedImageAdmission::DeferredToParser(ScannedImageDeferral::Disabled)
            }
        }
    }

    pub(super) fn admit_scanned_script_preload(
        &self,
        request_url: &Url,
        fetch_metadata: &crate::planning::ScriptFetchMetadata,
    ) -> ScannedScriptAdmission {
        if self.script_execution_disabled() {
            return ScannedScriptAdmission::DeferredToParser(
                ScannedScriptDeferral::ScriptExecutionDisabled,
            );
        }
        if self
            .vm()
            .fetch_subresource_interception_matches(crate::types::SubresourceResourceType::Script)
        {
            return ScannedScriptAdmission::DeferredToParser(
                ScannedScriptDeferral::FetchInterception,
            );
        }
        if self
            .vm()
            .document_runtime
            .script_element_request_csp_violation_with_request(
                request_url,
                crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
                    nonce: fetch_metadata.nonce.as_deref(),
                    integrity: fetch_metadata.integrity.as_deref(),
                    parser_inserted: true,
                },
            )
            .is_some()
        {
            return ScannedScriptAdmission::DeferredToParser(
                ScannedScriptDeferral::ContentSecurityPolicy,
            );
        }
        ScannedScriptAdmission::Admitted
    }

    fn bootstrap_page_vm_from_stream(
        page_id: PageId,
        local_executor: JsLocalExecutor,
        loader: &ResourceRequestClient,
        env: &PageVmEnvConfig,
        runtime_hooks: PageVmRuntimeHooks,
        parser_session: &mut DocumentParserSession,
        started: Instant,
    ) -> Result<PageVm> {
        let mut page_vm =
            parser_session.with_parser_stream_dom_host_for_bootstrap(|bootstrap_document| {
                PageVm::new_with_bootstrap_document_recovery(
                    page_id,
                    local_executor,
                    loader,
                    env,
                    runtime_hooks,
                    bootstrap_document,
                    started,
                )
            })?;
        {
            let fallback = env.document_default_language.as_deref();
            let dom_host = page_vm.vm_mut().document_runtime.dom_host_mut();
            crate::document_language::sync_document_default_language_from_meta(dom_host, fallback);
        }
        page_vm
            .vm_mut()
            .set_wpt_extensions_enabled(env.wpt_extensions_enabled)?;
        page_vm.restore_runtime_inspector_sessions_on_named_owner_lane(
            &env.runtime_inspector_session_restore_snapshots,
        )?;
        Ok(page_vm)
    }

    pub(super) fn vm(&self) -> &ScriptVm {
        self.vm
            .as_ref()
            .expect("PageVm must retain a live ScriptVm until drop")
    }

    pub(super) fn vm_mut(&mut self) -> &mut ScriptVm {
        self.vm
            .as_mut()
            .expect("PageVm must retain a live ScriptVm until drop")
    }

    pub(super) fn renderer_backend_node_id_for_live_handle(
        &mut self,
        handle: DomHandle,
    ) -> Option<u32> {
        let document_id = self.vm().document_id_for_live_node_handle(handle)?;
        Some(self.renderer_backend_node_id_for_node_key(document_id, handle))
    }

    pub(super) fn renderer_backend_node_id_for_inspector_node(
        &mut self,
        host: DomHandle,
        inspector_identity: moli_page_types::DocumentNodeInspectorIdentity,
    ) -> Option<u32> {
        let document_id = self.vm().document_id_for_live_node_handle(host)?;
        Some(self.dom_agent_state.backend_node_id_for_inspector_node(
            document_id,
            host,
            inspector_identity,
        ))
    }

    fn renderer_backend_node_id_for_node_key(
        &mut self,
        document_id: DocumentId,
        handle: DomHandle,
    ) -> u32 {
        self.dom_agent_state
            .backend_node_id_for_node(document_id, handle)
    }

    pub(super) fn live_handle_for_renderer_backend_node_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<DomHandle> {
        let key = self.current_renderer_backend_node_key_for_id(backend_node_id)?;
        key.inspector_identity.is_none().then_some(key.handle)
    }

    pub(super) fn current_renderer_backend_node_key_for_id(
        &mut self,
        backend_node_id: u32,
    ) -> Option<backend_node_registry::RendererBackendNodeKey> {
        let key = self
            .dom_agent_state
            .backend_node_key_for_id(backend_node_id)?;
        let handle = key.handle;
        let current_document_id = self.vm().document_id_for_live_node_handle(handle);
        let node_exists = self.vm().document_runtime.dom_host().node(handle).is_some();
        let still_current = current_document_id == Some(key.document_id) && node_exists;
        let retained_detached = node_exists
            && self
                .dom_agent_state
                .backend_node_resolves_while_detached(backend_node_id);
        if still_current || retained_detached {
            return Some(key);
        }

        self.dom_agent_state
            .remove_stale_backend_node_id(backend_node_id, key);
        None
    }

    fn current_dom_agent_document_id(&self) -> Option<DocumentId> {
        self.vm()
            .current_main_document_task_owner()
            .map(|owner| owner.document_id)
            .or_else(|| {
                let document_handle = self
                    .vm()
                    .document_runtime
                    .dom_host()
                    .dom()
                    .document_node_id();
                self.vm().document_id_for_live_node_handle(document_handle)
            })
    }

    pub(crate) fn configure_document_dom_agent_session(
        &mut self,
        inspector_session_id: Option<&str>,
        include_whitespace: bool,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.set_include_whitespace(
            inspector_session_id,
            document_id,
            include_whitespace,
        );
    }

    pub(super) fn document_dom_agent_includes_whitespace(
        &self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        self.dom_agent_state
            .includes_whitespace(inspector_session_id, self.current_dom_agent_document_id())
    }

    pub(super) fn discard_document_frontend_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .discard_frontend_bindings(inspector_session_id, document_id);
        self.sync_devtools_dom_mutation_recording_interest();
    }

    pub(in crate::runtime::page_vm) fn register_document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        matches: Vec<dom_search::DocumentSearchMatch>,
    ) -> crate::RendererDomSearchRegistration {
        let include_whitespace = self.document_dom_agent_includes_whitespace(inspector_session_id);
        let document_handle = self
            .vm()
            .document_runtime
            .dom_host()
            .dom()
            .document_node_id();
        let document_is_published = self
            .renderer_backend_node_id_for_live_handle(document_handle)
            .is_some_and(|document_backend_node_id| {
                self.document_has_frontend_node_id_for_backend_node_id(
                    inspector_session_id,
                    document_backend_node_id,
                )
            });
        let mut nodes = Vec::with_capacity(matches.len());
        for search_match in matches {
            let (backend_node_id, is_whitespace_text) = match search_match {
                dom_search::DocumentSearchMatch::Live(handle) => {
                    let Some(backend_node_id) =
                        self.renderer_backend_node_id_for_live_handle(handle)
                    else {
                        continue;
                    };
                    let is_whitespace_text = super::page_dom::inspector_whitespace_text_node(
                        self.vm().document_runtime.dom_host(),
                        handle,
                    );
                    (backend_node_id, is_whitespace_text)
                }
                dom_search::DocumentSearchMatch::Generated {
                    host,
                    identity,
                    is_whitespace_text,
                } => {
                    let Some(backend_node_id) =
                        self.renderer_backend_node_id_for_inspector_node(host, identity)
                    else {
                        continue;
                    };
                    (backend_node_id, is_whitespace_text)
                }
            };
            let frontend_node_id = if document_is_published {
                self.document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
                    inspector_session_id,
                    backend_node_id,
                    include_whitespace,
                    is_whitespace_text,
                )
            } else {
                // Chromium preserves the result count before a session has
                // requested its document, but keeps every result unbound.
                // Search must not become an implicit document publication path.
                0
            };
            nodes.push(crate::RendererDomSearchResultNode {
                frontend_node_id,
                backend_node_id,
            });
        }
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .register_search_results(inspector_session_id, document_id, nodes)
    }

    pub(super) fn document_frontend_node_id_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> u32 {
        let document_id = self.current_dom_agent_document_id();
        let frontend_node_id = self.dom_agent_state.frontend_node_id_for_backend_node_id(
            inspector_session_id,
            document_id,
            backend_node_id,
        );
        self.sync_devtools_dom_mutation_recording_interest();
        frontend_node_id
    }

    pub(super) fn document_frontend_node_id_for_backend_node_id_in_whitespace_projection(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        include_whitespace: bool,
        is_whitespace_text: bool,
    ) -> u32 {
        // Chromium keeps the node's backend identity and any search-result
        // position, but represents a node hidden from this session's Inspector
        // projection with frontend id 0. In particular, do not create a binding
        // that another command can use to re-expose the whitespace node.
        if !include_whitespace && is_whitespace_text {
            return 0;
        }
        self.document_frontend_node_id_for_backend_node_id(inspector_session_id, backend_node_id)
    }

    pub(super) fn document_has_frontend_node_id_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> bool {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .has_frontend_node_id_for_backend_node_id(
                inspector_session_id,
                document_id,
                backend_node_id,
            )
    }

    pub(super) fn document_node_children_requested(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> bool {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .children_requested(inspector_session_id, document_id, backend_node_id)
    }

    pub(super) fn document_frontend_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> crate::RendererDomFrontendNodeBindingResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.frontend_node_binding(
            inspector_session_id,
            document_id,
            frontend_node_id,
        )
    }

    pub(super) fn register_document_bidi_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        shared_id: String,
        backend_node_id: u32,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.register_bidi_node_binding(
            inspector_session_id,
            document_id,
            shared_id,
            backend_node_id,
        );
    }

    pub(super) fn document_bidi_node_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        shared_id: &str,
    ) -> crate::RendererDomBidiNodeBindingResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .bidi_node_binding(inspector_session_id, document_id, shared_id)
    }

    pub(super) fn document_bidi_node_shared_id_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
    ) -> crate::RendererDomBidiNodeSharedIdResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .bidi_node_shared_id_for_backend_node_id(
                inspector_session_id,
                document_id,
                backend_node_id,
            )
    }

    pub(super) fn document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        search_id: &str,
        from_index: usize,
        to_index: usize,
    ) -> crate::RendererDomSearchResultsResolution {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.search_results_slice(
            inspector_session_id,
            document_id,
            search_id,
            from_index,
            to_index,
        )
    }

    pub(super) fn discard_document_search_results(
        &mut self,
        inspector_session_id: Option<&str>,
        search_id: &str,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state
            .discard_search_results(inspector_session_id, document_id, search_id);
    }

    pub(super) fn set_document_node_stack_traces_enabled(
        &mut self,
        inspector_session_id: Option<&str>,
        enabled: bool,
    ) {
        let document_id = self.current_dom_agent_document_id();
        self.dom_agent_state.set_node_stack_traces_enabled(
            inspector_session_id,
            document_id,
            enabled,
        );
    }

    pub(super) fn document_node_stack_trace(
        &mut self,
        inspector_session_id: Option<&str>,
        frontend_node_id: u32,
    ) -> crate::RendererDomNodeStackTraceResolution {
        let document_id = self.current_dom_agent_document_id();
        let crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id) = self
            .dom_agent_state
            .frontend_node_binding(inspector_session_id, document_id, frontend_node_id)
        else {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        };
        let Some(key) = self
            .dom_agent_state
            .backend_node_key_for_id(backend_node_id)
        else {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        };
        if Some(key.document_id) != document_id
            || key.inspector_identity.is_some()
            || self
                .vm()
                .document_runtime
                .dom_host()
                .node(key.handle)
                .is_none()
        {
            return crate::RendererDomNodeStackTraceResolution::MissingNode;
        }
        crate::RendererDomNodeStackTraceResolution::Found(
            self.dom_agent_state.node_creation_stack_trace(
                inspector_session_id,
                document_id,
                key.document_id,
                key.handle,
            ),
        )
    }

    pub(super) fn bind_file_chooser_activation_backend_node_id(
        &mut self,
        activation: &mut crate::RendererPendingFileChooserActivation,
    ) -> bool {
        let Some((handle, document_id)) = activation.live_node_source() else {
            return activation.backend_node_id != 0;
        };
        activation.node_id = Some(handle);
        activation.backend_node_id =
            self.renderer_backend_node_id_for_node_key(document_id, handle);
        assert!(
            self.dom_agent_state
                .retain_detached_backend_node_resolution(activation.backend_node_id),
            "file chooser backend node id must exist before it is exposed"
        );
        true
    }

    /// Returns the exact authority for the currently committed main Document.
    ///
    /// `PageVm` deliberately does not cache this value: `document.open()` and
    /// cross-document commits replace the owner generation inside `ScriptVm`.
    /// Looking it up here prevents orchestration code from retaining a stale
    /// Document authority while the Page-level request client remains stable.
    pub(super) fn main_document_resource_loader(&self) -> DocumentResourceLoader {
        self.vm()
            .current_main_document_resource_loader()
            .expect("a live PageVm must retain its committed Document resource authority")
    }

    pub(super) fn resource_task_runner(&self) -> crate::network::RendererResourceTaskRunner {
        self.main_document_resource_loader().task_runner()
    }

    pub(super) fn set_document_character_set(&mut self, character_set: impl Into<String>) {
        self.vm_mut()
            .document_runtime
            .set_document_character_set(character_set);
    }

    pub(super) fn script_execution_disabled(&self) -> bool {
        self.vm().script_execution_disabled()
    }

    pub(crate) fn script_execution_control(
        &self,
    ) -> crate::script_execution_control::RendererScriptExecutionControl {
        self.vm().script_execution_control()
    }

    pub(crate) fn bind_script_execution_control(
        &mut self,
        control: crate::script_execution_control::RendererScriptExecutionControl,
    ) {
        self.vm_mut().bind_script_execution_control(control);
    }

    pub(super) fn set_target_stage(&mut self, stage: PageVmInitStage) {
        self.target_stage = stage;
    }

    #[cfg(test)]
    pub(super) async fn finish_post_parse_execution_on_named_owner_lane(
        mut self,
        work: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    ) -> Result<PageVmNavigationTurnOutcome> {
        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "post-parse runtime finish must execute on the matching named owner lane"
        );
        self.set_target_stage(stage);
        let lifecycle_driver = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut self;
            vm.as_mut()
                .expect("PageVm must retain a live ScriptVm until drop")
                .start_post_parse_lifecycle_round(stage, page_task_queue, report, work)
                .await
        };
        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmNavigationTurnOutcome::TriggeredNavigation);
        }
        let completion_action = self
            .drive_post_parse_lifecycle_loop_on_named_owner_lane(stage, lifecycle_driver)
            .await?;
        let triggered_navigation = matches!(
            completion_action,
            PostParseLifecycleCompletionAction::TriggeredNavigation
        );
        self.finish_post_parse_lifecycle_completion_on_named_owner_lane(
            stage,
            started,
            completion_action,
        )
        .await?;
        if triggered_navigation || self.vm().has_pending_location_navigation() {
            return Ok(PageVmNavigationTurnOutcome::TriggeredNavigation);
        }

        debug_assert!(
            is_on_named_owner_execution_lane_for(&self.local_executor),
            "phase-2 final handout must execute on the matching named owner lane"
        );
        Ok(PageVmNavigationTurnOutcome::Completed(Box::new(self)))
    }
}

#[cfg(test)]
async fn wait_for_page_task_source_load_arrival(
    load: Option<crate::planning::SharedScriptSourceLoad>,
) -> bool {
    let Some(load) = load else {
        return std::future::pending().await;
    };
    let _ = load.wait_outcome().await;
    true
}

fn register_page_vm_creation() -> u64 {
    PAGE_VM_DROP_TRACKER.with(|tracker| {
        let mut tracker = tracker.borrow_mut();
        tracker.next_id += 1;
        let id = tracker.next_id;
        tracker.creation_order.push(id);
        id
    })
}

fn defer_page_vm_drop(creation_id: u64, vm: ScriptVm) {
    let ready_to_drop = PAGE_VM_DROP_TRACKER.with(|tracker| {
        let mut tracker = tracker.borrow_mut();
        tracker.pending.insert(creation_id, vm);

        let mut ready = Vec::new();
        while let Some(latest_id) = tracker.creation_order.last().copied() {
            let Some(vm) = tracker.pending.remove(&latest_id) else {
                break;
            };
            tracker.creation_order.pop();
            ready.push(vm);
        }

        ready
    });

    for vm in ready_to_drop {
        drop(vm);
    }
}

#[cfg(test)]
pub(crate) fn deferred_page_vm_drop_pending_count_for_testing() -> usize {
    PAGE_VM_DROP_TRACKER.with(|tracker| tracker.borrow().pending.len())
}

fn javascript_location_navigation_source(url: &Url) -> String {
    let source = url
        .as_str()
        .strip_prefix("javascript:")
        .unwrap_or_else(|| url.path());
    percent_decode_str(source).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests;
