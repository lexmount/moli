use std::{pin::pin, sync::Arc};

use anyhow::{Result, anyhow, bail};

use super::{ScriptVm, inspector::PageInspectorSessionTarget};

impl ScriptVm {
    pub(crate) fn blob_uuid_for_runtime_object_id(
        &mut self,
        inspector_session_id: Option<&str>,
        object_id: &str,
    ) -> Result<String> {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_runtime.context;
        let page_inspector = &self.page_inspector;
        let renderer_document_isolate = self.renderer_document_isolate.clone();
        renderer_document_isolate.with_entered_renderer_document_isolate_and_inspector_mut(
            |isolate, inspector| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                page_inspector.with_session_and_outbound(
                    inspector,
                    PageInspectorSessionTarget::Frontend(inspector_session_id),
                    |session, _, _| {
                        let (value, context, _object_group) = session
                            .unwrap_object(
                                scope,
                                v8::inspector::StringView::from(object_id.as_bytes()),
                            )
                            .map_err(|error| {
                                let message = error
                                    .as_ref()
                                    .map(|error| format!("{}", error.string()))
                                    .filter(|message| !message.is_empty())
                                    .unwrap_or_else(|| "Invalid remote object id".to_owned());
                                anyhow!(message)
                            })?;
                        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
                            bail!("Object id doesn't reference a Blob");
                        };
                        let scope = &mut v8::ContextScope::new(scope, context);
                        crate::blob::blob_uuid_from_object(scope, object)
                            .ok_or_else(|| anyhow!("Object id doesn't reference a Blob"))
                    },
                )
            },
        )
    }

    pub(crate) fn blob_bytes_for_uuid(&self, uuid: &str) -> Option<Arc<[u8]>> {
        let partition_id = self
            ._context_host
            .borrow()
            .browser_context_runtime()
            .storage_partition_identity();
        crate::blob::blob_bytes_for_uuid(&partition_id, uuid)
    }
}
