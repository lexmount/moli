//! Font resource processing belongs to document lifecycle work. Output/layout
//! operations consume registered fonts without advancing the loading engine.

use super::*;

impl ScriptVm {
    pub(super) fn settle_loaded_document_font_set(&mut self, document: DomHandle) {
        // A document without a script realm cannot have an observed font
        // promise. Native load completion must preserve lazy realm creation.
        if let Err(error) = self.with_current_main_world_scope(|scope, host_ptr| {
            crate::context_bootstrap::settle_document_font_face_set_ready(
                scope, host_ptr, document,
            );
            Ok(())
        }) {
            tracing::warn!(%error, "failed to settle document font readiness after load");
        }
    }

    pub(super) fn reconcile_document_font_resources(&mut self) {
        if self.current_main_world().is_none() {
            let host = self._context_host.borrow();
            let fonts_enabled =
                host.current_main_document_resource_loader()
                    .is_some_and(|loader| {
                        loader.request_client().optional_resource_fetch_enabled(
                            crate::types::SubresourceResourceType::Font,
                        )
                    });
            // No JS FontFace requests or CSSOM projections exist before the first realm.
            // Native CSS fonts still enter the existing resource loading path on demand.
            if !fonts_enabled
                || !host
                    .active_layout_document_handles()
                    .into_iter()
                    .any(|document| {
                        host.dom_host()
                            .dom()
                            .document_element_handle_for_document(document)
                            .and_then(|root| {
                                DocumentRuntime::current_stylesheet_resources(&host, root)
                            })
                            .is_some_and(|resources| !resources.web_fonts().is_empty())
                    })
            {
                return;
            }
        }
        if let Err(error) = self.with_default_context_scope(|scope, host_ptr| {
            DocumentRuntime::reconcile_document_web_fonts_in_current_scope(scope, host_ptr);
            Ok(())
        }) {
            tracing::warn!(%error, "failed to reconcile current document font resources");
        }
    }

    pub(super) fn complete_document_web_font(
        &mut self,
        binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
        terminal: crate::css_resource_urls::CompletedStylesheetWebFont,
    ) {
        let Some(document) = self
            ._context_host
            .borrow()
            .stylesheet_web_font_document(binding)
        else {
            return;
        };
        let retry = DocumentRuntime::complete_document_web_font(
            &self._context_host.borrow(),
            document,
            terminal,
        );
        if let Some(request) = retry {
            let binding = self
                ._context_host
                .borrow_mut()
                .accept_current_stylesheet_subresource_load_delay_for_document(document);
            if let Some(binding) = binding
                && let Err(error) = self.with_default_context_scope(move |scope, host_ptr| {
                    DocumentRuntime::start_stylesheet_subresource_requests_in_current_scope(
                        scope,
                        host_ptr,
                        vec![(binding, request)],
                    );
                    Ok(())
                })
            {
                self.record_runtime_warning(format_args!(
                    "failed to enter stylesheet subresource request scope: {error}"
                ));
            }
        }
        if let Err(error) = self.with_default_context_scope(|scope, host_ptr| {
            crate::context_bootstrap::publish_font_face_load_changes(scope);
            crate::context_bootstrap::settle_document_font_face_set_ready(
                scope, host_ptr, document,
            );
            Ok(())
        }) {
            tracing::warn!(%error, "failed to settle current document font readiness");
        }
    }

    #[cfg(test)]
    pub(crate) fn document_web_font_counts_for_test(&mut self) -> (usize, usize, usize) {
        self._context_host
            .borrow()
            .document_web_font_counts_for_test()
    }
}
