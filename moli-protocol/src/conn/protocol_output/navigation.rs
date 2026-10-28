use std::{cell::RefCell, rc::Rc};

use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::conn::{
    BackgroundPreparedNavigationCommit, CdpCommandTaskStep, CdpConnection,
    CdpRendererOwnerTurnOutcome, CdpTurnOutcome, CommandDispatchContext, CommandFailureCleanup,
    CommandWorkflowContext, CompletedCommandWorkflow, DocumentNavigationToken,
    NavigationDispatchState, PreparedCommandWorkflow, PreparedDocumentCommitState,
    ProtocolStateAccess, ProtocolWorkflowDriver,
};
use crate::domains::{
    command_output::CommandOutputPlan,
    page::{BackgroundNavigationCompletion, NavigationProtocolCleanup},
};

pub(super) type SharedNavigationResult = Rc<RefCell<Option<CompletedCommandWorkflow>>>;

/// A final publication for the original background owner boundary. Keeping the
/// finite finalizer here preserves response/metadata -> native commit -> load.
pub struct BackgroundNavigationProtocolReady {
    receipt: Rc<BackgroundNavigationPublication>,
    sender: UnboundedSender<BackgroundNavigationCompletion>,
}

struct BackgroundNavigationPublication {
    pending: RefCell<Option<(CompletedCommandWorkflow, CommandDispatchContext)>>,
    metadata: Rc<BackgroundNavigationProtocolMetadata>,
}

struct BackgroundNavigationProtocolMetadata {
    token: DocumentNavigationToken,
    requested_url: url::Url,
    target_id: String,
}

impl BackgroundNavigationProtocolReady {
    pub(crate) fn requested_url(&self) -> &str {
        self.receipt.metadata.requested_url.as_str()
    }
}

impl Drop for BackgroundNavigationProtocolReady {
    fn drop(&mut self) {
        let pending = self.receipt.pending.borrow().is_some();
        if pending && !self.sender.is_closed() {
            // The existing owner channel retains this same finalizer if an
            // accepted ready envelope is dropped before its finite turn.
            // A closed receiver ends that responsibility with the connection.
            let _ = self
                .sender
                .send(BackgroundNavigationCompletion::ProtocolWorkflowReady(
                    Box::new(Self {
                        receipt: self.receipt.clone(),
                        sender: self.sender.clone(),
                    }),
                ));
        }
    }
}

/// Shared native waits reside in the existing protocol-work supervisor, even
/// when a newer navigation removes the old request's background task slot.
#[derive(Clone)]
pub(crate) struct PendingBackgroundNavigationProtocol {
    pending: Shared<LocalBoxFuture<'static, SharedNavigationResult>>,
    context: Rc<RefCell<CommandDispatchContext>>,
    metadata: Rc<BackgroundNavigationProtocolMetadata>,
    sender: UnboundedSender<BackgroundNavigationCompletion>,
}

struct BackgroundNavigationSource {
    pending: Option<(DocumentNavigationToken, NavigationDispatchState)>,
    sender: UnboundedSender<BackgroundNavigationCompletion>,
}

impl BackgroundNavigationSource {
    fn disarm(mut self) {
        self.pending.take();
    }
}

impl Drop for BackgroundNavigationSource {
    fn drop(&mut self) {
        if let Some((token, state)) = self.pending.take() {
            let completion = PreparedDocumentCommitState::new().cancel(token, state);
            let _ = self
                .sender
                .send(BackgroundNavigationCompletion::PreparedDocument(Box::new(
                    completion,
                )));
        }
    }
}

impl PendingBackgroundNavigationProtocol {
    fn prepare(
        conn: &CdpConnection,
        access: ProtocolStateAccess,
        completion: BackgroundPreparedNavigationCommit,
    ) -> Self {
        let source = completion.state.clone();
        let metadata = Rc::new(BackgroundNavigationProtocolMetadata {
            token: completion.token.clone(),
            target_id: completion.token.target_id.clone(),
            requested_url: source.requested_url.clone(),
        });
        let sender = conn
            .background_navigation_completion_sender_for_owner(&source.owner)
            .or_else(|| {
                conn.scheduler_hooks
                    .background_navigation_completion_sender()
            })
            .expect("background completion retains its original owner channel");
        let obligation = Rc::new(RefCell::new(Some(BackgroundNavigationSource {
            pending: Some((completion.token.clone(), source.clone())),
            sender: sender.clone(),
        })));
        let failure_obligation = obligation.clone();
        let cleanup = NavigationProtocolCleanup::default();
        let failure_cleanup = cleanup.clone();
        let failure_access = access.clone();
        let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
            let mut driver = ProtocolWorkflowDriver::owned(access);
            let admitted_obligation = obligation.clone();
            let prepared = context
                .with_state(&mut driver, move |state, context| {
                    let prepared =
                        state.prepare_navigation_protocol_completion(completion, context);
                    if prepared.source_terminal_is_retained()
                        && let Some(obligation) = admitted_obligation.borrow_mut().take()
                    {
                        obligation.disarm();
                    }
                    prepared
                })
                .await;
            match prepared {
                Ok(prepared) => {
                    prepared
                        .run(&mut driver, context, cleanup, move || {
                            if let Some(obligation) = obligation.borrow_mut().take() {
                                obligation.disarm();
                            }
                        })
                        .await
                }
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
        .with_failure_cleanup(move |response| {
            PreparedCommandWorkflow::new(move |mut context| async move {
                failure_cleanup
                    .recover(
                        &mut ProtocolWorkflowDriver::owned(failure_access),
                        &mut context,
                    )
                    .await;
                drop(failure_obligation.borrow_mut().take());
                context.complete(response)
            })
        });
        Self::from_workflow(
            workflow,
            CommandDispatchContext::default(),
            metadata,
            sender,
        )
    }

    fn prepare_materialized(
        conn: &mut CdpConnection,
        access: ProtocolStateAccess,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        navigation: crate::domains::network::MaterializedNavigationLoadOutcome,
    ) -> Self {
        let metadata = Rc::new(BackgroundNavigationProtocolMetadata {
            target_id: token.target_id.clone(),
            requested_url: state.requested_url.clone(),
            token: token.clone(),
        });
        let sender = conn
            .background_navigation_completion_sender_for_owner(&state.owner)
            .or_else(|| {
                conn.scheduler_hooks
                    .background_navigation_completion_sender()
            })
            .expect("background completion retains its original owner channel");
        // This owner turn already has finite access. Retain the original
        // failure/download terminal before handing its native cleanup off.
        let mut output = CommandDispatchContext::default();
        let (mut context, _) = CommandWorkflowContext::take_from(&mut output, None, None);
        conn.settle_background_navigation_completion(&token);
        let prepared = crate::domains::page::PreparedRendererNavigationResult::prepare_materialized(
            conn,
            token,
            state,
            navigation,
            &mut context,
            crate::domains::page::NavigationCommandOutput::Background,
            Vec::new(),
        );
        context.finish_projection_inline(conn, &mut output);
        let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
            if let Err(error) = prepared
                .run(&mut ProtocolWorkflowDriver::owned(access), &mut context)
                .await
            {
                tracing::warn!(
                    ?error,
                    "background navigation cleanup failed after retaining its source terminal"
                );
            }
            context.complete(CommandOutputPlan::default())
        });
        Self::from_workflow(workflow, output, metadata, sender)
    }

    fn from_workflow(
        workflow: PreparedCommandWorkflow,
        mut context: CommandDispatchContext,
        metadata: Rc<BackgroundNavigationProtocolMetadata>,
        sender: UnboundedSender<BackgroundNavigationCompletion>,
    ) -> Self {
        let pending = workflow.start(&mut context, None, None);
        Self {
            pending: async move { Rc::new(RefCell::new(Some(pending.wait().await))) }
                .boxed_local()
                .shared(),
            context: Rc::new(RefCell::new(context)),
            metadata,
            sender,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.metadata.target_id
    }

    pub(crate) async fn wait(&self) -> SharedNavigationResult {
        self.pending.clone().await
    }

    pub(crate) fn hand_off_ready(self, result: SharedNavigationResult) {
        let Some(completed) = result.borrow_mut().take() else {
            return;
        };
        let context = std::mem::take(&mut *self.context.borrow_mut());
        let _ = self
            .sender
            .send(BackgroundNavigationCompletion::ProtocolWorkflowReady(
                Box::new(BackgroundNavigationProtocolReady {
                    receipt: Rc::new(BackgroundNavigationPublication {
                        pending: RefCell::new(Some((completed, context))),
                        metadata: self.metadata,
                    }),
                    sender: self.sender.clone(),
                }),
            ));
    }
}

impl CdpConnection {
    /// Production admission is finite. Legacy callers retain their inline
    /// driver until they also own the protocol-work supervisor and receiver.
    pub fn try_start_background_navigation_completion_turn(
        &mut self,
        completion: BackgroundNavigationCompletion,
    ) -> Result<CdpRendererOwnerTurnOutcome, BackgroundNavigationCompletion> {
        match completion {
            BackgroundNavigationCompletion::ProtocolWorkflowReady(ready) => {
                Ok(self.finish_background_navigation_protocol(*ready))
            }
            BackgroundNavigationCompletion::PreparedDocument(completion) => {
                let Some(access) = self.protocol_state_access() else {
                    return Err(BackgroundNavigationCompletion::PreparedDocument(completion));
                };
                let pending =
                    PendingBackgroundNavigationProtocol::prepare(self, access, *completion);
                self.publish_background_navigation_protocol(pending);
                Ok(CdpTurnOutcome::new_with_protocol_events(
                    Vec::new(),
                    self.take_scheduler_events(),
                )
                .with_renderer_output_predecessor(None))
            }
            BackgroundNavigationCompletion::MainDocumentBody(completion) => {
                completion.record_if_current(self);
                Ok(CdpTurnOutcome::new_with_protocol_events(
                    Vec::new(),
                    self.take_scheduler_events(),
                )
                .with_renderer_output_predecessor(None))
            }
            BackgroundNavigationCompletion::Lifecycle(completion) => {
                let Some(access) = self.protocol_state_access() else {
                    return Err(BackgroundNavigationCompletion::Lifecycle(completion));
                };
                let (token, state, navigation) = completion.materialize(self).into_parts();
                match navigation {
                    crate::domains::network::MaterializedNavigationLoadOutcome::ResponseCommitReady(navigation)
                        if self.accepts_pending_document_navigation_for_owner(&state.owner,&token) => {
                        self.start_background_prepared_navigation_commit(token,state,*navigation);
                    }
                    navigation => {
                        let pending = PendingBackgroundNavigationProtocol::prepare_materialized(self,access,token,state,navigation);
                        self.publish_background_navigation_protocol(pending);
                    }
                }
                Ok(CdpTurnOutcome::new_with_protocol_events(
                    Vec::new(),
                    self.take_scheduler_events(),
                )
                .with_renderer_output_predecessor(None))
            }
        }
    }

    fn publish_background_navigation_protocol(
        &mut self,
        pending: PendingBackgroundNavigationProtocol,
    ) {
        let sequence = self
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work = crate::domains::activity::ProtocolSchedulerWork::navigation_protocol_completion(
            sequence, pending,
        );
        self.scheduler_state
            .push_scheduler_event(crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    pub(in crate::conn) fn finish_background_navigation_protocol(
        &mut self,
        ready: BackgroundNavigationProtocolReady,
    ) -> CdpRendererOwnerTurnOutcome {
        let publication = ready.receipt.pending.borrow_mut().take();
        let Some((completed, mut context)) = publication else {
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                self.take_scheduler_events(),
            )
            .with_renderer_output_predecessor(None);
        };
        let metadata = ready.receipt.metadata.clone();
        let sender = ready.sender.clone();
        let plan = completed.finish(self, &mut context);
        if let Some(cleanup) = context.take_command_failure_cleanup() {
            let CommandFailureCleanup::Continuation(cleanup) = cleanup else {
                unreachable!("background navigation only arms its original workflow cleanup");
            };
            let pending = PendingBackgroundNavigationProtocol::from_workflow(
                cleanup.recover(plan),
                context,
                metadata,
                sender,
            );
            self.publish_background_navigation_protocol(pending);
            return CdpTurnOutcome::new_with_protocol_events(
                Vec::new(),
                self.take_scheduler_events(),
            )
            .with_renderer_output_predecessor(None);
        }
        // Source readiness transfers the original navigation into this
        // workflow; it is not its terminal. Keep command admission waiting on
        // that exact navigation until the final owner publication.
        self.settle_background_navigation_completion(&metadata.token);
        let CdpCommandTaskStep::Complete(outcome) =
            self.complete_with_output_plan(&mut context, plan, None, None)
        else {
            unreachable!("background navigation publication has no remaining wait");
        };
        outcome
    }
}
