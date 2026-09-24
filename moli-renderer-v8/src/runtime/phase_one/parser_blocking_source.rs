use super::super::script_preloads::{
    AppliedPreloadedScriptSource, BufferedDocumentPreloadState, ParserBlockingPreloadDisposition,
};
use super::parser_blocking_owner::MainParserBlockingSourceResultOwner;
use super::parser_blocking_pending::{
    PendingParserBlockingSourceLoad, PendingParsingBlockingClassicScriptRunner,
};
use super::*;
use crate::content_security_policy::ContentSecurityPolicyScriptElementRequest;
use crate::parser_script::projection::ParserClassicScriptSourceResultApplication;
use crate::planning::{PreparedScript, ScriptFetchMetadata};

pub(super) struct MainParserBlockingSourceLoadDecision {
    pub(super) disposition: MainParserBlockingSourceDisposition,
    pub(super) applied_preload: Option<AppliedPreloadedScriptSource>,
}

pub(super) enum MainParserBlockingSourceDisposition {
    Ready,
    Pending(PendingParserBlockingSourceLoad),
    Suppressed,
}

pub(super) fn prepare_main_parser_blocking_source_load(
    page_vm: &mut PageVm,
    buffered_document_preloads: &mut BufferedDocumentPreloadState,
    script: &mut PreparedScript,
) -> MainParserBlockingSourceLoadDecision {
    if !matches!(script.source, crate::parser::ScriptSource::External) {
        return MainParserBlockingSourceLoadDecision {
            disposition: MainParserBlockingSourceDisposition::Ready,
            applied_preload: None,
        };
    }

    if !parser_blocking_script_can_start_external_source_load(page_vm, script) {
        return MainParserBlockingSourceLoadDecision {
            disposition: MainParserBlockingSourceDisposition::Suppressed,
            applied_preload: None,
        };
    }

    match buffered_document_preloads.parser_blocking_preload_disposition_for_script(script) {
        ParserBlockingPreloadDisposition::Ready(applied_preload) => {
            MainParserBlockingSourceLoadDecision {
                disposition: MainParserBlockingSourceDisposition::Ready,
                applied_preload: Some(applied_preload),
            }
        }
        ParserBlockingPreloadDisposition::ReusableSourceLoad(preload) => {
            arm_main_parser_source_load_continuation(page_vm, &preload);
            MainParserBlockingSourceLoadDecision {
                disposition: MainParserBlockingSourceDisposition::Pending(
                    PendingParserBlockingSourceLoad::ReusablePreload(preload),
                ),
                applied_preload: None,
            }
        }
        ParserBlockingPreloadDisposition::ExistingButNotReusable
        | ParserBlockingPreloadDisposition::Missing => {
            let document_character_set = page_vm
                .vm()
                .document_runtime
                .document_character_set()
                .to_owned();
            let source_load = spawn_parser_blocking_script_source_load(
                page_vm,
                script.clone(),
                document_character_set,
            );
            arm_main_parser_source_load_continuation(page_vm, &source_load);
            MainParserBlockingSourceLoadDecision {
                disposition: MainParserBlockingSourceDisposition::Pending(
                    PendingParserBlockingSourceLoad::ParserDiscovered(source_load),
                ),
                applied_preload: None,
            }
        }
    }
}

fn arm_main_parser_source_load_continuation(
    page_vm: &PageVm,
    load: &crate::planning::SharedScriptSourceLoad,
) {
    let producer = page_vm
        .vm()
        .document_runtime
        .main_parser_continuation_producer()
        .expect("parser-blocking source load requires an active parser continuation producer");
    load.register_completion_wake(move || {
        let _ = producer.request();
    });
}

pub(super) fn record_main_parser_blocking_applied_preload_network_result(
    page_vm: &mut PageVm,
    script: &PreparedScript,
    applied_preload: Option<&AppliedPreloadedScriptSource>,
) {
    if let Some(applied) = applied_preload
        && let Some(network_result) = applied.network_result.as_deref()
    {
        page_vm
            .vm_mut()
            .record_script_resource_timing(script.url.clone(), network_result);
    }
}

pub(super) fn apply_pending_parser_blocking_source_load_if_ready(
    page_vm: &mut PageVm,
    pending_runner: &mut PendingParsingBlockingClassicScriptRunner,
) -> bool {
    let mut owner = MainParserBlockingSourceResultOwner { page_vm };
    match pending_runner.apply_current_parser_blocking_source_result_if_ready_with_owner(&mut owner)
    {
        ParserClassicScriptSourceResultApplication::Applied(_)
        | ParserClassicScriptSourceResultApplication::NoSourceLoad => true,
        ParserClassicScriptSourceResultApplication::Waiting => false,
    }
}

pub(super) fn parser_blocking_script_can_start_external_source_load(
    page_vm: &PageVm,
    script: &PreparedScript,
) -> bool {
    if !page_vm.main_document_scripting_enabled() {
        return false;
    }
    if script.source_kind != crate::types::ScriptSourceKind::External {
        return false;
    }
    if page_vm
        .vm()
        .document_runtime
        .script_element_request_csp_violation_with_request(
            &script.url,
            parser_blocking_script_element_csp_request(&script.fetch_metadata),
        )
        .is_some()
    {
        return false;
    }
    true
}

fn parser_blocking_script_element_csp_request(
    fetch_metadata: &ScriptFetchMetadata,
) -> ContentSecurityPolicyScriptElementRequest<'_> {
    ContentSecurityPolicyScriptElementRequest {
        nonce: fetch_metadata.nonce.as_deref(),
        integrity: fetch_metadata.integrity.as_deref(),
        parser_inserted: true,
    }
}

fn spawn_parser_blocking_script_source_load(
    page_vm: &mut PageVm,
    script: PreparedScript,
    document_character_set: String,
) -> crate::planning::SharedScriptSourceLoad {
    let request_resource_type = moli_fetch::RequestResourceType::ParserBlockingScript;
    if page_vm
        .vm()
        .should_intercept_parser_script_source_fetch(&script)
    {
        let browser_context_runtime = page_vm.runtime_hooks.browser_context_runtime.clone();
        return page_vm
            .vm_mut()
            .start_parser_script_source_fetch_interception(
                script,
                browser_context_runtime,
                Some(document_character_set),
            );
    }
    let service_worker = page_vm
        .vm()
        .service_worker_subresource_fetch_context(&script.url);
    crate::planning::SharedScriptSourceLoad::spawn(
        script,
        page_vm.main_document_resource_loader(),
        Some(document_character_set),
        Some(request_resource_type),
        crate::types::SubresourceRequestInitiatorType::Parser,
        service_worker,
        None,
    )
}
