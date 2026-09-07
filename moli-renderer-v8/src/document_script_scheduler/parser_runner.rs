use crate::{
    frame_owner_model::DocumentLoadDelayTokenId,
    planning::{PreparedScript, PreparedScriptSourceLoadOutcome, SharedScriptSourceLoad},
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
    types::{ScriptKind, ScriptMode},
};
use std::collections::{HashSet, VecDeque};

use super::{
    module_ready::{
        ParserModuleGraphTerminalWork, ParserModulePendingScriptWatchResult,
        ParserModuleScriptRunner, ParserOrderedModuleTerminalState, ParserPendingScriptId,
        ParserPendingScriptKey,
    },
    post_parse::{
        ParserDeferredClassicReady, ResolvedDeferPhaseScript,
        should_directly_own_external_defer_like_source,
    },
};

#[derive(Debug)]
pub(crate) enum ParserDeferredScriptReady<Target, ParserModuleGraphFailure> {
    Classic(ParserDeferredClassicReady),
    Module(ParserDeferredModuleReady<Target, ParserModuleGraphFailure>),
}

#[derive(Debug)]
pub(crate) struct ParserDeferredModuleReady<Target, ParserModuleGraphFailure> {
    terminal: ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
    load_delay_token: DocumentLoadDelayTokenId,
}

impl<Target, ParserModuleGraphFailure> ParserDeferredModuleReady<Target, ParserModuleGraphFailure> {
    fn new(
        terminal: ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        Self {
            terminal,
            load_delay_token,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
        DocumentLoadDelayTokenId,
    ) {
        (self.terminal, self.load_delay_token)
    }
}

#[derive(Debug)]
pub(crate) struct ParserDeferredClassicSourceLoad<Owner> {
    pending_script_id: ParserPendingScriptId<Owner>,
    source_load: SharedScriptSourceLoad,
}

pub(crate) struct ParserDeferredClassicSourceLoadRequest<Owner> {
    pending_script_id: ParserPendingScriptId<Owner>,
    script: PreparedScript,
    shared_load: Option<SharedScriptSourceLoad>,
    document_character_set: Option<String>,
}

impl<Owner: Copy> ParserDeferredClassicSourceLoadRequest<Owner> {
    fn new(
        owner: Owner,
        key: ParserPendingScriptKey,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<String>,
    ) -> Self {
        Self {
            pending_script_id: ParserPendingScriptId::from_key(owner, key),
            script,
            shared_load,
            document_character_set,
        }
    }

    pub(crate) fn start(
        self,
        loader: &crate::network::ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
    ) -> ParserDeferredClassicSourceLoad<Owner> {
        let source_load = self.shared_load.unwrap_or_else(|| {
            SharedScriptSourceLoad::spawn_with_request_resource_type(
                self.script,
                loader.clone(),
                task_runner,
                self.document_character_set,
                None,
            )
        });
        ParserDeferredClassicSourceLoad {
            pending_script_id: self.pending_script_id,
            source_load,
        }
    }

    pub(crate) fn network_attribution_urls(&self) -> (url::Url, url::Url) {
        (self.script.initiator_url.clone(), self.script.url.clone())
    }

    #[cfg(test)]
    pub(crate) fn start_with_injected_source_load_for_test(
        self,
    ) -> ParserDeferredClassicSourceLoad<Owner> {
        ParserDeferredClassicSourceLoad {
            pending_script_id: self.pending_script_id,
            source_load: self
                .shared_load
                .expect("source-loading unit test must inject its SharedScriptSourceLoad"),
        }
    }

    pub(crate) fn into_failure_completion(
        self,
        message: impl Into<String>,
    ) -> ParserDeferredClassicSourceLoadCompletion<Owner> {
        ParserDeferredClassicSourceLoadCompletion::new(
            self.pending_script_id,
            PreparedScriptSourceLoadOutcome {
                source_result: Err(message.into()),
                source_bytes: None,
                network_result: None,
                muted_errors: false,
            },
        )
    }
}

pub(crate) struct ParserDeferredModuleGraphStart<Owner> {
    pending_script_id: ParserPendingScriptId<Owner>,
    script: PreparedScript,
}

impl<Owner: Copy> ParserDeferredModuleGraphStart<Owner> {
    pub(super) fn new(
        pending_script_id: ParserPendingScriptId<Owner>,
        script: PreparedScript,
    ) -> Self {
        Self {
            pending_script_id,
            script,
        }
    }

    pub(crate) fn into_parts(self) -> (ParserPendingScriptId<Owner>, PreparedScript) {
        (self.pending_script_id, self.script)
    }
}

pub(crate) enum ParserDeferredScriptStartAction<Owner> {
    NoFetch,
    ClassicSource(ParserDeferredClassicSourceLoadRequest<Owner>),
    ModuleGraph(ParserDeferredModuleGraphStart<Owner>),
}

impl<Owner: Copy> ParserDeferredClassicSourceLoad<Owner> {
    pub(crate) fn into_parts(self) -> (ParserPendingScriptId<Owner>, SharedScriptSourceLoad) {
        (self.pending_script_id, self.source_load)
    }
}

#[derive(Debug)]
pub(crate) struct ParserDeferredClassicSourceLoadCompletion<Owner> {
    pending_script_id: ParserPendingScriptId<Owner>,
    outcome: PreparedScriptSourceLoadOutcome,
}

impl<Owner: Copy> ParserDeferredClassicSourceLoadCompletion<Owner> {
    pub(crate) fn new(
        pending_script_id: ParserPendingScriptId<Owner>,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Self {
        Self {
            pending_script_id,
            outcome,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ParserPendingScriptId<Owner>,
        PreparedScriptSourceLoadOutcome,
    ) {
        (self.pending_script_id, self.outcome)
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<Owner> {
        self.pending_script_id
    }

    pub(crate) fn network_result(&self) -> Option<&crate::types::SharedNavigationResponseResult> {
        self.outcome.network_result.as_ref()
    }
}

pub(super) struct ParserDeferredClassicSourceLoadStart {
    key: ParserPendingScriptKey,
    script: PreparedScript,
    shared_load: Option<SharedScriptSourceLoad>,
    document_character_set: Option<String>,
}

pub(super) struct ParserDeferredScriptClaim {
    key: ParserPendingScriptKey,
    classic_source_load: Option<ParserDeferredClassicSourceLoadStart>,
}

impl ParserDeferredScriptClaim {
    fn new(
        key: ParserPendingScriptKey,
        classic_source_load: Option<ParserDeferredClassicSourceLoadStart>,
    ) -> Self {
        Self {
            key,
            classic_source_load,
        }
    }

    pub(super) fn key(&self) -> ParserPendingScriptKey {
        self.key
    }

    pub(super) fn into_classic_source_load(self) -> Option<ParserDeferredClassicSourceLoadStart> {
        self.classic_source_load
    }
}

impl ParserDeferredClassicSourceLoadStart {
    fn new(
        key: ParserPendingScriptKey,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<String>,
    ) -> Self {
        Self {
            key,
            script,
            shared_load,
            document_character_set,
        }
    }

    pub(super) fn bind_owner<Owner: Copy>(
        self,
        owner: Owner,
    ) -> ParserDeferredClassicSourceLoadRequest<Owner> {
        ParserDeferredClassicSourceLoadRequest::new(
            owner,
            self.key,
            self.script,
            self.shared_load,
            self.document_character_set,
        )
    }

    #[cfg(test)]
    pub(super) fn start_with_injected_source_load_for_test(
        self,
    ) -> (ParserPendingScriptKey, SharedScriptSourceLoad) {
        (
            self.key,
            self.shared_load
                .expect("source-loading unit test must inject its SharedScriptSourceLoad"),
        )
    }
}

/// Parser-owned script state shared by parser-connected defer/module-defer work.
///
/// The broad document runner never owns parser ordering or PendingScript state;
/// it only receives clean ready work after this runner releases it.
pub(super) struct ParserScriptRunner<Target, ParserModuleGraphFailure> {
    module_scripts: ParserModuleScriptRunner<Target, ParserModuleGraphFailure>,
    deferred_scripts: VecDeque<ResolvedDeferPhaseScript>,
    defer_phase_sealed: bool,
    after_parsing_module_keys: HashSet<ParserPendingScriptKey>,
}

impl<Target, ParserModuleGraphFailure> Default
    for ParserScriptRunner<Target, ParserModuleGraphFailure>
{
    fn default() -> Self {
        Self {
            module_scripts: ParserModuleScriptRunner::default(),
            deferred_scripts: VecDeque::new(),
            defer_phase_sealed: false,
            after_parsing_module_keys: HashSet::new(),
        }
    }
}

impl<Target, ParserModuleGraphFailure> ParserScriptRunner<Target, ParserModuleGraphFailure> {
    pub(super) fn register_module_script(
        &mut self,
        script: &PreparedScript,
    ) -> ParserPendingScriptKey {
        self.module_scripts.register(script)
    }

    pub(super) fn watch_module_script(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> ParserModulePendingScriptWatchResult<Target, ParserModuleGraphFailure> {
        self.module_scripts.watch(key)
    }

    pub(super) fn accept_parser_ordered_module_script(
        &mut self,
        script: &PreparedScript,
        blocking_stylesheet_signatures: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> Option<ParserPendingScriptKey> {
        self.module_scripts
            .accept_parser_ordered(script, blocking_stylesheet_signatures)
    }

    pub(super) fn module_script_blocking_stylesheet_signatures(
        &self,
        key: ParserPendingScriptKey,
    ) -> Option<&HashSet<DocumentBlockingStylesheetSignature>> {
        self.module_scripts.blocking_stylesheet_signatures(key)
    }

    pub(super) fn notify_module_tree_load_finished(
        &mut self,
        key: ParserPendingScriptKey,
        work: super::ModuleScriptGraphReadyWork<Target>,
    ) -> Option<Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>> {
        if self.module_scripts.is_retained_by_parser_order(key) {
            tracing::debug!(
                parser_position = key.parser_position(),
                script_node_id = ?key.script_node_id(),
                terminal = "graph-ready",
                "recording module terminal on parser-ordered PendingScript"
            );
        }
        self.module_scripts
            .notify_module_tree_load_finished(key, work)
    }

    pub(super) fn notify_module_tree_load_failed(
        &mut self,
        key: ParserPendingScriptKey,
        failure: ParserModuleGraphFailure,
    ) -> Option<Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>> {
        if self.module_scripts.is_retained_by_parser_order(key) {
            tracing::debug!(
                parser_position = key.parser_position(),
                script_node_id = ?key.script_node_id(),
                terminal = "graph-failed",
                "recording module terminal on parser-ordered PendingScript"
            );
        }
        self.module_scripts
            .notify_module_tree_load_failed(key, failure)
    }

    pub(super) fn claim_defer_script(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Option<ParserDeferredScriptClaim> {
        match script.mode {
            ScriptMode::Normal => {
                tracing::debug!(
                    url = %script.url,
                    "ignoring parser-owned normal script in deferred parser runner; parser-blocking runner owns this lane"
                );
                None
            }
            ScriptMode::Defer | ScriptMode::ModuleDefer => {
                let key = ParserPendingScriptKey::from_script(&script);
                if self
                    .deferred_scripts
                    .iter()
                    .any(|pending| ParserPendingScriptKey::from_script(pending) == key)
                {
                    tracing::debug!(
                        parser_position = key.parser_position(),
                        script_node_id = ?key.script_node_id(),
                        "ignoring duplicate parser-deferred PendingScript acceptance"
                    );
                    return None;
                }
                if script.kind == ScriptKind::Module {
                    let Some(module_key) = self.accept_parser_ordered_module_script(
                        &script,
                        blocking_signatures_before.clone(),
                    ) else {
                        tracing::warn!(
                            parser_position = key.parser_position(),
                            script_node_id = ?key.script_node_id(),
                            "rejecting parser-deferred module without atomic PendingScript ownership"
                        );
                        return None;
                    };
                    debug_assert_eq!(module_key, key);
                    self.after_parsing_module_keys.insert(key);
                }
                let needs_source_load = should_directly_own_external_defer_like_source(&script);
                let source_load_script = needs_source_load.then(|| script.clone());
                let pending = ResolvedDeferPhaseScript::from_script(
                    script,
                    blocking_signatures_before,
                    load_delay_token,
                );
                let insert_at = self
                    .deferred_scripts
                    .iter()
                    .position(|existing| existing.position > pending.position)
                    .unwrap_or(self.deferred_scripts.len());
                self.deferred_scripts.insert(insert_at, pending);
                tracing::debug!(
                    parser_position = key.parser_position(),
                    script_node_id = ?key.script_node_id(),
                    pending_count = self.deferred_scripts.len(),
                    source_pending = needs_source_load,
                    parser_finished = self.defer_phase_sealed,
                    "accepted parser-deferred PendingScript before source work"
                );
                let classic_source_load = needs_source_load.then(|| {
                    ParserDeferredClassicSourceLoadStart::new(
                        key,
                        source_load_script
                            .expect("source-owning classic defer requires its prepared script"),
                        shared_load,
                        document_character_set.map(str::to_owned),
                    )
                });
                Some(ParserDeferredScriptClaim::new(key, classic_source_load))
            }
            ScriptMode::Async => {
                unreachable!("async parser handoff must use the document async claim path")
            }
            ScriptMode::InOrder | ScriptMode::ImportMapInOrder | ScriptMode::ModuleInOrder => {
                unreachable!("parser-discovered scripts should not classify as dynamic in-order")
            }
        }
    }

    pub(super) fn seal_defer_phase(&mut self) -> Result<usize, ParserPendingScriptKey> {
        if let Some(missing) = self
            .deferred_scripts
            .iter()
            .filter(|script| script.kind == ScriptKind::Module)
            .map(|script| ParserPendingScriptKey::from_script(script))
            .find(|key| {
                !self.after_parsing_module_keys.contains(key)
                    || !self.module_scripts.contains(*key)
                    || !self.module_scripts.is_retained_by_parser_order(*key)
            })
        {
            tracing::debug!(
                parser_position = missing.parser_position(),
                script_node_id = ?missing.script_node_id(),
                pending_count = self.deferred_scripts.len(),
                "rejecting parser EOF seal without preparation-time module PendingScript"
            );
            return Err(missing);
        }
        self.defer_phase_sealed = true;
        let head_module_state = self.prepare_next_after_parsing_module_terminal();
        tracing::debug!(
            total_queued = self.deferred_scripts.len(),
            module_count = self.after_parsing_module_keys.len(),
            pending_classic_sources = self
                .deferred_scripts
                .iter()
                .filter(|script| {
                    script.kind == ScriptKind::Classic && !script.classic_source_is_terminal()
                })
                .count(),
            ?head_module_state,
            "sealed parser-deferred document-order queue at EOF"
        );
        Ok(self.deferred_scripts.len())
    }

    pub(super) fn complete_classic_source_load(
        &mut self,
        key: ParserPendingScriptKey,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> bool {
        let Some(script) = self
            .deferred_scripts
            .iter_mut()
            .find(|script| ParserPendingScriptKey::from_script(script) == key)
        else {
            tracing::warn!(
                parser_position = key.parser_position(),
                script_node_id = ?key.script_node_id(),
                "parser-deferred classic source completion lost its PendingScript"
            );
            return false;
        };
        let source_loaded = outcome.source_result.is_ok();
        if !script.apply_classic_source_load_outcome(outcome) {
            return false;
        }
        tracing::debug!(
            parser_position = key.parser_position(),
            script_node_id = ?key.script_node_id(),
            source_loaded,
            parser_finished = self.defer_phase_sealed,
            "applied parser-deferred classic source terminal to PendingScript"
        );
        true
    }

    pub(super) fn has_after_parsing_script(&self) -> bool {
        self.defer_phase_sealed && !self.deferred_scripts.is_empty()
    }

    pub(super) fn next_after_parsing_blocking_signatures(
        &self,
    ) -> Option<&HashSet<DocumentBlockingStylesheetSignature>> {
        self.defer_phase_sealed.then_some(())?;
        self.deferred_scripts
            .front()
            .map(ResolvedDeferPhaseScript::blocking_signatures_before)
    }

    pub(super) fn next_after_parsing_script_is_ready(&self) -> bool {
        if !self.defer_phase_sealed {
            return false;
        }
        let Some(script) = self.deferred_scripts.front() else {
            return false;
        };
        let key = ParserPendingScriptKey::from_script(script);
        if script.kind == ScriptKind::Classic && !script.classic_source_is_terminal() {
            return false;
        }
        if script.kind != ScriptKind::Module {
            return true;
        }
        let owned = self.after_parsing_module_keys.contains(&key);
        debug_assert!(
            owned,
            "queued parser-deferred module must retain its PendingScript ownership key"
        );
        owned && self.module_scripts.has_terminal(key)
    }

    pub(super) fn take_next_after_parsing_ready_script(
        &mut self,
    ) -> Option<ParserDeferredScriptReady<Target, ParserModuleGraphFailure>> {
        if !self.next_after_parsing_script_is_ready() {
            return None;
        }
        let script = self.deferred_scripts.pop_front()?;
        tracing::debug!(
            position = script.position,
            node_id = ?script.node_id,
            kind = ?script.kind,
            mode = ?script.mode,
            url = %script.url,
            remaining = self.deferred_scripts.len(),
            "releasing parser after-parsing document-order slot"
        );
        let ready = if script.kind != ScriptKind::Module {
            script
                .into_classic_ready()
                .map(ParserDeferredScriptReady::Classic)
        } else {
            let key = ParserPendingScriptKey::from_script(&script);
            let load_delay_token = script.load_delay_token();
            let owned = self.after_parsing_module_keys.remove(&key);
            debug_assert!(
                owned,
                "installed parser-deferred module must retain its PendingScript identity until release"
            );
            if owned {
                self.module_scripts
                    .take_parser_ordered_ready_terminal(key)
                    .map(|terminal| {
                        ParserDeferredScriptReady::Module(ParserDeferredModuleReady::new(
                            terminal,
                            load_delay_token,
                        ))
                    })
            } else {
                None
            }
        };
        let next_module_state = self.prepare_next_after_parsing_module_terminal();
        tracing::debug!(
            ?next_module_state,
            "prepared the next parser-deferred module head after ordered release"
        );
        ready
    }

    fn prepare_next_after_parsing_module_terminal(
        &mut self,
    ) -> Option<ParserOrderedModuleTerminalState> {
        if !self.defer_phase_sealed {
            return None;
        }
        let script = self.deferred_scripts.front()?;
        if script.kind != ScriptKind::Module {
            return None;
        }
        let key = ParserPendingScriptKey::from_script(script);
        Some(self.module_scripts.prepare_parser_ordered_terminal(key))
    }

    #[cfg(test)]
    pub(super) fn parser_ordered_module_terminal_is_ready(
        &self,
        key: ParserPendingScriptKey,
    ) -> bool {
        self.module_scripts.is_retained_by_parser_order(key)
            && self.module_scripts.has_terminal(key)
    }

    pub(super) fn prepare_parser_ordered_module_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> ParserOrderedModuleTerminalState {
        self.module_scripts.prepare_parser_ordered_terminal(key)
    }

    pub(super) fn take_parser_ordered_module_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> Option<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>> {
        self.module_scripts.take_parser_ordered_ready_terminal(key)
    }

    #[cfg(test)]
    pub(super) fn has_lifecycle_blocking_pending_script(&self) -> bool {
        !self.deferred_scripts.is_empty()
            || self.module_scripts.has_lifecycle_blocking_pending_script()
    }

    pub(super) fn has_module_script(&self, key: ParserPendingScriptKey) -> bool {
        self.module_scripts.contains(key)
    }

    #[cfg(test)]
    pub(super) fn module_script_is_watching_for_test(&self, key: ParserPendingScriptKey) -> bool {
        self.module_scripts.is_watching_for_test(key)
    }

    pub(super) fn discard_module_script(&mut self, key: ParserPendingScriptKey) -> bool {
        self.after_parsing_module_keys.remove(&key);
        self.module_scripts.discard(key)
    }

    pub(super) fn cancel_deferred_script(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> Option<DocumentLoadDelayTokenId> {
        let index = self
            .deferred_scripts
            .iter()
            .position(|pending| ParserPendingScriptKey::from_script(pending) == key)?;
        let pending = self.deferred_scripts.remove(index)?;
        self.after_parsing_module_keys.remove(&key);
        if pending.kind == ScriptKind::Module {
            self.module_scripts.discard(key);
        }
        tracing::debug!(
            parser_position = key.parser_position(),
            script_node_id = ?key.script_node_id(),
            remaining = self.deferred_scripts.len(),
            "canceled parser-deferred PendingScript before asynchronous start"
        );
        Some(pending.load_delay_token())
    }

    pub(super) fn defer_script_count(&self) -> usize {
        self.deferred_scripts.len()
    }

    #[cfg(test)]
    pub(super) fn pending_module_script_count(&self) -> usize {
        self.module_scripts.pending_count()
    }

    #[cfg(test)]
    pub(super) fn after_parsing_scripts(&self) -> &VecDeque<ResolvedDeferPhaseScript> {
        &self.deferred_scripts
    }
}
