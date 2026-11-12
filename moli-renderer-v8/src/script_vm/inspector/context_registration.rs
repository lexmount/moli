use super::{
    context_registry::{DocumentInspectorContextGroupId, DocumentInspectorContextRegistrationId},
    v8_backend::RendererInspectorIsolateBackendHandle,
};
use crate::script_vm::document_isolate::RendererIsolateHandle;
use std::{collections::BTreeMap, pin::pin};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentInspectorContextRegistrationKind {
    Default {
        context_group_id: DocumentInspectorContextGroupId,
    },
    NonDefault,
}

struct DocumentInspectorContextRegistration {
    context: Option<v8::Global<v8::Context>>,
    renderer_document_isolate: RendererIsolateHandle,
    isolate_backend: RendererInspectorIsolateBackendHandle,
    id: DocumentInspectorContextRegistrationId,
    kind: DocumentInspectorContextRegistrationKind,
}

impl DocumentInspectorContextRegistration {
    fn new(
        renderer_document_isolate: RendererIsolateHandle,
        isolate_backend: RendererInspectorIsolateBackendHandle,
        context: v8::Global<v8::Context>,
        kind: DocumentInspectorContextRegistrationKind,
    ) -> Self {
        Self {
            context: Some(context),
            renderer_document_isolate,
            isolate_backend,
            id: DocumentInspectorContextRegistrationId::next(),
            kind,
        }
    }

    fn disarm_without_context_destroyed(mut self) {
        // Re-reporting the same live V8 context transfers the eventual
        // destruction obligation to its replacement registration.
        self.context.take();
    }
}

impl Drop for DocumentInspectorContextRegistration {
    fn drop(&mut self) {
        let Some(context) = self.context.take() else {
            return;
        };
        let isolate_backend = self.isolate_backend.clone();
        let id = self.id;
        let kind = self.kind;
        self.renderer_document_isolate
            .with_renderer_document_isolate_and_inspector_mut(move |isolate, backend| {
                isolate_backend.assert_matches(backend);
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let local_context = v8::Local::new(scope, &context);
                match kind {
                    DocumentInspectorContextRegistrationKind::Default { context_group_id } => {
                        backend.default_context_destroyed(context_group_id, id, local_context);
                    }
                    DocumentInspectorContextRegistrationKind::NonDefault => {
                        backend.context_destroyed(local_context);
                    }
                }
                drop(context);
            });
    }
}

#[derive(Default)]
pub(super) struct DocumentInspectorContextRegistrations {
    registrations:
        BTreeMap<DocumentInspectorContextRegistrationId, DocumentInspectorContextRegistration>,
    default_registration_id: Option<DocumentInspectorContextRegistrationId>,
}

impl DocumentInspectorContextRegistrations {
    pub(super) fn register_default(
        &mut self,
        renderer_document_isolate: RendererIsolateHandle,
        isolate_backend: RendererInspectorIsolateBackendHandle,
        context_group_id: DocumentInspectorContextGroupId,
        context: v8::Global<v8::Context>,
    ) -> DocumentInspectorContextRegistrationId {
        assert!(
            self.default_registration_id.is_none(),
            "document Inspector binding registered more than one default context"
        );
        let registration = DocumentInspectorContextRegistration::new(
            renderer_document_isolate,
            isolate_backend,
            context,
            DocumentInspectorContextRegistrationKind::Default { context_group_id },
        );
        let id = registration.id;
        assert!(
            self.registrations.insert(id, registration).is_none(),
            "document Inspector context registration IDs must be unique"
        );
        self.default_registration_id = Some(id);
        id
    }

    pub(super) fn register_non_default(
        &mut self,
        renderer_document_isolate: RendererIsolateHandle,
        isolate_backend: RendererInspectorIsolateBackendHandle,
        context: v8::Global<v8::Context>,
        replaced_registration_id: Option<DocumentInspectorContextRegistrationId>,
    ) -> DocumentInspectorContextRegistrationId {
        if let Some(replaced_registration_id) = replaced_registration_id {
            let replaced = self
                .registrations
                .remove(&replaced_registration_id)
                .expect("replacement Inspector context registration must still be document-owned");
            assert!(
                replaced.kind == DocumentInspectorContextRegistrationKind::NonDefault,
                "non-default context replacement cannot disarm a default registration"
            );
            replaced.disarm_without_context_destroyed();
        }
        let registration = DocumentInspectorContextRegistration::new(
            renderer_document_isolate,
            isolate_backend,
            context,
            DocumentInspectorContextRegistrationKind::NonDefault,
        );
        let id = registration.id;
        assert!(
            self.registrations.insert(id, registration).is_none(),
            "document Inspector context registration IDs must be unique"
        );
        id
    }

    pub(super) fn default_registration_id(&self) -> Option<DocumentInspectorContextRegistrationId> {
        self.default_registration_id
    }

    pub(super) fn destroy(&mut self, id: DocumentInspectorContextRegistrationId) -> bool {
        let Some(registration) = self.registrations.remove(&id) else {
            return false;
        };
        if self.default_registration_id == Some(id) {
            self.default_registration_id = None;
        }
        drop(registration);
        true
    }

    pub(super) fn destroy_all(&mut self) {
        self.default_registration_id = None;
        self.registrations.clear();
    }

    pub(super) fn len(&self) -> usize {
        self.registrations.len()
    }
}
