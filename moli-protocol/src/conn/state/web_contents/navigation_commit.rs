use moli_core::{
    browser::{DocumentId, DocumentLifecycle, MainFrameSlotId, NavigationId, WebContentsId},
    page::{Page, RendererPageCommandPostResponseContinuation, RendererPageCreationArtifacts},
};
use url::Url;

use super::{DocumentHost, WebContents};

/// Browser-only commit participant in the private migration residence (20/24b).
/// No frontend identity, renderer attachment, arbitrary callback or Page lease.
pub(crate) struct PreparedDocumentNavigation {
    navigation: NavigationId,
    page: Page,
    lifecycle: DocumentLifecycle,
    info: CommittedDocumentInfo,
}

pub(in crate::conn) struct CommittedDocumentInfo {
    pub(in crate::conn) url: Url,
    pub(in crate::conn) title: String,
    pub(in crate::conn) security_origin: String,
    pub(in crate::conn) secure_context_type: String,
}

impl PreparedDocumentNavigation {
    pub(in crate::conn) fn navigation(&self) -> NavigationId {
        self.navigation
    }

    pub(in crate::conn) fn inspection_endpoint(
        &self,
    ) -> moli_renderer_v8::RendererInspectionEndpoint {
        self.page.renderer_inspection_endpoint()
    }

    pub(crate) fn new(
        navigation: NavigationId,
        page: Page,
        url: Url,
        security_origin: String,
        secure_context_type: String,
        artifacts: &RendererPageCreationArtifacts,
    ) -> Result<Self, &'static str> {
        if artifacts.lifecycle_snapshot.frame.page_id != page.renderer_page_id() {
            return Err("navigation lifecycle belongs to another renderer Page");
        }
        let lifecycle = DocumentLifecycle::from_creation_artifacts(artifacts)
            .ok_or("inconsistent navigation document lifecycle")?;
        let title = page.document_title();
        Ok(Self {
            navigation,
            page,
            lifecycle,
            info: CommittedDocumentInfo {
                url,
                title,
                security_origin,
                secure_context_type,
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
    pub(in crate::conn) fn commit_document_navigation(
        &mut self,
        mut prepared: PreparedDocumentNavigation,
    ) -> Result<CommittedDocumentNavigation, &'static str> {
        let Some((navigation, document_id)) = self
            .navigation
            .pending_document()
            .filter(|(navigation, _)| *navigation == prepared.navigation)
        else {
            return Err("stale navigation document candidate");
        };
        let previous_document = self
            .main_frame
            .current_document
            .as_ref()
            .map(|document| document.id);
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
        PreparedDocumentNavigation::new(
            navigation,
            page,
            url,
            "null".into(),
            "InsecureScheme".into(),
            &artifacts,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn browser_commit_is_complete_without_a_devtools_projection_or_retirement_await() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut contents = WebContents::default();
        let stable_owner = (contents.id(), contents.main_frame.id());
        let navigation = contents.navigation.start_document_navigation();
        let first = contents
            .commit_document_navigation(prepare(&browser, navigation, "first").await)
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
            .commit_document_navigation(prepare(&browser, navigation, "second").await)
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
            .commit_document_navigation(prepare(&browser, first, "first").await)
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
        let stale = prepare(&browser, stale, "stale").await;
        let current = contents.navigation.start_document_navigation();
        let pending = contents.navigation.pending_document();
        let mut foreign = WebContents::default();
        let foreign_navigation = foreign.navigation.start_document_navigation();
        let foreign_candidate = prepare(&browser, foreign_navigation, "foreign").await;

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
    async fn candidate_requires_its_own_consistent_renderer_lifecycle() {
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut first = browser.fetch("data:text/html,first").await.unwrap();
        let artifacts = first.take_page_creation_artifacts().unwrap();
        let second = browser.fetch("data:text/html,second").await.unwrap();
        let navigation = NavigationId::allocate();
        let url = second.final_url().clone();
        assert_eq!(
            PreparedDocumentNavigation::new(
                navigation,
                second,
                url,
                "null".into(),
                "InsecureScheme".into(),
                &artifacts
            )
            .err(),
            Some("navigation lifecycle belongs to another renderer Page")
        );
        let mut inconsistent = artifacts;
        inconsistent.active_epoch.0 += 1;
        let url = first.final_url().clone();
        assert_eq!(
            PreparedDocumentNavigation::new(
                navigation,
                first,
                url,
                "null".into(),
                "InsecureScheme".into(),
                &inconsistent
            )
            .err(),
            Some("inconsistent navigation document lifecycle")
        );
    }
}
