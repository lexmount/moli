use std::collections::{HashMap, VecDeque};

use moli_parser::{ParserScriptHandoff, PreparedImportMap, PreparedImportMapSource};

use crate::{
    content_security_policy::{
        ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyScriptElementRequest,
    },
    document_runtime::DomHandle,
    host::EventTargetHandle,
    native_bridge::{OwnerDispatchScope, WindowDocumentTaskTarget},
    page_task_queue::HtmlFragmentScriptTaskKind,
    planning::{
        PreparedScript, PreparedScriptSourceLoadOutcome, ScriptSource,
        load_service_worker_aware_external_script_source_outcome,
        prepared_script_with_loaded_source,
    },
    types::{ScriptKind, ScriptSourceKind},
};

use super::{
    super::inline_script_source_for_execution, FragmentStream, HtmlFragmentModuleTerminal,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum FragmentScriptOrder {
    Blocking,
    Async,
    Deferred,
}

type ClassicSourceResult = Result<Box<PreparedScriptSourceLoadOutcome>, String>;

struct FragmentScriptJob {
    order: FragmentScriptOrder,
    script: PreparedScript,
    source: FragmentScriptSource,
}

enum FragmentScriptSource {
    PendingClassic,
    PendingModule,
    LoadingModule,
    Classic(ClassicSourceResult),
    Module(HtmlFragmentModuleTerminal),
}

#[derive(Default)]
pub(super) struct FragmentScriptJobs {
    next_id: u64,
    jobs: HashMap<u64, FragmentScriptJob>,
    blocking: Option<u64>,
    deferred: VecDeque<u64>,
    deferred_turn_queued: bool,
    module_turn_running: bool,
}

impl FragmentScriptJobs {
    fn insert(&mut self, order: FragmentScriptOrder, script: PreparedScript) -> u64 {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("HTML stream script identities exhausted");
        let id = self.next_id;
        self.jobs.insert(
            id,
            FragmentScriptJob {
                order,
                source: if script.kind == ScriptKind::Module {
                    FragmentScriptSource::PendingModule
                } else {
                    FragmentScriptSource::PendingClassic
                },
                script,
            },
        );
        match order {
            FragmentScriptOrder::Blocking => {
                debug_assert!(self.blocking.is_none());
                self.blocking = Some(id);
            }
            FragmentScriptOrder::Deferred => self.deferred.push_back(id),
            FragmentScriptOrder::Async => {}
        }
        id
    }

    pub(super) fn has_tasks(&self) -> bool {
        self.blocking.is_some() || !self.jobs.is_empty() || self.module_turn_running
    }

    pub(super) fn is_blocked(&self) -> bool {
        self.blocking.is_some()
    }

    pub(super) fn abort_parser(&mut self) {
        self.blocking = None;
        self.deferred.clear();
        self.jobs
            .retain(|_, job| job.order == FragmentScriptOrder::Async);
        self.deferred_turn_queued = false;
    }
}

impl FragmentStream {
    pub(super) fn handle_script_handoff(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        handoff: ParserScriptHandoff,
    ) {
        let (node, line, column) = handoff.start_position();
        crate::host::apply_parser_script_element_state_without_execution(
            unsafe { &mut *self.owner.host }.dom_host_mut(),
            &handoff,
        );
        self.with_script_context(scope, node, |scope, target| {
            let host_ptr = self.owner.host;
            unsafe { &mut *host_ptr }.note_parser_script_start_position(node, line, column);
            match handoff {
                ParserScriptHandoff::BlockingClassic { script, .. } => {
                    if let ScriptSource::Inline(ref source) = script.source {
                        let source = inline_script_source_for_execution(
                            scope,
                            host_ptr,
                            node,
                            source,
                            script_element_request(&script),
                        );
                        let aborted = self.progress.borrow().is_aborted();
                        if let Some(source) = source
                            && !aborted
                        {
                            self.execute_classic(scope, target, &script, &source, line);
                        }
                    } else {
                        self.start_classic_source_load(
                            scope,
                            target,
                            FragmentScriptOrder::Blocking,
                            script,
                        );
                    }
                }
                ParserScriptHandoff::AsyncPostParse { script, .. } => {
                    self.start_script_load(scope, target, FragmentScriptOrder::Async, script);
                }
                ParserScriptHandoff::NonAsyncPostParse { script, .. } => {
                    self.start_script_load(scope, target, FragmentScriptOrder::Deferred, script);
                }
                ParserScriptHandoff::ImportMap { import_map, .. } => {
                    self.register_import_map(scope, target, import_map);
                }
                ParserScriptHandoff::PreparationFailure { .. } => {
                    unsafe { &mut *host_ptr }.queue_script_preparation_error(scope, node);
                }
                ParserScriptHandoff::NoExecution { .. } => {}
            }
        });
    }

    pub(super) fn with_script_context<R>(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        node: DomHandle,
        run: impl FnOnce(&mut v8::PinScope<'_, '_>, WindowDocumentTaskTarget) -> R,
    ) -> Option<R> {
        let target = self.owner.script_target?;
        if !self.script_is_in_preparation_document(target, node) {
            return None;
        }
        let host = unsafe { &mut *self.owner.host };
        let context =
            host.current_html_fragment_script_context(scope, target, self.owner.document)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch = target.dispatch_scope();
        let previous = dispatch.enter(scope);
        let result = run(scope, target);
        dispatch.restore(scope, previous);
        Some(result)
    }

    pub(super) fn script_is_in_preparation_document(
        &self,
        target: WindowDocumentTaskTarget,
        node: DomHandle,
    ) -> bool {
        let host = unsafe { &*self.owner.host };
        !self.progress.borrow().scripts_retired
            && host.current_window_document_task_target_for_dispatch_scope(target.dispatch_scope())
                == Some(target)
            && host.document_scripting_enabled(self.owner.document)
            && host.dom_host().owner_document_handle(node) == Some(self.owner.document)
    }

    fn start_script_load(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        order: FragmentScriptOrder,
        script: PreparedScript,
    ) {
        if script.kind == ScriptKind::Classic {
            self.start_classic_source_load(scope, target, order, script);
            return;
        }
        let host = unsafe { &*self.owner.host };
        let Some(sender) = host.html_fragment_script_task_sender() else {
            return;
        };
        let mut progress = self.progress.borrow_mut();
        let job = progress.scripts.insert(order, script);
        if sender
            .send_html_fragment_script(
                target,
                self.id,
                HtmlFragmentScriptTaskKind::StartModule { job },
            )
            .is_err()
        {
            progress.scripts.jobs.remove(&job);
            progress.scripts.deferred.retain(|id| *id != job);
        }
    }

    fn start_classic_source_load(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        order: FragmentScriptOrder,
        mut script: PreparedScript,
    ) {
        debug_assert_eq!(script.kind, ScriptKind::Classic);
        let host = unsafe { &mut *self.owner.host };
        let Some(sender) = host.html_fragment_script_task_sender().cloned() else {
            return;
        };
        script.document_referrer_policy =
            crate::context_bootstrap::document_referrer_policy_for_native_document(
                host,
                self.owner.document,
            );
        let character_set = host
            .document_character_set_for_handle(self.owner.document)
            .map(str::to_owned);
        let load = if host.allows_streamed_external_script_by_csp(
            scope,
            target.dispatch_scope(),
            &script,
            &script.url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        ) {
            host.document_resource_loader_for_window_owner(target.owner())
                .ok_or("The HTML stream no longer has a Document resource loader.")
        } else {
            Err("The Document Content Security Policy blocked the streamed script.")
        };
        let job = self
            .progress
            .borrow_mut()
            .scripts
            .insert(order, script.clone());
        let stream = self.id;
        let post = move |outcome| {
            sender.send_html_fragment_script(
                target,
                stream,
                HtmlFragmentScriptTaskKind::SourceReady { job, outcome },
            )
        };
        match load {
            Ok(loader) => {
                let origin = loader.fetch_context().request_origin();
                let client = loader.request_client().clone();
                let runner = loader.task_runner();
                let browsing_context = host.browser_context_runtime();
                let client_id =
                    host.service_worker_client_id_for_subresource_owner(target.dispatch_scope());
                let document_url = host.document_url_for_handle(self.owner.document);
                loader.spawn_resource_task(async move {
                    let outcome = load_service_worker_aware_external_script_source_outcome(
                        &script,
                        &origin,
                        &client,
                        runner,
                        character_set.as_deref(),
                        None,
                        browsing_context,
                        client_id,
                        document_url,
                        None,
                    )
                    .await;
                    let _ = post(Ok(Box::new(outcome)));
                });
            }
            Err(error) => {
                if post(Err(error.to_owned())).is_err() {
                    self.progress.borrow_mut().scripts.abort_parser();
                }
            }
        }
    }

    pub(super) fn apply_script_task(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        kind: HtmlFragmentScriptTaskKind,
    ) -> bool {
        match kind {
            HtmlFragmentScriptTaskKind::SourceReady { job, outcome } => {
                self.apply_loaded_classic(scope, job, outcome)
            }
            HtmlFragmentScriptTaskKind::RunDeferred => self.run_next_deferred_classic(scope),
            HtmlFragmentScriptTaskKind::Retire => self.retire_script_tasks(scope),
            HtmlFragmentScriptTaskKind::StartModule { .. }
            | HtmlFragmentScriptTaskKind::RunModule { .. } => {
                unreachable!("module tasks need the Document module owner")
            }
        }
    }

    fn apply_loaded_classic(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        id: u64,
        outcome: ClassicSourceResult,
    ) -> bool {
        let deferred = {
            let mut progress = self.progress.borrow_mut();
            let Some(job) = progress.scripts.jobs.get_mut(&id) else {
                return false;
            };
            if !matches!(job.source, FragmentScriptSource::PendingClassic) {
                return false;
            }
            job.source = FragmentScriptSource::Classic(outcome);
            job.order == FragmentScriptOrder::Deferred
        };
        if deferred {
            self.queue_next_deferred_script();
            return true;
        }
        let job = self
            .progress
            .borrow_mut()
            .scripts
            .jobs
            .remove(&id)
            .expect("ready classic script still belongs to its stream");
        let blocked = job.order == FragmentScriptOrder::Blocking;
        self.run_loaded_classic(scope, job);
        if blocked {
            self.progress.borrow_mut().scripts.blocking = None;
            let _ = self.run_fragment_parser(scope);
        }
        true
    }

    pub(super) fn queue_next_deferred_script(&self) {
        let Some(target) = self.owner.script_target else {
            return;
        };
        let host = unsafe { &*self.owner.host };
        let Some(sender) = host.html_fragment_script_task_sender() else {
            return;
        };
        let mut progress = self.progress.borrow_mut();
        if !progress.is_closed()
            || progress.scripts.deferred_turn_queued
            || progress.scripts.module_turn_running
        {
            return;
        }
        let Some(&id) = progress.scripts.deferred.front() else {
            return;
        };
        let kind = match progress.scripts.jobs.get(&id).map(|job| &job.source) {
            Some(FragmentScriptSource::Classic(_)) => HtmlFragmentScriptTaskKind::RunDeferred,
            Some(FragmentScriptSource::Module(_)) => {
                HtmlFragmentScriptTaskKind::RunModule { job: id }
            }
            _ => return,
        };
        progress.scripts.deferred_turn_queued = sender
            .send_html_fragment_script(target, self.id, kind)
            .is_ok();
    }

    fn run_next_deferred_classic(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let job = {
            let mut progress = self.progress.borrow_mut();
            if !progress.is_closed() || !progress.scripts.deferred_turn_queued {
                return false;
            }
            let Some(&id) = progress.scripts.deferred.front() else {
                return false;
            };
            if !progress
                .scripts
                .jobs
                .get(&id)
                .is_some_and(|job| matches!(job.source, FragmentScriptSource::Classic(_)))
            {
                return false;
            }
            progress.scripts.deferred_turn_queued = false;
            progress.scripts.deferred.pop_front();
            progress
                .scripts
                .jobs
                .remove(&id)
                .expect("queued deferred script has a job")
        };
        self.run_loaded_classic(scope, job);
        self.queue_next_deferred_script();
        true
    }

    pub(super) fn take_module_start(&self, id: u64) -> Option<PreparedScript> {
        let mut progress = self.progress.borrow_mut();
        let job = progress.scripts.jobs.get_mut(&id)?;
        if !matches!(job.source, FragmentScriptSource::PendingModule) {
            return None;
        }
        job.source = FragmentScriptSource::LoadingModule;
        Some(job.script.clone())
    }

    pub(super) fn accept_module_terminal(
        &self,
        target: WindowDocumentTaskTarget,
        id: u64,
        terminal: HtmlFragmentModuleTerminal,
    ) -> bool {
        let order = {
            let mut progress = self.progress.borrow_mut();
            let Some(job) = progress.scripts.jobs.get_mut(&id) else {
                return false;
            };
            if !matches!(job.source, FragmentScriptSource::LoadingModule) {
                return false;
            }
            job.source = FragmentScriptSource::Module(terminal);
            job.order
        };
        if order == FragmentScriptOrder::Deferred {
            self.queue_next_deferred_script();
        } else if let Some(sender) = unsafe { &*self.owner.host }.html_fragment_script_task_sender()
        {
            let result = sender.send_html_fragment_script(
                target,
                self.id,
                HtmlFragmentScriptTaskKind::RunModule { job: id },
            );
            if result.is_err() {
                self.progress.borrow_mut().scripts.jobs.remove(&id);
                return false;
            }
        }
        true
    }

    pub(super) fn take_ready_module(
        &self,
        target: WindowDocumentTaskTarget,
        id: u64,
    ) -> Option<HtmlFragmentModuleTerminal> {
        let job = {
            let mut progress = self.progress.borrow_mut();
            let job = progress.scripts.jobs.get(&id)?;
            if !matches!(job.source, FragmentScriptSource::Module(_)) {
                return None;
            }
            if job.order == FragmentScriptOrder::Deferred {
                if !progress.is_closed()
                    || !progress.scripts.deferred_turn_queued
                    || progress.scripts.deferred.front() != Some(&id)
                {
                    return None;
                }
                progress.scripts.deferred_turn_queued = false;
                progress.scripts.deferred.pop_front();
            }
            progress
                .scripts
                .jobs
                .remove(&id)
                .expect("queued streamed module has a job")
        };
        if !self.script_is_in_preparation_document(target, job.script.node_id) {
            self.queue_next_deferred_script();
            return None;
        }
        let FragmentScriptSource::Module(terminal) = job.source else {
            unreachable!("selected module job has a module terminal");
        };
        self.progress.borrow_mut().scripts.module_turn_running = true;
        Some(terminal)
    }

    pub(super) fn finish_module_turn(&self) {
        self.progress.borrow_mut().scripts.module_turn_running = false;
        self.queue_next_deferred_script();
    }

    fn run_loaded_classic(&self, scope: &mut v8::PinScope<'_, '_>, job: FragmentScriptJob) {
        let node = job.script.node_id;
        self.with_script_context(scope, node, |scope, target| {
            let script = job.script;
            let FragmentScriptSource::Classic(outcome) = job.source else {
                unreachable!("selected classic job has a source result");
            };
            let loaded = outcome.and_then(|outcome| {
                let host = unsafe { &mut *self.owner.host };
                if let Some(Ok(response)) = outcome
                    .network_result
                    .as_deref()
                    .map(|result| result.as_ref())
                    && response.redirected
                    && !host.allows_streamed_external_script_by_csp(
                        scope,
                        target.dispatch_scope(),
                        &script,
                        &response.final_url,
                        ContentSecurityPolicyRedirectStatus::FollowedRedirect,
                    )
                {
                    return Err(
                        "Content Security Policy blocked the redirected streamed script."
                            .to_owned(),
                    );
                }
                let source = outcome.source_result?;
                Ok(prepared_script_with_loaded_source(
                    script,
                    source,
                    outcome.source_bytes,
                    outcome.muted_errors,
                    outcome.fetch_timing,
                ))
            });
            let event = match loaded {
                Ok(script) => {
                    let ScriptSource::Loaded(source) = &script.source else {
                        unreachable!("a loaded classic script has text source");
                    };
                    self.execute_classic(scope, target, &script, source, 1);
                    "load"
                }
                Err(error) => {
                    tracing::debug!(%error, "streamed classic script failed to load");
                    "error"
                }
            };
            if self.script_is_in_preparation_document(target, node) {
                self.dispatch_script_event(scope, node, event);
            }
        });
    }

    fn execute_classic(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        script: &PreparedScript,
        source: &str,
        line: u64,
    ) {
        let node = script.node_id;
        if !self.script_is_in_preparation_document(target, node) {
            return;
        }
        let host = unsafe { &mut *self.owner.host };
        let dispatch = target.dispatch_scope();
        let _external_write_guard = (script.source_kind == ScriptSourceKind::External)
            .then(|| host.enter_ignore_destructive_writes(self.owner.document));
        let child_current_script = match dispatch {
            OwnerDispatchScope::Top => {
                host.push_current_inline_script(node);
                None
            }
            OwnerDispatchScope::Child(child) => {
                host.push_streamed_child_current_script(child, node)
            }
            OwnerDispatchScope::LightweightPopup(_) => None,
        };
        let line_offset = i32::try_from(line.saturating_sub(1)).unwrap_or(i32::MAX);
        let result = crate::script_vm::execute_source_text_on_current_stack(
            scope,
            source,
            Some(&script.url),
            Some(&script.base_url),
            line_offset,
            Some(&script.fetch_metadata),
            true,
        );
        let host = unsafe { &mut *self.owner.host };
        if dispatch == OwnerDispatchScope::Top {
            host.pop_current_inline_script(node);
        } else if let Some(token) = child_current_script {
            host.pop_child_current_script(token);
        }
        if let Err(error) = result {
            tracing::debug!(%error, "streamed classic script execution failed");
        }
    }

    pub(super) fn dispatch_script_event(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        node: DomHandle,
        event: &str,
    ) {
        let host_ptr = self.owner.host;
        let host = unsafe { &mut *host_ptr };
        let target = EventTargetHandle::Node(node);
        let event = host.prepare_native_event_dispatch(target, target, event);
        if let Err(error) = host.dispatch_native_event_in_scope(scope, host_ptr, event) {
            tracing::debug!(%error, "streamed script event dispatch failed");
        }
    }

    fn register_import_map(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        target: WindowDocumentTaskTarget,
        import_map: PreparedImportMap,
    ) {
        let node = import_map.node_id;
        let host_ptr = self.owner.host;
        let PreparedImportMapSource::Inline(source) = import_map.source else {
            unsafe { &mut *host_ptr }.queue_script_preparation_error(scope, node);
            return;
        };
        let Some(source) = inline_script_source_for_execution(
            scope,
            host_ptr,
            node,
            &source,
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(
                import_map.nonce.as_deref(),
            ),
        ) else {
            return;
        };
        if !self.script_is_in_preparation_document(target, node) {
            return;
        }
        let host = unsafe { &mut *host_ptr };
        let result = match target.dispatch_scope() {
            OwnerDispatchScope::Top => {
                host.register_import_map_source_with_base_url(&source, &import_map.base_url)
            }
            OwnerDispatchScope::Child(child) => host.register_current_child_document_import_map(
                child,
                self.owner.document,
                &source,
                &import_map.base_url,
            ),
            OwnerDispatchScope::LightweightPopup(_) => return,
        };
        if let Err(error) = result {
            let _ = crate::context_bootstrap::dispatch_window_report_error_message(
                scope,
                host_ptr,
                &error,
                Some(import_map.initiator_url.as_str()),
            );
        }
    }

    pub(super) fn retire_script_tasks(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        {
            let mut progress = self.progress.borrow_mut();
            progress.scripts = FragmentScriptJobs::default();
            progress.scripts_retired = true;
        }
        let Some(context) = self
            .writable
            .to_local(scope)
            .and_then(|stream| stream.get_creation_context(scope))
        else {
            self.abort_parser(scope, None);
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let _ = self.run_fragment_parser(scope);
        true
    }
}

fn script_element_request(
    script: &PreparedScript,
) -> ContentSecurityPolicyScriptElementRequest<'_> {
    ContentSecurityPolicyScriptElementRequest {
        nonce: script.fetch_metadata.nonce.as_deref(),
        integrity: script.fetch_metadata.integrity.as_deref(),
        parser_inserted: script.fetch_metadata.parser_inserted,
    }
}
