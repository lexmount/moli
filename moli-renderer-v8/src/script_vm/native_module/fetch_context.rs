use crate::{
    module_runtime::{ModuleFetchMetadata, ModuleKind, NativeModuleFetchContext},
    native_bridge::OwnerDispatchScope,
};
use std::sync::Arc;

impl crate::script_vm::ScriptVm {
    pub(crate) fn prepare_main_module_graph_fetch(
        &mut self,
        request: crate::module_runtime::NativeModuleGraphFetchRequest,
    ) -> Result<
        crate::module_runtime::NativeModuleGraphFetchRequest,
        crate::module_runtime::ModuleLoadError,
    > {
        use crate::module_runtime::{ModuleLoadError, ModuleLoadStage};
        let context = self
            .with_default_context_scope(|scope, host_ptr| {
                NativeModuleFetchContext::capture(
                    scope,
                    unsafe { &*host_ptr },
                    OwnerDispatchScope::Top,
                    request.module_kind(),
                    request.fetch_metadata(),
                    request.csp_source_location().cloned(),
                )
                .ok_or_else(|| {
                    anyhow::anyhow!("module fetch lost its exact main Document fetch context")
                })
            })
            .map_err(|error| ModuleLoadError::new(ModuleLoadStage::Fetch, error.to_string()))?;
        Ok(request.with_fetch_context(context))
    }

    pub(crate) fn capture_child_module_graph_fetch_context(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: crate::frame_owner_model::FrameRealmId,
        request: &crate::module_runtime::NativeModuleGraphFetchRequest,
    ) -> Result<Arc<NativeModuleFetchContext>, String> {
        let handle = self
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot_for_realm(realm_id)
            .filter(|snapshot| {
                snapshot.local_window_id == owner.local_window_id
                    && snapshot.document_id == owner.document_id
            })
            .map(|snapshot| snapshot.owner_handle)
            .ok_or_else(|| "module fetch lost its exact child Document realm".to_owned())?;
        self.capture_module_fetch_context(
            OwnerDispatchScope::Child(handle),
            request.module_kind(),
            request.fetch_metadata(),
            request.csp_source_location().cloned(),
        )
    }

    pub(crate) fn capture_module_fetch_context(
        &mut self,
        owner: OwnerDispatchScope,
        kind: ModuleKind,
        metadata: &ModuleFetchMetadata,
        source_location: Option<
            crate::content_security_policy::ContentSecurityPolicySourceLocation,
        >,
    ) -> Result<Arc<NativeModuleFetchContext>, String> {
        self.with_default_context_scope(|scope, host_ptr| {
            NativeModuleFetchContext::capture(
                scope,
                unsafe { &*host_ptr },
                owner,
                kind,
                metadata,
                source_location,
            )
            .ok_or_else(|| anyhow::anyhow!("module fetch lost its exact Document fetch context"))
        })
        .map_err(|error| error.to_string())
    }
}
