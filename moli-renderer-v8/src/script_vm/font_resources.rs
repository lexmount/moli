//! Font resource processing belongs to document lifecycle work. Output/layout
//! operations consume registered fonts without advancing the loading engine.

use super::*;

impl ScriptVm {
    pub(super) fn reconcile_document_font_resources(&mut self) {
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
