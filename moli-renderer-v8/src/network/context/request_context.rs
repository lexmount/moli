use std::sync::Arc;
use url::Url;

use crate::native_bridge::WindowDocumentOwner;

/// A script request's origin plus the identity needed to recognize its own
/// opaque-origin Blob URLs. Network origin checks still use WebOrigin.
#[derive(Clone, Debug)]
pub(crate) struct ScriptFetchOrigin {
    origin: moli_url::WebOrigin,
    blob_url_access_key: Option<Arc<crate::blob::ObjectUrlAccessKey>>,
}

impl From<moli_url::WebOrigin> for ScriptFetchOrigin {
    fn from(origin: moli_url::WebOrigin) -> Self {
        Self {
            origin,
            blob_url_access_key: None,
        }
    }
}

impl ScriptFetchOrigin {
    pub(crate) fn network_origin(&self) -> &moli_url::WebOrigin {
        &self.origin
    }

    pub(crate) fn blob_url_is_same_origin(
        &self,
        url: &Url,
        creator_access_key: Option<&crate::blob::ObjectUrlAccessKey>,
    ) -> bool {
        self.origin.same_origin(&moli_url::WebOrigin::from_url(url))
            || self
                .blob_url_access_key
                .as_deref()
                .zip(creator_access_key)
                .is_some_and(|(request_key, creator_key)| request_key == creator_key)
    }
}

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
    blob_url_access_key: Option<Arc<crate::blob::ObjectUrlAccessKey>>,
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
            blob_url_access_key: None,
        }
    }

    pub(crate) fn with_blob_url_access_key(
        mut self,
        partition: crate::runtime::RendererStoragePartitionIdentity,
        storage_key: moli_storage_key::MoliStorageKey,
    ) -> Self {
        self.blob_url_access_key = Some(Arc::new(crate::blob::ObjectUrlAccessKey::new(
            partition,
            storage_key,
        )));
        self
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

    pub(crate) fn script_fetch_origin(&self) -> ScriptFetchOrigin {
        ScriptFetchOrigin {
            origin: self.request_origin(),
            blob_url_access_key: self.blob_url_access_key.clone(),
        }
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
