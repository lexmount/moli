use tokio::sync::{oneshot, watch};

use super::navigation_driver::on_owner;
use super::{Browser, BrowserContextHandle, BrowserLocalSender};
use crate::browser::{
    NavigationDecision, WebContentsHandle,
    web_contents::{
        AdmittedInitialDocumentBuild, CommittedInitialDocument, InheritedDocumentPolicy,
        InitialDocumentAdmission, InitialDocumentBuildKey, InitialDocumentInspectionClaim,
        InitialDocumentInspectionPhase, InitialDocumentInspectionStage,
        InitialDocumentPageBuildWaiter,
    },
};

pub struct BrowserCommittedInitialDocument {
    pub key: InitialDocumentBuildKey,
    pub snapshot: crate::browser::web_contents::DocumentCommitSnapshot,
    pub diagnostics: crate::page::RendererPageCreationDiagnostics,
}

/// Observation only: dropping this waiter cannot cancel a Browser construction.
pub struct BrowserInitialDocumentWaiter {
    key: InitialDocumentBuildKey,
    completion: InitialDocumentObservation,
}

enum InitialDocumentObservation {
    Started(oneshot::Receiver<Result<BrowserCommittedInitialDocument, String>>),
    Joined(InitialDocumentPageBuildWaiter),
}

impl BrowserInitialDocumentWaiter {
    pub fn key(&self) -> InitialDocumentBuildKey {
        self.key
    }

    pub async fn wait(self) -> Result<Option<BrowserCommittedInitialDocument>, String> {
        match self.completion {
            InitialDocumentObservation::Started(completion) => completion
                .await
                .map_err(|_| "Browser stopped during initial document construction".to_owned())?
                .map(Some),
            InitialDocumentObservation::Joined(waiter) => waiter.wait().await.map(|()| None),
        }
    }
}

struct PendingInspection {
    result: oneshot::Receiver<NavigationDecision>,
    provider: watch::Receiver<()>,
    phase: InitialDocumentInspectionPhase,
}

impl BrowserContextHandle {
    pub fn start_initial_document(
        &self,
        contents: WebContentsHandle,
        inherited: InheritedDocumentPolicy,
    ) -> Result<Option<BrowserInitialDocumentWaiter>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            browser.context(context)?.web_contents(contents)?;
            browser.start_initial_document(contents, inherited)
        })?
    }

    pub fn initial_document_build_key(
        &self,
        contents: WebContentsHandle,
    ) -> Result<Option<InitialDocumentBuildKey>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            Ok(browser
                .context(context)?
                .web_contents(contents)?
                .navigation()
                .initial_document_build()
                .filter(|build| build.completion.pending())
                .map(|build| build.key))
        })?
    }

    pub fn claim_initial_document_inspection(
        &self,
        contents: WebContentsHandle,
        key: InitialDocumentBuildKey,
    ) -> Result<Option<InitialDocumentInspectionClaim>, String> {
        let context = self.id;
        self.browser.execute(move |browser| {
            Ok(browser
                .context_mut(context)?
                .web_contents_mut(contents)?
                .navigation_mut()
                .initial_document_build_mut()
                .filter(|build| build.key == key && build.completion.pending())
                .and_then(|build| build.claim_inspection()))
        })?
    }
}

impl Browser {
    fn publish_initial_document_commit(
        &mut self,
        contents: WebContentsHandle,
        committed: CommittedInitialDocument,
    ) -> BrowserCommittedInitialDocument {
        let CommittedInitialDocument {
            key,
            lifecycle,
            diagnostics,
            inspection_endpoint: _,
        } = committed;
        let document = crate::browser::DocumentHandle::new(contents, key.document());
        let snapshot = self
            .context(contents.context())
            .expect("committed Context")
            .document_commit_snapshot(document)
            .expect("committed Document occurrence");
        self.events.publish_committed(
            lifecycle.browser_sequence,
            crate::browser::BrowserEvent::DocumentCommitted(document),
        );
        self.observe_document_lifecycle(document);
        self.observe_javascript_dialogs(document);
        self.observe_popup_inputs(document);
        BrowserCommittedInitialDocument {
            key,
            snapshot,
            diagnostics,
        }
    }

    pub(super) fn start_initial_document(
        &mut self,
        contents: WebContentsHandle,
        inherited: InheritedDocumentPolicy,
    ) -> Result<Option<BrowserInitialDocumentWaiter>, String> {
        let admission = self
            .context_mut(contents.context())?
            .start_initial_document(contents, inherited)?;
        let build = match admission {
            InitialDocumentAdmission::Present => return Ok(None),
            InitialDocumentAdmission::Join(waiter) => {
                let key = self
                    .context(contents.context())?
                    .web_contents(contents)?
                    .navigation()
                    .initial_document_build()
                    .expect("joined initial construction")
                    .key;
                return Ok(Some(BrowserInitialDocumentWaiter {
                    key,
                    completion: InitialDocumentObservation::Joined(waiter),
                }));
            }
            InitialDocumentAdmission::Build(build) => build,
        };
        let key = build.key();
        let cancelled = self
            .context(contents.context())?
            .web_contents(contents)?
            .navigation()
            .initial_document_build()
            .expect("admitted construction")
            .waiter();
        let inspection = self.begin_initial_document_inspection(
            contents,
            key,
            InitialDocumentInspectionStage::Reserved,
        )?;
        let owner = self.local_sender.clone();
        let (finished, completion) = oneshot::channel();
        tokio::task::spawn_local(async move {
            let result = tokio::select! {
                biased;
                error = async move {
                    match cancelled.wait().await {
                        Err(error) => error,
                        Ok(()) => std::future::pending::<String>().await,
                    }
                } => Err(error),
                result = construct(&owner, contents, *build, inspection) => result,
            };
            if result.is_err() {
                let _ = on_owner(&owner, move |browser| {
                    let navigation = browser
                        .context_mut(contents.context())?
                        .web_contents_mut(contents)?
                        .navigation_mut();
                    if navigation
                        .initial_document_build()
                        .is_some_and(|build| build.key == key)
                    {
                        navigation.cancel_initial_document_build();
                    }
                    browser.events.publish(
                        crate::browser::BrowserEvent::InitialDocumentConstructionFailed {
                            web_contents: contents,
                            key,
                        },
                    );
                    Ok(())
                })
                .await;
            }
            let _ = finished.send(result);
        });
        Ok(Some(BrowserInitialDocumentWaiter {
            key,
            completion: InitialDocumentObservation::Started(completion),
        }))
    }

    fn begin_initial_document_inspection(
        &mut self,
        contents: WebContentsHandle,
        key: InitialDocumentBuildKey,
        stage: InitialDocumentInspectionStage,
    ) -> Result<Option<PendingInspection>, String> {
        let Some(provider) = self
            .document_decision_provider
            .as_ref()
            .filter(|provider| provider.has_changed().is_ok())
            .cloned()
        else {
            return Ok(None);
        };
        let phase = stage.phase();
        let result = self
            .context_mut(contents.context())?
            .web_contents_mut(contents)?
            .navigation_mut()
            .initial_document_build_mut()
            .filter(|build| build.key == key)
            .ok_or("initial document construction retired")?
            .pause_inspection(stage)?;
        self.events.publish(
            crate::browser::BrowserEvent::InitialDocumentAwaitingInspection {
                web_contents: contents,
                key,
            },
        );
        Ok(Some(PendingInspection {
            result,
            provider,
            phase,
        }))
    }
}

async fn await_inspection(
    owner: &BrowserLocalSender,
    contents: WebContentsHandle,
    key: InitialDocumentBuildKey,
    inspection: Option<PendingInspection>,
) -> Result<(), String> {
    let Some(PendingInspection {
        mut result,
        mut provider,
        phase,
    }) = inspection
    else {
        return Ok(());
    };
    let decision = tokio::select! {
        decision = &mut result => decision,
        _ = provider.changed() => {
            on_owner(owner, move |browser| {
                let build = browser.context(contents.context())?.web_contents(contents)?.navigation()
                    .initial_document_build().filter(|build| build.key == key)
                    .ok_or("initial document construction retired")?;
                build.release_inspection(phase);
                Ok(())
            }).await?;
            result.await
        }
    }.map_err(|_| "initial document inspection canceled".to_owned())?;
    if !matches!(decision, NavigationDecision::Continue) {
        return Err("initial document inspection canceled".into());
    }
    on_owner(owner, move |browser| {
        if browser
            .context_mut(contents.context())?
            .web_contents_mut(contents)?
            .navigation_mut()
            .initial_document_build_mut()
            .filter(|build| build.key == key)
            .is_some_and(|build| build.finish_inspection(phase))
        {
            Ok(())
        } else {
            Err("initial document construction retired".into())
        }
    })
    .await
}

async fn construct(
    owner: &BrowserLocalSender,
    contents: WebContentsHandle,
    mut build: AdmittedInitialDocumentBuild,
    inspection: Option<PendingInspection>,
) -> Result<BrowserCommittedInitialDocument, String> {
    let key = build.key();
    await_inspection(owner, contents, key, inspection).await?;
    build
        .start_preparation()
        .map_err(|error| error.to_string())?;
    let endpoint = build.inspection_endpoint();
    let inspection = on_owner(owner, move |browser| {
        browser.begin_initial_document_inspection(
            contents,
            key,
            InitialDocumentInspectionStage::Prepared(endpoint),
        )
    })
    .await?;
    await_inspection(owner, contents, key, inspection).await?;
    let built = build
        .materialize()
        .await
        .map_err(|error| error.to_string())?;
    on_owner(owner, move |browser| {
        let committed = match browser.context_mut(contents.context()) {
            Ok(context) => context.commit_initial_document(built),
            Err(_) => Err(Box::new(built)),
        };
        match committed {
            Ok(committed) => Ok(browser.publish_initial_document_commit(contents, committed)),
            Err(stale) => {
                tokio::task::spawn_local(stale.retire());
                Err("initial document construction superseded".into())
            }
        }
    })
    .await
}
