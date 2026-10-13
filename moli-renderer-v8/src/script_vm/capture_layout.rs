use std::collections::HashSet;

use moli_layout::{
    LayoutError, LayoutFlushReason, LayoutFrameCoverage, LayoutPassRequest, LayoutPassResult,
    LayoutViewport, PaintCaptureRequest, PaintSnapshot, PaintViewport,
};

use super::ScriptVm;
use crate::{
    capture_layout_resources::{
        CaptureLayoutCssImage, CaptureLayoutDiscovery, CaptureLayoutIdentity, CaptureLayoutOverlay,
        CaptureLayoutPass,
    },
    document_runtime::DomHandle,
    native_bridge::LayoutPublicationRequest,
    style_engine::StyloStyleEnvironment,
};

impl ScriptVm {
    pub(crate) fn current_capture_layout_identity(&self) -> Option<CaptureLayoutIdentity> {
        let host = self._context_host.borrow();
        host.capture_layout_identity_for_document(host.document_handle())
    }

    pub(crate) fn validate_capture_layout_identity(&self, identity: CaptureLayoutIdentity) -> bool {
        self._context_host
            .borrow()
            .capture_layout_identity_for_document(identity.document)
            == Some(identity)
    }

    pub(crate) fn discover_capture_layout_resources(
        &mut self,
        expected: CaptureLayoutIdentity,
        viewport: LayoutViewport,
        environment: Option<StyloStyleEnvironment>,
        overlay: &CaptureLayoutOverlay,
    ) -> Result<CaptureLayoutDiscovery, LayoutError> {
        let request = LayoutPassRequest::new(viewport, LayoutFlushReason::Print)
            .with_frame_coverage(LayoutFrameCoverage::AllLayoutBoxes);
        self.with_capture_layout_pass(expected, request, environment, overlay, |_| Ok(()))?
            .map(|(_, discovery)| discovery)
            .ok_or(LayoutError::NoLayoutRoot)
    }

    pub(crate) fn paint_capture_layout_with_resources(
        &mut self,
        expected: CaptureLayoutIdentity,
        viewport: PaintViewport,
        reason: LayoutFlushReason,
        capture: PaintCaptureRequest,
        environment: Option<StyloStyleEnvironment>,
        overlay: &CaptureLayoutOverlay,
    ) -> Result<Option<(PaintSnapshot, CaptureLayoutDiscovery)>, LayoutError> {
        self.with_capture_layout_pass(
            expected,
            LayoutPassRequest::with_capture(viewport, reason, capture)
                .with_frame_coverage(LayoutFrameCoverage::AllLayoutBoxes),
            environment,
            overlay,
            LayoutPassResult::take_paint_snapshot,
        )
    }

    fn with_capture_layout_pass<T>(
        &mut self,
        expected: CaptureLayoutIdentity,
        request: LayoutPassRequest,
        environment: Option<StyloStyleEnvironment>,
        overlay: &CaptureLayoutOverlay,
        consume: impl FnOnce(&mut LayoutPassResult<DomHandle>) -> Result<T, LayoutError>,
    ) -> Result<Option<(T, CaptureLayoutDiscovery)>, LayoutError> {
        // The normal entry point reconciles live CSS fonts and starts live CSS
        // image requests. Capture explicitly uses this synchronous transient
        // route instead. The shared native style observation still drains
        // committed style invalidations before reading the temporary world.
        let host = self._context_host.borrow();
        if host.capture_layout_identity_for_document(expected.document) != Some(expected) {
            return Err(LayoutError::source_contract(
                "capture owner",
                "capture Document or root was replaced",
            ));
        }
        let publication = host.prepare_layout_publication(
            expected.document,
            request.viewport,
            LayoutPublicationRequest::Transient,
        )?;
        let mut discovery = CaptureLayoutDiscovery::default();
        let Some(mut pass) = host.build_capture_layout_pass_in_environment(
            expected,
            request,
            environment,
            CaptureLayoutPass {
                overlay,
                discovery: &mut discovery,
            },
        )?
        else {
            return Ok(None);
        };
        let mut seen = HashSet::new();
        for reference in pass.css_image_references() {
            let document = host
                .layout_document_for_source(reference.source)
                .ok_or_else(|| {
                    LayoutError::source_contract(
                        "capture image",
                        "CSS image source has no document",
                    )
                })?;
            let identity = discovery
                .members
                .iter()
                .find(|member| member.identity.document == document)
                .map(|member| member.identity)
                .ok_or_else(|| {
                    LayoutError::source_contract(
                        "capture image",
                        "CSS image source was not a capture member",
                    )
                })?;
            if !seen.insert((identity, reference.resolved_url.clone())) {
                continue;
            }
            let ready = overlay
                .image(identity, &reference.resolved_url)
                .cloned()
                .or_else(|| host.ready_css_image_for_capture(document, &reference.resolved_url));
            discovery.css_images.push(CaptureLayoutCssImage {
                identity,
                resolved_url: reference.resolved_url.clone(),
                ready,
            });
        }
        // Nothing in this pass may execute author code. Still validate every
        // discovered member before handing owned resource inputs to an await.
        if discovery.members.iter().any(|member| {
            host.capture_layout_identity_for_document(member.identity.document)
                != Some(member.identity)
        }) {
            return Err(LayoutError::source_contract(
                "capture owner",
                "capture member was retired",
            ));
        }
        let value = consume(&mut pass)?;
        host.finish_layout_publication(publication, pass)?;
        Ok(Some((value, discovery)))
    }
}
