use super::*;

impl ScriptVm {
    pub(crate) fn start_subresource_response_links(
        &mut self,
        response: crate::response_links::SubresourceResponseLinks,
    ) -> Result<crate::script_vm::AsyncSubresourceFetchBodyActivity> {
        use crate::script_vm::AsyncSubresourceFetchBodyActivity;

        let Some(target) = self
            ._context_host
            .borrow()
            .window_document_task_target_for_owner(response.owner)
        else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        if !self
            ._context_host
            .borrow()
            .document_resource_loader_for_window_owner(response.owner)
            .is_some_and(|loader| loader.identity() == response.loader)
        {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        let links = crate::response_links::DocumentResponseLinks::parse_subresource(
            &response.head.final_url,
            &response.head.headers,
        );
        if links.is_empty() {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        let modules = self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            if !host.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            ) {
                return Ok(Vec::new());
            }
            let policy = crate::network_host::effective_subresource_policy_context(
                scope,
                host,
                target.dispatch_scope(),
            );
            if response.source == crate::response_links::ResponseLinkResponseSource::Network
                && crate::network_host::validate_fetch_response_security_policy_with_body(
                    &response.request_origin,
                    &response.head,
                    &response.body_prefix,
                    response.request_mode,
                    response.credentials_mode,
                    policy,
                )
                .is_err()
            {
                return Ok(Vec::new());
            }
            crate::network_host::start_document_response_links(scope, host, target, &links);
            if target.dispatch_scope() != crate::native_bridge::OwnerDispatchScope::Top
                || !host.owner_scripting_enabled(target.dispatch_scope())
            {
                return Ok(Vec::new());
            }
            let Some(loader) = host.document_resource_loader_for_window_owner(target.owner())
            else {
                return Ok(Vec::new());
            };
            let context = loader.fetch_context();
            Ok(links
                .modules
                .iter()
                .filter_map(|preload| preload.fetch_candidate(context.document_url(), None))
                .collect::<Vec<_>>())
        })?;
        for candidate in modules {
            if self
                .current_main_document_task_owner()
                .map(crate::native_bridge::WindowDocumentOwner::Frame)
                != Some(response.owner)
            {
                break;
            }
            if let Err(error) = self.register_native_modulepreload_for_owner(candidate.request) {
                tracing::debug!(error, "subresource response modulepreload was not loaded");
            }
        }
        Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
    }

    pub(crate) fn start_main_response_links(
        &mut self,
        links: &crate::response_links::DocumentResponseLinks,
    ) -> Result<()> {
        if links.is_empty() {
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
