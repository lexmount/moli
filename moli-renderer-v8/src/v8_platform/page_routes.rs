use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use parking_lot::Mutex;

use crate::{
    page_task_queue::{RendererPageV8ForegroundTask, RendererPageV8ForegroundTaskSender},
    resource_ready::ReadyPageTask,
    runtime::RendererPageToken,
};

/// One isolate's foreground tasks can run through any admitted live Page.
/// Membership is owned by the Page's task source, never by a Document realm.
#[derive(Clone, Debug)]
pub(crate) struct RendererIsolateForegroundTaskRouter {
    pages: Arc<Mutex<Vec<RendererPageV8ForegroundTaskSender>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererIsolatePageMembership {
    inner: Arc<RendererIsolatePageMembershipState>,
}

#[derive(Debug)]
struct RendererIsolatePageMembershipState {
    router: RendererIsolateForegroundTaskRouter,
    page: RendererPageToken,
    active: AtomicBool,
}

impl RendererIsolatePageMembership {
    pub(crate) fn is_active(&self) -> bool {
        self.inner.active.load(Ordering::Acquire)
    }

    pub(crate) fn admit_related_page(
        &self,
        route: RendererPageV8ForegroundTaskSender,
    ) -> anyhow::Result<()> {
        self.inner.router.admit_page(Some(self), route)
    }

    pub(crate) fn retire_with_queued_tasks(
        &self,
        take_tasks: impl FnOnce() -> VecDeque<ReadyPageTask<RendererPageV8ForegroundTask>>,
    ) {
        self.inner.active.store(false, Ordering::Release);
        let mut discarded = Vec::new();
        let mut pages = self.inner.router.pages.lock();
        pages.retain(|route| route.page_token() != self.inner.page);
        // Hold the dispatch boundary until every previously accepted task is
        // transferred. New V8 publications must follow them on the survivor.
        for mut ready in take_tasks() {
            loop {
                let Some(route) = pages.first() else {
                    discarded.push(ready.value.into_task());
                    break;
                };
                match route.transfer(ready) {
                    Ok(()) => break,
                    Err(returned) => {
                        ready = returned;
                        pages.remove(0);
                    }
                }
            }
        }
        drop(pages);
        // Native task destructors may themselves post foreground work.
        drop(discarded);
    }
}

impl RendererIsolatePageMembershipState {
    fn retire(&self) {
        if self.active.swap(false, Ordering::AcqRel) {
            self.router
                .pages
                .lock()
                .retain(|route| route.page_token() != self.page);
        }
    }
}

impl Drop for RendererIsolatePageMembershipState {
    fn drop(&mut self) {
        self.retire();
    }
}

impl RendererIsolateForegroundTaskRouter {
    pub(crate) fn new(route: RendererPageV8ForegroundTaskSender) -> anyhow::Result<Self> {
        let router = Self {
            pages: Arc::new(Mutex::new(Vec::new())),
        };
        router.admit_page(None, route)?;
        Ok(router)
    }

    pub(crate) fn retire(&self) {
        let mut pages = self.pages.lock();
        for route in pages.drain(..) {
            if let Ok(membership) = route.isolate_membership() {
                membership.inner.active.store(false, Ordering::Release);
            }
        }
    }

    fn admit_page(
        &self,
        source: Option<&RendererIsolatePageMembership>,
        route: RendererPageV8ForegroundTaskSender,
    ) -> anyhow::Result<()> {
        let page = route.page_token();
        let mut pages = self.pages.lock();
        if let Some(source) = source {
            anyhow::ensure!(
                source.inner.active.load(Ordering::Acquire)
                    && pages
                        .iter()
                        .any(|route| route.page_token() == source.inner.page),
                "a retired Page cannot admit a related isolate Page"
            );
            anyhow::ensure!(
                source.inner.page.local_host_id() == page.local_host_id(),
                "related isolate Pages must share their renderer owner"
            );
        }
        anyhow::ensure!(
            !pages.iter().any(|route| route.page_token() == page),
            "the isolate already admits this Page"
        );
        let membership = RendererIsolatePageMembership {
            inner: Arc::new(RendererIsolatePageMembershipState {
                router: self.clone(),
                page,
                active: AtomicBool::new(false),
            }),
        };
        // A failed binding must not run a retirement callback under `pages`.
        // The source owns the successful membership before it becomes live.
        route.bind_isolate_membership(membership.clone())?;
        pages.push(route);
        membership.inner.active.store(true, Ordering::Release);
        drop(pages);
        drop(membership);
        Ok(())
    }

    pub(crate) fn dispatch(&self, mut task: moli_v8_platform::V8ForegroundTask) {
        let mut pages = self.pages.lock();
        while let Some(route) = pages.first() {
            match route.send(task, self.clone()) {
                Ok(()) => return,
                Err(returned_task) => {
                    task = returned_task;
                    pages.remove(0);
                }
            }
        }
        // No Page remains. Drop the raw task without a redispatch wrapper.
    }
}
