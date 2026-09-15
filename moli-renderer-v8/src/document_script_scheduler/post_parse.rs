use std::collections::HashSet;

use crate::{
    frame_owner_model::DocumentLoadDelayTokenId,
    planning::{
        PreparedScript, PreparedScriptSourceLoadOutcome, prepared_script_with_loaded_source,
    },
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
    types::{ScriptKind, ScriptMode, ScriptSourceKind, SharedNavigationResponseResult},
};

#[derive(Debug)]
pub(crate) enum ParserDeferredClassicReady {
    Execute {
        script: Box<PreparedScript>,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_token: DocumentLoadDelayTokenId,
    },
    SourceFailure {
        script: Box<PreparedScript>,
        error: String,
        source_network_result: Option<SharedNavigationResponseResult>,
        load_delay_token: DocumentLoadDelayTokenId,
    },
}

impl ParserDeferredClassicReady {
    #[cfg(test)]
    pub(super) fn script(&self) -> &PreparedScript {
        match self {
            Self::Execute { script, .. } | Self::SourceFailure { script, .. } => script,
        }
    }
}

#[derive(Debug)]
enum ParserDeferredClassicSourceState {
    NotClassic,
    Pending,
    Ready(Option<SharedNavigationResponseResult>),
    Failed {
        error: String,
        network_result: Option<SharedNavigationResponseResult>,
    },
}

#[derive(Debug)]
pub(crate) struct ResolvedDeferPhaseScript {
    script: PreparedScript,
    classic_source_state: ParserDeferredClassicSourceState,
    blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    load_delay_token: DocumentLoadDelayTokenId,
}

impl ResolvedDeferPhaseScript {
    pub(crate) fn from_script(
        script: PreparedScript,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
        load_delay_token: DocumentLoadDelayTokenId,
    ) -> Self {
        let classic_source_state = if script.kind != ScriptKind::Classic {
            ParserDeferredClassicSourceState::NotClassic
        } else if should_directly_own_external_defer_like_source(&script) {
            ParserDeferredClassicSourceState::Pending
        } else {
            ParserDeferredClassicSourceState::Ready(None)
        };
        Self {
            script,
            classic_source_state,
            blocking_signatures_before,
            load_delay_token,
        }
    }

    #[cfg(test)]
    pub(super) fn source_network_result(
        &self,
    ) -> Option<&std::result::Result<crate::types::NavigationResponse, String>> {
        match &self.classic_source_state {
            ParserDeferredClassicSourceState::Ready(network_result)
            | ParserDeferredClassicSourceState::Failed { network_result, .. } => {
                network_result.as_deref()
            }
            ParserDeferredClassicSourceState::NotClassic
            | ParserDeferredClassicSourceState::Pending => None,
        }
    }

    pub(crate) fn blocking_signatures_before(
        &self,
    ) -> &HashSet<DocumentBlockingStylesheetSignature> {
        &self.blocking_signatures_before
    }

    pub(super) fn apply_classic_source_load_outcome(
        &mut self,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> bool {
        if !matches!(
            self.classic_source_state,
            ParserDeferredClassicSourceState::Pending
        ) {
            return false;
        }
        let PreparedScriptSourceLoadOutcome {
            source_result,
            source_bytes,
            network_result,
            muted_errors,
        } = outcome;
        match source_result {
            Ok(source) => {
                self.script = prepared_script_with_loaded_source(
                    self.script.clone(),
                    source,
                    source_bytes,
                    muted_errors,
                );
                self.classic_source_state = ParserDeferredClassicSourceState::Ready(network_result);
            }
            Err(error) => {
                self.classic_source_state = ParserDeferredClassicSourceState::Failed {
                    error,
                    network_result,
                };
            }
        }
        true
    }

    pub(super) fn classic_source_is_terminal(&self) -> bool {
        matches!(
            self.classic_source_state,
            ParserDeferredClassicSourceState::Ready(_)
                | ParserDeferredClassicSourceState::Failed { .. }
        )
    }

    pub(super) fn into_classic_ready(self) -> Option<ParserDeferredClassicReady> {
        let Self {
            script,
            classic_source_state,
            blocking_signatures_before: _,
            load_delay_token,
        } = self;
        match classic_source_state {
            ParserDeferredClassicSourceState::Ready(source_network_result) => {
                Some(ParserDeferredClassicReady::Execute {
                    script: Box::new(script),
                    source_network_result,
                    load_delay_token,
                })
            }
            ParserDeferredClassicSourceState::Failed {
                error,
                network_result,
            } => Some(ParserDeferredClassicReady::SourceFailure {
                script: Box::new(script),
                error,
                source_network_result: network_result,
                load_delay_token,
            }),
            ParserDeferredClassicSourceState::NotClassic
            | ParserDeferredClassicSourceState::Pending => None,
        }
    }

    pub(super) fn load_delay_token(&self) -> DocumentLoadDelayTokenId {
        self.load_delay_token
    }
}

impl std::ops::Deref for ResolvedDeferPhaseScript {
    type Target = PreparedScript;

    fn deref(&self) -> &Self::Target {
        &self.script
    }
}

pub(super) fn should_directly_own_external_defer_like_source(script: &PreparedScript) -> bool {
    script.source_kind == ScriptSourceKind::External
        && matches!(script.source, crate::parser::ScriptSource::External)
        && matches!(
            (script.kind, script.mode),
            (ScriptKind::Classic, ScriptMode::Defer)
        )
}
