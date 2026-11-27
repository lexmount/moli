use url::Url;

use crate::native_bridge::WindowDocumentOwner;

/// Captured security authority plus the live base used only for URL resolution.
#[derive(Clone, Debug)]
pub(crate) struct SubresourceRequestEnvironment {
    pub(crate) document_url: Url,
    pub(crate) base_url: Url,
    pub(crate) request_origin: moli_url::WebOrigin,
    pub(crate) frame_id: Option<String>,
}

/// Request settings captured for one exact committed Document.
#[derive(Clone, Debug)]
pub(crate) struct DocumentFetchContext {
    owner: WindowDocumentOwner,
    document_url: Url,
    base_url: Url,
    origin: Box<str>,
    sandboxed_origin: bool,
}

impl DocumentFetchContext {
    pub(crate) fn new(
        owner: WindowDocumentOwner,
        document_url: Url,
        base_url: Url,
        origin: impl Into<Box<str>>,
    ) -> Self {
        Self {
            owner,
            document_url,
            base_url,
            origin: origin.into(),
            sandboxed_origin: false,
        }
    }

    pub(crate) fn with_sandboxed_origin(mut self, sandboxed: bool) -> Self {
        self.sandboxed_origin = sandboxed;
        self
    }

    pub(super) fn has_sandboxed_origin(&self) -> bool {
        self.sandboxed_origin
    }

    pub(crate) fn owner(&self) -> WindowDocumentOwner {
        self.owner
    }

    pub(crate) fn document_url(&self) -> &Url {
        &self.document_url
    }

    pub(crate) fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn request_origin(&self) -> moli_url::WebOrigin {
        moli_url::WebOrigin::from_serialized(&self.origin)
    }

    pub(crate) fn subresource_environment(
        &self,
        base_url: Url,
        frame_id: Option<String>,
    ) -> SubresourceRequestEnvironment {
        SubresourceRequestEnvironment {
            document_url: self.document_url.clone(),
            base_url,
            request_origin: self.request_origin(),
            frame_id,
        }
    }
}
