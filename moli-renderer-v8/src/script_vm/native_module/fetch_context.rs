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
    ) -> Result<Arc<NativeModuleFetchContext>, String> {
        self.with_default_context_scope(|scope, host_ptr| {
            NativeModuleFetchContext::capture(
                scope,
                unsafe { &mut *host_ptr },
                owner,
                kind,
                metadata,
            )
            .ok_or_else(|| anyhow::anyhow!("modulepreload lost its exact Document fetch context"))
        })
        .map_err(|error| error.to_string())
    }

    pub(crate) fn report_module_fetch_context(
        &mut self,
        context: &NativeModuleFetchContext,
        dispatch_dom_events: bool,
    ) {
        if let Err(error) = self.with_default_context_scope(|scope, host_ptr| {
            context.report_violations(scope, unsafe { &mut *host_ptr }, dispatch_dom_events);
            Ok(())
        }) {
            self.record_runtime_warning(format_args!(
                "modulepreload CSP reporting failed: {error}"
            ));
        }
    }
}
