use crate::{
    module_runtime::{ModuleFetchMetadata, ModuleKind, NativeModuleFetchContext},
    native_bridge::OwnerDispatchScope,
};
use std::sync::Arc;

impl crate::script_vm::ScriptVm {
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
                unsafe { &mut *host_ptr },
                owner,
                kind,
                metadata,
                source_location,
            )
            .ok_or_else(|| anyhow::anyhow!("modulepreload lost its exact Document fetch context"))
        })
        .map_err(|error| error.to_string())
    }
}
