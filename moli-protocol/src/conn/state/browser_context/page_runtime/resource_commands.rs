use super::BrowserContext;
use moli_core::page::{
    CompletedPageCommand, PendingPageCommand, RendererAppManifestLoadPreparation,
    RendererAppManifestLoadPublication, RendererCommandTurnOutput,
    RendererNetworkResourceLoadPreparation,
};
use std::sync::Arc;
use url::Url;

impl BrowserContext {
    pub(crate) fn start_target_blob_read(
        &self,
        target_id: &str,
        uuid: String,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_blob_bytes_for_uuid(uuid)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_blob_read(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<Option<Arc<[u8]>>, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_blob_bytes_for_uuid(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_target_network_resource_load(
        &self,
        target_id: &str,
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_prepare_network_resource_load(frame_id, url, disable_cache, include_credentials)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_network_resource_load_preparation(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererNetworkResourceLoadPreparation, String> {
        let page = self
            .loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?;
        if !completion.is_from_page(page) {
            return Err("Document changed while preparing the network resource load".to_owned());
        }
        page.finish_prepare_network_resource_load(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_target_app_manifest_load(
        &self,
        target_id: &str,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_prepare_app_manifest_load()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_app_manifest_load_preparation(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererAppManifestLoadPreparation, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_prepare_app_manifest_load(completion)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_target_app_manifest_publication(
        &self,
        target_id: &str,
        publication: RendererAppManifestLoadPublication,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_publish_app_manifest_load(publication)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_app_manifest_publication(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<RendererCommandTurnOutput, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_publish_app_manifest_load(completion)
            .map_err(|error| error.to_string())
    }
}
