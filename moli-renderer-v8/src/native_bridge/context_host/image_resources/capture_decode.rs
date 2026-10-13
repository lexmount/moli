use crate::{
    capture_resources::{CaptureResourceKind, CaptureResourceResult},
    page_resource_completion::CaptureResourceCompletion,
    types::PendingCaptureResource,
};

use super::{CaptureReadyImage, decode::ImageDecodeResult};

impl super::super::JsContextHost {
    pub(crate) fn queue_capture_resource_result(
        &self,
        capture: PendingCaptureResource,
        result: CaptureResourceResult,
    ) {
        if !capture.endpoint.is_active()
            || self.current_window_document_task_target_for_dispatch_scope(
                capture.target.dispatch_scope(),
            ) != Some(capture.target)
        {
            return;
        }
        let _ =
            self.resource_completion_sender()
                .send_capture_resource(CaptureResourceCompletion {
                    target: capture.target,
                    entry_id: capture.entry_id,
                    endpoint: capture.endpoint,
                    result,
                });
    }

    /// Called only after the normal subresource response security checks.
    /// The response belongs to this finite capture, never a live resource slot.
    pub(crate) fn complete_capture_resource_response(
        &self,
        capture: PendingCaptureResource,
        response: &crate::protocol_types::NavigationResponse,
        encoded: Option<&moli_parkable_image::ParkableImage>,
    ) {
        if !capture.endpoint.is_active()
            || self.current_window_document_task_target_for_dispatch_scope(
                capture.target.dispatch_scope(),
            ) != Some(capture.target)
        {
            return;
        }
        if !(200..=299).contains(&response.status) {
            self.queue_capture_resource_result(
                capture,
                CaptureResourceResult::Failed(format!(
                    "capture resource returned HTTP {}",
                    response.status
                )),
            );
            return;
        }
        let encoded_len = match capture.kind {
            CaptureResourceKind::Font => response.body_bytes().len(),
            CaptureResourceKind::Image => {
                let Some(encoded) = encoded else {
                    self.queue_capture_resource_result(
                        capture,
                        CaptureResourceResult::Failed(
                            "capture image response has no encoded body".to_owned(),
                        ),
                    );
                    return;
                };
                encoded.len()
            }
        };
        // This caps job acceptance and decode entry. The existing transport's
        // global response limit remains responsible for buffering before here.
        if let Err(error) =
            capture
                .endpoint
                .accept_encoded(capture.entry_id, capture.kind, encoded_len)
        {
            self.queue_capture_resource_result(capture, CaptureResourceResult::Failed(error));
            return;
        }
        match capture.kind {
            CaptureResourceKind::Font => self.queue_capture_resource_result(
                capture,
                CaptureResourceResult::FontBytes(response.clone_body_bytes().into()),
            ),
            CaptureResourceKind::Image => {
                let encoded = encoded.expect("capture image body was checked above");
                let Some(descriptor) =
                    crate::network_host::image_response_descriptor_from_parkable(response, encoded)
                else {
                    self.queue_capture_resource_result(
                        capture,
                        CaptureResourceResult::Failed(
                            "capture image response failed MIME or intrinsic metadata checks"
                                .to_owned(),
                        ),
                    );
                    return;
                };
                let Some(loader) =
                    self.document_resource_loader_for_window_owner(capture.target.owner())
                else {
                    self.queue_capture_resource_result(
                        capture,
                        CaptureResourceResult::Failed(
                            "capture image Document resource authority is unavailable".to_owned(),
                        ),
                    );
                    return;
                };
                let sender = self.resource_completion_sender();
                let completion_capture = capture.clone();
                let submission = self.image_resources.decode.submit_preload(
                    loader.task_runner(),
                    descriptor.decode_metadata,
                    encoded.clone(),
                    move |result| {
                        if !completion_capture.endpoint.is_active() {
                            return;
                        }
                        let result = match result {
                            ImageDecodeResult::Ready(decoded) => CaptureResourceResult::ImageReady(
                                CaptureReadyImage::new(descriptor, decoded),
                            ),
                            ImageDecodeResult::Failed(error) => {
                                CaptureResourceResult::Failed(error)
                            }
                        };
                        // The worker has no Host, DOM or live resource store.
                        // Sending a typed terminal supplies the ordinary Page wake.
                        let _ = sender.send_capture_resource(CaptureResourceCompletion {
                            target: completion_capture.target,
                            entry_id: completion_capture.entry_id,
                            endpoint: completion_capture.endpoint,
                            result,
                        });
                    },
                );
                if let Err(error) = submission {
                    self.queue_capture_resource_result(
                        capture,
                        CaptureResourceResult::Failed(format!(
                            "capture image decode queue rejected the resource: {error:?}"
                        )),
                    );
                }
            }
        }
    }
}
