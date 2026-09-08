use std::collections::HashMap;

use moli_module_script_tree::{
    CompiledModuleSnapshot, CredentialsMode, FetchPriorityHint, FetchedModuleSource,
    ModuleAttributesKey, ModuleDependencyEdge, ModuleDependencySnapshot, ModuleEntryId,
    ModuleErrorConstructorKind, ModuleExternalRootInput, ModuleFetchMetadata, ModuleFetchOutcome,
    ModuleFetchRequest, ModuleFetchResult, ModuleGraphHandle, ModuleGraphLevel, ModuleImportPhase,
    ModuleKind, ModuleLoadError, ModuleLoadStage, ModuleMapKey, ModuleReferrer,
    ModuleRequestContext, ModuleRequestRecord, ModuleRootInput, ModuleScriptTreeDrive,
    ModuleScriptTreeHost, ModuleScriptTreeJob, ModuleScriptTreePoll, ModuleSource,
    ModuleTreeAbortReason, ModuleTreeConfig, ModuleTreeId, ModuleTreeOwner, ModuleTreeState,
    ReadyModule, ReferrerPolicy, RenderBlockingBehavior, ResolvedModuleRequest,
    ScriptFetchSchedulerPriority, SingleModuleClientToken, SingleModuleFetchDisposition,
    TextPosition,
};
use url::Url;

#[derive(Debug, Clone)]
enum FakeEntry {
    Fetching {
        clients: Vec<SingleModuleClientToken>,
        phase: ModuleImportPhase,
    },
    Ready {
        entry: ModuleEntryId,
        phase: ModuleImportPhase,
        deps: Vec<ModuleRequestRecord>,
        metadata: ModuleFetchMetadata,
    },
    Failed {
        error: ModuleLoadError,
        phase: ModuleImportPhase,
    },
}

#[derive(Debug, Default)]
struct FakeHost {
    entries: HashMap<ModuleMapKey, FakeEntry>,
    started: Vec<ModuleFetchRequest>,
    queued: Vec<ModuleFetchResult>,
    compile_errors: HashMap<ModuleMapKey, ModuleLoadError>,
    link_calls: usize,
    next_fetch_id: u64,
    next_entry_id: u32,
    fail_link: Option<ModuleLoadError>,
}

impl FakeHost {
    fn new() -> Self {
        Self { ..Self::default() }
    }

    fn ready(&mut self, key: ModuleMapKey, entry: ModuleEntryId, deps: Vec<ModuleRequestRecord>) {
        self.next_entry_id = self.next_entry_id.max(entry.0);
        self.entries.insert(
            key,
            FakeEntry::Ready {
                entry,
                phase: ModuleImportPhase::Evaluation,
                deps,
                metadata: ModuleFetchMetadata::default(),
            },
        );
    }

    fn fail_compile(&mut self, key: ModuleMapKey, error: ModuleLoadError) {
        self.compile_errors.insert(key, error);
    }

    fn take_single_client(&self, key: &ModuleMapKey) -> SingleModuleClientToken {
        match self.entries.get(key).expect("entry should exist") {
            FakeEntry::Fetching { clients, .. } => {
                *clients.first().expect("fetching entry should have client")
            }
            _ => panic!("entry should be fetching"),
        }
    }
}

impl ModuleScriptTreeHost for FakeHost {
    fn resolve_module_request(
        &mut self,
        specifier: &str,
        base_url: &Url,
        attributes: &ModuleAttributesKey,
        requested_phase: ModuleImportPhase,
    ) -> Result<ResolvedModuleRequest, ModuleLoadError> {
        let source_url = base_url.join(specifier).map_err(|error| {
            ModuleLoadError::new(ModuleLoadStage::Resolve, error.to_string())
                .with_error_constructor(ModuleErrorConstructorKind::TypeError)
        })?;
        let kind = if attributes
            .attributes
            .iter()
            .any(|(key, value)| key == "type" && value == "json")
        {
            ModuleKind::Json
        } else {
            ModuleKind::JavaScript
        };
        let key = ModuleMapKey::new(source_url.clone(), kind, attributes.clone());
        Ok(ResolvedModuleRequest {
            key,
            source_url,
            base_url: base_url.clone(),
            kind,
            attributes: attributes.clone(),
            phase: requested_phase,
            integrity: None,
        })
    }

    fn start_or_join_single_module_fetch(
        &mut self,
        request: ModuleFetchRequest,
        client: SingleModuleClientToken,
    ) -> SingleModuleFetchDisposition {
        match self.entries.get_mut(&request.key) {
            Some(FakeEntry::Fetching { clients, phase }) => {
                clients.push(client);
                *phase = phase.strongest(request.phase);
                SingleModuleFetchDisposition::JoinedExistingFetch
            }
            Some(FakeEntry::Ready {
                entry,
                phase,
                metadata,
                ..
            }) => {
                *phase = phase.strongest(request.phase);
                let key = request.key.clone();
                let result = ModuleFetchResult {
                    key: request.key,
                    client,
                    requested_phase: request.phase,
                    outcome: ready_outcome_with_metadata(key, *entry, metadata.clone()),
                };
                self.queued.push(result.clone());
                SingleModuleFetchDisposition::Completed(result.outcome)
            }
            Some(FakeEntry::Failed { error, phase }) => {
                *phase = phase.strongest(request.phase);
                let result = ModuleFetchResult {
                    key: request.key,
                    client,
                    requested_phase: request.phase,
                    outcome: ModuleFetchOutcome::Failed(error.clone()),
                };
                self.queued.push(result.clone());
                SingleModuleFetchDisposition::Completed(result.outcome)
            }
            None => {
                self.entries.insert(
                    request.key.clone(),
                    FakeEntry::Fetching {
                        clients: vec![client],
                        phase: request.phase,
                    },
                );
                self.started.push(request);
                let fetch_id = moli_module_script_tree::ModuleFetchId(self.next_fetch_id);
                self.next_fetch_id += 1;
                SingleModuleFetchDisposition::StartedNetworkFetch { fetch_id }
            }
        }
    }

    fn compile_module_source(
        &mut self,
        fetched_source: FetchedModuleSource,
        phase: ModuleImportPhase,
    ) -> Result<CompiledModuleSnapshot, ModuleLoadError> {
        let key = fetched_source.key;
        if let Some(error) = self.compile_errors.get(&key).cloned() {
            return Err(error);
        }
        self.next_entry_id += 1;
        let entry = ModuleEntryId(self.next_entry_id);
        self.entries.insert(
            key.clone(),
            FakeEntry::Ready {
                entry,
                phase,
                deps: Vec::new(),
                metadata: fetched_source.effective_fetch_metadata.clone(),
            },
        );
        Ok(CompiledModuleSnapshot {
            entry,
            key,
            base_url: fetched_source.base_url,
            effective_fetch_metadata: fetched_source.effective_fetch_metadata,
            requested_modules: Vec::new(),
            phase,
            has_parse_error: false,
            parse_error: None,
        })
    }

    fn module_dependencies(
        &self,
        entry: ModuleEntryId,
    ) -> Result<ModuleDependencySnapshot, ModuleLoadError> {
        self.entries
            .iter()
            .find_map(|(key, entry_state)| match entry_state {
                FakeEntry::Ready {
                    entry: ready_entry,
                    deps,
                    metadata,
                    ..
                } if *ready_entry == entry => Some(ModuleDependencySnapshot {
                    entry,
                    key: key.clone(),
                    base_url: key.url.clone(),
                    effective_fetch_metadata: metadata.clone(),
                    requested_modules: deps.clone(),
                }),
                _ => None,
            })
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::DependencyDiscovery,
                    "missing dependency snapshot",
                )
            })
    }

    fn link_module_graph(
        &mut self,
        root: ModuleEntryId,
        entries: &[ModuleEntryId],
        dependency_edges: &[ModuleDependencyEdge],
    ) -> Result<ModuleGraphHandle, ModuleLoadError> {
        self.link_calls += 1;
        if let Some(error) = self.fail_link.clone() {
            return Err(error);
        }
        Ok(ModuleGraphHandle {
            root_entry: root,
            entries: entries.to_vec(),
            entry_phases: HashMap::new(),
            dependency_edges: dependency_edges.to_vec(),
        })
    }

    fn mark_module_failed(&mut self, _key: ModuleMapKey, _error: ModuleLoadError) -> ModuleEntryId {
        ModuleEntryId(0)
    }

    fn cache_module_request_error(
        &mut self,
        key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleLoadError {
        let error = error.with_key(key.clone()).with_exception_id(
            moli_module_script_tree::ModuleExceptionId(self.next_entry_id as u64),
        );
        self.next_entry_id += 1;
        self.entries.insert(
            key,
            FakeEntry::Failed {
                error: error.clone(),
                phase: ModuleImportPhase::Evaluation,
            },
        );
        error
    }
}

fn url(input: &str) -> Url {
    Url::parse(input).expect("test URL should parse")
}

fn assert_waiting_for_owned_clients(poll: ModuleScriptTreePoll, expected_count: usize) {
    let ModuleScriptTreePoll::WaitingForSingleModuleClients(wait) = poll else {
        panic!("expected pending single-module clients");
    };
    assert_eq!(wait.client_count, expected_count);
}

fn assert_waiting_for_clients(poll: ModuleScriptTreePoll, expected_count: usize) {
    let ModuleScriptTreePoll::WaitingForSingleModuleClients(wait) = poll else {
        panic!("expected pending single-module clients");
    };
    assert_eq!(wait.client_count, expected_count);
}

fn key(input: &str) -> ModuleMapKey {
    ModuleMapKey::javascript(url(input))
}

fn ready_outcome(key: ModuleMapKey, entry: ModuleEntryId) -> ModuleFetchOutcome {
    ready_outcome_with_metadata(key, entry, ModuleFetchMetadata::default())
}

fn ready_outcome_with_metadata(
    key: ModuleMapKey,
    entry: ModuleEntryId,
    metadata: ModuleFetchMetadata,
) -> ModuleFetchOutcome {
    ModuleFetchOutcome::Ready(Box::new(ReadyModule::new(
        entry,
        key.clone(),
        key.url.clone(),
        metadata,
    )))
}

fn fetched_outcome(key: ModuleMapKey, source: ModuleSource) -> ModuleFetchOutcome {
    ModuleFetchOutcome::Fetched(Box::new(FetchedModuleSource::new(
        key.clone(),
        key.clone(),
        key.url.clone(),
        key.url.clone(),
        source,
        ModuleFetchMetadata::default(),
    )))
}

fn request(specifier: &str, phase: ModuleImportPhase) -> ModuleRequestRecord {
    ModuleRequestRecord {
        specifier: specifier.to_owned(),
        attributes: ModuleAttributesKey::empty(),
        phase,
        position: TextPosition { line: 1, column: 1 },
    }
}

fn syntax_error(key: ModuleMapKey, message: &str) -> ModuleLoadError {
    ModuleLoadError::new(ModuleLoadStage::Compile, message)
        .with_key(key)
        .with_error_constructor(ModuleErrorConstructorKind::SyntaxError)
}

fn external_root(source_url: Url, phase: ModuleImportPhase) -> ModuleRootInput {
    ModuleRootInput::External(ModuleExternalRootInput {
        base_url: source_url.clone(),
        initiator_url: source_url.clone(),
        source_url,
        attributes: ModuleAttributesKey::empty(),
        phase,
        kind_hint: Some(ModuleKind::JavaScript),
        fetch_metadata: ModuleFetchMetadata {
            integrity: Some("sha256-root".to_owned()),
            nonce: Some("nonce".to_owned()),
            charset: Some("utf-8".to_owned()),
            fetch_priority: FetchPriorityHint::High,
            ..ModuleFetchMetadata::default()
        },
        referrer: ModuleReferrer::client(),
        position: TextPosition::default(),
    })
}

fn inline_root(root_key: ModuleMapKey, root_entry: ModuleEntryId) -> ModuleRootInput {
    inline_root_with_metadata(root_key, root_entry, ModuleFetchMetadata::default())
}

fn inline_root_with_metadata(
    root_key: ModuleMapKey,
    root_entry: ModuleEntryId,
    fetch_metadata: ModuleFetchMetadata,
) -> ModuleRootInput {
    ModuleRootInput::Inline(moli_module_script_tree::ModuleInlineRootInput {
        root_key,
        root_entry,
        source_url: url("https://example.test/app/root.mjs"),
        base_url: url("https://example.test/app/root.mjs"),
        phase: ModuleImportPhase::Evaluation,
        fetch_metadata,
        referrer: ModuleReferrer::client(),
        position: TextPosition::default(),
    })
}

fn job(root: ModuleRootInput) -> ModuleScriptTreeJob {
    job_with_config(
        root,
        ModuleTreeConfig {
            tree_id: ModuleTreeId(7),
            ..ModuleTreeConfig::default()
        },
    )
}

fn job_with_config(root: ModuleRootInput, config: ModuleTreeConfig) -> ModuleScriptTreeJob {
    ModuleScriptTreeJob::new(
        root,
        ModuleTreeConfig {
            tree_id: ModuleTreeId(7),
            ..config
        },
    )
}

#[test]
fn chromium_fetch_tree_external_root_returns_top_level_fetch() {
    let root_url = url("https://example.test/app/root.mjs");
    let mut job = job(external_root(
        root_url.clone(),
        ModuleImportPhase::Evaluation,
    ));
    let mut host = FakeHost::new();

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("external root should need fetch");
    };

    assert_eq!(fetches.len(), 1);
    assert_eq!(fetches[0].source_url, root_url);
    assert_eq!(fetches[0].graph_level, ModuleGraphLevel::TopLevel);
    assert_eq!(fetches[0].render_blocking, RenderBlockingBehavior::Blocking);
}

#[test]
fn chromium_inline_root_skips_root_fetch_and_fetches_descendants() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("inline root should fetch descendants");
    };

    assert_eq!(fetches.len(), 1);
    assert_eq!(fetches[0].key, child_key);
    assert_eq!(fetches[0].graph_level, ModuleGraphLevel::Dependent);
}

#[test]
fn chromium_descendant_fetch_options_use_effective_parent_metadata() {
    let root_key = key("https://example.test/app/root.mjs");
    let metadata = ModuleFetchMetadata {
        credentials_mode: CredentialsMode::Include,
        referrer_policy: ReferrerPolicy::StrictOrigin,
        integrity: Some("sha256-parent".to_owned()),
        nonce: Some("parent-nonce".to_owned()),
        charset: Some("utf-8".to_owned()),
        fetch_priority: FetchPriorityHint::High,
        scheduler_priority: Some(ScriptFetchSchedulerPriority::VeryHigh),
        request_context: ModuleRequestContext::Script,
        parser_inserted: true,
        ..ModuleFetchMetadata::default()
    };
    let mut host = FakeHost::new();
    host.entries.insert(
        root_key.clone(),
        FakeEntry::Ready {
            entry: ModuleEntryId(1),
            phase: ModuleImportPhase::Evaluation,
            deps: vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
            metadata: metadata.clone(),
        },
    );
    let mut job = job(inline_root_with_metadata(
        root_key,
        ModuleEntryId(1),
        metadata,
    ));

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("dependency should fetch");
    };

    let child_metadata = &fetches[0].fetch_metadata;
    assert_eq!(child_metadata.credentials_mode, CredentialsMode::Include);
    assert_eq!(child_metadata.referrer_policy, ReferrerPolicy::StrictOrigin);
    assert_eq!(child_metadata.integrity, None);
    assert_eq!(child_metadata.nonce.as_deref(), Some("parent-nonce"));
    assert_eq!(child_metadata.charset, None);
    assert_eq!(child_metadata.fetch_priority, FetchPriorityHint::Auto);
    assert_eq!(child_metadata.scheduler_priority, None);
    assert!(child_metadata.parser_inserted);
    assert_eq!(
        fetches[0].render_blocking,
        RenderBlockingBehavior::NonBlocking
    );
}

#[test]
fn chromium_source_phase_does_not_fetch_descendants() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    let mut job = job(external_root(root_url, ModuleImportPhase::Source));
    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("root source phase should fetch root");
    };
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    let result = ModuleFetchResult {
        key: root_key.clone(),
        client: fetches[0].client,
        requested_phase: ModuleImportPhase::Source,
        outcome: ready_outcome(root_key, ModuleEntryId(1)),
    };

    let ModuleScriptTreePoll::Complete(graph) =
        job.resume_single_module(&mut host, fetches[0].client, result)
    else {
        panic!("source phase root should complete without descendants");
    };

    assert!(graph.dependency_edges.is_empty());
    assert_eq!(host.started.len(), 1);
}

#[test]
fn network_fetch_source_is_compiled_by_tree_before_link() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    let mut job = job(external_root(
        root_url.clone(),
        ModuleImportPhase::Evaluation,
    ));
    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("root should fetch");
    };
    let result = ModuleFetchResult {
        key: root_key.clone(),
        client: fetches[0].client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: fetched_outcome(root_key, ModuleSource::Text("export {};".to_owned())),
    };

    let ModuleScriptTreePoll::Complete(graph) =
        job.resume_single_module(&mut host, fetches[0].client, result)
    else {
        panic!("fetched source should be compiled and linked by the tree");
    };

    assert_eq!(graph.root_entry, ModuleEntryId(1));
    assert_eq!(host.link_calls, 1);
}

#[test]
fn chromium_module_map_fetching_entry_joins_without_new_fetch() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let poll = job.poll(&mut host);

    assert_waiting_for_clients(poll, 1);
    assert_eq!(host.started.len(), 0);
    assert_eq!(job.pending_client_count(), 1);
}

#[test]
fn chromium_module_map_ready_hit_queues_completion_without_reentrant_link() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    host.ready(root_key.clone(), ModuleEntryId(1), Vec::new());
    let mut job = job(external_root(root_url, ModuleImportPhase::Evaluation));

    let poll = job.poll(&mut host);

    assert_waiting_for_owned_clients(poll, 1);
    assert_eq!(host.queued.len(), 1);
    assert_eq!(host.link_calls, 0);
}

#[test]
fn drive_consumes_ready_module_map_completion_inside_tree() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    host.ready(root_key, ModuleEntryId(1), Vec::new());
    let mut job = job(external_root(root_url, ModuleImportPhase::Evaluation));

    let ModuleScriptTreeDrive::Complete(graph) = job.drive(&mut host) else {
        panic!("ready module map completion should be driven inside module tree");
    };

    assert_eq!(graph.root_entry, ModuleEntryId(1));
    assert_eq!(host.link_calls, 1);
}

#[test]
fn drive_returns_joined_fetch_waiters_without_renderer_side_channel() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreeDrive::WaitingForSingleModuleClients(wait) = job.drive(&mut host) else {
        panic!("joined module map fetch should wait for owner completion");
    };

    assert_eq!(wait.client_count, 1);
    assert_eq!(wait.joined_fetches.len(), 1);
    assert_eq!(wait.joined_fetches[0].key, child_key);
}

#[test]
fn chromium_module_map_failed_entry_is_sticky() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let error = ModuleLoadError::new(ModuleLoadStage::Fetch, "network failed");
    let mut host = FakeHost::new();
    host.entries.insert(
        root_key,
        FakeEntry::Failed {
            error: error.clone(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(external_root(root_url, ModuleImportPhase::Evaluation));
    assert_waiting_for_owned_clients(job.poll(&mut host), 1);
    let queued = host.queued.remove(0);

    let ModuleScriptTreePoll::Failed(returned) =
        job.resume_single_module(&mut host, queued.client, queued)
    else {
        panic!("sticky failure should fail tree");
    };

    assert_eq!(returned, error);
}

#[test]
fn unknown_single_module_client_completion_is_ignored() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    let mut job = job(external_root(root_url, ModuleImportPhase::Evaluation));
    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("root should fetch");
    };
    let stale_client = SingleModuleClientToken {
        tree_id: fetches[0].client.tree_id,
        sequence: fetches[0].client.sequence + 1,
    };
    let result = ModuleFetchResult {
        key: root_key.clone(),
        client: stale_client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: ready_outcome(root_key, ModuleEntryId(1)),
    };

    let poll = job.resume_single_module(&mut host, stale_client, result);

    assert_eq!(poll, ModuleScriptTreePoll::IgnoredStaleCompletion);
    assert_eq!(job.pending_client_count(), 1);
}

#[test]
fn canceled_tree_ignores_late_single_module_completion() {
    let root_url = url("https://example.test/app/root.mjs");
    let root_key = key(root_url.as_str());
    let mut host = FakeHost::new();
    let mut job = job(external_root(root_url, ModuleImportPhase::Evaluation));
    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("root should fetch");
    };
    assert_eq!(
        job.cancel(ModuleTreeAbortReason::ExplicitCancel),
        ModuleScriptTreePoll::Aborted(ModuleTreeAbortReason::ExplicitCancel)
    );
    let result = ModuleFetchResult {
        key: root_key.clone(),
        client: fetches[0].client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: ready_outcome(root_key, ModuleEntryId(1)),
    };

    let poll = job.resume_single_module(&mut host, fetches[0].client, result);

    assert_eq!(poll, ModuleScriptTreePoll::IgnoredStaleCompletion);
}

#[test]
fn chromium_fetch_descendants_can_issue_sibling_fetches_in_parallel() {
    let root_key = key("https://example.test/app/root.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./a.mjs", ModuleImportPhase::Evaluation),
            request("./b.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("fanout should fetch siblings");
    };

    assert_eq!(fetches.len(), 2);
    assert_eq!(
        fetches[0].source_url.as_str(),
        "https://example.test/app/a.mjs"
    );
    assert_eq!(
        fetches[1].source_url.as_str(),
        "https://example.test/app/b.mjs"
    );
}

#[test]
fn parse_error_waits_for_pending_sibling_fetches_before_failing() {
    let root_key = key("https://example.test/app/root.mjs");
    let a_key = key("https://example.test/app/a.mjs");
    let b_key = key("https://example.test/app/b.mjs");
    let a_error = syntax_error(a_key.clone(), "a parse failed");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./a.mjs", ModuleImportPhase::Evaluation),
            request("./b.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    host.fail_compile(a_key.clone(), a_error.clone());
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    let ModuleScriptTreePoll::NeedFetches(mut fetches) = job.poll(&mut host) else {
        panic!("root should fetch sibling dependencies");
    };
    assert_eq!(fetches.len(), 2);
    let a_index = fetches
        .iter()
        .position(|fetch| fetch.key == a_key)
        .expect("a.mjs fetch should be present");
    let a_fetch = fetches.remove(a_index);

    let wait = job.resume_single_module(
        &mut host,
        a_fetch.client,
        ModuleFetchResult {
            key: a_key.clone(),
            client: a_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: fetched_outcome(a_key.clone(), ModuleSource::Text("bad".to_owned())),
        },
    );
    assert_waiting_for_clients(wait, 1);
    assert_eq!(
        job.state(),
        ModuleTreeState::FetchingDependencies,
        "tree should keep waiting for already-issued sibling fetches"
    );

    let b_fetch = fetches.pop().expect("b.mjs fetch should remain");
    assert_eq!(b_fetch.key, b_key);
    let ModuleScriptTreePoll::Failed(error) = job.resume_single_module(
        &mut host,
        b_fetch.client,
        ModuleFetchResult {
            key: b_key.clone(),
            client: b_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: fetched_outcome(b_key, ModuleSource::Text("export {};".to_owned())),
        },
    ) else {
        panic!("tree should fail with delayed parse error after sibling completion");
    };

    assert_eq!(error, a_error);
    assert_eq!(host.link_calls, 0);
}

#[test]
fn cached_parse_error_waits_for_pending_sibling_network_failure() {
    let root_key = key("https://example.test/app/root.mjs");
    let parse_error_key = key("https://example.test/app/parse-error.mjs");
    let network_error_key = key("https://example.test/app/network-error.mjs");
    let parse_error = syntax_error(parse_error_key.clone(), "cached parse error");
    let network_error = ModuleLoadError::new(ModuleLoadStage::Fetch, "sibling network failure")
        .with_key(network_error_key.clone());
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./parse-error.mjs", ModuleImportPhase::Evaluation),
            request("./network-error.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    host.entries.insert(
        parse_error_key,
        FakeEntry::Failed {
            error: parse_error,
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreeDrive::NeedFetches(mut fetches) = job.drive(&mut host) else {
        panic!("cached parse error should wait for the pending sibling fetch");
    };
    assert_eq!(fetches.fetches.len(), 1);
    let network_fetch = fetches.fetches.pop().expect("network fetch should exist");
    assert_eq!(network_fetch.key, network_error_key);
    assert_eq!(job.pending_client_count(), 1);

    let ModuleScriptTreePoll::Failed(error) = job.resume_single_module(
        &mut host,
        network_fetch.client,
        ModuleFetchResult {
            key: network_error_key,
            client: network_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: ModuleFetchOutcome::Failed(network_error.clone()),
        },
    ) else {
        panic!("network failure should take priority over the cached parse error");
    };

    assert_eq!(error, network_error);
    assert_eq!(host.link_calls, 0);
}

#[test]
fn cached_parse_error_does_not_skip_successful_sibling_descendants() {
    let root_key = key("https://example.test/app/root.mjs");
    let parse_error_key = key("https://example.test/app/parse-error.mjs");
    let sibling_key = key("https://example.test/app/sibling.mjs");
    let nested_network_error_key = key("https://example.test/app/nested-network-error.mjs");
    let parse_error = syntax_error(parse_error_key.clone(), "cached parse error");
    let nested_network_error =
        ModuleLoadError::new(ModuleLoadStage::Fetch, "nested network failure")
            .with_key(nested_network_error_key.clone());
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./parse-error.mjs", ModuleImportPhase::Evaluation),
            request("./sibling.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    host.entries.insert(
        parse_error_key,
        FakeEntry::Failed {
            error: parse_error,
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    let ModuleScriptTreeDrive::NeedFetches(mut root_fetches) = job.drive(&mut host) else {
        panic!("cached parse error should wait for the successful sibling fetch");
    };
    let sibling_fetch = root_fetches
        .fetches
        .pop()
        .expect("sibling fetch should exist");
    assert_eq!(sibling_fetch.key, sibling_key);
    host.ready(
        sibling_key.clone(),
        ModuleEntryId(2),
        vec![request(
            "./nested-network-error.mjs",
            ModuleImportPhase::Evaluation,
        )],
    );

    let ModuleScriptTreePoll::NeedFetches(mut nested_fetches) = job.resume_single_module(
        &mut host,
        sibling_fetch.client,
        ModuleFetchResult {
            key: sibling_key.clone(),
            client: sibling_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: ready_outcome(sibling_key, ModuleEntryId(2)),
        },
    ) else {
        panic!("successful sibling descendants should still be discovered");
    };
    let nested_fetch = nested_fetches
        .pop()
        .expect("nested network fetch should exist");
    assert_eq!(nested_fetch.key, nested_network_error_key);

    let ModuleScriptTreePoll::Failed(error) = job.resume_single_module(
        &mut host,
        nested_fetch.client,
        ModuleFetchResult {
            key: nested_network_error_key,
            client: nested_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: ModuleFetchOutcome::Failed(nested_network_error.clone()),
        },
    ) else {
        panic!("nested network failure should take priority over the cached parse error");
    };

    assert_eq!(error, nested_network_error);
    assert_eq!(host.link_calls, 0);
}

#[test]
fn parse_error_result_uses_module_discovery_order_not_completion_order() {
    let root_key = key("https://example.test/app/root.mjs");
    let a_key = key("https://example.test/app/a.mjs");
    let b_key = key("https://example.test/app/b.mjs");
    let a_error = syntax_error(a_key.clone(), "a parse failed");
    let b_error = syntax_error(b_key.clone(), "b parse failed");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./a.mjs", ModuleImportPhase::Evaluation),
            request("./b.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    host.fail_compile(a_key.clone(), a_error.clone());
    host.fail_compile(b_key.clone(), b_error);
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    let ModuleScriptTreePoll::NeedFetches(mut fetches) = job.poll(&mut host) else {
        panic!("root should fetch sibling dependencies");
    };
    assert_eq!(fetches.len(), 2);
    let b_index = fetches
        .iter()
        .position(|fetch| fetch.key == b_key)
        .expect("b.mjs fetch should be present");
    let b_fetch = fetches.remove(b_index);

    let wait = job.resume_single_module(
        &mut host,
        b_fetch.client,
        ModuleFetchResult {
            key: b_key.clone(),
            client: b_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: fetched_outcome(b_key.clone(), ModuleSource::Text("bad b".to_owned())),
        },
    );
    assert_waiting_for_clients(wait, 1);

    let a_fetch = fetches.pop().expect("a.mjs fetch should remain");
    assert_eq!(a_fetch.key, a_key);
    let ModuleScriptTreePoll::Failed(error) = job.resume_single_module(
        &mut host,
        a_fetch.client,
        ModuleFetchResult {
            key: a_key.clone(),
            client: a_fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: fetched_outcome(a_key, ModuleSource::Text("bad a".to_owned())),
        },
    ) else {
        panic!("tree should fail once all sibling fetches finish");
    };

    assert_eq!(error, a_error);
    assert_eq!(host.link_calls, 0);
}

#[test]
fn static_request_error_is_cached_before_any_dependency_fetch_starts() {
    let root_key = key("https://example.test/app/root.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./valid.mjs", ModuleImportPhase::Evaluation),
            request("http://[", ModuleImportPhase::Evaluation),
        ],
    );
    let mut first = job(inline_root(root_key.clone(), ModuleEntryId(1)));
    let ModuleScriptTreePoll::Failed(error) = first.poll(&mut host) else {
        panic!("invalid static request must fail the graph");
    };
    assert!(
        host.started.is_empty(),
        "all requests must be validated before fetching any"
    );
    assert_eq!(error.key.as_deref(), Some(&root_key));
    assert!(error.exception_id.is_some());
    assert_eq!(
        error.error_constructor,
        Some(ModuleErrorConstructorKind::TypeError)
    );
    let mut second = job(external_root(
        root_key.url.clone(),
        ModuleImportPhase::Evaluation,
    ));
    assert_eq!(
        second.drive(&mut host),
        ModuleScriptTreeDrive::Failed(error)
    );
    assert!(host.started.is_empty());
    assert_eq!(host.link_calls, 0);
}

#[test]
fn static_request_type_error_waits_for_pending_siblings_and_preserves_network_failure() {
    let root_key = key("https://example.test/app/root.mjs");
    let bad_key = key("https://example.test/app/bad.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./bad.mjs", ModuleImportPhase::Evaluation),
            request("./network.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    host.ready(
        bad_key.clone(),
        ModuleEntryId(2),
        vec![request("http://[", ModuleImportPhase::Evaluation)],
    );
    let mut tree = job(inline_root(root_key, ModuleEntryId(1)));
    let ModuleScriptTreePoll::NeedFetches(mut fetches) = tree.poll(&mut host) else {
        panic!("root must start its uncached sibling fetch");
    };
    assert_eq!(fetches.len(), 1);
    let ModuleScriptTreeDrive::WaitingForSingleModuleClients(wait) = tree.drive(&mut host) else {
        panic!("static request failure must wait for the outstanding network fetch");
    };
    assert_eq!(wait.client_count, 1);
    assert!(
        matches!(host.entries.get(&bad_key), Some(FakeEntry::Failed { error, .. })
        if error.exception_id.is_some())
    );
    let fetch = fetches.pop().unwrap();
    let error = ModuleLoadError::new(ModuleLoadStage::Fetch, "network failure");
    let result = tree.resume_single_module(
        &mut host,
        fetch.client,
        ModuleFetchResult {
            key: fetch.key,
            client: fetch.client,
            requested_phase: ModuleImportPhase::Evaluation,
            outcome: ModuleFetchOutcome::Failed(error.clone()),
        },
    );
    assert_eq!(result, ModuleScriptTreePoll::Failed(error));
    assert_eq!(host.link_calls, 0);
}

#[test]
fn evaluation_phase_upgrades_source_phase_for_same_key() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![
            request("./child.mjs", ModuleImportPhase::Source),
            request("./child.mjs", ModuleImportPhase::Evaluation),
        ],
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("child should fetch once at strongest phase");
    };

    assert_eq!(fetches.len(), 1);
    assert_eq!(fetches[0].key, child_key);
    assert_eq!(fetches[0].phase, ModuleImportPhase::Evaluation);
}

#[test]
fn joined_fetch_completion_can_complete_graph() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    assert_waiting_for_clients(job.poll(&mut host), 1);
    let client = host.take_single_client(&child_key);
    host.ready(child_key.clone(), ModuleEntryId(2), Vec::new());
    let result = ModuleFetchResult {
        key: child_key.clone(),
        client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: ready_outcome(child_key, ModuleEntryId(2)),
    };

    let ModuleScriptTreePoll::Complete(graph) = job.resume_single_module(&mut host, client, result)
    else {
        panic!("joined fetch should complete graph");
    };

    assert_eq!(graph.root_entry, ModuleEntryId(1));
    assert_eq!(graph.entries, vec![ModuleEntryId(1), ModuleEntryId(2)]);
}

#[test]
fn joined_fetch_ready_module_map_entry_advances_from_fanout_completion() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    assert_waiting_for_clients(job.poll(&mut host), 1);
    let client = host.take_single_client(&child_key);

    host.ready(child_key.clone(), ModuleEntryId(2), Vec::new());
    let result = ModuleFetchResult {
        key: child_key.clone(),
        client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: ready_outcome(child_key, ModuleEntryId(2)),
    };

    let ModuleScriptTreePoll::Complete(graph) = job.resume_single_module(&mut host, client, result)
    else {
        panic!("joined module map ready entry should complete graph from fanout completion");
    };

    assert_eq!(graph.root_entry, ModuleEntryId(1));
    assert_eq!(graph.entries, vec![ModuleEntryId(1), ModuleEntryId(2)]);
}

#[test]
fn joined_fetch_fetched_module_map_entry_compiles_from_fanout_completion() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    assert_waiting_for_clients(job.poll(&mut host), 1);
    let client = host.take_single_client(&child_key);
    let result = ModuleFetchResult {
        key: child_key.clone(),
        client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: fetched_outcome(child_key, ModuleSource::Text("export {};".to_owned())),
    };

    let ModuleScriptTreePoll::Complete(graph) = job.resume_single_module(&mut host, client, result)
    else {
        panic!("joined fetched module should compile and complete graph from fanout completion");
    };

    assert_eq!(graph.root_entry, ModuleEntryId(1));
    assert_eq!(graph.entries, vec![ModuleEntryId(1), ModuleEntryId(2)]);
}

#[test]
fn joined_fetch_failed_module_map_entry_fails_from_fanout_completion() {
    let root_key = key("https://example.test/app/root.mjs");
    let child_key = key("https://example.test/app/child.mjs");
    let error = ModuleLoadError::new(ModuleLoadStage::Fetch, "child failed");
    let mut host = FakeHost::new();
    host.ready(
        root_key.clone(),
        ModuleEntryId(1),
        vec![request("./child.mjs", ModuleImportPhase::Evaluation)],
    );
    host.entries.insert(
        child_key.clone(),
        FakeEntry::Fetching {
            clients: Vec::new(),
            phase: ModuleImportPhase::Evaluation,
        },
    );
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));
    assert_waiting_for_clients(job.poll(&mut host), 1);
    let client = host.take_single_client(&child_key);
    let result = ModuleFetchResult {
        key: child_key,
        client,
        requested_phase: ModuleImportPhase::Evaluation,
        outcome: ModuleFetchOutcome::Failed(error.clone()),
    };

    let ModuleScriptTreePoll::Failed(returned) =
        job.resume_single_module(&mut host, client, result)
    else {
        panic!("joined module map failed entry should fail graph from fanout completion");
    };

    assert_eq!(returned, error);
}

#[test]
fn host_link_error_is_propagated_without_tree_specific_error_rewrite() {
    let root_key = key("https://example.test/app/root.mjs");
    let error = ModuleLoadError::new(ModuleLoadStage::Link, "link failed");
    let mut host = FakeHost::new();
    host.fail_link = Some(error.clone());
    host.ready(root_key.clone(), ModuleEntryId(1), Vec::new());
    let mut job = job(inline_root(root_key, ModuleEntryId(1)));

    let ModuleScriptTreePoll::Failed(returned) = job.poll(&mut host) else {
        panic!("link error should fail tree");
    };

    assert_eq!(returned, error);
}

#[test]
fn runtime_module_script_owner_metadata_is_preserved_in_fetch_request() {
    let root_url = url("https://example.test/app/runtime-module.mjs");
    let mut job = ModuleScriptTreeJob::new(
        external_root(root_url, ModuleImportPhase::Evaluation),
        ModuleTreeConfig {
            tree_id: ModuleTreeId(10),
            owner: ModuleTreeOwner::runtime_module_script(),
            ..ModuleTreeConfig::default()
        },
    );
    let mut host = FakeHost::new();

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("runtime module script should fetch root");
    };

    assert_eq!(
        fetches[0].requester,
        moli_module_script_tree::ModuleFetchRequester::RuntimeModuleScript
    );
    assert_eq!(
        fetches[0].ordering,
        moli_module_script_tree::ModuleFetchOrdering::Runtime
    );
    assert_eq!(job.state(), ModuleTreeState::FetchingRoot);
}

#[test]
fn dynamic_import_owner_metadata_is_preserved_in_fetch_request() {
    let root_url = url("https://example.test/app/dynamic.mjs");
    let mut job = ModuleScriptTreeJob::new(
        external_root(root_url, ModuleImportPhase::Evaluation),
        ModuleTreeConfig {
            tree_id: ModuleTreeId(9),
            owner: ModuleTreeOwner::dynamic_import(),
            ..ModuleTreeConfig::default()
        },
    );
    let mut host = FakeHost::new();

    let ModuleScriptTreePoll::NeedFetches(fetches) = job.poll(&mut host) else {
        panic!("dynamic import should fetch root");
    };

    assert_eq!(
        fetches[0].requester,
        moli_module_script_tree::ModuleFetchRequester::DynamicImport
    );
    assert_eq!(
        fetches[0].ordering,
        moli_module_script_tree::ModuleFetchOrdering::Runtime
    );
    assert_eq!(job.state(), ModuleTreeState::FetchingRoot);
}
