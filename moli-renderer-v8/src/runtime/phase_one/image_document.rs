use crate::{
    native_bridge::{NavigationImageDocumentCompletion, NavigationImageDocumentLoad},
    navigation_document::IMAGE_DOCUMENT_SHELL,
};

enum ImageDocumentHost {
    WaitingForInput,
    ShellQueued,
    Loading(NavigationImageDocumentLoad),
}

/// Raw response bytes stay with the parkable main-resource continuation. Only
/// the trusted shell enters the parser; its exact created image owns the load.
pub(super) struct StreamingImageDocument {
    headers: Vec<(String, Vec<u8>)>,
    bytes: Vec<u8>,
    host: ImageDocumentHost,
}

impl StreamingImageDocument {
    pub(super) fn new(headers: &[(String, Vec<u8>)]) -> Self {
        Self {
            headers: headers.to_vec(),
            bytes: Vec::new(),
            host: ImageDocumentHost::WaitingForInput,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) -> Option<String> {
        self.bytes.extend_from_slice(chunk);
        if chunk.is_empty() {
            return None;
        }
        self.queue_shell()
    }

    pub(super) fn queue_shell(&mut self) -> Option<String> {
        if !matches!(self.host, ImageDocumentHost::WaitingForInput) {
            return None;
        }
        self.host = ImageDocumentHost::ShellQueued;
        Some(IMAGE_DOCUMENT_SHELL.to_owned())
    }

    pub(super) fn needs_host(&self) -> bool {
        matches!(self.host, ImageDocumentHost::ShellQueued)
    }

    pub(super) fn bind(&mut self, load: NavigationImageDocumentLoad) {
        assert!(self.needs_host());
        self.host = ImageDocumentHost::Loading(load);
    }

    pub(super) fn into_completion(self) -> Option<NavigationImageDocumentCompletion> {
        let ImageDocumentHost::Loading(load) = self.host else {
            return None;
        };
        Some(NavigationImageDocumentCompletion {
            load,
            headers: self.headers,
            bytes: self.bytes,
        })
    }
}
