use std::fmt;

use moli_module_script_tree as module_tree;

use super::{
    ModuleCompiledRecordId, ModuleFetchMetadata, ModuleIdentityHash, ModuleKind, ModuleLoadError,
    ModuleMapEntryState, ModuleMapFetchClient, ModuleMapKey, ModuleMapTerminalClients,
    ModuleResolvedDependency, ModuleSource,
};

pub(crate) struct ModuleMapEntry {
    key: ModuleMapKey,
    effective_key: ModuleMapKey,
    state: ModuleMapEntryState,
    source: Option<ModuleSource>,
    effective_fetch_metadata: ModuleFetchMetadata,
    compiled_record_id: Option<ModuleCompiledRecordId>,
    record_identity: Option<ModuleIdentityHash>,
    resolved_dependencies: Vec<ModuleResolvedDependency>,
    failure: Option<ModuleLoadError>,
    fetch_clients: Vec<ModuleMapFetchClient>,
}

impl fmt::Debug for ModuleMapEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModuleMapEntry")
            .field("key", &self.key)
            .field("effective_key", &self.effective_key)
            .field("state", &self.state)
            .field("has_source", &self.source.is_some())
            .field("compiled_record_id", &self.compiled_record_id)
            .field("record_identity", &self.record_identity)
            .field(
                "resolved_dependency_count",
                &self.resolved_dependencies.len(),
            )
            .field("failure", &self.failure)
            .field("fetch_client_count", &self.fetch_client_count())
            .field(
                "single_module_fetch_client_count",
                &self.single_module_fetch_client_count(),
            )
            .field(
                "module_script_client_count",
                &self.module_script_client_count(),
            )
            .field(
                "modulepreload_link_client_count",
                &self.modulepreload_link_client_count(),
            )
            .field(
                "parser_root_module_client_count",
                &self.parser_root_module_client_count(),
            )
            .finish()
    }
}

impl ModuleMapEntry {
    pub(crate) fn new(key: ModuleMapKey, state: ModuleMapEntryState) -> Self {
        Self {
            effective_key: key.clone(),
            key,
            state,
            source: None,
            effective_fetch_metadata: ModuleFetchMetadata::default(),
            compiled_record_id: None,
            record_identity: None,
            resolved_dependencies: Vec::new(),
            failure: None,
            fetch_clients: Vec::new(),
        }
    }

    pub(crate) fn effective_key(&self) -> &ModuleMapKey {
        &self.effective_key
    }

    pub(crate) fn key(&self) -> &ModuleMapKey {
        &self.key
    }

    pub(crate) fn state(&self) -> ModuleMapEntryState {
        self.state
    }

    pub(crate) fn source(&self) -> Option<&ModuleSource> {
        self.source.as_ref()
    }

    pub(crate) fn effective_fetch_metadata(&self) -> &ModuleFetchMetadata {
        &self.effective_fetch_metadata
    }

    pub(crate) fn failure(&self) -> Option<&ModuleLoadError> {
        self.failure.as_ref()
    }

    pub(crate) fn compiled_record_id(&self) -> Option<ModuleCompiledRecordId> {
        self.compiled_record_id
    }

    pub(crate) fn usable_compiled_record_id(&self) -> Option<ModuleCompiledRecordId> {
        matches!(
            self.state,
            ModuleMapEntryState::Compiled
                | ModuleMapEntryState::Instantiated
                | ModuleMapEntryState::Evaluating
                | ModuleMapEntryState::Evaluated
        )
        .then_some(self.compiled_record_id)
        .flatten()
    }

    pub(crate) fn resolved_dependencies(&self) -> &[ModuleResolvedDependency] {
        &self.resolved_dependencies
    }

    pub(crate) fn set_fetched_source(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        source: ModuleSource,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) {
        self.key = request_key;
        self.effective_key = effective_key;
        self.state = ModuleMapEntryState::Fetched;
        self.source = Some(source);
        self.effective_fetch_metadata = effective_fetch_metadata;
        self.resolved_dependencies.clear();
        self.failure = None;
    }

    pub(crate) fn set_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) {
        self.key = key.clone();
        self.effective_key = key;
        self.state = ModuleMapEntryState::Failed;
        self.effective_fetch_metadata = ModuleFetchMetadata::default();
        self.resolved_dependencies.clear();
        self.failure = Some(error);
    }

    pub(crate) fn set_compiled_record(
        &mut self,
        request_key: ModuleMapKey,
        effective_key: ModuleMapKey,
        compiled_record_id: ModuleCompiledRecordId,
        identity: ModuleIdentityHash,
        effective_fetch_metadata: ModuleFetchMetadata,
    ) {
        // V8 owns the compiled JavaScript module from this point onward. Keep
        // synthetic-module sources because their evaluation callbacks still
        // consume them after compilation.
        if effective_key.kind() == ModuleKind::JavaScript {
            self.source = None;
        }
        self.key = request_key;
        self.effective_key = effective_key;
        self.state = ModuleMapEntryState::Compiled;
        self.effective_fetch_metadata = effective_fetch_metadata;
        self.compiled_record_id = Some(compiled_record_id);
        self.record_identity = Some(identity);
        self.resolved_dependencies.clear();
        self.failure = None;
    }

    pub(crate) fn set_resolved_dependencies(
        &mut self,
        dependencies: Vec<ModuleResolvedDependency>,
    ) {
        self.resolved_dependencies = dependencies;
    }

    pub(crate) fn mark_instantiated(&mut self) {
        self.state = ModuleMapEntryState::Instantiated;
    }

    pub(crate) fn mark_evaluating(&mut self) {
        self.state = ModuleMapEntryState::Evaluating;
    }

    pub(crate) fn mark_evaluated(&mut self) {
        self.state = ModuleMapEntryState::Evaluated;
    }

    pub(crate) fn clear_compiled_record(
        &mut self,
    ) -> Option<(ModuleCompiledRecordId, Option<ModuleIdentityHash>)> {
        let record_id = self.compiled_record_id.take()?;
        Some((record_id, self.record_identity.take()))
    }

    pub(crate) fn push_fetch_client(&mut self, client: ModuleMapFetchClient) {
        self.fetch_clients.push(client);
    }

    pub(crate) fn fetch_client_count(&self) -> usize {
        self.fetch_clients.len()
    }

    pub(crate) fn single_module_fetch_client_count(&self) -> usize {
        self.fetch_clients
            .iter()
            .filter(|client| client.is_single_module_fetch())
            .count()
    }

    pub(crate) fn module_script_client_count(&self) -> usize {
        self.fetch_clients
            .iter()
            .filter(|client| client.is_module_script())
            .count()
    }

    pub(crate) fn modulepreload_link_client_count(&self) -> usize {
        self.fetch_clients
            .iter()
            .filter(|client| client.is_modulepreload_link())
            .count()
    }

    pub(crate) fn parser_root_module_client_count(&self) -> usize {
        self.fetch_clients
            .iter()
            .filter(|client| client.is_parser_root_module())
            .count()
    }

    pub(crate) fn detach_single_module_fetch_client(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> bool {
        let Some(position) = self
            .fetch_clients
            .iter()
            .position(|current| current.detach_single_module_client(client))
        else {
            return false;
        };
        self.fetch_clients.remove(position);
        true
    }

    pub(crate) fn retain_dynamic_import_clients(&mut self) {
        self.fetch_clients
            .retain(ModuleMapFetchClient::is_dynamic_import);
    }

    pub(crate) fn take_terminal_clients(&mut self) -> ModuleMapTerminalClients {
        let mut clients = ModuleMapTerminalClients::default();
        for client in std::mem::take(&mut self.fetch_clients) {
            clients.push(client);
        }
        clients
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_module_graph::ModuleAttributesKey;
    use url::Url;

    fn compiled_entry_for(key: ModuleMapKey) -> ModuleMapEntry {
        let mut entry = ModuleMapEntry::new(key.clone(), ModuleMapEntryState::Fetching);
        entry.set_fetched_source(
            key.clone(),
            key.clone(),
            ModuleSource::text("export default 1;".to_owned()),
            ModuleFetchMetadata::default(),
        );
        entry.set_compiled_record(
            key.clone(),
            key,
            ModuleCompiledRecordId::from_index(0),
            ModuleIdentityHash::from_raw(1),
            ModuleFetchMetadata::default(),
        );
        entry
    }

    #[test]
    fn compiled_javascript_entry_releases_source() {
        let key = ModuleMapKey::java_script(
            Url::parse("https://example.test/module.js").expect("module URL"),
        );

        assert!(compiled_entry_for(key).source().is_none());
    }

    #[test]
    fn compiled_synthetic_entry_retains_source_for_evaluation() {
        let key = ModuleMapKey::json_with_attributes(
            Url::parse("https://example.test/data.json").expect("module URL"),
            ModuleAttributesKey::from_pairs(vec![("type".to_owned(), "json".to_owned())]),
        );

        assert!(compiled_entry_for(key).source().is_some());
    }
}
