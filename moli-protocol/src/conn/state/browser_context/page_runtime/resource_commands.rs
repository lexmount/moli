use super::{
    BrowserContext,
    document_commands::{
        CompletedAppManifestLoadPreparation, CompletedAppManifestPublication,
        CompletedDocumentBlobRead, CompletedNetworkResourceLoadPreparation,
        PendingAppManifestLoadPreparation, PendingAppManifestPublication, PendingDocumentBlobRead,
        PendingNetworkResourceLoadPreparation,
    },
};
use moli_core::browser::DocumentHandle;
use moli_core::page::{
    RendererAppManifestLoadPreparation, RendererAppManifestLoadPublication,
    RendererCommandTurnOutput, RendererNetworkResourceLoadPreparation,
};
use std::sync::Arc;
use url::Url;

pub(crate) struct BrowserAppManifestLoadPreparation {
    pub(crate) document: DocumentHandle,
    pub(crate) preparation: RendererAppManifestLoadPreparation,
}

impl BrowserContext {
    pub(crate) fn start_document_blob_read(
        &self,
        document: DocumentHandle,
        uuid: String,
    ) -> Result<PendingDocumentBlobRead, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_blob_bytes_for_uuid(uuid)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentBlobRead::new(document, pending))
    }

    pub(crate) fn finish_document_blob_read(
        &mut self,
        completed: CompletedDocumentBlobRead,
    ) -> Result<Option<Arc<[u8]>>, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_blob_bytes_for_uuid(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_network_resource_load_preparation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> Result<PendingNetworkResourceLoadPreparation, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_prepare_network_resource_load(frame_id, url, disable_cache, include_credentials)
            .map_err(|error| error.to_string())?;
        Ok(PendingNetworkResourceLoadPreparation::new(
            document, pending,
        ))
    }

    pub(crate) fn finish_network_resource_load_preparation(
        &mut self,
        completed: CompletedNetworkResourceLoadPreparation,
    ) -> Result<RendererNetworkResourceLoadPreparation, String> {
        let (document, completion) = completed.into_parts();
        let page = self.physical.document_mut(document).map_err(|error| {
            if error == "Document changed" {
                "Document changed while preparing the network resource load".to_owned()
            } else {
                error
            }
        })?;
        page.page
            .finish_prepare_network_resource_load(completion?)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_app_manifest_load_preparation(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingAppManifestLoadPreparation, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_prepare_app_manifest_load()
            .map_err(|error| error.to_string())?;
        Ok(PendingAppManifestLoadPreparation::new(document, pending))
    }

    pub(crate) fn finish_app_manifest_load_preparation(
        &mut self,
        completed: CompletedAppManifestLoadPreparation,
    ) -> Result<BrowserAppManifestLoadPreparation, String> {
        let (document, completion) = completed.into_parts();
        let preparation = self
            .physical
            .document_mut(document)?
            .page
            .finish_prepare_app_manifest_load(completion?)
            .map_err(|error| error.to_string())?;
        Ok(BrowserAppManifestLoadPreparation {
            document,
            preparation,
        })
    }

    pub(crate) fn start_app_manifest_publication(
        &self,
        document: DocumentHandle,
        publication: RendererAppManifestLoadPublication,
    ) -> Result<PendingAppManifestPublication, String> {
        let pending = self
            .physical
            .document(document)?
            .page
            .start_publish_app_manifest_load(publication)
            .map_err(|error| error.to_string())?;
        Ok(PendingAppManifestPublication::new(document, pending))
    }

    pub(crate) fn finish_app_manifest_publication(
        &mut self,
        completed: CompletedAppManifestPublication,
    ) -> Result<RendererCommandTurnOutput, String> {
        let (document, completion) = completed.into_parts();
        self.physical
            .document_mut(document)?
            .page
            .finish_publish_app_manifest_load(completion?)
            .map_err(|error| error.to_string())
    }
}
