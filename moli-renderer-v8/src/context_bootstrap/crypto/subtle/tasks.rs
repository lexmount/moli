use super::*;

/// Where a finished WebCrypto blocking task routes its completion.
///
/// `Page` settles through the renderer owner queue (window/iframe runtimes);
/// `Worker` settles through the dedicated/shared worker event loop. Both lanes
/// snapshot inputs at the V8 boundary, run the primitive on the blocking pool,
/// and reuse the same `WebCryptoTaskResult` / rejection mapping, so the
/// renderer-visible promise outcome is identical regardless of lane.
pub(crate) enum WebCryptoCompletionSink {
    Page(crate::page_task_queue::RendererPageWebCryptoTaskProducer),
    Worker {
        task_id: u64,
        tx: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerWebCryptoCompletion>,
    },
}

impl WebCryptoCompletionSink {
    /// Deliver exactly one result through the owner captured at registration.
    pub(crate) fn send(self, result: Result<WebCryptoTaskResult, WebCryptoRejection>) {
        match self {
            WebCryptoCompletionSink::Page(producer) => {
                let _ = producer.send(result);
            }
            WebCryptoCompletionSink::Worker { task_id, tx } => {
                let _ = tx.send(crate::worker::WorkerWebCryptoCompletion { task_id, result });
            }
        }
    }
}

pub(crate) fn register_webcrypto_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: &PendingCryptoPromise<'s>,
) -> Option<(tokio::runtime::Handle, WebCryptoCompletionSink)> {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return None;
    };
    // Page runtimes expose a JsContextHost on the global bridge and settle
    // through the renderer owner queue. Worker globals do not, so fall back to
    // the worker-owned completion lane routed through the worker event loop.
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the global bridge stores the current realm's live
        // JsContextHost. Registration runs synchronously inside the V8
        // callback before control returns to the owner. The returned producer
        // captures the exact PageVm, Window realm, and never-reused task id.
        let producer =
            unsafe { &mut *host_ptr }.register_pending_webcrypto_task(scope, promise.resolver())?;
        return Some((handle, WebCryptoCompletionSink::Page(producer)));
    }
    let (task_id, completion_tx) =
        crate::worker::register_worker_webcrypto_task(scope, promise.resolver())?;
    Some((
        handle,
        WebCryptoCompletionSink::Worker {
            task_id,
            tx: completion_tx,
        },
    ))
}

pub(crate) fn spawn_webcrypto_bytes_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<Vec<u8>, WebCryptoError> + Send + 'static,
{
    // Callers must finish all JS-observable normalization before this point;
    // the blocking closure may only touch snapshotted bytes and enum params.
    handle.spawn_blocking(move || {
        let result = operation()
            .map(WebCryptoTaskResult::Bytes)
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_key_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<CryptoKeyClonePayload, WebCryptoRejection> + Send + 'static,
{
    // The task receives a clone payload rather than a V8 handle so all
    // primitive work and key-material import stay off the renderer callback.
    handle.spawn_blocking(move || {
        let result = operation().map(|payload| WebCryptoTaskResult::CryptoKey(Box::new(payload)));
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_key_pair_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<(CryptoKeyClonePayload, CryptoKeyClonePayload), WebCryptoError>
        + Send
        + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation()
            .map(
                |(private_key, public_key)| WebCryptoTaskResult::CryptoKeyPair {
                    private_key: Box::new(private_key),
                    public_key: Box::new(public_key),
                },
            )
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_bool_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<bool, WebCryptoError> + Send + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation()
            .map(WebCryptoTaskResult::Bool)
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_result_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<WebCryptoTaskResult, WebCryptoRejection> + Send + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation();
        completion_tx.send(result);
    });
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn webcrypto_bytes_tasks_start_independently_on_the_blocking_pool() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(2)
            .enable_time()
            .build()
            .expect("WebCrypto dispatch test runtime should build");
        let handle = runtime.handle().clone();
        let (completion_tx, mut completion_rx) = tokio::sync::mpsc::unbounded_channel();
        let (started_tx, started_rx) = mpsc::channel();
        let mut release_txs = Vec::new();

        for (task_id, byte) in [(1_u64, 1_u8), (2, 2)] {
            let (release_tx, release_rx) = mpsc::channel();
            release_txs.push(release_tx);
            let started_tx = started_tx.clone();
            spawn_webcrypto_bytes_task(
                handle.clone(),
                WebCryptoCompletionSink::Worker {
                    task_id,
                    tx: completion_tx.clone(),
                },
                move || {
                    started_tx
                        .send(task_id)
                        .expect("dispatch test should still receive task starts");
                    release_rx
                        .recv()
                        .expect("dispatch test should release each blocking task");
                    Ok(vec![byte])
                },
            );
        }
        drop(started_tx);
        drop(completion_tx);

        // Hold both operations open and require both blocking closures to
        // start before either is allowed to finish. Unlike a wall-clock
        // speedup assertion, this observes independent dispatch directly and
        // remains valid on a single-CPU or heavily loaded runner.
        let mut started_task_ids = Vec::new();
        for _ in 0..2 {
            if let Ok(task_id) = started_rx.recv_timeout(DISPATCH_TIMEOUT) {
                started_task_ids.push(task_id);
            }
        }
        for release_tx in release_txs {
            release_tx
                .send(())
                .expect("blocking task should remain live until release");
        }
        started_task_ids.sort_unstable();
        assert_eq!(
            started_task_ids,
            vec![1, 2],
            "both WebCrypto operations must enter the blocking pool before either completes"
        );

        let mut completed_task_ids = runtime.block_on(async {
            tokio::time::timeout(DISPATCH_TIMEOUT, async {
                let first = completion_rx
                    .recv()
                    .await
                    .expect("first WebCrypto completion should arrive");
                let second = completion_rx
                    .recv()
                    .await
                    .expect("second WebCrypto completion should arrive");
                [first.task_id, second.task_id]
            })
            .await
            .expect("independently dispatched WebCrypto tasks should complete")
        });
        completed_task_ids.sort_unstable();
        assert_eq!(completed_task_ids, [1, 2]);
    }
}
