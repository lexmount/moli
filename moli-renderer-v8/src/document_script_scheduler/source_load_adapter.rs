use crate::{
    frame_owner_model::DocumentLoadDelayTokenId,
    network::context::DocumentResourceLoader,
    page_task_queue::RendererOwnerWakeSender,
    planning::{PreparedScript, SharedScriptSourceLoad},
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
};
use std::collections::HashSet;

use super::{
    DocumentScriptScheduler, parser_runner::ParserDeferredScriptClaim,
    source_load_port::DocumentScriptSourceLoadPort,
};

pub(super) fn document_script_source_load_port(
    loader: &DocumentResourceLoader,
    owner_wake: Option<RendererOwnerWakeSender>,
) -> DocumentScriptSourceLoadPort {
    let loader = loader.clone();
    DocumentScriptSourceLoadPort::new(move |script, document_character_set| {
        SharedScriptSourceLoad::spawn(
            script,
            loader.clone(),
            document_character_set,
            None,
            crate::types::SubresourceRequestInitiatorType::Parser,
            None,
            owner_wake.clone(),
        )
    })
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptScheduler<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    /// Claim a parser-discovered script that is *not* being executed immediately
    /// at the current parser hand-off point.
    ///
    /// Scripts are classified into the queue matching their HTML-standard category:
    /// - `Normal` -> parser-blocking owner work, executed by the parser runner
    /// - `Defer` / `ModuleDefer` -> defer-like phase (post-parse, pre-DCL)
    /// - `Async` -> parse-time async queue if eligible, otherwise fallback
    ///
    /// Queue ownership is decided here and never changes later. Completion only
    /// advances state within the already-chosen queue; it does not need to
    /// rediscover which semantic lane the script belongs to.
    #[cfg(test)]
    pub(super) fn claim_parser_non_async_post_parse_script(&mut self, script: PreparedScript) {
        self.claim_parser_non_async_post_parse_script_with_shared_load(script, None);
    }

    #[cfg(test)]
    pub(super) fn claim_parser_non_async_post_parse_script_with_shared_load(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
    ) {
        let load_delay_token = DocumentLoadDelayTokenId(script.position as u64 + 1);
        let claim = self
            .claim_parser_non_async_post_parse_script_with_shared_load_and_document_character_set(
                script,
                shared_load,
                None,
                HashSet::new(),
                load_delay_token,
            );
        if let Some(source_load) =
            claim.and_then(ParserDeferredScriptClaim::into_classic_source_load)
        {
            let (key, source_load) = source_load.start_with_injected_source_load_for_test();
            if let Some(outcome) = source_load.try_outcome() {
                assert!(
                    self.complete_parser_deferred_classic_source_load(key, outcome),
                    "test source terminal must update its accepted PendingScript"
                );
            }
        }
    }

    pub(super) fn claim_parser_non_async_post_parse_script_with_shared_load_and_document_character_set(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Option<ParserDeferredScriptClaim> {
        self.parser_runner.claim_defer_script(
            script,
            shared_load,
            document_character_set,
            blocking_signatures_before,
            load_delay_token,
        )
    }

    #[cfg(test)]
    pub(crate) fn on_parser_discovered_async_candidate_with_shared_load(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
    ) -> bool {
        self.on_parser_discovered_async_candidate_with_shared_load_and_document_character_set(
            script,
            shared_load,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn on_parser_discovered_async_candidate_with_shared_load_and_document_character_set(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
    ) -> bool {
        let source_load_port = DocumentScriptSourceLoadPort::new(|_, _| {
            panic!("source-loading unit test must inject its SharedScriptSourceLoad")
        });
        self.runner
            .on_parser_discovered_async_candidate_with_source_load_port(
                script,
                &source_load_port,
                shared_load,
                document_character_set,
                |_| None,
            )
    }

    pub(crate) fn accept_parser_discovered_async_candidate(
        &mut self,
        script: PreparedScript,
        loader: &DocumentResourceLoader,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(
            &PreparedScript,
        )
            -> crate::frame_owner_model::MainDocumentScriptLoadDelayLease,
    ) -> bool {
        let source_load_port =
            document_script_source_load_port(loader, self.runner.owner_wake.clone());
        self.runner
            .on_parser_discovered_async_candidate_with_source_load_port(
                script,
                &source_load_port,
                shared_load,
                document_character_set,
                |script| Some(bind_load_delay(script)),
            )
    }

    #[cfg(test)]
    pub(crate) fn recover_parse_time_async_handoff(&mut self, script: PreparedScript) -> bool {
        let source_load_port = DocumentScriptSourceLoadPort::new(|_, _| {
            panic!("source-loading unit test must inject its SharedScriptSourceLoad")
        });
        self.runner
            .recover_parse_time_async_handoff_with_source_load_port(
                script,
                &source_load_port,
                None,
                None,
                |_| None,
            )
    }

    pub(crate) fn recover_parse_time_async_handoff_with_load_delay_binding(
        &mut self,
        script: PreparedScript,
        loader: &DocumentResourceLoader,
        shared_load: Option<SharedScriptSourceLoad>,
        document_character_set: Option<&str>,
        bind_load_delay: impl FnOnce(
            &PreparedScript,
        )
            -> crate::frame_owner_model::MainDocumentScriptLoadDelayLease,
    ) -> bool {
        let source_load_port =
            document_script_source_load_port(loader, self.runner.owner_wake.clone());
        self.runner
            .recover_parse_time_async_handoff_with_source_load_port(
                script,
                &source_load_port,
                shared_load,
                document_character_set,
                |script| Some(bind_load_delay(script)),
            )
    }

    #[cfg(test)]
    pub(super) fn claim_parse_time_async_handoff_with_shared_load(
        &mut self,
        script: PreparedScript,
        shared_load: Option<SharedScriptSourceLoad>,
    ) -> bool {
        let source_load_port = DocumentScriptSourceLoadPort::new(|_, _| {
            panic!("source-loading unit test must inject its SharedScriptSourceLoad")
        });
        self.runner
            .claim_existing_parse_time_async_handoff(script.node_id)
            || self
                .runner
                .recover_parse_time_async_handoff_with_source_load_port(
                    script,
                    &source_load_port,
                    shared_load,
                    None,
                    |_| None,
                )
    }
}
