use std::sync::Arc;

use crate::{
    frame_owner_model::ChildDocumentModuleFetchTarget,
    module_runtime::{ModuleGraphFetchedSource, NativeModuleGraphFetchRequest},
    network::context::DocumentResourceLoader,
    page_resource_completion::{
        MainDynamicImportGraphFetchCompletion, MainDynamicImportGraphFetchTarget,
        MainModuleFetchNetworkAttribution, MainModulepreloadFetchCompletion,
        MainModulepreloadFetchTarget, MainParserModuleGraphFetchCompletion,
        MainParserModuleGraphFetchTarget, MainRuntimeModuleGraphFetchCompletion,
        MainRuntimeModuleGraphFetchTarget,
    },
    page_task_queue::RendererResourceCompletionSender,
    types::{
        ChildDynamicImportFetchCompletion, ChildModuleFetchNetworkAttribution,
        ChildModulepreloadFetchCompletion, ModuleGraphFetchOrdering, ModuleGraphFetchRequester,
        SharedNavigationResponseResult,
    },
};
use moli_fetch::ScriptFetchSchedulerPriority;

/// Type-safe producer route for main-Document module network terminals.
///
/// The fetch transport is shared, while executable currentness is deliberately
/// lane-specific. Keeping the target variants here avoids closure-based
/// wrap/unwrap adapters and makes it impossible to send a parser target through
/// the runtime or modulepreload completion route.
#[derive(Clone, Copy)]
enum MainModuleFetchSchedule {
    Parser(MainParserModuleGraphFetchTarget),
    Runtime(MainRuntimeModuleGraphFetchTarget),
    DynamicImport(MainDynamicImportGraphFetchTarget),
    Modulepreload(MainModulepreloadFetchTarget),
}

impl MainModuleFetchSchedule {
    fn load_id(self) -> u64 {
        match self {
            Self::Parser(target) => target.load_id(),
            Self::Runtime(target) => target.load_id(),
            Self::DynamicImport(target) => target.load_id(),
            Self::Modulepreload(target) => target.load_id(),
        }
    }

    fn requester(self) -> ModuleGraphFetchRequester {
        match self {
            Self::Parser(_) => ModuleGraphFetchRequester::ParserOwnedModuleScript,
            Self::Runtime(_) => ModuleGraphFetchRequester::RuntimeOwnedModuleScript,
            Self::DynamicImport(_) => ModuleGraphFetchRequester::DynamicImport,
            Self::Modulepreload(_) => ModuleGraphFetchRequester::ModulePreload,
        }
    }

    fn ordering(self) -> ModuleGraphFetchOrdering {
        match self {
            Self::Parser(_) => ModuleGraphFetchOrdering::DclCritical,
            Self::Runtime(_) => ModuleGraphFetchOrdering::Runtime,
            Self::DynamicImport(_) => ModuleGraphFetchOrdering::Runtime,
            Self::Modulepreload(_) => ModuleGraphFetchOrdering::BackgroundPreload,
        }
    }

    fn send_completion(
        self,
        completion_tx: &RendererResourceCompletionSender,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
        network_result: Option<SharedNavigationResponseResult>,
        network_attribution: MainModuleFetchNetworkAttribution,
    ) {
        match self {
            Self::Parser(target) => {
                let _ = completion_tx.send_main_parser_module_graph_fetch(
                    MainParserModuleGraphFetchCompletion::new(
                        target,
                        result,
                        network_result,
                        network_attribution,
                    ),
                );
            }
            Self::Runtime(target) => {
                let _ = completion_tx.send_main_runtime_module_graph_fetch(
                    MainRuntimeModuleGraphFetchCompletion::new(
                        target,
                        result,
                        network_result,
                        network_attribution,
                    ),
                );
            }
            Self::DynamicImport(target) => {
                let _ = completion_tx.send_main_dynamic_import_graph_fetch(
                    MainDynamicImportGraphFetchCompletion::new(
                        target,
                        result,
                        network_result,
                        network_attribution,
                    ),
                );
            }
            Self::Modulepreload(target) => {
                let _ = completion_tx.send_main_modulepreload_fetch(
                    MainModulepreloadFetchCompletion::new(
                        target,
                        result,
                        network_result,
                        network_attribution,
                    ),
                );
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct RendererResourceScheduler {
    completion_tx: RendererResourceCompletionSender,
}

impl RendererResourceScheduler {
    pub(crate) fn new(completion_tx: RendererResourceCompletionSender) -> Self {
        Self { completion_tx }
    }

    pub(crate) fn schedule_main_parser_module_graph_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: MainParserModuleGraphFetchTarget,
        request: NativeModuleGraphFetchRequest,
        document_url: url::Url,
    ) {
        self.schedule_main_module_fetch(
            loader,
            MainModuleFetchSchedule::Parser(target),
            request,
            document_url,
        );
    }

    pub(crate) fn schedule_main_runtime_module_graph_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: MainRuntimeModuleGraphFetchTarget,
        request: NativeModuleGraphFetchRequest,
        document_url: url::Url,
    ) {
        self.schedule_main_module_fetch(
            loader,
            MainModuleFetchSchedule::Runtime(target),
            request,
            document_url,
        );
    }

    pub(crate) fn schedule_main_runtime_module_root_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: MainRuntimeModuleGraphFetchTarget,
        script: crate::planning::PreparedScript,
        service_worker: crate::dynamic_script_owner::DynamicScriptServiceWorkerContext,
    ) {
        let schedule = MainModuleFetchSchedule::Runtime(target);
        let attribution = MainModuleFetchNetworkAttribution::new(
            service_worker.document_url.clone(),
            script.url.clone(),
        );
        let Some(load) = loader.register_load(
            crate::network::loads::ResourceLoadKind::Script,
            crate::network::loads::ResourceLoadDisposition::Ordinary,
            None,
        ) else {
            let error = "Document detached before module root fetch registration".to_owned();
            schedule.send_completion(
                &self.completion_tx,
                Err(error.clone()),
                Some(Arc::new(Err(error))),
                attribution,
            );
            return;
        };
        let completion_tx = self.completion_tx.clone();
        let request_client = load.request_client();
        let request_origin = loader.fetch_context().request_origin();
        let runner = load.task_runner();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        load.attach_consumer_cancel(move || {
            let _ = cancel_tx.send(());
        });
        runner.clone().spawn(async move {
            // This is the script-element transport previously used before map
            // lookup. Keep its Service Worker response filtering and SRI checks,
            // but publish the result to the graph's single-module reservation.
            let outcome = tokio::select! {
                outcome = crate::planning::load_service_worker_aware_external_script_source_outcome(
                    &script, &request_origin, &request_client, runner, None, None,
                    service_worker.browser_context_runtime, service_worker.client_id,
                    service_worker.document_url,
                ) => outcome,
                _ = cancel_rx => {
                    let error = "module root fetch cancelled".to_owned();
                    load.finish();
                    schedule.send_completion(
                        &completion_tx, Err(error.clone()), Some(Arc::new(Err(error))), attribution,
                    );
                    return;
                }
            };
            load.finish();
            let result = outcome.source_result.map(|source| {
                let response = outcome
                    .network_result
                    .as_deref()
                    .and_then(|result| result.as_ref().ok());
                let (url, redirected, referrer_policy) = response.map_or_else(
                    || (script.url.clone(), false, None),
                    |response| {
                        (
                            response.final_url.clone(),
                            response.redirected,
                            crate::referrer_policy::response_referrer_policy_from_headers(
                                &response.headers,
                            ),
                        )
                    },
                );
                ModuleGraphFetchedSource::new(
                    url,
                    redirected,
                    outcome
                        .source_bytes
                        .filter(|_| crate::planning::is_webassembly_module_script_url(&script.url))
                        .map_or_else(
                            || crate::module_runtime::ModuleSource::text(source),
                            crate::module_runtime::ModuleSource::binary,
                        ),
                )
                .with_response_referrer_policy(referrer_policy)
            });
            schedule.send_completion(&completion_tx, result, outcome.network_result, attribution);
        });
    }

    pub(crate) fn schedule_main_modulepreload_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: MainModulepreloadFetchTarget,
        request: NativeModuleGraphFetchRequest,
        document_url: url::Url,
    ) {
        self.schedule_main_module_fetch(
            loader,
            MainModuleFetchSchedule::Modulepreload(target),
            request,
            document_url,
        );
    }

    pub(crate) fn schedule_main_dynamic_import_graph_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: MainDynamicImportGraphFetchTarget,
        request: NativeModuleGraphFetchRequest,
        document_url: url::Url,
    ) {
        self.schedule_main_module_fetch(
            loader,
            MainModuleFetchSchedule::DynamicImport(target),
            request,
            document_url,
        );
    }

    fn schedule_main_module_fetch(
        &self,
        loader: DocumentResourceLoader,
        schedule: MainModuleFetchSchedule,
        request: NativeModuleGraphFetchRequest,
        document_url: url::Url,
    ) {
        let requester = schedule.requester();
        let ordering = schedule.ordering();
        let request = prioritized_module_graph_fetch_request(requester, ordering, request);
        let request_url = request.source_url().clone();
        let network_attribution =
            MainModuleFetchNetworkAttribution::new(document_url, request_url.clone());
        trace_module_graph_fetch_scheduled(schedule.load_id(), requester, ordering, &request_url);

        let completion_tx = self.completion_tx.clone();
        let callback_attribution = network_attribution.clone();
        let callback_request_url = request_url.clone();
        let send_completion =
            move |result: std::result::Result<ModuleGraphFetchedSource, String>,
                  network_result: Option<SharedNavigationResponseResult>| {
                trace_module_graph_fetch_callback(
                    schedule.load_id(),
                    requester,
                    ordering,
                    &callback_request_url,
                    result.is_ok(),
                );
                schedule.send_completion(
                    &completion_tx,
                    result,
                    network_result,
                    callback_attribution,
                );
            };
        if let Err(error) = request.fetch_source_for_document(&loader, send_completion) {
            let error = error.to_string();
            trace_module_graph_fetch_schedule_error(
                schedule.load_id(),
                requester,
                ordering,
                &request_url,
                &error,
            );
            schedule.send_completion(
                &self.completion_tx,
                Err(error.clone()),
                Some(Arc::new(Err(error))),
                network_attribution,
            );
        }
    }

    pub(crate) fn schedule_child_dynamic_module_graph_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: ChildDocumentModuleFetchTarget,
        load_id: u64,
        request: NativeModuleGraphFetchRequest,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) {
        let requester = ModuleGraphFetchRequester::DynamicImport;
        let ordering = ModuleGraphFetchOrdering::Runtime;
        let request = prioritized_module_graph_fetch_request(requester, ordering, request);
        let request_url = request.source_url().clone();
        trace_module_graph_fetch_scheduled(load_id, requester, ordering, &request_url);

        let completion_tx = self.completion_tx.clone();
        let callback_attribution = network_attribution.clone();
        let callback_request_url = request_url.clone();
        let send_completion =
            move |result: std::result::Result<ModuleGraphFetchedSource, String>,
                  network_result: Option<SharedNavigationResponseResult>| {
                trace_module_graph_fetch_callback(
                    load_id,
                    requester,
                    ordering,
                    &callback_request_url,
                    result.is_ok(),
                );
                let _ = completion_tx.send_child_dynamic_import_fetch(
                    ChildDynamicImportFetchCompletion::new(
                        target,
                        load_id,
                        result,
                        network_result,
                        callback_attribution,
                    ),
                );
            };
        if let Err(error) = request.fetch_source_for_document(&loader, send_completion) {
            let error = error.to_string();
            trace_module_graph_fetch_schedule_error(
                load_id,
                requester,
                ordering,
                &request_url,
                &error,
            );
            let _ = self.completion_tx.send_child_dynamic_import_fetch(
                ChildDynamicImportFetchCompletion::new(
                    target,
                    load_id,
                    Err(error.clone()),
                    Some(Arc::new(Err(error))),
                    network_attribution,
                ),
            );
        }
    }

    pub(crate) fn schedule_child_modulepreload_graph_fetch(
        &self,
        loader: DocumentResourceLoader,
        target: ChildDocumentModuleFetchTarget,
        load_id: u64,
        request: NativeModuleGraphFetchRequest,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) {
        let requester = ModuleGraphFetchRequester::ModulePreload;
        let ordering = ModuleGraphFetchOrdering::BackgroundPreload;
        let request = prioritized_module_graph_fetch_request(requester, ordering, request);
        let request_url = request.source_url().clone();
        trace_module_graph_fetch_scheduled(load_id, requester, ordering, &request_url);

        let completion_tx = self.completion_tx.clone();
        let callback_attribution = network_attribution.clone();
        let callback_request_url = request_url.clone();
        let send_completion =
            move |result: std::result::Result<ModuleGraphFetchedSource, String>,
                  network_result: Option<SharedNavigationResponseResult>| {
                trace_module_graph_fetch_callback(
                    load_id,
                    requester,
                    ordering,
                    &callback_request_url,
                    result.is_ok(),
                );
                let _ = completion_tx.send_child_modulepreload_fetch(
                    ChildModulepreloadFetchCompletion::new(
                        target,
                        load_id,
                        result,
                        network_result,
                        callback_attribution,
                    ),
                );
            };
        if let Err(error) = request.fetch_source_for_document(&loader, send_completion) {
            let error = error.to_string();
            trace_module_graph_fetch_schedule_error(
                load_id,
                requester,
                ordering,
                &request_url,
                &error,
            );
            let _ = self.completion_tx.send_child_modulepreload_fetch(
                ChildModulepreloadFetchCompletion::new(
                    target,
                    load_id,
                    Err(error.clone()),
                    Some(Arc::new(Err(error))),
                    network_attribution,
                ),
            );
        }
    }
}

fn trace_module_graph_fetch_scheduled(
    load_id: u64,
    requester: ModuleGraphFetchRequester,
    ordering: ModuleGraphFetchOrdering,
    request_url: &url::Url,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_graph_fetch_scheduled",
        load_id,
        requester = ?requester,
        ordering = ?ordering,
        url = %request_url,
    );
}

fn trace_module_graph_fetch_callback(
    load_id: u64,
    requester: ModuleGraphFetchRequester,
    ordering: ModuleGraphFetchOrdering,
    request_url: &url::Url,
    ok: bool,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_graph_fetch_callback",
        load_id,
        requester = ?requester,
        ordering = ?ordering,
        url = %request_url,
        ok,
    );
}

fn trace_module_graph_fetch_schedule_error(
    load_id: u64,
    requester: ModuleGraphFetchRequester,
    ordering: ModuleGraphFetchOrdering,
    request_url: &url::Url,
    error: &str,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_graph_fetch_schedule_error",
        load_id,
        requester = ?requester,
        ordering = ?ordering,
        url = %request_url,
        error,
    );
}

fn prioritized_module_graph_fetch_request(
    requester: ModuleGraphFetchRequester,
    ordering: ModuleGraphFetchOrdering,
    request: NativeModuleGraphFetchRequest,
) -> NativeModuleGraphFetchRequest {
    if requester == ModuleGraphFetchRequester::ParserOwnedModuleScript
        && ordering == ModuleGraphFetchOrdering::DclCritical
    {
        return request.with_scheduler_priority(ScriptFetchSchedulerPriority::VeryHigh);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_runtime::{ModuleFetchMetadata, ModuleKind};

    fn module_request() -> NativeModuleGraphFetchRequest {
        NativeModuleGraphFetchRequest::new_for_test(
            url::Url::parse("https://app.example.test/entry.mjs").expect("entry url"),
            url::Url::parse("https://app.example.test/page").expect("page url"),
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
        )
    }

    #[test]
    fn parser_owned_module_fetches_are_very_high_priority() {
        let request = prioritized_module_graph_fetch_request(
            ModuleGraphFetchRequester::ParserOwnedModuleScript,
            ModuleGraphFetchOrdering::DclCritical,
            module_request(),
        );
        assert_eq!(
            request.scheduler_priority_for_test(),
            Some(ScriptFetchSchedulerPriority::VeryHigh)
        );
    }

    #[test]
    fn non_parser_owned_module_fetch_priority_is_not_promoted() {
        let modulepreload = prioritized_module_graph_fetch_request(
            ModuleGraphFetchRequester::ModulePreload,
            ModuleGraphFetchOrdering::BackgroundPreload,
            module_request(),
        );
        assert_eq!(modulepreload.scheduler_priority_for_test(), None);

        let dynamic_import = prioritized_module_graph_fetch_request(
            ModuleGraphFetchRequester::DynamicImport,
            ModuleGraphFetchOrdering::Runtime,
            module_request(),
        );
        assert_eq!(dynamic_import.scheduler_priority_for_test(), None);
    }
}
