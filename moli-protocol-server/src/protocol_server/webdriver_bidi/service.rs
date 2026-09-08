use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::sync::oneshot;

use super::*;

/// A frontend attachment, not a Browser or a second DevTools projection.
pub(crate) struct BidiFrontendAttach {
    pub(super) socket: WebSocket,
    pub(super) web_socket_url: String,
    pub(super) session_registry: SharedBidiSessionRegistry,
    pub(super) finished_tx: oneshot::Sender<()>,
}

struct BidiFrontend {
    actor: BidiSocketActor,
    registry: SharedBidiSessionRegistry,
    finished_tx: oneshot::Sender<()>,
}

#[derive(Default)]
pub(crate) struct BidiServiceFrontends {
    next_id: u64,
    frontends: BTreeMap<u64, BidiFrontend>,
}

impl BidiServiceFrontends {
    pub(crate) fn attach(&mut self, attach: BidiFrontendAttach) {
        let id = self.next_id;
        self.next_id = id.checked_add(1).expect("BiDi frontend id space exhausted");
        self.frontends.insert(
            id,
            BidiFrontend {
                actor: BidiSocketActor::new(attach.socket, attach.web_socket_url),
                registry: attach.session_registry,
                finished_tx: attach.finished_tx,
            },
        );
    }

    pub(crate) async fn recv(&mut self) -> (u64, BidiSocketActorInput) {
        let mut inputs = self.frontends.iter_mut().map(|(&id, frontend)| async move {
            let input = tokio::select! {
                biased;
                message = frontend.actor.socket.recv() => BidiSocketActorInput::Socket(message),
                completed = recv_bidi_navigation_completion(&mut frontend.actor.pending_command) => {
                    BidiSocketActorInput::NavigationCompletion(completed.map(Box::new))
                }
            };
            (id, input)
        }).collect::<FuturesUnordered<_>>();
        match inputs.next().await {
            Some(input) => input,
            None => std::future::pending().await,
        }
    }

    pub(crate) fn runtime_response_owner(
        &self,
        response: &RuntimeInspectorResponseReady,
    ) -> Option<u64> {
        self.frontends.iter().find_map(|(&id, frontend)| {
            frontend
                .actor
                .pending_command
                .as_ref()
                .and_then(BidiPendingCommand::runtime_pending)
                .is_some_and(|pending| pending.command_id() == response.command_id())
                .then_some(id)
        })
    }

    pub(crate) async fn handle_input(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        id: u64,
        input: BidiSocketActorInput,
    ) -> bool {
        let Some(frontend) = self.frontends.get_mut(&id) else {
            return false;
        };
        scheduler.set_bidi_frontend_turn(Some(id));
        let keep = match input {
            BidiSocketActorInput::Socket(Some(message)) => {
                frontend
                    .actor
                    .handle_socket_message(scheduler, receivers, &frontend.registry, message)
                    .await
            }
            BidiSocketActorInput::Socket(None)
            | BidiSocketActorInput::RuntimeResponseReady(None) => false,
            BidiSocketActorInput::NavigationCompletion(completed) => {
                frontend
                    .actor
                    .handle_navigation_completion(scheduler, receivers, completed)
                    .await
            }
            BidiSocketActorInput::RuntimeResponseReady(Some(response)) => {
                frontend
                    .actor
                    .handle_runtime_response_ready(scheduler, receivers, *response)
                    .await
            }
            BidiSocketActorInput::AdapterScheduler(_) => {
                unreachable!("the shared owner advances its adapter scheduler")
            }
        };
        scheduler.set_bidi_frontend_turn(None);
        scheduler.register_bidi_session(id, frontend.actor.bidi.session_id());
        if !keep {
            self.detach(scheduler, receivers, id).await;
        }
        !keep
    }

    pub(crate) fn send_outputs<'a>(
        &'a mut self,
        scheduler: &'a mut CdpScheduler,
        receivers: &'a mut CdpSchedulerEventReceivers,
        batches: Vec<(Option<u64>, ProtocolOutputSequence)>,
    ) -> futures_util::future::LocalBoxFuture<'a, bool> {
        async move {
            let mut outputs: BTreeMap<u64, Vec<BackgroundProtocolEvent>> = BTreeMap::new();
            for (origin, output) in batches {
                for event in output.into_deliveries() {
                    let session_owner = event
                        .protocol_session_id()
                        .and_then(|session| scheduler.bidi_frontend_for_session(session));
                    if event.is_notification() {
                        // Fan out only frozen notifications, never command completions
                        // or the renderer's linear transport/release capabilities.
                        for &id in self.frontends.keys() {
                            if Some(id) != origin && session_owner.is_none_or(|owner| owner == id) {
                                outputs.entry(id).or_default().push(event.clone());
                            }
                        }
                    } else {
                        let response_owner = event
                            .as_runtime_inspector_response_ready()
                            .and_then(|response| self.runtime_response_owner(response))
                            .or_else(|| {
                                event.protocol_message_id().and_then(|command_id| {
                                    self.frontends.iter().find_map(|(&id, frontend)| {
                                        frontend
                                            .actor
                                            .pending_navigation_response
                                            .as_ref()
                                            .or_else(|| {
                                                frontend.actor.pending_command.as_ref().and_then(
                                                    |pending| {
                                                        pending
                                                            .pending_navigation_candidate
                                                            .as_ref()
                                                    },
                                                )
                                            })
                                            .is_some_and(|pending| {
                                                pending.background_command_id == command_id
                                            })
                                            .then_some(id)
                                    })
                                })
                            })
                            .or(session_owner);
                        if let Some(id) = response_owner.filter(|id| Some(*id) != origin) {
                            outputs.entry(id).or_default().push(event);
                        }
                    }
                }
            }
            let mut detached = Vec::new();
            for (&id, frontend) in &mut self.frontends {
                let output = ProtocolOutputSequence::from_background_events(
                    outputs.remove(&id).unwrap_or_default(),
                );
                scheduler.set_bidi_observer_turn(id);
                if !frontend
                    .actor
                    .send_or_route_protocol_output(scheduler, receivers, output, None)
                    .await
                {
                    detached.push(id);
                }
                scheduler.set_bidi_frontend_turn(None);
            }
            let any_detached = !detached.is_empty();
            for id in detached {
                self.detach(scheduler, receivers, id).await;
            }
            any_detached
        }
        .boxed_local()
    }

    async fn detach(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        id: u64,
    ) {
        let Some(mut frontend) = self.frontends.remove(&id) else {
            return;
        };
        scheduler.set_bidi_frontend_turn(Some(id));
        frontend
            .actor
            .release_event_sources(scheduler, receivers)
            .await;
        scheduler.set_bidi_frontend_turn(None);
        frontend
            .actor
            .release_session(&mut frontend.registry.lock());
        scheduler.register_bidi_session(id, None);
        let _ = frontend.finished_tx.send(());
    }

    pub(crate) async fn shutdown(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
    ) {
        while let Some(&id) = self.frontends.keys().next() {
            self.detach(scheduler, receivers, id).await;
        }
    }
}
