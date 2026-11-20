//! Target-owned initialization. Creators submit one workflow; observers only
//! join its receipts. Page construction and installation stay inside that work.
use std::{
    cell::{Cell, RefCell},
    panic::AssertUnwindSafe,
    rc::Rc,
};

use anyhow::Result;
use futures_util::{
    FutureExt as _,
    future::{LocalBoxFuture, Shared},
};

use super::runtime_load::LoadedPageCreationDiagnosticsParts;
use super::*;
use crate::automation::{DevToolsError, DevToolsErrorKind};

pub(crate) type TargetInitializationResult =
    Result<Rc<TargetInitializationDiagnostics>, DevToolsError>;
type InitializationBody = Box<
    dyn for<'a> FnOnce(
        TargetInitializationTask,
        &'a mut ProtocolWorkflowDriver,
        &'a mut CommandWorkflowContext,
    ) -> LocalBoxFuture<
        'a,
        Result<LoadedPageCreationDiagnosticsParts, DevToolsError>,
    >,
>;

pub(crate) struct TargetInitializationDiagnostics {
    pub(crate) page: LoadedPageCreationDiagnosticsParts,
    pub(crate) output_dependencies: RendererOutputDependencies,
}

impl TargetInitializationDiagnostics {
    fn capture(page: LoadedPageCreationDiagnosticsParts, context: &CommandDispatchContext) -> Self {
        let mut output_dependencies = context.renderer_output_dependencies.clone();
        if let Some(boundary) = context.renderer_output_boundary.clone() {
            output_dependencies.add_same_stream(boundary);
        }
        Self {
            page,
            output_dependencies,
        }
    }

    pub(crate) fn retain_output_dependencies(&self, context: &mut CommandDispatchContext) {
        context.extend_renderer_output_dependencies(self.output_dependencies.clone());
        if let Some(predecessor) = self.page.renderer_output_predecessor.clone() {
            context.set_renderer_output_predecessor(predecessor);
        }
    }
}

struct InitializationProgress {
    installed: Option<Rc<TargetInitializationDiagnostics>>,
    setup: Option<TargetInitializationResult>,
    setup_output: Option<CommandDispatchContext>,
    completion_output: Option<CommandDispatchContext>,
}

struct InitializationReceipt {
    state: RefCell<InitializationProgress>,
    publication_released: tokio::sync::watch::Receiver<bool>,
    progress: tokio::sync::watch::Receiver<Option<TargetInitializationStage>>,
    target: String,
    host: PageTargetHostIdentity,
    predecessor: Option<moli_core::RendererOutputCursor>,
}

#[derive(Clone, Copy)]
pub(crate) enum TargetInitializationStage {
    /// The initial Page is installed; initial URL navigation may still wait.
    PageInstalled,
    /// Automation realm/listener setup and creation output are committed.
    SetupFinished,
    /// This workflow ended. A debugger barrier can leave navigation deferred.
    Complete,
}

#[derive(Clone)]
pub(crate) struct TargetInitialDocumentWaiter {
    completion: Shared<LocalBoxFuture<'static, TargetInitializationResult>>,
    receipt: Rc<InitializationReceipt>,
}

impl std::fmt::Debug for TargetInitialDocumentWaiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TargetInitialDocumentWaiter")
            .field("target", &self.receipt.target)
            .finish_non_exhaustive()
    }
}

impl TargetInitialDocumentWaiter {
    fn stage_result(&self, stage: TargetInitializationStage) -> Option<TargetInitializationResult> {
        let receipt = self.receipt.state.borrow();
        match stage {
            TargetInitializationStage::PageInstalled => receipt.installed.clone().map(Ok),
            TargetInitializationStage::SetupFinished => receipt.setup.clone(),
            TargetInitializationStage::Complete => None,
        }
    }

    pub(crate) async fn wait(self) -> TargetInitializationResult {
        self.wait_for(TargetInitializationStage::Complete).await
    }

    pub(crate) async fn wait_page(self) -> TargetInitializationResult {
        self.wait_for(TargetInitializationStage::PageInstalled)
            .await
    }

    pub(crate) async fn wait_for(
        mut self,
        stage: TargetInitializationStage,
    ) -> TargetInitializationResult {
        let mut progress = self.receipt.progress.clone();
        loop {
            if let Some(result) = self.stage_result(stage) {
                return result;
            }
            tokio::select! {
                result = &mut self.completion => return self.stage_result(stage).unwrap_or(result),
                changed = progress.changed() => {
                    if changed.is_err() {
                        let result = self.completion.clone().await;
                        return self.stage_result(stage).unwrap_or(result);
                    }
                }
            }
        }
    }
}

pub(crate) struct PendingTargetCreation {
    initialization: TargetInitialDocumentWaiter,
    stage: TargetInitializationStage,
    claim: TargetInitializationOutputClaim,
}

pub(crate) struct CompletedTargetCreation {
    result: TargetInitializationResult,
    claim: TargetInitializationOutputClaim,
}

impl PendingTargetCreation {
    pub(crate) async fn wait(self) -> CompletedTargetCreation {
        let result = self.initialization.wait_for(self.stage).await;
        let mut claim = self.claim;
        if matches!(claim.stage, TargetInitializationStage::SetupFinished)
            && claim.initialization.receipt.state.borrow().setup.is_none()
        {
            claim.stage = TargetInitializationStage::Complete;
        }
        CompletedTargetCreation { result, claim }
    }
}

impl CompletedTargetCreation {
    pub(crate) fn publish_into(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> TargetInitializationResult {
        self.claim
            .publish_into(driver, context)
            .map_err(initialization_error)?;
        if let Ok(diagnostics) = &self.result {
            diagnostics.retain_output_dependencies(context.context_mut());
        }
        self.result
    }
}

struct TargetInitializationOutputClaim {
    initialization: TargetInitialDocumentWaiter,
    stage: TargetInitializationStage,
    release: tokio::sync::watch::Sender<bool>,
}

impl Drop for TargetInitializationOutputClaim {
    fn drop(&mut self) {
        self.release.send_replace(true);
    }
}

impl TargetInitializationOutputClaim {
    fn publish_into(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        context.with_state(driver, move |state, context| {
            state.append_target_initialization_output(
                &self.initialization,
                self.stage,
                context.context_mut(),
            );
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TargetInitialDocumentWork {
    Staged(std::rc::Weak<()>),
    Running(TargetInitialDocumentWaiter),
}

/// A creation entry owns this reservation until its attachments and debugger
/// policy are committed. Only then may submission capture renderer inputs.
pub(crate) struct TargetInitializationReservation {
    owner: CommandOwnerScope,
    target: String,
    host: PageTargetHostIdentity,
    token: Rc<()>,
}

pub(crate) struct TargetInitializationTask {
    pub(crate) owner: CommandOwnerScope,
    pub(crate) host: PageTargetHostIdentity,
    pub(crate) page: Option<TargetPageResidenceIdentity>,
    build: Result<Option<PendingInitialDocumentPageBuild>>,
    receipt: Rc<InitializationReceipt>,
    phase: Rc<Cell<&'static str>>,
    progress: tokio::sync::watch::Sender<Option<TargetInitializationStage>>,
}

pub(crate) fn initialization_error(error: impl std::fmt::Display) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, format!("{error:#}"))
}

fn retired_target_error() -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "UnknownTargetId")
}

impl TargetInitializationTask {
    pub(crate) async fn install(
        &mut self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> Result<LoadedPageCreationDiagnosticsParts, DevToolsError> {
        self.install_with_activation(driver, None)
            .await
            .map(|(page, _)| page)
    }

    pub(crate) async fn install_with_activation(
        &mut self,
        driver: &mut ProtocolWorkflowDriver,
        activation: Option<TargetActivationTransition>,
    ) -> Result<
        (
            LoadedPageCreationDiagnosticsParts,
            Vec<BackgroundProtocolEvent>,
        ),
        DevToolsError,
    > {
        // Keep a panic during construction/install in the same error path as
        // an ordinary build failure, including popup Page cleanup.
        AssertUnwindSafe(async {
            self.phase.set("build");
            let completed =
                match std::mem::replace(&mut self.build, Ok(None)).map_err(initialization_error)? {
                    Some(pending) => Some(match pending.wait().await {
                        Ok(completed) => completed,
                        Err(failed) => {
                            return Err(initialization_error(
                                driver
                                    .with_state(move |state| {
                                        state.reset_failed_initial_document(failed)
                                    })
                                    .map_err(initialization_error)?,
                            ));
                        }
                    }),
                    None => None,
                };
            let current = self.is_current(driver)?;
            if !current {
                return Err(retired_target_error());
            }
            self.phase.set("activation");
            let activation = match activation {
                Some(activation) => activation
                    .complete_using(driver)
                    .await
                    .map_err(initialization_error)?,
                None => Vec::new(),
            };
            self.phase.set("install");
            let page = match completed {
                Some(completed) => completed
                    .install_using(driver)
                    .await
                    .map_err(initialization_error)?,
                None => LoadedPageCreationDiagnosticsParts::default(),
            };
            if !self.is_current(driver)? {
                return Err(retired_target_error());
            }
            let diagnostics = TargetInitializationDiagnostics::capture(
                page.clone(),
                &CommandDispatchContext::default(),
            );
            self.receipt.state.borrow_mut().installed = Some(Rc::new(diagnostics));
            self.progress
                .send_replace(Some(TargetInitializationStage::PageInstalled));
            self.phase.set("setup");
            Ok((page, activation))
        })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(initialization_error(OwnedProtocolWaitFailure::Panicked)))
    }

    fn is_current(&self, driver: &mut ProtocolWorkflowDriver) -> Result<bool, DevToolsError> {
        let owner = self.owner.clone();
        let host = self.host.clone();
        driver
            .with_state(move |state| state.initial_document_target_is_current(&owner, Some(&host)))
            .map_err(initialization_error)
    }

    pub(crate) fn setup_finished(
        &self,
        context: &mut CommandWorkflowContext,
        page: &LoadedPageCreationDiagnosticsParts,
    ) {
        let mut deferred = crate::domains::command_output::CommandOutputPlan::default();
        deferred.extend_background_events(context.context_mut().take_deferred_command_events());
        context.retain_output(deferred);
        context.materialize_projection_output();
        let diagnostics =
            TargetInitializationDiagnostics::capture(page.clone(), context.context_mut());
        let replacement =
            CommandDispatchContext::new(context.context_mut().response_flush().clone());
        let output = std::mem::replace(context.context_mut(), replacement);
        let mut receipt = self.receipt.state.borrow_mut();
        receipt.setup_output = Some(output);
        receipt.setup = Some(Ok(Rc::new(diagnostics)));
        self.progress
            .send_replace(Some(TargetInitializationStage::SetupFinished));
    }

    pub(crate) async fn navigate(
        &self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<(), DevToolsError> {
        let owner = self.owner.clone();
        let host = self.host.clone();
        let navigation = driver
            .with_state(move |state| state.prepare_target_initial_navigation(owner, Some(host)))
            .map_err(initialization_error)?;
        self.run_navigation(driver, context, navigation).await
    }

    pub(crate) async fn run_navigation(
        &self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        navigation: Option<crate::domains::target::PreparedPopupNavigation>,
    ) -> Result<(), DevToolsError> {
        self.phase.set("navigation");
        context.materialize_projection_output();
        if let Some(navigation) = navigation {
            navigation
                .run(driver, context)
                .await
                .map_err(initialization_error)?;
        }
        Ok(())
    }
}

impl TargetInitializationReservation {
    fn validate(&self, conn: &mut CdpConnection) -> Result<(), DevToolsError> {
        if !conn
            .page_target_host_identity_for_owner(&self.owner)
            .is_some_and(|host| self.host.matches(&host))
        {
            return Err(retired_target_error());
        }
        if !matches!(conn.initial_document_work_mut(&self.owner), Some(Some(TargetInitialDocumentWork::Staged(token))) if token.ptr_eq(&Rc::downgrade(&self.token)))
        {
            return Err(initialization_error(
                "Target initialization reservation is no longer current",
            ));
        }
        Ok(())
    }

    pub(crate) fn submit(
        self,
        conn: &mut CdpConnection,
        predecessor: Option<moli_core::RendererOutputCursor>,
        body: InitializationBody,
    ) -> Result<TargetInitialDocumentWaiter, DevToolsError> {
        self.validate(conn)?;
        let build = conn.start_initial_document_page_build_for_owner(&self.owner);
        self.start(conn, build, predecessor, false, body)
            .map(|(waiter, _)| waiter)
    }

    pub(crate) fn submit_creation(
        self,
        conn: &mut CdpConnection,
        stage: TargetInitializationStage,
        body: InitializationBody,
    ) -> Result<PendingTargetCreation, DevToolsError> {
        self.validate(conn)?;
        let build = conn.start_initial_document_page_build_for_owner(&self.owner);
        let (initialization, release) = self.start(conn, build, None, true, body)?;
        let claim = TargetInitializationOutputClaim {
            initialization: initialization.clone(),
            stage,
            release,
        };
        Ok(PendingTargetCreation {
            initialization,
            stage,
            claim,
        })
    }

    fn start(
        self,
        conn: &mut CdpConnection,
        build: Result<Option<PendingInitialDocumentPageBuild>>,
        predecessor: Option<moli_core::RendererOutputCursor>,
        claim_output: bool,
        body: InitializationBody,
    ) -> Result<
        (
            TargetInitialDocumentWaiter,
            tokio::sync::watch::Sender<bool>,
        ),
        DevToolsError,
    > {
        self.validate(conn)?;
        let failure_owner = build
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .and_then(|pending| match &pending.kind {
                super::runtime_load::PendingInitialDocumentPageBuildKind::Build {
                    owner, ..
                } => Some(owner.clone()),
                _ => None,
            });
        let (release, publication_released) = tokio::sync::watch::channel(!claim_output);
        let (progress, progress_wait) = tokio::sync::watch::channel(None);
        let receipt = Rc::new(InitializationReceipt {
            state: RefCell::new(InitializationProgress {
                installed: None,
                setup: None,
                setup_output: None,
                completion_output: None,
            }),
            publication_released,
            progress: progress_wait,
            target: self.target.clone(),
            host: self.host.clone(),
            predecessor,
        });
        let phase = Rc::new(Cell::new("build"));
        let page = conn
            .pending_target_page_residence_identity_for_owner(&self.owner)
            .or_else(|| conn.target_page_residence_identity_for_owner(&self.owner));
        let task = TargetInitializationTask {
            owner: self.owner.clone(),
            host: self.host.clone(),
            page,
            build,
            receipt: receipt.clone(),
            phase: phase.clone(),
            progress,
        };
        let access = conn.protocol_state_access();
        let task_access = access.clone();
        let task_receipt = receipt.clone();
        let task_target = self.target.clone();
        let task_host = self.host.clone();
        let task_owner = self.owner.clone();
        let completion = async move {
            let (send, result) = tokio::sync::oneshot::channel();
            let workflow = PreparedCommandWorkflow::new(move |mut context| async move {
                let mut driver = ProtocolWorkflowDriver::new(task_access);
                let result = AssertUnwindSafe(async { body(task, &mut driver, &mut context).await }).catch_unwind().await
                    .unwrap_or_else(|_| Err(initialization_error(OwnedProtocolWaitFailure::Panicked)));
                let _ = send.send(result);
                context.complete(crate::domains::command_output::CommandOutputPlan::default())
            });
            let mut output = CommandDispatchContext::default();
            let completed = workflow.start(&mut output, None, None).wait().await;
            let result = result.await.unwrap_or_else(|_| Err(initialization_error("Target initialization produced no result")));
            let mut driver = ProtocolWorkflowDriver::new(access);
            let (output, failure) = driver.with_state(move |state| state.complete_target_initialization_workflow(completed, output)).map_err(initialization_error)?;
            let result = match failure {
                Some(error) => { phase.set("completion"); Err(error) },
                None => result,
            }.map(|page| {
                let mut diagnostics = TargetInitializationDiagnostics::capture(page, &output);
                if let Some(Ok(setup)) = &task_receipt.state.borrow().setup {
                    diagnostics.output_dependencies.merge(setup.output_dependencies.clone());
                }
                Rc::new(diagnostics)
            });
            if let Err(error) = &result {
                let message = error.message.clone();
                let current = driver.with_state(move |state| state.fail_target_initialization(task_owner, task_host, failure_owner, message)).unwrap_or(false);
                if !current || error.kind == DevToolsErrorKind::NoSuchTarget {
                    tracing::debug!(target_id = %task_target, phase = phase.get(), %error, "Target initialization retired");
                } else {
                    tracing::warn!(target_id = %task_target, phase = phase.get(), %error, "Target initialization failed");
                }
            }
            task_receipt.state.borrow_mut().completion_output = Some(output);
            result
        }.boxed_local().shared();
        let waiter = TargetInitialDocumentWaiter {
            completion,
            receipt,
        };
        conn.initial_document_work_mut(&self.owner)
            .ok_or_else(retired_target_error)?
            .replace(TargetInitialDocumentWork::Running(waiter.clone()));
        let supervised = waiter.clone();
        let access = conn.protocol_state_access();
        let publication_access = access.clone();
        let pending = PendingProtocolSchedulerWork::for_target_initialization(
            async move {
                let mut released = supervised.receipt.publication_released.clone();
                let mut progress = supervised.receipt.progress.clone();
                let mut progress_open = true;
                let mut release_open = true;
                let mut driver = ProtocolWorkflowDriver::new(publication_access);
                loop {
                    if *released.borrow() {
                        let initialization = supervised.clone();
                        let _ = driver.with_state(move |state| state.publish_target_initialization_setup(&initialization));
                    }
                    tokio::select! {
                        _ = supervised.clone().wait() => break,
                        changed = progress.changed(), if progress_open => { progress_open = changed.is_ok(); }
                        changed = released.changed(), if release_open => { release_open = changed.is_ok(); }
                    }
                }
                while !*released.borrow_and_update() {
                    if released.changed().await.is_err() {
                        break;
                    }
                }
                let _ = driver.with_state(move |state| {
                    state.finish_target_initialization_publication(supervised)
                });
            }
            .boxed_local(),
            access,
        );
        conn.push_scheduler_event(CdpSchedulerEvent::ProtocolTaskPrepared { pending });
        Ok((waiter, release))
    }
}

impl CdpConnection {
    pub(crate) fn target_can_start_initial_navigation(&self, owner: &CommandOwnerScope) -> bool {
        !self.owner_target_has_waiting_for_debugger_session(owner)
            && self.runtime_session_owner_needs_initial_document_navigation_for_owner(owner)
    }

    fn initial_document_work_mut(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<&mut Option<TargetInitialDocumentWork>> {
        let (context, target) = self.target_owner_identity_for_owner(owner)?;
        Some(
            &mut self
                .browser_context_by_id_mut(&context)?
                .page_target_mut(&target?)?
                .owner_state
                .initial_document_work,
        )
    }

    pub(crate) fn reserve_target_initialization(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<TargetInitializationReservation, DevToolsError> {
        let (_, target) = self
            .target_owner_identity_for_owner(owner)
            .ok_or_else(retired_target_error)?;
        let target = target.ok_or_else(retired_target_error)?;
        let owner = CommandOwnerScope::for_route(
            self.target_session_route_for_target_id(&target)
                .ok_or_else(retired_target_error)?,
        );
        let host = self
            .page_target_host_identity_for_owner(&owner)
            .ok_or_else(retired_target_error)?;
        let work = self
            .initial_document_work_mut(&owner)
            .ok_or_else(retired_target_error)?;
        if matches!(work, Some(TargetInitialDocumentWork::Running(_)))
            || matches!(work, Some(TargetInitialDocumentWork::Staged(token)) if token.upgrade().is_some())
        {
            return Err(initialization_error(
                "Target initialization is already reserved",
            ));
        }
        let token = Rc::new(());
        *work = Some(TargetInitialDocumentWork::Staged(Rc::downgrade(&token)));
        Ok(TargetInitializationReservation {
            owner,
            target,
            host,
            token,
        })
    }

    pub(crate) fn target_initial_document_waiter(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetInitialDocumentWaiter> {
        let (context, target) = self.target_owner_identity_for_owner(owner)?;
        match &self
            .browser_context_by_id(&context)?
            .page_target(&target?)?
            .owner_state
            .initial_document_work
        {
            Some(TargetInitialDocumentWork::Running(waiter))
                if waiter.completion.peek().is_none() =>
            {
                Some(waiter.clone())
            }
            _ => None,
        }
    }

    pub(crate) fn ensure_target_initialization(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<TargetInitialDocumentWaiter>, DevToolsError> {
        let work = self
            .initial_document_work_mut(owner)
            .map(|work| work.clone())
            .ok_or_else(retired_target_error)?;
        match work {
            Some(TargetInitialDocumentWork::Staged(token)) if token.upgrade().is_some() => {
                return Ok(None);
            }
            Some(TargetInitialDocumentWork::Staged(_)) => {
                self.initial_document_work_mut(owner).unwrap().take();
            }
            Some(TargetInitialDocumentWork::Running(waiter))
                if waiter.completion.peek().is_some()
                    && *waiter.receipt.publication_released.borrow() =>
            {
                self.finish_target_initialization(waiter)
            }
            Some(TargetInitialDocumentWork::Running(waiter)) => return Ok(Some(waiter)),
            None => {}
        }
        let build = self
            .start_initial_document_page_build_for_owner(owner)
            .map_err(initialization_error)?;
        let Some(build) = build else {
            return Ok(None);
        };
        let reservation = self.reserve_target_initialization(owner)?;
        reservation
            .start(
                self,
                Ok(Some(build)),
                None,
                false,
                Box::new(|mut task, driver, context| {
                    Box::pin(async move {
                        let page = task.install(driver).await?;
                        if let Some(predecessor) = page.renderer_output_predecessor.clone() {
                            context
                                .context_mut()
                                .set_renderer_output_predecessor(predecessor);
                        }
                        task.navigate(driver, context).await?;
                        Ok(page)
                    })
                }),
            )
            .map(|(waiter, _)| Some(waiter))
    }

    pub(super) fn append_initialization_output(
        &mut self,
        waiter: &TargetInitialDocumentWaiter,
        stage: TargetInitializationStage,
        into: Option<&mut CommandDispatchContext>,
    ) {
        let output = {
            let mut receipt = waiter.receipt.state.borrow_mut();
            match stage {
                TargetInitializationStage::SetupFinished => receipt.setup_output.take(),
                TargetInitializationStage::Complete => receipt.completion_output.take(),
                TargetInitializationStage::PageInstalled => None,
            }
        };
        let Some(output) = output else {
            return;
        };
        let start = self.scheduler_state.protocol_work_publication_mark();
        match into {
            Some(context) => context.append_projection_output(output),
            None => {
                let events = self
                    .finish_protocol_output_projection(output, Some(waiter.receipt.target.clone()));
                if !events.is_empty() {
                    self.publish_renderer_fenced_protocol_output_after(
                        events,
                        Vec::new(),
                        Some(waiter.receipt.target.clone()),
                    );
                }
            }
        }
        if let Some(predecessor) = waiter.receipt.predecessor {
            self.scheduler_state
                .bind_renderer_delivery_predecessor_since(start, predecessor);
        }
    }

    pub(super) fn finish_target_initialization(&mut self, waiter: TargetInitialDocumentWaiter) {
        self.append_initialization_output(&waiter, TargetInitializationStage::SetupFinished, None);
        self.append_initialization_output(&waiter, TargetInitializationStage::Complete, None);
        if let Some(route) = self.target_session_route_for_target_id(&waiter.receipt.target) {
            let owner = CommandOwnerScope::for_route(route);
            if self
                .page_target_host_identity_for_owner(&owner)
                .is_some_and(|host| waiter.receipt.host.matches(&host))
                && let Some(work) = self.initial_document_work_mut(&owner)
                && matches!(work, Some(TargetInitialDocumentWork::Running(current)) if Rc::ptr_eq(&current.receipt, &waiter.receipt))
            {
                *work = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
