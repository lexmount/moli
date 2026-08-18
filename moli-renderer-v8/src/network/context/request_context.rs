use url::Url;

use crate::native_bridge::WindowDocumentOwner;

/// Request settings captured for one exact committed Document.
#[derive(Clone, Debug)]
pub(crate) struct DocumentFetchContext {
    owner: WindowDocumentOwner,
    document_url: Url,
    base_url: Url,
    origin: Box<str>,
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
        }
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
}
