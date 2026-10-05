use crate::document_runtime::DomHandle;
use crate::planning::{
    PreparedScript, PreparedScriptSourceLoadOutcome, ScriptSource,
    prepared_script_with_loaded_source,
};
use crate::types::{ScriptSourceKind, SharedNavigationResponseResult};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptMetadata {
    script_handle: DomHandle,
    start_line: u64,
}

impl ParserClassicScriptMetadata {
    pub(crate) fn new(script_handle: DomHandle, start_line: u64) -> Self {
        Self {
            script_handle,
            start_line: start_line.max(1),
        }
    }

    pub(crate) fn script_handle(self) -> DomHandle {
        self.script_handle
    }

    #[cfg(test)]
    pub(crate) fn start_line(self) -> u64 {
        self.start_line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParserClassicScriptSourceIdentity {
    metadata: ParserClassicScriptMetadata,
    load_id: Option<u64>,
}

impl ParserClassicScriptSourceIdentity {
    pub(crate) fn new(metadata: ParserClassicScriptMetadata, load_id: Option<u64>) -> Self {
        Self { metadata, load_id }
    }

    #[cfg(test)]
    pub(crate) fn for_external_load(metadata: ParserClassicScriptMetadata, load_id: u64) -> Self {
        Self::new(metadata, Some(load_id))
    }

    pub(crate) fn metadata(self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn load_id(self) -> Option<u64> {
        self.load_id
    }

    pub(crate) fn matches_source_result(&self, result: &ParserClassicScriptSourceResult) -> bool {
        self.metadata.script_handle() == result.metadata().script_handle()
            && self.load_id == result.load_id()
    }

    pub(crate) fn into_source_result(
        self,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> ParserClassicScriptSourceResult {
        ParserClassicScriptSourceResult::from_identity(self, outcome)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserPreparedClassicScript {
    metadata: ParserClassicScriptMetadata,
    script: Box<PreparedScript>,
}

impl ParserPreparedClassicScript {
    pub(crate) fn new(metadata: ParserClassicScriptMetadata, script: PreparedScript) -> Self {
        Self {
            metadata,
            script: Box::new(script),
        }
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn script_mut(&mut self) -> &mut PreparedScript {
        &mut self.script
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script.url
    }

    fn ready_source_text(&self) -> Option<&str> {
        match &self.script.source {
            ScriptSource::Inline(source) | ScriptSource::Loaded(source) => Some(source),
            ScriptSource::LoadedBinary { source, .. } => Some(source),
            ScriptSource::External => None,
        }
    }

    pub(crate) fn ready_script(&self) -> Option<ParserReadyClassicScript> {
        self.ready_source_text()?;
        Some(ParserReadyClassicScript::new(
            self.metadata,
            self.script.url.clone(),
        ))
    }

    pub(crate) fn apply_source_load_outcome(
        &mut self,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Result<Option<SharedNavigationResponseResult>, String> {
        let PreparedScriptSourceLoadOutcome {
            source_result,
            source_bytes,
            network_result,
            muted_errors,
        } = outcome;
        let source = source_result?;
        *self.script = prepared_script_with_loaded_source(
            (*self.script).clone(),
            source,
            source_bytes,
            muted_errors,
        );
        Ok(network_result)
    }

    pub(crate) fn into_script(self) -> PreparedScript {
        *self.script
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserExecutableClassicScript {
    script: PreparedScript,
}

impl ParserExecutableClassicScript {
    pub(crate) fn from_prepared_script(script: PreparedScript) -> Option<Self> {
        match &script.source {
            ScriptSource::Inline(_)
            | ScriptSource::Loaded(_)
            | ScriptSource::LoadedBinary { .. } => Some(Self { script }),
            ScriptSource::External => None,
        }
    }

    pub(crate) fn into_prepared_script(self) -> PreparedScript {
        self.script
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script.url
    }

    pub(crate) fn source_kind(&self) -> ScriptSourceKind {
        self.script.source_kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParserReadyClassicScript {
    metadata: ParserClassicScriptMetadata,
    script_url: Url,
}

impl ParserReadyClassicScript {
    pub(crate) fn new(metadata: ParserClassicScriptMetadata, script_url: Url) -> Self {
        Self {
            metadata,
            script_url,
        }
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.metadata
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.metadata.script_handle()
    }

    #[cfg(test)]
    pub(crate) fn start_line(&self) -> u64 {
        self.metadata.start_line()
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptSourceFailure {
    pub(crate) metadata: ParserClassicScriptMetadata,
    pub(crate) script_url: Url,
    pub(crate) error: String,
    pub(crate) prepared_script: Option<Box<PreparedScript>>,
    pub(crate) source_network_result: Option<SharedNavigationResponseResult>,
}

impl ParserClassicScriptSourceFailure {
    pub(crate) fn script_handle(&self) -> DomHandle {
        self.metadata.script_handle()
    }

    pub(crate) fn script_url(&self) -> &Url {
        &self.script_url
    }

    pub(crate) fn error(&self) -> &str {
        &self.error
    }

    pub(crate) fn into_execution_failure_parts(
        self,
    ) -> Option<(
        PreparedScript,
        String,
        Option<SharedNavigationResponseResult>,
    )> {
        Some((
            *self.prepared_script?,
            self.error,
            self.source_network_result,
        ))
    }
}

impl PartialEq for ParserClassicScriptSourceFailure {
    fn eq(&self, other: &Self) -> bool {
        self.metadata == other.metadata
            && self.script_url == other.script_url
            && self.error == other.error
    }
}

impl Eq for ParserClassicScriptSourceFailure {}

#[derive(Debug, Clone)]
pub(crate) struct ParserClassicScriptSourceResult {
    identity: ParserClassicScriptSourceIdentity,
    outcome: PreparedScriptSourceLoadOutcome,
}

impl ParserClassicScriptSourceResult {
    #[cfg(test)]
    pub(crate) fn new(
        load_id: u64,
        metadata: ParserClassicScriptMetadata,
        result: Result<String, String>,
    ) -> Self {
        Self::from_identity_result(
            ParserClassicScriptSourceIdentity::for_external_load(metadata, load_id),
            result,
        )
    }

    pub(crate) fn from_identity_result(
        identity: ParserClassicScriptSourceIdentity,
        result: Result<String, String>,
    ) -> Self {
        Self::from_identity(
            identity,
            PreparedScriptSourceLoadOutcome {
                source_result: result,
                source_bytes: None,
                network_result: None,
                muted_errors: false,
            },
        )
    }

    pub(crate) fn from_identity(
        identity: ParserClassicScriptSourceIdentity,
        outcome: PreparedScriptSourceLoadOutcome,
    ) -> Self {
        Self { identity, outcome }
    }

    pub(crate) fn load_id(&self) -> Option<u64> {
        self.identity.load_id()
    }

    pub(crate) fn metadata(&self) -> ParserClassicScriptMetadata {
        self.identity.metadata()
    }

    pub(crate) fn network_result(&self) -> Option<&SharedNavigationResponseResult> {
        self.outcome.network_result.as_ref()
    }

    pub(crate) fn into_outcome(self) -> PreparedScriptSourceLoadOutcome {
        self.outcome
    }
}
