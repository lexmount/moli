mod constructor_preservation;
mod parse_errors;
use std::pin::pin;

use super::{
    DYNAMIC_MODULE_REACTION_ID_SLOT, MainNativeModuleSelectedTaskApplication,
    NativeChildModuleScriptReactionDataDeclaration, NativeDynamicModuleReactionDataDeclaration,
    NativeModuleScriptReactionDataDeclaration, ScriptVmCheckpointingMainNativeModuleTaskBody,
    get_u64_reaction_data_slot, native_child_module_script_reaction_data,
    native_module_script_reaction_data, preserve_current_v8_module_exception,
};
use crate::dom::native::{DomHost, NativeDom};
use crate::ensure_v8_for_test as ensure_v8;
use crate::frame_owner_model::{
    ChildFrameSemanticTurnKind, DocumentId, FrameDocumentDynamicImportGraphAdvanceFollowup,
    FrameDocumentDynamicImportMissingJoinedTerminalFetch, FrameDocumentDynamicImportOwnerAction,
    FrameDocumentDynamicImportPendingJobResume, FrameDocumentOwner, FrameDocumentTaskOwner,
    FrameRealmId, FrameSchedulerLaneId, LocalWindowId,
};
use crate::module_runtime::{
    DynamicModuleFetchFailure, DynamicModuleFetchOwnerAdvance, DynamicModuleImportOwner,
    ModuleAttributesKey, ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource,
    ModuleGraphHandle, ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage,
    ModuleMapEntryState, ModuleMapKey, ModuleSource, NativeModuleGraphFetchRequest,
    NativeModuleGraphJob, NativeModuleGraphJobAdvance, PendingDynamicModuleImport,
};
use crate::module_script_continuation::NativeDynamicModuleTerminalFanout;
use crate::network::ResourceRequestClient;
use crate::script_vm::{ScriptVm, ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness};
use crate::types::{
    ModuleGraphFetchCompletion, ModuleGraphFetchOrdering, ModuleGraphFetchRequester,
    ScriptErrorConstructorKind,
};
use crate::util::v8str;
use moli_fetch::FetchConfig;
use url::Url;

fn new_test_vm(url: &str) -> StandaloneScriptVmHarness {
    let _js_runtime = crate::JsRuntime::initialize();
    let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
    let post_domcontentloaded_page_task_sender =
        page_task_queue.owner_attached_runtime_page_task_sender_for_test();
    let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
    let page_runtime_task_source = page_task_queue.residence();
    ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
        DomHost::from_dom(NativeDom::new(Url::parse(url).expect("test url"))),
        post_domcontentloaded_page_task_sender,
        page_task_front_injection_tx,
    )
    .expect("script vm bootstrap should succeed")
    .finish()
    .map(|mut vm| {
        vm.install_page_task_residence_for_executor_test(page_runtime_task_source);
        vm
    })
    .expect("script vm finish should succeed")
}

fn dynamic_import_completion(load_id: u64, request_url: &str) -> ModuleGraphFetchCompletion {
    dynamic_import_completion_with_source(load_id, request_url, "export const value = 1;")
}

fn dynamic_import_completion_with_source(
    load_id: u64,
    request_url: &str,
    source: &str,
) -> ModuleGraphFetchCompletion {
    let url = Url::parse(request_url).expect("request URL should parse");
    ModuleGraphFetchCompletion {
        load_id,
        requester: ModuleGraphFetchRequester::DynamicImport,
        ordering: ModuleGraphFetchOrdering::Runtime,
        request_url: url.clone(),
        result: Ok(ModuleGraphFetchedSource::new(
            url,
            false,
            ModuleSource::text(source.to_owned()),
        )),
        network_result: None,
    }
}

fn dynamic_import_reaction_parts_in_vm(
    vm: &mut ScriptVm,
    owner: DynamicModuleImportOwner,
) -> (
    PendingDynamicModuleImport,
    crate::module_runtime::DynamicModuleEvaluationTarget,
) {
    let context_ptr: *const v8::Global<v8::Context> = match owner.child_parts() {
        None => &vm.page_default_context,
        Some((_child_handle, _task_owner, realm_id)) => vm
            .frame_realm_context_ptr(realm_id)
            .expect("child dynamic-import reaction test realm must be materialized"),
    };
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let source = v8::String::new(scope, "export default 1;").expect("test module source");
            let source_name =
                v8::String::new(scope, "https://dynamic-reaction-owner.test/pending.mjs")
                    .expect("test module source name");
            let origin = v8::ScriptOrigin::new(
                scope,
                source_name.into(),
                0,
                0,
                false,
                -1,
                None,
                false,
                false,
                true,
                None,
            );
            let mut compiler_source = v8::script_compiler::Source::new(source, Some(&origin));
            let module = v8::script_compiler::compile_module(scope, &mut compiler_source)
                .expect("test module should compile");
            Ok((
                PendingDynamicModuleImport::new(
                    v8::Global::new(scope, scope.get_current_context()),
                    v8::Global::new(scope, resolver),
                    owner,
                    "./pending.mjs",
                    Url::parse("https://dynamic-reaction-owner.test/page.html")
                        .expect("test base URL"),
                    ModuleAttributesKey::empty(),
                    ModuleImportPhase::Evaluation,
                ),
                crate::module_runtime::DynamicModuleEvaluationTarget::new(
                    ModuleEntryId::for_test(0),
                    v8::Global::new(scope, module),
                ),
            ))
        })
        .expect("test reaction payload should be created in the ScriptVm isolate")
}

fn dynamic_import_request_in_vm(
    vm: &mut ScriptVm,
    specifier: &str,
    base_url: Url,
    phase: ModuleImportPhase,
) -> PendingDynamicModuleImport {
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &vm.page_default_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let owner = context_host
                .borrow()
                .current_dynamic_module_import_owner(scope, None)
                .expect("test ScriptVm must have a main dynamic-import owner");
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            Ok(PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                owner,
                specifier,
                base_url,
                ModuleAttributesKey::empty(),
                phase,
            ))
        })
        .expect("test dynamic import request should be created in the ScriptVm isolate")
}

fn current_child_dynamic_import_owner(
    vm: &mut ScriptVm,
    child_handle: crate::document_runtime::DomHandle,
) -> DynamicModuleImportOwner {
    let realm_id = vm
        .child_frame_realm_store
        .values()
        .find(|realm| realm.child_handle == child_handle)
        .map(|realm| realm.owner_realm_id)
        .expect("test child document must have a current realm");
    vm.with_frame_realm_scope_and_checkpoint_for_test(realm_id, move |scope, host_ptr| {
        unsafe { &*host_ptr }
            .current_dynamic_module_import_owner(scope, Some(child_handle))
            .ok_or_else(|| {
                anyhow::anyhow!("test child document must have a current dynamic-import owner")
            })
    })
    .expect("test child owner scope should enter")
}

async fn commit_child_document_and_run_parser_script_for_dynamic_import_test(
    vm: &mut ScriptVm,
    label: &str,
) {
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "{label} should commit its srcdoc navigation before script execution"
    );
    while vm
        .run_child_realm_materialization_body_for_test()
        .expect("child realm materialization prerequisite should succeed")
    {
        // Consume only consecutive realm tasks at the child-family head;
        // a script task must never be bypassed to reach a later realm.
    }
    assert!(
        vm.run_child_frame_task_source_once_for_test(
            ChildFrameSemanticTurnKind::DocumentScriptReady
        )
        .await,
        "{label} parser script should run from DocumentScriptReady"
    );
}

async fn finish_child_document_after_parser_script_for_dynamic_import_test(
    vm: &mut ScriptVm,
    label: &str,
) {
    for transition in ["DOMContentLoaded", "complete"] {
        assert!(
            vm.run_child_frame_task_source_once_for_test(
                ChildFrameSemanticTurnKind::DocumentLifecycle,
            )
            .await,
            "{label} should run its {transition} lifecycle turn"
        );
    }
    assert!(
        vm.run_child_frame_task_source_once_for_test(ChildFrameSemanticTurnKind::HostLoad)
            .await,
        "{label} should finish through a later HostLoad turn"
    );
}

async fn commit_child_document_and_run_parser_script_for_page_executor_test(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    label: &str,
) {
    for expected in [
        ChildFrameSemanticTurnKind::NavigationCommit,
        ChildFrameSemanticTurnKind::RealmMaterialization,
        ChildFrameSemanticTurnKind::DocumentScriptReady,
    ] {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn().await,
            Some(expected),
            "{label} should advance {expected:?} through the real Page-owned realm prerequisite"
        );
    }
}

async fn finish_child_document_after_parser_script_for_page_executor_test(
    vm: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    label: &str,
) {
    for expected in [
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::DocumentLifecycle,
        ChildFrameSemanticTurnKind::HostLoad,
    ] {
        assert_eq!(
            vm.run_next_child_frame_semantic_turn().await,
            Some(expected),
            "{label} should advance {expected:?} after its parser script"
        );
    }
}

fn dynamic_import_job_in_vm(
    vm: &mut ScriptVm,
    specifier: &str,
    base_url: Url,
    phase: ModuleImportPhase,
) -> NativeModuleGraphJob {
    NativeModuleGraphJob::dynamic_import(dynamic_import_request_in_vm(
        vm, specifier, base_url, phase,
    ))
}

#[test]
fn reaction_data_slots_preserve_u64_values_above_js_safe_integer() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let reaction_id = (1_u64 << 53) + 123;
    let data = NativeDynamicModuleReactionDataDeclaration {
        reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
    }
    .bind(scope)
    .expect("reaction data declaration should bind");

    assert_eq!(
        get_u64_reaction_data_slot(scope, data, DYNAMIC_MODULE_REACTION_ID_SLOT),
        Some(reaction_id)
    );

    let scheduler_lane_id = (1_u64 << 53) + 125;
    let local_window_id = (1_u64 << 53) + 126;
    let document_id = (1_u64 << 53) + 127;
    let realm_id = -17_i64;
    let document_owner = FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(scheduler_lane_id),
        LocalWindowId(local_window_id),
        DocumentId(document_id),
    );
    let main_data = NativeModuleScriptReactionDataDeclaration {
        module_script_reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
        scheduler_lane_id: v8::BigInt::new_from_u64(scope, scheduler_lane_id),
        local_window_id: v8::BigInt::new_from_u64(scope, local_window_id),
        document_id: v8::BigInt::new_from_u64(scope, document_id),
    }
    .bind(scope)
    .expect("main reaction data declaration should bind");
    assert_eq!(
        native_module_script_reaction_data(scope, main_data.into()),
        Some((document_owner, reaction_id)),
        "callback data must preserve the exact main Document without Number coercion"
    );
    let child_data = NativeChildModuleScriptReactionDataDeclaration {
        module_script_reaction_id: v8::BigInt::new_from_u64(scope, reaction_id),
        scheduler_lane_id: v8::BigInt::new_from_u64(scope, scheduler_lane_id),
        local_window_id: v8::BigInt::new_from_u64(scope, local_window_id),
        document_id: v8::BigInt::new_from_u64(scope, document_id),
        realm_id: v8::BigInt::new_from_i64(scope, realm_id),
    }
    .bind(scope)
    .expect("child reaction data declaration should bind");
    assert_eq!(
        native_child_module_script_reaction_data(scope, child_data.into()),
        Some((document_owner, FrameRealmId(realm_id), reaction_id)),
        "callback data must preserve the exact child Document and realm without Number coercion"
    );
}

#[test]
fn native_module_instantiate_errors_use_stage_appropriate_constructors() {
    let javascript_error = super::native_module_instantiate_load_error(
        "javascript link failed".to_owned(),
        None,
        false,
    );
    assert_eq!(javascript_error.stage(), ModuleLoadStage::Instantiate);
    assert_eq!(
        javascript_error.error_constructor(),
        Some(ScriptErrorConstructorKind::SyntaxError)
    );

    let wasm_error = super::native_module_instantiate_load_error(
        "WebAssembly link failed".to_owned(),
        None,
        true,
    );
    assert_eq!(
        wasm_error.error_constructor(),
        Some(ScriptErrorConstructorKind::WebAssemblyLinkError)
    );

    let caught_syntax_error = super::native_module_instantiate_load_error(
        "mixed graph failed".to_owned(),
        Some(ScriptErrorConstructorKind::SyntaxError),
        true,
    );
    assert_eq!(
        caught_syntax_error.error_constructor(),
        Some(ScriptErrorConstructorKind::SyntaxError),
        "an exact V8 exception constructor must win over the module-kind fallback"
    );
}

#[test]
fn native_module_instantiate_preserves_caught_v8_type_errors() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let source = v8str(&scope, "null.missing");
    let script = v8::Script::compile(&scope, source, None).expect("valid script");
    assert!(crate::script_execution::execute_compiled_script(&mut scope, script).is_none());
    let exception = scope.exception().expect("V8 should throw a TypeError");
    let caught_constructor = super::script_error_constructor_kind_from_value(&mut scope, exception);
    assert_eq!(
        caught_constructor,
        Some(ScriptErrorConstructorKind::TypeError)
    );

    for has_wasm_entry in [false, true] {
        let error = super::native_module_instantiate_load_error(
            "caught V8 exception".to_owned(),
            caught_constructor,
            has_wasm_entry,
        );
        assert_eq!(error.stage(), ModuleLoadStage::Instantiate);
        assert_eq!(
            error.error_constructor(),
            Some(ScriptErrorConstructorKind::TypeError),
            "the graph fallback must preserve the caught TypeError (wasm={has_wasm_entry})"
        );
    }
}

#[test]
fn dynamic_import_fetch_completion_requires_owner_facade() {
    let mut vm = new_test_vm("https://app.example.test/page.html");
    let document_owner = vm
        .current_main_document_task_owner()
        .expect("dynamic import test must have a main document owner");
    vm.eval("import('./dynamic.mjs'); 'queued'")
        .expect("dynamic import should queue from current document isolate");

    let queued_job = vm
        .document_runtime
        .take_next_native_dynamic_module_import()
        .expect("dynamic import callback should enqueue one graph job");
    let queued_owner = queued_job
        .dynamic_import_request()
        .expect("dynamic graph job must retain its request")
        .owner();
    assert_eq!(queued_owner.task_owner(), document_owner);
    assert_eq!(
        queued_owner.execution_context_owner(),
        crate::native_bridge::WindowExecutionContextOwner::Frame(document_owner.local_window_id),
        "dynamic import acceptance must capture the exact current Window execution context"
    );
    vm.document_runtime
        .resume_native_dynamic_module_import_front(queued_job);

    assert!(
        matches!(
            vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
            MainNativeModuleSelectedTaskApplication::Applied(_)
        ),
        "dynamic import should start its module graph through the selected body"
    );
    assert!(
        vm.has_inflight_dynamic_module_fetch(),
        "dynamic import root fetch should be suspended in owner state"
    );

    vm.complete_native_module_graph_fetch(dynamic_import_completion(
        0,
        "https://app.example.test/dynamic.mjs",
    ))
    .expect("bare graph completion helper should tolerate stale completions");
    assert!(
        vm.has_inflight_dynamic_module_fetch(),
        "bare graph completion helper must not consume dynamic import owner fetches"
    );
    assert!(
            vm.runtime_observable_lifecycle_errors_for_testing()
                .iter()
                .any(|message| message.contains(
                    "native module graph fetch completion 0 arrived without an in-flight module graph job"
                )),
            "bare helper should record the unmatched completion instead of consuming owner state"
        );

    let target = vm
        .current_main_dynamic_import_graph_fetch_target(0)
        .expect("dynamic import must expose its exact resolver target");
    let completion = dynamic_import_completion(0, "https://app.example.test/dynamic.mjs");
    let actions = vm
        .complete_current_main_dynamic_import_graph_fetch_result(target, completion.result)
        .expect("owner facade should complete dynamic import fetch");
    assert!(
        actions.into_parts().0.is_empty(),
        "dynamic import owner completion should not synthesize module-script run actions"
    );
    assert!(
        !vm.has_inflight_dynamic_module_fetch(),
        "owner facade should consume dynamic import fetch state"
    );
}

#[test]
fn dynamic_import_fetch_failures_reject_joined_and_cached_imports() {
    for shared_dependency in [false, true] {
        let mut vm = new_test_vm("https://app.example.test/page.html");
        let second_specifier = if shared_dependency {
            "./second.mjs"
        } else {
            "./first.mjs"
        };
        vm.eval(&format!(
            r#"
globalThis.__fetchErrors = [];
globalThis.__failedImport = specifier => import(specifier).then(
  () => __fetchErrors.push(null),
  error => __fetchErrors.push(error)
);
__failedImport('./first.mjs');
__failedImport({second_specifier:?});
"queued"
"#
        ))
        .expect("both imports should enqueue their graph jobs");
        for _ in 0..2 {
            assert!(matches!(
                vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
                MainNativeModuleSelectedTaskApplication::Applied(_)
            ));
        }

        let (failed_load_id, failed_url) = if shared_dependency {
            for (load_id, root) in [(0, "first"), (1, "second")] {
                let target = vm
                    .current_main_dynamic_import_graph_fetch_target(load_id)
                    .expect("each root should have its own fetch");
                let completion = dynamic_import_completion_with_source(
                    load_id,
                    &format!("https://app.example.test/{root}.mjs"),
                    "import './shared.mjs';",
                );
                vm.complete_current_main_dynamic_import_graph_fetch_result(
                    target,
                    completion.result,
                )
                .expect("the roots should start or join the shared dependency fetch");
            }
            (2, "https://app.example.test/shared.mjs")
        } else {
            (0, "https://app.example.test/first.mjs")
        };
        assert_eq!(vm.eval("String(__fetchErrors.length)").unwrap(), "0");
        let target = vm
            .current_main_dynamic_import_graph_fetch_target(failed_load_id)
            .expect("the shared request should retain its fetch owner");
        vm.complete_current_main_dynamic_import_graph_fetch_result(
            target,
            Err("HTTP 404".to_owned()),
        )
        .expect("fetch failure should reject both graph clients");

        let key = ModuleMapKey::java_script(url::Url::parse(failed_url).unwrap());
        let entry = vm.document_runtime.native_module_entry_id(&key).unwrap();
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry),
            ModuleMapEntryState::Failed,
            "a failed request must settle the shared module map entry"
        );
        assert_eq!(
                vm.eval("JSON.stringify([__fetchErrors.length, __fetchErrors.every(e => e instanceof TypeError), new Set(__fetchErrors).size])")
                    .unwrap(),
                "[2,true,2]",
                "both concurrent imports must reject with distinct TypeErrors"
            );

        vm.eval("__failedImport('./first.mjs'); 'queued'").unwrap();
        assert!(matches!(
            vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
            MainNativeModuleSelectedTaskApplication::Applied(_)
        ));
        // The selected body leaves Promise reactions to its task-end checkpoint.
        vm.perform_script_task_checkpoint(None)
            .expect("the cached import task should dispatch its rejection reaction");
        assert_eq!(
                vm.eval("JSON.stringify([__fetchErrors.length, __fetchErrors.every(e => e instanceof TypeError), new Set(__fetchErrors).size])")
                    .unwrap(),
                "[3,true,3]",
                "a cached fetch failure must reject a later import with a fresh TypeError"
            );
        assert!(!vm.has_inflight_dynamic_module_fetch());
    }
}

#[test]
fn module_reaction_source_consumes_exactly_one_current_target_per_turn() {
    let mut vm = new_test_vm("https://module-reaction-one-turn.test/page.html");
    let document_owner = vm
        .current_main_document_task_owner()
        .expect("module reaction fixture requires a main Document owner");
    for reaction_id in [71, 72] {
        vm._context_host
            .borrow_mut()
            .queue_document_module_script_evaluation_fulfilled(document_owner, reaction_id);
    }

    assert_eq!(
        vm.run_page_module_reaction_body_for_test()
            .expect("first module-reaction turn"),
        Some(crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction)
    );
    assert!(
        vm.has_page_module_reaction_for_executor_test(),
        "the second accepted reaction must remain queued after one bounded turn"
    );
    assert_eq!(
        vm.run_page_module_reaction_body_for_test()
            .expect("second module-reaction turn"),
        Some(crate::page_task_queue::PageModuleReactionTargetEffect::DiscardedMissingReaction)
    );
    assert!(!vm.has_page_module_reaction_for_executor_test());
}

#[test]
fn dynamic_import_owner_survives_main_document_open_in_same_execution_context() {
    let mut vm = new_test_vm("https://dynamic-reaction-owner.test/page.html");
    let request = dynamic_import_request_in_vm(
        &mut vm,
        "./pending.mjs",
        Url::parse("https://dynamic-reaction-owner.test/page.html").expect("base URL"),
        ModuleImportPhase::Evaluation,
    );
    let import_owner = request.owner();
    let retired_document_owner = import_owner.task_owner();

    vm.eval("document.open(); 'replaced'")
        .expect("document.open should rotate the main document owner");
    assert_ne!(
        vm.current_main_document_task_owner(),
        Some(retired_document_owner),
        "test setup must retire the original document owner"
    );
    assert!(
        vm.dynamic_module_import_owner_is_current(import_owner),
        "document.open must preserve the ScriptState-like dynamic-import owner"
    );
    assert_eq!(
        import_owner.execution_context_owner(),
        crate::native_bridge::WindowExecutionContextOwner::Frame(
            vm.current_main_document_task_owner()
                .expect("replacement owner")
                .local_window_id
        )
    );
}

#[tokio::test]
async fn child_replacement_retires_registered_dynamic_import_reaction() {
    let mut vm = new_test_vm("https://dynamic-reaction-child.test/page.html");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<script>parent.__dynamicReactionChildReady = true;<\/script>";
  body.appendChild(frame);
})()
"#,
    )
    .expect("dynamic reaction child setup should evaluate");
    commit_child_document_and_run_parser_script_for_dynamic_import_test(
        &mut vm,
        "dynamic reaction child",
    )
    .await;
    finish_child_document_after_parser_script_for_dynamic_import_test(
        &mut vm,
        "dynamic reaction child",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("dynamic reaction child realm should exist");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("dynamic reaction child realm record should exist")
        .child_handle;
    let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
    let (request, target) = dynamic_import_reaction_parts_in_vm(&mut vm, document_owner);
    let reaction_id = vm
        .document_runtime
        .reserve_native_dynamic_module_evaluation_reaction(request, target);
    vm._context_host
        .borrow_mut()
        .queue_native_dynamic_module_evaluation_fulfilled_for_owner_for_test(
            document_owner,
            reaction_id,
        );
    assert_eq!(
        vm.document_runtime
            .native_dynamic_module_evaluation_reaction_owner(reaction_id),
        Some(document_owner),
        "test must reserve a real V8-backed reaction before replacement"
    );
    assert!(vm.has_page_module_reaction_for_executor_test());

    vm.eval("document.querySelector('iframe').srcdoc = '<p>replacement</p>'; 'queued'")
        .expect("child replacement should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "child replacement should commit through NavigationCommit"
    );

    assert_eq!(
        vm.document_runtime
            .native_dynamic_module_evaluation_reaction_owner(reaction_id),
        None,
        "owner transaction must drop the retired reaction's V8 context, resolver, and module"
    );
    assert!(
        vm.has_page_module_reaction_for_executor_test(),
        "stable Page source must retain the old exact-owner reaction until arbitration"
    );
    assert_eq!(
        vm.run_page_module_reaction_body_for_test()
            .expect("stale reaction arbitration should succeed"),
        Some(crate::page_task_queue::PageModuleReactionTargetEffect::IgnoredStaleOwner),
        "replacement must discard the queued reaction without applying it to the new realm"
    );
    assert!(!vm.has_page_module_reaction_for_executor_test());
    assert!(
        !vm.dynamic_module_import_owner_is_current(document_owner),
        "navigation must retire the reaction's exact child execution context"
    );
}

#[test]
fn dynamic_import_body_commits_module_before_task_end_document_open_reentry() {
    let mut vm = new_test_vm("https://dynamic-reentry.test/page.html");
    let original_owner = vm
        .current_main_document_task_owner()
        .expect("dynamic reentry test must have a main document owner");
    vm.eval(
        r#"
globalThis.__dynamicEvaluationGate = new Promise(resolve => {
  globalThis.__resolveDynamicEvaluationGate = resolve;
});
import('./dynamic.mjs').then(() => {
  document.open();
  document.write('<p id="replacement">replacement</p>');
  document.close();
});
'queued'
"#,
    )
    .expect("dynamic import should queue");
    assert!(
        matches!(
            vm.run_next_native_dynamic_module_owner_action_selected_task_body(),
            MainNativeModuleSelectedTaskApplication::Applied(_)
        ),
        "dynamic import graph should start a root fetch through the selected body"
    );
    let target = vm
        .current_main_dynamic_import_graph_fetch_target(0)
        .expect("dynamic TLA import must expose its exact resolver target");
    let completion = dynamic_import_completion_with_source(
        0,
        "https://dynamic-reentry.test/dynamic.mjs",
        "export const value = await globalThis.__dynamicEvaluationGate;",
    );
    vm.complete_current_main_dynamic_import_graph_fetch_result(target, completion.result)
        .expect("dynamic TLA graph completion should start evaluation");
    assert_eq!(
        vm.current_main_document_task_owner(),
        Some(original_owner),
        "pending TLA must not replace the document before fulfillment"
    );

    vm.eval("globalThis.__resolveDynamicEvaluationGate(); 'resolved'")
        .expect("dynamic TLA gate should resolve");
    assert!(
        vm.has_page_module_reaction_for_executor_test(),
        "TLA fulfillment should queue the registered dynamic-import reaction"
    );
    assert_eq!(
            vm.run_page_module_reaction_body_for_test()
                .expect("dynamic import fulfillment body should run"),
            Some(
                crate::page_task_queue::PageModuleReactionTargetEffect::AppliedToCurrentOwner(
                    crate::page_task_queue::PageModuleReactionCurrentEffect::DynamicImportPromiseSettled,
                )
            ),
            "fulfilled reaction body should settle one exact import Promise"
        );
    assert_eq!(
        vm.current_main_document_task_owner(),
        Some(original_owner),
        "the body must leave user Promise reactions for selected-task completion"
    );
    vm.perform_script_task_checkpoint(None)
        .expect("selected-task checkpoint should run");
    assert_ne!(
        vm.current_main_document_task_owner(),
        Some(original_owner),
        "the task-end checkpoint should run the user reaction and replace the document"
    );
}

#[test]
fn selected_dynamic_import_rejection_body_defers_user_reaction_to_task_end() {
    let mut vm = new_test_vm("https://dynamic-rejection-body.test/page.html");
    let context_host = vm._context_host.clone();
    let request = vm
        .renderer_document_isolate
        .with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, &vm.page_default_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let owner = context_host
                .borrow()
                .current_dynamic_module_import_owner(scope, None)
                .expect("test ScriptVm must have a current dynamic-import owner");
            let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
            let promise = resolver.get_promise(scope);
            let global = scope.get_current_context().global(scope);
            assert_eq!(
                global.set(
                    scope,
                    v8str(scope, "__selectedDynamicImportPromise").into(),
                    promise.into(),
                ),
                Some(true),
            );
            Ok(PendingDynamicModuleImport::new(
                v8::Global::new(scope, scope.get_current_context()),
                v8::Global::new(scope, resolver),
                owner,
                "./failed.mjs",
                Url::parse("https://dynamic-rejection-body.test/page.html").expect("base URL"),
                ModuleAttributesKey::empty(),
                ModuleImportPhase::Evaluation,
            ))
        })
        .expect("selected rejection request should be created");
    vm.eval_without_microtask_checkpoint_for_test(
        r#"
globalThis.__selectedDynamicImportReactions = [];
__selectedDynamicImportPromise.catch(() => {
  __selectedDynamicImportReactions.push("rejected");
});
"attached"
"#,
    )
    .expect("rejection observer should attach without running a checkpoint");

    vm.reject_native_dynamic_module_import_with_error_selected_task_body(
        request,
        &ModuleLoadError::new(ModuleLoadStage::Fetch, "forced selected-task failure"),
    )
    .expect("selected rejection body should settle the exact Promise");
    assert_eq!(
            vm.eval_without_microtask_checkpoint_for_test(
                "__selectedDynamicImportReactions.join('|')",
            )
            .expect("body-only observation should succeed"),
            "",
            "the rejection body must not run the user Promise reaction"
        );

    vm.perform_script_task_checkpoint(None)
        .expect("selected-task checkpoint should run");
    assert_eq!(
            vm.eval_without_microtask_checkpoint_for_test(
                "__selectedDynamicImportReactions.join('|')",
            )
            .expect("task-end observation should succeed"),
            "rejected",
            "the task-end checkpoint must run the deferred user reaction"
        );
}

#[test]
fn dynamic_import_owner_action_survives_main_document_open_for_rejection() {
    let mut vm = new_test_vm("https://dynamic-action-owner.test/page.html");
    let base_url = Url::parse("https://dynamic-action-owner.test/page.html").expect("base URL");
    let request = dynamic_import_request_in_vm(
        &mut vm,
        "./failed.mjs",
        base_url,
        ModuleImportPhase::Evaluation,
    );
    let retired_owner = request.owner();

    vm.eval("document.open(); 'replaced'")
        .expect("document.open should rotate the main document owner");
    assert!(
        vm.dynamic_module_import_owner_is_current(retired_owner),
        "document.open must preserve the request's execution-context owner"
    );

    let outcome = vm
        .run_main_dynamic_import_owner_action_with_body(
            FrameDocumentDynamicImportOwnerAction::fetch_failed(
                request,
                ModuleLoadError::new(ModuleLoadStage::Fetch, "forced stale fetch failure"),
            ),
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
        .expect("stale dynamic import rejection should be consumed");

    assert!(outcome.made_progress());
    assert!(!outcome.stale_owner_was_dropped());
    assert!(
        outcome.dynamic_import_was_rejected(),
        "same-execution-context replacement must settle the original resolver"
    );
}

#[test]
fn child_dynamic_import_resume_pending_job_reports_followup_progress() {
    let mut vm = new_test_vm("https://child-dynamic-resume.test/page.html");
    let job = {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let request = PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            DynamicModuleImportOwner::main_for_test(),
            "./dynamic-resume.js",
            Url::parse("https://child-dynamic-resume.test/page.html").expect("base URL"),
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        );
        NativeModuleGraphJob::dynamic_import(request)
    };

    let followup = vm.apply_child_dynamic_import_followup(
        FrameDocumentDynamicImportGraphAdvanceFollowup::ResumePendingJob(
            FrameDocumentDynamicImportPendingJobResume::new(job),
        ),
    );

    assert!(followup.made_progress());
    assert!(followup.dynamic_import_job_was_resumed());
    assert!(
        vm.document_runtime
            .take_next_native_dynamic_module_import()
            .is_some(),
        "ResumePendingJob should push the dynamic import job back to the runtime queue"
    );
}

#[test]
fn child_dynamic_import_missing_joined_fetch_followup_reports_warning_progress() {
    let mut vm = new_test_vm("https://child-dynamic-missing-fetch.test/page.html");

    let followup = vm.apply_child_dynamic_import_followup(
        FrameDocumentDynamicImportGraphAdvanceFollowup::RecordMissingJoinedTerminalFetch(
            FrameDocumentDynamicImportMissingJoinedTerminalFetch::new(
                FrameDocumentOwner::new(LocalWindowId(2), DocumentId(3)),
                FrameRealmId(4),
                55,
            ),
        ),
    );

    assert!(followup.made_progress());
    assert!(followup.terminal_warning_was_recorded());
    assert!(
        vm.runtime_observable_lifecycle_errors_for_testing()
            .iter()
            .any(|message| message
                .contains("child dynamic import fetch finish 55 for FrameDocumentOwner")),
        "missing joined fetch follow-up should record a diagnostic warning"
    );
}

#[tokio::test]
async fn child_dynamic_import_need_fetches_queues_waiting_owner_action() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        Url::parse("https://parent-dynamic-owner.test/page.html").expect("page URL"),
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-owner.test/nested/frame.html">
    <script>parent.__childDynamicImportOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import owner setup should evaluate");
    commit_child_document_and_run_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import waiting setup frame",
    )
    .await;
    finish_child_document_after_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import waiting setup frame",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;
    let child_initiator_url = vm
        .child_browsing_context_module_request_initiator_url(child_handle)
        .expect("child document should expose a module request initiator URL");
    let base_url =
        Url::parse("https://child-dynamic-owner.test/nested/frame.html").expect("base URL");
    let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
        base_url.join("dynamic-root.js").expect("dynamic URL"),
        base_url,
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
    );
    let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
    let job = {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let request = PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            document_owner,
            "./dynamic-root.js",
            child_initiator_url,
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        );
        NativeModuleGraphJob::dynamic_import(request)
    };

    vm.continue_child_native_dynamic_module_import_after_tree_advance(
        job,
        NativeModuleGraphJobAdvance::NeedFetches(vec![fetch_request]),
    )
    .expect("child dynamic import waiting action should queue graph work");

    let task = vm
            .take_dynamic_import_owner_action_for_routing_test()
            .unwrap_or_else(|| {
                panic!(
                    "child dynamic import should queue a dynamic import owner action follow-up; warnings: {:?}",
                    vm.runtime_observable_lifecycle_errors_for_testing()
                )
            });
    assert!(
        matches!(
            task.action(),
            crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Waiting { .. }
        ),
        "NeedFetches should queue Waiting as a later dynamic-import owner action"
    );
}

#[tokio::test]
async fn native_dynamic_terminal_fanout_reports_child_ready_followup() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        Url::parse("https://parent-dynamic-generic-ready.test/page.html").expect("page URL"),
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-generic-ready.test/nested/frame.html">
    <script>parent.__childDynamicImportGenericReadyOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import generic ready setup should evaluate");
    commit_child_document_and_run_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import generic ready setup frame",
    )
    .await;
    finish_child_document_after_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import generic ready setup frame",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;
    let child_initiator_url = vm
        .child_browsing_context_module_request_initiator_url(child_handle)
        .expect("child document should expose a module request initiator URL");
    let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);

    let job = {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let request = PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            document_owner,
            "./dynamic-generic-complete.js",
            child_initiator_url,
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        );
        NativeModuleGraphJob::dynamic_import(request)
    };
    let graph = ModuleGraphHandle {
        root_entry: ModuleEntryId::for_test(42),
        entries: vec![ModuleEntryId::for_test(42)],
    };
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady { job, graph });

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("generic child dynamic import fanout should queue owner action");
    let followup = outcome.child_followup();
    assert_eq!(
        outcome.ready_import_count(),
        1,
        "fanout outcome should expose that a ready dynamic import was handled"
    );
    assert!(
        followup.dynamic_import_owner_action_was_queued(),
        "fanout should report the child dynamic-import owner action follow-up"
    );

    let task = vm
            .take_dynamic_import_owner_action_for_routing_test()
            .unwrap_or_else(|| {
                panic!(
                    "generic child dynamic import completion should queue a ready owner action; warnings: {:?}",
                    vm.runtime_observable_lifecycle_errors_for_testing()
                )
            });
    assert!(
        matches!(
            task.action(),
            crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Ready(_)
        ),
        "generic child Complete fallback must queue Ready instead of settling inline"
    );
}

#[tokio::test]
async fn native_dynamic_terminal_fanout_does_not_rebind_to_replacement_child_owner() {
    let mut vm = new_test_vm("https://parent-dynamic-stale-ready.test/page.html");
    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = "<script>parent.__staleDynamicImportChildReady = true;<\/script>";
  body.appendChild(frame);
})()
"#,
    )
    .expect("stale child dynamic import setup should evaluate");
    commit_child_document_and_run_parser_script_for_dynamic_import_test(
        &mut vm,
        "original stale dynamic import child",
    )
    .await;
    finish_child_document_after_parser_script_for_dynamic_import_test(
        &mut vm,
        "original stale dynamic import child",
    )
    .await;
    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("stale dynamic import test should create a child realm");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("stale dynamic import child realm record should exist")
        .child_handle;
    let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);
    let base_url =
        Url::parse("https://child-dynamic-stale-ready.test/nested/frame.html").expect("url");

    let job = {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let request = PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            document_owner,
            "./dynamic-stale-complete.js",
            base_url,
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        );
        NativeModuleGraphJob::dynamic_import(request)
    };
    vm.eval("document.querySelector('iframe').srcdoc = '<p>replacement child</p>'; 'queued'")
        .expect("replacement child navigation should queue");
    assert_eq!(
        vm.run_next_child_frame_semantic_turn_for_test().await,
        Some(ChildFrameSemanticTurnKind::NavigationCommit),
        "child replacement must rotate the exact document owner before terminal fanout"
    );
    let (_captured_handle, retired_task_owner, _retired_realm_id) = document_owner
        .child_parts()
        .expect("test request must retain its child owner");
    let replacement_task_owner = vm
        ._context_host
        .borrow()
        .current_child_document_task_owner(child_handle)
        .expect("NavigationCommit should install the replacement document owner");
    assert_ne!(
        replacement_task_owner, retired_task_owner,
        "replacement must preserve the frame handle while changing document identity"
    );
    let graph = ModuleGraphHandle {
        root_entry: ModuleEntryId::for_test(77),
        entries: vec![ModuleEntryId::for_test(77)],
    };
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady { job, graph });

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("retired child dynamic import ready fanout should be handled");
    let followup = outcome.child_followup();

    assert_eq!(
        outcome.ready_import_count(),
        1,
        "fanout outcome should expose that the retired ready import was consumed"
    );
    assert!(
        !followup.made_progress(),
        "stale work must not synthesize a child task-source wake"
    );
    assert!(
        !followup.dynamic_import_owner_action_was_queued(),
        "retired child work must not be rebound to the replacement document owner"
    );
    assert!(
        vm.runtime_observable_lifecycle_errors_for_testing()
            .iter()
            .any(|warning| {
                warning.contains("child dynamic import continuation")
                    && warning.contains("owner is no longer current")
            }),
        "stale exact-owner drop should remain diagnosable without a synthetic wake"
    );
}

#[tokio::test]
async fn child_dynamic_import_graph_error_queues_failed_owner_action() {
    let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
        Url::parse("https://parent-dynamic-failed.test/page.html").expect("page URL"),
        &loader,
    );

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  frame.srcdoc = `
    <base href="https://child-dynamic-failed.test/nested/frame.html">
    <script>parent.__childDynamicImportFailedOwnerReady = true;<\/script>
  `;
  body.appendChild(frame);
})()
"#,
    )
    .expect("child dynamic import failed owner setup should evaluate");
    commit_child_document_and_run_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import failed setup frame",
    )
    .await;
    finish_child_document_after_parser_script_for_page_executor_test(
        &mut vm,
        "child dynamic import failed setup frame",
    )
    .await;

    let child_context_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .map(|realm| realm.context_id)
        .next()
        .expect("child default execution context should be created");
    let child_handle = vm
        .child_frame_realm_store
        .get(&child_context_id)
        .expect("child realm record should exist")
        .child_handle;
    let child_initiator_url = vm
        .child_browsing_context_module_request_initiator_url(child_handle)
        .expect("child document should expose a module request initiator URL");
    let document_owner = current_child_dynamic_import_owner(&mut vm, child_handle);

    let job = {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let request = PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            document_owner,
            "./dynamic-failed.js",
            child_initiator_url,
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        );
        NativeModuleGraphJob::dynamic_import(request)
    };
    let error = ModuleLoadError::new(ModuleLoadStage::Resolve, "forced graph failure");

    let followup = vm
        .enqueue_child_dynamic_import_graph_advance_failed_owner_action(job, error)
        .expect("child dynamic import failed action should queue graph failure work");
    assert!(
        followup.dynamic_import_owner_action_was_queued(),
        "graph failure should report the queued child dynamic-import owner action"
    );

    let task = vm
        .take_dynamic_import_owner_action_for_routing_test()
        .unwrap_or_else(|| {
            panic!(
                "child dynamic import should queue a failed owner action follow-up; warnings: {:?}",
                vm.runtime_observable_lifecycle_errors_for_testing()
            )
        });
    assert!(
        matches!(
            task.action(),
            crate::frame_owner_model::FrameDocumentDynamicImportOwnerAction::Reject(_)
        ),
        "graph errors should queue Reject as a later dynamic-import owner action"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_scheduled_fetch_effects() {
    let mut vm = new_test_vm("https://app-dynamic-schedule.test/page.html");
    let base_url = Url::parse("https://app-dynamic-schedule.test/page.html").expect("base URL");
    let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
        base_url
            .join("dynamic-dependency.js")
            .expect("dynamic dependency URL"),
        base_url.clone(),
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
    );
    let job = dynamic_import_job_in_vm(
        &mut vm,
        "./dynamic-dependency.js",
        base_url,
        ModuleImportPhase::Evaluation,
    );
    let scheduled = vm
        .document_runtime
        .suspend_native_dynamic_module_import_fetches(
            vec![fetch_request],
            Vec::new(),
            job,
            vec![None],
        );
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.extend_scheduled_dynamic_import_fetches(scheduled);

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("native dynamic import fanout should schedule fetches");

    assert_eq!(
        outcome.scheduled_dynamic_import_fetch_count(),
        1,
        "fanout outcome should expose concrete fetch scheduling progress"
    );
    assert!(
        !outcome.child_followup().made_progress(),
        "main dynamic fetch scheduling must not synthesize child frame follow-up work"
    );
}

#[test]
fn native_dynamic_waiting_without_joined_clients_reports_job_resume_effect() {
    let mut vm = new_test_vm("https://app-dynamic-resume.test/page.html");
    let base_url = Url::parse("https://app-dynamic-resume.test/page.html").expect("base URL");
    let job = dynamic_import_job_in_vm(
        &mut vm,
        "./dynamic-resume.js",
        base_url,
        ModuleImportPhase::Evaluation,
    );

    let outcome = vm
        .continue_main_native_dynamic_module_import_after_tree_advance_with_body(
            job,
            NativeModuleGraphJobAdvance::WaitingForFetches,
            &mut ScriptVmCheckpointingMainNativeModuleTaskBody,
        )
        .expect("main dynamic waiting advance should resume the pending job");

    assert_eq!(
        outcome.dynamic_import_job_resumed_count(),
        1,
        "main WaitingForFetches without joined clients should expose job-resume progress"
    );
    assert!(
        !outcome.child_followup().made_progress(),
        "main dynamic job resume must not synthesize child frame follow-up work"
    );
    assert!(
        vm.document_runtime
            .take_next_native_dynamic_module_import()
            .is_some(),
        "the resumed job should be available on the runtime dynamic-import queue"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_failed_fetch_rejection_effect() {
    let mut vm = new_test_vm("https://app-dynamic-fetch-failure.test/page.html");
    let base_url =
        Url::parse("https://app-dynamic-fetch-failure.test/page.html").expect("base URL");
    let request = dynamic_import_request_in_vm(
        &mut vm,
        "./dynamic-fetch-failure.js",
        base_url,
        ModuleImportPhase::Evaluation,
    );
    let failure = DynamicModuleFetchFailure::for_test(
        request,
        ModuleLoadError::new(ModuleLoadStage::Fetch, "forced dynamic fetch failure"),
    );
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_failed_fetch(failure);

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("failed dynamic fetch fanout should reject the import");

    assert_eq!(outcome.failed_fetch_rejected_count(), 1);
    assert!(
        !outcome.child_followup().made_progress(),
        "main dynamic fetch failure must not synthesize child frame follow-up work"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_graph_advance_rejection_effect() {
    let mut vm = new_test_vm("https://app-dynamic-graph-failure.test/page.html");
    let base_url =
        Url::parse("https://app-dynamic-graph-failure.test/page.html").expect("base URL");
    let job = dynamic_import_job_in_vm(
        &mut vm,
        "./dynamic-graph-failure.js",
        base_url,
        ModuleImportPhase::Evaluation,
    );
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_graph_advance_failure(
        job,
        ModuleLoadError::new(ModuleLoadStage::Resolve, "forced dynamic graph failure"),
    );

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("graph failure fanout should reject the import");

    assert_eq!(outcome.graph_advance_failure_handled_count(), 1);
    assert!(
        !outcome.child_followup().made_progress(),
        "main dynamic graph failure must not synthesize child frame follow-up work"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_source_import_rejection_effect() {
    let mut vm = new_test_vm("https://app-dynamic-source.test/page.html");
    let base_url = Url::parse("https://app-dynamic-source.test/page.html").expect("base URL");
    let module_key = ModuleMapKey::java_script(
        base_url
            .join("dynamic-source.js")
            .expect("dynamic source URL"),
    );
    let root_entry = vm.document_runtime.insert_native_module_source(
        module_key,
        ModuleSource::text("export const value = 1;".to_owned()),
    );
    let job = dynamic_import_job_in_vm(
        &mut vm,
        "./dynamic-source.js",
        base_url,
        ModuleImportPhase::Source,
    );
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
        job,
        graph: ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    });

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("source-phase dynamic import fanout should reject non-Wasm source import");

    assert_eq!(outcome.ready_import_count(), 1);
    assert_eq!(outcome.source_import_rejected_count(), 1);
    assert!(
        !outcome.child_followup().made_progress(),
        "main source-phase dynamic import rejection must not synthesize child follow-up work"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_evaluation_rejection_effect() {
    let mut vm = new_test_vm("https://app-dynamic-eval.test/page.html");
    let base_url = Url::parse("https://app-dynamic-eval.test/page.html").expect("base URL");
    let module_key = ModuleMapKey::java_script(
        base_url
            .join("dynamic-eval.js")
            .expect("dynamic evaluation URL"),
    );
    let root_entry = vm.document_runtime.insert_native_module_source(
        module_key,
        ModuleSource::text("export const value = 1;".to_owned()),
    );
    let job = dynamic_import_job_in_vm(
        &mut vm,
        "./dynamic-eval.js",
        base_url,
        ModuleImportPhase::Evaluation,
    );
    let mut fanout = NativeDynamicModuleTerminalFanout::default();
    fanout.push_ready_import(crate::module_runtime::NativeDynamicModuleImportReady {
        job,
        graph: ModuleGraphHandle {
            root_entry,
            entries: vec![root_entry],
        },
    });

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("evaluation dynamic import fanout should reject a non-ready root");

    assert_eq!(outcome.ready_import_count(), 1);
    assert_eq!(outcome.evaluation_import_rejected_count(), 1);
    assert!(
        !outcome.child_followup().made_progress(),
        "main evaluation dynamic import rejection must not synthesize child follow-up work"
    );
}

#[test]
fn native_dynamic_terminal_fanout_reports_unexpected_complete_warning_followup() {
    let mut vm = new_test_vm("https://app-dynamic-warning.test/page.html");
    let fanout = NativeDynamicModuleTerminalFanout::from_owner_advance(
        DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete,
    );

    let outcome = vm
        .handle_dynamic_module_terminal_fanout_for_owner(fanout)
        .expect("native dynamic import fanout should record unexpected-complete warning");
    let followup = outcome.child_followup();

    assert!(followup.made_progress());
    assert!(followup.terminal_warning_was_recorded());
    assert!(
        !followup.dynamic_import_owner_action_was_queued(),
        "warning-only fanout must not enqueue a dynamic-import owner action"
    );
}

#[test]
fn preserve_current_exception_keeps_v8_exception_message() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let try_catch = pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();

    let message = v8::String::new(&scope, "original syntax failure").expect("v8 string allocation");
    let exception = v8::Exception::syntax_error(&scope, message);
    scope.throw_exception(exception);
    assert!(preserve_current_v8_module_exception(&mut scope).is_none());

    let exception = scope
        .exception()
        .expect("original exception should still be pending");
    let message = exception
        .to_string(&scope)
        .expect("exception should stringify")
        .to_rust_string_lossy(&scope);
    assert!(message.contains("original syntax failure"));
}

#[test]
fn wasm_compile_with_options_preserves_v8_compile_exception() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let truncated_wasm_module = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01];

    let module = v8::WasmModuleObject::compile_with_options(
        &scope,
        &truncated_wasm_module,
        v8::WasmCompileOptions {
            js_string_builtins: true,
            imported_string_constants_module: None,
        },
    );

    assert!(module.is_none());
    let exception = scope
        .exception()
        .expect("wasm compile exception should be rethrown to caller scope");
    let message = exception
        .to_string(&scope)
        .expect("wasm compile exception should stringify")
        .to_rust_string_lossy(&scope);
    assert!(message.contains("CompileError"), "{message}");
    assert!(message.contains("WebAssembly.Module"), "{message}");
    assert!(!message.contains("unknown wasm compile exception"));
}

#[test]
fn wasm_compile_with_options_does_not_use_page_webassembly_constructor() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let patch_source = v8str(
        scope,
        r#"
            WebAssembly.Module = function() {
                throw new Error("patched WebAssembly.Module should not be used");
            };
            "#,
    );
    let patch_script =
        v8::Script::compile(scope, patch_source, None).expect("patch script should compile");
    crate::script_execution::execute_compiled_script(scope, patch_script)
        .expect("patch script should run");

    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let truncated_wasm_module = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01];

    let module = v8::WasmModuleObject::compile_with_options(
        &scope,
        &truncated_wasm_module,
        v8::WasmCompileOptions {
            js_string_builtins: true,
            imported_string_constants_module: None,
        },
    );

    assert!(module.is_none());
    let exception = scope
        .exception()
        .expect("wasm compile exception should be pending");
    let message = exception
        .to_string(&scope)
        .expect("wasm compile exception should stringify")
        .to_rust_string_lossy(&scope);
    assert!(message.contains("CompileError"), "{message}");
    assert!(
        !message.contains("patched WebAssembly.Module should not be used"),
        "{message}"
    );
}
