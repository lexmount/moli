use super::*;
use crate::browser::{
    BrowserEvent, BrowserSequence, DownloadBody, DownloadEvent, DownloadObservation,
    DownloadPolicy, NavigationFailureReason, NavigationRequest, RendererPageResidenceIdentity,
};

impl BrowserHandle {
    pub fn set_download_policy(&self, policy: DownloadPolicy) {
        self.execute(move |browser| browser.download_policy = policy)
            .expect("live Browser owner must accept download policy");
    }

    pub fn download_policy(&self) -> DownloadPolicy {
        self.execute(|browser| browser.download_policy.clone())
            .expect("live Browser owner must snapshot download policy")
    }
}

impl Browser {
    pub(super) fn download_navigation_response(
        &mut self,
        request: NavigationRequest,
        renderer: RendererPageResidenceIdentity,
        response: moli_fetch::ResponseHead,
        body: DownloadBody,
    ) -> Result<(), String> {
        let context = self.context(request.web_contents.context())?;
        let page = context.web_contents(request.web_contents)?;
        if page.navigation().pending_document() != Some((request.navigation, request.document))
            || !page
                .navigation()
                .accepts_document_preparation(request.navigation, renderer)
        {
            return Err("stale navigation download response".into());
        }
        let policy = context
            .download_policy()
            .unwrap_or(&self.download_policy)
            .clone();
        let context = self.context_mut(request.web_contents.context())?;
        let admitted = context.start_download_response(
            request.web_contents,
            &policy,
            response.final_url,
            response.headers,
            body,
        )?;
        // The response is now owned by the download manager. Retire this exact
        // candidate without revoking its transferred transport or current Document.
        context
            .web_contents_mut(request.web_contents)?
            .navigation_mut()
            .finish_navigation_as_download(request.navigation);
        if let Some(admitted) = admitted {
            self.admit_download(admitted);
        }
        self.events
            .publish(BrowserEvent::NavigationResponseChanged(request));
        self.events.publish(BrowserEvent::NavigationFailed {
            request,
            reason: NavigationFailureReason::Download,
        });
        Ok(())
    }

    pub(super) fn admit_download(
        &mut self,
        admitted: crate::browser::downloads::AdmittedDownload,
    ) -> DownloadObservation {
        let crate::browser::downloads::AdmittedDownload {
            mut observation,
            state,
            progress,
        } = admitted;
        let initial = observation.event();
        self.events.publish_committed(
            initial.sequence,
            BrowserEvent::DownloadCreated(observation.record_snapshot()),
        );
        if let Some(mut progress) = progress {
            let sender = self.local_sender.clone();
            tokio::task::spawn_local(async move {
                while progress.changed().await.is_ok() {
                    let snapshot = progress.borrow_and_update().clone();
                    let (committed, completion) = oneshot::channel();
                    let state = state.clone();
                    let initial = initial.clone();
                    if sender
                        .send(Box::new(move |browser| {
                            let event = Arc::new(DownloadEvent {
                                sequence: BrowserSequence::allocate(),
                                web_contents: initial.web_contents,
                                guid: initial.guid.clone(),
                                snapshot,
                            });
                            // A retired Context cannot be resurrected. Its transfer
                            // still owns cleanup, and publishes terminal only after
                            // that cleanup finishes, to its original observation.
                            state.send_replace(event.clone());
                            browser.events.publish_committed(
                                event.sequence,
                                BrowserEvent::DownloadUpdated(event),
                            );
                            let _ = committed.send(());
                        }))
                        .is_err()
                        || completion.await.is_err()
                    {
                        break;
                    }
                    // One outstanding owner completion per download; the
                    // transfer's watch coalesces progress while we await it.
                }
            });
        }
        observation
    }
}
