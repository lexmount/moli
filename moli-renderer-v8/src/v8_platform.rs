//! Custom V8 platform that routes foreground tasks to their script-agent owner,
//! replacing the need for manual `pump_message_loop()` calls.
//!
//! When V8 background threads complete async work (e.g. WebAssembly
//! compilation), they post foreground continuation tasks through the platform.
//! With V8's `DefaultPlatform` these tasks sit in an internal queue and require
//! explicit pumping. For a document script agent, this platform transfers each
//! concrete task into one live member Page source so the owner scheduler can
//! arbitrate and execute exactly one task per turn. Other member Pages then
//! receive typed checkpoint tasks because their realms can own microtasks
//! released by that isolate-level continuation. Worker isolates retain their
//! own thread-local wake-and-drain loop.

use crate::{
    browsing_context_model::{ScriptAgentId, ScriptAgentScope},
    page_task_queue::RendererPageV8ForegroundTaskSender,
    runtime::PageId,
};
pub(crate) use moli_v8_platform::V8PlatformIsolateRegistration;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Isolate-lifetime routing authority for the Pages admitted to one script
/// agent. Fresh Pages install one member. A renderer-accepted, opener-related
/// auxiliary Page may explicitly join that agent without widening admission
/// to the whole renderer owner.
#[derive(Clone)]
pub(crate) struct RendererScriptAgentV8ForegroundTaskRouter {
    inner: Arc<Mutex<RendererScriptAgentV8ForegroundTaskRouterState>>,
}

struct RendererScriptAgentV8ForegroundTaskRouterState {
    script_agent_id: ScriptAgentId,
    scope: ScriptAgentScope,
    page_routes: Vec<(PageId, RendererPageV8ForegroundTaskSender)>,
}

impl std::fmt::Debug for RendererScriptAgentV8ForegroundTaskRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.inner.lock();
        f.debug_struct("RendererScriptAgentV8ForegroundTaskRouter")
            .field("script_agent_id", &state.script_agent_id)
            .field("scope", &state.scope)
            .field("page_count", &state.page_routes.len())
            .finish()
    }
}

/// RAII membership shared by a stable Page environment and all of its
/// replacement Document generations.
#[derive(Clone, Debug)]
pub(crate) struct RendererScriptAgentPageMembership {
    inner: Arc<RendererScriptAgentPageMembershipState>,
}

/// Post-task capability used to checkpoint the other live Page realms only
/// after the concrete isolate-level V8 task has run exactly once.
#[derive(Clone, Debug)]
pub(crate) struct RendererScriptAgentV8ForegroundTaskCompletion {
    router: RendererScriptAgentV8ForegroundTaskRouter,
}

impl RendererScriptAgentV8ForegroundTaskCompletion {
    pub(crate) fn enqueue_peer_checkpoints(&self, task_page_id: PageId) {
        self.router.enqueue_peer_checkpoints(task_page_id);
    }

    pub(crate) fn redispatch_after_page_retirement(self, task: moli_v8_platform::V8ForegroundTask) {
        self.router.dispatch(task);
    }
}

#[derive(Debug)]
struct RendererScriptAgentPageMembershipState {
    router: RendererScriptAgentV8ForegroundTaskRouter,
    script_agent_id: ScriptAgentId,
    page_id: PageId,
    active: AtomicBool,
}

impl RendererScriptAgentPageMembership {
    pub(crate) fn script_agent_id(&self) -> ScriptAgentId {
        self.inner.script_agent_id
    }

    pub(crate) fn page_id(&self) -> PageId {
        self.inner.page_id
    }

    /// Admits one explicitly related Page through the capability retained by
    /// a live source Page.
    ///
    /// Native callbacks can invoke this while the shared document isolate is
    /// already mutably borrowed. Keeping admission on the membership avoids a
    /// second borrow of the isolate holder merely to reach its router.
    pub(crate) fn admit_related_page(
        &self,
        page_route: RendererPageV8ForegroundTaskSender,
    ) -> anyhow::Result<RendererScriptAgentPageMembership> {
        self.inner.router.admit_related_page_from(
            self.inner.page_id,
            &self.inner.active,
            page_route,
        )
    }

    pub(crate) fn retire(&self) {
        if self.inner.active.swap(false, Ordering::AcqRel) {
            self.inner.router.remove_page(self.inner.page_id);
        }
    }
}

impl Drop for RendererScriptAgentPageMembershipState {
    fn drop(&mut self) {
        if self.active.swap(false, Ordering::AcqRel) {
            self.router.remove_page(self.page_id);
        }
    }
}

impl RendererScriptAgentV8ForegroundTaskRouter {
    pub(crate) fn new(
        script_agent_id: ScriptAgentId,
        initial_page_route: RendererPageV8ForegroundTaskSender,
    ) -> (Self, RendererScriptAgentPageMembership) {
        let router = Self {
            inner: Arc::new(Mutex::new(RendererScriptAgentV8ForegroundTaskRouterState {
                script_agent_id,
                scope: ScriptAgentScope::PageScriptEnvironment,
                page_routes: Vec::new(),
            })),
        };
        let membership = router
            .admit_page(initial_page_route, ScriptAgentScope::PageScriptEnvironment)
            .expect("a new script agent must accept its initial Page route");
        (router, membership)
    }

    fn admit_related_page_from(
        &self,
        source_page_id: PageId,
        source_active: &AtomicBool,
        page_route: RendererPageV8ForegroundTaskSender,
    ) -> anyhow::Result<RendererScriptAgentPageMembership> {
        let mut state = self.inner.lock();
        anyhow::ensure!(
            source_active.load(Ordering::Acquire)
                && state
                    .page_routes
                    .iter()
                    .any(|(page_id, _)| *page_id == source_page_id),
            "retired script-agent Page membership cannot admit a related Page"
        );
        self.admit_page_with_locked_state(
            &mut state,
            page_route,
            ScriptAgentScope::RelatedPageGroup,
        )
    }

    fn admit_page(
        &self,
        page_route: RendererPageV8ForegroundTaskSender,
        scope: ScriptAgentScope,
    ) -> anyhow::Result<RendererScriptAgentPageMembership> {
        let mut state = self.inner.lock();
        self.admit_page_with_locked_state(&mut state, page_route, scope)
    }

    fn admit_page_with_locked_state(
        &self,
        state: &mut RendererScriptAgentV8ForegroundTaskRouterState,
        page_route: RendererPageV8ForegroundTaskSender,
        scope: ScriptAgentScope,
    ) -> anyhow::Result<RendererScriptAgentPageMembership> {
        let page_id = page_route.page_id();
        anyhow::ensure!(
            !state
                .page_routes
                .iter()
                .any(|(existing_page_id, _)| *existing_page_id == page_id),
            "script agent {} already retains Page {}",
            state.script_agent_id.value(),
            page_id.as_u64()
        );
        state.scope = scope;
        state.page_routes.push((page_id, page_route));
        Ok(RendererScriptAgentPageMembership {
            inner: Arc::new(RendererScriptAgentPageMembershipState {
                router: self.clone(),
                script_agent_id: state.script_agent_id,
                page_id,
                active: AtomicBool::new(true),
            }),
        })
    }

    fn remove_page(&self, page_id: PageId) {
        let mut state = self.inner.lock();
        state
            .page_routes
            .retain(|(retained_page_id, _)| *retained_page_id != page_id);
    }

    fn dispatch(&self, task: moli_v8_platform::V8ForegroundTask) {
        let mut state = self.inner.lock();
        // V8 identifies only the isolate, not the originating Context. Run the
        // concrete task through one live member; its completion schedules the
        // other member checkpoints needed for realm-owned promise jobs.
        let completion = RendererScriptAgentV8ForegroundTaskCompletion {
            router: self.clone(),
        };
        let mut task = task;
        while let Some((_, route)) = state.page_routes.first() {
            match route.send_script_agent_task(task, completion.clone()) {
                Ok(()) => return,
                Err((returned_task, _)) => {
                    task = returned_task;
                    state.page_routes.remove(0);
                }
            }
        }
    }

    fn enqueue_peer_checkpoints(&self, task_page_id: PageId) {
        let mut state = self.inner.lock();
        let mut closed_page_ids = Vec::new();
        for (page_id, route) in &state.page_routes {
            if *page_id != task_page_id && route.send_script_agent_checkpoint().is_err() {
                closed_page_ids.push(*page_id);
            }
        }
        if !closed_page_ids.is_empty() {
            state
                .page_routes
                .retain(|(page_id, _)| !closed_page_ids.contains(page_id));
        }
    }

    pub(crate) fn scope(&self) -> ScriptAgentScope {
        self.inner.lock().scope
    }

    pub(crate) fn page_count(&self) -> usize {
        self.inner.lock().page_routes.len()
    }
}

/// Isolate-scoped dispatch target installed in the V8 platform registration.
///
/// V8 posts foreground tasks when background work completes, including async
/// WebAssembly compilation. Script-agent registrations transfer the concrete
/// task to one live Page source and checkpoint the remaining member realms.
/// Worker registrations signal their thread-local loop, which then drains the
/// platform task. Neither path relies on a polling timeout.
///
/// This wake is isolate-scoped. Chromium's Gin platform also exposes foreground
/// task runners from `V8Platform::GetForegroundTaskRunner(v8::Isolate*)`; Blink
/// inspector context groups are attached at DevTools/session/current-context
/// boundaries, not to foreground task callbacks themselves.
#[derive(Clone, Debug)]
pub(crate) struct V8ForegroundTaskWake {
    kind: V8ForegroundTaskWakeKind,
}

#[derive(Clone, Debug)]
enum V8ForegroundTaskWakeKind {
    ScriptAgent(RendererScriptAgentV8ForegroundTaskRouter),
    Worker(tokio::sync::mpsc::UnboundedSender<()>),
}

impl V8ForegroundTaskWake {
    pub(crate) fn script_agent(router: RendererScriptAgentV8ForegroundTaskRouter) -> Self {
        Self {
            kind: V8ForegroundTaskWakeKind::ScriptAgent(router),
        }
    }

    pub(crate) fn worker(tx: tokio::sync::mpsc::UnboundedSender<()>) -> Self {
        Self {
            kind: V8ForegroundTaskWakeKind::Worker(tx),
        }
    }

    pub(crate) fn into_platform_wake(self) -> moli_v8_platform::V8ForegroundTaskWake {
        match self.kind {
            V8ForegroundTaskWakeKind::ScriptAgent(router) => {
                moli_v8_platform::V8ForegroundTaskWake::queued(move |task| {
                    router.dispatch(task);
                })
            }
            V8ForegroundTaskWakeKind::Worker(tx) => {
                moli_v8_platform::V8ForegroundTaskWake::new(move || {
                    let _ = tx.send(());
                })
            }
        }
    }
}

pub(crate) fn initialization_flags() -> &'static str {
    if cfg!(debug_assertions) {
        // Debug Rust frames are much larger than release frames. Keep V8's
        // debug JS stack budget above its small default, but still well below
        // the render runtime's 8 MiB native stack.
        "--stack-size=4096 --harmony-import-attributes --js-source-phase-imports --experimental-wasm-type-reflection"
    } else {
        "--harmony-import-attributes --js-source-phase-imports --experimental-wasm-type-reflection"
    }
}

/// Create the shared V8 platform using our custom foreground task routing.
///
/// `thread_pool_size = 0` lets V8 choose the default worker count.
/// `idle_task_support = false` because we don't implement idle scheduling.
/// `unprotected = false` keeps V8's thread-isolated allocation protection
/// (Memory Protection Keys / pkeys) enabled. This is safe because our
/// production isolates are created on the render_runtime thread — V8 is
/// initialized exactly once and subsequent isolate creations on the same or
/// child threads do not violate pkey constraints on current Linux kernels
/// (pkeys are per-process, not per-thread-of-init).
pub(crate) fn create_platform() -> v8::SharedRef<v8::Platform> {
    moli_v8_platform::create_platform()
}
