use tokio::sync::mpsc;

use crate::cdp_frontend_router::CdpFrontendRouter;

use super::{CdpScheduler, ProtocolOutputSequence};

#[derive(Clone, Copy)]
pub(super) struct BidiFrontendTurn {
    pub(super) id: u64,
    publish_notifications: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_sources_release_only_the_last_exact_frontend() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            moli_protocol::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        for source in [
            BidiEventSource::Runtime("TID-source".to_owned()),
            BidiEventSource::Network("TID-source".to_owned()),
            BidiEventSource::FileDialog("TID-source".to_owned()),
            BidiEventSource::Download,
        ] {
            scheduler.set_bidi_frontend_turn(Some(1));
            scheduler
                .bidi_event_sources
                .entry(source)
                .or_default()
                .insert(1);
        }
        scheduler.set_bidi_observer_turn(2);
        for source in [
            BidiEventSource::Runtime("TID-source".to_owned()),
            BidiEventSource::Network("TID-source".to_owned()),
            BidiEventSource::FileDialog("TID-source".to_owned()),
            BidiEventSource::Download,
        ] {
            scheduler.retain_bidi_event_source(source);
        }
        for source in [
            BidiEventSource::Runtime("TID-source".to_owned()),
            BidiEventSource::Network("TID-source".to_owned()),
            BidiEventSource::FileDialog("TID-source".to_owned()),
            BidiEventSource::Download,
        ] {
            scheduler.set_bidi_frontend_turn(Some(1));
            assert!(!scheduler.release_bidi_event_source(&source));
            assert!(!scheduler.release_bidi_event_source(&source));
            scheduler.set_bidi_frontend_turn(Some(2));
            assert!(scheduler.release_bidi_event_source(&source));
            assert!(!scheduler.release_bidi_event_source(&source));
        }
        assert!(scheduler.bidi_event_sources.is_empty());
    }

    #[tokio::test]
    async fn concurrent_discovery_leases_restore_the_original_setting() {
        let service = moli_core::browser::BrowserService::start().unwrap();
        let (mut scheduler, _) = CdpScheduler::new_with_initial_state_runtime_config(
            service.handle(),
            moli_protocol::CdpInitialStoragePartition::memory(),
            Default::default(),
        );
        for initial in [false, true] {
            for order in [[1, 2], [2, 1]] {
                scheduler.set_bidi_frontend_turn(None);
                scheduler.replace_target_discovery_enabled(initial);
                for id in [1, 2] {
                    scheduler.set_bidi_frontend_turn(Some(id));
                    assert!(!scheduler.replace_target_discovery_enabled(true));
                }
                scheduler.set_bidi_frontend_turn(Some(order[0]));
                assert!(scheduler.replace_target_discovery_enabled(false));
                assert!(scheduler.conn.replace_root_target_discovery_enabled(true));
                scheduler.set_bidi_frontend_turn(Some(order[1]));
                assert!(scheduler.replace_target_discovery_enabled(false));
                assert_eq!(
                    scheduler
                        .conn
                        .replace_root_target_discovery_enabled(initial),
                    initial
                );
                assert!(scheduler.bidi_event_sources.is_empty());
                assert!(scheduler.bidi_initial_target_discovery.is_none());
            }
        }
    }
}

#[derive(Hash, PartialEq, Eq)]
pub(super) enum BidiEventSource {
    Runtime(String),
    Network(String),
    FileDialog(String),
    Download,
    TargetDiscovery,
}

/// Only already projected envelopes cross the frontend boundary. Renderer
/// transport and command-response release permits still have one owner.
pub(crate) enum DevToolsFrontendOutput {
    Cdp(ProtocolOutputSequence),
    Bidi {
        origin: Option<u64>,
        output: ProtocolOutputSequence,
    },
}

impl CdpScheduler {
    pub(crate) fn webdriver_event_is_visible(
        &self,
        session: Option<&str>,
        event: &moli_protocol::BackgroundProtocolEvent,
    ) -> bool {
        self.conn.webdriver_event_is_visible(session, event)
    }

    pub(crate) fn webdriver_automation_event_is_visible(
        &self,
        session: Option<&str>,
        event: Option<&moli_protocol::devtools_runtime::AutomationEvent>,
        fallback_target: Option<&str>,
    ) -> bool {
        self.conn
            .webdriver_automation_event_is_visible(session, event, fallback_target)
    }
    pub(crate) fn publish_devtools_output(&self, output: ProtocolOutputSequence) {
        if !output.is_empty()
            && let Some(tx) = &self.frontend_output_tx
        {
            let _ = tx.send(DevToolsFrontendOutput::Cdp(output));
        }
    }

    pub(super) fn retire_bidi_target_event_sources(&mut self, target_id: &str) {
        self.bidi_event_sources.retain(|source, _| match source {
            BidiEventSource::Runtime(target)
            | BidiEventSource::Network(target)
            | BidiEventSource::FileDialog(target) => target != target_id,
            BidiEventSource::Download | BidiEventSource::TargetDiscovery => true,
        });
    }

    pub(super) fn retain_bidi_event_source(&mut self, source: BidiEventSource) {
        if let Some(frontend) = self.bidi_frontend_turn {
            self.bidi_event_sources
                .entry(source)
                .or_default()
                .insert(frontend.id);
        }
    }

    /// Returns true only when the physical listener should be disabled.
    pub(super) fn release_bidi_event_source(&mut self, source: &BidiEventSource) -> bool {
        let Some(frontend) = self.bidi_frontend_turn else {
            return true;
        };
        let Some(owners) = self.bidi_event_sources.get_mut(source) else {
            return false;
        };
        if !owners.remove(&frontend.id) || !owners.is_empty() {
            return false;
        }
        self.bidi_event_sources.remove(source);
        true
    }

    pub(crate) fn register_bidi_session(&mut self, id: u64, session: Option<&str>) {
        self.bidi_sessions.retain(|_, owner| *owner != id);
        if let Some(session) = session {
            self.bidi_sessions.insert(session.to_owned(), id);
        }
    }

    pub(crate) fn bidi_frontend_for_session(&self, session: &str) -> Option<u64> {
        self.bidi_sessions.get(session).copied()
    }

    fn is_bidi_output(&self, event: &moli_protocol::BackgroundProtocolEvent) -> bool {
        !event.is_target_session_control()
            && (self
                .conn
                .is_automation_protocol_session(event.protocol_session_id())
                || event
                    .protocol_session_id()
                    .is_some_and(|id| self.bidi_sessions.contains_key(id)))
    }

    pub(crate) fn next_automation_command_id(&mut self) -> u64 {
        self.conn.next_internal_devtools_command_id()
    }

    pub(super) fn bind_frontend_output(
        &mut self,
    ) -> mpsc::UnboundedReceiver<DevToolsFrontendOutput> {
        let (tx, rx) = mpsc::unbounded_channel();
        assert!(self.frontend_output_tx.replace(tx).is_none());
        rx
    }

    pub(crate) fn set_bidi_frontend_turn(&mut self, frontend: Option<u64>) {
        self.bidi_frontend_turn = frontend.map(|id| BidiFrontendTurn {
            id,
            publish_notifications: true,
        });
    }

    pub(crate) fn set_bidi_observer_turn(&mut self, id: u64) {
        self.bidi_frontend_turn = Some(BidiFrontendTurn {
            id,
            publish_notifications: false,
        });
    }

    pub(crate) fn enqueue_frontend_output(
        &self,
        router: &CdpFrontendRouter,
        output: ProtocolOutputSequence,
    ) -> bool {
        let Some(tx) = &self.frontend_output_tx else {
            return router.enqueue_protocol_output_sequence(output);
        };
        let (automation, cdp): (Vec<_>, Vec<_>) = output
            .into_deliveries()
            .into_iter()
            .partition(|event| self.is_bidi_output(event));
        if !automation.is_empty() {
            let _ = tx.send(DevToolsFrontendOutput::Bidi {
                origin: None,
                output: ProtocolOutputSequence::from_background_events(automation),
            });
        }
        router.enqueue_protocol_output_sequence(ProtocolOutputSequence::from_background_events(cdp))
    }

    pub(crate) fn publish_bidi_protocol_output(
        &self,
        output: ProtocolOutputSequence,
    ) -> ProtocolOutputSequence {
        let Some(tx) = &self.frontend_output_tx else {
            return output;
        };
        let (automation, cdp): (Vec<_>, Vec<_>) = output
            .into_deliveries()
            .into_iter()
            .partition(|event| self.is_bidi_output(event));
        if !cdp.is_empty() {
            let _ = tx.send(DevToolsFrontendOutput::Cdp(
                ProtocolOutputSequence::from_background_events(cdp),
            ));
        }
        if let Some(frontend) = self
            .bidi_frontend_turn
            .filter(|turn| turn.publish_notifications)
        {
            let observations = automation
                .iter()
                .filter(|event| event.is_notification())
                .cloned()
                .collect::<Vec<_>>();
            if !observations.is_empty() {
                let _ = tx.send(DevToolsFrontendOutput::Bidi {
                    origin: Some(frontend.id),
                    output: ProtocolOutputSequence::from_background_events(observations),
                });
            }
        }
        ProtocolOutputSequence::from_background_events(automation)
    }
}
