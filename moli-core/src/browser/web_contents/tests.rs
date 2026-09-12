use super::*;
use crate::{
    browser::{BrowserContextStoragePartitionHandles, NavigationId, NavigationRequestLoadPolicy},
    runtime::{
        CommittedDocumentResourceSource, ExternalRawDocumentBodyStream, PageVmInitStage,
        RendererReplyBoundary,
    },
};
use url::Url;

pub(super) struct BrowserFixture {
    pub(super) contents: WebContents,
    storage: BrowserContextStoragePartitionHandles,
}

impl BrowserFixture {
    pub(super) async fn navigate(&mut self, title: &str) -> crate::browser::DocumentId {
        let navigation = self.contents.navigation.start_document_navigation();
        self.complete_navigation(navigation, title).await
    }

    pub(super) async fn complete_navigation(
        &mut self,
        navigation: NavigationId,
        title: &str,
    ) -> crate::browser::DocumentId {
        let response = prepare(&mut self.start(navigation).unwrap(), title)
            .await
            .unwrap();
        let built = self
            .materialization(navigation, response)
            .unwrap()
            .materialize()
            .await
            .unwrap();
        let committed = self
            .contents
            .commit_document_navigation(built.page)
            .unwrap();
        if let Some(continuation) = committed.post_response_continuation {
            continuation.release();
        }
        committed.retirement.close().await;
        committed.document
    }

    pub(super) async fn evaluate(&mut self, expression: &str) -> serde_json::Value {
        self.contents
            .main_frame
            .current_document
            .as_mut()
            .unwrap()
            .page
            .evaluate_runtime_expression_by_value_async(expression)
            .await
            .unwrap()["value"]
            .clone()
    }

    pub(super) fn new() -> Self {
        let mut contents = WebContents::default();
        contents.install_navigation_engine(NavigationEngine::new());
        Self {
            contents,
            storage: BrowserContextStoragePartitionHandles::memory(),
        }
    }

    pub(super) fn inherited(&self) -> InheritedDocumentPolicy {
        InheritedDocumentPolicy {
            fetch_config: Default::default(),
            extra_headers: Vec::new(),
            emulation: Default::default(),
            permissions: Vec::new(),
            storage: self.storage.clone(),
        }
    }

    pub(super) fn start(
        &mut self,
        navigation: NavigationId,
    ) -> Result<AdmittedNavigationLoad, String> {
        self.contents.start_navigation_load(
            navigation,
            NavigationRequestLoadPolicy::DocumentInitiated,
            self.inherited(),
        )
    }

    pub(super) fn materialization(
        &mut self,
        navigation: NavigationId,
        response: PreparedNavigationResponse,
    ) -> Result<super::AdmittedDocumentMaterialization, String> {
        self.contents.start_document_materialization(
            navigation,
            response,
            DocumentNavigationDestination::Document {
                url: Url::parse("https://navigation.example/").unwrap(),
                security_origin: "https://navigation.example".into(),
                secure_context_type: "Secure".into(),
            },
            self.inherited(),
            true,
        )
    }
}

pub(super) async fn prepare(
    load: &mut AdmittedNavigationLoad,
    title: &str,
) -> anyhow::Result<PreparedNavigationResponse> {
    let url = Url::parse("https://navigation.example/").unwrap();
    load.prepare_document_response_async(
        url.clone(),
        url,
        false,
        0,
        200,
        vec![("content-type".into(), "text/html".into())],
        ExternalRawDocumentBodyStream::from_bytes(format!("<title>{title}</title>").into_bytes()),
        PageVmInitStage::DomContentLoaded,
        RendererReplyBoundary::Stage,
        CommittedDocumentResourceSource::Synthetic,
        None,
    )
    .await
}
