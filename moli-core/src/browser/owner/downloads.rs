use super::*;
use crate::browser::{BrowserEvent, BrowserSequence, DownloadEvent, DownloadObservation};

impl Browser {
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
