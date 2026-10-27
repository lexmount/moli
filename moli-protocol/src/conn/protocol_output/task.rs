use std::{cell::Cell, rc::Rc};

use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use super::*;
#[cfg(test)]
use crate::conn::BackgroundProtocolEvent;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};
use crate::domains::activity::{ProtocolSchedulerWork, ProtocolSchedulerWorkKind};
use crate::domains::page::{CompletedTargetTermination, TargetTermination};
use crate::domains::target::activation::TargetActivationOutput;

/// Owned protocol work submitted to the adapter's existing task supervisor.
/// Clones retain the same native waits and final publication authority.
#[derive(Clone)]
pub struct PendingProtocolSchedulerWork {
    work: PendingProtocolWork,
    access: ProtocolStateAccess,
}

#[derive(Clone)]
enum PendingProtocolWork {
    WorkerRendererProjection(super::renderer::PendingWorkerRendererProjection),
    Termination(TargetTermination),
    PopupActivation(PendingPopupActivation),
    BidiChannel(PendingBidiChannel),
    Navigation(super::navigation::PendingBackgroundNavigationProtocol),
}

#[derive(Clone)]
struct PendingPopupActivation {
    pending: Shared<LocalBoxFuture<'static, Option<TargetActivationOutput>>>,
    action: Rc<crate::conn::PopupTargetActivationAction>,
    committed: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct PendingBidiChannel {
    pending: Shared<LocalBoxFuture<'static, crate::conn::BidiChannelOutput>>,
    committed: Rc<Cell<bool>>,
}

impl PendingBidiChannel {
    fn prepare(
        conn: &mut CdpConnection,
        action: crate::conn::BidiChannelOwnerAction,
        access: ProtocolStateAccess,
    ) -> Option<Self> {
        let prepared = crate::conn::PreparedBidiChannelAction::prepare(conn, action)?;
        let pending = async move {
            prepared
                .run(&mut ProtocolWorkflowDriver::owned(access))
                .await
        }
        .boxed_local()
        .shared();
        Some(Self {
            pending,
            committed: Rc::new(Cell::new(false)),
        })
    }
}

impl PendingPopupActivation {
    fn prepare(
        conn: &mut CdpConnection,
        action: crate::conn::PopupTargetActivationAction,
        access: ProtocolStateAccess,
    ) -> Option<Self> {
        let prepared = match crate::domains::target::prepare_popup_target_activation(conn, &action)
        {
            Ok(prepared) => prepared?,
            Err(error) => {
                tracing::debug!(?error, "popup target could not be activated");
                return None;
            }
        };
        let pending = async move {
            let mut driver = ProtocolWorkflowDriver::owned(access);
            match prepared.run(&mut driver).await {
                Ok(output) => Some(output),
                Err(error) => {
                    tracing::debug!(%error, "popup target could not be activated");
                    None
                }
            }
        }
        .boxed_local()
        .shared();
        Some(Self {
            pending,
            action: Rc::new(action),
            committed: Rc::new(Cell::new(false)),
        })
    }
}

pub struct CompletedProtocolSchedulerWork {
    work: CompletedProtocolWork,
}

enum CompletedProtocolWork {
    WorkerRendererProjection(
        super::renderer::PendingWorkerRendererProjection,
        super::renderer::SharedRendererProjectionResult,
    ),
    Termination(Box<CompletedTargetTermination>),
    PopupActivation(PendingPopupActivation, Option<TargetActivationOutput>),
    BidiChannel(PendingBidiChannel, crate::conn::BidiChannelOutput),
    Navigation(
        super::navigation::PendingBackgroundNavigationProtocol,
        super::navigation::SharedNavigationResult,
    ),
}

impl std::fmt::Debug for PendingProtocolSchedulerWork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.work {
            PendingProtocolWork::WorkerRendererProjection(_) => "WorkerRendererProjection",
            PendingProtocolWork::Termination(_) => "Termination",
            PendingProtocolWork::PopupActivation(_) => "PopupActivation",
            PendingProtocolWork::BidiChannel(_) => "BidiChannel",
            PendingProtocolWork::Navigation(_) => "Navigation",
        };
        formatter
            .debug_struct("PendingProtocolSchedulerWork")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}

impl CompletedProtocolSchedulerWork {
    pub(crate) fn for_termination(completed: CompletedTargetTermination) -> Self {
        Self {
            work: CompletedProtocolWork::Termination(Box::new(completed)),
        }
    }
}

impl PendingProtocolSchedulerWork {
    pub(super) fn for_worker_renderer_projection(
        pending: super::renderer::PendingWorkerRendererProjection,
        access: ProtocolStateAccess,
    ) -> Self {
        Self {
            work: PendingProtocolWork::WorkerRendererProjection(pending),
            access,
        }
    }

    /// The adapter and finite admission share this migration boundary.
    pub fn handles(kind: ProtocolSchedulerWorkKind) -> bool {
        matches!(
            kind,
            ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction
                | ProtocolSchedulerWorkKind::PopupTargetActivationAction
                | ProtocolSchedulerWorkKind::BidiChannelOwnerAction
                | ProtocolSchedulerWorkKind::NavigationProtocolCompletionOwnerAction
        )
    }

    pub async fn wait(self) -> CompletedProtocolSchedulerWork {
        match self.work {
            PendingProtocolWork::WorkerRendererProjection(projection) => {
                let result = projection.wait().await;
                CompletedProtocolSchedulerWork {
                    work: CompletedProtocolWork::WorkerRendererProjection(projection, result),
                }
            }
            PendingProtocolWork::Navigation(navigation) => {
                let ready = navigation.wait().await;
                CompletedProtocolSchedulerWork {
                    work: CompletedProtocolWork::Navigation(navigation, ready),
                }
            }
            PendingProtocolWork::BidiChannel(channel) => {
                let events = channel.pending.clone().await;
                CompletedProtocolSchedulerWork {
                    work: CompletedProtocolWork::BidiChannel(channel, events),
                }
            }
            PendingProtocolWork::Termination(termination) => {
                let completed = termination
                    .run(&mut ProtocolWorkflowDriver::owned(self.access))
                    .await;
                CompletedProtocolSchedulerWork::for_termination(completed)
            }
            PendingProtocolWork::PopupActivation(activation) => {
                let output = activation.pending.clone().await;
                CompletedProtocolSchedulerWork {
                    work: CompletedProtocolWork::PopupActivation(activation, output),
                }
            }
        }
    }
}

impl CdpConnection {
    /// Unsupported work stays with its existing driver during migration.
    pub fn try_start_owned_protocol_scheduler_work(
        &mut self,
        work: ProtocolSchedulerWork,
    ) -> Result<Option<PendingProtocolSchedulerWork>, Box<ProtocolSchedulerWork>> {
        let Some(access) = self.protocol_state_access() else {
            return Err(Box::new(work));
        };
        if !PendingProtocolSchedulerWork::handles(work.kind()) {
            return Err(Box::new(work));
        }
        let prepared = match work.into_ready() {
            ReadyProtocolSchedulerWork::NavigationProtocolCompletion(work) => {
                Some(PendingProtocolWork::Navigation(work))
            }
            ReadyProtocolSchedulerWork::PageTargetTerminationOwnerAction(action) => {
                TargetTermination::prepare(self, action).map(PendingProtocolWork::Termination)
            }
            ReadyProtocolSchedulerWork::PopupTargetActivationAction(action) => {
                PendingPopupActivation::prepare(self, action, access.clone())
                    .map(PendingProtocolWork::PopupActivation)
            }
            ReadyProtocolSchedulerWork::BidiChannelOwnerAction(action) => {
                PendingBidiChannel::prepare(self, action, access.clone())
                    .map(PendingProtocolWork::BidiChannel)
            }
            _ => unreachable!("the work kind identifies its owned payload"),
        };
        Ok(prepared.map(|work| PendingProtocolSchedulerWork { work, access }))
    }

    pub fn complete_owned_protocol_scheduler_work(
        &mut self,
        completed: CompletedProtocolSchedulerWork,
    ) -> CdpTurnOutcome {
        match completed.work {
            CompletedProtocolWork::WorkerRendererProjection(projection, result) => {
                projection.finish(self, result)
            }
            CompletedProtocolWork::Navigation(navigation, ready) => {
                navigation.hand_off_ready(ready);
                CdpTurnOutcome::new_with_protocol_events(Vec::new(), self.take_scheduler_events())
            }
            CompletedProtocolWork::BidiChannel(channel, events) => {
                // Native state completion already authorized and materialized
                // these envelopes. Retain their original payloads/routes;
                // retiring the Page cannot discard the entire output prefix.
                let events = if !channel.committed.replace(true) {
                    events.publish(self)
                } else {
                    Vec::new()
                };
                CdpTurnOutcome::new_with_protocol_events(events, self.take_scheduler_events())
            }
            CompletedProtocolWork::Termination(termination) => termination.finish(self),
            CompletedProtocolWork::PopupActivation(activation, output) => {
                let events =
                    if !activation.committed.replace(true) && activation.action.is_current(self) {
                        output.map(|output| output.finish(self)).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                CdpTurnOutcome::new_with_protocol_events(events, self.take_scheduler_events())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand,
        PopupTargetActivationAction, ProtocolStateOwner,
    };
    use crate::domains::activity::ProtocolWorkPublishSequence;
    use crate::testing::TestContext;
    use serde_json::json;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn bidi_listener_rejoins_original_startup_after_cancellation_and_ready_loss() {
        let test = async {
            for hold_after_ready in [false, true] {
                let mut context = fixture().await;
                let command_owner = crate::conn::CommandOwnerScope::for_session("SID-popup-first");
                let mut driver = ProtocolWorkflowDriver::inline(&mut context.conn);
                let realm =
                    crate::conn::runtime_realm_inventory_using(&mut driver, command_owner.clone())
                        .await
                        .unwrap()
                        .into_iter()
                        .find(|realm| realm.is_default != Some(false))
                        .unwrap();
                let response = crate::conn::dispatch_runtime_helper_using(&mut driver, command_owner.clone(), "Runtime.evaluate", json!({
                    "expression":"globalThis.__ownedRecoveryChannel = {getMessage() { return new Promise(resolve => { this.resolve = resolve; }); }, sendMessage(value) { this.resolve(value); }}; globalThis.__ownedRecoveryChannel",
                    "objectGroup":"owned-recovery-channel", "returnByValue":false,
                })).await.unwrap();
                let crate::conn::BackgroundCommandResponsePayload::Success { result } = response
                else {
                    panic!("the proxy must exist");
                };
                let listener = crate::conn::PendingBidiChannelListener::new(
                    Some("TID-popup-first".into()),
                    realm.realm_id,
                    result["result"]["objectId"].as_str().unwrap().into(),
                    "owned-recovery-channel".into(),
                    crate::automation::DevToolsBidiChannelProperties {
                        channel: "recovery-channel".into(),
                        ownership: crate::automation::DevToolsResultOwnership::None,
                        serialization_options: None,
                    },
                )
                .unwrap();
                let page_owner = crate::conn::BidiChannelPageOwner::capture_for_owner(
                    &context.conn,
                    command_owner.clone(),
                )
                .unwrap();
                let action = crate::conn::BidiChannelOwnerAction::start_listener(
                    crate::conn::BidiChannelListenerResidence::new(page_owner, listener),
                );
                let (tx, mut replies) = tokio::sync::mpsc::unbounded_channel();
                context.conn.set_runtime_inspector_response_ready_sender(tx);
                drop(context.conn.take_protocol_state_requests());
                let owner = ProtocolStateOwner::new(context.conn);
                let handle = owner.handle();
                let access = ProtocolStateAccess::Direct(handle.clone());
                let work = ProtocolSchedulerWork::bidi_channel_owner_action(
                    ProtocolWorkPublishSequence::new(1),
                    action,
                );
                let mut pending = handle
                    .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                    .unwrap()
                    .unwrap_or_else(|_| {
                        panic!("the production adapter must admit channel owner work")
                    })
                    .unwrap();
                let PendingProtocolWork::BidiChannel(channel) = &mut pending.work else {
                    panic!("the original work retains its kind");
                };
                let actual = channel.pending.clone();
                let (entered, ready) = oneshot::channel();
                let (release, held) = oneshot::channel();
                let runs = Rc::new(Cell::new(0));
                let actual_runs = runs.clone();
                channel.pending = async move {
                    actual_runs.set(actual_runs.get() + 1);
                    if hold_after_ready {
                        let events = actual.await;
                        entered.send(()).unwrap();
                        held.await.unwrap();
                        events
                    } else {
                        entered.send(()).unwrap();
                        held.await.unwrap();
                        actual.await
                    }
                }
                .boxed_local()
                .shared();
                let mut wait = Box::pin(pending.clone().wait());
                tokio::select! {
                    _ = &mut wait => panic!("the controlled native handoff is held"),
                    result = ready => result.unwrap(),
                }
                let version =
                    ParsedCdpCommand::parse_str(r#"{"id":50,"method":"Browser.getVersion"}"#)
                        .unwrap();
                assert!(matches!(
                    handle
                        .with_state(|state| state.start_parsed_command_dispatch_with_context(
                            &version,
                            &mut CommandDispatchContext::default()
                        ))
                        .unwrap(),
                    CdpCommandTaskStep::Complete(_)
                ));
                drop(wait);
                release.send(()).unwrap();
                drop(pending.clone().wait().await);
                let completed = pending.clone().wait().await;
                handle
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                    .unwrap();
                let repeated = pending.wait().await;
                assert!(
                    handle
                        .with_state(|state| state.complete_owned_protocol_scheduler_work(repeated))
                        .unwrap()
                        .into_parts()
                        .0
                        .is_empty()
                );
                assert_eq!(runs.get(), 1);
                crate::conn::dispatch_runtime_helper_using(&mut ProtocolWorkflowDriver::owned(access.clone()), command_owner, "Runtime.evaluate", json!({"expression":"__ownedRecoveryChannel.sendMessage('once')","returnByValue":true})).await.unwrap();
                let response =
                    tokio::time::timeout(std::time::Duration::from_secs(10), replies.recv())
                        .await
                        .unwrap()
                        .unwrap();
                access.defer_cleanup(Box::new(move |conn| {
                    let mut replies = Vec::new();
                    let mut messages = Vec::new();
                    conn.route_registered_runtime_inspector_response_into(response, &mut replies, &mut messages);
                    assert!(replies.is_empty());
                    assert_eq!(messages.len(), 1, "the original long listener owns one message");
                    let (_, Some(crate::automation::AutomationEvent::ScriptMessage(message))) = messages.pop().unwrap().into_parts() else { panic!("the original listener produces a channel event"); };
                    assert_eq!(message.channel, "recovery-channel");
                    assert_eq!(message.data.value, json!("once"));
                    let restarts = conn.take_scheduler_events().into_iter().filter(|event| matches!(event, crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.bidi_channel_owner_action_kind() == Some(crate::conn::BidiChannelOwnerActionKind::StartListener))).count();
                    assert_eq!(restarts, 1, "recovery cannot add a second listener");
                }));
                assert!(matches!(
                    replies.try_recv(),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                ));
            }
        };
        tokio::task::LocalSet::new().run_until(test).await;
    }

    #[tokio::test]
    async fn bidi_materialized_output_survives_page_retirement_and_ready_loss_once() {
        let mut context = fixture().await;
        let command_owner = crate::conn::CommandOwnerScope::for_session("SID-popup-first");
        let page_owner =
            crate::conn::BidiChannelPageOwner::capture_for_owner(&context.conn, command_owner)
                .unwrap();
        let action = crate::conn::BidiChannelOwnerAction::release_object_group(
            page_owner.clone(),
            "completed-channel-group",
        );
        drop(context.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(context.conn);
        let handle = owner.handle();
        let access = ProtocolStateAccess::Direct(handle.clone());
        let work = ProtocolSchedulerWork::bidi_channel_owner_action(
            ProtocolWorkPublishSequence::new(1),
            action,
        );
        let mut pending = handle
            .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
            .unwrap()
            .unwrap_or_else(|_| panic!("group cleanup uses the original owner-work supervisor"))
            .unwrap();
        let PendingProtocolWork::BidiChannel(channel) = &mut pending.work else {
            panic!("the original work retains its channel kind");
        };
        let realm = crate::automation::DevToolsRealmId::from("original-realm");
        let fact = BackgroundProtocolEvent::immediate_automation_event(
            json!({"method":"Moli.scriptMessage","params":{}}),
            crate::automation::AutomationEvent::ScriptMessage(
                crate::automation::ScriptMessageEvent {
                    target_id: Some("TID-popup-first".into()),
                    realm_id: Some(realm.clone()),
                    channel: "completed-channel".into(),
                    data: crate::automation::DevToolsRemoteValue::from_cdp_remote_object(
                        &json!({"type":"string","value":"already materialized"}),
                        false,
                        Some(realm),
                    ),
                },
            ),
        );
        let expected = fact.clone();
        let actual = channel.pending.clone();
        // Model an already materialized typed fact in the native turn prefix.
        // A recovery wait owns this exact envelope, not permission to recreate
        // it against whichever Page happens to be current later.
        channel.pending = async move {
            let mut events = actual.await;
            events.events.push(fact);
            events
        }
        .boxed_local()
        .shared();
        let ready = pending.clone().wait().await;
        let CompletedProtocolWork::BidiChannel(_, output) = &ready.work else {
            panic!("the ready result retains its channel output");
        };
        let expected_cursor = output
            .predecessor
            .as_ref()
            .expect("the real Native receipt owns an ordering lease")
            .cursor();
        drop(ready);
        access.defer_cleanup(Box::new(move |conn| {
            conn.runtime_session_owner_slot_mut(Some("SID-popup-first"))
                .unwrap()
                .replace_page_attachment_id_for_test();
            assert!(!page_owner.is_current(conn));
        }));
        let completed = pending.clone().wait().await;
        let (events, scheduled) = handle
            .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
            .unwrap()
            .into_protocol_event_parts();
        assert!(
            events.is_empty(),
            "the fact must remain fenced rather than bypassing projection"
        );
        let work = scheduled
            .into_iter()
            .find_map(|event| match event {
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work }
                    if work.renderer_output_predecessor().is_some() =>
                {
                    Some(work)
                }
                _ => None,
            })
            .expect("recovery publishes the original fenced fact");
        assert_eq!(
            work.renderer_output_predecessor().unwrap().cursor(),
            expected_cursor
        );
        assert_eq!(work.navigation_gate_target_id(), Some("TID-popup-first"));
        let ReadyProtocolSchedulerWork::RendererFencedOutput(output) = work.into_ready() else {
            panic!("a materialized fact cannot re-admit Native work");
        };
        assert_eq!(
            output.events,
            vec![expected],
            "the exact payload and route survive Page retirement"
        );
        let repeated = pending.wait().await;
        assert!(
            handle
                .with_state(|state| state.complete_owned_protocol_scheduler_work(repeated))
                .unwrap()
                .into_protocol_event_parts()
                .0
                .is_empty()
        );
    }

    #[tokio::test]
    async fn materialized_output_waits_for_projection_of_its_original_renderer_cursor() {
        use moli_core::{
            PageId, RendererOutputCursor, RendererOutputFence, RendererOutputItem,
            RendererOutputPublication, RendererOutputRecord, RendererOutputStreamControl,
            RendererOutputStreamIdentity, RendererOutputTransportMessage,
            RendererProtocolObservation,
        };
        let mut conn = CdpConnection::default();
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(77));
        let renderer_page =
            crate::conn::RendererPageResidenceIdentity::from_residence(stream.residence()).unwrap();
        let page_owner = crate::conn::TargetPageResidenceIdentity::new_for_test(
            "BID-fenced-fact".into(),
            Some("TID-fenced-fact".into()),
            1,
        );
        conn.bind_renderer_page_output_owner(renderer_page, page_owner);
        conn.apply_renderer_output_stream_control(RendererOutputStreamControl::Opened { stream });
        let cursor = RendererOutputCursor::new_for_test(stream, 2);
        let fact = BackgroundProtocolEvent::immediate(
            json!({"method":"Moli.originalFact","params":{"targetId":"TID-fenced-fact"}}),
        );
        conn.publish_renderer_fenced_protocol_output(
            vec![fact.clone()],
            RendererOutputFence::new_for_test(cursor),
            Some("TID-fenced-fact".into()),
        );
        let work = conn
            .take_scheduler_events()
            .into_iter()
            .find_map(|event| match event {
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } => Some(work),
                _ => None,
            })
            .unwrap();
        assert!(!work.renderer_output_predecessor_is_projected(&conn));
        let publication = |sequence| {
            RendererOutputPublication::new_for_test(
                RendererOutputCursor::new_for_test(stream, sequence),
                vec![RendererOutputRecord::new_for_test(
                    RendererOutputItem::Observation(
                        RendererProtocolObservation::RuntimeLifecycleError {
                            text: "historical renderer fact".into(),
                            execution_context_id: None,
                        },
                    ),
                )],
            )
        };
        assert!(matches!(
            conn.admit_renderer_output_publication(publication(2)),
            crate::domains::activity::RendererOutputIngressAdmission::Buffered
        ));
        assert!(
            !work.renderer_output_predecessor_is_projected(&conn),
            "transport admission cannot substitute for typed projection"
        );
        // Fill the original FIFO gap through the production ingress. Both
        // nonempty batches are projected against their retired owner rather
        // than manufacturing projection completion in the test.
        let projected = conn
            .ingest_renderer_output_turn_async(RendererOutputTransportMessage::Publication(
                publication(1),
            ))
            .await;
        assert!(projected.into_protocol_event_parts().0.is_empty());
        assert!(work.renderer_output_predecessor_is_projected(&conn));
        let output = conn.complete_ready_protocol_scheduler_work_turn(work).await;
        assert_eq!(output.into_protocol_event_parts().0, vec![fact]);
    }

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-popup-owned".into());
        browser.set_active_target_id("TID-popup-first");
        browser.attach_active_session("SID-popup-first");
        browser.stage_background_target(
            "TID-popup-second".into(),
            Some("SID-popup-second".into()),
            "about:blank".into(),
            None,
            None,
        );
        context
            .conn
            .install_browser_context_fixture_for_test(browser);
        for session in ["SID-popup-first", "SID-popup-second"] {
            context
                .install_navigation_fixture_for_session_owner("about:blank", Some(session))
                .await;
        }
        context
    }

    #[tokio::test]
    async fn popup_activation_rejoins_admitted_work_after_cancellation_and_ready_loss() {
        for hold_after_ready in [false, true] {
            let mut context = fixture().await;
            for (id, session) in [(1, "SID-popup-first"), (2, "SID-popup-second")] {
                context.process_async(json!({"id":id,"sessionId":session,"method":"Page.startScreencast","params":{}})).await;
                assert!(context.take_response_by_id(id).get("result").is_some());
            }
            let action = PopupTargetActivationAction::capture(
                &context.conn,
                "BID-popup-owned",
                "TID-popup-second",
            )
            .unwrap();
            drop(context.conn.take_protocol_state_requests());
            let owner = ProtocolStateOwner::new(context.conn);
            let handle = owner.handle();
            let work = ProtocolSchedulerWork::popup_target_activation_action(
                ProtocolWorkPublishSequence::new(1),
                action,
            );
            let mut pending = handle
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap_or_else(|_| {
                    panic!("popup activation must use the existing owned-work supervisor")
                })
                .expect("the live popup owns its surface update");
            let PendingProtocolWork::PopupActivation(activation) = &mut pending.work else {
                panic!("popup work keeps its activation kind");
            };
            let actual = activation.pending.clone();
            let (entered, ready) = oneshot::channel();
            let (release, held) = oneshot::channel();
            let runs = Rc::new(Cell::new(0));
            let actual_runs = runs.clone();
            activation.pending = async move {
                actual_runs.set(actual_runs.get() + 1);
                if hold_after_ready {
                    let output = actual.await;
                    entered.send(()).unwrap();
                    held.await.unwrap();
                    output
                } else {
                    entered.send(()).unwrap();
                    held.await.unwrap();
                    actual.await
                }
            }
            .boxed_local()
            .shared();
            let mut wait = Box::pin(pending.clone().wait());
            tokio::select! {
                _ = &mut wait => panic!("the controlled native handoff is still held"),
                result = ready => result.unwrap(),
            }
            let version =
                ParsedCdpCommand::parse_str(r#"{"id":50,"method":"Browser.getVersion"}"#).unwrap();
            assert!(matches!(
                handle
                    .with_state(|state| state.start_parsed_command_dispatch_with_context(
                        &version,
                        &mut CommandDispatchContext::default()
                    ))
                    .unwrap(),
                CdpCommandTaskStep::Complete(_)
            ));
            drop(wait);
            release.send(()).unwrap();
            // A ready result can be discarded before the actor consumes it.
            // The supervised work still owns the same native result and events.
            drop(pending.clone().wait().await);
            let completed = pending.clone().wait().await;
            let messages = handle
                .with_state(|state| state.complete_owned_protocol_scheduler_work(completed))
                .unwrap()
                .into_parts()
                .0;
            let visible: Vec<_> = messages
                .iter()
                .filter(|event| event["method"] == "Page.screencastVisibilityChanged")
                .map(|event| json!([event["sessionId"], event["params"]["visible"]]))
                .collect();
            assert_eq!(
                visible,
                vec![
                    json!(["SID-popup-first", false]),
                    json!(["SID-popup-second", true])
                ]
            );
            assert_eq!(runs.get(), 1, "recovery cannot repeat native admission");
            let repeated = pending.wait().await;
            assert!(
                handle
                    .with_state(|state| state.complete_owned_protocol_scheduler_work(repeated))
                    .unwrap()
                    .into_parts()
                    .0
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn popup_activation_cannot_select_a_recreated_target_with_the_same_id() {
        let mut context = fixture().await;
        let action = PopupTargetActivationAction::capture(
            &context.conn,
            "BID-popup-owned",
            "TID-popup-second",
        )
        .unwrap();
        let old = context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .take_page_target_for_close("TID-popup-second")
            .unwrap();
        context
            .conn
            .browser_context
            .as_mut()
            .unwrap()
            .insert_page_target_host(crate::conn::PageTargetHost::with_url(
                "TID-popup-second".into(),
                Some("SID-popup-second".into()),
                "about:blank".into(),
            ));
        context
            .install_navigation_fixture_for_session_owner("about:blank", Some("SID-popup-second"))
            .await;
        assert!(!action.is_current(&context.conn));
        let work = ProtocolSchedulerWork::popup_target_activation_action(
            ProtocolWorkPublishSequence::new(1),
            action,
        );
        drop(context.conn.take_protocol_state_requests());
        let owner = ProtocolStateOwner::new(context.conn);
        assert!(
            owner
                .handle()
                .with_state(|state| state.try_start_owned_protocol_scheduler_work(work))
                .unwrap()
                .unwrap_or_else(|_| panic!("a stale popup action is consumed without legacy work"))
                .is_none()
        );
        drop(old);
    }
}
