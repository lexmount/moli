use std::{collections::HashMap, sync::Arc};

use moli_parkable_image::ParkableImage;
use parking_lot::Mutex;

use super::{
    ImageDecodeMetadata, ImageResourceRequestIdentity,
    budget::*,
    css::{CssImageResourceRequestIdentity, CssImageResourceStore},
};
use crate::{
    native_bridge::WindowDocumentTaskTarget,
    network::RendererResourceTaskRunner,
    page_task_queue::{
        RendererPageImageLoadEventKind, RendererPageImageLoadEventSender,
        RendererPageImageLoadEventTaskId,
    },
};

const IMAGE_DECODE_CONCURRENCY: usize = 4;

#[derive(Clone)]
pub(super) struct ImageDecodeCoordinator {
    inner: Arc<ImageDecodeCoordinatorInner>,
}

struct ImageDecodeCoordinatorInner {
    budget: SharedImageResourceBudget,
    concurrency: Arc<tokio::sync::Semaphore>,
    completions: Mutex<HashMap<RendererPageImageLoadEventTaskId, ImageDecodeCompletion>>,
}

impl Default for ImageDecodeCoordinator {
    fn default() -> Self {
        Self {
            inner: Arc::new(ImageDecodeCoordinatorInner {
                budget: SharedImageResourceBudget::default(),
                concurrency: Arc::new(tokio::sync::Semaphore::new(IMAGE_DECODE_CONCURRENCY)),
                completions: Mutex::new(HashMap::new()),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageDecodeQueueError {
    JobLimit,
    EncodedByteLimit,
    EncodedSource,
}

pub(super) struct ReadyDecodedImage {
    pub(super) content: DecodedImageContent,
    pub(super) decoded_bytes_permit: ImageDecodedBytesPermit,
}

pub(super) enum DecodedImageContent {
    Raster(Arc<moli_image::RgbaImage>),
    Svg(Arc<moli_image::SvgImage>),
}

pub(super) enum ImageDecodeResult {
    Ready(ReadyDecodedImage),
    Failed(String),
}

pub(super) struct ImageDecodeCompletion {
    pub(super) identity: ImageResourceRequestIdentity,
    pub(super) target: WindowDocumentTaskTarget,
    pub(super) result: ImageDecodeResult,
    _job_permit: ImageDecodeJobPermit,
}

impl ImageDecodeCompletion {
    pub(super) fn kind(&self) -> RendererPageImageLoadEventKind {
        match self.result {
            ImageDecodeResult::Ready(_) => RendererPageImageLoadEventKind::Load,
            ImageDecodeResult::Failed(_) => RendererPageImageLoadEventKind::Error,
        }
    }
}

impl ImageDecodeCoordinator {
    pub(super) fn submit(
        &self,
        runner: RendererResourceTaskRunner,
        sender: RendererPageImageLoadEventSender,
        identity: ImageResourceRequestIdentity,
        target: WindowDocumentTaskTarget,
        metadata: ImageDecodeMetadata,
        encoded: ParkableImage,
    ) -> Result<(), ImageDecodeQueueError> {
        let coordinator = self.clone();
        let task_id = RendererPageImageLoadEventTaskId::new(identity.element, identity.sequence);
        self.submit_job(runner, metadata, encoded, move |result, job_permit| {
            let completion = ImageDecodeCompletion {
                identity,
                target,
                result,
                _job_permit: job_permit,
            };
            let kind = completion.kind();
            let previous = coordinator
                .inner
                .completions
                .lock()
                .insert(task_id, completion);
            debug_assert!(
                previous.is_none(),
                "one image sequence queued two decode completions"
            );
            if sender.send(target, task_id, kind).is_err() {
                coordinator.inner.completions.lock().remove(&task_id);
            }
        })
    }

    pub(super) fn submit_css(
        &self,
        runner: RendererResourceTaskRunner,
        store: CssImageResourceStore,
        identity: CssImageResourceRequestIdentity,
        metadata: ImageDecodeMetadata,
        encoded: ParkableImage,
    ) -> Result<(), ImageDecodeQueueError> {
        self.submit_job(
            runner,
            metadata,
            encoded,
            move |result, _job_permit| match result {
                ImageDecodeResult::Ready(ready) => {
                    if !store.complete_decode(&identity, ready) {
                        tracing::debug!(
                            document = identity.document_handle.index(),
                            request_id = identity.request_id,
                            "discarded stale CSS image decode completion"
                        );
                    }
                }
                ImageDecodeResult::Failed(error) => {
                    tracing::debug!(
                        document = identity.document_handle.index(),
                        request_id = identity.request_id,
                        %error,
                        "CSS image resource decode failed"
                    );
                    let _ = store.fail(&identity);
                }
            },
        )
    }

    pub(super) fn submit_preload(
        &self,
        runner: RendererResourceTaskRunner,
        metadata: ImageDecodeMetadata,
        encoded: ParkableImage,
        complete: impl FnOnce(ImageDecodeResult) + Send + 'static,
    ) -> Result<(), ImageDecodeQueueError> {
        self.submit_job(runner, metadata, encoded, move |result, _job_permit| {
            complete(result);
        })
    }

    fn submit_job(
        &self,
        runner: RendererResourceTaskRunner,
        metadata: ImageDecodeMetadata,
        encoded: ParkableImage,
        complete: impl FnOnce(ImageDecodeResult, ImageDecodeJobPermit) + Send + 'static,
    ) -> Result<(), ImageDecodeQueueError> {
        let encoded_len = encoded.len();
        let job_permit = self
            .inner
            .budget
            .admit_job(encoded_len)
            .map_err(|error| match error {
                ImageResourceBudgetError::JobLimit => ImageDecodeQueueError::JobLimit,
                ImageResourceBudgetError::EncodedByteLimit
                | ImageResourceBudgetError::DecodedByteLimit => {
                    ImageDecodeQueueError::EncodedByteLimit
                }
            })?;
        let snapshot = encoded
            .snapshot()
            .map_err(|_| ImageDecodeQueueError::EncodedSource)?;
        let coordinator = self.clone();
        runner.spawn(async move {
            let Ok(_concurrency) = coordinator.inner.concurrency.clone().acquire_owned().await
            else {
                return;
            };
            let budget = coordinator.inner.budget.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                let result = (|| {
                    let retained_bytes = metadata
                        .retained_byte_len(encoded_len)
                        .ok_or_else(|| "decoded image byte estimate overflowed".to_owned())?;
                    let decoded_bytes_permit =
                        budget.reserve_decoded(retained_bytes).map_err(|error| {
                            format!("decoded image budget rejected the resource: {error:?}")
                        })?;
                    let content = match metadata {
                        ImageDecodeMetadata::Raster(metadata) => {
                            let decoded =
                                moli_image::decode_raster_image_with_metadata(&snapshot, metadata)
                                    .map_err(|error| error.to_string())?;
                            DecodedImageContent::Raster(Arc::new(decoded.image))
                        }
                        ImageDecodeMetadata::Svg(metadata) => {
                            let decoded =
                                moli_image::decode_svg_image_with_metadata(&snapshot, metadata)
                                    .map_err(|error| error.to_string())?;
                            DecodedImageContent::Svg(Arc::new(decoded))
                        }
                    };
                    Ok::<_, String>((content, decoded_bytes_permit))
                })();
                result.map(|(content, decoded_bytes_permit)| ReadyDecodedImage {
                    content,
                    decoded_bytes_permit,
                })
            })
            .await;
            let mut job_permit = job_permit;
            job_permit.release_encoded_bytes();
            let result = match decoded {
                Ok(Ok(ready)) => ImageDecodeResult::Ready(ready),
                Ok(Err(error)) => ImageDecodeResult::Failed(error),
                Err(error) => {
                    ImageDecodeResult::Failed(format!("image decode worker failed: {error}"))
                }
            };
            complete(result, job_permit);
        });
        Ok(())
    }

    pub(super) fn completion_kind(
        &self,
        task_id: RendererPageImageLoadEventTaskId,
        identity: &ImageResourceRequestIdentity,
        target: WindowDocumentTaskTarget,
    ) -> Option<RendererPageImageLoadEventKind> {
        self.inner
            .completions
            .lock()
            .get(&task_id)
            .filter(|completion| completion.identity == *identity && completion.target == target)
            .map(ImageDecodeCompletion::kind)
    }

    pub(super) fn take_completion(
        &self,
        task_id: RendererPageImageLoadEventTaskId,
        identity: &ImageResourceRequestIdentity,
        target: WindowDocumentTaskTarget,
        kind: RendererPageImageLoadEventKind,
    ) -> Option<ImageDecodeCompletion> {
        let mut completions = self.inner.completions.lock();
        let completion = completions.get(&task_id)?;
        if completion.identity != *identity
            || completion.target != target
            || completion.kind() != kind
        {
            return None;
        }
        completions.remove(&task_id)
    }

    pub(super) fn discard_completion(&self, task_id: RendererPageImageLoadEventTaskId) -> bool {
        self.inner.completions.lock().remove(&task_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use moli_disk_pool::DiskPool;
    use moli_parkable_image::{ParkableImageManager, ParkableImagePolicy};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn decode_releases_its_snapshot_for_manager_parking() {
        let pixels = moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 255]).unwrap();
        let bytes = moli_image::encode_png(&pixels).unwrap().bytes;
        let metadata = moli_image::probe_raster_image(&bytes).unwrap();
        let pool = DiskPool::new(None).unwrap();
        let manager = ParkableImageManager::new(
            Some(pool.clone()),
            ParkableImagePolicy {
                min_size_to_park: 1,
                parking_delay: Duration::ZERO,
            },
        );
        let encoded = manager.from_frozen_bytes(bytes.clone());
        let coordinator = ImageDecodeCoordinator::default();
        let runner = RendererResourceTaskRunner::from_current_tokio().unwrap();
        let (complete_tx, complete_rx) = tokio::sync::oneshot::channel();

        coordinator
            .submit_preload(
                runner,
                ImageDecodeMetadata::Raster(metadata),
                encoded.clone(),
                move |result| {
                    let _ = complete_tx.send(result);
                },
            )
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), complete_rx)
            .await
            .expect("image decode should complete")
            .expect("decode callback should send its result");
        assert!(matches!(result, ImageDecodeResult::Ready(_)));
        manager.park_images_now();
        assert_eq!(manager.diagnostics().parked_count, 1);
        assert_eq!(manager.diagnostics().retained_memory_bytes, 0);
        assert_eq!(pool.diagnostics().disk_footprint_bytes, bytes.len() as u64);
        assert_eq!(encoded.snapshot().unwrap().as_ref(), bytes);
    }
}
