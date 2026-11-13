use super::*;

pub(super) const BOOTSTRAP_TREE: module_tree::ModuleTreeId = module_tree::ModuleTreeId(0);

pub(super) enum WorkerModuleTreeTarget {
    Bootstrap,
    Import(Box<WorkerDynamicModuleImportJob>),
}

pub(super) struct WorkerModuleTreeTask {
    pub(super) tree: module_tree::ModuleScriptTreeJob,
    pub(super) target: WorkerModuleTreeTarget,
    pub(super) options: WorkerModuleGraphFetchOptions,
}

pub(super) struct WorkerSingleModuleCompletion {
    pub(super) client: module_tree::SingleModuleClientToken,
    pub(super) result: Result<usize, module_tree::ModuleLoadError>,
}

#[derive(Default)]
pub(super) struct WorkerModuleTrees {
    pub(super) tasks: HashMap<module_tree::ModuleTreeId, WorkerModuleTreeTask>,
    pub(super) notifications: VecDeque<WorkerSingleModuleCompletion>,
    pub(super) fetches_to_start: Vec<WorkerModuleGraphFetchRequest>,
}

pub(super) struct WorkerModuleTreeHost<'a, 's, 'i> {
    pub(super) scope: &'a mut v8::PinScope<'s, 'i>,
    pub(super) modules: Rc<RefCell<WorkerModuleMap>>,
    pub(super) work: Rc<RefCell<WorkerModuleTrees>>,
    pub(super) options: &'a WorkerModuleGraphFetchOptions,
}

impl module_tree::ModuleScriptTreeHost for WorkerModuleTreeHost<'_, '_, '_> {
    fn resolve_module_request(
        &mut self,
        specifier: &str,
        base_url: &Url,
        attributes: &module_tree::ModuleAttributesKey,
        requested_phase: module_tree::ModuleImportPhase,
    ) -> Result<module_tree::ResolvedModuleRequest, module_tree::ModuleLoadError> {
        let url = resolve_worker_module_specifier(specifier, base_url).map_err(|message| {
            module_tree::ModuleLoadError::new(module_tree::ModuleLoadStage::Resolve, message)
        })?;
        let native_attributes = ModuleAttributesKey::from_pairs(attributes.attributes.clone());
        let key =
            worker_module_key_for_attributes(&url, &native_attributes).map_err(|message| {
                module_tree::ModuleLoadError::new(module_tree::ModuleLoadStage::Resolve, message)
            })?;
        Ok(module_tree::ResolvedModuleRequest {
            key: tree_key(&key),
            source_url: url.clone(),
            base_url: url,
            kind: tree_kind(key.module_type),
            attributes: attributes.clone(),
            phase: requested_phase,
            integrity: None,
        })
    }

    fn start_or_join_single_module_fetch(
        &mut self,
        request: module_tree::ModuleFetchRequest,
        client: module_tree::SingleModuleClientToken,
    ) -> module_tree::SingleModuleFetchDisposition {
        let key = match native_key(&request.key) {
            Ok(key) => key,
            Err(error) => {
                return module_tree::SingleModuleFetchDisposition::Completed(
                    module_tree::ModuleFetchOutcome::Failed(error),
                );
            }
        };
        let existing = self.modules.borrow().entries.contains_key(&key);
        if !existing {
            match load_worker_static_module_dependency(key.url.clone(), key.module_type) {
                Ok(WorkerModuleDependencyLoad::Source { url, source }) => {
                    let entry = create_worker_module_script(
                        self.scope,
                        &self.modules,
                        &source,
                        key,
                        url,
                        native_referrer_policy(request.fetch_metadata.referrer_policy)
                            .map(str::to_owned),
                    );
                    return module_tree::SingleModuleFetchDisposition::Completed(ready_module(
                        &self.modules.borrow(),
                        entry,
                    ));
                }
                Ok(WorkerModuleDependencyLoad::Network) => {}
                Err(message) => {
                    return module_tree::SingleModuleFetchDisposition::Completed(
                        module_tree::ModuleFetchOutcome::Failed(
                            module_tree::ModuleLoadError::new(
                                module_tree::ModuleLoadStage::Fetch,
                                message,
                            )
                            .with_key(request.key),
                        ),
                    );
                }
            }
        }
        let mut options = self.options.clone();
        options.referrer_policy =
            native_referrer_policy(request.fetch_metadata.referrer_policy).map(str::to_owned);
        // Keep the Worker inside-settings initiator. The common tree's parent
        // URL is for module resolution, not for Worker fetch security checks.
        self.modules.borrow_mut().start_or_join_fetch(
            key,
            client,
            &options,
            &mut self.work.borrow_mut().fetches_to_start,
        )
    }

    fn compile_module_source(
        &mut self,
        source: module_tree::FetchedModuleSource,
        phase: module_tree::ModuleImportPhase,
    ) -> Result<module_tree::CompiledModuleSnapshot, module_tree::ModuleLoadError> {
        let key = native_key(&source.request_key)?;
        let native_source = match source.source {
            module_tree::ModuleSource::Text(text) => WorkerModuleSource::text(text.to_string()),
            module_tree::ModuleSource::Binary(bytes) => WorkerModuleSource::binary(bytes.to_vec()),
        };
        let entry = create_worker_module_script(
            self.scope,
            &self.modules,
            &native_source,
            key,
            source.source_url,
            native_referrer_policy(source.effective_fetch_metadata.referrer_policy)
                .map(str::to_owned),
        );
        let modules = self.modules.borrow();
        let script = modules.script(entry);
        Ok(module_tree::CompiledModuleSnapshot {
            entry: entry_id(entry),
            key: tree_key(&script.key),
            base_url: script.response_url.clone(),
            effective_fetch_metadata: fetch_metadata(
                modules.credentials_mode,
                script.referrer_policy.as_deref(),
            ),
            requested_modules: script
                .creation
                .as_ref()
                .map_or_else(|_| Vec::new(), |record| tree_requests(&record.requests)),
            phase,
        })
    }

    fn module_creation_error(
        &self,
        entry: module_tree::ModuleEntryId,
    ) -> Option<module_tree::ModuleLoadError> {
        creation_error(&self.modules.borrow(), entry.0 as usize)
    }

    fn module_dependencies(
        &self,
        entry: module_tree::ModuleEntryId,
    ) -> Result<module_tree::ModuleDependencySnapshot, module_tree::ModuleLoadError> {
        let modules = self.modules.borrow();
        let script = modules.script(entry.0 as usize);
        let record = script.creation.as_ref().map_err(|_| {
            creation_error(&modules, entry.0 as usize).expect("stored creation failure")
        })?;
        Ok(module_tree::ModuleDependencySnapshot {
            entry,
            key: tree_key(&script.key),
            base_url: script.response_url.clone(),
            effective_fetch_metadata: fetch_metadata(
                modules.credentials_mode,
                script.referrer_policy.as_deref(),
            ),
            requested_modules: tree_requests(&record.requests),
        })
    }

    fn link_module_graph(
        &mut self,
        root: module_tree::ModuleEntryId,
        entries: &[module_tree::ModuleEntryId],
        edges: &[module_tree::ModuleDependencyEdge],
    ) -> Result<module_tree::ModuleGraphHandle, module_tree::ModuleLoadError> {
        let mut modules = self.modules.borrow_mut();
        for edge in edges {
            let key = native_key(&edge.child_key)?;
            let child = modules
                .loaded_entry(&key)
                .expect("tree links only created module scripts");
            let parent = edge.parent_entry.0 as usize;
            let attributes = ModuleAttributesKey::from_pairs(edge.attributes.attributes.clone());
            if modules
                .dependency_entry(parent, &edge.specifier, &attributes)
                .is_none()
            {
                modules.add_dependency(parent, edge.specifier.clone(), attributes, child);
            }
        }
        Ok(module_tree::ModuleGraphHandle {
            root_entry: root,
            entries: entries.to_vec(),
            entry_phases: HashMap::new(),
            dependency_edges: edges.to_vec(),
        })
    }

    fn mark_module_failed(
        &mut self,
        key: module_tree::ModuleMapKey,
        error: module_tree::ModuleLoadError,
    ) -> module_tree::ModuleEntryId {
        let key = native_key(&key)
            .unwrap_or_else(|_| WorkerModuleKey::javascript_or_wasm(key.url.clone()));
        if let Some(entry) = self.modules.borrow().loaded_entry(&key) {
            return entry_id(entry);
        }
        let failure = Box::new(worker_bootstrap_error(
            self.scope,
            key.url.as_str(),
            &error.message,
            WorkerParentErrorEventKind::Event,
        ));
        let script = WorkerModuleScript {
            response_url: key.url.clone(),
            key,
            referrer_policy: None,
            creation: Err(failure),
        };
        entry_id(self.modules.borrow_mut().insert_script(script))
    }
}

pub(super) fn creation_error(
    modules: &WorkerModuleMap,
    entry: usize,
) -> Option<module_tree::ModuleLoadError> {
    let script = modules.script(entry);
    let error = script.creation.as_ref().err()?;
    Some(
        module_tree::ModuleLoadError::new(
            module_tree::ModuleLoadStage::Compile,
            error.0.summary.clone(),
        )
        .with_key(tree_key(&script.key))
        .with_module_entry(entry_id(entry)),
    )
}

pub(super) fn ready_module(
    modules: &WorkerModuleMap,
    entry: usize,
) -> module_tree::ModuleFetchOutcome {
    let script = modules.script(entry);
    module_tree::ModuleFetchOutcome::Ready(Box::new(module_tree::ReadyModule::new(
        entry_id(entry),
        tree_key(&script.key),
        script.response_url.clone(),
        fetch_metadata(modules.credentials_mode, script.referrer_policy.as_deref()),
    )))
}

pub(super) fn entry_id(entry: usize) -> module_tree::ModuleEntryId {
    module_tree::ModuleEntryId(
        u32::try_from(entry).expect("worker module entry id space exhausted"),
    )
}

pub(super) fn tree_key(key: &WorkerModuleKey) -> module_tree::ModuleMapKey {
    module_tree::ModuleMapKey::new(
        key.url.clone(),
        tree_kind(key.module_type),
        module_tree::ModuleAttributesKey::from_pairs(key.attributes.pairs().to_vec()),
    )
}

fn native_key(
    key: &module_tree::ModuleMapKey,
) -> Result<WorkerModuleKey, module_tree::ModuleLoadError> {
    worker_module_key_for_attributes(
        &key.url,
        &ModuleAttributesKey::from_pairs(key.attributes.attributes.clone()),
    )
    .map_err(|message| {
        module_tree::ModuleLoadError::new(module_tree::ModuleLoadStage::Resolve, message)
    })
}

fn tree_kind(kind: WorkerModuleType) -> module_tree::ModuleKind {
    match kind {
        WorkerModuleType::JavaScriptOrWebAssembly => module_tree::ModuleKind::JavaScript,
        WorkerModuleType::Json => module_tree::ModuleKind::Json,
        WorkerModuleType::Text => module_tree::ModuleKind::Text,
    }
}

pub(super) fn tree_phase(phase: ModuleImportPhase) -> module_tree::ModuleImportPhase {
    match phase {
        ModuleImportPhase::Evaluation => module_tree::ModuleImportPhase::Evaluation,
        ModuleImportPhase::Source => module_tree::ModuleImportPhase::Source,
    }
}

fn tree_requests(requests: &[WorkerModuleRequest]) -> Vec<module_tree::ModuleRequestRecord> {
    requests
        .iter()
        .map(|request| module_tree::ModuleRequestRecord {
            specifier: request.specifier.clone(),
            attributes: module_tree::ModuleAttributesKey::from_pairs(
                request.attributes.pairs().to_vec(),
            ),
            phase: tree_phase(request.phase),
            position: module_tree::TextPosition::default(),
        })
        .collect()
}

pub(super) fn fetch_metadata(
    credentials: RequestCredentialsMode,
    policy: Option<&str>,
) -> module_tree::ModuleFetchMetadata {
    module_tree::ModuleFetchMetadata {
        credentials_mode: match credentials {
            RequestCredentialsMode::Omit => module_tree::CredentialsMode::Omit,
            RequestCredentialsMode::SameOrigin => module_tree::CredentialsMode::SameOrigin,
            RequestCredentialsMode::Include => module_tree::CredentialsMode::Include,
        },
        referrer_policy: match policy {
            Some("no-referrer") => module_tree::ReferrerPolicy::NoReferrer,
            Some("no-referrer-when-downgrade") => {
                module_tree::ReferrerPolicy::NoReferrerWhenDowngrade
            }
            Some("origin") => module_tree::ReferrerPolicy::Origin,
            Some("origin-when-cross-origin") => module_tree::ReferrerPolicy::OriginWhenCrossOrigin,
            Some("same-origin") => module_tree::ReferrerPolicy::SameOrigin,
            Some("strict-origin") => module_tree::ReferrerPolicy::StrictOrigin,
            Some("strict-origin-when-cross-origin") => {
                module_tree::ReferrerPolicy::StrictOriginWhenCrossOrigin
            }
            Some("unsafe-url") => module_tree::ReferrerPolicy::UnsafeUrl,
            _ => module_tree::ReferrerPolicy::EmptyString,
        },
        ..module_tree::ModuleFetchMetadata::default()
    }
}

fn native_referrer_policy(policy: module_tree::ReferrerPolicy) -> Option<&'static str> {
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
