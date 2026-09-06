use moli_core::{
    browser::{
        DocumentId, DocumentLifecycle, MainFrameSlotId, NavigationId,
        RendererPageResidenceIdentity, WebContentsId,
    },
    page::{Page, RendererPageCommandPostResponseContinuation, RendererPageCreationArtifacts},
    runtime::{BuiltDocumentPage, PreparedDocumentPage, PreparedDocumentPagePolicy},
};
use url::Url;

use super::{DocumentHost, WebContents};

/// Browser-only commit participant in the private migration residence (20/24b).
/// No frontend identity, renderer attachment, arbitrary callback or Page lease.
#[derive(Debug)]
pub(crate) struct PreparedDocumentNavigation {
    identity: DocumentNavigationIdentity,
    page: Page,
    lifecycle: DocumentLifecycle,
    info: CommittedDocumentInfo,
}

/// Admission freezes the Browser objects before the move-owned work can await.
/// Only WebContents can create this identity; Protocol cannot retarget a result.
#[derive(Debug)]
struct DocumentNavigationIdentity {
    web_contents: WebContentsId,
    frame_slot: MainFrameSlotId,
    navigation: NavigationId,
    document: DocumentId,
    cancellation: moli_fetch::FetchCancelHandle,
}

pub(crate) struct DocumentNavigationDestination {
    pub(crate) url: Url,
    pub(crate) security_origin: String,
    pub(crate) secure_context_type: String,
}

/// An admitted operation owns the candidate, not a reusable permission to
/// configure or commit a different renderer reservation.
pub(in crate::conn) struct AdmittedDocumentMaterialization {
    identity: DocumentNavigationIdentity,
    page: PreparedDocumentPage,
    destination: DocumentNavigationDestination,
}

impl AdmittedDocumentMaterialization {
    pub(in crate::conn) async fn materialize(
        self,
        policy: PreparedDocumentPagePolicy,
    ) -> anyhow::Result<BuiltDocumentPage<PreparedDocumentNavigation>> {
        anyhow::ensure!(
            !self.identity.cancellation.is_cancelled(),
            "canceled navigation document candidate"
        );
        let BuiltDocumentPage {
            page,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        } = self.page.materialize(Some(policy)).await?;
        anyhow::ensure!(
            !self.identity.cancellation.is_cancelled(),
            "canceled navigation document candidate"
        );
        let page = PreparedDocumentNavigation::new(
            self.identity,
            page,
            self.destination,
            &page_creation_artifacts,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(BuiltDocumentPage {
            page,
            page_creation_diagnostics,
            page_creation_artifacts,
            pending_download,
        })
    }
}

#[derive(Debug)]
pub(in crate::conn) struct CommittedDocumentInfo {
    pub(in crate::conn) url: Url,
    pub(in crate::conn) title: String,
    pub(in crate::conn) security_origin: String,
    pub(in crate::conn) secure_context_type: String,
}

impl PreparedDocumentNavigation {
    /// Configure the move-owned Browser participant without borrowing its owner.
    /// These are effective values, never frontend registration/session identities.
    async fn apply_document_policy(
        mut self,
        interception: (bool, Option<moli_core::page::SubresourceResourceType>),
        permissions: &[moli_core::page::PermissionOverrideRegistration],
    ) -> anyhow::Result<Self> {
        use anyhow::Context;
        if interception.0 || interception.1.is_some() {
            self.page
                .set_fetch_subresource_interception_async(interception.0, interception.1)
                .await
                .context("failed to restore page fetch interception state")?;
        }
        if !permissions.is_empty() {
            self.page
                .set_permission_overrides_async(permissions)
                .await
                .context("failed to apply page permission overrides")?;
        }
        Ok(self)
    }

    pub(in crate::conn) fn navigation(&self) -> NavigationId {
        self.identity.navigation
    }

    pub(in crate::conn) fn web_contents_id(&self) -> WebContentsId {
        self.identity.web_contents
    }

    fn new(
        identity: DocumentNavigationIdentity,
        page: Page,
        destination: DocumentNavigationDestination,
        artifacts: &RendererPageCreationArtifacts,
    ) -> Result<Self, &'static str> {
        if artifacts.lifecycle_snapshot.frame.page_id != page.renderer_page_id() {
            return Err("navigation lifecycle belongs to another renderer Page");
        }
        let lifecycle = DocumentLifecycle::from_creation_artifacts(artifacts)
            .ok_or("inconsistent navigation document lifecycle")?;
        let title = page.document_title();
        Ok(Self {
            identity,
            page,
            lifecycle,
            info: CommittedDocumentInfo {
                url: destination.url,
                title,
                security_origin: destination.security_origin,
                secure_context_type: destination.secure_context_type,
            },
        })
    }
}

/// Self-contained Browser occurrence plus a move-owned post-commit retirement.
pub(in crate::conn) struct CommittedDocumentNavigation {
    pub(in crate::conn) web_contents: WebContentsId,
    pub(in crate::conn) frame_slot: MainFrameSlotId,
    pub(in crate::conn) navigation: NavigationId,
    pub(in crate::conn) document: DocumentId,
    pub(in crate::conn) previous_document: Option<DocumentId>,
    pub(in crate::conn) previous_renderer: Option<RendererPageResidenceIdentity>,
    pub(in crate::conn) inspection_endpoint: moli_renderer_v8::RendererInspectionEndpoint,
    pub(in crate::conn) info: CommittedDocumentInfo,
    pub(in crate::conn) retirement: RetiringDocument,
    pub(in crate::conn) post_response_continuation:
        Option<RendererPageCommandPostResponseContinuation>,
}

/// Retirement has no commit authority and never retains a Browser registry borrow.
pub(crate) struct RetiringDocument(Option<Page>);

impl RetiringDocument {
    pub(crate) async fn close(self) {
        if let Some(page) = self.0 {
            let _ = page.close_async().await;
        }
    }
}

impl WebContents {
    fn document_navigation_identity(
        &self,
        navigation: NavigationId,
    ) -> Result<DocumentNavigationIdentity, &'static str> {
        let (_, document) = self
            .navigation
            .pending_document()
            .filter(|(pending, _)| *pending == navigation)
            .ok_or("stale navigation document candidate")?;
        let cancellation = self
            .navigation
            .document_navigation_cancellation_handle(&navigation)
            .ok_or("stale navigation document candidate")?;
        if cancellation.is_cancelled() {
            return Err("canceled navigation document candidate");
        }
        Ok(DocumentNavigationIdentity {
            web_contents: self.id(),
            frame_slot: self.main_frame.id(),
            navigation,
            document,
            cancellation,
        })
    }

    /// Start/configure/complete stays inside the Browser participant. The
    /// returned future owns its Page and policy, never a Browser registry borrow.
    pub(in crate::conn) fn start_loaded_document_navigation(
        &self,
        navigation: NavigationId,
        page: Page,
        destination: DocumentNavigationDestination,
        artifacts: &RendererPageCreationArtifacts,
        interception: (bool, Option<moli_core::page::SubresourceResourceType>),
        permissions: Vec<moli_core::page::PermissionOverrideRegistration>,
    ) -> Result<
        impl std::future::Future<Output = anyhow::Result<PreparedDocumentNavigation>> + use<>,
        &'static str,
    > {
        let identity = self.document_navigation_identity(navigation)?;
        let prepared = PreparedDocumentNavigation::new(identity, page, destination, artifacts)?;
        Ok(async move {
            anyhow::ensure!(
                !prepared.identity.cancellation.is_cancelled(),
                "canceled navigation document candidate"
            );
            let prepared = prepared
                .apply_document_policy(interception, &permissions)
                .await?;
            anyhow::ensure!(
                !prepared.identity.cancellation.is_cancelled(),
                "canceled navigation document candidate"
            );
            Ok(prepared)
        })
    }

    pub(in crate::conn) fn start_document_materialization(
        &self,
        navigation: NavigationId,
        page: PreparedDocumentPage,
        destination: DocumentNavigationDestination,
    ) -> Result<AdmittedDocumentMaterialization, &'static str> {
        let identity = self.document_navigation_identity(navigation)?;
        Ok(AdmittedDocumentMaterialization {
            identity,
            page,
            destination,
        })
    }

    pub(in crate::conn) fn commit_document_navigation(
        &mut self,
        mut prepared: PreparedDocumentNavigation,
    ) -> Result<CommittedDocumentNavigation, &'static str> {
        let Some((navigation, document_id)) =
            self.navigation
                .pending_document()
                .filter(|(navigation, document)| {
                    *navigation == prepared.identity.navigation
                        && *document == prepared.identity.document
                        && self.id() == prepared.identity.web_contents
                        && self.main_frame.id() == prepared.identity.frame_slot
                        && !prepared.identity.cancellation.is_cancelled()
                })
        else {
            return Err("stale navigation document candidate");
        };
        let previous_document = self
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
        let previous_renderer = self
            .main_frame
            .current_document
            .as_ref()
            .map(|document| RendererPageResidenceIdentity::from_page(&document.page));
        let inspection_endpoint = prepared.page.renderer_inspection_endpoint();
        // The controller already owns observed history titles. An outgoing
        // Page's cached inventory must not rewind a later title observation.
        self.navigation.record_loaded_page_navigation_history((
            prepared.info.url.to_string(),
            prepared.info.title.clone(),
        ));
        let post_response_continuation = prepared
            .page
            .take_committed_document_post_response_continuation();
        let mut document = DocumentHost::new(document_id, prepared.page);
        document.lifecycle = prepared.lifecycle;
        let retirement = RetiringDocument(self.replace_document(Some(document)));
        assert!(
            self.navigation
                .commit_pending_document_navigation_if_matches(&navigation)
        );
        Ok(CommittedDocumentNavigation {
            web_contents: self.id(),
            frame_slot: self.main_frame.id(),
            navigation,
            document: document_id,
            previous_document,
            previous_renderer,
            inspection_endpoint,
            info: prepared.info,
            retirement,
            post_response_continuation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::{
        browser::DocumentRetirement,
        runtime::{Browser, BrowserConfig},
    };
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    async fn prepare(
        contents: &WebContents,
        browser: &Browser,
        navigation: NavigationId,
        title: &str,
    ) -> PreparedDocumentNavigation {
        let mut page = browser
            .fetch(&format!("data:text/html,<title>{title}</title>"))
            .await
            .unwrap();
        let artifacts = page.take_page_creation_artifacts().unwrap();
        let url = page.final_url().clone();
        contents
            .start_loaded_document_navigation(
                navigation,
                page,
                DocumentNavigationDestination {
                    url,
                    security_origin: "null".into(),
                    secure_context_type: "InsecureScheme".into(),
                },
                &artifacts,
                (false, None),
                Vec::new(),
            )
            .unwrap()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn browser_commit_is_complete_without_a_devtools_projection_or_retirement_await() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut contents = WebContents::default();
        let stable_owner = (contents.id(), contents.main_frame.id());
        let navigation = contents.navigation.start_document_navigation();
        let first = contents
            .commit_document_navigation(prepare(&contents, &browser, navigation, "first").await)
            .unwrap();
        assert!(first.previous_document.is_none());
        first.retirement.close().await;
        let old_document = first.document;
        assert!(
            contents
                .navigation
                .refresh_current_navigation_history_title("observed first".into())
        );
        let observer = contents
            .main_frame
            .current_document
            .as_mut()
            .unwrap()
            .lifetime
            .observe();
        let mut retired = Box::pin(observer.wait());
        let mut context = Context::from_waker(Waker::noop());
        assert_eq!(retired.as_mut().poll(&mut context), Poll::Pending);
        let navigation = contents.navigation.start_document_navigation();
        let expected_document = contents.navigation.pending_document().unwrap().1;
        let committed = contents
            .commit_document_navigation(prepare(&contents, &browser, navigation, "second").await)
            .unwrap();

        assert_eq!((committed.web_contents, committed.frame_slot), stable_owner);
        assert_eq!(committed.navigation, navigation);
        assert_eq!(committed.document, expected_document);
        assert_eq!(committed.previous_document, Some(old_document));
        assert_eq!(contents.navigation.pending_document(), None);
        assert_eq!(
            contents.navigation.committed_document_navigation(),
            Some(navigation)
        );
        let document = contents.main_frame.current_document.as_ref().unwrap();
        assert_eq!(document.id, expected_document);
        assert!(document.lifecycle.snapshot().is_some());
        assert_eq!(
            retired.as_mut().poll(&mut context),
            Poll::Ready(DocumentRetirement::Superseded)
        );
        let (index, history) = contents.navigation.navigation_history_snapshot(None);
        assert_eq!(index, 1);
        assert_eq!(
            history
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            ["observed first", "second"]
        );
        assert_eq!(history[1].url, committed.info.url.as_str());

        // Cancellation drops only the old, move-owned Page, never a partially
        // committed Browser transaction or the new Document's lifetime.
        drop(committed.retirement.close());
        let document = contents.main_frame.current_document.as_mut().unwrap();
        assert_eq!(document.id, expected_document);
        assert_eq!(document.page.document_title(), "second");
        let mut current_lifetime = Box::pin(document.lifetime.observe().wait());
        assert_eq!(current_lifetime.as_mut().poll(&mut context), Poll::Pending);
        contents.begin_close().close_async().await;
        assert_eq!(current_lifetime.await, DocumentRetirement::Superseded);
    }

    #[tokio::test]
    async fn stale_and_foreign_navigation_candidates_cannot_mutate_the_current_document() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut contents = WebContents::default();
        let first = contents.navigation.start_document_navigation();
        let first = contents
            .commit_document_navigation(prepare(&contents, &browser, first, "first").await)
            .unwrap();
        let old_document = first.document;
        let mut old_lifetime = Box::pin(
            contents
                .main_frame
                .current_document
                .as_mut()
                .unwrap()
                .lifetime
                .observe()
                .wait(),
        );
        let stale = contents.navigation.start_document_navigation();
        let stale = prepare(&contents, &browser, stale, "stale").await;
        let current = contents.navigation.start_document_navigation();
        let pending = contents.navigation.pending_document();
        let mut foreign = WebContents::default();
        let foreign_navigation = foreign.navigation.start_document_navigation();
        let foreign_candidate = prepare(&foreign, &browser, foreign_navigation, "foreign").await;

        for rejected in [stale, foreign_candidate] {
            assert_eq!(
                contents.commit_document_navigation(rejected).err(),
                Some("stale navigation document candidate")
            );
            assert_eq!(contents.navigation.pending_document(), pending);
            assert_eq!(
                contents.navigation.current_document_navigation(),
                Some(current)
            );
            assert_eq!(
                contents.navigation.committed_document_navigation(),
                Some(first.navigation)
            );
            assert_eq!(
                contents.main_frame.current_document.as_ref().unwrap().id,
                old_document
            );
            let (index, history) = contents.navigation.navigation_history_snapshot(None);
            assert_eq!(index, 0);
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].title, "first");
            let mut context = Context::from_waker(Waker::noop());
            assert_eq!(old_lifetime.as_mut().poll(&mut context), Poll::Pending);
        }
        assert_eq!(
            foreign.navigation.current_document_navigation(),
            Some(foreign_navigation)
        );
        assert!(foreign.main_frame.current_document.is_none());
    }

    #[tokio::test]
    async fn failed_candidate_policy_never_retires_the_committed_document() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut contents = WebContents::default();
        let navigation = contents.navigation.start_document_navigation();
        let first = contents
            .commit_document_navigation(prepare(&contents, &browser, navigation, "first").await)
            .unwrap();
        let mut lifetime = Box::pin(
            contents
                .main_frame
                .current_document
                .as_mut()
                .unwrap()
                .lifetime
                .observe()
                .wait(),
        );
        for (interception, permissions) in [
            ((true, None), Vec::new()),
            (
                (false, None),
                vec![moli_core::page::PermissionOverrideRegistration {
                    permission: serde_json::json!({"name": "geolocation"}),
                    setting: "granted".into(),
                    origin: None,
                    embedded_origin: None,
                }],
            ),
        ] {
            let navigation = contents.navigation.start_document_navigation();
            let candidate_browser = Browser::new(BrowserConfig::default()).unwrap();
            let mut page = candidate_browser
                .fetch("data:text/html,candidate")
                .await
                .unwrap();
            let artifacts = page.take_page_creation_artifacts().unwrap();
            let url = page.final_url().clone();
            let preparation = contents
                .start_loaded_document_navigation(
                    navigation,
                    page,
                    DocumentNavigationDestination {
                        url,
                        security_origin: "null".into(),
                        secure_context_type: "InsecureScheme".into(),
                    },
                    &artifacts,
                    interception,
                    permissions,
                )
                .unwrap();
            // Retire the candidate's independent renderer owner synchronously.
            // DevTools Page.crash is not a native-command admission fence.
            drop(candidate_browser);
            assert!(preparation.await.is_err());
            assert_eq!(
                contents.main_frame.current_document.as_ref().unwrap().id,
                first.document
            );
            assert_eq!(
                contents.navigation.pending_document().unwrap().0,
                navigation
            );
            assert_eq!(
                contents
                    .navigation
                    .navigation_history_snapshot(None)
                    .1
                    .len(),
                1
            );
            assert_eq!(
                lifetime
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            );
        }
        assert_eq!(
            contents
                .main_frame
                .current_document
                .as_mut()
                .unwrap()
                .page
                .evaluate_runtime_expression_async("40 + 2")
                .await
                .unwrap()["value"],
            42
        );
    }

    #[tokio::test]
    async fn owned_preparation_observes_supersession_and_browser_retirement_without_a_borrow() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        for retire in [false, true] {
            let mut contents = WebContents::default();
            let first = contents.navigation.start_document_navigation();
            let first = contents
                .commit_document_navigation(prepare(&contents, &browser, first, "first").await)
                .unwrap();
            let navigation = contents.navigation.start_document_navigation();
            let mut page = browser.fetch("data:text/html,candidate").await.unwrap();
            let artifacts = page.take_page_creation_artifacts().unwrap();
            let destination = DocumentNavigationDestination {
                url: page.final_url().clone(),
                security_origin: "null".into(),
                secure_context_type: "InsecureScheme".into(),
            };
            let pending = contents
                .start_loaded_document_navigation(
                    navigation,
                    page,
                    destination,
                    &artifacts,
                    (false, None),
                    Vec::new(),
                )
                .unwrap();
            if retire {
                let closing = contents.begin_close();
                assert_eq!(
                    pending.await.unwrap_err().to_string(),
                    "canceled navigation document candidate"
                );
                closing.close_async().await;
            } else {
                let replacement = contents.navigation.start_document_navigation();
                assert_eq!(
                    pending.await.unwrap_err().to_string(),
                    "canceled navigation document candidate"
                );
                assert_eq!(
                    contents.navigation.pending_document().unwrap().0,
                    replacement
                );
                assert_eq!(
                    contents.main_frame.current_document.as_ref().unwrap().id,
                    first.document
                );
                assert_eq!(
                    contents
                        .navigation
                        .navigation_history_snapshot(None)
                        .1
                        .len(),
                    1
                );
            }
            first.retirement.close().await;
        }
    }

    #[tokio::test]
    async fn completion_validates_every_identity_captured_at_admission() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut contents = WebContents::default();
        let navigation = contents.navigation.start_document_navigation();
        let pending = contents.navigation.pending_document();
        for mismatch in 0..3 {
            let mut prepared = prepare(&contents, &browser, navigation, "candidate").await;
            // Fault injection inside the native module: production callers
            // cannot forge or rewrite any of these opaque participant fields.
            match mismatch {
                0 => prepared.identity.web_contents = WebContentsId::allocate(),
                1 => prepared.identity.frame_slot = MainFrameSlotId::allocate(),
                _ => prepared.identity.document = DocumentId::allocate(),
            }
            assert_eq!(
                contents.commit_document_navigation(prepared).err(),
                Some("stale navigation document candidate")
            );
            assert_eq!(contents.navigation.pending_document(), pending);
            assert!(contents.main_frame.current_document.is_none());
            assert!(
                contents
                    .navigation
                    .navigation_history_snapshot(None)
                    .1
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn candidate_requires_its_own_consistent_renderer_lifecycle() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut first = browser.fetch("data:text/html,first").await.unwrap();
        let artifacts = first.take_page_creation_artifacts().unwrap();
        let second = browser.fetch("data:text/html,second").await.unwrap();
        let mut contents = WebContents::default();
        let navigation = contents.navigation.start_document_navigation();
        let url = second.final_url().clone();
        assert_eq!(
            contents
                .start_loaded_document_navigation(
                    navigation,
                    second,
                    DocumentNavigationDestination {
                        url,
                        security_origin: "null".into(),
                        secure_context_type: "InsecureScheme".into(),
                    },
                    &artifacts,
                    (false, None),
                    Vec::new(),
                )
                .err(),
            Some("navigation lifecycle belongs to another renderer Page")
        );
        let mut inconsistent = artifacts;
        inconsistent.active_epoch.0 += 1;
        let url = first.final_url().clone();
        assert_eq!(
            contents
                .start_loaded_document_navigation(
                    navigation,
                    first,
                    DocumentNavigationDestination {
                        url,
                        security_origin: "null".into(),
                        secure_context_type: "InsecureScheme".into(),
                    },
                    &inconsistent,
                    (false, None),
                    Vec::new(),
                )
                .err(),
            Some("inconsistent navigation document lifecycle")
        );
    }
}
