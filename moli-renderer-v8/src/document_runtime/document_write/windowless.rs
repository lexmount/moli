use super::*;
use std::rc::Rc;

pub(in crate::document_runtime) struct WindowlessDocumentParserState {
    stream: Option<DocumentStream>,
    token: Rc<()>,
}

impl std::fmt::Debug for WindowlessDocumentParserState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowlessDocumentParserState")
            .field("has_stream", &self.stream.is_some())
            .finish_non_exhaustive()
    }
}

impl DocumentRuntime {
    pub(crate) fn has_windowless_document_parser(&self, document: DomHandle) -> bool {
        self.windowless_document_parsers
            .get(&document)
            .is_some_and(|state| state.stream.is_some())
    }

    pub(crate) fn start_windowless_document_parser(&mut self, document: DomHandle) -> Option<()> {
        let source = self.dom_host().node(document)?.as_document()?;
        let stream = HtmlParser::with_scripting_enabled(false)
            .start_live_document_root_with_declarative_shadow_roots(
                source.url().clone(),
                document,
                source.allow_declarative_shadow_roots(),
            );
        self.dom_host_mut().set_document_quirks_mode_for_handle(
            document,
            selectors::matching::QuirksMode::NoQuirks,
        );
        self.dom_host_mut()
            .set_document_scripting_enabled_for_handle(document, false);
        self.dom_host_mut()
            .set_document_ready_state_for_handle(document, DocumentReadyState::Loading);
        self.windowless_document_parsers.insert(
            document,
            WindowlessDocumentParserState {
                stream: Some(stream),
                token: Rc::new(()),
            },
        );
        Some(())
    }

    pub(crate) fn write_windowless_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document: DomHandle,
        html: &str,
    ) -> Option<()> {
        let state = self.windowless_document_parsers.get_mut(&document)?;
        let stream = state.stream.take()?;
        let token = state.token.clone();
        self.with_dom_host_parse_step(|runtime| {
            let mut owner = DocumentWriteParserMutationOwner {
                runtime,
                scope,
                host_ptr,
                target: DocumentWriteParserMutationTarget::WindowlessDocument {
                    owner_document: document,
                },
            };
            stream.feed_with_runtime_dom_consumer(html, &mut owner);
        });
        custom_elements::apply_parser_created_null_registry_associations(
            host_ptr,
            &stream.take_parser_stream_null_custom_element_registry_elements(),
        );
        if self.windowless_document_parser_is_current(document, &token) {
            self.windowless_document_parsers.get_mut(&document)?.stream = Some(stream);
        }
        Some(())
    }

    pub(crate) fn finish_windowless_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document: DomHandle,
    ) -> Option<Rc<()>> {
        let state = self.windowless_document_parsers.get_mut(&document)?;
        let stream = state.stream.take()?;
        let token = state.token.clone();
        let finish_signals = self.with_dom_host_parse_step(|runtime| {
            let mut owner = DocumentWriteParserMutationOwner {
                runtime,
                scope,
                host_ptr,
                target: DocumentWriteParserMutationTarget::WindowlessDocument {
                    owner_document: document,
                },
            };
            stream.finish_with_runtime_dom_consumer(&mut owner)
        });
        custom_elements::apply_parser_created_null_registry_associations(
            host_ptr,
            &finish_signals.parser_created_null_registry_elements,
        );
        Some(token)
    }

    pub(crate) fn windowless_document_parser_is_current(
        &self,
        document: DomHandle,
        token: &Rc<()>,
    ) -> bool {
        self.windowless_document_parsers
            .get(&document)
            .is_some_and(|state| Rc::ptr_eq(&state.token, token))
    }

    pub(crate) fn release_finished_windowless_document_parser(
        &mut self,
        document: DomHandle,
        token: &Rc<()>,
    ) {
        if self.windowless_document_parser_is_current(document, token) {
            self.windowless_document_parsers.remove(&document);
        }
    }
}
