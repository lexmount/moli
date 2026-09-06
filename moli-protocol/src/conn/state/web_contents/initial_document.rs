use moli_core::{
    browser::{
        DocumentId, DocumentLifecycle, MainFrameSlotId, RendererPageResidenceIdentity,
        WebContentsId,
    },
    runtime::{BuiltDocumentPage, PendingPreparedDocumentPage, PreparedDocumentPagePolicy},
};
use tokio::sync::watch;

#[cfg(test)]
mod tests;

use super::{CommittedDocumentLifecycle, DocumentHost, InheritedDocumentPolicy, WebContents};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitialDocumentBuildKey {
    web_contents: WebContentsId,
    frame_slot: MainFrameSlotId,
    document: DocumentId,
    renderer: RendererPageResidenceIdentity,
}

impl InitialDocumentBuildKey {
    pub(in crate::conn) fn web_contents(&self) -> WebContentsId {
        self.web_contents
    }
    pub(in crate::conn) fn document(&self) -> DocumentId {
        self.document
    }
    pub(in crate::conn) fn renderer(&self) -> RendererPageResidenceIdentity {
        self.renderer
    }
}

/// A build has two owning guards: the Browser state and its move-owned work.
/// Dropping either finishes the same one-shot observation without borrowing the
/// registry. A stale worker can never complete a later build's waiters.
#[derive(Debug)]
pub(in crate::conn::state) struct InitialDocumentBuildCompletion {
    sender: watch::Sender<Option<Result<(), String>>>,
}

impl InitialDocumentBuildCompletion {
    fn new() -> Self {
        Self {
            sender: watch::channel(None).0,
        }
    }
    fn work_guard(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
    fn finish(&self, result: Result<(), String>) {
        self.sender.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(result);
            true
        });
    }
    pub(in crate::conn::state) fn pending(&self) -> bool {
        self.sender.borrow().is_none()
    }
    fn waiter(&self) -> InitialDocumentPageBuildWaiter {
        InitialDocumentPageBuildWaiter {
            receiver: self.sender.subscribe(),
        }
    }
}

impl Drop for InitialDocumentBuildCompletion {
    fn drop(&mut self) {
        self.finish(Err("InitialDocumentPageBuildCancelled".into()));
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InitialDocumentPageBuildWaiter {
    receiver: watch::Receiver<Option<Result<(), String>>>,
}

impl InitialDocumentPageBuildWaiter {
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result;
            }
            self.receiver
                .changed()
                .await
                .map_err(|_| "InitialDocumentPageBuildCancelled".to_owned())?;
        }
    }
}

#[derive(Debug)]
pub(in crate::conn::state) struct InitialDocumentBuildState {
    pub(in crate::conn::state) key: InitialDocumentBuildKey,
    pub(in crate::conn::state) completion: InitialDocumentBuildCompletion,
}

impl InitialDocumentBuildState {
    pub(in crate::conn::state) fn waiter(&self) -> InitialDocumentPageBuildWaiter {
        self.completion.waiter()
    }
}

pub(in crate::conn) struct AdmittedInitialDocumentBuild {
    key: InitialDocumentBuildKey,
    completion: InitialDocumentBuildCompletion,
    page: PendingPreparedDocumentPage,
    policy: PreparedDocumentPagePolicy,
}

pub(in crate::conn) enum InitialDocumentAdmission {
    Present,
    Join(InitialDocumentPageBuildWaiter),
    Build(Box<AdmittedInitialDocumentBuild>),
}

impl AdmittedInitialDocumentBuild {
    pub(in crate::conn) fn start_preparation(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.completion.pending(),
            "InitialDocumentPageBuildCancelled"
        );
        self.page.start_preparation()
    }
    pub(in crate::conn) fn key(&self) -> InitialDocumentBuildKey {
        self.key
    }
    pub(in crate::conn) fn inspection_endpoint(
        &self,
    ) -> moli_renderer_v8::RendererPreparedDocumentInspectionEndpoint {
        self.page.inspection_configuration_endpoint()
    }
    pub(in crate::conn) async fn materialize(self) -> anyhow::Result<BuiltInitialDocument> {
        let Self {
            key,
            completion,
            page,
            policy,
        } = self;
        anyhow::ensure!(completion.pending(), "InitialDocumentPageBuildCancelled");
        let built = match page.materialize(policy).await {
            Ok(built) => built,
            Err(error) => {
                completion.finish(Err(error.to_string()));
                return Err(error);
            }
        };
        anyhow::ensure!(completion.pending(), "InitialDocumentPageBuildCancelled");
        anyhow::ensure!(
            key.renderer == RendererPageResidenceIdentity::from_page(&built.page),
            "initial document renderer identity changed"
        );
        let lifecycle = DocumentLifecycle::from_creation_artifacts(&built.page_creation_artifacts)
            .ok_or_else(|| anyhow::anyhow!("inconsistent initial document lifecycle"))?;
        Ok(BuiltInitialDocument {
            key,
            completion,
            built,
            lifecycle,
        })
    }
}

pub(crate) struct BuiltInitialDocument {
    key: InitialDocumentBuildKey,
    completion: InitialDocumentBuildCompletion,
    built: BuiltDocumentPage,
    lifecycle: DocumentLifecycle,
}

impl BuiltInitialDocument {
    pub(in crate::conn) fn key(&self) -> InitialDocumentBuildKey {
        self.key
    }
    pub(in crate::conn) async fn retire(self) {
        let _ = self.built.page.close_async().await;
    }
}

pub(in crate::conn) struct CommittedInitialDocument {
    pub(in crate::conn) key: InitialDocumentBuildKey,
    pub(in crate::conn) lifecycle: CommittedDocumentLifecycle,
    pub(in crate::conn) diagnostics: moli_core::page::RendererPageCreationDiagnostics,
    pub(in crate::conn) inspection_endpoint: moli_renderer_v8::RendererInspectionEndpoint,
}

impl WebContents {
    pub(in crate::conn::state) fn start_initial_document_build(
        &mut self,
        inherited: InheritedDocumentPolicy,
    ) -> Result<InitialDocumentAdmission, String> {
        if self.main_frame.current_document.is_some() {
            return Ok(InitialDocumentAdmission::Present);
        }
        if let Some(build) = self
            .navigation
            .initial_document_build()
            .filter(|build| build.completion.pending())
        {
            return Ok(InitialDocumentAdmission::Join(build.waiter()));
        }
        if !self
            .navigation
            .can_install_current_initial_empty_document_page()
        {
            return Err("initial document is not available for construction".into());
        }
        let url = self
            .navigation
            .initial_empty_document_url_if_current()
            .and_then(|url| url::Url::parse(url).ok())
            .unwrap_or_else(|| url::Url::parse("about:blank").expect("valid initial URL"));
        let storage_key = self
            .navigation
            .initial_empty_document_storage_key_if_current()
            .cloned();
        let storage = inherited
            .storage
            .page_storage_handles(self.session_storage.store().clone())
            .into_navigation_storage();
        let policy = self.capture_document_policy(inherited, &url)?;
        let page = self
            .navigation_engine
            .as_mut()
            .ok_or("navigation WebContents engine unavailable")?
            .reserve_initial_document(storage, url.clone(), storage_key)
            .map_err(|error| error.to_string())?;
        let key = InitialDocumentBuildKey {
            web_contents: self.id(),
            frame_slot: self.main_frame.id(),
            document: DocumentId::allocate(),
            renderer: page.renderer_residence(),
        };
        let completion = InitialDocumentBuildCompletion::new();
        self.navigation.admit_initial_document_build(
            url.as_str(),
            InitialDocumentBuildState {
                key,
                completion: completion.work_guard(),
            },
        );
        Ok(InitialDocumentAdmission::Build(Box::new(
            AdmittedInitialDocumentBuild {
                key,
                completion,
                page,
                policy,
            },
        )))
    }

    pub(in crate::conn::state) fn commit_initial_document(
        &mut self,
        candidate: BuiltInitialDocument,
    ) -> Result<CommittedInitialDocument, Box<BuiltInitialDocument>> {
        if self.main_frame.current_document.is_some()
            || self.id() != candidate.key.web_contents
            || self.main_frame.id() != candidate.key.frame_slot
            || !self
                .navigation
                .can_install_current_initial_empty_document_page()
            || !candidate.completion.pending()
            || !self
                .navigation
                .initial_document_build()
                .is_some_and(|build| build.key == candidate.key)
        {
            return Err(Box::new(candidate));
        }
        let BuiltInitialDocument {
            key,
            completion,
            built,
            lifecycle,
        } = candidate;
        let BuiltDocumentPage {
            page,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        } = built;
        debug_assert!(
            pending_download.is_none(),
            "an initial empty document cannot be a download"
        );
        let inspection_endpoint = page.renderer_inspection_endpoint();
        // Retain the native guard until the entire transaction is installed.
        // Observers must not see success before document/lifecycle replacement.
        let _build = self.navigation.take_initial_document_build_for_commit();
        let mut document = DocumentHost::new(key.document, page);
        document.lifecycle = lifecycle;
        self.replace_document(Some(document));
        if page_creation_artifacts
            .initial_lifecycle_events
            .iter()
            .any(|event| {
                matches!(event.kind,
            moli_core::page::RendererDocumentLifecycleEventKind::Started { reason:
                moli_core::page::RendererLifecycleStartReason::ExplicitDocumentOpen
                | moli_core::page::RendererLifecycleStartReason::JavascriptDocumentReplacement })
            })
        {
            self.navigation.mark_initial_empty_document_exited();
        }
        completion.finish(Ok(()));
        Ok(CommittedInitialDocument {
            key,
            inspection_endpoint,
            lifecycle: CommittedDocumentLifecycle {
                document: key.document,
                artifacts: page_creation_artifacts,
            },
            diagnostics: page_creation_diagnostics,
        })
    }
}
