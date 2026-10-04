//! Custom V8 platform that routes foreground tasks to their isolate owner,
//! replacing the need for manual `pump_message_loop()` calls.
//!
//! When V8 background threads complete async work (e.g. WebAssembly
//! compilation), they post foreground continuation tasks through the platform.
//! With V8's `DefaultPlatform` these tasks sit in an internal queue and require
//! explicit pumping. For a Page isolate, this platform transfers each concrete
//! task into the stable Page-owned source so the owner scheduler can arbitrate
//! and execute exactly one task per turn. Worker isolates retain their own
//! thread-local wake-and-drain loop.

use crate::page_task_queue::RendererPageV8ForegroundTaskSender;
pub(crate) use moli_v8_platform::V8PlatformIsolateRegistration;
mod page_routes;
pub(crate) use page_routes::{RendererIsolateForegroundTaskRouter, RendererIsolatePageMembership};

/// Isolate-scoped dispatch target installed in the V8 platform registration.
///
/// V8 posts foreground tasks when background work completes, including async
/// WebAssembly compilation. Page registrations transfer the concrete task to
/// the stable Page source. Worker registrations signal their thread-local loop,
/// which then drains the platform task. Neither path relies on a polling
/// timeout.
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
    Page(RendererIsolateForegroundTaskRouter),
    Worker(tokio::sync::mpsc::UnboundedSender<()>),
}

impl V8ForegroundTaskWake {
    pub(crate) fn page_router(&self) -> Option<RendererIsolateForegroundTaskRouter> {
        match &self.kind {
            V8ForegroundTaskWakeKind::Page(router) => Some(router.clone()),
            V8ForegroundTaskWakeKind::Worker(_) => None,
        }
    }

    pub(crate) fn page(sender: RendererPageV8ForegroundTaskSender) -> anyhow::Result<Self> {
        Ok(Self {
            kind: V8ForegroundTaskWakeKind::Page(RendererIsolateForegroundTaskRouter::new(sender)?),
        })
    }

    pub(crate) fn worker(tx: tokio::sync::mpsc::UnboundedSender<()>) -> Self {
        Self {
            kind: V8ForegroundTaskWakeKind::Worker(tx),
        }
    }

    pub(crate) fn into_platform_wake(self) -> moli_v8_platform::V8ForegroundTaskWake {
        match self.kind {
            V8ForegroundTaskWakeKind::Page(router) => {
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
        "--stack-size=4096 --harmony-import-attributes --js-source-phase-imports"
    } else {
        "--harmony-import-attributes --js-source-phase-imports"
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
