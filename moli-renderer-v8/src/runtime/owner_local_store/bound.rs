use super::*;

pub(in crate::runtime) struct RenderRuntimeOwnerLocalStoreBinding;

fn with_bound_render_runtime_owner_local_store_session<R>(
    f: impl FnOnce(RendererOwnerLocalStoreSession<'_>) -> R,
) -> R {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
        let mut store = current_store
            .borrow()
            .expect("bound render-runtime owner-local store should exist on current thread");
        // Safety: the pointer is installed only for the lifetime of the
        // render-runtime owner loop on the current thread.
        unsafe {
            f(RendererOwnerLocalStoreSession {
                store: store.as_mut(),
            })
        }
    })
}

pub(in crate::runtime) fn bind_render_runtime_owner_local_store(
    store: &mut RendererOwnerLocalStore,
) -> RenderRuntimeOwnerLocalStoreBinding {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
        let mut current_store = current_store.borrow_mut();
        assert!(
            current_store.is_none(),
            "render-runtime owner-local store must not be rebound while already active"
        );
        *current_store = Some(NonNull::from(store));
    });
    RenderRuntimeOwnerLocalStoreBinding
}

pub(in crate::runtime) fn has_current_render_runtime_owner_local_store() -> bool {
    CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| current_store.borrow().is_some())
}

pub(in crate::runtime) fn owner_local_store_session(
    store: &mut RendererOwnerLocalStore,
) -> RendererOwnerLocalStoreSession<'_> {
    RendererOwnerLocalStoreSession { store }
}

pub(in crate::runtime) fn install_page_vm_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    requested_url: Url,
    navigation_initiator_url: Option<Url>,
    navigation_redirected: bool,
    navigation_redirect_count: usize,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    vm: PageVm,
    pending_download: Option<RendererPendingDownloadActivation>,
    lifecycle_gate: Option<PageVmInitStage>,
) -> Result<RendererPendingPageCreation> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.install_page_vm(
            owner_local,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            vm,
            pending_download,
            lifecycle_gate,
        )
    })
}

pub(in crate::runtime) fn install_phase_one_blocked_page_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    requested_url: Url,
    navigation_initiator_url: Option<Url>,
    navigation_redirected: bool,
    navigation_redirect_count: usize,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    pending_navigation: PageVmPendingPhaseOneNavigation,
    lifecycle_gate: Option<PageVmInitStage>,
) -> Result<RendererPendingPageCreation> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.install_phase_one_blocked_page_for_owner(
            owner_local,
            requested_url,
            navigation_initiator_url,
            navigation_redirected,
            navigation_redirect_count,
            response_status,
            response_headers,
            pending_navigation,
            lifecycle_gate,
        )
    })
}

pub(in crate::runtime) fn finalize_pending_page_creation_on_bound_owner_local_store(
    pending: RendererPendingPageCreation,
) -> RendererPageCreationCommit {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.finalize_pending_page_creation(pending)
    })
}

pub(in crate::runtime) fn resolve_pending_page_creation_on_bound_owner_local_store(
    pending: RendererPendingPageCreation,
    document: RendererDocumentLifecycleIdentity,
    target_stage: PageVmInitStage,
    navigation_reply_policy: NavigationReplyPolicy,
) -> RendererPageCreationResolution {
    // This operation runs inside one owner-lane task and contains no await
    // boundary. Before the task starts the entry remains resident; once it is
    // checked out, observation and the resulting page-state refresh,
    // restoration, or retirement run to completion without exposing the entry
    // to the outer owner loop.
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.resolve_pending_page_creation(
            pending,
            document,
            target_stage,
            navigation_reply_policy,
        )
    })
}

pub(super) fn reserve_renderer_document_isolate_on_bound_owner_local_store(
    owner_local: &RendererOwnerLocalContext,
    page_id: PageId,
    page_runtime_task_source: crate::page_task_queue::PageRuntimeTaskSource,
) -> Result<(
    RendererDocumentIsolateBootstrap,
    RendererDocumentIsolateReservation,
)> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.reserve_renderer_document_isolate(owner_local, page_id, page_runtime_task_source)
    })
}

pub(super) fn remove_reserved_renderer_document_isolate_on_bound_owner_local_store(
    token: RendererPageToken,
    reservation_id: u64,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.remove_reserved_renderer_document_isolate(token, reservation_id)
    })
}

async fn run_on_bound_owner_local_store_local_task<R, F>(
    local_executor: JsLocalExecutor,
    future: F,
) -> Result<R>
where
    R: 'static,
    F: Future<Output = Result<R>> + 'static,
{
    run_named_owner_local_task(
        local_executor,
        "bound render-runtime owner-local local task was cancelled",
        future,
    )
    .await
}

pub(in crate::runtime) type EntryLocalTaskFuture<'a, R> =
    Pin<Box<dyn Future<Output = Result<R>> + 'a>>;

#[cfg(debug_assertions)]
const PANIC_WAIT_FOR_SELECTOR_FOR_TESTING: &str = "__moli_panic_wait_for_selector_for_testing__";
#[cfg(debug_assertions)]
const PANIC_WAIT_FOR_SCRIPT_TRUTHY_FOR_TESTING: &str =
    "__moli_panic_wait_for_script_truthy_for_testing__";

pub(super) struct EntryLocalTaskGuard<E, R> {
    entry: Option<E>,
    reply_tx: Option<oneshot::Sender<(E, Result<R>)>>,
}

impl<E, R> EntryLocalTaskGuard<E, R> {
    pub(super) fn new(entry: E, reply_tx: oneshot::Sender<(E, Result<R>)>) -> Self {
        Self {
            entry: Some(entry),
            reply_tx: Some(reply_tx),
        }
    }

    fn entry_mut(&mut self) -> &mut E {
        self.entry
            .as_mut()
            .expect("entry local task guard should retain its page entry")
    }

    fn complete(mut self, result: Result<R>) {
        let entry = self
            .entry
            .take()
            .expect("entry local task guard should retain its page entry on completion");
        let reply_tx = self
            .reply_tx
            .take()
            .expect("entry local task guard should retain its reply sender on completion");
        let _ = reply_tx.send((entry, result));
    }
}

impl<E, R> Drop for EntryLocalTaskGuard<E, R> {
    fn drop(&mut self) {
        if let (Some(entry), Some(reply_tx)) = (self.entry.take(), self.reply_tx.take()) {
            let _ = reply_tx.send((
                entry,
                Err(anyhow!(
                    "bound render-runtime owner-local local task panicked before restoring its page entry"
                )),
            ));
        }
    }
}

pub(in crate::runtime) async fn run_entry_on_bound_owner_local_store_local_task<R, F>(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    operation: F,
) -> (LivePageEntry, Result<R>)
where
    R: 'static,
    F: for<'a> FnOnce(&'a mut LivePageEntry) -> EntryLocalTaskFuture<'a, R> + 'static,
{
    let (reply_tx, reply_rx) = oneshot::channel();
    // Construct the guard before spawning. If the local task is cancelled
    // before its first poll, dropping the future must still return the entry
    // and an inner error to the owner so command-specific cleanup can run.
    let guard = EntryLocalTaskGuard::new(entry, reply_tx);
    tokio::task::spawn_local(async move {
        let mut guard = guard;
        let result = local_executor
            .scope_on_current_thread(operation(guard.entry_mut()))
            .await;
        guard.complete(result);
    });
    reply_rx
        .await
        .expect("entry local task guard must return the page entry on task termination")
}

pub(in crate::runtime) fn take_entry_for_command_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Result<LivePageEntry> {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.take_entry_for_command(token)
    })
}

pub(in crate::runtime) fn checkout_entry_for_owner_turn_on_bound_owner_local_store(
    token: RendererPageToken,
) -> LivePageEntryCheckout {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.checkout_entry_for_owner_turn(token)
    })
}

pub(in crate::runtime) fn schedule_page_turn_on_bound_owner_local_store(
    token: RendererPageToken,
    trigger: PageTurnTrigger,
) -> RendererPageTurnAdmission {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.schedule_page_turn(token, trigger)
    })
}

pub(in crate::runtime) fn release_post_response_document_lifecycle_on_bound_owner_local_store(
    token: RendererPageToken,
    document: RendererDocumentLifecycleIdentity,
) -> bool {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .release_post_response_document_lifecycle(token, document)
    })
}

pub(in crate::runtime) fn checkout_scheduled_page_turn_on_bound_owner_local_store(
    token: RendererPageToken,
) -> RendererPageTurnCheckout {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.checkout_scheduled_page_turn(token)
    })
}

pub(in crate::runtime) fn has_ready_page_networking_task_on_bound_owner_local_store(
    token: RendererPageToken,
    current_document: RendererDocumentToken,
) -> bool {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .has_ready_page_networking_task(token, current_document)
    })
}

/// Reconcile one restored phase-one residence against its stable
/// producer sources.
///
/// The residence must already be visible in the Page slot before this function
/// is called. A producer may have published its payload and spent its
/// empty-to-nonempty wake before restoration completed; current source
/// readiness is therefore authoritative, while the stored suspension reason
/// only selects the exact source to inspect. No task is dequeued here.
pub(in crate::runtime) fn pending_phase_one_admission_after_restore_on_bound_owner_local_store(
    token: RendererPageToken,
) -> PhaseOneResidenceAdmission {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .pending_phase_one_admission_after_restore(token)
    })
}

pub(in crate::runtime) fn page_turn_readiness_after_restore_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Option<PageOwnerTurnReadiness> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.page_turn_readiness_after_restore(token)
    })
}

pub(in crate::runtime) fn renderer_output_fence_for_tail_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Option<RendererOutputFence> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .page_hosts
            .get(&token.local_host_id)
            .and_then(|host| host.pages.get(&token.page_id))
            .and_then(|slot| {
                let journal = slot.script_environment_pin.environment.output_journal();
                journal
                    .last_published_cursor()
                    .map(|cursor| journal.declare_fence(cursor))
            })
    })
}

pub(in crate::runtime) fn restore_entry_after_command_on_bound_owner_local_store(
    token: RendererPageToken,
    entry: LivePageEntry,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.restore_entry_after_command(token, entry);
    });
}

pub(in crate::runtime) fn restore_retiring_entry_after_command_on_bound_owner_local_store(
    token: RendererPageToken,
    entry: RetiringPageEntry,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.restore_retiring_entry_after_command(token, entry);
    });
}

pub(in crate::runtime) fn restore_entry_after_document_lifecycle_on_bound_owner_local_store(
    token: RendererPageToken,
    entry: LivePageEntry,
    reconsider_displaced_ordinary: bool,
) {
    with_bound_render_runtime_owner_local_store_session(|mut session| {
        session.restore_entry_after_command(token, entry);
        if let Some(gate) = session
            .store
            .page_hosts
            .get_mut(&token.local_host_id)
            .and_then(|host| host.pages.get_mut(&token.page_id))
            .and_then(|slot| slot.lifecycle_gate.as_mut())
        {
            gate.settle_lifecycle_turn(reconsider_displaced_ordinary);
        }
    });
}

pub(in crate::runtime) fn release_lifecycle_gate_on_bound_owner_local_store(
    token: RendererPageToken,
) -> Result<ReleasedLifecycleGate> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.release_lifecycle_gate(token)
    })
}

pub(in crate::runtime) fn renderer_page_token_for_owner_context(
    owner: &RendererOwnerLocalContext,
    page_id: PageId,
) -> RendererPageToken {
    RendererPageToken {
        local_host_id: owner.local_host_id,
        #[cfg(debug_assertions)]
        local_thread_id: owner.local_thread_id,
        page_id,
    }
}

pub(in crate::runtime) async fn dispatch_async_command_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    command: RendererPageCommand,
) -> (LivePageEntry, Result<RendererPageCommandDispatch>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(
            async move { RendererOwnerLocalStore::dispatch_async_on_entry(entry, command).await },
        )
    })
    .await
}

/// Command result plus any replacement lifecycle admission created while the
/// Page entry was checked out. The owner must restore stable residence before
/// publishing the admitted lifecycle turn.
pub(in crate::runtime) struct RendererPageCommandDispatch {
    pub(in crate::runtime) reply: RendererPageReply,
    pub(in crate::runtime) replacement_lifecycle: Option<DocumentLifecycleTurnOutcome>,
    pub(in crate::runtime) turn_records: Vec<PendingRendererOutputRecord>,
}

pub(in crate::runtime) async fn advance_runtime_command_lifecycle_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    scope_id: PageVmRuntimeCommandOutputScopeId,
) -> (LivePageEntry, Result<PageVmRuntimeCommandLifecycleAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_pending_runtime_command_lifecycle_one_turn(scope_id)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn begin_post_parse_lifecycle_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    work: Vec<PostParsePageOwnedWork>,
    stage: PageVmInitStage,
    started: std::time::Instant,
) -> (LivePageEntry, Result<DocumentLifecycleTurnOutcome>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let (page_vm, pending_document_lifecycle_turn) =
                entry.page_vm_and_document_lifecycle_turn_mut();
            page_vm
                .begin_post_parse_lifecycle_on_named_owner_lane(
                    pending_document_lifecycle_turn,
                    work,
                    stage,
                    started,
                )
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn commit_page_state_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
) -> (LivePageEntry, Result<Arc<RendererPageState>>) {
    commit_page_state_on_entry_via_local_task_with_policy(
        local_executor,
        entry,
        super::RendererPageStateCapturePolicy::FullReport,
    )
    .await
}

pub(in crate::runtime) async fn commit_page_state_on_entry_via_local_task_with_policy(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    capture_policy: super::RendererPageStateCapturePolicy,
) -> (LivePageEntry, Result<Arc<RendererPageState>>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            RendererOwnerLocalStore::commit_current_vm_page_state_on_entry_with_policy(
                entry,
                capture_policy,
            )
            .map_err(|error| anyhow!("failed to refresh renderer owner page view: {error}"))
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_network_idle_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    state: PageVmNetworkIdleWaitState,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmNetworkIdleWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_network_idle_wait_turn(state, remaining)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_dom_stable_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    state: PageVmDomStableWaitState,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmDomStableWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_dom_stable_wait_turn(state, remaining)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_selector_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    selector: String,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmCommandWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            #[cfg(debug_assertions)]
            if selector == PANIC_WAIT_FOR_SELECTOR_FOR_TESTING {
                panic!("wait-for-selector local task panicked for testing")
            }
            entry
                .page_vm_mut()
                .advance_selector_wait_turn(&selector, remaining)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_script_truthy_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    expression: String,
    pending_call: Option<PendingRuntimeEvaluateCall>,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmScriptTruthyWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            #[cfg(debug_assertions)]
            if expression == PANIC_WAIT_FOR_SCRIPT_TRUTHY_FOR_TESTING {
                panic!("wait-for-script-truthy local task panicked for testing")
            }
            entry
                .page_vm_mut()
                .advance_script_truthy_wait_turn(&expression, pending_call, remaining)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_runtime_expression_await_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    execution_context_id: Option<i64>,
    expression: String,
    pending_call: Option<PendingRuntimeEvaluateCall>,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmRuntimeExpressionAwaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_runtime_expression_await_turn(
                    execution_context_id,
                    &expression,
                    pending_call,
                    remaining,
                )
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn advance_subresource_response_wait_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    criteria: SubresourceResponseWaitCriteria,
    remaining: std::time::Duration,
) -> (LivePageEntry, Result<PageVmSubresourceResponseWaitAdvance>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            entry
                .page_vm_mut()
                .advance_subresource_response_wait_turn(&criteria, remaining)
                .await
        })
    })
    .await
}

pub(in crate::runtime) async fn follow_pending_location_navigation_one_turn_on_entry_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    stage: PageVmInitStage,
) -> (LivePageEntry, Result<LivePageNavigationFollowTurn>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let outcome = {
                let (page_vm, pending_document_lifecycle_turn) =
                    entry.page_vm_and_document_lifecycle_turn_mut();
                page_vm
                    .prepare_pending_location_navigation_document_commit_one_turn_async(
                        pending_document_lifecycle_turn,
                        stage,
                    )
                    .await
            };
            let outcome = match outcome? {
                PageVmFollowNavigationTurnOutcome::Completed => {
                    LivePageNavigationFollowOutcome::Completed
                }
                PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                    target_stage,
                    outcome,
                } => LivePageNavigationFollowOutcome::PostParseLifecycle {
                    target_stage,
                    outcome,
                },
                PageVmFollowNavigationTurnOutcome::Download(download) => {
                    LivePageNavigationFollowOutcome::Download(download)
                }
                PageVmFollowNavigationTurnOutcome::PendingPhaseOne(pending) => {
                    let wake_token = entry.install_new_pending_phase_one_navigation(pending)?;
                    LivePageNavigationFollowOutcome::PendingPhaseOne { wake_token }
                }
                PageVmFollowNavigationTurnOutcome::TriggeredNavigation { stage } => {
                    LivePageNavigationFollowOutcome::TriggeredNavigation { stage }
                }
            };
            let document_commit = entry
                .has_uncommitted_page_vm()
                .then(|| entry.publish_replacement_document_commit())
                .transpose()?;
            Ok(LivePageNavigationFollowTurn {
                outcome,
                document_commit,
            })
        })
    })
    .await
}

pub(in crate::runtime) fn remove_page_on_bound_owner_local_store(token: RendererPageToken) {
    with_bound_render_runtime_owner_local_store_session(|mut session| session.remove_page(token))
}

pub(in crate::runtime) fn publish_page_navigation_failure_on_bound_owner_local_store(
    token: RendererPageToken,
    failure: PageNavigationOwnerFailure,
) -> Result<PageCreationNavigationFailurePublication> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .publish_page_navigation_failure(token, failure)
    })
}

pub(in crate::runtime) async fn remove_page_on_bound_owner_local_store_via_local_task(
    local_executor: JsLocalExecutor,
    token: RendererPageToken,
) -> Result<()> {
    run_on_bound_owner_local_store_local_task(local_executor, async move {
        remove_page_on_bound_owner_local_store(token);
        Ok(())
    })
    .await
}

pub(in crate::runtime) fn next_page_task_deadline_on_bound_owner_local_store()
-> Option<std::time::Instant> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.next_page_task_deadline()
    })
}

/// Snapshot Pages with any due owner-scheduled task from the derived deadline
/// index. Only resident entries are indexed because a checked-out PageVm may
/// change its timer heap or delayed typed-source state before restoration.
pub(in crate::runtime) fn snapshot_due_page_task_tokens_on_bound_owner_local_store(
    due_at_or_before: std::time::Instant,
) -> Vec<RendererPageToken> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session
            .store
            .snapshot_due_page_task_tokens(due_at_or_before)
    })
}

pub(in crate::runtime) fn next_owner_maintenance_deadline_on_bound_owner_local_store()
-> Option<std::time::Instant> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.next_owner_maintenance_deadline()
    })
}

pub(in crate::runtime) fn claim_due_owner_maintenance_task_on_bound_owner_local_store(
    now: std::time::Instant,
) -> Option<RendererOwnerMaintenanceTask> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.claim_due_owner_maintenance_task(now)
    })
}

pub(in crate::runtime) fn settle_owner_maintenance_task_on_bound_owner_local_store(
    task: RendererOwnerMaintenanceTask,
    now: std::time::Instant,
) -> Result<()> {
    with_bound_render_runtime_owner_local_store_session(|session| {
        session.store.settle_owner_maintenance_task(task, now)
    })
}

pub(super) struct PageReadyDescriptorSnapshot {
    pub(super) eligible: Vec<crate::page_task_queue::RendererPageReadyDescriptor>,
    pub(super) stable_source_was_ready: bool,
    due_deadline_was_ready: bool,
}

impl PageReadyDescriptorSnapshot {
    pub(super) const fn has_ready_ordinary_source(&self) -> bool {
        self.stable_source_was_ready || self.due_deadline_was_ready
    }
}

pub(super) fn page_ready_descriptor_snapshot(
    entry: &mut LivePageEntry,
    task_sources: &mut RendererPageOwnedTaskSources,
) -> PageReadyDescriptorSnapshot {
    let mut descriptors = task_sources.ready_descriptors();
    let stable_source_was_ready = !descriptors.is_empty();
    let mut due_deadline_was_ready = false;
    if let Some(timer) = entry.page_vm().due_page_timer_ready_descriptor() {
        descriptors.push(timer);
        due_deadline_was_ready = true;
    }
    if let Some(action_window) = entry
        .page_vm()
        .due_page_action_window_ready_descriptor(std::time::Instant::now())
    {
        descriptors.push(action_window);
        due_deadline_was_ready = true;
    }
    descriptors.retain(|descriptor| {
        entry
            .page_vm_mut()
            .page_ready_descriptor_is_eligible(*descriptor)
    });
    PageReadyDescriptorSnapshot {
        eligible: descriptors,
        stable_source_was_ready,
        due_deadline_was_ready,
    }
}

pub(super) fn select_page_scheduler_turn(
    scheduler: &mut PageTurnScheduler<LivePageEntry>,
    entry: &mut LivePageEntry,
    task_sources: &mut RendererPageOwnedTaskSources,
    lifecycle_gate: &mut Option<LifecycleGate>,
    trigger: PageTurnTrigger,
) -> RendererPageScheduledTurn {
    let snapshot = page_ready_descriptor_snapshot(entry, task_sources);
    // A lifecycle action can change an already-queued source's eligibility.
    // Preserve queue readiness, not only the pre-action eligible set, so a
    // blocked/idle lifecycle result can request exactly one fresh arbitration
    // without re-reading mutable source state after execution.
    let lifecycle_is_deferred = entry.document_lifecycle_is_deferred_until_response();
    let has_pending_document_lifecycle_turn =
        !lifecycle_is_deferred && entry.pending_document_lifecycle_identity().is_some();
    let document_lifecycle_owner_turn_is_runnable =
        !lifecycle_is_deferred && entry.document_lifecycle_owner_turn_is_runnable();
    let has_ready_main_parser_script_continuation =
        !lifecycle_is_deferred && entry.has_ready_main_parser_script_continuation();
    let document_lifecycle = DocumentLifecycleClassReadiness::from_resident_state(
        has_pending_document_lifecycle_turn,
        document_lifecycle_owner_turn_is_runnable,
        has_ready_main_parser_script_continuation,
    );
    let gate_policy = lifecycle_gate
        .as_mut()
        .map(|gate| gate.turn_policy(entry, !snapshot.eligible.is_empty()))
        .unwrap_or(LifecycleGateTurnPolicy::Normal);
    let selected_class = match gate_policy {
        LifecycleGateTurnPolicy::Normal => {
            scheduler.select_turn_class(trigger, !snapshot.eligible.is_empty(), document_lifecycle)
        }
        LifecycleGateTurnPolicy::Drive {
            reconsider_displaced_ordinary,
        } => scheduler.select_lifecycle_turn(
            reconsider_displaced_ordinary,
            !snapshot.eligible.is_empty(),
            document_lifecycle,
        ),
        LifecycleGateTurnPolicy::Park => {
            return RendererPageScheduledTurn::SpentWake;
        }
    };
    match selected_class {
        Some(PageTurnClass::DocumentLifecycle) => RendererPageScheduledTurn::DocumentLifecycle {
            displaced_ordinary: RendererDisplacedOrdinaryTurn::from_ready_source(
                snapshot.has_ready_ordinary_source(),
            ),
        },
        Some(PageTurnClass::Ordinary) => {
            let selected = scheduler
                .select_ready_descriptor(snapshot.eligible)
                .expect("selected ordinary Page-turn class must retain an eligible descriptor");
            let task = task_sources.take_task(selected);
            RendererPageScheduledTurn::Ordinary(Box::new(task))
        }
        None => RendererPageScheduledTurn::SpentWake,
    }
}

/// Run one ordinary page-owner turn already selected by the Page scheduler.
/// Every ordinary task is a concrete typed source payload or a due timer. The
/// caller must restore the returned entry before scheduling any continuation.
pub(in crate::runtime) async fn advance_page_owner_one_turn_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
    task: RendererPageSchedulerTask,
    loader: ResourceRequestClient,
) -> (LivePageEntry, Result<()>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let replacement_lifecycle_snapshot = entry
                .page_vm()
                .document_replacement_lifecycle_action_snapshot();
            // This scope belongs to the common Page owner, not the selected
            // task executor: command-local wait drivers reuse that executor
            // but retain their own typed navigation continuation.
            let _navigation_handoff_scope = entry
                .page_vm()
                .vm()
                .begin_ordinary_page_turn_navigation_handoff()?;
            let application = entry
                .page_vm_mut()
                .apply_selected_page_scheduler_task(task, &loader)
                .await;
            // Any JavaScript-capable Page action can synchronously replace the
            // main Document through document.open()/document.close(). Reconcile
            // the exact transition caused by this action at the common owner
            // boundary. Reconciliation must still run when the action reports
            // an error: JavaScript side effects are not rolled back by an
            // exception. A selected Page action cannot repair an older missed
            // transition.
            let reconciliation = {
                let (page_vm, pending_document_lifecycle_turn) =
                    entry.page_vm_and_document_lifecycle_turn_mut();
                page_vm
                    .reconcile_document_replacement_lifecycle_after_owner_action(
                        replacement_lifecycle_snapshot,
                        pending_document_lifecycle_turn,
                    )
                    .await
            };

            match (application, reconciliation) {
                (Ok(()), Ok(_)) => Ok(()),
                (Err(action_error), Ok(_)) => Err(action_error),
                (Ok(_), Err(reconciliation_error)) => Err(reconciliation_error),
                (Err(action_error), Err(reconciliation_error)) => Err(anyhow!(
                    "page action failed ({action_error:#}) and its Document replacement lifecycle reconciliation also failed ({reconciliation_error:#})"
                )),
            }
        })
    })
    .await
}

/// Execute at most one action from the exact-Document lifecycle resident.
/// A missing resident is an idle stale-wake outcome; this helper never binds
/// a page wake to whichever Document happens to be current.
pub(in crate::runtime) async fn advance_document_lifecycle_one_page_turn_via_local_task(
    local_executor: JsLocalExecutor,
    entry: LivePageEntry,
) -> (LivePageEntry, Result<DocumentLifecycleTurnOutcome>) {
    run_entry_on_bound_owner_local_store_local_task(local_executor, entry, move |entry| {
        Box::pin(async move {
            let Some(document) = entry.pending_document_lifecycle_identity() else {
                return Ok(DocumentLifecycleTurnOutcome::idle(
                    DocumentLifecycleTurnAction::None,
                ));
            };
            let (page_vm, pending_document_lifecycle_turn) =
                entry.page_vm_and_document_lifecycle_turn_mut();
            let outcome = page_vm
                .advance_post_parse_lifecycle_one_owner_turn(
                    pending_document_lifecycle_turn,
                    document,
                )
                .await?;
            if let Some(pending) = pending_document_lifecycle_turn.as_mut() {
                pending.owner_turn_is_runnable = matches!(
                    outcome.readiness,
                    DocumentLifecycleTurnReadiness::Runnable { .. }
                );
            }
            Ok(outcome)
        })
    })
    .await
}

pub(in crate::runtime) fn observe_document_lifecycle_on_entry(
    entry: &mut LivePageEntry,
    document: RendererDocumentLifecycleIdentity,
    target_stage: PageVmInitStage,
) -> DocumentLifecycleObserverOutcome {
    entry.observe_document_lifecycle(document, target_stage)
}

pub(super) fn reconcile_page_creation_lifecycle_observation(
    observation: DocumentLifecycleObserverOutcome,
    has_pending_location_navigation: bool,
) -> DocumentLifecycleObserverOutcome {
    match observation {
        DocumentLifecycleObserverOutcome::Reached if has_pending_location_navigation => {
            DocumentLifecycleObserverOutcome::NavigationPending
        }
        observation => observation,
    }
}

pub(in crate::runtime) fn has_pending_document_lifecycle_turn_on_entry(
    entry: &mut LivePageEntry,
) -> bool {
    entry.pending_document_lifecycle_identity().is_some()
}

impl Drop for RenderRuntimeOwnerLocalStoreBinding {
    fn drop(&mut self) {
        CURRENT_RENDER_RUNTIME_OWNER_LOCAL_STORE.with(|current_store| {
            // Cleanup must remain idempotent and non-asserting: this guard can
            // run during panic unwinding, where a second panic would abort the
            // process instead of preserving the original failure.
            current_store.borrow_mut().take();
        });
    }
}
