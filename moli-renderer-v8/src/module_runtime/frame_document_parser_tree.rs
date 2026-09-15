use moli_module_script_tree as module_tree;
use url::Url;

use crate::document_module_graph::{
    ModuleEntryId, ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleLoadError, ModuleLoadStage,
    ModuleMapFetchDisposition, ModuleMapKey, ModuleRequestRecord,
};
use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::ParserPendingScriptKey;
use crate::frame_owner_model::{
    FrameDocumentModuleClientEntryId, FrameDocumentModuleClientId,
    FrameDocumentModuleClientRegistration, FrameDocumentModuleClientReservation,
    FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleFetchDisposition,
    FrameDocumentModuleFetchTerminalResult, FrameDocumentParserRootModuleClient,
    FrameDocumentParserRootTerminalClient, FrameDocumentParserRootTerminalWork,
    FrameDocumentStaticDependencyModuleClient, FrameDocumentTaskOwner, FrameRealmId,
};
use crate::planning::PreparedScript;

use super::graph::{NativeModuleGraphFetchRequest, NativeModuleGraphJob};
use super::modulator::{NativeDocumentModulator, NativeFrameDocumentDependencyFetchBuildFailure};
use super::parser_tree_registry::{NativeParserModuleTreeJobResume, NativeParserModuleTreeRoot};

impl NativeDocumentModulator {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_frame_document_compiled_parser_root(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_key: ParserPendingScriptKey,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        source_url: Url,
        entry_id: ModuleEntryId,
        parent_key: ModuleMapKey,
        requests: Vec<ModuleRequestRecord>,
        effective_fetch_metadata: ModuleFetchMetadata,
        load_delay_token: crate::frame_owner_model::ChildDocumentModuleScriptLoadDelay,
    ) -> module_tree::ModuleTreeId {
        let request_count = requests.len();
        let parser_tree_job = NativeModuleGraphJob::parser_owned_compiled_entry(
            parent_key.clone(),
            entry_id,
            source_url.clone(),
            source_url.clone(),
            effective_fetch_metadata,
        );
        let tree_id = parser_tree_job
            .tree_id()
            .expect("parser-owned compiled entry graph job should have a tree id");
        let root = NativeParserModuleTreeRoot::new(
            owner,
            realm_id,
            pending_script_key,
            script,
            script_handle,
            request_key,
            tree_id,
            entry_id,
            request_count,
            load_delay_token,
        );
        tracing::debug!(
            owner = ?root.owner(),
            realm_id = ?root.realm_id(),
            script_handle = ?root.script_handle(),
            url = %root.request_key().url(),
            tree_id = root.tree_id().0,
            entry_id = root.entry_id().raw(),
            request_count = root.request_count(),
            dependency_count = root.dependency_count(),
            "frame document parser module root compiled with shared tree job state"
        );
        self.insert_parser_module_tree_job(root, parser_tree_job);
        tree_id
    }

    pub(crate) fn reserve_frame_document_parser_root_module_client(
        &mut self,
        owner: FrameDocumentTaskOwner,
        key: ModuleMapKey,
        client: FrameDocumentParserRootModuleClient,
    ) -> FrameDocumentModuleClientReservation {
        let document_owner = owner.document_owner();
        let fetch_disposition = self.start_or_join_fetch(key.clone());
        let client_id = self.reserve_parser_root_module_client_id();
        let terminal_client = FrameDocumentParserRootTerminalClient::new(client);
        self.add_parser_root_module_client(key.clone(), terminal_client);
        FrameDocumentModuleClientReservation::new(
            document_owner,
            key,
            FrameDocumentModuleClientRegistration::new(
                FrameDocumentModuleClientEntryId::from_raw(fetch_disposition.entry_id().raw()),
                client_id,
                frame_document_module_fetch_disposition(fetch_disposition),
            ),
        )
    }

    pub(crate) fn finish_parser_root_module_fetch(
        &mut self,
        request_key: ModuleMapKey,
        result: Result<ModuleGraphFetchedSource, String>,
    ) {
        match result {
            Ok(fetched_source) => {
                let effective_key = fetched_source.effective_key_for_request(&request_key);
                let effective_fetch_metadata = ModuleFetchMetadata::default()
                    .with_response_referrer_policy(fetched_source.response_referrer_policy());
                self.insert_fetched_source_for_request(
                    request_key,
                    effective_key,
                    fetched_source.into_source(),
                    effective_fetch_metadata,
                );
            }
            Err(error) => {
                self.mark_failed(
                    request_key,
                    ModuleLoadError::new(ModuleLoadStage::Fetch, error),
                );
            }
        }
    }

    pub(crate) fn parser_root_terminal_result(
        &self,
        key: &ModuleMapKey,
        successful: bool,
    ) -> Option<FrameDocumentModuleFetchTerminalResult> {
        let entry_id = self.entry_id(key)?;
        let entry = self.entry(entry_id);
        if successful {
            let source = entry.source().cloned()?;
            let final_url = entry.effective_key().url().clone();
            let response_referrer_policy = entry
                .effective_fetch_metadata()
                .request_metadata
                .referrer_policy
                .clone();
            Some(FrameDocumentModuleFetchTerminalResult::Fetched(
                ModuleGraphFetchedSource::new(final_url.clone(), final_url != *key.url(), source)
                    .with_response_referrer_policy(response_referrer_policy),
            ))
        } else {
            let message = entry
                .failure()
                .map(|error| error.message().to_owned())
                .unwrap_or_else(|| "module map entry fetch failed".to_owned());
            Some(FrameDocumentModuleFetchTerminalResult::Failed(message))
        }
    }

    pub(crate) fn parser_root_terminal_works(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        key: ModuleMapKey,
        clients: Vec<FrameDocumentParserRootTerminalClient>,
        successful: bool,
    ) -> Option<Vec<FrameDocumentParserRootTerminalWork>> {
        let result = self.parser_root_terminal_result(&key, successful)?;
        Some(
            clients
                .into_iter()
                .map(|client| {
                    FrameDocumentParserRootTerminalWork::from_terminal_parts(
                        owner,
                        realm_id,
                        key.clone(),
                        client,
                        result.clone(),
                    )
                })
                .collect(),
        )
    }

    pub(crate) fn frame_document_static_dependency_fetch_task(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        fetch: NativeModuleGraphFetchRequest,
    ) -> Result<
        FrameDocumentModuleDependencyFetchTask,
        Box<NativeFrameDocumentDependencyFetchBuildFailure>,
    > {
        let Some(dependency_key) = fetch.pending_fetch_key().cloned() else {
            let fallback_key = ModuleMapKey::java_script(fetch.source_url().clone());
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "shared child module tree fetch was missing its module map key",
            );
            return Err(Box::new(
                NativeFrameDocumentDependencyFetchBuildFailure::new(error, fallback_key, None),
            ));
        };
        let Some(tree_client) = fetch.tree_client() else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "shared child module tree fetch was missing its tree client",
            );
            return Err(Box::new(
                NativeFrameDocumentDependencyFetchBuildFailure::new(error, dependency_key, None),
            ));
        };
        let Some(dependency) = fetch.dependency().cloned() else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "shared child module tree fetch was missing dependency parent payload",
            );
            return Err(Box::new(
                NativeFrameDocumentDependencyFetchBuildFailure::new(error, dependency_key, None),
            ));
        };
        let parent_entry_id = dependency.parent_entry_id();
        let Some(entry_id) = self.entry_id(&dependency_key) else {
            let error = ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "shared child module tree fetch had no owner module map entry",
            );
            return Err(Box::new(
                NativeFrameDocumentDependencyFetchBuildFailure::new(
                    error,
                    dependency_key,
                    Some(parent_entry_id),
                ),
            ));
        };
        let client = FrameDocumentStaticDependencyModuleClient::new(
            parent_entry_id,
            dependency.parent_key().clone(),
            dependency.specifier().to_owned(),
            dependency.phase(),
            tree_client,
        );
        let reservation = FrameDocumentModuleClientReservation::new(
            owner.document_owner(),
            dependency_key.clone(),
            FrameDocumentModuleClientRegistration::new(
                FrameDocumentModuleClientEntryId::from_raw(entry_id.raw()),
                FrameDocumentModuleClientId::from_raw(tree_client.sequence),
                FrameDocumentModuleFetchDisposition::StartedFetch(
                    FrameDocumentModuleClientEntryId::from_raw(entry_id.raw()),
                ),
            ),
        );
        Ok(
            FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
                owner,
                realm_id,
                dependency_key,
                client,
                reservation,
                fetch,
            ),
        )
    }

    pub(crate) fn frame_document_static_dependency_fetch_tasks(
        &self,
        resume: &mut NativeParserModuleTreeJobResume,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> Result<
        Vec<FrameDocumentModuleDependencyFetchTask>,
        Box<NativeFrameDocumentDependencyFetchBuildFailure>,
    > {
        let owner = resume.root().owner();
        let realm_id = resume.root().realm_id();
        let mut retained = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            retained
                .push(self.frame_document_static_dependency_fetch_task(owner, realm_id, fetch)?);
        }
        resume.root_mut().add_dependencies(retained.len());
        Ok(retained)
    }
}

fn frame_document_module_fetch_disposition(
    disposition: ModuleMapFetchDisposition,
) -> FrameDocumentModuleFetchDisposition {
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(disposition.entry_id().raw());
    match disposition {
        ModuleMapFetchDisposition::StartedFetch(_) => {
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id)
        }
        ModuleMapFetchDisposition::JoinedFetching(_) => {
            FrameDocumentModuleFetchDisposition::JoinedFetching(entry_id)
        }
        ModuleMapFetchDisposition::AlreadyFetched(_) => {
            FrameDocumentModuleFetchDisposition::AlreadyFetched(entry_id)
        }
        ModuleMapFetchDisposition::AlreadyCompiled(_) => {
            FrameDocumentModuleFetchDisposition::AlreadyLinked(entry_id)
        }
        ModuleMapFetchDisposition::AlreadyFailed(_) => {
            FrameDocumentModuleFetchDisposition::AlreadyFailed(entry_id)
        }
    }
}
