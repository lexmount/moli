use std::collections::{HashMap, VecDeque};

use crate::host::ModuleScriptTreeHost;
use crate::types::*;

#[derive(Debug, Clone)]
pub struct ModuleScriptTreeJob {
    tree_id: ModuleTreeId,
    client: ModuleTreeClientToken,
    root: ModuleRootInput,
    config: ModuleTreeConfig,
    state: ModuleTreeState,
    root_key: Option<ModuleMapKey>,
    root_entry: Option<ModuleEntryId>,
    next_single_client_sequence: u64,
    pending_clients: HashMap<SingleModuleClientToken, PendingModuleLoad>,
    queued_or_visited: HashMap<ModuleMapKey, ModuleImportPhase>,
    entries: Vec<ModuleEntryId>,
    entry_phases: HashMap<ModuleEntryId, ModuleImportPhase>,
    entry_contexts: HashMap<ModuleEntryId, ModuleEntryContext>,
    dependency_edges: Vec<ModuleDependencyEdge>,
    queued_fetches: VecDeque<ModuleFetchRequest>,
    queued_completions: VecDeque<ModuleFetchResult>,
    joined_fetches: Vec<ModuleFetchRequest>,
    module_order: Vec<ModuleMapKey>,
    parse_errors: HashMap<ModuleMapKey, ModuleLoadError>,
    terminal: Option<ModuleTreeTerminalState>,
}

#[derive(Debug, Clone)]
struct PendingModuleLoad {
    key: ModuleMapKey,
    phase: ModuleImportPhase,
    graph_level: ModuleGraphLevel,
}

#[derive(Debug, Clone)]
struct ModuleEntryContext {
    key: ModuleMapKey,
    base_url: url::Url,
    fetch_metadata: ModuleFetchMetadata,
}

impl ModuleScriptTreeJob {
    pub fn new(root: ModuleRootInput, config: ModuleTreeConfig) -> Self {
        let tree_id = config.tree_id;
        Self {
            tree_id,
            client: ModuleTreeClientToken {
                tree_id,
                sequence: config.client_sequence,
            },
            root,
            config,
            state: ModuleTreeState::Initial,
            root_key: None,
            root_entry: None,
            next_single_client_sequence: 0,
            pending_clients: HashMap::new(),
            queued_or_visited: HashMap::new(),
            entries: Vec::new(),
            entry_phases: HashMap::new(),
            entry_contexts: HashMap::new(),
            dependency_edges: Vec::new(),
            queued_fetches: VecDeque::new(),
            queued_completions: VecDeque::new(),
            joined_fetches: Vec::new(),
            module_order: Vec::new(),
            parse_errors: HashMap::new(),
            terminal: None,
        }
    }

    pub fn state(&self) -> ModuleTreeState {
        self.state
    }

    pub fn tree_id(&self) -> ModuleTreeId {
        self.tree_id
    }

    pub fn client(&self) -> ModuleTreeClientToken {
        self.client
    }

    pub fn pending_client_count(&self) -> usize {
        self.pending_clients.len()
    }

    pub fn poll<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreePoll {
        if let Some(terminal) = self.terminal.clone() {
            return self.poll_terminal(terminal);
        }

        match self.state {
            ModuleTreeState::Initial => self.start(host),
            ModuleTreeState::FetchingRoot
            | ModuleTreeState::FetchingDependencies
            | ModuleTreeState::Linking => self.flush_or_wait(host),
            ModuleTreeState::Finished | ModuleTreeState::Aborted => {
                self.poll_terminal(self.terminal.clone().expect("terminal state should be set"))
            }
        }
    }

    pub fn drive<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreeDrive {
        let poll = self.poll(host);
        self.drive_queued_completions(host, poll)
    }

    pub fn resume_single_module<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        client: SingleModuleClientToken,
        result: ModuleFetchResult,
    ) -> ModuleScriptTreePoll {
        self.resume_single_module_outcome(host, client, result.outcome)
    }

    pub fn resume_single_module_and_drive<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        client: SingleModuleClientToken,
        result: ModuleFetchResult,
    ) -> ModuleScriptTreeDrive {
        let poll = self.resume_single_module(host, client, result);
        self.drive_queued_completions(host, poll)
    }

    pub fn resume_single_module_outcome<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        client: SingleModuleClientToken,
        outcome: ModuleFetchOutcome,
    ) -> ModuleScriptTreePoll {
        if self.state == ModuleTreeState::Aborted || self.state == ModuleTreeState::Finished {
            return ModuleScriptTreePoll::IgnoredStaleCompletion;
        }
        let Some(load) = self.pending_clients.remove(&client) else {
            return ModuleScriptTreePoll::IgnoredStaleCompletion;
        };

        self.finish_module_load(host, load, outcome)
    }

    pub fn resume_single_module_outcome_and_drive<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        client: SingleModuleClientToken,
        outcome: ModuleFetchOutcome,
    ) -> ModuleScriptTreeDrive {
        let poll = self.resume_single_module_outcome(host, client, outcome);
        self.drive_queued_completions(host, poll)
    }

    pub fn cancel(&mut self, reason: ModuleTreeAbortReason) -> ModuleScriptTreePoll {
        self.abort(reason)
    }

    fn start<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreePoll {
        match self.root.clone() {
            ModuleRootInput::External(root) => self.start_external_root(host, root),
            ModuleRootInput::Inline(root) => self.start_inline_root(host, root),
        }
    }

    fn start_external_root<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        root: ModuleExternalRootInput,
    ) -> ModuleScriptTreePoll {
        let kind = root.kind_hint.unwrap_or(ModuleKind::JavaScript);
        let key = ModuleMapKey::new(root.source_url.clone(), kind, root.attributes.clone());
        self.root_key = Some(key.clone());
        self.queued_or_visited.insert(key.clone(), root.phase);
        self.record_module_order(key.clone());
        self.state = ModuleTreeState::FetchingRoot;

        let client = self.next_single_module_client();
        let request = ModuleFetchRequest {
            key,
            tree_id: self.tree_id,
            client,
            specifier: None,
            source_url: root.source_url.clone(),
            base_url: root.base_url.clone(),
            initiator_url: root.initiator_url,
            referrer: root.referrer,
            position: root.position,
            parent: None,
            kind,
            attributes: root.attributes,
            phase: root.phase,
            graph_level: ModuleGraphLevel::TopLevel,
            fetch_metadata: root.fetch_metadata,
            render_blocking: RenderBlockingBehavior::Blocking,
            requester: self.config.owner.requester,
            ordering: self.config.owner.ordering,
            custom_fetch_type: self.config.custom_fetch_type,
        };
        self.queue_fetch_request(request);
        self.flush_or_wait(host)
    }

    fn start_inline_root<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        root: ModuleInlineRootInput,
    ) -> ModuleScriptTreePoll {
        self.root_key = Some(root.root_key.clone());
        self.root_entry = Some(root.root_entry);
        self.queued_or_visited
            .insert(root.root_key.clone(), root.phase);
        self.record_module_order(root.root_key.clone());
        self.record_entry_context(
            root.root_entry,
            ModuleEntryContext {
                key: root.root_key,
                base_url: root.base_url,
                fetch_metadata: root.fetch_metadata,
            },
        );
        self.add_entry(root.root_entry, root.phase);
        self.state = ModuleTreeState::FetchingDependencies;

        if root.phase == ModuleImportPhase::Source {
            return self.link_if_ready(host);
        }
        self.discover_dependencies(host, root.root_entry)
    }

    fn finish_module_load<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        load: PendingModuleLoad,
        outcome: ModuleFetchOutcome,
    ) -> ModuleScriptTreePoll {
        let mut current_load_had_parse_error = false;
        let (entry, context) = match outcome {
            ModuleFetchOutcome::Fetched(source) => {
                let context = ModuleEntryContext {
                    key: source.key.clone(),
                    base_url: source.base_url.clone(),
                    fetch_metadata: source.effective_fetch_metadata.clone(),
                };
                match host.compile_module_source(*source, load.phase) {
                    Ok(compiled) => {
                        if let Some(error) = compiled.parse_error.clone() {
                            self.record_parse_error(compiled.key.clone(), error);
                            current_load_had_parse_error = true;
                        } else if compiled.has_parse_error {
                            self.record_parse_error(
                                compiled.key.clone(),
                                ModuleLoadError::new(
                                    ModuleLoadStage::Compile,
                                    "module source had a parse error",
                                )
                                .with_key(compiled.key.clone())
                                .with_error_constructor(ModuleErrorConstructorKind::SyntaxError),
                            );
                            current_load_had_parse_error = true;
                        }
                        (compiled.entry, context)
                    }
                    Err(error) => {
                        host.mark_module_failed(load.key.clone(), error.clone());
                        return self.finish_module_load_error(host, load.key, error);
                    }
                }
            }
            ModuleFetchOutcome::Ready(module) => (
                module.entry,
                ModuleEntryContext {
                    key: module.key,
                    base_url: module.base_url,
                    fetch_metadata: module.effective_fetch_metadata,
                },
            ),
            ModuleFetchOutcome::Failed(error) => {
                return self.finish_module_load_error(host, load.key, error);
            }
        };
        if load.graph_level == ModuleGraphLevel::TopLevel {
            self.root_entry = Some(entry);
            self.state = ModuleTreeState::FetchingDependencies;
        }
        self.record_entry_context(entry, context);
        self.add_entry(entry, load.phase);

        if current_load_had_parse_error {
            return self.finish_after_parse_error(host);
        }
        if load.phase == ModuleImportPhase::Source {
            return self.link_if_ready(host);
        }
        self.discover_dependencies(host, entry)
    }

    fn discover_dependencies<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        entry: ModuleEntryId,
    ) -> ModuleScriptTreePoll {
        let snapshot = match host.module_dependencies(entry) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Some(key) = error.key.as_deref().cloned() {
                    return self.finish_module_load_error(host, key, error);
                }
                return self.fail(error);
            }
        };
        let snapshot = self.dependency_snapshot_with_tree_context(snapshot);
        if snapshot.requested_modules.is_empty() {
            return self.link_if_ready(host);
        }

        let parent = ParentModuleRef {
            key: snapshot.key.clone(),
            entry: snapshot.entry,
            base_url: snapshot.base_url.clone(),
            effective_fetch_metadata: snapshot.effective_fetch_metadata.clone(),
        };
        let candidates = match self.collect_dependency_candidates(host, &snapshot) {
            Ok(candidates) => candidates,
            Err(error) => {
                let error = host.cache_module_request_error(snapshot.key.clone(), error);
                self.record_parse_error(snapshot.key, error);
                return self.finish_after_parse_error(host);
            }
        };

        for candidate in candidates {
            if candidate.previous_phase == Some(candidate.phase) {
                continue;
            }
            let client = self.next_single_module_client();
            let request = self.dependency_fetch_request(&snapshot, &parent, candidate, client);
            self.queue_fetch_request(request);
        }
        self.flush_or_link(host)
    }

    fn collect_dependency_candidates<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        snapshot: &ModuleDependencySnapshot,
    ) -> Result<Vec<DependencyCandidate>, ModuleLoadError> {
        let mut candidates: Vec<DependencyCandidate> = Vec::new();
        for request_record in &snapshot.requested_modules {
            let resolved = host.resolve_module_request(
                &request_record.specifier,
                &snapshot.base_url,
                &request_record.attributes,
                request_record.phase,
            )?;
            let existing_phase = self.queued_or_visited.get(&resolved.key).copied();
            let strongest_phase = existing_phase
                .unwrap_or(request_record.phase)
                .strongest(request_record.phase);
            self.queued_or_visited
                .insert(resolved.key.clone(), strongest_phase);
            if existing_phase.is_none() {
                self.record_module_order(resolved.key.clone());
            }
            self.dependency_edges.push(ModuleDependencyEdge {
                parent_key: snapshot.key.clone(),
                parent_entry: snapshot.entry,
                child_key: resolved.key.clone(),
                specifier: request_record.specifier.clone(),
                attributes: request_record.attributes.clone(),
                phase: request_record.phase,
                position: request_record.position,
            });

            if let Some(candidate) = candidates
                .iter_mut()
                .find(|candidate| candidate.resolved.key == resolved.key)
            {
                candidate.phase = candidate.phase.strongest(request_record.phase);
                continue;
            }
            candidates.push(DependencyCandidate {
                resolved,
                specifier: request_record.specifier.clone(),
                phase: request_record.phase,
                previous_phase: existing_phase,
                position: request_record.position,
            });
        }
        Ok(candidates)
    }

    fn dependency_fetch_request(
        &self,
        snapshot: &ModuleDependencySnapshot,
        parent: &ParentModuleRef,
        candidate: DependencyCandidate,
        client: SingleModuleClientToken,
    ) -> ModuleFetchRequest {
        let mut fetch_metadata = snapshot.effective_fetch_metadata.descendant();
        fetch_metadata.integrity = candidate.resolved.integrity.clone();
        ModuleFetchRequest {
            key: candidate.resolved.key.clone(),
            tree_id: self.tree_id,
            client,
            specifier: Some(candidate.specifier),
            source_url: candidate.resolved.source_url,
            base_url: candidate.resolved.base_url,
            initiator_url: snapshot.base_url.clone(),
            referrer: ModuleReferrer::from_url(snapshot.base_url.clone()),
            position: candidate.position,
            parent: Some(parent.clone()),
            kind: candidate.resolved.kind,
            attributes: candidate.resolved.attributes,
            phase: candidate.phase,
            graph_level: ModuleGraphLevel::Dependent,
            fetch_metadata,
            render_blocking: RenderBlockingBehavior::NonBlocking,
            requester: self.config.owner.requester,
            ordering: self.config.owner.ordering,
            custom_fetch_type: self.config.custom_fetch_type,
        }
    }

    fn queue_fetch_request(&mut self, request: ModuleFetchRequest) {
        self.queued_fetches.push_back(request);
    }

    fn register_fetch<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        request: ModuleFetchRequest,
    ) -> Option<ModuleFetchRequest> {
        let client = request.client;
        self.pending_clients.insert(
            client,
            PendingModuleLoad {
                key: request.key.clone(),
                phase: request.phase,
                graph_level: request.graph_level,
            },
        );
        match host.start_or_join_single_module_fetch(request.clone(), client) {
            SingleModuleFetchDisposition::StartedNetworkFetch { .. } => Some(request),
            SingleModuleFetchDisposition::JoinedExistingFetch => {
                self.joined_fetches.push(request);
                None
            }
            SingleModuleFetchDisposition::Completed(outcome) => {
                self.queued_completions.push_back(ModuleFetchResult {
                    key: request.key,
                    client,
                    requested_phase: request.phase,
                    outcome,
                });
                None
            }
        }
    }

    fn flush_or_link<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreePoll {
        if !self.queued_fetches.is_empty() {
            return self.flush_or_wait(host);
        }
        self.link_if_ready(host)
    }

    fn flush_or_wait<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreePoll {
        if !self.queued_fetches.is_empty() {
            let mut fetches = Vec::new();
            while let Some(request) = self.queued_fetches.pop_front() {
                if let Some(fetch) = self.register_fetch(host, request) {
                    fetches.push(fetch);
                }
            }
            if !fetches.is_empty() {
                return ModuleScriptTreePoll::NeedFetches(fetches);
            }
        }
        if !self.pending_clients.is_empty() {
            return ModuleScriptTreePoll::WaitingForSingleModuleClients(self.pending_client_wait());
        }
        ModuleScriptTreePoll::Pending
    }

    fn pending_client_wait(&self) -> ModulePendingClientWait {
        ModulePendingClientWait {
            client_count: self.pending_clients.len(),
        }
    }

    fn link_if_ready<H: ModuleScriptTreeHost>(&mut self, host: &mut H) -> ModuleScriptTreePoll {
        if !self.queued_fetches.is_empty() || !self.pending_clients.is_empty() {
            return self.flush_or_wait(host);
        }
        if let Some(error) = self.first_parse_error() {
            return self.fail(error);
        }
        let Some(root_entry) = self.root_entry else {
            return self.fail(ModuleLoadError::new(
                ModuleLoadStage::Link,
                "module tree reached link without root entry",
            ));
        };
        self.state = ModuleTreeState::Linking;
        match host.link_module_graph(root_entry, &self.entries, &self.dependency_edges) {
            Ok(graph) => {
                self.state = ModuleTreeState::Finished;
                self.terminal = Some(ModuleTreeTerminalState::Complete(graph.clone()));
                ModuleScriptTreePoll::Complete(graph)
            }
            Err(error) => self.fail(error),
        }
    }

    fn add_entry(&mut self, entry: ModuleEntryId, phase: ModuleImportPhase) {
        if !self.entries.contains(&entry) {
            self.entries.push(entry);
        }
        self.entry_phases
            .entry(entry)
            .and_modify(|stored| *stored = stored.strongest(phase))
            .or_insert(phase);
    }

    fn record_entry_context(&mut self, entry: ModuleEntryId, context: ModuleEntryContext) {
        self.entry_contexts.entry(entry).or_insert(context);
    }

    fn record_module_order(&mut self, key: ModuleMapKey) {
        if !self.module_order.contains(&key) {
            self.module_order.push(key);
        }
    }

    fn record_parse_error(&mut self, key: ModuleMapKey, error: ModuleLoadError) {
        self.parse_errors.entry(key).or_insert(error);
    }

    fn first_parse_error(&self) -> Option<ModuleLoadError> {
        self.module_order
            .iter()
            .find_map(|key| self.parse_errors.get(key).cloned())
            .or_else(|| self.parse_errors.values().next().cloned())
    }

    fn finish_after_parse_error<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
    ) -> ModuleScriptTreePoll {
        if !self.queued_fetches.is_empty() || !self.pending_clients.is_empty() {
            return self.flush_or_wait(host);
        }
        let error = self.first_parse_error().unwrap_or_else(|| {
            ModuleLoadError::new(ModuleLoadStage::Compile, "module source had a parse error")
                .with_error_constructor(ModuleErrorConstructorKind::SyntaxError)
        });
        self.fail(error)
    }

    fn finish_module_load_error<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        request_key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleScriptTreePoll {
        if !is_parse_error(&error) {
            return self.fail(error);
        }
        let key = error.key.as_deref().cloned().unwrap_or(request_key);
        self.record_parse_error(key, error);
        self.finish_after_parse_error(host)
    }

    fn dependency_snapshot_with_tree_context(
        &self,
        mut snapshot: ModuleDependencySnapshot,
    ) -> ModuleDependencySnapshot {
        if let Some(context) = self.entry_contexts.get(&snapshot.entry) {
            snapshot.key = context.key.clone();
            snapshot.base_url = context.base_url.clone();
            snapshot.effective_fetch_metadata = context.fetch_metadata.clone();
        }
        snapshot
    }

    fn next_single_module_client(&mut self) -> SingleModuleClientToken {
        let token = SingleModuleClientToken {
            tree_id: self.tree_id,
            sequence: self.next_single_client_sequence,
        };
        self.next_single_client_sequence += 1;
        token
    }

    fn poll_terminal(&self, terminal: ModuleTreeTerminalState) -> ModuleScriptTreePoll {
        match terminal {
            ModuleTreeTerminalState::Complete(graph) => ModuleScriptTreePoll::Complete(graph),
            ModuleTreeTerminalState::Failed(error) => ModuleScriptTreePoll::Failed(error),
            ModuleTreeTerminalState::Aborted(reason) => ModuleScriptTreePoll::Aborted(reason),
        }
    }

    fn fail(&mut self, error: ModuleLoadError) -> ModuleScriptTreePoll {
        self.state = ModuleTreeState::Finished;
        self.pending_clients.clear();
        self.queued_fetches.clear();
        self.queued_completions.clear();
        self.joined_fetches.clear();
        self.terminal = Some(ModuleTreeTerminalState::Failed(error.clone()));
        ModuleScriptTreePoll::Failed(error)
    }

    fn abort(&mut self, reason: ModuleTreeAbortReason) -> ModuleScriptTreePoll {
        self.state = ModuleTreeState::Aborted;
        self.pending_clients.clear();
        self.queued_fetches.clear();
        self.queued_completions.clear();
        self.joined_fetches.clear();
        self.terminal = Some(ModuleTreeTerminalState::Aborted(reason));
        ModuleScriptTreePoll::Aborted(reason)
    }

    fn drive_queued_completions<H: ModuleScriptTreeHost>(
        &mut self,
        host: &mut H,
        mut poll: ModuleScriptTreePoll,
    ) -> ModuleScriptTreeDrive {
        let mut pending_fetches = Vec::new();
        if let ModuleScriptTreePoll::NeedFetches(fetches) = poll {
            pending_fetches.extend(fetches);
            poll = ModuleScriptTreePoll::Pending;
        }
        while let Some(completion) = self.queued_completions.pop_front() {
            let client = completion.client;
            match self.resume_single_module(host, client, completion) {
                ModuleScriptTreePoll::NeedFetches(fetches) => {
                    pending_fetches.extend(fetches);
                }
                ModuleScriptTreePoll::Failed(error) => {
                    return ModuleScriptTreeDrive::Failed(error);
                }
                ModuleScriptTreePoll::Aborted(reason) => {
                    return ModuleScriptTreeDrive::Aborted(reason);
                }
                ModuleScriptTreePoll::Complete(graph) => {
                    return ModuleScriptTreeDrive::Complete(graph);
                }
                other => {
                    poll = other;
                }
            }
        }

        let joined_fetches = std::mem::take(&mut self.joined_fetches);
        if !pending_fetches.is_empty() {
            return ModuleScriptTreeDrive::NeedFetches(ModuleScriptTreeFetches {
                fetches: pending_fetches,
                joined_fetches,
            });
        }
        match poll {
            ModuleScriptTreePoll::Pending => {
                ModuleScriptTreeDrive::Pending(ModuleScriptTreeIdle { joined_fetches })
            }
            ModuleScriptTreePoll::NeedFetches(fetches) => {
                ModuleScriptTreeDrive::NeedFetches(ModuleScriptTreeFetches {
                    fetches,
                    joined_fetches,
                })
            }
            ModuleScriptTreePoll::WaitingForSingleModuleClients(wait) => {
                ModuleScriptTreeDrive::WaitingForSingleModuleClients(ModuleScriptTreeWait {
                    client_count: wait.client_count,
                    joined_fetches,
                })
            }
            ModuleScriptTreePoll::Complete(graph) => ModuleScriptTreeDrive::Complete(graph),
            ModuleScriptTreePoll::Failed(error) => ModuleScriptTreeDrive::Failed(error),
            ModuleScriptTreePoll::Aborted(reason) => ModuleScriptTreeDrive::Aborted(reason),
            ModuleScriptTreePoll::IgnoredStaleCompletion => {
                ModuleScriptTreeDrive::IgnoredStaleCompletion(ModuleScriptTreeIdle {
                    joined_fetches,
                })
            }
        }
    }
}

#[derive(Clone)]
struct DependencyCandidate {
    resolved: ResolvedModuleRequest,
    specifier: String,
    phase: ModuleImportPhase,
    previous_phase: Option<ModuleImportPhase>,
    position: TextPosition,
}

fn is_parse_error(error: &ModuleLoadError) -> bool {
    error.exception_id.is_some()
        || error.error_constructor == Some(ModuleErrorConstructorKind::SyntaxError)
}
