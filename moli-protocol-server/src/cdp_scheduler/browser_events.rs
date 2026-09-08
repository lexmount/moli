use moli_core::browser::{BrowserEvent, BrowserEventReceiver, BrowserEventRecord};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use super::{
    CdpScheduler, CdpSchedulerEventReceivers, CdpSchedulerInterleavedInput, ProtocolOutputSequence,
};

pub(super) type BrowserEventInput = Result<BrowserEventRecord, RecvError>;

pub(super) async fn recv_browser_event(
    receiver: &mut Option<BrowserEventReceiver>,
) -> BrowserEventInput {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

impl CdpScheduler {
    pub(crate) fn is_browser_closed(&self) -> bool {
        self.browser_event_rx.is_none()
    }

    pub(crate) async fn recv_browser_event(&mut self) -> BrowserEventInput {
        recv_browser_event(&mut self.browser_event_rx).await
    }

    pub(crate) async fn recv_interleaved_input(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
    ) -> Option<CdpSchedulerInterleavedInput> {
        receivers
            .recv_interleaved_input(&mut self.browser_event_rx)
            .await
    }

    pub(crate) async fn drain_browser_events(&mut self) -> ProtocolOutputSequence {
        let mut output = ProtocolOutputSequence::empty();
        while let Some(receiver) = self.browser_event_rx.as_mut() {
            let event = match receiver.try_recv() {
                Ok(event) => Ok(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(count)) => Err(RecvError::Lagged(count)),
                Err(TryRecvError::Closed) => Err(RecvError::Closed),
            };
            output.append(self.handle_browser_event(event).await);
        }
        output
    }

    pub(crate) async fn handle_browser_event(
        &mut self,
        event: BrowserEventInput,
    ) -> ProtocolOutputSequence {
        let mut closed = Vec::new();
        let mut documents = Vec::new();
        let disposed = match event {
            Ok(BrowserEventRecord {
                event: BrowserEvent::ContextCreated(_) | BrowserEvent::WebContentsCreated(_),
                ..
            }) => return ProtocolOutputSequence::empty(),
            Ok(BrowserEventRecord {
                event: BrowserEvent::DocumentCommitted(document),
                ..
            }) => {
                return ProtocolOutputSequence::from_background_events(
                    self.conn.project_browser_document_commit(document).await,
                );
            }
            Ok(BrowserEventRecord {
                event: BrowserEvent::ContextDisposed(context),
                ..
            }) => vec![context],
            Ok(BrowserEventRecord {
                event:
                    BrowserEvent::WebContentsClosed {
                        web_contents,
                        activated,
                    },
                ..
            }) => {
                return ProtocolOutputSequence::from_background_events(
                    self.conn
                        .project_closed_web_contents(web_contents, activated)
                        .await,
                );
            }
            Err(error) => {
                let live = match error {
                    RecvError::Lagged(_) => self.conn.subscribe_browser_events().ok(),
                    RecvError::Closed => None,
                };
                let contexts =
                    if let Some((snapshot, receiver)) = live {
                        self.browser_event_rx = Some(receiver);
                        documents = snapshot.documents;
                        closed =
                            self.conn
                                .projected_web_contents()
                                .into_iter()
                                .filter(|handle| !snapshot.web_contents.contains(handle))
                                .map(|handle| {
                                    (
                                        handle,
                                        snapshot.selected_web_contents.iter().copied().find(
                                            |selected| selected.context() == handle.context(),
                                        ),
                                    )
                                })
                                .collect();
                        snapshot.contexts
                    } else {
                        self.browser_event_rx = None;
                        Vec::new()
                    };
                self.conn
                    .browser_contexts()
                    .map(|context| context.browser_context_id())
                    .filter(|context| !contexts.contains(context))
                    .collect()
            }
        };
        let mut output = ProtocolOutputSequence::empty();
        for context in disposed {
            output.append(ProtocolOutputSequence::from_background_events(
                self.conn.project_disposed_browser_context(context).await,
            ));
        }
        for (handle, activated) in closed {
            output.append(ProtocolOutputSequence::from_background_events(
                self.conn
                    .project_closed_web_contents(handle, activated)
                    .await,
            ));
        }
        for document in documents {
            output.append(ProtocolOutputSequence::from_background_events(
                self.conn.project_browser_document_commit(document).await,
            ));
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::browser::BrowserService;
    use moli_protocol::CdpInitialStoragePartition;

    #[tokio::test]
    async fn native_web_contents_close_and_lag_recovery_retire_only_exact_projections() {
        for lagged in [false, true] {
            let service = BrowserService::start().unwrap();
            let browser = service.handle();
            let (mut scheduler, mut receivers) =
                CdpScheduler::new_with_initial_state_runtime_config(
                    browser.clone(),
                    CdpInitialStoragePartition::memory(),
                    Default::default(),
                );
            let created = scheduler.execute_internal_protocol_message(&mut receivers, serde_json::json!({
                "id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true},
            })).await.unwrap_or_else(|failure| panic!("{:?}", failure.into_parts().1)).into_messages();
            assert!(
                created
                    .iter()
                    .any(|message| message["method"] == "Target.targetCreated")
            );
            let handle = scheduler.conn.projected_web_contents()[0];
            // A live occurrence, or a forged stale record, cannot retire a live projection.
            assert!(
                scheduler
                    .conn
                    .project_closed_web_contents(handle, None)
                    .await
                    .is_empty()
            );
            browser
                .close_web_contents(handle)
                .unwrap()
                .close_async()
                .await;
            let output = if lagged {
                scheduler
                    .handle_browser_event(Err(RecvError::Lagged(1)))
                    .await
            } else {
                scheduler.drain_browser_events().await
            }
            .into_messages();
            assert!(scheduler.conn.projected_web_contents().is_empty());
            assert_eq!(scheduler.conn.browser_contexts().count(), 1);
            assert!(browser.contains_context(handle.context()));
            assert_eq!(
                output
                    .iter()
                    .filter(|message| message["method"] == "Target.targetDestroyed"
                        && message["params"]["targetId"] == scheduler.conn.default_target_id())
                    .count(),
                1
            );
            assert!(
                scheduler
                    .conn
                    .project_closed_web_contents(handle, None)
                    .await
                    .is_empty()
            );
            assert!(
                scheduler
                    .drain_browser_events()
                    .await
                    .into_messages()
                    .is_empty()
            );
            service.shutdown();
        }
    }

    #[tokio::test]
    async fn browser_shutdown_retires_all_context_sessions_before_closed_observation() {
        let service = BrowserService::start().unwrap();
        let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        for command in [
            serde_json::json!({"id": 1, "method": "Target.attachToTarget", "params": {
                "targetId": scheduler.conn.default_target_id(), "flatten": true,
            }}),
            serde_json::json!({"id": 2, "method": "Target.createBrowserContext", "params": {}}),
        ] {
            let id = command["id"].clone();
            let output = scheduler
                .execute_internal_protocol_message(&mut receivers, command)
                .await
                .unwrap_or_else(|failure| panic!("setup failed: {:?}", failure.into_parts().1))
                .into_messages();
            assert!(
                output
                    .iter()
                    .any(|message| message["id"] == id && message.get("result").is_some()),
                "{output:?}"
            );
        }
        assert_eq!(scheduler.conn.browser_contexts().count(), 2);
        service.shutdown();
        let output = scheduler.drain_browser_events().await.into_messages();
        assert!(scheduler.is_browser_closed());
        assert!(scheduler.conn.browser_contexts().next().is_none());
        assert_eq!(
            output
                .iter()
                .filter(|message| message["method"] == "Target.detachedFromTarget")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn lagged_browser_subscription_retires_only_missing_context_projections() {
        let service = BrowserService::start().unwrap();
        let browser = service.handle();
        let (mut first, mut first_rx) = CdpScheduler::new_with_initial_state_runtime_config(
            browser.clone(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let (mut peer, mut peer_rx) = CdpScheduler::new_with_initial_state_runtime_config(
            browser.clone(),
            CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        let first_id = first
            .conn
            .browser_contexts()
            .next()
            .unwrap()
            .browser_context_id();
        let peer_id = peer
            .conn
            .browser_contexts()
            .next()
            .unwrap()
            .browser_context_id();
        for (scheduler, receivers) in [(&mut first, &mut first_rx), (&mut peer, &mut peer_rx)] {
            let created = scheduler.execute_internal_protocol_message(receivers, serde_json::json!({
                "id": 1, "method": "Target.setDiscoverTargets", "params": {"discover": true},
            })).await.unwrap_or_else(|failure| panic!("discovery failed: {:?}", failure.into_parts().1)).into_messages();
            assert!(
                created
                    .iter()
                    .any(|message| message["method"] == "Target.targetCreated")
            );
        }
        assert!(browser.remove_context(first_id).unwrap());
        let output = first.handle_browser_event(Err(RecvError::Lagged(1))).await;
        assert!(first.conn.browser_contexts().next().is_none());
        assert!(!first.is_browser_closed());
        assert_eq!(
            output
                .into_messages()
                .iter()
                .filter(|message| {
                    message["method"] == "Target.targetDestroyed"
                        && message["params"]["targetId"] == first.conn.default_target_id()
                })
                .count(),
            1
        );
        assert!(
            first
                .drain_browser_events()
                .await
                .into_messages()
                .is_empty()
        );
        assert!(peer.drain_browser_events().await.into_messages().is_empty());
        assert_eq!(
            peer.conn
                .browser_contexts()
                .next()
                .unwrap()
                .browser_context_id(),
            peer_id
        );

        // Both the recovered subscriber and its peer receive the next event;
        // only the peer owns a projection for that exact Context.
        assert!(browser.remove_context(peer_id).unwrap());
        assert!(
            first
                .drain_browser_events()
                .await
                .into_messages()
                .is_empty()
        );
        assert!(!peer.drain_browser_events().await.into_messages().is_empty());
        assert!(peer.conn.browser_contexts().next().is_none());
        service.shutdown();
        first.drain_browser_events().await;
        peer.drain_browser_events().await;
        assert!(first.is_browser_closed());
        assert!(peer.is_browser_closed());
    }
}
