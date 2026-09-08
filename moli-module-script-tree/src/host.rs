use url::Url;

use crate::types::{
    CompiledModuleSnapshot, FetchedModuleSource, ModuleAttributesKey, ModuleDependencyEdge,
    ModuleDependencySnapshot, ModuleEntryId, ModuleFetchRequest, ModuleGraphHandle,
    ModuleLoadError, ModuleMapKey, ResolvedModuleRequest, SingleModuleClientToken,
    SingleModuleFetchDisposition,
};

pub trait ModuleScriptTreeHost {
    fn resolve_module_request(
        &mut self,
        specifier: &str,
        base_url: &Url,
        attributes: &ModuleAttributesKey,
        requested_phase: crate::ModuleImportPhase,
    ) -> Result<ResolvedModuleRequest, ModuleLoadError>;

    fn start_or_join_single_module_fetch(
        &mut self,
        request: ModuleFetchRequest,
        client: SingleModuleClientToken,
    ) -> SingleModuleFetchDisposition;

    fn compile_module_source(
        &mut self,
        fetched_source: FetchedModuleSource,
        phase: crate::ModuleImportPhase,
    ) -> Result<CompiledModuleSnapshot, ModuleLoadError>;

    fn module_dependencies(
        &self,
        entry: ModuleEntryId,
    ) -> Result<ModuleDependencySnapshot, ModuleLoadError>;

    fn link_module_graph(
        &mut self,
        root: ModuleEntryId,
        entries: &[ModuleEntryId],
        dependency_edges: &[ModuleDependencyEdge],
    ) -> Result<ModuleGraphHandle, ModuleLoadError>;

    fn mark_module_failed(&mut self, key: ModuleMapKey, error: ModuleLoadError) -> ModuleEntryId;

    /// Static requested-module validation errors belong to the module script,
    /// unlike failures to resolve the specifier passed directly to import().
    /// Retain the actual exception and cache it for subsequent graph loads.
    fn cache_module_request_error(
        &mut self,
        key: ModuleMapKey,
        error: ModuleLoadError,
    ) -> ModuleLoadError;
}
