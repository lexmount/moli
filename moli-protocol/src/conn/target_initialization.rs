//! One supervised initial-document workflow per Target. Callers hold only
//! shared receipts; dropping a Session or command cannot drop the public work.
use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use super::runtime_load::LoadedPageCreationDiagnosticsParts;
use super::*;
use crate::automation::{DevToolsError, DevToolsErrorKind};

type InitializationResult = Result<Rc<LoadedPageCreationDiagnosticsParts>, DevToolsError>;

#[derive(Debug, thiserror::Error)]
#[error("{}", .0.message)]
struct InitializationProtocolError(DevToolsError);

#[derive(Clone)]
pub(crate) struct TargetInitialDocumentWaiter {
    completion: Shared<LocalBoxFuture<'static, InitializationResult>>,
    installed: Rc<RefCell<Option<InitializationResult>>>,
    output: Rc<RefCell<Option<TargetInitializationOutput>>>,
    publication_released: tokio::sync::watch::Receiver<bool>,
}

pub(crate) struct TargetInitializationOutput {
    completed: CompletedCommandWorkflow,
    output: CommandDispatchContext,
    target: String,
    host: Option<PageTargetHostIdentity>,
    predecessor: Option<moli_core::RendererOutputCursor>,
}

pub(crate) struct TargetInitializationOutputClaim {
    output: Rc<RefCell<Option<TargetInitializationOutput>>>,
    release: tokio::sync::watch::Sender<bool>,
}

impl Drop for TargetInitializationOutputClaim {
    fn drop(&mut self) {
        self.release.send_replace(true);
    }
}

impl TargetInitializationOutputClaim {
    pub(crate) fn publish(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: Option<&mut CommandWorkflowContext>,
    ) -> Result<()> {
        let output = self.output.borrow_mut().take();
        if let Some(output) = output {
            match context {
                Some(context) => context.with_state(driver, move |state, context| {
                    state.finish_target_initialization_into_command(output, context.context_mut())
                })?,
                None => driver.with_state(move |state| {
                    state.finish_target_initialization_publication(output)
                })?,
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for TargetInitialDocumentWaiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetInitialDocumentWaiter")
            .finish_non_exhaustive()
    }
}

impl TargetInitialDocumentWaiter {
    pub(crate) async fn wait(self) -> InitializationResult {
        self.completion.await
    }

    pub(crate) async fn wait_page(mut self) -> InitializationResult {
        futures_util::future::poll_fn(move |cx| {
            if let Some(result) = self.installed.borrow().as_ref() {
                return std::task::Poll::Ready(result.clone());
            }
            if let std::task::Poll::Ready(result) = self.completion.poll_unpin(cx) {
                return std::task::Poll::Ready(result);
            }
            match self.installed.borrow().as_ref() {
                Some(result) => std::task::Poll::Ready(result.clone()),
                None => std::task::Poll::Pending,
            }
        })
        .await
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TargetInitialDocumentWork {
    /// Browser attachments and creation inputs must be committed first.
    Staged,
    Running(TargetInitialDocumentWaiter),
}

pub(crate) enum InitialDocumentContinuation {
    Navigate,
    Popup(Option<moli_core::RendererOutputCursor>),
    Creation(crate::domains::target::TargetCreationCommit),
    Deferred(tokio::sync::oneshot::Receiver<()>),
}

impl CdpConnection {
    pub(crate) fn target_can_start_initial_navigation(&self, owner: &CommandOwnerScope) -> bool {
        !self.owner_target_has_waiting_for_debugger_session(owner)
            && self.runtime_session_owner_needs_initial_document_navigation_for_owner(owner)
    }

    pub(super) fn publish_target_initialization_output(
        &mut self,
        output: TargetInitializationOutput,
        into: Option<&mut CommandDispatchContext>,
    ) {
        let TargetInitializationOutput {
            completed,
            mut output,
            target,
            host,
            predecessor,
        } = output;
        let start = self.scheduler_state.protocol_work_publication_mark();
        let plan = completed.finish(self, &mut output);
        super::dispatch::append_command_output_plan(&mut Vec::new(), &mut output, plan, None, None);
        match into {
            Some(context) => context.append_projection_output(output),
            None => {
                let events = self.finish_protocol_output_projection(output, Some(target.clone()));
                if !events.is_empty() {
                    self.publish_renderer_fenced_protocol_output_after(
                        events,
                        Vec::new(),
                        Some(target.clone()),
                    );
                }
            }
        }
        if let Some(predecessor) = predecessor {
            self.scheduler_state
                .bind_renderer_delivery_predecessor_since(start, predecessor);
        }
        if let Some(route) = self.target_session_route_for_target_id(&target)
            && host.as_ref().is_some_and(|host| {
                self.page_target_host_identity_for_owner(&CommandOwnerScope::for_route(
                    route.clone(),
                ))
                .is_some_and(|current| host.matches(&current))
            })
            && let Some(context) = route.browser_context_id()
            && let Some(host) = self
                .browser_context_by_id_mut(context)
                .and_then(|bc| bc.page_target_mut(&target))
        {
            host.owner_state.initial_document_work = None;
        }
    }

    pub(crate) fn target_initial_document_waiter(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetInitialDocumentWaiter> {
        let (context, target) = self.target_owner_identity_for_owner(owner)?;
        let host = self
            .browser_context_by_id(&context)?
            .page_target(&target?)?;
        match &host.owner_state.initial_document_work {
            Some(TargetInitialDocumentWork::Running(waiter))
                if waiter.completion.peek().is_none() =>
            {
                Some(waiter.clone())
            }
            _ => None,
        }
    }

    pub(crate) fn stage_target_initial_document(&mut self, owner: &CommandOwnerScope) {
        if let Some((context, Some(target))) = self.target_owner_identity_for_owner(owner)
            && let Some(host) = self
                .browser_context_by_id_mut(&context)
                .and_then(|bc| bc.page_target_mut(&target))
        {
            host.owner_state
                .initial_document_work
                .get_or_insert(TargetInitialDocumentWork::Staged);
        }
    }

    pub(crate) fn start_initial_document_page_ensure_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<PendingInitialDocumentPageBuild>> {
        self.ensure_target_initial_document(owner, None)
    }

    pub(crate) fn start_target_initial_document(
        &mut self,
        owner: &CommandOwnerScope,
        continuation: InitialDocumentContinuation,
    ) -> Result<Option<PendingInitialDocumentPageBuild>> {
        self.ensure_target_initial_document(owner, Some(continuation))
    }

    fn ensure_target_initial_document(
        &mut self,
        owner: &CommandOwnerScope,
        continuation: Option<InitialDocumentContinuation>,
    ) -> Result<Option<PendingInitialDocumentPageBuild>> {
        let Some((browser, Some(target))) = self.target_owner_identity_for_owner(owner) else {
            return Ok(None);
        };
        let Some(host) = self
            .browser_context_by_id(&browser)
            .and_then(|bc| bc.page_target(&target))
        else {
            return Ok(None);
        };
        match &host.owner_state.initial_document_work {
            Some(TargetInitialDocumentWork::Staged) if continuation.is_none() => return Ok(None),
            Some(TargetInitialDocumentWork::Running(waiter))
                if waiter.completion.peek().is_some() && *waiter.publication_released.borrow() =>
            {
                // Settling unclaimed output also retires this receipt. A
                // later Page replacement must not join its old ready result.
                let output = waiter.output.borrow_mut().take();
                if let Some(output) = output {
                    self.publish_target_initialization_output(output, None);
                }
            }
            Some(TargetInitialDocumentWork::Running(waiter))
                if waiter.completion.peek().is_some()
                    && self
                        .runtime_session_owner_slot_for_owner(owner)
                        .is_some_and(|slot| slot.has_loaded_page()) =>
            {
                return Ok(None);
            }
            // Publication belongs to this workflow too. A late waiter joins
            // its retained result until the output has been handed off.
            Some(TargetInitialDocumentWork::Running(waiter)) => {
                return Ok(Some(PendingInitialDocumentPageBuild::for_target(
                    waiter.clone(),
                )));
            }
            _ => {}
        }
        let pending = self.start_initial_document_page_build_for_owner(owner);
        if matches!(pending, Ok(None)) && continuation.is_none() {
            return Ok(None);
        }
        let owner = CommandOwnerScope::for_route(
            self.target_session_route_for_target_id(&target)
                .expect("initial Target route"),
        );
        let identity = self.page_target_host_identity_for_owner(&owner);
        let output_identity = identity.clone();
        let page = self
            .pending_target_page_residence_identity_for_owner(&owner)
            .or_else(|| self.target_page_residence_identity_for_owner(&owner));
        let access = self.protocol_state_access();
        let task_access = access.clone();
        let output_target = target.clone();
        let continuation = continuation.unwrap_or(InitialDocumentContinuation::Navigate);
        let page_only = matches!(&continuation, InitialDocumentContinuation::Deferred(_));
        let publication_predecessor = match &continuation {
            InitialDocumentContinuation::Popup(cursor) => *cursor,
            _ => None,
        };
        let claim_output = matches!(&continuation, InitialDocumentContinuation::Creation(_));
        let publication = Rc::new(RefCell::new(None));
        let ready_publication = publication.clone();
        let (release, publication_released) = tokio::sync::watch::channel(!claim_output);
        let page_ready = Rc::new(RefCell::new(None));
        let task_page_ready = page_ready.clone();
        let future = async move {
            let workflow_access = task_access;
            let result = Rc::new(RefCell::new(None));
            let task_result = result.clone();
            let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
                let mut driver = ProtocolWorkflowDriver::new(workflow_access);
                let popup = match &continuation {
                    InitialDocumentContinuation::Popup(cursor) => Some(*cursor),
                    _ => None,
                };
                let run = async {
                    let completed = match pending? {
                        Some(pending) => Some(match pending.wait().await {
                            Ok(completed) => completed,
                            Err(failed) => {
                                let message = driver.with_state(move |state| {
                                    state.reset_failed_initial_document(failed)
                                })?;
                                anyhow::bail!(message);
                            }
                        }),
                        None => None,
                    };
                    let activation = match &continuation {
                        InitialDocumentContinuation::Creation(commit) => {
                            let current = driver.with_state({
                                let owner = owner.clone();
                                let identity = identity.clone();
                                move |state| {
                                    state.initial_document_target_is_current(
                                        &owner,
                                        identity.as_ref(),
                                    )
                                }
                            })?;
                            match commit.activation().filter(|_| current) {
                                Some(activation) => {
                                    activation.clone().complete_using(&mut driver).await?
                                }
                                None => Vec::new(),
                            }
                        }
                        _ => Vec::new(),
                    };
                    let mut diagnostics = match completed {
                        Some(completed) => completed.install_using(&mut driver).await?,
                        None => LoadedPageCreationDiagnosticsParts::default(),
                    };
                    let current = driver.with_state({
                        let owner = owner.clone();
                        let identity = identity.clone();
                        move |state| {
                            state.initial_document_target_is_current(&owner, identity.as_ref())
                        }
                    })?;
                    if !current && matches!(&continuation, InitialDocumentContinuation::Creation(_))
                    {
                        return Err(InitializationProtocolError(DevToolsError::new(
                            DevToolsErrorKind::NoSuchTarget,
                            "UnknownTargetId",
                        ))
                        .into());
                    }
                    anyhow::ensure!(current, NoDocumentLoaded);
                    if let Some(predecessor) = diagnostics.renderer_output_predecessor.clone() {
                        context
                            .context_mut()
                            .set_renderer_output_predecessor(predecessor);
                    }
                    *task_page_ready.borrow_mut() = Some(Ok(Rc::new(diagnostics.clone())));
                    let navigation = match continuation {
                        InitialDocumentContinuation::Creation(commit) => context
                            .with_state(&mut driver, move |state, context| {
                                state.prepare_target_creation_outputs(commit, activation, context)
                            })?
                            .map_err(InitializationProtocolError)?,
                        continuation => {
                            if let InitialDocumentContinuation::Deferred(release) = continuation {
                                // Cancellation releases the gate too; the
                                // command's wait never owns Target navigation.
                                let _ = release.await;
                            }
                            driver.with_state({
                                let owner = owner.clone();
                                let identity = identity.clone();
                                move |state| {
                                    state.prepare_target_initial_navigation(owner, identity)
                                }
                            })?
                        }
                    };
                    context.materialize_projection_output();
                    if let Some(navigation) = navigation {
                        navigation.run(&mut driver, &mut context).await?;
                    }
                    diagnostics.initialization_output_dependencies =
                        context.context_mut().renderer_output_dependencies.clone();
                    if let Some(boundary) = context.context_mut().renderer_output_boundary.clone() {
                        diagnostics
                            .initialization_output_dependencies
                            .add_same_stream(boundary);
                    }
                    Ok::<_, anyhow::Error>(diagnostics)
                }
                .await;
                if let (Some(predecessor), Some(identity)) = (popup, identity) {
                    let initialized = run.is_ok();
                    let _ = driver.with_state(move |state| {
                        state.finish_popup_initialization(
                            owner,
                            identity,
                            page,
                            initialized,
                            predecessor,
                        )
                    });
                }
                *task_result.borrow_mut() = Some(run.map(Rc::new).map_err(|error| {
                    match error.downcast::<InitializationProtocolError>() {
                        Ok(error) => error.0,
                        Err(error) => DevToolsError::new(DevToolsErrorKind::Internal, error),
                    }
                }));
                context.complete(crate::domains::command_output::CommandOutputPlan::default())
            });
            let mut output = CommandDispatchContext::default();
            let completed = workflow.start(&mut output, None, None).wait().await;
            *ready_publication.borrow_mut() = Some(TargetInitializationOutput {
                completed,
                output,
                target: output_target,
                host: output_identity,
                predecessor: publication_predecessor,
            });
            result
                .borrow_mut()
                .take()
                .expect("initialization result is retained before completion")
        }
        .boxed_local()
        .shared();
        let waiter = TargetInitialDocumentWaiter {
            completion: future,
            installed: page_ready,
            output: publication,
            publication_released,
        };
        self.browser_context_by_id_mut(&browser)
            .and_then(|bc| bc.page_target_mut(&target))
            .expect("initialization Target")
            .owner_state
            .initial_document_work = Some(TargetInitialDocumentWork::Running(waiter.clone()));
        let supervised = waiter.clone();
        let access_for_publication = access.clone();
        let pending = PendingProtocolSchedulerWork::for_target_initialization(
            async move {
                let _ = supervised.clone().wait().await;
                let mut released = supervised.publication_released.clone();
                while !*released.borrow_and_update() {
                    if released.changed().await.is_err() {
                        break;
                    }
                }
                let output = supervised.output.borrow_mut().take();
                if let Some(output) = output {
                    let mut driver = ProtocolWorkflowDriver::new(access_for_publication);
                    let _ = driver.with_state(move |state| {
                        state.finish_target_initialization_publication(output)
                    });
                }
            }
            .boxed_local(),
            access,
        );
        self.push_scheduler_event(CdpSchedulerEvent::ProtocolTaskPrepared { pending });
        Ok(Some(if page_only {
            PendingInitialDocumentPageBuild {
                kind: super::runtime_load::PendingInitialDocumentPageBuildKind::TargetPage(waiter),
            }
        } else if claim_output {
            let claim = TargetInitializationOutputClaim {
                output: waiter.output.clone(),
                release,
            };
            PendingInitialDocumentPageBuild {
                kind: super::runtime_load::PendingInitialDocumentPageBuildKind::TargetCreation(
                    waiter, claim,
                ),
            }
        } else {
            PendingInitialDocumentPageBuild::for_target(waiter)
        }))
    }
}

impl PendingInitialDocumentPageBuild {
    fn for_target(waiter: TargetInitialDocumentWaiter) -> Self {
        Self {
            kind: super::runtime_load::PendingInitialDocumentPageBuildKind::Target(waiter),
        }
    }
}

#[cfg(test)]
mod tests;
