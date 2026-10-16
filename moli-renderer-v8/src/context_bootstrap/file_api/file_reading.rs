use super::file_reader::{FileReaderTask, FileReaderTaskEffect};
use crate::page_task_queue::{PageFileReadingTargetEffect, RendererPageFileReadingTaskKind};

/// Window and Worker completions share the same file-reading FIFO, including
/// FileReader phases and Blob byte delivery. Queuing never consults author JS.
pub(crate) enum FileReadingTask {
    Reader(FileReaderTask),
    Blob(crate::blob::BlobReadTask),
}

impl FileReadingTask {
    pub(crate) fn kind(&self) -> RendererPageFileReadingTaskKind {
        match self {
            Self::Reader(task) => RendererPageFileReadingTaskKind::FileReader(task.phase()),
            Self::Blob(_) => RendererPageFileReadingTaskKind::BlobRead,
        }
    }

    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) -> PageFileReadingTargetEffect {
        match self {
            Self::Reader(task) => match task.invoke(scope) {
                FileReaderTaskEffect::Advanced => {
                    PageFileReadingTargetEffect::FileReaderPhaseAdvanced
                }
                FileReaderTaskEffect::Stale => {
                    PageFileReadingTargetEffect::DiscardedStaleReaderRequest
                }
            },
            Self::Blob(task) => {
                task.settle(scope);
                PageFileReadingTargetEffect::BlobReadCompleted
            }
        }
    }
}

pub(crate) fn queue_file_reading_task(
    scope: &mut v8::PinScope<'_, '_>,
    task: FileReadingTask,
) -> bool {
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        return unsafe { &mut *host_ptr }.queue_file_reading_task(scope, task);
    }
    crate::worker::queue_worker_file_reading_task(scope, task)
}

pub(crate) fn flush_one_pending_file_reading_task(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let Some(task) = crate::worker::take_worker_file_reading_task(scope) else {
        return false;
    };
    let _ = task.invoke(scope);
    true
}
