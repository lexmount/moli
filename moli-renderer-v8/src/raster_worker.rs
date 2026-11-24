use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, LazyLock};
use std::thread;

use anyhow::{Context, Result};
use parking_lot::{Condvar, Mutex};
use tokio::sync::oneshot;

type RasterJob = Box<dyn FnOnce() + Send>;

static RASTER_WORKER: LazyLock<Arc<RasterWorker>> = LazyLock::new(Arc::default);

#[derive(Default)]
struct RasterWorker {
    queue: Mutex<RasterQueue>,
    ready: Condvar,
}

#[derive(Default)]
struct RasterQueue {
    next_stream: u64,
    live_streams: usize,
    running: bool,
    pending: VecDeque<(u64, RasterJob)>,
}

/// One output's slot on the process-wide raster thread. The first job starts
/// the thread, which exits once the last stream closes and active work ends.
/// New requests supersede waiting work without changing its queue position.
pub struct RendererRasterStream {
    worker: Arc<RasterWorker>,
    id: u64,
}

impl Default for RendererRasterStream {
    fn default() -> Self {
        RASTER_WORKER.stream()
    }
}

impl RendererRasterStream {
    pub fn submit<T: Send + 'static>(
        &self,
        raster: impl FnOnce() -> T + Send + 'static,
    ) -> Result<oneshot::Receiver<T>> {
        let (result, receiver) = oneshot::channel();
        let job: RasterJob = Box::new(move || {
            if !result.is_closed() {
                let _ = result.send(raster());
            }
        });
        let replaced = {
            let mut queue = self.worker.queue.lock();
            if !queue.running {
                let worker = Arc::clone(&self.worker);
                thread::Builder::new()
                    .name("moli-raster".to_owned())
                    .spawn(move || worker.run())
                    .context("could not start the raster thread")?;
                queue.running = true;
            }
            let replaced = if let Some((_, pending)) =
                queue.pending.iter_mut().find(|(id, _)| *id == self.id)
            {
                Some(std::mem::replace(pending, job))
            } else {
                queue.pending.push_back((self.id, job));
                None
            };
            self.worker.ready.notify_one();
            replaced
        };
        // Releasing a superseded snapshot can drop substantial resources.
        drop(replaced);
        Ok(receiver)
    }
}

impl Drop for RendererRasterStream {
    fn drop(&mut self) {
        let cancelled = {
            let mut queue = self.worker.queue.lock();
            queue.live_streams -= 1;
            let cancelled = queue
                .pending
                .iter()
                .position(|(id, _)| *id == self.id)
                .and_then(|index| queue.pending.remove(index));
            if queue.live_streams == 0 {
                self.worker.ready.notify_one();
            }
            cancelled
        };
        drop(cancelled);
    }
}

impl RasterWorker {
    fn stream(self: &Arc<Self>) -> RendererRasterStream {
        let mut queue = self.queue.lock();
        let id = queue.next_stream;
        queue.next_stream += 1;
        queue.live_streams += 1;
        RendererRasterStream {
            worker: Arc::clone(self),
            id,
        }
    }

    fn next_job(&self) -> Option<RasterJob> {
        let mut queue = self.queue.lock();
        while queue.pending.is_empty() && queue.live_streams != 0 {
            self.ready.wait(&mut queue);
        }
        match queue.pending.pop_front() {
            Some((_, job)) => Some(job),
            None => {
                queue.running = false;
                None
            }
        }
    }

    fn run(&self) {
        while let Some(job) = self.next_job() {
            if catch_unwind(AssertUnwindSafe(job)).is_err() {
                tracing::error!("raster job panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn starts_on_demand_and_keeps_only_the_latest_job_per_stream() {
        let worker = Arc::new(RasterWorker::default());
        let first = worker.stream();
        let second = worker.stream();
        assert!(!worker.queue.lock().running);

        let (entered, wait_for_entry) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel();
        let active = first
            .submit(move || {
                assert_eq!(thread::current().name(), Some("moli-raster"));
                entered.send(()).unwrap();
                wait_for_release
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            })
            .unwrap();
        wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
        let stale = first.submit(|| panic!("superseded job ran")).unwrap();
        let (completed, completion_order) = mpsc::channel();
        let other = second
            .submit({
                let completed = completed.clone();
                move || completed.send(2).unwrap()
            })
            .unwrap();
        let latest = first.submit(move || completed.send(1).unwrap()).unwrap();
        assert!(stale.blocking_recv().is_err());
        assert_eq!(worker.queue.lock().pending.len(), 2);

        release.send(()).unwrap();
        active.blocking_recv().unwrap();
        latest.blocking_recv().unwrap();
        other.blocking_recv().unwrap();
        assert_eq!(completion_order.into_iter().collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn closing_a_stream_releases_its_pending_job() {
        let worker = Arc::new(RasterWorker::default());
        let active_stream = worker.stream();
        let retired_stream = worker.stream();
        let (entered, wait_for_entry) = mpsc::channel();
        let (release, wait_for_release) = mpsc::channel();
        let active = active_stream
            .submit(move || {
                entered.send(()).unwrap();
                wait_for_release
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            })
            .unwrap();
        wait_for_entry.recv_timeout(Duration::from_secs(5)).unwrap();
        let pending = retired_stream.submit(|| panic!("retired job ran")).unwrap();
        drop(retired_stream);
        assert!(pending.blocking_recv().is_err());
        assert!(worker.queue.lock().pending.is_empty());
        release.send(()).unwrap();
        active.blocking_recv().unwrap();
    }
}
