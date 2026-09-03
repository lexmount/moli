use std::collections::HashMap;

use moli_fetch::{
    BrowserRequestMetadata, Request, RequestCredentialsMode, RequestResourceType,
    ScriptFetchRequestMetadata, ScriptFetchSchedulerPriority,
};
use moli_module_script_tree as module_tree;
use url::Url;

use crate::module_script_continuation::ModuleScriptCompletionOwner;
use crate::network::ResourceRequestClient;
use crate::planning::ScriptFetchMetadata;
use crate::protocol_types::NavigationResponse;
use crate::script_vm::ScriptVm;
use crate::types::SharedNavigationResponseResult;
use crate::types::{ScriptErrorConstructorKind, ScriptKind};

use super::driver::next_inline_module_eval_id;
use super::tree_adapter;
use super::tree_job::{
    NativeModuleGraphDependencyRequest, NativeModuleTreeFetchRequest, NativeModuleTreeJob,
    NativeModuleTreeJobAdvance,
};
use super::tree_owner::{NativeModuleTreeDocumentOwner, NativeModuleTreeDocumentOwnerAdapter};
use super::{
    ModuleAttributesKey, ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource,
    ModuleImportPhase, ModuleKind, ModuleLoadError, ModuleLoadStage, ModuleMapEntryState,
    ModuleMapFetchDisposition, ModuleMapKey, ModuleRequestRecord, ModuleResolvedDependency,
    ModuleScriptExecutionOutcome, ModuleSource, PendingDynamicModuleImport,
};

const NATIVE_PARSER_MODULE_GRAPH_JOB: &str = "native parser module graph job";
const NATIVE_RUNTIME_MODULE_GRAPH_JOB: &str = "native runtime module graph job";
const NATIVE_DYNAMIC_IMPORT_GRAPH_JOB: &str = "native dynamic import graph job";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleRootInput {
    pub(crate) source_url: Url,
    pub(crate) base_url: Url,
    pub(crate) initiator_url: Url,
    pub(crate) attributes: ModuleAttributesKey,
    pub(crate) phase: ModuleImportPhase,
    pub(crate) source_override: Option<ModuleSource>,
    pub(crate) fetch_metadata: ModuleFetchMetadata,
    pub(crate) parser_owned: bool,
}

impl ModuleRootInput {
    fn with_import_map_integrity_if_absent(mut self, integrity: Option<String>) -> Self {
        self.fetch_metadata = self
            .fetch_metadata
            .with_import_map_integrity_if_absent(integrity);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleGraphHandle {
    pub(crate) root_entry: ModuleEntryId,
    pub(crate) entries: Vec<ModuleEntryId>,
}

pub(crate) struct NativeModuleGraphJob {
    tree_job: Option<NativeModuleTreeJob>,
    kind: NativeModuleGraphJobKind,
}

enum NativeModuleGraphJobKind {
    ParserOwned,
    RuntimeModuleScript,
    DynamicImport(Box<PendingDynamicModuleImport>),
}

pub(crate) enum NativeModuleGraphJobAdvance {
    NeedFetches(Vec<NativeModuleGraphFetchRequest>),
    WaitingForFetches,
    Complete(ModuleGraphHandle),
}

pub(crate) struct NativeDynamicModuleImportReady {
    pub(crate) job: NativeModuleGraphJob,
    pub(crate) graph: ModuleGraphHandle,
}

pub(crate) enum ModuleScriptGraphAdvance {
    NeedFetches(Box<ModuleScriptGraphFetchBatch>),
    Complete(ModuleGraphHandle),
}

#[derive(Clone)]
pub(crate) struct NativeModuleGraphFetchRequest {
    source_url: Url,
    initiator_url: Url,
    fetch_metadata: ModuleFetchMetadata,
    kind: ModuleKind,
    tree_client: Option<module_tree::SingleModuleClientToken>,
    tree_graph_level: Option<module_tree::ModuleGraphLevel>,
    module_key: Option<ModuleMapKey>,
    dependency: Option<NativeModuleGraphDependencyRequest>,
}

pub(crate) struct ModuleScriptGraphFetchContinuation {
    request: NativeModuleGraphFetchRequest,
}

pub(crate) struct ModuleScriptGraphFetchBatch {
    job: NativeModuleGraphJob,
    fetches: Vec<ModuleScriptGraphFetchContinuation>,
}

impl ModuleScriptGraphFetchContinuation {
    pub(crate) fn is_top_level_tree_fetch(&self) -> bool {
        self.request.is_top_level_tree_fetch()
    }

    #[cfg(test)]
    pub(crate) fn pending_fetch_key(&self) -> Option<&ModuleMapKey> {
        self.request.pending_fetch_key()
    }

    pub(crate) fn request(&self) -> &NativeModuleGraphFetchRequest {
        &self.request
    }

    pub(crate) fn finish_fetch_into_job(
        self,
        vm: &mut ScriptVm,
        job: &mut NativeModuleGraphJob,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        job.finish_module_script_fetch_for_request(vm, &self.request, source)
    }

    #[cfg(test)]
    pub(crate) fn finish_source_for_test(
        self,
        vm: &mut ScriptVm,
        job: &mut NativeModuleGraphJob,
        source: std::result::Result<ModuleSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let source = source.map(|source| {
            ModuleGraphFetchedSource::new(self.request.source_url.clone(), false, source)
        });
        self.finish_fetch_into_job(vm, job, source)
    }
}

impl ModuleScriptGraphFetchBatch {
    fn new(job: NativeModuleGraphJob, fetches: Vec<ModuleScriptGraphFetchContinuation>) -> Self {
        Self { job, fetches }
    }

    #[cfg(test)]
    pub(crate) fn request(&self) -> &NativeModuleGraphFetchRequest {
        assert_eq!(
            self.fetches.len(),
            1,
            "single-fetch batch helper requires exactly one fetch"
        );
        self.fetches
            .first()
            .expect("single-fetch batch should contain a fetch")
            .request()
    }

    #[cfg(test)]
    pub(crate) fn pending_fetch_key(&self) -> Option<&ModuleMapKey> {
        self.request().pending_fetch_key()
    }

    #[cfg(test)]
    pub(crate) fn finish_fetch(
        self,
        vm: &mut ScriptVm,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<ModuleScriptGraphAdvance, ModuleLoadError> {
        let (mut job, mut fetches) = self.into_parts();
        assert_eq!(
            fetches.len(),
            1,
            "single-fetch batch completion requires exactly one fetch"
        );
        let fetch = fetches
            .pop()
            .expect("single-fetch batch should contain a fetch");
        let advance = fetch.finish_fetch_into_job(vm, &mut job, source)?;
        Ok(module_script_graph_advance_from_native(job, advance))
    }

    #[cfg(test)]
    pub(crate) fn finish_source_for_test(
        self,
        vm: &mut ScriptVm,
        source: std::result::Result<ModuleSource, ModuleLoadError>,
    ) -> std::result::Result<ModuleScriptGraphAdvance, ModuleLoadError> {
        let source_url = self.request().source_url().clone();
        self.finish_fetch(
            vm,
            source.map(|source| ModuleGraphFetchedSource::new(source_url, false, source)),
        )
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        NativeModuleGraphJob,
        Vec<ModuleScriptGraphFetchContinuation>,
    ) {
        (self.job, self.fetches)
    }
}

impl NativeModuleGraphFetchRequest {
    pub(crate) fn new(
        source_url: Url,
        initiator_url: Url,
        fetch_metadata: ModuleFetchMetadata,
        kind: ModuleKind,
    ) -> Self {
        Self {
            source_url,
            initiator_url,
            fetch_metadata,
            kind,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
    }

    pub(crate) fn source_url(&self) -> &Url {
        &self.source_url
    }

    pub(crate) fn initiator_url(&self) -> &Url {
        &self.initiator_url
    }

    #[cfg(test)]
    pub(crate) fn initiator_url_for_test(&self) -> &Url {
        self.initiator_url()
    }

    pub(crate) fn nonce(&self) -> Option<&str> {
        self.fetch_metadata.nonce()
    }

    pub(crate) fn fetch_metadata(&self) -> &ModuleFetchMetadata {
        &self.fetch_metadata
    }

    pub(crate) fn pending_fetch_key(&self) -> Option<&ModuleMapKey> {
        self.module_key.as_ref()
    }

    pub(crate) fn tree_client(&self) -> Option<module_tree::SingleModuleClientToken> {
        self.tree_client
    }

    pub(crate) fn dependency(&self) -> Option<&NativeModuleGraphDependencyRequest> {
        self.dependency.as_ref()
    }

    pub(crate) fn effective_key_for_fetched_source(
        &self,
        fetched_source: &ModuleGraphFetchedSource,
    ) -> Option<ModuleMapKey> {
        self.pending_fetch_key()
            .map(|key| fetched_source.effective_key_for_request(key))
    }

    pub(crate) fn effective_fetch_metadata_for_fetched_source(
        &self,
        fetched_source: &ModuleGraphFetchedSource,
    ) -> ModuleFetchMetadata {
        self.fetch_metadata
            .clone()
            .with_response_referrer_policy(fetched_source.response_referrer_policy())
    }

    fn with_tree_fetch(
        mut self,
        client: module_tree::SingleModuleClientToken,
        graph_level: module_tree::ModuleGraphLevel,
        key: ModuleMapKey,
        dependency: Option<NativeModuleGraphDependencyRequest>,
    ) -> Self {
        self.tree_client = Some(client);
        self.tree_graph_level = Some(graph_level);
        self.module_key = Some(key);
        self.dependency = dependency;
        self
    }

    pub(crate) fn is_top_level_tree_fetch(&self) -> bool {
        self.tree_graph_level == Some(module_tree::ModuleGraphLevel::TopLevel)
    }

    pub(crate) fn with_scheduler_priority(
        mut self,
        priority: ScriptFetchSchedulerPriority,
    ) -> Self {
        self.fetch_metadata.request_metadata.scheduler_priority = Some(priority);
        self
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        source_url: Url,
        initiator_url: Url,
        fetch_metadata: ModuleFetchMetadata,
        kind: ModuleKind,
    ) -> Self {
        Self::new(source_url, initiator_url, fetch_metadata, kind)
    }

    #[cfg(test)]
    pub(crate) fn new_tree_dependency_for_test(
        source_url: Url,
        initiator_url: Url,
        fetch_metadata: ModuleFetchMetadata,
        kind: ModuleKind,
        client: module_tree::SingleModuleClientToken,
        key: ModuleMapKey,
        parent_key: ModuleMapKey,
        parent_entry_id: ModuleEntryId,
        specifier: String,
        phase: ModuleImportPhase,
    ) -> Self {
        let dependency =
            NativeModuleGraphDependencyRequest::new(parent_key, parent_entry_id, specifier, phase);
        Self::new(source_url, initiator_url, fetch_metadata, kind).with_tree_fetch(
            client,
            module_tree::ModuleGraphLevel::Dependent,
            key,
            Some(dependency),
        )
    }

    #[cfg(test)]
    pub(crate) fn scheduler_priority_for_test(&self) -> Option<ScriptFetchSchedulerPriority> {
        self.fetch_metadata.request_metadata.scheduler_priority
    }

    #[cfg(test)]
    pub(crate) fn integrity_for_test(&self) -> Option<&str> {
        self.fetch_metadata.request_metadata.integrity.as_deref()
    }

    fn request(&self) -> anyhow::Result<Request> {
        let request = Request::new("GET", self.source_url.as_str(), None, vec![])
            .expect("module graph URL should already be parsed")
            .with_page_network_policy()
            .with_initiator_url(&self.initiator_url)
            .with_credentials_mode(self.fetch_metadata.credentials_mode)
            .with_script_fetch_metadata(self.fetch_metadata.request_metadata.clone());
        Ok(match self.kind {
            ModuleKind::Css => request
                .with_resource_type(RequestResourceType::CssStyleSheet)
                .with_browser_request_metadata(BrowserRequestMetadata::StyleModule),
            ModuleKind::Json => {
                request.with_browser_request_metadata(BrowserRequestMetadata::JsonModule)
            }
            ModuleKind::JavaScript | ModuleKind::ModulePreloadText | ModuleKind::WebAssembly => {
                request
            }
        })
    }

    pub(crate) fn fetch_source_callback_with_load<F>(
        &self,
        loader: &ResourceRequestClient,
        load: crate::network::loads::ResourceLoadLease,
        callback: F,
    ) -> anyhow::Result<()>
    where
        F: FnOnce(
                std::result::Result<ModuleGraphFetchedSource, String>,
                Option<SharedNavigationResponseResult>,
            ) + Send
            + 'static,
    {
        self.fetch_source_callback_inner(loader, load, callback)
    }

    pub(crate) fn fetch_source_for_document<F>(
        &self,
        loader: &crate::network::context::DocumentResourceLoader,
        callback: F,
    ) -> anyhow::Result<()>
    where
        F: FnOnce(
                std::result::Result<ModuleGraphFetchedSource, String>,
                Option<SharedNavigationResponseResult>,
            ) + Send
            + 'static,
    {
        let load = loader
            .register_load(
                crate::network::loads::ResourceLoadKind::Script,
                crate::network::loads::ResourceLoadDisposition::Ordinary,
                None,
            )
            .ok_or_else(|| anyhow::anyhow!("Document detached before module fetch registration"))?;
        let request_client = load.request_client();
        self.fetch_source_callback_with_load(&request_client, load, callback)
    }

    fn fetch_source_callback_inner<F>(
        &self,
        loader: &ResourceRequestClient,
        load: crate::network::loads::ResourceLoadLease,
        callback: F,
    ) -> anyhow::Result<()>
    where
        F: FnOnce(
                std::result::Result<ModuleGraphFetchedSource, String>,
                Option<SharedNavigationResponseResult>,
            ) + Send
            + 'static,
    {
        let source_url = self.source_url.clone();
        let kind = self.kind;
        let integrity = self.fetch_metadata.request_metadata.integrity.clone();
        let request = self.request()?;
        let completion = move |response: anyhow::Result<moli_fetch::Response>| {
            let mut network_result: Option<SharedNavigationResponseResult> = None;
            let result = response
                .map_err(|error| {
                    ModuleLoadError::new(
                        ModuleLoadStage::Fetch,
                        format!("failed to fetch module `{source_url}`: {error} (FailedToLoad)"),
                    )
                    .message()
                    .to_owned()
                })
                .and_then(|response| {
                    let response = NavigationResponse::from(response);
                    network_result = Some(std::sync::Arc::new(Ok(response.clone())));
                    if !(200..=299).contains(&response.status) {
                        return Err(ModuleLoadError::new(
                            ModuleLoadStage::Fetch,
                            format!(
                                "module request `{source_url}` returned HTTP {} (FailedToLoad)",
                                response.status
                            ),
                        )
                        .message()
                        .to_owned());
                    }
                    if matches!(kind, ModuleKind::JavaScript | ModuleKind::WebAssembly)
                        && let Err(error) = crate::planning::validate_external_script_response_mime(
                            &source_url,
                            ScriptKind::Module,
                            &response,
                        )
                    {
                        return Err(ModuleLoadError::new(ModuleLoadStage::Fetch, error)
                            .message()
                            .to_owned());
                    }
                    let response_referrer_policy =
                        crate::referrer_policy::response_referrer_policy_from_headers(
                            &response.headers,
                        );
                    let (head, _body, body_bytes) = response.into_parts();
                    if !crate::subresource_integrity::response_body_matches_subresource_integrity_metadata(
                        &body_bytes,
                        integrity.as_deref(),
                    ) {
                        return Err(ModuleLoadError::new(
                            ModuleLoadStage::Fetch,
                            format!(
                                "module request `{source_url}` failed its integrity check (FailedToLoad)"
                            ),
                        )
                        .message()
                        .to_owned());
                    }
                    Ok(match kind {
                        ModuleKind::WebAssembly => ModuleGraphFetchedSource::new(
                            head.final_url,
                            head.redirected,
                            ModuleSource::binary(body_bytes),
                        )
                        .with_response_referrer_policy(response_referrer_policy),
                        ModuleKind::JavaScript
                        | ModuleKind::Json
                        | ModuleKind::Css
                        | ModuleKind::ModulePreloadText => ModuleGraphFetchedSource::new(
                            head.final_url,
                            head.redirected,
                            ModuleSource::text(moli_encoding::decode_utf8(&body_bytes)),
                        )
                        .with_response_referrer_policy(response_referrer_policy),
                    })
                });
            if network_result.is_none()
                && let Err(error) = &result
            {
                network_result = Some(std::sync::Arc::new(Err(error.clone())));
            }
            callback(result, network_result);
        };
        loader.fetch_cacheable_script_text_callback_with_load(request, load, completion)
    }
}

impl NativeModuleGraphJob {
    fn module_script(root: ModuleRootInput, owner: ModuleScriptCompletionOwner) -> Self {
        let (label, chromium_tree, kind) = match owner {
            ModuleScriptCompletionOwner::Parser => (
                "parser_owned_external",
                external_chromium_tree_root_input(&root)
                    .expect("parser-owned module graph root should be valid")
                    .map(tree_adapter::parser_owned_tree_job)
                    .expect("parser-owned module graph jobs require an external tree root"),
                NativeModuleGraphJobKind::ParserOwned,
            ),
            ModuleScriptCompletionOwner::Runtime => (
                "runtime_module_script_external",
                external_chromium_tree_root_input(&root)
                    .expect("runtime module graph root should be valid")
                    .map(tree_adapter::runtime_module_script_tree_job)
                    .expect("runtime module graph jobs require an external tree root"),
                NativeModuleGraphJobKind::RuntimeModuleScript,
            ),
        };
        trace_module_graph_job_created(label, &root);
        Self {
            tree_job: Some(NativeModuleTreeJob::new(chromium_tree)),
            kind,
        }
    }

    fn parser_owned(root: ModuleRootInput) -> Self {
        Self::module_script(root, ModuleScriptCompletionOwner::Parser)
    }

    fn runtime_module_script(root: ModuleRootInput) -> Self {
        Self::module_script(root, ModuleScriptCompletionOwner::Runtime)
    }

    pub(crate) fn parser_owned_compiled_entry(
        key: ModuleMapKey,
        entry: ModuleEntryId,
        source_url: Url,
        base_url: Url,
        fetch_metadata: ModuleFetchMetadata,
    ) -> Self {
        native_module_graph_job_for_inline_entry(
            &ModuleRootInput {
                source_url,
                base_url,
                initiator_url: key.url().clone(),
                attributes: key.attributes().clone(),
                phase: ModuleImportPhase::Evaluation,
                source_override: None,
                fetch_metadata,
                parser_owned: true,
            },
            &key,
            entry,
            ModuleScriptCompletionOwner::Parser,
        )
    }

    pub(crate) fn tree_id(&self) -> Option<module_tree::ModuleTreeId> {
        self.tree_job
            .as_ref()
            .map(|tree_job| tree_job.chromium_tree().tree_id())
    }

    pub(crate) fn dynamic_import(request: PendingDynamicModuleImport) -> Self {
        Self {
            tree_job: None,
            kind: NativeModuleGraphJobKind::DynamicImport(Box::new(request)),
        }
    }

    pub(crate) fn needs_dynamic_import_graph_start(&self) -> bool {
        matches!(&self.kind, NativeModuleGraphJobKind::DynamicImport(_)) && self.tree_job.is_none()
    }

    fn kind_label(&self) -> &'static str {
        match self.kind {
            NativeModuleGraphJobKind::ParserOwned => "parser_owned",
            NativeModuleGraphJobKind::RuntimeModuleScript => "runtime_module_script",
            NativeModuleGraphJobKind::DynamicImport(_) => "dynamic_import",
        }
    }

    pub(crate) fn take_pending_joined_clients(
        &mut self,
    ) -> Vec<module_tree::SingleModuleClientToken> {
        self.tree_job
            .as_mut()
            .map(NativeModuleTreeJob::take_pending_joined_clients)
            .unwrap_or_default()
    }

    pub(crate) fn advance_module_script_owner_lane(
        &mut self,
        vm: &mut ScriptVm,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        match self.kind {
            NativeModuleGraphJobKind::ParserOwned => {
                self.advance_owner_lane(vm, NATIVE_PARSER_MODULE_GRAPH_JOB)
            }
            NativeModuleGraphJobKind::RuntimeModuleScript => {
                self.advance_owner_lane(vm, NATIVE_RUNTIME_MODULE_GRAPH_JOB)
            }
            NativeModuleGraphJobKind::DynamicImport(_) => {
                self.advance_owner_lane(vm, NATIVE_DYNAMIC_IMPORT_GRAPH_JOB)
            }
        }
    }

    pub(crate) fn advance_dynamic_import_owner_lane(
        &mut self,
        vm: &mut ScriptVm,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let mut owner = NativeModuleTreeDocumentOwner::new(vm);
        self.advance_dynamic_import_owner_lane_with_owner(&mut owner)
    }

    pub(crate) fn advance_dynamic_import_owner_lane_with_owner<O>(
        &mut self,
        owner: &mut O,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        self.prepare_dynamic_import_loader_with_owner(owner)?;
        self.advance_chromium_tree_owner_lane_with_owner(owner)
    }

    pub(crate) fn finish_dynamic_import_fetch_for_request(
        &mut self,
        vm: &mut ScriptVm,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let mut owner = NativeModuleTreeDocumentOwner::new(vm);
        self.finish_dynamic_import_fetch_for_request_with_owner(&mut owner, request, source)
    }

    pub(crate) fn finish_dynamic_import_fetch_for_request_with_owner<O>(
        &mut self,
        owner: &mut O,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        self.finish_pending_chromium_tree_fetch_for_request_with_owner(owner, request, source)
    }

    pub(crate) fn finish_module_tree_fetch_for_request_with_owner<O>(
        &mut self,
        owner: &mut O,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        self.finish_pending_chromium_tree_fetch_for_request_with_owner(owner, request, source)
    }

    pub(crate) fn finish_joined_module_map_fetch(
        &mut self,
        vm: &mut ScriptVm,
        key: module_tree::ModuleMapKey,
        client: module_tree::SingleModuleClientToken,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let mut owner = NativeModuleTreeDocumentOwner::new(vm);
        self.finish_joined_module_map_fetch_with_owner(&mut owner, key, client)
    }

    pub(crate) fn finish_joined_module_map_fetch_with_owner<O>(
        &mut self,
        owner: &mut O,
        key: module_tree::ModuleMapKey,
        client: module_tree::SingleModuleClientToken,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        let outcome = module_map_fetch_outcome_for_key_with_owner(owner, &key)?;
        self.resume_chromium_tree_fetch_outcome_with_owner(owner, client, outcome)
    }

    pub(crate) fn finish_joined_module_map_fetch_for_local_key_with_owner<O>(
        &mut self,
        owner: &mut O,
        key: &ModuleMapKey,
        client: module_tree::SingleModuleClientToken,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        self.finish_joined_module_map_fetch_with_owner(owner, chromium_module_key(key), client)
    }

    fn advance_owner_lane(
        &mut self,
        vm: &mut ScriptVm,
        _job_name: &str,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        self.advance_chromium_tree_owner_lane(vm)
    }

    #[cfg(test)]
    pub(crate) fn has_chromium_tree_for_test(&self) -> bool {
        self.tree_job.is_some()
    }

    #[cfg(test)]
    pub(crate) fn pending_joined_client_count_for_test(&self) -> usize {
        self.tree_job
            .as_ref()
            .map(NativeModuleTreeJob::pending_joined_client_count)
            .unwrap_or(0)
    }

    fn prepare_dynamic_import_loader_with_owner<O>(
        &mut self,
        owner: &mut O,
    ) -> std::result::Result<(), ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        if self.tree_job.is_some() {
            return Ok(());
        }
        let NativeModuleGraphJobKind::DynamicImport(request) = &self.kind else {
            panic!("native dynamic import graph job missing request");
        };
        let root = dynamic_import_root_input(owner, request)?;
        trace_module_graph_job_created("dynamic_import_external", &root);
        self.tree_job = external_chromium_tree_root_input(&root)?
            .map(tree_adapter::dynamic_import_tree_job)
            .map(NativeModuleTreeJob::new);
        Ok(())
    }

    pub(crate) fn into_dynamic_import_request(self) -> PendingDynamicModuleImport {
        match self.kind {
            NativeModuleGraphJobKind::DynamicImport(request) => *request,
            NativeModuleGraphJobKind::ParserOwned => {
                panic!("native dynamic import graph job missing request")
            }
            NativeModuleGraphJobKind::RuntimeModuleScript => {
                panic!("native dynamic import graph job missing request")
            }
        }
    }

    pub(crate) fn dynamic_import_request(&self) -> Option<&PendingDynamicModuleImport> {
        match &self.kind {
            NativeModuleGraphJobKind::DynamicImport(request) => Some(request),
            NativeModuleGraphJobKind::ParserOwned
            | NativeModuleGraphJobKind::RuntimeModuleScript => None,
        }
    }

    fn advance_chromium_tree_owner_lane(
        &mut self,
        vm: &mut ScriptVm,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let mut owner = NativeModuleTreeDocumentOwner::new(vm);
        self.advance_chromium_tree_owner_lane_with_owner(&mut owner)
    }

    pub(crate) fn advance_chromium_tree_owner_lane_with_owner<O>(
        &mut self,
        owner: &mut O,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        let job_kind = self.kind_label();
        let (poll, tree_id) = {
            let tree_job = self.tree_job_mut()?;
            let tree_id = tree_job.chromium_tree().tree_id();
            trace_module_tree_poll_start(job_kind, tree_job.chromium_tree());
            let mut host = RendererModuleScriptTreeHost::new(&mut *owner);
            let drive = tree_job.drive(&mut host);
            (drive, tree_id)
        };
        trace_module_tree_drive_result(job_kind, tree_id, &poll);
        self.advance_from_chromium_tree_drive(owner, poll)
    }

    fn finish_module_script_fetch_for_request(
        &mut self,
        vm: &mut ScriptVm,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        self.finish_pending_chromium_tree_fetch_for_request(vm, request, source)
    }

    fn finish_pending_chromium_tree_fetch_for_request(
        &mut self,
        vm: &mut ScriptVm,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let client = request.tree_client.ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module graph fetch request was missing its module tree client token",
            )
        })?;
        if let Err(error) = &source {
            let key = request.pending_fetch_key().cloned().ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    "failed module graph fetch request was missing its module map key",
                )
            })?;
            vm.document_runtime
                .mark_native_module_failed(key, error.clone());
        }
        self.finish_pending_chromium_tree_fetch_for_client(vm, client, request, source)
    }

    fn finish_pending_chromium_tree_fetch_for_request_with_owner<O>(
        &mut self,
        owner: &mut O,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        let client = request.tree_client.ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module graph fetch request was missing its module tree client token",
            )
        })?;
        self.finish_pending_chromium_tree_fetch_for_client_with_owner(
            owner, client, request, source,
        )
    }

    fn finish_pending_chromium_tree_fetch_for_client(
        &mut self,
        vm: &mut ScriptVm,
        client: module_tree::SingleModuleClientToken,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        trace_module_tree_fetch_completed_to_job(client, source.is_ok());
        let outcome = match source {
            Ok(source) => module_tree::ModuleFetchOutcome::Fetched(Box::new(
                chromium_fetched_source_for_request(source, request)?,
            )),
            Err(error) => module_tree::ModuleFetchOutcome::Failed(chromium_error(error)),
        };
        self.resume_chromium_tree_fetch_outcome(vm, client, outcome)
    }

    fn finish_pending_chromium_tree_fetch_for_client_with_owner<O>(
        &mut self,
        owner: &mut O,
        client: module_tree::SingleModuleClientToken,
        request: &NativeModuleGraphFetchRequest,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        trace_module_tree_fetch_completed_to_job(client, source.is_ok());
        let outcome = match source {
            Ok(source) => module_tree::ModuleFetchOutcome::Fetched(Box::new(
                chromium_fetched_source_for_request(source, request)?,
            )),
            Err(error) => module_tree::ModuleFetchOutcome::Failed(chromium_error(error)),
        };
        self.resume_chromium_tree_fetch_outcome_with_owner(owner, client, outcome)
    }

    fn resume_chromium_tree_fetch_outcome(
        &mut self,
        vm: &mut ScriptVm,
        client: module_tree::SingleModuleClientToken,
        outcome: module_tree::ModuleFetchOutcome,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError> {
        let mut owner = NativeModuleTreeDocumentOwner::new(vm);
        self.resume_chromium_tree_fetch_outcome_with_owner(&mut owner, client, outcome)
    }

    pub(crate) fn resume_chromium_tree_fetch_outcome_with_owner<O>(
        &mut self,
        owner: &mut O,
        client: module_tree::SingleModuleClientToken,
        outcome: module_tree::ModuleFetchOutcome,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        let drive = {
            let tree_job = self.tree_job_mut()?;
            let mut host = RendererModuleScriptTreeHost::new(&mut *owner);
            tree_job.resume_single_module_outcome_and_drive(&mut host, client, outcome)
        };
        trace_module_tree_resume_drive(self.kind_label(), &drive);
        self.advance_from_chromium_tree_drive(owner, drive)
    }

    fn tree_job_mut(&mut self) -> std::result::Result<&mut NativeModuleTreeJob, ModuleLoadError> {
        self.tree_job.as_mut().ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Compile,
                "native module graph job was missing module script tree",
            )
        })
    }

    fn advance_from_chromium_tree_drive<O>(
        &mut self,
        owner: &mut O,
        drive: module_tree::ModuleScriptTreeDrive,
    ) -> std::result::Result<NativeModuleGraphJobAdvance, ModuleLoadError>
    where
        O: NativeModuleTreeDocumentOwnerAdapter,
    {
        let advance = self.tree_job_mut()?.advance_from_drive(
            drive,
            |key| local_module_key(key).map(|_| ()),
            native_tree_fetch_request_from_chromium,
        )?;
        match advance {
            NativeModuleTreeJobAdvance::NeedFetches(fetches) => {
                let mut requests = Vec::with_capacity(fetches.len());
                for fetch in fetches {
                    let key = fetch.key().clone();
                    let request = native_fetch_request_from_tree_fetch(&fetch)?.with_tree_fetch(
                        fetch.client(),
                        fetch.graph_level(),
                        key.clone(),
                        fetch.dependency().cloned(),
                    );
                    owner.dispatch_module_fetch_csp_report_only_violation(
                        &key,
                        &request.fetch_metadata,
                    );
                    if let Some(error) =
                        owner.csp_blocked_module_fetch_error(&key, &request.fetch_metadata)
                    {
                        owner.mark_module_failed(key.clone(), error.clone());
                        return Err(error);
                    }
                    trace_module_tree_fetch_emitted(fetch.request(), &key);
                    requests.push(request);
                }
                Ok(NativeModuleGraphJobAdvance::NeedFetches(requests))
            }
            NativeModuleTreeJobAdvance::WaitingForFetches { client_count } => {
                if client_count > 0 {
                    trace_module_tree_waiting(self.kind_label(), client_count);
                }
                Ok(NativeModuleGraphJobAdvance::WaitingForFetches)
            }
            NativeModuleTreeJobAdvance::Complete(graph) => {
                trace_module_tree_complete(self.kind_label(), &graph);
                Ok(NativeModuleGraphJobAdvance::Complete(local_graph(graph)))
            }
            NativeModuleTreeJobAdvance::Failed(error) => Err(local_error(error)),
            NativeModuleTreeJobAdvance::Aborted(reason) => Err(chromium_abort_error(reason)),
            NativeModuleTreeJobAdvance::PendingWithoutWork => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module script tree reached a pending state without fetch, wait, or completion",
            )),
            NativeModuleTreeJobAdvance::IgnoredStaleCompletion => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module script tree ignored a stale completion during owner-lane advance",
            )),
        }
    }
}

fn trace_module_graph_job_created(kind: &'static str, root: &ModuleRootInput) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_graph_job_created",
        kind,
        url = %root.source_url,
        base_url = %root.base_url,
        initiator_url = %root.initiator_url,
        phase = ?root.phase,
        parser_owned = root.parser_owned,
        integrity = ?root.fetch_metadata.request_metadata.integrity,
    );
}

fn trace_module_tree_poll_start(job_kind: &'static str, tree: &module_tree::ModuleScriptTreeJob) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_poll_start",
        job_kind,
        tree_id = tree.tree_id().0,
        state = ?tree.state(),
        pending_client_count = tree.pending_client_count(),
    );
}

fn trace_module_tree_drive_result(
    job_kind: &'static str,
    tree_id: module_tree::ModuleTreeId,
    drive: &module_tree::ModuleScriptTreeDrive,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    match drive {
        module_tree::ModuleScriptTreeDrive::NeedFetches(fetches) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "need_fetches",
                fetch_count = fetches.len(),
                joined_fetch_count = fetches.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::WaitingForSingleModuleClients(wait) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "waiting_for_single_module_clients",
                client_count = wait.client_count,
                joined_fetch_count = wait.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::Complete(graph) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "complete",
                entry_count = graph.entries.len(),
                dependency_edge_count = graph.dependency_edges.len(),
                joined_fetch_count = 0usize,
            );
        }
        module_tree::ModuleScriptTreeDrive::Failed(error) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "failed",
                stage = ?error.stage,
                joined_fetch_count = 0usize,
            );
        }
        module_tree::ModuleScriptTreeDrive::Aborted(reason) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "aborted",
                reason = ?reason,
                joined_fetch_count = 0usize,
            );
        }
        module_tree::ModuleScriptTreeDrive::Pending(idle) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "pending",
                joined_fetch_count = idle.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::IgnoredStaleCompletion(idle) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_drive_result",
                job_kind,
                tree_id = tree_id.0,
                result = "ignored_stale_completion",
                joined_fetch_count = idle.joined_fetches.len(),
            );
        }
    }
}

fn trace_module_tree_resume_drive(
    job_kind: &'static str,
    drive: &module_tree::ModuleScriptTreeDrive,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    match drive {
        module_tree::ModuleScriptTreeDrive::NeedFetches(fetches) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "need_fetches",
                fetch_count = fetches.len(),
                joined_fetch_count = fetches.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::WaitingForSingleModuleClients(wait) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "waiting_for_single_module_clients",
                client_count = wait.client_count,
                joined_fetch_count = wait.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::Complete(graph) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "complete",
                entry_count = graph.entries.len(),
                dependency_edge_count = graph.dependency_edges.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::Failed(error) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "failed",
                stage = ?error.stage,
            );
        }
        module_tree::ModuleScriptTreeDrive::Aborted(reason) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "aborted",
                reason = ?reason,
            );
        }
        module_tree::ModuleScriptTreeDrive::Pending(idle) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "pending",
                joined_fetch_count = idle.joined_fetches.len(),
            );
        }
        module_tree::ModuleScriptTreeDrive::IgnoredStaleCompletion(idle) => {
            tracing::info!(
                target: "moli_module_load",
                event = "module_tree_resume_drive",
                job_kind,
                result = "ignored_stale_completion",
                joined_fetch_count = idle.joined_fetches.len(),
            );
        }
    }
}

fn trace_module_tree_fetch_emitted(fetch: &module_tree::ModuleFetchRequest, key: &ModuleMapKey) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_fetch_emitted",
        tree_id = fetch.tree_id.0,
        client_sequence = fetch.client.sequence,
        url = %fetch.source_url,
        key_url = %key.url(),
        graph_level = ?fetch.graph_level,
        phase = ?fetch.phase,
        requester = ?fetch.requester,
        ordering = ?fetch.ordering,
        initiator_url = %fetch.initiator_url,
    );
}

fn trace_module_tree_fetch_registered(
    request: &module_tree::ModuleFetchRequest,
    client: module_tree::SingleModuleClientToken,
    disposition: &'static str,
    entry_id: Option<u64>,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_fetch_registered",
        tree_id = request.tree_id.0,
        client_sequence = client.sequence,
        url = %request.source_url,
        graph_level = ?request.graph_level,
        phase = ?request.phase,
        requester = ?request.requester,
        ordering = ?request.ordering,
        disposition,
        entry_id,
    );
}

fn trace_module_tree_fetch_completed_to_job(
    client: module_tree::SingleModuleClientToken,
    ok: bool,
) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_fetch_completed_to_job",
        tree_id = client.tree_id.0,
        client_sequence = client.sequence,
        ok,
    );
}

fn trace_module_tree_waiting(job_kind: &'static str, client_count: usize) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_waiting_for_clients",
        job_kind,
        client_count,
    );
}

fn trace_module_tree_complete(job_kind: &'static str, graph: &module_tree::ModuleGraphHandle) {
    if !moli_trace::module_load_trace_enabled() {
        return;
    }
    tracing::info!(
        target: "moli_module_load",
        event = "module_tree_complete",
        job_kind,
        root_entry = graph.root_entry.0,
        entry_count = graph.entries.len(),
        dependency_edge_count = graph.dependency_edges.len(),
    );
}

struct RendererModuleScriptTreeHost<O> {
    owner: O,
}

impl<O> RendererModuleScriptTreeHost<O> {
    fn new(owner: O) -> Self {
        Self { owner }
    }
}

impl<O: NativeModuleTreeDocumentOwnerAdapter> RendererModuleScriptTreeHost<O> {
    fn owner_fetched_source_from_entry(
        &self,
        entry: ModuleEntryId,
        request_key: module_tree::ModuleMapKey,
        source: ModuleSource,
    ) -> module_tree::FetchedModuleSource {
        let effective_key = self.owner.module_entry_key(entry);
        let source_url = effective_key.url().clone();
        let effective_fetch_metadata = self.owner.module_effective_fetch_metadata(entry);
        module_tree::FetchedModuleSource::new(
            request_key,
            chromium_module_key(&effective_key),
            source_url.clone(),
            source_url,
            chromium_source(source),
            chromium_fetch_metadata(&effective_fetch_metadata),
        )
    }

    fn owner_ready_module_from_entry(&self, entry: ModuleEntryId) -> module_tree::ReadyModule {
        let effective_key = self.owner.module_entry_key(entry);
        let source_url = effective_key.url().clone();
        let effective_fetch_metadata = self.owner.module_effective_fetch_metadata(entry);
        module_tree::ReadyModule::new(
            chromium_entry_id(entry),
            chromium_module_key(&effective_key),
            source_url.clone(),
            chromium_fetch_metadata(&effective_fetch_metadata),
        )
    }
}

impl<O: NativeModuleTreeDocumentOwnerAdapter> module_tree::ModuleScriptTreeHost
    for RendererModuleScriptTreeHost<O>
{
    fn resolve_module_request(
        &mut self,
        specifier: &str,
        base_url: &Url,
        attributes: &module_tree::ModuleAttributesKey,
        requested_phase: module_tree::ModuleImportPhase,
    ) -> std::result::Result<module_tree::ResolvedModuleRequest, module_tree::ModuleLoadError> {
        let resolved_url = self
            .owner
            .resolve_module_specifier(specifier, base_url)
            .map_err(|error| {
                module_tree::ModuleLoadError::new(
                    module_tree::ModuleLoadStage::Resolve,
                    format!(
                        "failed to resolve module specifier `{specifier}` from `{base_url}`: {error}"
                    ),
                )
                .with_error_constructor(module_tree::ModuleErrorConstructorKind::TypeError)
            })?;
        let attributes = local_attributes(attributes);
        let local_key = ModuleMapKey::from_url_and_attributes(&resolved_url, &attributes).map_err(
            |message| {
                module_tree::ModuleLoadError::new(
                    module_tree::ModuleLoadStage::Resolve,
                    format!("{message} for import `{specifier}`"),
                )
                .with_error_constructor(module_tree::ModuleErrorConstructorKind::TypeError)
            },
        )?;
        if requested_phase == module_tree::ModuleImportPhase::Source
            && local_key.kind() != ModuleKind::WebAssembly
        {
            return Err(module_tree::ModuleLoadError::new(
                module_tree::ModuleLoadStage::Resolve,
                format!(
                    "source-phase import `{specifier}` does not resolve to a WebAssembly module"
                ),
            )
            .with_error_constructor(module_tree::ModuleErrorConstructorKind::SyntaxError));
        }
        let integrity = self.owner.resolve_module_integrity(&resolved_url);
        Ok(module_tree::ResolvedModuleRequest {
            key: chromium_module_key(&local_key),
            source_url: resolved_url.clone(),
            base_url: resolved_url,
            kind: chromium_module_kind(local_key.kind()),
            attributes: chromium_attributes(&attributes),
            phase: requested_phase,
            integrity,
        })
    }

    fn start_or_join_single_module_fetch(
        &mut self,
        request: module_tree::ModuleFetchRequest,
        client: module_tree::SingleModuleClientToken,
    ) -> module_tree::SingleModuleFetchDisposition {
        let key = match local_module_key(&request.key) {
            Ok(key) => key,
            Err(error) => {
                trace_module_tree_fetch_registered(&request, client, "failed_key_conversion", None);
                return module_tree::SingleModuleFetchDisposition::Completed(
                    module_tree::ModuleFetchOutcome::Failed(chromium_error(error)),
                );
            }
        };
        match self.owner.start_or_join_module_fetch(key.clone()) {
            ModuleMapFetchDisposition::StartedFetch(entry_id) => {
                trace_module_tree_fetch_registered(
                    &request,
                    client,
                    "started_network_fetch",
                    Some(u64::from(entry_id.raw())),
                );
                module_tree::SingleModuleFetchDisposition::StartedNetworkFetch {
                    fetch_id: module_tree::ModuleFetchId(u64::from(entry_id.raw())),
                }
            }
            ModuleMapFetchDisposition::JoinedFetching(_) => {
                self.register_joined_module_fetch_client(key, &request);
                trace_module_tree_fetch_registered(&request, client, "joined_existing_fetch", None);
                module_tree::SingleModuleFetchDisposition::JoinedExistingFetch
            }
            ModuleMapFetchDisposition::AlreadyFetched(entry_id) => {
                trace_module_tree_fetch_registered(
                    &request,
                    client,
                    "already_fetched_completion_queued",
                    Some(u64::from(entry_id.raw())),
                );
                let outcome = self
                    .owner
                    .module_source(entry_id)
                    .map(|source| {
                        module_tree::ModuleFetchOutcome::Fetched(Box::new(
                            self.owner_fetched_source_from_entry(
                                entry_id,
                                request.key.clone(),
                                source,
                            ),
                        ))
                    })
                    .unwrap_or_else(|| {
                        module_tree::ModuleFetchOutcome::Failed(module_tree::ModuleLoadError::new(
                            module_tree::ModuleLoadStage::Fetch,
                            "fetched module map entry did not retain source",
                        ))
                    });
                module_tree::SingleModuleFetchDisposition::Completed(outcome)
            }
            ModuleMapFetchDisposition::AlreadyCompiled(entry_id) => {
                trace_module_tree_fetch_registered(
                    &request,
                    client,
                    "already_compiled_completion_queued",
                    Some(u64::from(entry_id.raw())),
                );
                module_tree::SingleModuleFetchDisposition::Completed(
                    module_tree::ModuleFetchOutcome::Ready(Box::new(
                        self.owner_ready_module_from_entry(entry_id),
                    )),
                )
            }
            ModuleMapFetchDisposition::AlreadyFailed(entry_id) => {
                trace_module_tree_fetch_registered(
                    &request,
                    client,
                    "already_failed_completion_queued",
                    Some(u64::from(entry_id.raw())),
                );
                let error = self
                    .owner
                    .module_failure(entry_id)
                    .map(chromium_error)
                    .unwrap_or_else(|| {
                        module_tree::ModuleLoadError::new(
                            module_tree::ModuleLoadStage::Resolve,
                            "module previously failed to load",
                        )
                    });
                module_tree::SingleModuleFetchDisposition::Completed(
                    module_tree::ModuleFetchOutcome::Failed(error),
                )
            }
        }
    }

    fn compile_module_source(
        &mut self,
        fetched_source: module_tree::FetchedModuleSource,
        phase: module_tree::ModuleImportPhase,
    ) -> std::result::Result<module_tree::CompiledModuleSnapshot, module_tree::ModuleLoadError>
    {
        let local_request_key =
            local_module_key(&fetched_source.request_key).map_err(chromium_error)?;
        let local_key = local_module_key(&fetched_source.key).map_err(chromium_error)?;
        let source = local_source(fetched_source.source);
        let source_url = fetched_source.source_url.clone();
        let local_effective_fetch_metadata =
            local_fetch_metadata(&fetched_source.effective_fetch_metadata);
        self.owner.insert_module_source_for_request(
            local_request_key.clone(),
            local_key.clone(),
            source.clone(),
            local_effective_fetch_metadata.clone(),
        );
        let (record, identity) = self
            .owner
            .compile_module_record(
                local_key.clone(),
                &source,
                &source_url,
                &local_effective_fetch_metadata,
            )
            .map_err(chromium_error)?;
        let requests = record
            .requests()
            .iter()
            .map(chromium_request_record)
            .collect();
        let entry = self.owner.insert_compiled_module_record(
            local_request_key,
            record,
            identity,
            local_effective_fetch_metadata,
        );
        Ok(module_tree::CompiledModuleSnapshot {
            entry: chromium_entry_id(entry),
            key: fetched_source.key,
            base_url: fetched_source.base_url,
            effective_fetch_metadata: fetched_source.effective_fetch_metadata,
            requested_modules: requests,
            phase,
            has_parse_error: false,
            parse_error: None,
        })
    }

    fn module_dependencies(
        &self,
        entry: module_tree::ModuleEntryId,
    ) -> std::result::Result<module_tree::ModuleDependencySnapshot, module_tree::ModuleLoadError>
    {
        let entry = local_entry_id(entry);
        let key = self.owner.module_entry_key(entry);
        let base_url = self.owner.module_entry_url(entry);
        let effective_fetch_metadata = self.owner.module_effective_fetch_metadata(entry);
        let requested_modules = self
            .owner
            .module_requests(entry)
            .iter()
            .map(chromium_request_record)
            .collect();
        Ok(module_tree::ModuleDependencySnapshot {
            entry: chromium_entry_id(entry),
            key: chromium_module_key(&key),
            base_url,
            effective_fetch_metadata: chromium_fetch_metadata(&effective_fetch_metadata),
            requested_modules,
        })
    }

    fn link_module_graph(
        &mut self,
        root: module_tree::ModuleEntryId,
        entries: &[module_tree::ModuleEntryId],
        dependency_edges: &[module_tree::ModuleDependencyEdge],
    ) -> std::result::Result<module_tree::ModuleGraphHandle, module_tree::ModuleLoadError> {
        let mut dependencies_by_parent: HashMap<ModuleEntryId, Vec<ModuleResolvedDependency>> =
            HashMap::new();
        for edge in dependency_edges {
            let parent = local_entry_id(edge.parent_entry);
            let child_key = local_module_key(&edge.child_key).map_err(chromium_error)?;
            dependencies_by_parent
                .entry(parent)
                .or_default()
                .push(ModuleResolvedDependency::new(
                    edge.specifier.clone(),
                    local_attributes(&edge.attributes),
                    child_key,
                ));
        }
        for (entry, dependencies) in dependencies_by_parent {
            self.owner
                .set_module_resolved_dependencies(entry, dependencies);
        }
        Ok(module_tree::ModuleGraphHandle {
            root_entry: root,
            entries: entries.to_vec(),
            entry_phases: HashMap::new(),
            dependency_edges: dependency_edges.to_vec(),
        })
    }

    fn mark_module_failed(
        &mut self,
        key: module_tree::ModuleMapKey,
        error: module_tree::ModuleLoadError,
    ) -> module_tree::ModuleEntryId {
        let local_key =
            local_module_key(&key).unwrap_or_else(|_| ModuleMapKey::java_script(key.url.clone()));
        let entry = self.owner.mark_module_failed(local_key, local_error(error));
        chromium_entry_id(entry)
    }
}

fn module_map_fetch_outcome_for_key_with_owner<O>(
    owner: &O,
    request_key: &module_tree::ModuleMapKey,
) -> std::result::Result<module_tree::ModuleFetchOutcome, ModuleLoadError>
where
    O: NativeModuleTreeDocumentOwnerAdapter,
{
    let local_key = local_module_key(request_key)?;
    let entry_id = owner.module_entry_id(&local_key).ok_or_else(|| {
        ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            format!(
                "module map fanout for `{}` had no module map entry",
                local_key.url()
            ),
        )
    })?;
    let outcome = match owner.module_entry_state(entry_id) {
        ModuleMapEntryState::Fetched => {
            let source = owner.module_source(entry_id).ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    "fetched module map entry did not retain source",
                )
            })?;
            module_tree::ModuleFetchOutcome::Fetched(Box::new(
                chromium_fetched_source_from_owner_entry(
                    owner,
                    entry_id,
                    request_key.clone(),
                    source,
                ),
            ))
        }
        ModuleMapEntryState::Compiled
        | ModuleMapEntryState::Instantiated
        | ModuleMapEntryState::Evaluating
        | ModuleMapEntryState::Evaluated => module_tree::ModuleFetchOutcome::Ready(Box::new(
            chromium_ready_module_from_owner_entry(owner, entry_id),
        )),
        ModuleMapEntryState::Failed => {
            let error = owner
                .module_failure(entry_id)
                .map(chromium_error)
                .unwrap_or_else(|| {
                    module_tree::ModuleLoadError::new(
                        module_tree::ModuleLoadStage::Fetch,
                        "module previously failed to load",
                    )
                });
            module_tree::ModuleFetchOutcome::Failed(error)
        }
        ModuleMapEntryState::Fetching => {
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!(
                    "module map fanout for `{}` fired while entry was still fetching",
                    local_key.url()
                ),
            ));
        }
    };
    Ok(outcome)
}

fn native_tree_fetch_request_from_chromium(
    request: module_tree::ModuleFetchRequest,
) -> std::result::Result<NativeModuleTreeFetchRequest, ModuleLoadError> {
    let key = local_module_key(&request.key)?;
    let dependency = native_dependency_request_from_chromium(&request)?;
    Ok(NativeModuleTreeFetchRequest::new(request, key, dependency))
}

fn native_fetch_request_from_tree_fetch(
    request: &NativeModuleTreeFetchRequest,
) -> std::result::Result<NativeModuleGraphFetchRequest, ModuleLoadError> {
    let raw = request.request();
    Ok(NativeModuleGraphFetchRequest::new(
        raw.source_url.clone(),
        raw.initiator_url.clone(),
        local_fetch_metadata(&raw.fetch_metadata),
        local_module_kind(raw.kind),
    ))
}

fn native_dependency_request_from_chromium(
    request: &module_tree::ModuleFetchRequest,
) -> std::result::Result<Option<NativeModuleGraphDependencyRequest>, ModuleLoadError> {
    let Some(parent) = request.parent.as_ref() else {
        return Ok(None);
    };
    let specifier = request.specifier.clone().ok_or_else(|| {
        ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            "dependent module fetch request was missing its original specifier",
        )
    })?;
    Ok(Some(NativeModuleGraphDependencyRequest::new(
        local_module_key(&parent.key)?,
        local_entry_id(parent.entry),
        specifier,
        local_import_phase(request.phase),
    )))
}

fn chromium_entry_id(entry: ModuleEntryId) -> module_tree::ModuleEntryId {
    module_tree::ModuleEntryId(entry.raw())
}

fn local_entry_id(entry: module_tree::ModuleEntryId) -> ModuleEntryId {
    ModuleEntryId::from_raw(entry.0)
}

fn chromium_source(source: ModuleSource) -> module_tree::ModuleSource {
    match source {
        ModuleSource::Text(source) => module_tree::ModuleSource::Text(source),
        ModuleSource::Binary(bytes) => module_tree::ModuleSource::Binary(bytes),
    }
}

fn chromium_fetched_source_from_owner_entry<O>(
    owner: &O,
    entry_id: ModuleEntryId,
    request_key: module_tree::ModuleMapKey,
    source: ModuleSource,
) -> module_tree::FetchedModuleSource
where
    O: NativeModuleTreeDocumentOwnerAdapter,
{
    let effective_key = owner.module_entry_key(entry_id);
    let source_url = effective_key.url().clone();
    let effective_fetch_metadata = owner.module_effective_fetch_metadata(entry_id);
    module_tree::FetchedModuleSource::new(
        request_key,
        chromium_module_key(&effective_key),
        source_url.clone(),
        source_url,
        chromium_source(source),
        chromium_fetch_metadata(&effective_fetch_metadata),
    )
}

fn chromium_ready_module_from_owner_entry<O>(
    owner: &O,
    entry_id: ModuleEntryId,
) -> module_tree::ReadyModule
where
    O: NativeModuleTreeDocumentOwnerAdapter,
{
    let effective_key = owner.module_entry_key(entry_id);
    let source_url = effective_key.url().clone();
    let effective_fetch_metadata = owner.module_effective_fetch_metadata(entry_id);
    module_tree::ReadyModule::new(
        chromium_entry_id(entry_id),
        chromium_module_key(&effective_key),
        source_url.clone(),
        chromium_fetch_metadata(&effective_fetch_metadata),
    )
}

fn chromium_fetched_source_for_request(
    fetched_source: ModuleGraphFetchedSource,
    request: &NativeModuleGraphFetchRequest,
) -> std::result::Result<module_tree::FetchedModuleSource, ModuleLoadError> {
    let request_key = request.pending_fetch_key().ok_or_else(|| {
        ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            "module graph fetched source was missing its pending module map key",
        )
    })?;
    let effective_key = request
        .effective_key_for_fetched_source(&fetched_source)
        .ok_or_else(|| {
            ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "module graph fetched source was missing its effective module map key",
            )
        })?;
    let final_url = fetched_source.final_url().clone();
    let effective_fetch_metadata =
        request.effective_fetch_metadata_for_fetched_source(&fetched_source);
    Ok(module_tree::FetchedModuleSource::new(
        chromium_module_key(request_key),
        chromium_module_key(&effective_key),
        final_url.clone(),
        final_url,
        chromium_source(fetched_source.into_source()),
        chromium_fetch_metadata(&effective_fetch_metadata),
    ))
}

fn local_source(source: module_tree::ModuleSource) -> ModuleSource {
    match source {
        module_tree::ModuleSource::Text(source) => ModuleSource::Text(source),
        module_tree::ModuleSource::Binary(bytes) => ModuleSource::Binary(bytes),
    }
}

fn native_module_map_single_module_client_from_request(
    request: &module_tree::ModuleFetchRequest,
) -> super::NativeModuleMapSingleModuleClient {
    match request.requester {
        module_tree::ModuleFetchRequester::ParserPendingScript
        | module_tree::ModuleFetchRequester::RuntimeModuleScript => {
            super::NativeModuleMapSingleModuleClient::module_script(request.client, request.phase)
        }
        module_tree::ModuleFetchRequester::DynamicImport => {
            super::NativeModuleMapSingleModuleClient::dynamic_import(request.client, request.phase)
        }
        module_tree::ModuleFetchRequester::ModulePreload => {
            unreachable!("modulepreload cannot be represented as a single-module tree client")
        }
    }
}

impl<O: NativeModuleTreeDocumentOwnerAdapter> RendererModuleScriptTreeHost<O> {
    fn register_joined_module_fetch_client(
        &mut self,
        key: ModuleMapKey,
        request: &module_tree::ModuleFetchRequest,
    ) {
        match request.requester {
            module_tree::ModuleFetchRequester::ParserPendingScript
            | module_tree::ModuleFetchRequester::RuntimeModuleScript
            | module_tree::ModuleFetchRequester::DynamicImport => {
                let client = native_module_map_single_module_client_from_request(request);
                self.owner.suspend_module_fetch_waiter(key, client);
            }
            module_tree::ModuleFetchRequester::ModulePreload => {
                self.owner.record_runtime_warning(format_args!(
                    "module script tree produced a modulepreload joined fetch client for `{}`",
                    key.url()
                ));
            }
        }
    }
}

fn chromium_request_record(request: &ModuleRequestRecord) -> module_tree::ModuleRequestRecord {
    module_tree::ModuleRequestRecord {
        specifier: request.specifier().to_owned(),
        attributes: chromium_attributes(request.attributes()),
        phase: chromium_import_phase(request.phase()),
        position: module_tree::TextPosition::default(),
    }
}

fn chromium_module_key(key: &ModuleMapKey) -> module_tree::ModuleMapKey {
    module_tree::ModuleMapKey::new(
        key.url().clone(),
        chromium_module_kind(key.kind()),
        chromium_attributes(key.attributes()),
    )
}

fn local_module_key(
    key: &module_tree::ModuleMapKey,
) -> std::result::Result<ModuleMapKey, ModuleLoadError> {
    let attributes = local_attributes(&key.attributes);
    match key.kind {
        module_tree::ModuleKind::JavaScript => Ok(ModuleMapKey::java_script_with_attributes(
            key.url.clone(),
            attributes,
        )),
        module_tree::ModuleKind::Json => Ok(ModuleMapKey::json_with_attributes(
            key.url.clone(),
            attributes,
        )),
        module_tree::ModuleKind::Css => Ok(ModuleMapKey::css_with_attributes(
            key.url.clone(),
            attributes,
        )),
        module_tree::ModuleKind::WebAssembly => Ok(ModuleMapKey::webassembly(key.url.clone())),
    }
}

fn local_attributes(attributes: &module_tree::ModuleAttributesKey) -> ModuleAttributesKey {
    ModuleAttributesKey::from_pairs(attributes.attributes.clone())
}

fn local_module_kind(kind: module_tree::ModuleKind) -> ModuleKind {
    match kind {
        module_tree::ModuleKind::JavaScript => ModuleKind::JavaScript,
        module_tree::ModuleKind::Json => ModuleKind::Json,
        module_tree::ModuleKind::Css => ModuleKind::Css,
        module_tree::ModuleKind::WebAssembly => ModuleKind::WebAssembly,
    }
}

fn local_graph(graph: module_tree::ModuleGraphHandle) -> ModuleGraphHandle {
    ModuleGraphHandle {
        root_entry: local_entry_id(graph.root_entry),
        entries: graph.entries.into_iter().map(local_entry_id).collect(),
    }
}

fn chromium_error(error: ModuleLoadError) -> module_tree::ModuleLoadError {
    let mut converted =
        module_tree::ModuleLoadError::new(chromium_load_stage(error.stage()), error.message());
    if let Some(constructor) = error.error_constructor() {
        let constructor = match constructor {
            ScriptErrorConstructorKind::SyntaxError => {
                module_tree::ModuleErrorConstructorKind::SyntaxError
            }
            ScriptErrorConstructorKind::TypeError => {
                module_tree::ModuleErrorConstructorKind::TypeError
            }
            ScriptErrorConstructorKind::Error
            | ScriptErrorConstructorKind::WebAssemblyCompileError
            | ScriptErrorConstructorKind::WebAssemblyLinkError => return converted,
        };
        converted = converted.with_error_constructor(constructor);
    }
    converted
}

fn local_error(error: module_tree::ModuleLoadError) -> ModuleLoadError {
    let mut converted = ModuleLoadError::new(local_load_stage(error.stage), error.message);
    if let Some(constructor) = error.error_constructor {
        converted = converted.with_error_constructor(local_error_constructor(constructor));
    }
    converted
}

fn local_error_constructor(
    constructor: module_tree::ModuleErrorConstructorKind,
) -> ScriptErrorConstructorKind {
    match constructor {
        module_tree::ModuleErrorConstructorKind::SyntaxError => {
            ScriptErrorConstructorKind::SyntaxError
        }
        module_tree::ModuleErrorConstructorKind::TypeError => ScriptErrorConstructorKind::TypeError,
    }
}

fn chromium_abort_error(reason: module_tree::ModuleTreeAbortReason) -> ModuleLoadError {
    ModuleLoadError::new(
        ModuleLoadStage::Fetch,
        format!("module script tree aborted: {reason:?}"),
    )
}

fn chromium_load_stage(stage: ModuleLoadStage) -> module_tree::ModuleLoadStage {
    match stage {
        ModuleLoadStage::Fetch => module_tree::ModuleLoadStage::Fetch,
        ModuleLoadStage::Compile => module_tree::ModuleLoadStage::Compile,
        ModuleLoadStage::Resolve => module_tree::ModuleLoadStage::Resolve,
        ModuleLoadStage::Instantiate => module_tree::ModuleLoadStage::Instantiate,
        ModuleLoadStage::Evaluate => module_tree::ModuleLoadStage::Evaluate,
    }
}

fn local_load_stage(stage: module_tree::ModuleLoadStage) -> ModuleLoadStage {
    match stage {
        module_tree::ModuleLoadStage::Resolve => ModuleLoadStage::Resolve,
        module_tree::ModuleLoadStage::Fetch => ModuleLoadStage::Fetch,
        module_tree::ModuleLoadStage::Instantiate => ModuleLoadStage::Instantiate,
        module_tree::ModuleLoadStage::Evaluate => ModuleLoadStage::Evaluate,
        module_tree::ModuleLoadStage::Decode
        | module_tree::ModuleLoadStage::TypeCheck
        | module_tree::ModuleLoadStage::Compile
        | module_tree::ModuleLoadStage::DependencyDiscovery
        | module_tree::ModuleLoadStage::Link
        | module_tree::ModuleLoadStage::Abort => ModuleLoadStage::Compile,
    }
}

fn local_fetch_metadata(metadata: &module_tree::ModuleFetchMetadata) -> ModuleFetchMetadata {
    ModuleFetchMetadata {
        credentials_mode: match metadata.credentials_mode {
            module_tree::CredentialsMode::Omit => RequestCredentialsMode::Omit,
            module_tree::CredentialsMode::SameOrigin => RequestCredentialsMode::SameOrigin,
            module_tree::CredentialsMode::Include => RequestCredentialsMode::Include,
        },
        request_metadata: ScriptFetchRequestMetadata {
            cross_origin: None,
            referrer_policy: local_referrer_policy(metadata.referrer_policy).map(str::to_owned),
            document_referrer_policy: None,
            charset: metadata.charset.clone(),
            integrity: metadata.integrity.clone(),
            nonce: metadata.nonce.clone(),
            fetch_priority: match metadata.fetch_priority {
                module_tree::FetchPriorityHint::Auto => Some(moli_fetch::FetchPriorityHint::Auto),
                module_tree::FetchPriorityHint::Low => Some(moli_fetch::FetchPriorityHint::Low),
                module_tree::FetchPriorityHint::High => Some(moli_fetch::FetchPriorityHint::High),
            },
            scheduler_priority: metadata.scheduler_priority.map(|priority| match priority {
                module_tree::ScriptFetchSchedulerPriority::VeryHigh => {
                    ScriptFetchSchedulerPriority::VeryHigh
                }
                module_tree::ScriptFetchSchedulerPriority::High => {
                    ScriptFetchSchedulerPriority::High
                }
                module_tree::ScriptFetchSchedulerPriority::Normal => {
                    ScriptFetchSchedulerPriority::Auto
                }
                module_tree::ScriptFetchSchedulerPriority::Low => ScriptFetchSchedulerPriority::Low,
                module_tree::ScriptFetchSchedulerPriority::Background => {
                    ScriptFetchSchedulerPriority::Low
                }
            }),
        },
        parser_inserted: metadata.parser_inserted,
    }
}

fn local_referrer_policy(policy: module_tree::ReferrerPolicy) -> Option<&'static str> {
    match policy {
        module_tree::ReferrerPolicy::EmptyString => None,
        module_tree::ReferrerPolicy::NoReferrer => Some("no-referrer"),
        module_tree::ReferrerPolicy::NoReferrerWhenDowngrade => Some("no-referrer-when-downgrade"),
        module_tree::ReferrerPolicy::Origin => Some("origin"),
        module_tree::ReferrerPolicy::OriginWhenCrossOrigin => Some("origin-when-cross-origin"),
        module_tree::ReferrerPolicy::SameOrigin => Some("same-origin"),
        module_tree::ReferrerPolicy::StrictOrigin => Some("strict-origin"),
        module_tree::ReferrerPolicy::StrictOriginWhenCrossOrigin => {
            Some("strict-origin-when-cross-origin")
        }
        module_tree::ReferrerPolicy::UnsafeUrl => Some("unsafe-url"),
    }
}

fn module_key_for_root(
    root: &ModuleRootInput,
) -> std::result::Result<ModuleMapKey, ModuleLoadError> {
    let key = module_key_for_attributes(&root.source_url, &root.attributes).map_err(|message| {
        ModuleLoadError::new(
            ModuleLoadStage::Resolve,
            format!("{message} for module `{}`", root.source_url),
        )
        .with_error_constructor(ScriptErrorConstructorKind::TypeError)
    })?;
    validate_source_phase_key(root.phase, &key, "module", root.source_url.as_str())?;
    Ok(key)
}

fn module_key_for_attributes(
    url: &Url,
    attributes: &ModuleAttributesKey,
) -> std::result::Result<ModuleMapKey, String> {
    ModuleMapKey::from_url_and_attributes(url, attributes)
}

fn validate_source_phase_key(
    phase: ModuleImportPhase,
    key: &ModuleMapKey,
    label: &str,
    subject: &str,
) -> std::result::Result<(), ModuleLoadError> {
    if phase == ModuleImportPhase::Source && key.kind() != ModuleKind::WebAssembly {
        return Err(ModuleLoadError::new(
            ModuleLoadStage::Resolve,
            format!("source-phase {label} `{subject}` does not resolve to a WebAssembly module"),
        )
        .with_error_constructor(ScriptErrorConstructorKind::SyntaxError));
    }
    Ok(())
}

pub(crate) async fn execute_native_module_script_source(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<ModuleScriptExecutionOutcome, ModuleLoadError> {
    let job = module_script_graph_job_for_owner(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        completion_owner,
    )?;
    execute_native_module_script_graph_job(vm, job)
}

pub(crate) async fn execute_external_native_module_script_graph(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<ModuleScriptExecutionOutcome, ModuleLoadError> {
    let job = external_module_script_graph_job(
        vm,
        base_url,
        initiator_url,
        fetch_metadata,
        completion_owner,
    );
    execute_native_module_script_graph_job(vm, job)
}

fn execute_native_module_script_graph_job(
    vm: &mut ScriptVm,
    job: NativeModuleGraphJob,
) -> std::result::Result<ModuleScriptExecutionOutcome, ModuleLoadError> {
    let advance = advance_module_script_graph(vm, job)?;
    match advance {
        ModuleScriptGraphAdvance::NeedFetches(batch) => {
            Ok(ModuleScriptExecutionOutcome::SuspendedModuleFetches(batch))
        }
        ModuleScriptGraphAdvance::Complete(graph) => {
            Ok(ModuleScriptExecutionOutcome::CompletedModuleGraph(graph))
        }
    }
}

fn module_script_root_input_with_source_override(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
    completion_owner: ModuleScriptCompletionOwner,
) -> ModuleRootInput {
    let source_url = if source_is_external {
        base_url.clone()
    } else {
        next_inline_module_url(vm, base_url)
    };
    ModuleRootInput {
        source_url,
        base_url: base_url.clone(),
        initiator_url: initiator_url.clone(),
        attributes: ModuleAttributesKey::empty(),
        phase: ModuleImportPhase::Evaluation,
        source_override: Some(source),
        fetch_metadata: ModuleFetchMetadata::from_loaded_module_script_fetch_metadata(
            fetch_metadata,
        ),
        parser_owned: completion_owner == ModuleScriptCompletionOwner::Parser,
    }
}

fn external_module_script_root_input(
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    completion_owner: ModuleScriptCompletionOwner,
) -> ModuleRootInput {
    ModuleRootInput {
        source_url: base_url.clone(),
        base_url: base_url.clone(),
        initiator_url: initiator_url.clone(),
        attributes: ModuleAttributesKey::empty(),
        phase: ModuleImportPhase::Evaluation,
        source_override: None,
        fetch_metadata: ModuleFetchMetadata::from_top_level_script_fetch_metadata(fetch_metadata),
        parser_owned: completion_owner == ModuleScriptCompletionOwner::Parser,
    }
}

fn external_chromium_tree_root_input(
    root: &ModuleRootInput,
) -> std::result::Result<Option<moli_module_script_tree::ModuleRootInput>, ModuleLoadError> {
    if root.source_override.is_some() {
        return Ok(None);
    }
    let key = module_key_for_root(root)?;
    Ok(Some(moli_module_script_tree::ModuleRootInput::External(
        moli_module_script_tree::ModuleExternalRootInput {
            source_url: root.source_url.clone(),
            base_url: root.base_url.clone(),
            initiator_url: root.initiator_url.clone(),
            attributes: chromium_attributes(&root.attributes),
            phase: chromium_import_phase(root.phase),
            kind_hint: Some(chromium_module_kind(key.kind())),
            fetch_metadata: chromium_fetch_metadata(&root.fetch_metadata),
            referrer: moli_module_script_tree::ModuleReferrer::client(),
            position: moli_module_script_tree::TextPosition::default(),
        },
    )))
}

fn chromium_attributes(
    attributes: &ModuleAttributesKey,
) -> moli_module_script_tree::ModuleAttributesKey {
    moli_module_script_tree::ModuleAttributesKey::from_pairs(attributes.pairs().to_vec())
}

fn chromium_module_kind(kind: ModuleKind) -> moli_module_script_tree::ModuleKind {
    match kind {
        ModuleKind::JavaScript => moli_module_script_tree::ModuleKind::JavaScript,
        ModuleKind::Json => moli_module_script_tree::ModuleKind::Json,
        ModuleKind::Css => moli_module_script_tree::ModuleKind::Css,
        ModuleKind::ModulePreloadText => moli_module_script_tree::ModuleKind::JavaScript,
        ModuleKind::WebAssembly => moli_module_script_tree::ModuleKind::WebAssembly,
    }
}

fn chromium_import_phase(phase: ModuleImportPhase) -> moli_module_script_tree::ModuleImportPhase {
    match phase {
        ModuleImportPhase::Evaluation => moli_module_script_tree::ModuleImportPhase::Evaluation,
        ModuleImportPhase::Source => moli_module_script_tree::ModuleImportPhase::Source,
    }
}

fn local_import_phase(phase: moli_module_script_tree::ModuleImportPhase) -> ModuleImportPhase {
    match phase {
        moli_module_script_tree::ModuleImportPhase::Evaluation => ModuleImportPhase::Evaluation,
        moli_module_script_tree::ModuleImportPhase::Source => ModuleImportPhase::Source,
    }
}

fn chromium_fetch_metadata(
    metadata: &ModuleFetchMetadata,
) -> moli_module_script_tree::ModuleFetchMetadata {
    moli_module_script_tree::ModuleFetchMetadata {
        credentials_mode: match metadata.credentials_mode {
            RequestCredentialsMode::Omit => moli_module_script_tree::CredentialsMode::Omit,
            RequestCredentialsMode::SameOrigin => {
                moli_module_script_tree::CredentialsMode::SameOrigin
            }
            RequestCredentialsMode::Include => moli_module_script_tree::CredentialsMode::Include,
        },
        referrer_policy: chromium_referrer_policy(
            metadata.request_metadata.referrer_policy.as_deref(),
        ),
        integrity: metadata.request_metadata.integrity.clone(),
        nonce: metadata.request_metadata.nonce.clone(),
        charset: metadata.request_metadata.charset.clone(),
        fetch_priority: match metadata.request_metadata.fetch_priority {
            Some(moli_fetch::FetchPriorityHint::Low) => {
                moli_module_script_tree::FetchPriorityHint::Low
            }
            Some(moli_fetch::FetchPriorityHint::High) => {
                moli_module_script_tree::FetchPriorityHint::High
            }
            Some(moli_fetch::FetchPriorityHint::Auto) | None => {
                moli_module_script_tree::FetchPriorityHint::Auto
            }
        },
        scheduler_priority: metadata
            .request_metadata
            .scheduler_priority
            .map(|priority| match priority {
                ScriptFetchSchedulerPriority::VeryHigh => {
                    moli_module_script_tree::ScriptFetchSchedulerPriority::VeryHigh
                }
                ScriptFetchSchedulerPriority::High => {
                    moli_module_script_tree::ScriptFetchSchedulerPriority::High
                }
                ScriptFetchSchedulerPriority::Low => {
                    moli_module_script_tree::ScriptFetchSchedulerPriority::Low
                }
                ScriptFetchSchedulerPriority::Auto => {
                    moli_module_script_tree::ScriptFetchSchedulerPriority::Normal
                }
            }),
        request_context: moli_module_script_tree::ModuleRequestContext::Script,
        destination: moli_module_script_tree::ModuleRequestDestination::Script,
        parser_inserted: metadata.parser_inserted,
    }
}

fn chromium_referrer_policy(policy: Option<&str>) -> moli_module_script_tree::ReferrerPolicy {
    match policy {
        Some("no-referrer") => moli_module_script_tree::ReferrerPolicy::NoReferrer,
        Some("no-referrer-when-downgrade") => {
            moli_module_script_tree::ReferrerPolicy::NoReferrerWhenDowngrade
        }
        Some("origin") => moli_module_script_tree::ReferrerPolicy::Origin,
        Some("origin-when-cross-origin") => {
            moli_module_script_tree::ReferrerPolicy::OriginWhenCrossOrigin
        }
        Some("same-origin") => moli_module_script_tree::ReferrerPolicy::SameOrigin,
        Some("strict-origin") => moli_module_script_tree::ReferrerPolicy::StrictOrigin,
        Some("strict-origin-when-cross-origin") => {
            moli_module_script_tree::ReferrerPolicy::StrictOriginWhenCrossOrigin
        }
        Some("unsafe-url") => moli_module_script_tree::ReferrerPolicy::UnsafeUrl,
        _ => moli_module_script_tree::ReferrerPolicy::EmptyString,
    }
}

#[cfg(test)]
fn module_script_inline_tree_job(
    vm: &mut ScriptVm,
    root: ModuleRootInput,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    module_script_inline_tree_job_for_owner(vm, root, ModuleScriptCompletionOwner::Parser)
}

fn module_script_inline_tree_job_for_owner(
    vm: &mut ScriptVm,
    root: ModuleRootInput,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    let trace_label = match completion_owner {
        ModuleScriptCompletionOwner::Parser => "parser_owned_inline",
        ModuleScriptCompletionOwner::Runtime => "runtime_module_script_inline",
    };
    trace_module_graph_job_created(trace_label, &root);
    let key = module_key_for_root(&root)?;
    vm.dispatch_module_fetch_csp_report_only_violation_for_owner(&key, &root.fetch_metadata);
    if let Some(error) = vm.csp_blocked_module_fetch_error_for_owner(&key, &root.fetch_metadata) {
        vm.document_runtime
            .mark_native_module_failed(key.clone(), error.clone());
        return Err(error);
    }
    if let Some(entry) = reusable_inline_module_entry(vm, &key)? {
        return Ok(native_module_graph_job_for_inline_entry(
            &root,
            &key,
            entry,
            completion_owner,
        ));
    }
    let source = root.source_override.clone().ok_or_else(|| {
        ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            format!("module `{}` has no source override", root.source_url),
        )
    })?;
    vm.document_runtime.insert_native_module_source_for_request(
        key.clone(),
        key.clone(),
        source.clone(),
        root.fetch_metadata.clone(),
    );
    let (record, identity) = match vm.compile_native_module_record(
        key.clone(),
        &source,
        &root.base_url,
        &root.fetch_metadata,
    ) {
        Ok(result) => result,
        Err(error) => {
            vm.document_runtime
                .mark_native_module_failed(key.clone(), error.clone());
            return Err(error);
        }
    };
    let entry = vm
        .document_runtime
        .insert_native_compiled_module_record_with_metadata(
            key.clone(),
            record,
            identity,
            root.fetch_metadata.clone(),
        );
    Ok(native_module_graph_job_for_inline_entry(
        &root,
        &key,
        entry,
        completion_owner,
    ))
}

fn reusable_inline_module_entry(
    vm: &ScriptVm,
    key: &ModuleMapKey,
) -> std::result::Result<Option<ModuleEntryId>, ModuleLoadError> {
    let Some(entry) = vm.document_runtime.native_module_entry_id(key) else {
        return Ok(None);
    };
    match vm.document_runtime.native_module_entry_state(entry) {
        ModuleMapEntryState::Compiled
        | ModuleMapEntryState::Instantiated
        | ModuleMapEntryState::Evaluating
        | ModuleMapEntryState::Evaluated => Ok(Some(entry)),
        ModuleMapEntryState::Failed => Err(vm
            .document_runtime
            .native_module_failure(entry)
            .unwrap_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    format!("module `{}` previously failed to load", key.url()),
                )
            })),
        ModuleMapEntryState::Fetching | ModuleMapEntryState::Fetched => Ok(None),
    }
}

fn native_module_graph_job_for_inline_entry(
    root: &ModuleRootInput,
    key: &ModuleMapKey,
    entry: ModuleEntryId,
    completion_owner: ModuleScriptCompletionOwner,
) -> NativeModuleGraphJob {
    let chromium_root = moli_module_script_tree::ModuleRootInput::Inline(
        moli_module_script_tree::ModuleInlineRootInput {
            root_key: chromium_module_key(key),
            root_entry: chromium_entry_id(entry),
            source_url: root.source_url.clone(),
            base_url: root.base_url.clone(),
            phase: chromium_import_phase(root.phase),
            fetch_metadata: chromium_fetch_metadata(&root.fetch_metadata),
            referrer: moli_module_script_tree::ModuleReferrer::client(),
            position: moli_module_script_tree::TextPosition::default(),
        },
    );
    let chromium_tree = match completion_owner {
        ModuleScriptCompletionOwner::Parser => tree_adapter::parser_owned_tree_job(chromium_root),
        ModuleScriptCompletionOwner::Runtime => {
            tree_adapter::runtime_module_script_tree_job(chromium_root)
        }
    };
    let kind = match completion_owner {
        ModuleScriptCompletionOwner::Parser => NativeModuleGraphJobKind::ParserOwned,
        ModuleScriptCompletionOwner::Runtime => NativeModuleGraphJobKind::RuntimeModuleScript,
    };
    NativeModuleGraphJob {
        tree_job: Some(NativeModuleTreeJob::new(chromium_tree)),
        kind,
    }
}

#[cfg(test)]
fn module_script_graph_job(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    module_script_graph_job_for_owner(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        ModuleScriptCompletionOwner::Parser,
    )
}

fn module_script_graph_job_for_owner(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
    completion_owner: ModuleScriptCompletionOwner,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    let root = module_script_root_input_with_source_override(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        completion_owner,
    );
    module_script_inline_tree_job_for_owner(vm, root, completion_owner)
}

pub(crate) fn parser_owned_loaded_module_script_graph_job(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    module_script_graph_job_for_owner(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        ModuleScriptCompletionOwner::Parser,
    )
}

pub(crate) fn runtime_owned_loaded_module_script_graph_job(
    vm: &mut ScriptVm,
    source: ModuleSource,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    source_is_external: bool,
) -> std::result::Result<NativeModuleGraphJob, ModuleLoadError> {
    module_script_graph_job_for_owner(
        vm,
        source,
        base_url,
        initiator_url,
        fetch_metadata,
        source_is_external,
        ModuleScriptCompletionOwner::Runtime,
    )
}

fn external_module_script_graph_job(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
    completion_owner: ModuleScriptCompletionOwner,
) -> NativeModuleGraphJob {
    let integrity = super::resolve_module_integrity(vm, base_url);
    let root = external_module_script_root_input(
        base_url,
        initiator_url,
        fetch_metadata,
        completion_owner,
    )
    .with_import_map_integrity_if_absent(integrity);
    match completion_owner {
        ModuleScriptCompletionOwner::Parser => NativeModuleGraphJob::parser_owned(root),
        ModuleScriptCompletionOwner::Runtime => NativeModuleGraphJob::runtime_module_script(root),
    }
}

pub(crate) fn parser_owned_external_module_script_graph_job(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
) -> NativeModuleGraphJob {
    external_module_script_graph_job(
        vm,
        base_url,
        initiator_url,
        fetch_metadata,
        ModuleScriptCompletionOwner::Parser,
    )
}

pub(crate) fn runtime_owned_external_module_script_graph_job(
    vm: &mut ScriptVm,
    base_url: &Url,
    initiator_url: &Url,
    fetch_metadata: &ScriptFetchMetadata,
) -> NativeModuleGraphJob {
    external_module_script_graph_job(
        vm,
        base_url,
        initiator_url,
        fetch_metadata,
        ModuleScriptCompletionOwner::Runtime,
    )
}

pub(crate) fn advance_module_script_graph(
    vm: &mut ScriptVm,
    mut job: NativeModuleGraphJob,
) -> std::result::Result<ModuleScriptGraphAdvance, ModuleLoadError> {
    let advance = job.advance_module_script_owner_lane(vm)?;
    Ok(module_script_graph_advance_from_native(job, advance))
}

pub(crate) fn module_script_graph_advance_from_native(
    job: NativeModuleGraphJob,
    advance: NativeModuleGraphJobAdvance,
) -> ModuleScriptGraphAdvance {
    match advance {
        NativeModuleGraphJobAdvance::NeedFetches(requests) => {
            ModuleScriptGraphAdvance::NeedFetches(Box::new(ModuleScriptGraphFetchBatch::new(
                job,
                requests
                    .into_iter()
                    .map(|request| ModuleScriptGraphFetchContinuation { request })
                    .collect(),
            )))
        }
        NativeModuleGraphJobAdvance::WaitingForFetches => ModuleScriptGraphAdvance::NeedFetches(
            Box::new(ModuleScriptGraphFetchBatch::new(job, Vec::new())),
        ),
        NativeModuleGraphJobAdvance::Complete(graph) => ModuleScriptGraphAdvance::Complete(graph),
    }
}

pub(crate) fn next_inline_module_url(vm: &mut ScriptVm, base_url: &Url) -> Url {
    let mut inline_url = base_url.clone();
    inline_url.set_fragment(Some(&format!(
        "__moli_inline_module_{}",
        next_inline_module_eval_id(vm)
    )));
    inline_url
}

fn dynamic_import_root_input<O>(
    owner: &mut O,
    request: &PendingDynamicModuleImport,
) -> std::result::Result<ModuleRootInput, ModuleLoadError>
where
    O: NativeModuleTreeDocumentOwnerAdapter,
{
    let source_url = match request.resolved_url() {
        Some(url) => url.clone(),
        None => owner
            .resolve_module_specifier(request.specifier(), request.base_url())
            .map_err(|error| {
                ModuleLoadError::new(
                    ModuleLoadStage::Resolve,
                    format!(
                        "failed to resolve dynamic import `{}` from `{}`: {error}",
                        request.specifier(),
                        request.base_url()
                    ),
                )
                .with_error_constructor(ScriptErrorConstructorKind::TypeError)
            })?,
    };
    let integrity = owner.resolve_module_integrity(&source_url);
    let initiator_url = owner.module_request_initiator_url(request.child_browsing_context_handle());
    Ok(ModuleRootInput {
        source_url: source_url.clone(),
        base_url: source_url,
        initiator_url,
        attributes: request.attributes().clone(),
        phase: request.phase(),
        source_override: None,
        fetch_metadata: ModuleFetchMetadata::from_dynamic_import_referrer_fetch_metadata(
            request.fetch_metadata(),
        )
        .with_import_map_integrity(integrity),
        parser_owned: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_runtime::NativeModulepreloadFetchStart;
    use crate::{
        dom::native::{DomHost, NativeDom},
        module_runtime::NativeModuleSingleFetchRequest,
        network::{
            RendererResourceTaskRunner, ResourceRequestClient,
            loads::{ResourceLoadDisposition, ResourceLoadKind, ResourceLoadRegistry},
        },
        script_vm::{ScriptVmDefaultWorldBootstrap, StandaloneScriptVmHarness},
        types::{ModuleGraphFetchCompletion, ModuleGraphFetchOrdering, ModuleGraphFetchRequester},
    };
    use moli_fetch::FetchConfig;
    use tokio::sync::oneshot;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("test url should parse")
    }

    fn new_test_vm(url: &str) -> StandaloneScriptVmHarness {
        let _js_runtime = crate::JsRuntime::initialize();
        let page_task_queue = crate::page_task_queue::PageTaskQueueTestHarness::new();
        let post_domcontentloaded_page_task_sender =
            page_task_queue.owner_attached_runtime_page_task_sender_for_test();
        let page_task_front_injection_tx = page_task_queue.parser_boundary_sender();
        ScriptVmDefaultWorldBootstrap::standalone_from_dom_host_for_test(
            DomHost::from_dom(NativeDom::new(url::Url::parse(url).expect("test url"))),
            post_domcontentloaded_page_task_sender,
            page_task_front_injection_tx,
        )
        .expect("script vm bootstrap should succeed")
        .finish()
        .expect("script vm finish should succeed")
    }

    #[test]
    fn native_json_module_fetch_request_uses_json_destination_metadata() {
        let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
            url("https://app.example.test/data.json"),
            url("https://app.example.test/page"),
            ModuleFetchMetadata::default(),
            ModuleKind::Json,
        )
        .request()
        .expect("native JSON module request should build");

        assert_eq!(
            fetch_request.browser_request_metadata(),
            Some(BrowserRequestMetadata::JsonModule)
        );
    }

    #[test]
    fn native_css_module_fetch_request_uses_style_destination_metadata() {
        let fetch_request = NativeModuleGraphFetchRequest::new_for_test(
            url("https://app.example.test/sheet.css"),
            url("https://app.example.test/page"),
            ModuleFetchMetadata::default(),
            ModuleKind::Css,
        )
        .request()
        .expect("native CSS module request should build");

        assert_eq!(
            fetch_request.resource_type,
            RequestResourceType::CssStyleSheet
        );
        assert_eq!(
            fetch_request.browser_request_metadata(),
            Some(BrowserRequestMetadata::StyleModule)
        );
    }

    #[test]
    fn modulepreload_json_single_fetch_uses_json_destination_metadata() {
        let root_url = url("https://app.example.test/data.json");
        let single_fetch = NativeModuleSingleFetchRequest::new(
            root_url.clone(),
            root_url.clone(),
            url("https://app.example.test/page"),
            ModuleMapKey::json_with_attributes(root_url, ModuleAttributesKey::empty()),
            ModuleFetchMetadata::default(),
        );
        let graph_request = single_fetch.fetch_request();
        let fetch_request = graph_request
            .request()
            .expect("modulepreload JSON fetch request should build");

        assert_eq!(
            fetch_request.browser_request_metadata(),
            Some(BrowserRequestMetadata::JsonModule)
        );
    }

    #[test]
    fn modulepreload_css_single_fetch_uses_style_destination_metadata() {
        let root_url = url("https://app.example.test/sheet.css");
        let single_fetch = NativeModuleSingleFetchRequest::new(
            root_url.clone(),
            root_url.clone(),
            url("https://app.example.test/page"),
            ModuleMapKey::css_with_attributes(root_url, ModuleAttributesKey::empty()),
            ModuleFetchMetadata::default(),
        );
        let graph_request = single_fetch.fetch_request();
        let fetch_request = graph_request
            .request()
            .expect("modulepreload CSS fetch request should build");

        assert_eq!(
            fetch_request.resource_type,
            RequestResourceType::CssStyleSheet
        );
        assert_eq!(
            fetch_request.browser_request_metadata(),
            Some(BrowserRequestMetadata::StyleModule)
        );
    }

    fn modulepreload_single_fetch_request(root_url: Url) -> NativeModuleSingleFetchRequest {
        modulepreload_single_fetch_request_with_metadata(root_url, ModuleFetchMetadata::default())
    }

    fn modulepreload_single_fetch_request_with_metadata(
        root_url: Url,
        metadata: ModuleFetchMetadata,
    ) -> NativeModuleSingleFetchRequest {
        let module_key = ModuleMapKey::java_script(root_url.clone());
        NativeModuleSingleFetchRequest::new(
            root_url.clone(),
            root_url,
            url("https://app.example.test/page"),
            module_key,
            metadata,
        )
    }

    fn suspend_registered_modulepreload_fetch(
        vm: &mut ScriptVm,
        request: NativeModuleSingleFetchRequest,
    ) -> u64 {
        let start = vm
            .document_runtime
            .fetch_single_native_module_for_modulepreload(request)
            .expect("modulepreload single fetch registration should succeed");
        let NativeModulepreloadFetchStart::Started(request) = start else {
            panic!("new modulepreload single fetch should start network scheduling");
        };
        vm.document_runtime
            .suspend_native_modulepreload_fetch(*request)
    }

    fn wasm_root_input(
        source_url: &str,
        bytes: &[u8],
        phase: ModuleImportPhase,
    ) -> ModuleRootInput {
        root_input(source_url, ModuleSource::binary(bytes.to_vec()), phase)
    }

    fn root_input(
        source_url: &str,
        source: ModuleSource,
        phase: ModuleImportPhase,
    ) -> ModuleRootInput {
        let source_url = url(source_url);
        ModuleRootInput {
            source_url: source_url.clone(),
            base_url: source_url,
            initiator_url: url("https://app.example.test/page"),
            attributes: ModuleAttributesKey::empty(),
            phase,
            source_override: Some(source),
            fetch_metadata: ModuleFetchMetadata::default(),
            parser_owned: true,
        }
    }

    fn expect_single_fetch(
        advance: ModuleScriptGraphAdvance,
        context: &str,
    ) -> Box<ModuleScriptGraphFetchBatch> {
        match advance {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                assert_eq!(
                    fetches.fetches.len(),
                    1,
                    "{context} should request exactly one fetch"
                );
                fetches
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("{context} should not complete before fetch")
            }
        }
    }

    fn expect_single_joined_wait(
        advance: ModuleScriptGraphAdvance,
        context: &str,
    ) -> NativeModuleGraphJob {
        match advance {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert!(
                    fetches.is_empty(),
                    "{context} should not request owned fetches while waiting for a joined module map fetch"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    1,
                    "{context} should wait for exactly one joined module map fetch"
                );
                job
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("{context} should not complete before joined fetch")
            }
        }
    }

    #[test]
    fn parser_external_graph_job_carries_chromium_tree_shadow_state() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &url("https://app.example.test/root.mjs"),
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        assert!(
            job.has_chromium_tree_for_test(),
            "external parser module roots should create the new module tree job"
        );
    }

    #[test]
    fn source_override_graph_job_uses_inline_chromium_tree_in_production_path() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text("export const value = 1;".to_owned()),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("source override root should compile into inline tree");

        assert!(
            job.has_chromium_tree_for_test(),
            "production source_override roots should use an inline module tree"
        );
        let graph = match advance_module_script_graph(&mut vm, job)
            .expect("source override graph should complete without network fetch")
        {
            ModuleScriptGraphAdvance::Complete(graph) => graph,
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (_, fetches) = fetches.into_parts();
                panic!(
                    "source override without dependencies should not fetch {} modules",
                    fetches.len()
                )
            }
        };
        let entry = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("source override root should be in module map");
        assert_eq!(graph.root_entry, entry);
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry),
            ModuleMapEntryState::Compiled
        );
    }

    #[test]
    fn source_override_tree_fetches_sibling_dependencies_in_one_batch() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./a.mjs";
import "./b.mjs";
import "./c.mjs";
"#
                .to_owned(),
            ),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("source override root should compile into inline tree");

        match advance_module_script_graph(&mut vm, job)
            .expect("source override graph should request sibling dependencies")
        {
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("source override graph should not complete before dependency fetches")
            }
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (_, fetches) = fetches.into_parts();
                let urls: Vec<_> = fetches
                    .iter()
                    .map(|fetch| fetch.request().source_url().as_str())
                    .collect();
                assert_eq!(
                    urls,
                    vec![
                        "https://app.example.test/a.mjs",
                        "https://app.example.test/b.mjs",
                        "https://app.example.test/c.mjs",
                    ]
                );
            }
        }
    }

    #[test]
    fn module_fetch_metadata_defaults_to_same_origin_credentials() {
        let metadata = ModuleFetchMetadata::default();
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://cdn.example.test/dep.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        assert_eq!(
            request.credentials_mode,
            RequestCredentialsMode::SameOrigin,
            "module graph fetches should not use Request::new's Include default"
        );
        assert_eq!(
            request.cookie_context.initiator_url.as_ref(),
            Some(&url("https://app.example.test/page")),
            "same-origin credentials need the document initiator to classify the target URL"
        );
    }

    #[test]
    fn module_fetch_metadata_maps_use_credentials_to_include() {
        let script_metadata = ScriptFetchMetadata {
            cross_origin: Some("use-credentials".to_owned()),
            referrer_policy: Some("no-referrer".to_owned()),
            fetch_priority: Some(moli_fetch::FetchPriorityHint::High),
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_parser_owned_script_fetch_metadata(&script_metadata);
        assert_eq!(
            metadata.request_metadata.cross_origin.as_deref(),
            Some("use-credentials")
        );
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/dep.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        assert_eq!(request.credentials_mode, RequestCredentialsMode::Include);
        let request_metadata = request
            .subresource_request_metadata()
            .expect("module graph request should carry generic request metadata");
        assert_eq!(
            request_metadata.referrer_policy.as_deref(),
            Some("no-referrer")
        );
        assert_eq!(
            request.priority_hints.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::High)
        );
        assert_eq!(
            request_metadata.integrity, None,
            "top-level script integrity must not be inherited by every graph dependency"
        );
    }

    #[test]
    fn top_level_module_fetch_metadata_preserves_element_only_options() {
        let script_metadata = ScriptFetchMetadata {
            charset: Some("utf-8".to_owned()),
            integrity: Some("sha384-entry".to_owned()),
            nonce: Some("nonce-entry".to_owned()),
            parser_inserted: true,
            ..ScriptFetchMetadata::default()
        };
        let metadata = ModuleFetchMetadata::from_top_level_script_fetch_metadata(&script_metadata);
        assert_eq!(metadata.request_metadata.charset.as_deref(), Some("utf-8"));
        assert_eq!(
            metadata.request_metadata.nonce.as_deref(),
            Some("nonce-entry")
        );
        assert!(metadata.parser_inserted);
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/entry.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        let request_metadata = request
            .subresource_request_metadata()
            .expect("module graph request should carry generic request metadata");
        assert_eq!(request_metadata.integrity.as_deref(), Some("sha384-entry"));
    }

    #[test]
    fn modulepreload_fetch_metadata_uses_auto_priority() {
        let script_metadata = ScriptFetchMetadata {
            cross_origin: Some("use-credentials".to_owned()),
            referrer_policy: Some("no-referrer".to_owned()),
            integrity: Some("sha384-preload".to_owned()),
            fetch_priority: Some(moli_fetch::FetchPriorityHint::High),
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_modulepreload_script_fetch_metadata(&script_metadata);
        assert_eq!(
            metadata.request_metadata.fetch_priority, None,
            "Chromium ModulePreloadIfNeeded passes FetchPriorityHint::kAuto regardless of the link fetchpriority attribute"
        );
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/preload.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        assert_eq!(request.credentials_mode, RequestCredentialsMode::Include);
        let request_metadata = request
            .subresource_request_metadata()
            .expect("modulepreload request should carry generic request metadata");
        assert_eq!(
            request_metadata.referrer_policy.as_deref(),
            Some("no-referrer")
        );
        assert_eq!(
            request_metadata.integrity.as_deref(),
            Some("sha384-preload")
        );
        assert_eq!(
            request.priority_hints.fetch_priority, None,
            "modulepreload priority lowering should not leave a request hint"
        );
    }

    #[test]
    fn descendant_module_fetch_metadata_preserves_csp_provenance() {
        let script_metadata = ScriptFetchMetadata {
            cross_origin: Some("use-credentials".to_owned()),
            charset: Some("utf-8".to_owned()),
            integrity: Some("sha384-entry".to_owned()),
            nonce: Some("nonce-entry".to_owned()),
            fetch_priority: Some(moli_fetch::FetchPriorityHint::High),
            parser_inserted: true,
            ..ScriptFetchMetadata::default()
        };
        let dependency_metadata =
            ModuleFetchMetadata::from_top_level_script_fetch_metadata(&script_metadata)
                .for_descendant_fetches();
        assert_eq!(dependency_metadata.request_metadata.charset, None);
        assert_eq!(dependency_metadata.request_metadata.integrity, None);
        assert_eq!(dependency_metadata.nonce(), Some("nonce-entry"));
        assert!(dependency_metadata.parser_inserted);
        assert_eq!(
            dependency_metadata.request_metadata.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::Auto)
        );
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/dep.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: dependency_metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        let request_metadata = request
            .subresource_request_metadata()
            .expect("module graph request should carry generic request metadata");
        assert_eq!(request.credentials_mode, RequestCredentialsMode::Include);
        assert_eq!(
            request.priority_hints.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::Auto)
        );
        assert_eq!(request_metadata.integrity, None);
    }

    #[test]
    fn dynamic_import_root_preserves_referrer_nonce() {
        let script_metadata = ScriptFetchMetadata {
            nonce: Some("nonce-entry".to_owned()),
            charset: Some("utf-8".to_owned()),
            integrity: Some("sha384-entry".to_owned()),
            parser_inserted: true,
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_dynamic_import_referrer_fetch_metadata(&script_metadata);
        assert_eq!(metadata.nonce(), Some("nonce-entry"));
        assert!(metadata.parser_inserted);
        assert_eq!(
            metadata.request_metadata.charset, None,
            "dynamic import root must not inherit parser-only charset"
        );
        assert_eq!(
            metadata.request_metadata.integrity, None,
            "dynamic import root integrity comes from import maps, not the referrer script element"
        );
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/dynamic.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        let request_metadata = request
            .subresource_request_metadata()
            .expect("dynamic import root should carry generic request metadata");
        assert_eq!(request_metadata.integrity, None);
    }

    #[test]
    fn loaded_module_script_source_preserves_nonce_without_element_fetch_options() {
        let script_metadata = ScriptFetchMetadata {
            nonce: Some("nonce-entry".to_owned()),
            charset: Some("utf-8".to_owned()),
            integrity: Some("sha384-entry".to_owned()),
            parser_inserted: true,
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_loaded_module_script_fetch_metadata(&script_metadata);
        assert_eq!(metadata.nonce(), Some("nonce-entry"));
        assert!(metadata.parser_inserted);
        assert_eq!(metadata.request_metadata.charset, None);
        assert_eq!(metadata.request_metadata.integrity, None);
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/inline.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        let request_metadata = request
            .subresource_request_metadata()
            .expect("loaded module source should carry generic request metadata");
        assert_eq!(request_metadata.integrity, None);
    }

    #[test]
    fn parser_owned_module_fetch_metadata_uses_default_script_priority() {
        let script_metadata = ScriptFetchMetadata {
            fetch_priority: Some(moli_fetch::FetchPriorityHint::Low),
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_parser_owned_script_fetch_metadata(&script_metadata);
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://app.example.test/entry.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        assert_eq!(
            request.priority_hints.fetch_priority,
            Some(moli_fetch::FetchPriorityHint::Low)
        );
        assert_eq!(
            request.resource_type,
            moli_fetch::RequestResourceType::Script,
            "module graph requests use Chromium's default script priority before author hints"
        );
    }

    #[test]
    fn external_parser_module_graph_starts_with_root_fetching_entry() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/entry");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser module graph should advance"),
            "external module root",
        );
        assert_eq!(fetch.request().source_url(), &root_url);
        assert_eq!(fetch.pending_fetch_key(), Some(&root_key));

        let entry = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("external root fetch should reserve module map entry");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry),
            ModuleMapEntryState::Fetching
        );
    }

    #[test]
    fn external_parser_module_tree_fetches_and_compiles_dependency_graph() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let dep_url = url("https://app.example.test/dep.mjs");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let dep_key = ModuleMapKey::java_script(dep_url.clone());
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser graph should request root"),
            "external parser root",
        );
        assert_eq!(root_fetch.request().source_url(), &root_url);
        assert!(
            root_fetch.request().dependency.is_none(),
            "top-level root fetch should not carry dependency parent payload"
        );
        let dep_fetch = expect_single_fetch(
            root_fetch
                .finish_source_for_test(
                    &mut vm,
                    Ok(ModuleSource::text(
                        "import { value } from './dep.mjs'; export { value };".to_owned(),
                    )),
                )
                .expect("root fetch should compile and discover dependency"),
            "single dependency graph",
        );
        assert_eq!(dep_fetch.request().source_url(), &dep_url);
        let root_entry_before_dependency = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("root should be in module map after root fetch completion");
        let dependency = dep_fetch
            .request()
            .dependency
            .as_ref()
            .expect("dependent fetch should retain tree parent payload");
        assert_eq!(dependency.parent_key(), &root_key);
        assert_eq!(dependency.parent_entry_id(), root_entry_before_dependency);
        assert_eq!(dependency.specifier(), "./dep.mjs");
        assert_eq!(dependency.phase(), ModuleImportPhase::Evaluation);
        let graph = match dep_fetch
            .finish_source_for_test(
                &mut vm,
                Ok(ModuleSource::text("export const value = 1;".to_owned())),
            )
            .expect("dependency fetch should compile")
        {
            ModuleScriptGraphAdvance::Complete(graph) => graph,
            ModuleScriptGraphAdvance::NeedFetches(_) => {
                panic!("external parser graph should not request more fetches")
            }
        };

        let root_entry = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("root should be in module map");
        let dep_entry = vm
            .document_runtime
            .native_module_entry_id(&dep_key)
            .expect("dependency should be in module map");
        assert_eq!(graph.root_entry, root_entry);
        assert_eq!(
            vm.document_runtime.native_module_entry_state(root_entry),
            ModuleMapEntryState::Compiled
        );
        assert_eq!(
            vm.document_runtime.native_module_entry_state(dep_entry),
            ModuleMapEntryState::Compiled
        );
        assert_eq!(
            vm.document_runtime
                .native_module_resolved_dependencies(root_entry)
                .len(),
            1
        );
    }

    #[test]
    fn static_bare_specifier_resolve_failure_is_a_type_error() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser graph should request root"),
            "bare-specifier root",
        );
        let error = match root_fetch.finish_source_for_test(
            &mut vm,
            Ok(ModuleSource::text(
                r#"import "unmapped-bare-specifier";"#.to_owned(),
            )),
        ) {
            Ok(_) => panic!("an unmapped bare specifier should fail during graph resolution"),
            Err(error) => error,
        };

        assert_eq!(error.stage(), ModuleLoadStage::Resolve);
        assert_eq!(
            error.error_constructor(),
            Some(ScriptErrorConstructorKind::TypeError)
        );
        assert!(
            error.message().contains("unmapped-bare-specifier"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn static_import_with_invalid_attribute_key_fails_before_dependency_fetch() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser graph should request root"),
            "invalid import attribute root",
        );
        let error = match root_fetch.finish_source_for_test(
            &mut vm,
            Ok(ModuleSource::text(
                r#"import "./dep.mjs" with { foo: "bar" };"#.to_owned(),
            )),
        ) {
            Ok(_) => panic!("invalid import attribute key should fail during module creation"),
            Err(error) => error,
        };

        assert_eq!(error.stage(), ModuleLoadStage::Compile);
        assert_eq!(
            error.error_constructor(),
            Some(ScriptErrorConstructorKind::SyntaxError)
        );
        assert!(
            error.message().contains("Invalid attribute key \"foo\"."),
            "{}",
            error.message()
        );
    }

    #[test]
    fn static_import_with_text_module_type_fails_before_dependency_fetch() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser graph should request root"),
            "text module type root",
        );
        let error = match root_fetch.finish_source_for_test(
            &mut vm,
            Ok(ModuleSource::text(
                r#"import text from "./dep.txt" with { type: "text" };"#.to_owned(),
            )),
        ) {
            Ok(_) => panic!("text import attribute type should fail before dependency fetch"),
            Err(error) => error,
        };

        assert_eq!(error.stage(), ModuleLoadStage::Resolve);
        assert_eq!(
            error.error_constructor(),
            Some(ScriptErrorConstructorKind::TypeError)
        );
        assert!(
            error
                .message()
                .contains("module type `text` is not a valid module type for import `./dep.txt`"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn dynamic_import_with_text_module_type_fails_before_fetch() {
        let mut vm = new_test_vm("https://example.test/app/page.html");
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let mut job = NativeModuleGraphJob::dynamic_import(PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            crate::module_runtime::DynamicModuleImportOwner::main_for_test(),
            "./dep.txt",
            url("https://example.test/app/page.html"),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "text".to_owned())]),
            ModuleImportPhase::Evaluation,
        ));

        let error = match job.advance_dynamic_import_owner_lane(&mut vm) {
            Ok(_) => panic!("text module type dynamic import should fail before root fetch"),
            Err(error) => error,
        };

        assert!(!job.has_chromium_tree_for_test());
        assert_eq!(error.stage(), ModuleLoadStage::Resolve);
        assert_eq!(
            error.error_constructor(),
            Some(ScriptErrorConstructorKind::TypeError)
        );
        assert!(
            error.message().contains(
                "module type `text` is not a valid module type for module `https://example.test/app/dep.txt`"
            ),
            "{}",
            error.message()
        );
    }

    #[test]
    fn external_parser_module_uses_response_url_as_dependency_base_without_aliasing_map_key() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let request_url = url("https://app.example.test/request/root.mjs");
        let response_url = url("https://cdn.example.test/final/root.mjs");
        let dep_url = url("https://cdn.example.test/final/dep.mjs");
        let request_key = ModuleMapKey::java_script(request_url.clone());
        let response_key = ModuleMapKey::java_script(response_url.clone());
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &request_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("external parser graph should request root"),
            "redirected external parser root",
        );
        assert_eq!(root_fetch.request().source_url(), &request_url);
        let dep_fetch = expect_single_fetch(
            root_fetch
                .finish_fetch(
                    &mut vm,
                    Ok(ModuleGraphFetchedSource::new(
                        response_url.clone(),
                        true,
                        ModuleSource::text("import './dep.mjs';".to_owned()),
                    )
                    .with_response_referrer_policy(Some("no-referrer".to_owned()))),
                )
                .expect("redirected root fetch should compile and discover dependency"),
            "redirected dependency graph",
        );

        assert_eq!(
            dep_fetch.request().source_url(),
            &dep_url,
            "relative imports must resolve against the response final URL"
        );
        assert_eq!(
            dep_fetch
                .request()
                .fetch_metadata
                .request_metadata
                .referrer_policy
                .as_deref(),
            Some("no-referrer"),
            "response Referrer-Policy must become descendant fetch metadata"
        );

        let root_entry = vm
            .document_runtime
            .native_module_entry_id(&request_key)
            .expect("request URL remains the module map key");
        assert!(
            vm.document_runtime
                .native_module_entry_id(&response_key)
                .is_none(),
            "Chromium module map keys are request URLs, not response final URLs"
        );
        assert_eq!(
            vm.document_runtime
                .native_module_entry_key(root_entry)
                .url()
                .as_str(),
            response_url.as_str(),
            "compiled module source/base identity should use the response URL"
        );
    }

    #[test]
    fn already_compiled_redirected_module_reuses_effective_base_for_dependencies() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let request_url = url("https://app.example.test/request/root.mjs");
        let response_url = url("https://cdn.example.test/final/root.mjs");
        let dep_url = url("https://cdn.example.test/final/dep.mjs");
        let request_key = ModuleMapKey::java_script(request_url.clone());

        let first_job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &request_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );
        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, first_job)
                .expect("first graph should request root"),
            "first redirected root",
        );
        let dep_fetch = expect_single_fetch(
            root_fetch
                .finish_fetch(
                    &mut vm,
                    Ok(ModuleGraphFetchedSource::new(
                        response_url.clone(),
                        true,
                        ModuleSource::text("import './dep.mjs';".to_owned()),
                    )),
                )
                .expect("first root fetch should discover dependency"),
            "first redirected dependency",
        );
        assert_eq!(dep_fetch.request().source_url(), &dep_url);
        match dep_fetch
            .finish_source_for_test(
                &mut vm,
                Ok(ModuleSource::text("export const dep = 1;".to_owned())),
            )
            .expect("dependency fetch should complete first graph")
        {
            ModuleScriptGraphAdvance::Complete(_) => {}
            _ => panic!("first graph should complete after dependency"),
        }

        assert_eq!(
            vm.document_runtime.native_module_entry_state(
                vm.document_runtime
                    .native_module_entry_id(&request_key)
                    .expect("request URL entry should remain")
            ),
            ModuleMapEntryState::Compiled
        );

        let second_job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &request_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );
        match advance_module_script_graph(&mut vm, second_job)
            .expect("second graph should reuse compiled root and dependency")
        {
            ModuleScriptGraphAdvance::Complete(_) => {}
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (_, fetches) = fetches.into_parts();
                panic!(
                    "second graph should reuse compiled redirected graph, requested {} fetches",
                    fetches.len()
                )
            }
        }
    }

    #[test]
    fn dependency_fetch_uses_import_map_integrity_for_resolved_url() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let import_map_base = url("https://app.example.test/app/index.html");
        vm.document_runtime
            .register_import_map_source(r#"{"integrity":{"/dep.mjs":"sha384-dep"}}"#)
            .expect("import map should register");
        let root_url = url("https://app.example.test/root.mjs");
        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text("import './dep.mjs';".to_owned()),
            &root_url,
            &import_map_base,
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser graph should compile source override");

        let dep_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job)
                .expect("source override graph should request dependency"),
            "import-map integrity dependency",
        );
        assert_eq!(
            dep_fetch.request().source_url().as_str(),
            "https://app.example.test/dep.mjs"
        );
        assert_eq!(
            dep_fetch.request().integrity_for_test(),
            Some("sha384-dep"),
            "dependency fetch integrity should come from import map"
        );
    }

    #[test]
    fn dynamic_import_root_uses_import_map_integrity_for_resolved_url() {
        let mut vm = new_test_vm("https://example.test/app/page.html");
        vm.document_runtime
            .register_import_map_source(
                r#"{"imports":{"dyn":"./dynamic.mjs"},"integrity":{"./dynamic.mjs":"sha384-dyn"}}"#,
            )
            .expect("import map should register");
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        let mut job = NativeModuleGraphJob::dynamic_import(PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            crate::module_runtime::DynamicModuleImportOwner::main_for_test(),
            "dyn",
            url("https://example.test/app/page.html"),
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        ));

        let fetches = match job
            .advance_dynamic_import_owner_lane(&mut vm)
            .expect("dynamic import graph should request root")
        {
            NativeModuleGraphJobAdvance::NeedFetches(requests) => requests,
            NativeModuleGraphJobAdvance::Complete(_) => {
                panic!("dynamic import should not complete before root fetch")
            }
            NativeModuleGraphJobAdvance::WaitingForFetches => {
                panic!("dynamic import should not wait before root fetch")
            }
        };
        assert_eq!(fetches.len(), 1);
        let fetch = fetches
            .first()
            .expect("dynamic import root fetch should be present");
        assert_eq!(
            fetch.source_url().as_str(),
            "https://example.test/app/dynamic.mjs"
        );
        assert_eq!(fetch.integrity_for_test(), Some("sha384-dyn"));
    }

    #[test]
    fn external_module_root_uses_import_map_integrity_when_element_integrity_is_absent() {
        let mut vm = new_test_vm("https://app.example.test/page");
        vm.document_runtime
            .register_import_map_source(r#"{"integrity":{"/entry.mjs":"sha384-entry"}}"#)
            .expect("import map should register");
        let entry_url = url("https://app.example.test/entry.mjs");
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &entry_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job).expect("external graph should request root"),
            "external root import-map integrity",
        );
        assert_eq!(root_fetch.request().source_url(), &entry_url);
        assert_eq!(
            root_fetch.request().integrity_for_test(),
            Some("sha384-entry")
        );
    }

    #[test]
    fn external_module_root_element_integrity_overrides_import_map_integrity() {
        let mut vm = new_test_vm("https://app.example.test/page");
        vm.document_runtime
            .register_import_map_source(r#"{"integrity":{"/entry.mjs":"sha384-map"}}"#)
            .expect("import map should register");
        let entry_url = url("https://app.example.test/entry.mjs");
        let script_metadata = ScriptFetchMetadata {
            integrity: Some("sha384-element".to_owned()),
            ..ScriptFetchMetadata::default()
        };
        let job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &entry_url,
            &url("https://app.example.test/page"),
            &script_metadata,
        );

        let root_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, job).expect("external graph should request root"),
            "external root element integrity",
        );
        assert_eq!(
            root_fetch.request().integrity_for_test(),
            Some("sha384-element")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn module_graph_fetch_reuses_script_text_cache_across_same_site_page_initiators()
    -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let body = "export default function cachedModule() {}";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nCache-Control: max-age=60\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                .await
                .unwrap();

            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "second module fetch should be served from the loader script text cache"
            );
        });

        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let module_url = Url::parse(&format!("http://{addr}/cached-module.js"))?;
        let first_response = fetch_module_for_test(
            &loader,
            module_url.clone(),
            Url::parse(&format!("http://{addr}/first-page.html"))?,
            ModuleFetchMetadata::default(),
        )
        .await?;
        let second_response = fetch_module_for_test(
            &loader,
            module_url,
            Url::parse(&format!("http://{addr}/second-page.html"))?,
            ModuleFetchMetadata::default(),
        )
        .await?;

        assert!(
            !first_response.from_cache,
            "first network result should come from the network"
        );
        assert!(
            second_response.from_cache,
            "second module graph network result should preserve loader cache provenance"
        );

        server.await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn module_graph_fetch_rejects_integrity_mismatch() -> anyhow::Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request_head(&mut stream).await.unwrap();
            let body = "export default 'not the expected body';";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                .await
                .unwrap();
        });

        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        let module_url = Url::parse(&format!("http://{addr}/bad-integrity.js"))?;
        let metadata = ModuleFetchMetadata {
            request_metadata: ScriptFetchRequestMetadata {
                integrity: Some(
                    "sha384-Li9vy3DqF8tnTXuiaAJuML3ky+er10rcgNR/VqsVpcw+ThHmYcwiB1pbOxEbzJr7"
                        .to_owned(),
                ),
                ..ScriptFetchRequestMetadata::default()
            },
            ..ModuleFetchMetadata::default()
        };
        let error = fetch_module_for_test(
            &loader,
            module_url,
            Url::parse(&format!("http://{addr}/page.html"))?,
            metadata,
        )
        .await
        .expect_err("an integrity mismatch must reject the module fetch");

        assert!(
            error.to_string().contains("failed its integrity check"),
            "module graph fetch should report an integrity mismatch: {error:#}"
        );
        server.await?;
        Ok(())
    }

    async fn fetch_module_for_test(
        loader: &ResourceRequestClient,
        source_url: Url,
        initiator_url: Url,
        fetch_metadata: ModuleFetchMetadata,
    ) -> anyhow::Result<crate::protocol_types::NavigationResponse> {
        let request = NativeModuleGraphFetchRequest {
            source_url,
            initiator_url,
            fetch_metadata,
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        };
        let (tx, rx) = oneshot::channel();
        let registry = ResourceLoadRegistry::new(
            RendererResourceTaskRunner::from_current_tokio()
                .expect("module fetch test must own a Tokio runtime"),
        );
        let load = registry
            .register(
                ResourceLoadKind::Script,
                ResourceLoadDisposition::Ordinary,
                loader.frozen_request_client(),
                None,
            )
            .expect("module fetch test load should register");
        request.fetch_source_callback_with_load(loader, load, move |result, network_result| {
            let _ = tx.send((result, network_result));
        })?;
        let (result, network_result) = rx.await?;
        let source_result = result.map_err(anyhow::Error::msg);
        let network_result = network_result.expect("module fetch should record network result");
        let response = match network_result.as_ref() {
            Ok(response) => response.clone(),
            Err(error) => return Err(anyhow::anyhow!(error.clone())),
        };
        source_result?;
        Ok(response)
    }

    async fn read_http_request_head(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
        use tokio::io::AsyncReadExt;

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

    #[test]
    fn dependency_load_inherits_module_fetch_metadata_and_initiator() {
        let script_metadata = ScriptFetchMetadata {
            referrer_policy: Some("origin".to_owned()),
            ..ScriptFetchMetadata::default()
        };
        let root_metadata =
            ModuleFetchMetadata::from_parser_owned_script_fetch_metadata(&script_metadata);
        let request = NativeModuleGraphFetchRequest {
            source_url: url("https://cdn.example.test/dep.mjs"),
            initiator_url: url("https://app.example.test/page"),
            fetch_metadata: root_metadata.for_descendant_fetches(),
            kind: ModuleKind::JavaScript,
            tree_client: None,
            tree_graph_level: None,
            module_key: None,
            dependency: None,
        }
        .request()
        .expect("request should build");

        assert_eq!(request.credentials_mode, RequestCredentialsMode::SameOrigin);
        assert_eq!(
            request.cookie_context.initiator_url.as_ref(),
            Some(&url("https://app.example.test/page"))
        );
        assert_eq!(
            request
                .subresource_request_metadata()
                .and_then(|metadata| metadata.referrer_policy.as_deref()),
            Some("origin")
        );
    }

    #[test]
    fn graph_waits_for_reused_compiled_dependency_pending_static_child() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let parent_url = url("https://app.example.test/parent.mjs");
        let parent_job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"import { value } from "./child.mjs"; export { value };"#.to_owned(),
            ),
            &parent_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parent graph should compile source override");

        let child_key = match advance_module_script_graph(&mut vm, parent_job)
            .expect("parent graph should advance")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (_, mut fetches) = fetches.into_parts();
                assert_eq!(fetches.len(), 1);
                fetches
                    .pop()
                    .expect("dependency fetch should be present")
                    .pending_fetch_key()
                    .expect("dependency fetch should have a module key")
                    .clone()
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("parent graph should wait for child fetch")
            }
        };
        assert_eq!(
            child_key.url().as_str(),
            "https://app.example.test/child.mjs"
        );

        let root_url = url("https://app.example.test/root.mjs");
        let parser_job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"import { value } from "./parent.mjs"; window.graphValue = value;"#.to_owned(),
            ),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser graph should compile source override");

        let joined_job = expect_single_joined_wait(
            advance_module_script_graph(&mut vm, parser_job).expect("parser graph should advance"),
            "parser graph reusing pending child",
        );
        assert_eq!(
            joined_job.pending_joined_client_count_for_test(),
            1,
            "parser graph should wait on the pending child fetch through its owner job"
        );
        assert_eq!(
            vm.document_runtime
                .native_module_script_client_count_for_testing(),
            1,
            "joined child client should be stored on the module map entry"
        );
    }

    #[test]
    fn source_phase_graph_reusing_evaluation_wasm_entry_does_not_chase_dependencies() {
        const WASM_IMPORT_JS_DEPENDENCY: &[u8] = &[
            0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
            0x02, 0x14, 0x01, 0x08, 0x2e, 0x2f, 0x64, 0x65, 0x70, 0x2e, 0x6a, 0x73, 0x07, 0x6c,
            0x6f, 0x67, 0x45, 0x78, 0x65, 0x63, 0x00, 0x00, 0x07, 0x07, 0x01, 0x03, 0x6c, 0x6f,
            0x67, 0x00, 0x00,
        ];
        let mut vm = new_test_vm("https://app.example.test/page");
        let wasm_url = "https://app.example.test/execute-start.wasm";
        let wasm_key = ModuleMapKey::webassembly(url(wasm_url));
        let dep_key = ModuleMapKey::java_script(url("https://app.example.test/dep.js"));

        let evaluation_job = module_script_inline_tree_job(
            &mut vm,
            wasm_root_input(
                wasm_url,
                WASM_IMPORT_JS_DEPENDENCY,
                ModuleImportPhase::Evaluation,
            ),
        )
        .expect("evaluation wasm graph should compile source override");
        let fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, evaluation_job)
                .expect("evaluation graph should request wasm dependency fetch"),
            "evaluation wasm graph",
        );
        assert_eq!(fetch.pending_fetch_key(), Some(&dep_key));

        let wasm_entry = vm
            .document_runtime
            .native_module_entry_id(&wasm_key)
            .expect("wasm entry should be compiled");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(wasm_entry),
            ModuleMapEntryState::Compiled
        );
        let dep_entry = vm
            .document_runtime
            .native_module_entry_id(&dep_key)
            .expect("dependency fetch should be registered");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(dep_entry),
            ModuleMapEntryState::Fetching
        );

        let source_job = module_script_inline_tree_job(
            &mut vm,
            wasm_root_input(
                wasm_url,
                WASM_IMPORT_JS_DEPENDENCY,
                ModuleImportPhase::Source,
            ),
        )
        .expect("source-phase wasm graph should compile source override");
        match advance_module_script_graph(&mut vm, source_job)
            .expect("source-phase graph should ignore evaluation dependencies")
        {
            ModuleScriptGraphAdvance::Complete(_) => {}
            ModuleScriptGraphAdvance::NeedFetches(_) => {
                panic!("source-phase graph should not batch fetch dependencies")
            }
        }
    }

    #[test]
    fn parser_owned_graph_fetches_sibling_dependencies_in_one_batch() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./a.mjs";
import "./b.mjs";
import "./c.mjs";
"#
                .to_owned(),
            ),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser-owned graph should compile source override");

        match advance_module_script_graph(&mut vm, job).expect("parser-owned graph should advance")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (_, fetches) = fetches.into_parts();
                let urls: Vec<_> = fetches
                    .iter()
                    .map(|fetch| fetch.request().source_url().as_str())
                    .collect();
                assert_eq!(
                    urls,
                    vec![
                        "https://app.example.test/a.mjs",
                        "https://app.example.test/b.mjs",
                        "https://app.example.test/c.mjs",
                    ]
                );
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("parser-owned graph should not complete before dependency fetches")
            }
        }
    }

    #[test]
    fn parser_owned_sibling_dependency_batch_accepts_out_of_order_completions() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./a.mjs";
import "./b.mjs";
"#
                .to_owned(),
            ),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser-owned graph should compile source override");

        let (mut job, mut fetches) =
            match advance_module_script_graph(&mut vm, job).expect("graph should advance") {
                ModuleScriptGraphAdvance::NeedFetches(fetches) => fetches.into_parts(),
                ModuleScriptGraphAdvance::Complete(_) => {
                    panic!("graph should not complete before dependency fetches")
                }
            };
        assert_eq!(
            job.pending_joined_client_count_for_test(),
            0,
            "graph should not join existing fetches before scheduling siblings"
        );
        assert_eq!(fetches.len(), 2);
        fetches.sort_by(|left, right| {
            right
                .request()
                .source_url()
                .cmp(left.request().source_url())
        });
        let first_completion = fetches.remove(0);
        assert_eq!(
            first_completion.request().source_url().as_str(),
            "https://app.example.test/b.mjs"
        );
        first_completion
            .finish_source_for_test(
                &mut vm,
                &mut job,
                Ok(ModuleSource::text("export const b = 2;".to_owned())),
            )
            .expect("b.mjs should compile");
        let job = match advance_module_script_graph(&mut vm, job)
            .expect("graph should still wait for a.mjs")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert!(
                    fetches.is_empty(),
                    "out-of-order owned completion should not schedule additional fetches"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    0,
                    "out-of-order owned completion should not register joined fetches"
                );
                job
            }
            _ => panic!("expected graph to wait for the remaining dependency client"),
        };

        let mut job = job;
        let second_completion = fetches.remove(0);
        assert_eq!(
            second_completion.request().source_url().as_str(),
            "https://app.example.test/a.mjs"
        );
        second_completion
            .finish_source_for_test(
                &mut vm,
                &mut job,
                Ok(ModuleSource::text("export const a = 1;".to_owned())),
            )
            .expect("a.mjs should compile");
        match advance_module_script_graph(&mut vm, job).expect("graph should complete") {
            ModuleScriptGraphAdvance::Complete(_) => {}
            _ => panic!("expected graph completion after both dependency clients"),
        }
    }

    #[test]
    fn parser_owned_join_wait_does_not_drop_pending_owned_fetch_client() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let leaf_url = url("https://app.example.test/leaf.mjs");
        let _modulepreload_load_id = suspend_registered_modulepreload_fetch(
            &mut vm,
            modulepreload_single_fetch_request(leaf_url.clone()),
        );

        let job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./parent-a.mjs";
import "./parent-b.mjs";
"#
                .to_owned(),
            ),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser-owned graph should compile source override");

        let (mut job, mut fetches) =
            match advance_module_script_graph(&mut vm, job).expect("graph should advance") {
                ModuleScriptGraphAdvance::NeedFetches(fetches) => fetches.into_parts(),
                ModuleScriptGraphAdvance::Complete(_) => {
                    panic!("graph should not complete before dependency fetches")
                }
            };
        assert_eq!(
            job.pending_joined_client_count_for_test(),
            0,
            "graph should not join existing fetches before scheduling parents"
        );
        assert_eq!(fetches.len(), 2);

        let parent_a_index = fetches
            .iter()
            .position(|fetch| {
                fetch.request().source_url().as_str() == "https://app.example.test/parent-a.mjs"
            })
            .expect("parent-a fetch should be present");
        let parent_a = fetches.remove(parent_a_index);
        let parent_a_advance = parent_a
            .finish_source_for_test(
                &mut vm,
                &mut job,
                Ok(ModuleSource::text(
                    "import './leaf.mjs'; export const a = 1;".to_owned(),
                )),
            )
            .expect("parent-a should compile and join leaf modulepreload");
        let mut job = match module_script_graph_advance_from_native(job, parent_a_advance) {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert!(
                    fetches.is_empty(),
                    "parent-a should not schedule more owned fetches while parent-b is still pending"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    1,
                    "parent-a should register the joined leaf while parent-b is still pending"
                );
                assert_eq!(
                    vm.document_runtime
                        .native_module_script_client_count_for_testing(),
                    1,
                    "joined leaf should be stored on the module map entry"
                );
                job
            }
            _ => panic!("graph should wait for the remaining owned dependency client"),
        };
        let joined_clients = job.take_pending_joined_clients();
        assert_eq!(
            joined_clients.len(),
            1,
            "owner facade should drain the joined leaf client before restoring the graph job"
        );

        let parent_b = fetches.pop().expect("parent-b fetch should remain");
        assert_eq!(
            parent_b.request().source_url().as_str(),
            "https://app.example.test/parent-b.mjs"
        );
        parent_b
            .finish_source_for_test(
                &mut vm,
                &mut job,
                Ok(ModuleSource::text(
                    "import './leaf.mjs'; export const b = 2;".to_owned(),
                )),
            )
            .expect("parent-b should compile");
        match advance_module_script_graph(&mut vm, job)
            .expect("graph should now wait for the joined leaf modulepreload")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert!(
                    fetches.is_empty(),
                    "joined leaf wait should not schedule another owned fetch"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    0,
                    "joined leaf should already have been registered while parent-b was pending"
                );
            }
            _ => panic!("graph should keep waiting for the already-registered leaf join"),
        }
    }

    #[test]
    fn parser_owned_sibling_dependency_batches_join_repeated_module_map_entries() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let first_root_url = url("https://app.example.test/first-root.mjs");
        let first_job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./a.mjs";
import "./shared.mjs";
"#
                .to_owned(),
            ),
            &first_root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("first parser-owned graph should compile source override");

        match advance_module_script_graph(&mut vm, first_job)
            .expect("first parser-owned graph should advance")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    0,
                    "first graph should not join existing fetches before scheduling siblings"
                );
                let urls: Vec<_> = fetches
                    .iter()
                    .map(|fetch| fetch.request().source_url().as_str())
                    .collect();
                assert_eq!(
                    urls,
                    vec![
                        "https://app.example.test/a.mjs",
                        "https://app.example.test/shared.mjs",
                    ]
                );
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("first graph should not complete before dependency fetches")
            }
        }
        let second_root_url = url("https://app.example.test/second-root.mjs");
        let second_job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(
                r#"
import "./b.mjs";
import "./shared.mjs";
"#
                .to_owned(),
            ),
            &second_root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("second parser-owned graph should compile source override");
        match advance_module_script_graph(&mut vm, second_job)
            .expect("second parser-owned graph should advance")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                let urls: Vec<_> = fetches
                    .iter()
                    .map(|fetch| fetch.request().source_url().as_str())
                    .collect();
                assert_eq!(
                    urls,
                    vec!["https://app.example.test/b.mjs"],
                    "second graph should fetch only the new sibling; shared.mjs joins the module map entry reserved by the first graph"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    1,
                    "second graph should register one joined waiter for shared.mjs"
                );
                assert_eq!(
                    vm.document_runtime
                        .native_module_script_client_count_for_testing(),
                    1,
                    "joined shared module should be stored on the module map entry"
                );
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("second graph should not complete before dependency fetches")
            }
        }
    }

    #[test]
    fn parser_graph_joins_pending_modulepreload_without_taking_owner() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let start = vm
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload_single_fetch_request(
                root_url.clone(),
            ))
            .expect("modulepreload should reserve the module map entry");
        let NativeModulepreloadFetchStart::Started(request) = start else {
            panic!("new modulepreload should start one single fetch")
        };
        vm.document_runtime
            .suspend_native_modulepreload_fetch(*request);

        let parser_job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );
        match advance_module_script_graph(&mut vm, parser_job)
            .expect("parser graph should join the pending modulepreload")
        {
            ModuleScriptGraphAdvance::NeedFetches(fetches) => {
                let (job, fetches) = fetches.into_parts();
                assert!(
                    fetches.is_empty(),
                    "parser graph must not take ownership of the pending modulepreload"
                );
                assert_eq!(
                    job.pending_joined_client_count_for_test(),
                    1,
                    "parser graph should register one module map waiter"
                );
            }
            ModuleScriptGraphAdvance::Complete(_) => {
                panic!("parser graph should not complete before modulepreload fetch completion")
            }
        }
        assert!(
            vm.document_runtime
                .has_inflight_native_modulepreload_fetch(),
            "joined parser graph must leave the in-flight modulepreload fetch owned by modulepreload"
        );
        assert!(
            vm.document_runtime.has_native_module_script_fetch_waiters(),
            "FetchSingle JoinedFetching should register the parser module script client on the module map entry"
        );
    }

    #[test]
    fn modulepreload_completion_fetches_single_module_without_descendants() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let dep_url = url("https://app.example.test/dep.mjs");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let dep_key = ModuleMapKey::java_script(dep_url);
        let load_id = suspend_registered_modulepreload_fetch(
            &mut vm,
            modulepreload_single_fetch_request(root_url.clone()),
        );

        vm.complete_native_module_graph_fetch(crate::types::ModuleGraphFetchCompletion {
            load_id,
            requester: ModuleGraphFetchRequester::ModulePreload,
            ordering: ModuleGraphFetchOrdering::BackgroundPreload,
            request_url: root_url.clone(),
            result: Ok(ModuleGraphFetchedSource::new(
                root_url,
                false,
                ModuleSource::text("import './dep.mjs'; export const root = 1;".to_owned()),
            )),
            network_result: None,
        })
        .expect("modulepreload completion should be accepted");

        let root_entry = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("modulepreload root should remain in module map");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(root_entry),
            ModuleMapEntryState::Fetched,
            "modulepreload should fetch the single root module but not compile the graph"
        );
        assert!(
            vm.document_runtime
                .native_module_entry_id(&dep_key)
                .is_none(),
            "modulepreload must not fetch descendants"
        );
    }

    #[test]
    fn modulepreload_single_fetch_uses_import_map_integrity_when_element_integrity_is_absent() {
        let mut vm = new_test_vm("https://app.example.test/page");
        vm.document_runtime
            .register_import_map_source(r#"{"integrity":{"/root.mjs":"sha384-root"}}"#)
            .expect("import map should register");
        let root_url = url("https://app.example.test/root.mjs");
        let metadata = ModuleFetchMetadata::from_modulepreload_script_fetch_metadata(
            &ScriptFetchMetadata::default(),
        )
        .with_import_map_integrity_if_absent(
            vm.document_runtime.resolve_module_integrity(&root_url),
        );

        let start = vm
            .document_runtime
            .fetch_single_native_module_for_modulepreload(
                modulepreload_single_fetch_request_with_metadata(root_url, metadata),
            )
            .expect("modulepreload should reserve the module map entry");
        let NativeModulepreloadFetchStart::Started(started) = start else {
            panic!("new modulepreload should start a single fetch")
        };
        assert_eq!(
            started
                .fetch_metadata()
                .request_metadata
                .integrity
                .as_deref(),
            Some("sha384-root")
        );
    }

    #[test]
    fn modulepreload_element_integrity_overrides_import_map_integrity() {
        let mut vm = new_test_vm("https://app.example.test/page");
        vm.document_runtime
            .register_import_map_source(r#"{"integrity":{"/root.mjs":"sha384-map"}}"#)
            .expect("import map should register");
        let root_url = url("https://app.example.test/root.mjs");
        let script_metadata = ScriptFetchMetadata {
            integrity: Some("sha384-element".to_owned()),
            ..ScriptFetchMetadata::default()
        };
        let metadata =
            ModuleFetchMetadata::from_modulepreload_script_fetch_metadata(&script_metadata)
                .with_import_map_integrity_if_absent(
                    vm.document_runtime.resolve_module_integrity(&root_url),
                );

        let start = vm
            .document_runtime
            .fetch_single_native_module_for_modulepreload(
                modulepreload_single_fetch_request_with_metadata(root_url, metadata),
            )
            .expect("modulepreload should reserve the module map entry");
        let NativeModulepreloadFetchStart::Started(started) = start else {
            panic!("new modulepreload should start a single fetch")
        };
        assert_eq!(
            started
                .fetch_metadata()
                .request_metadata
                .integrity
                .as_deref(),
            Some("sha384-element")
        );
    }

    #[test]
    fn parser_graph_reuses_modulepreload_single_fetch_then_fetches_dependencies() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let dep_url = url("https://app.example.test/dep.mjs");
        let root_source = "import './dep.mjs'; export const root = 1;";
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let dep_key = ModuleMapKey::java_script(dep_url.clone());
        let load_id = suspend_registered_modulepreload_fetch(
            &mut vm,
            modulepreload_single_fetch_request(root_url.clone()),
        );
        vm.complete_native_module_graph_fetch(crate::types::ModuleGraphFetchCompletion {
            load_id,
            requester: ModuleGraphFetchRequester::ModulePreload,
            ordering: ModuleGraphFetchOrdering::BackgroundPreload,
            request_url: root_url.clone(),
            result: Ok(ModuleGraphFetchedSource::new(
                root_url.clone(),
                false,
                ModuleSource::text(root_source.to_owned()),
            )),
            network_result: None,
        })
        .expect("modulepreload completion should be accepted");

        let parser_job = module_script_graph_job(
            &mut vm,
            ModuleSource::text(root_source.to_owned()),
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
            true,
        )
        .expect("parser graph should compile source override");
        let fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, parser_job)
                .expect("parser graph should reuse modulepreload root"),
            "parser graph after modulepreload root reuse",
        );
        assert_eq!(fetch.request().source_url(), &dep_url);
        assert_eq!(
            vm.document_runtime.native_module_entry_state(
                vm.document_runtime
                    .native_module_entry_id(&root_key)
                    .expect("root entry should remain")
            ),
            ModuleMapEntryState::Compiled,
            "parser graph should compile the fetched modulepreload root"
        );
        assert_eq!(
            vm.document_runtime.native_module_entry_state(
                vm.document_runtime
                    .native_module_entry_id(&dep_key)
                    .expect("dependency fetch should be registered")
            ),
            ModuleMapEntryState::Fetching
        );
    }

    #[test]
    fn modulepreload_joins_existing_parser_fetch_without_resetting_entry() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let parser_job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );

        let fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, parser_job)
                .expect("parser graph should reserve root fetch"),
            "parser graph root",
        );
        assert_eq!(fetch.pending_fetch_key(), Some(&root_key));
        let entry_id = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("parser graph should create root entry");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry_id),
            ModuleMapEntryState::Fetching
        );

        let start = vm
            .document_runtime
            .fetch_single_native_module_for_modulepreload(modulepreload_single_fetch_request(
                root_url.clone(),
            ))
            .expect("modulepreload should join existing root fetch");
        assert_eq!(
            start,
            NativeModulepreloadFetchStart::Joined,
            "joining an existing parser fetch should not schedule a modulepreload network fetch"
        );

        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry_id),
            ModuleMapEntryState::Fetching,
            "modulepreload join must not reset or complete the parser-owned fetching entry"
        );
        assert!(
            !vm.document_runtime
                .has_inflight_native_modulepreload_fetch(),
            "joining an existing module fetch must not create an in-flight modulepreload fetch"
        );
    }

    #[test]
    fn failed_owned_fetch_settles_module_map_and_notifies_joined_graph() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let initiator_url = url("https://app.example.test/page");
        let first_root_url = url("https://app.example.test/first.mjs");
        let second_root_url = url("https://app.example.test/second.mjs");
        let dependency_url = url("https://app.example.test/shared.mjs");
        let dependency_key = ModuleMapKey::java_script(dependency_url.clone());
        let source = ModuleSource::text("import './shared.mjs';".to_owned());

        let first_job = module_script_graph_job_for_owner(
            &mut vm,
            source.clone(),
            &first_root_url,
            &initiator_url,
            &ScriptFetchMetadata::default(),
            false,
            ModuleScriptCompletionOwner::Runtime,
        )
        .expect("first runtime module graph should build");
        let first_fetch = expect_single_fetch(
            advance_module_script_graph(&mut vm, first_job)
                .expect("first runtime module graph should reserve the dependency fetch"),
            "first runtime module graph dependency",
        );

        let second_job = module_script_graph_job_for_owner(
            &mut vm,
            source,
            &second_root_url,
            &initiator_url,
            &ScriptFetchMetadata::default(),
            false,
            ModuleScriptCompletionOwner::Runtime,
        )
        .expect("second runtime module graph should build");
        let _second_job = expect_single_joined_wait(
            advance_module_script_graph(&mut vm, second_job)
                .expect("second runtime module graph should join the dependency fetch"),
            "second runtime module graph dependency",
        );

        let fetch_error = ModuleLoadError::new(ModuleLoadStage::Fetch, "shared fetch failed");
        let error = match first_fetch.finish_source_for_test(&mut vm, Err(fetch_error.clone())) {
            Ok(_) => panic!("the owning runtime module graph should fail"),
            Err(error) => error,
        };
        assert_eq!(error, fetch_error);

        let dependency_entry = vm
            .document_runtime
            .native_module_entry_id(&dependency_key)
            .expect("failed dependency should remain in the module map");
        assert_eq!(
            vm.document_runtime
                .native_module_entry_state(dependency_entry),
            ModuleMapEntryState::Failed,
            "the fetch owner must settle the shared module map entry before its graph fails"
        );

        let event = vm
            .document_runtime
            .take_next_native_module_owner_event()
            .expect("the joined graph should receive a terminal module map notification");
        let super::super::NativeModuleOwnerEvent::ModuleMapTerminalNotification(notification) =
            event
        else {
            panic!("joined module graph should receive a module map terminal notification");
        };
        let (key, clients, successful) = notification.into_parts();
        let (single_module_clients, parser_root_module_clients, modulepreload_link_clients) =
            clients.into_parts();
        assert_eq!(key, dependency_key);
        assert!(!successful);
        assert_eq!(single_module_clients.len(), 1);
        assert!(parser_root_module_clients.is_empty());
        assert!(modulepreload_link_clients.is_empty());
    }

    #[test]
    fn parser_graph_observes_sticky_modulepreload_failure_without_refetching() {
        let mut vm = new_test_vm("https://app.example.test/page");
        let root_url = url("https://app.example.test/root.mjs");
        let root_key = ModuleMapKey::java_script(root_url.clone());
        let load_id = suspend_registered_modulepreload_fetch(
            &mut vm,
            modulepreload_single_fetch_request(root_url.clone()),
        );

        vm.complete_native_module_graph_fetch(ModuleGraphFetchCompletion {
            load_id,
            requester: ModuleGraphFetchRequester::ModulePreload,
            ordering: ModuleGraphFetchOrdering::BackgroundPreload,
            request_url: root_url.clone(),
            result: Err("network failure".to_owned()),
            network_result: None,
        })
        .expect("modulepreload failure completion should be accepted");
        let entry_id = vm
            .document_runtime
            .native_module_entry_id(&root_key)
            .expect("failed modulepreload should leave sticky module map entry");
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry_id),
            ModuleMapEntryState::Failed
        );

        let parser_job = parser_owned_external_module_script_graph_job(
            &mut vm,
            &root_url,
            &url("https://app.example.test/page"),
            &ScriptFetchMetadata::default(),
        );
        let error = match advance_module_script_graph(&mut vm, parser_job) {
            Ok(ModuleScriptGraphAdvance::Complete(_)) => {
                panic!("parser graph should not complete after root failure")
            }
            Ok(ModuleScriptGraphAdvance::NeedFetches(_)) => {
                panic!("parser graph should not batch fetch after root failure")
            }
            Err(error) => error,
        };
        assert!(
            error.message().contains("network failure"),
            "parser graph should receive the sticky modulepreload failure, got: {}",
            error.message()
        );
        assert_eq!(
            vm.document_runtime.native_module_entry_state(entry_id),
            ModuleMapEntryState::Failed,
            "failed module entry must not be reset to a new fetch"
        );
    }
}
