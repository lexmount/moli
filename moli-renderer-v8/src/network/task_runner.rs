use std::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, mpsc},
    thread,
};

/// Browser-context-owned executor for native resource coordination.
///
/// The renderer and Worker executors may spend an entire turn running author
/// JavaScript. Resource futures must continue submitting requests and consuming
/// native responses during that turn. Only this thread-affine owner can stop
/// and join the executor; request-side runners cannot extend its lifetime.
#[derive(Debug)]
pub(in crate::network) struct RendererResourceTaskOwner {
    runner: RendererResourceTaskRunner,
    shutdown: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Option<thread::JoinHandle<()>>,
    _thread_affine: PhantomData<Rc<()>>,
}

impl RendererResourceTaskOwner {
    pub(in crate::network) fn new() -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("lm-resource-io".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .max_blocking_threads(
                        crate::tokio_blocking_budget::tokio_blocking_thread_budget(),
                    )
                    .build()
                    .expect("browser resource executor should initialize");
                if ready_tx.send(runtime.handle().clone()).is_ok() {
                    let _ = runtime.block_on(shutdown_rx);
                }
                // Dropping the runtime cancels pending futures and joins its
                // blocking pool before the owner joins this thread.
            })
            .expect("browser resource executor thread should start");
        let handle = match ready_rx.recv() {
            Ok(handle) => handle,
            Err(error) => {
                let _ = thread.join();
                panic!("browser resource executor did not initialize: {error}");
            }
        };
        Self {
            runner: RendererResourceTaskRunner::from_tokio_handle(handle),
            shutdown: RefCell::new(Some(shutdown_tx)),
            thread: Some(thread),
            _thread_affine: PhantomData,
        }
    }

    pub(in crate::network) fn runner(&self) -> RendererResourceTaskRunner {
        self.runner.clone()
    }

    pub(in crate::network) fn request_shutdown(&self) {
        if let Some(shutdown) = self.shutdown.borrow_mut().take() {
            let _ = shutdown.send(());
        }
    }

    pub(in crate::network) fn join(&mut self) {
        self.request_shutdown();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("browser resource executor panicked");
        }
    }
}

impl Drop for RendererResourceTaskOwner {
    fn drop(&mut self) {
        self.join();
    }
}

/// Executor selected by a concrete resource-loading authority.
///
/// Resource code must never rely on whichever Tokio runtime happens to be
/// entered when a DOM callback runs. A committed Document or live Worker
/// captures its runner once, then every asynchronous load spawned for that
/// execution context uses the captured runner.
#[derive(Clone)]
pub struct RendererResourceTaskRunner {
    handle: Arc<tokio::runtime::Handle>,
}

impl RendererResourceTaskRunner {
    /// Captures an explicitly supplied runtime, including test executors.
    /// Browser contexts supply their independent resource executor instead of
    /// capturing the renderer's JavaScript executor.
    pub fn from_current_tokio() -> anyhow::Result<Self> {
        let handle = tokio::runtime::Handle::try_current().map_err(|error| {
            anyhow::anyhow!("resource authority requires a Tokio runtime: {error}")
        })?;
        Ok(Self::from_tokio_handle(handle))
    }

    pub(crate) fn from_tokio_handle(handle: tokio::runtime::Handle) -> Self {
        Self {
            handle: Arc::new(handle),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("resource test runtime should initialize")
        });
        Self::from_tokio_handle(runtime.handle().clone())
    }

    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        drop(self.spawn_abortable(task));
    }

    pub(crate) fn spawn_abortable(
        &self,
        task: impl Future<Output = ()> + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        self.handle.spawn(task)
    }

    pub(crate) fn shares_executor_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.handle, &other.handle)
    }
}

impl std::fmt::Debug for RendererResourceTaskRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererResourceTaskRunner")
            .field("kind", &"tokio")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captured_tokio_runner_executes_resource_work() {
        let runner = RendererResourceTaskRunner::from_current_tokio()
            .expect("Tokio test should expose its runtime");
        let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();

        runner.spawn(async move {
            let _ = completed_tx.send(());
        });

        completed_rx
            .await
            .expect("captured resource runner should poll spawned work");
    }
}
