use super::{
    BrowserContext,
    document_commands::{
        CompletedAppManifestLoadPreparation, CompletedAppManifestPublication,
        CompletedDocumentBlobRead, CompletedNetworkResourceLoadPreparation,
        PendingAppManifestLoadPreparation, PendingAppManifestPublication, PendingDocumentBlobRead,
        PendingNetworkResourceLoadPreparation,
    },
};
use crate::browser::DocumentHandle;
use crate::page::{
    RendererAppManifestLoadPreparation, RendererAppManifestLoadPublication,
    RendererCommandTurnOutput, RendererNetworkResourceLoadPreparation,
};
use std::sync::Arc;
use url::Url;

pub struct BrowserAppManifestLoadPreparation {
    pub document: DocumentHandle,
    pub preparation: RendererAppManifestLoadPreparation,
}

impl BrowserContext {
    pub fn start_document_blob_read(
        &self,
        document: DocumentHandle,
        uuid: String,
    ) -> Result<PendingDocumentBlobRead, String> {
        let pending = self
            .document(document)?
            .page
            .start_blob_bytes_for_uuid(uuid)
            .map_err(|error| error.to_string())?;
        Ok(PendingDocumentBlobRead::new(document, pending))
    }

    pub fn finish_document_blob_read(
        &mut self,
        completed: CompletedDocumentBlobRead,
    ) -> Result<Option<Arc<[u8]>>, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_blob_bytes_for_uuid(completion?)
            .map_err(|error| error.to_string())
    }

    pub fn start_network_resource_load_preparation(
        &self,
        document: DocumentHandle,
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> Result<PendingNetworkResourceLoadPreparation, String> {
        let pending = self
            .document(document)?
            .page
            .start_prepare_network_resource_load(frame_id, url, disable_cache, include_credentials)
            .map_err(|error| error.to_string())?;
        Ok(PendingNetworkResourceLoadPreparation::new(
            document, pending,
        ))
    }

    pub fn finish_network_resource_load_preparation(
        &mut self,
        completed: CompletedNetworkResourceLoadPreparation,
    ) -> Result<RendererNetworkResourceLoadPreparation, String> {
        let (document, completion) = completed.into_parts();
        let page = self.document_mut(document).map_err(|error| {
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

    pub fn start_app_manifest_load_preparation(
        &self,
        document: DocumentHandle,
    ) -> Result<PendingAppManifestLoadPreparation, String> {
        let pending = self
            .document(document)?
            .page
            .start_prepare_app_manifest_load()
            .map_err(|error| error.to_string())?;
        Ok(PendingAppManifestLoadPreparation::new(document, pending))
    }

    pub fn finish_app_manifest_load_preparation(
        &mut self,
        completed: CompletedAppManifestLoadPreparation,
    ) -> Result<BrowserAppManifestLoadPreparation, String> {
        let (document, completion) = completed.into_parts();
        let preparation = self
            .document_mut(document)?
            .page
            .finish_prepare_app_manifest_load(completion?)
            .map_err(|error| error.to_string())?;
        Ok(BrowserAppManifestLoadPreparation {
            document,
            preparation,
        })
    }

    pub fn start_app_manifest_publication(
        &self,
        document: DocumentHandle,
        publication: RendererAppManifestLoadPublication,
    ) -> Result<PendingAppManifestPublication, String> {
        let pending = self
            .document(document)?
            .page
            .start_publish_app_manifest_load(publication)
            .map_err(|error| error.to_string())?;
        Ok(PendingAppManifestPublication::new(document, pending))
    }

    pub fn finish_app_manifest_publication(
        &mut self,
        completed: CompletedAppManifestPublication,
    ) -> Result<RendererCommandTurnOutput, String> {
        let (document, completion) = completed.into_parts();
        self.document_mut(document)?
            .page
            .finish_publish_app_manifest_load(completion?)
            .map_err(|error| error.to_string())
    }
}
