use super::*;

impl ScriptVm {
    pub(crate) fn start_main_response_links(
        &mut self,
        links: &crate::response_links::DocumentResponseLinks,
    ) -> Result<()> {
        if links.dictionaries.is_empty() && links.scripts.is_empty() && links.modules.is_empty() {
            return Ok(());
        }
        let owner = self.current_main_document_task_owner();
        let (scripting_enabled, referrer_policy) =
            self.with_default_context_scope(|scope, host_ptr| {
                let host = unsafe { &mut *host_ptr };
                let dispatch_scope = crate::native_bridge::OwnerDispatchScope::Top;
                let scripting_enabled = host.owner_scripting_enabled(dispatch_scope);
                let referrer_policy =
                    host.document_response_preload_referrer_policy(dispatch_scope);
                if let Some(target) =
                    host.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
                {
                    crate::network_host::start_document_response_links(scope, host, target, links);
                }
                Ok((scripting_enabled, referrer_policy))
            })?;
        if !scripting_enabled || owner.is_none() {
            return Ok(());
        }
        for preload in &links.modules {
            if self.current_main_document_task_owner() != owner {
                break;
            }
            if let Some(candidate) = preload.fetch_candidate(
                self.document_runtime.document_url(),
                referrer_policy.clone(),
            ) && let Err(error) = self.register_native_modulepreload_for_owner(candidate.request)
            {
                tracing::debug!(url = %preload.url, error, "response modulepreload was not loaded");
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn start_main_response_dictionary_loads(&mut self, links: &[Url]) -> Result<()> {
        if links.is_empty() {
            return Ok(());
        }
        self.with_default_context_scope(|scope, host_ptr| {
            // This body operation runs on this Document's isolate owner lane.
            // The resource consumer retains native Document identity, not V8.
            let host = unsafe { &mut *host_ptr };
            if let Some(target) = host.current_window_document_task_target_for_dispatch_scope(
                crate::native_bridge::OwnerDispatchScope::Top,
            ) {
                crate::network_host::start_response_dictionary_loads(scope, host, target, links);
            }
            Ok(())
        })
    }
}
