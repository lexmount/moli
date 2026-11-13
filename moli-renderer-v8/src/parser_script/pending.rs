use crate::document_runtime::DomHandle;
use crate::parser_script::action::{
    ParserClassicScriptRunnerStep, ParserPendingClassicScriptExecution,
    ParserPendingClassicScriptNotification, ParserPendingClassicScriptReady,
    ParserPendingClassicScriptReadyKind,
};
use crate::parser_script::payload::{
    ParserClassicScriptMetadata, ParserClassicScriptSourceFailure,
    ParserClassicScriptSourceIdentity, ParserClassicScriptSourceResult,
    ParserPreparedClassicScript, ParserReadyClassicScript,
};
use crate::planning::PreparedScript;
use crate::types::SharedNavigationResponseResult;
use url::Url;

#[derive(Debug, Clone)]
pub(crate) struct ParserPendingClassicScript {
    input: ParserPreparedClassicScript,
}

impl ParserPendingClassicScript {
    pub(crate) fn new(input: ParserPreparedClassicScript) -> Self {
        Self { input }
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.input.metadata()
    }

    pub(crate) fn input(&self) -> &ParserPreparedClassicScript {
        &self.input
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        self.input.script()
    }

    pub(crate) fn script_mut(&mut self) -> &mut PreparedScript {
        self.input.script_mut()
    }

    pub(crate) fn script_url(&self) -> &Url {
        self.input.script_url()
    }

    pub(crate) fn source_identity(
        &self,
        load_id: Option<u64>,
    ) -> ParserClassicScriptSourceIdentity {
        ParserClassicScriptSourceIdentity::new(self.metadata(), load_id)
    }

    pub(crate) fn ready_script(&self) -> Option<ParserReadyClassicScript> {
        self.input.ready_script()
    }

    pub(crate) fn apply_source_result(
        &mut self,
        load_id: Option<u64>,
        source_result: ParserClassicScriptSourceResult,
    ) -> Option<Result<Option<SharedNavigationResponseResult>, String>> {
        if !self
            .source_identity(load_id)
            .matches_source_result(&source_result)
        {
            return None;
        }
        Some(
            self.input
                .apply_source_load_outcome(source_result.into_outcome()),
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserPendingClassicScriptEntry {
    readiness: ParserPendingClassicScriptReadiness,
}

impl ParserPendingClassicScriptEntry {
    pub(crate) fn inline_ready(input: ParserPreparedClassicScript) -> Self {
        Self {
            readiness: ParserPendingClassicScriptReadiness::inline_ready(input),
        }
    }

    pub(crate) fn external_pending(input: ParserPreparedClassicScript) -> Self {
        Self {
            readiness: ParserPendingClassicScriptReadiness::external_pending(input),
        }
    }

    #[cfg(test)]
    pub(crate) fn external_ready(input: ParserPreparedClassicScript) -> Self {
        Self {
            readiness: ParserPendingClassicScriptReadiness::external_ready(input),
        }
    }

    #[cfg(test)]
    pub(crate) fn external_loading(
        input: ParserPreparedClassicScript,
        source_identity: ParserClassicScriptSourceIdentity,
    ) -> Self {
        Self {
            readiness: ParserPendingClassicScriptReadiness::external_loading(
                input,
                source_identity,
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn external_failed(
        metadata: ParserClassicScriptMetadata,
        script_url: Url,
        error: impl Into<String>,
    ) -> Self {
        Self {
            readiness: ParserPendingClassicScriptReadiness::external_failed(
                metadata, script_url, error,
            ),
        }
    }

    pub(crate) fn runner_step(&self) -> Option<ParserClassicScriptRunnerStep> {
        self.readiness.runner_step()
    }

    pub(crate) fn ready_execution(
        &self,
        script_handle: DomHandle,
    ) -> Option<ParserPendingClassicScriptExecution> {
        self.readiness.ready_execution(script_handle)
    }

    pub(crate) fn external_pending_script_url(&self) -> Option<&Url> {
        self.readiness.external_pending_script_url()
    }

    pub(crate) fn begin_external_load(
        &mut self,
        load_id: u64,
    ) -> Option<(
        ParserClassicScriptSourceIdentity,
        ParserPreparedClassicScript,
    )> {
        self.readiness.begin_external_load(load_id)
    }

    pub(crate) fn begin_external_load_with_load_id(
        &mut self,
        load_id: Option<u64>,
    ) -> Option<(
        ParserClassicScriptSourceIdentity,
        ParserPreparedClassicScript,
    )> {
        self.readiness.begin_external_load_with_load_id(load_id)
    }

    pub(crate) fn fail_external_pending_before_load(&mut self, error: String) -> bool {
        self.readiness.fail_external_pending_before_load(error)
    }

    pub(crate) fn external_load_identity(
        &self,
    ) -> Option<(ParserClassicScriptSourceIdentity, Url)> {
        self.readiness.external_load_identity()
    }

    pub(crate) fn failed_source(&self) -> Option<ParserClassicScriptSourceFailure> {
        self.readiness.failed_source()
    }

    pub(crate) fn metadata(&self) -> Option<ParserClassicScriptMetadata> {
        self.readiness.metadata()
    }

    pub(crate) fn script(&self) -> Option<&PreparedScript> {
        self.readiness.script()
    }

    pub(crate) fn script_mut(&mut self) -> Option<&mut PreparedScript> {
        self.readiness.script_mut()
    }

    pub(crate) fn promote_external_pending_to_ready(&mut self) -> bool {
        self.readiness.promote_external_pending_to_ready()
    }

    pub(crate) fn notify_source_result_with_network_result(
        &mut self,
        source_result: ParserClassicScriptSourceResult,
    ) -> Option<(
        ParserPendingClassicScriptNotification,
        Option<SharedNavigationResponseResult>,
    )> {
        self.readiness
            .notify_source_result_with_network_result(source_result)
    }
}

#[derive(Debug, Clone)]
enum ParserPendingClassicScriptReadiness {
    InlineReady {
        script: ParserPendingClassicScript,
    },
    ExternalPending {
        script: ParserPendingClassicScript,
    },
    ExternalLoading {
        script: ParserPendingClassicScript,
        source_identity: ParserClassicScriptSourceIdentity,
    },
    ExternalReady {
        script: ParserPendingClassicScript,
    },
    ExternalFailed {
        metadata: ParserClassicScriptMetadata,
        script_url: Url,
        error: String,
        prepared_script: Option<Box<PreparedScript>>,
        source_network_result: Option<SharedNavigationResponseResult>,
        network_request_handle: Option<crate::types::SubresourceNetworkRequestHandle>,
    },
}

impl ParserPendingClassicScriptReadiness {
    pub(crate) fn inline_ready(input: ParserPreparedClassicScript) -> Self {
        Self::InlineReady {
            script: ParserPendingClassicScript::new(input),
        }
    }

    pub(crate) fn external_pending(input: ParserPreparedClassicScript) -> Self {
        Self::ExternalPending {
            script: ParserPendingClassicScript::new(input),
        }
    }

    #[cfg(test)]
    pub(crate) fn external_ready(input: ParserPreparedClassicScript) -> Self {
        Self::ExternalReady {
            script: ParserPendingClassicScript::new(input),
        }
    }

    #[cfg(test)]
    pub(crate) fn external_loading(
        input: ParserPreparedClassicScript,
        source_identity: ParserClassicScriptSourceIdentity,
    ) -> Self {
        Self::ExternalLoading {
            script: ParserPendingClassicScript::new(input),
            source_identity,
        }
    }

    #[cfg(test)]
    pub(crate) fn external_failed(
        metadata: ParserClassicScriptMetadata,
        script_url: Url,
        error: impl Into<String>,
    ) -> Self {
        Self::ExternalFailed {
            metadata,
            script_url,
            error: error.into(),
            prepared_script: None,
            source_network_result: None,
            network_request_handle: None,
        }
    }

    pub(crate) fn ready_script(&self) -> Option<ParserPendingClassicScriptReady> {
        match self {
            Self::InlineReady { script } => Some(ParserPendingClassicScriptReady::new(
                script.ready_script()?,
                ParserPendingClassicScriptReadyKind::ParserConnected,
            )),
            Self::ExternalReady { script } => Some(ParserPendingClassicScriptReady::new(
                script.ready_script()?,
                ParserPendingClassicScriptReadyKind::External,
            )),
            _ => None,
        }
    }

    pub(crate) fn runner_step(&self) -> Option<ParserClassicScriptRunnerStep> {
        if let Some(ready) = self.ready_script() {
            return Some(ParserClassicScriptRunnerStep::Ready(ready));
        }
        self.failed_source()
            .map(ParserClassicScriptRunnerStep::SourceFailed)
    }

    pub(crate) fn ready_execution(
        &self,
        script_handle: DomHandle,
    ) -> Option<ParserPendingClassicScriptExecution> {
        self.ready_script()?.execution_for_script(script_handle)
    }

    pub(crate) fn external_pending_script_url(&self) -> Option<&Url> {
        let Self::ExternalPending { script } = self else {
            return None;
        };
        Some(script.script_url())
    }

    pub(crate) fn begin_external_load(
        &mut self,
        load_id: u64,
    ) -> Option<(
        ParserClassicScriptSourceIdentity,
        ParserPreparedClassicScript,
    )> {
        self.begin_external_load_with_load_id(Some(load_id))
    }

    pub(crate) fn begin_external_load_with_load_id(
        &mut self,
        load_id: Option<u64>,
    ) -> Option<(
        ParserClassicScriptSourceIdentity,
        ParserPreparedClassicScript,
    )> {
        let Self::ExternalPending { script } = self else {
            return None;
        };
        let script = script.clone();
        let source_identity = script.source_identity(load_id);
        *self = Self::ExternalLoading {
            script: script.clone(),
            source_identity,
        };
        Some((source_identity, script.input().clone()))
    }

    pub(crate) fn fail_external_pending_before_load(&mut self, error: String) -> bool {
        let Self::ExternalPending { script } = self else {
            return false;
        };
        let metadata = script.metadata();
        let script_url = script.script_url().clone();
        let prepared_script = Some(Box::new(script.script().clone()));
        *self = Self::ExternalFailed {
            metadata,
            script_url,
            error,
            prepared_script,
            source_network_result: None,
            network_request_handle: None,
        };
        true
    }

    pub(crate) fn external_load_identity(
        &self,
    ) -> Option<(ParserClassicScriptSourceIdentity, Url)> {
        let Self::ExternalLoading {
            script,
            source_identity,
        } = self
        else {
            return None;
        };
        Some((*source_identity, script.script_url().clone()))
    }

    pub(crate) fn failed_source(&self) -> Option<ParserClassicScriptSourceFailure> {
        let Self::ExternalFailed {
            metadata,
            script_url,
            error,
            prepared_script,
            source_network_result,
            network_request_handle,
        } = self
        else {
            return None;
        };
        Some(ParserClassicScriptSourceFailure {
            metadata: *metadata,
            script_url: script_url.clone(),
            error: error.clone(),
            prepared_script: prepared_script.clone(),
            source_network_result: source_network_result.clone(),
            network_request_handle: *network_request_handle,
        })
    }

    pub(crate) fn metadata(&self) -> Option<ParserClassicScriptMetadata> {
        match self {
            Self::ExternalFailed { metadata, .. } => Some(*metadata),
            _ => Some(self.pending_script()?.metadata()),
        }
    }

    pub(crate) fn script(&self) -> Option<&PreparedScript> {
        Some(self.pending_script()?.script())
    }

    pub(crate) fn script_mut(&mut self) -> Option<&mut PreparedScript> {
        Some(self.pending_script_mut()?.script_mut())
    }

    pub(crate) fn promote_external_pending_to_ready(&mut self) -> bool {
        let Self::ExternalPending { script } = self else {
            return false;
        };
        *self = Self::ExternalReady {
            script: script.clone(),
        };
        true
    }

    fn pending_script(&self) -> Option<&ParserPendingClassicScript> {
        match self {
            Self::InlineReady { script }
            | Self::ExternalPending { script }
            | Self::ExternalLoading { script, .. }
            | Self::ExternalReady { script } => Some(script),
            Self::ExternalFailed { .. } => None,
        }
    }

    fn pending_script_mut(&mut self) -> Option<&mut ParserPendingClassicScript> {
        match self {
            Self::InlineReady { script }
            | Self::ExternalPending { script }
            | Self::ExternalLoading { script, .. }
            | Self::ExternalReady { script } => Some(script),
            Self::ExternalFailed { .. } => None,
        }
    }

    pub(crate) fn notify_source_result_with_network_result(
        &mut self,
        source_result: ParserClassicScriptSourceResult,
    ) -> Option<(
        ParserPendingClassicScriptNotification,
        Option<SharedNavigationResponseResult>,
    )> {
        let Self::ExternalLoading {
            script,
            source_identity,
        } = self
        else {
            return None;
        };
        if !source_identity.matches_source_result(&source_result) {
            return None;
        }
        let mut script = script.clone();
        let metadata = script.metadata();
        let script_url = script.script_url().clone();
        let prepared_script = script.script().clone();
        let source_network_result = source_result.network_result().cloned();
        let network_request_handle = source_result.network_request_handle();
        match script.apply_source_result(source_identity.load_id(), source_result) {
            Some(Ok(network_result)) => {
                *self = Self::ExternalReady { script };
                Some((
                    ParserPendingClassicScriptNotification::SourceReady,
                    network_result,
                ))
            }
            Some(Err(error)) => {
                *self = Self::ExternalFailed {
                    metadata,
                    script_url,
                    error,
                    prepared_script: Some(Box::new(prepared_script)),
                    source_network_result,
                    network_request_handle,
                };
                Some((ParserPendingClassicScriptNotification::SourceFailed, None))
            }
            None => None,
        }
    }
}
