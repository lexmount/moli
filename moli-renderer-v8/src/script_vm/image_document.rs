use super::ScriptVm;
use crate::{dom::native::NativeNodeId, native_bridge::NavigationImageDocumentLoad};

impl ScriptVm {
    pub(crate) fn begin_navigation_image_document_load(
        &mut self,
        image: NativeNodeId,
    ) -> anyhow::Result<NavigationImageDocumentLoad> {
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *host_ptr }.begin_navigation_image_document_load(scope, image))
        })
    }

    pub(crate) fn finish_navigation_image_document_load(
        &mut self,
        load: NavigationImageDocumentLoad,
        headers: &[(String, Vec<u8>)],
        bytes: Vec<u8>,
    ) -> anyhow::Result<()> {
        self.with_default_context_scope(|_, host_ptr| {
            unsafe { &mut *host_ptr }.finish_navigation_image_document_load(load, headers, bytes);
            Ok(())
        })
    }
}
