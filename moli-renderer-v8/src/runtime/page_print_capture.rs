use std::time::Instant;

use anyhow::{Context, Result, anyhow, ensure};
use moli_layout::{LayoutFlushReason, LayoutViewport, WebFontRegistrationError};

use crate::{
    capture_layout_resources::CaptureLayoutDiscovery,
    capture_resources::{
        CaptureResourceEntry, CaptureResourceFetch, CaptureResourceKind, CaptureResourceResult,
        CaptureResourceSpec, CaptureResourceState, MAX_CAPTURE_DISCOVERY_WAVES,
        MAX_CAPTURE_FONT_DECODED_BYTES, MAX_CAPTURE_RESOURCE_CONCURRENCY,
        MAX_CAPTURE_RESOURCE_ENTRIES, MAX_CAPTURE_TOTAL_FONT_DECODED_BYTES, PrintCaptureAdvance,
        PrintCaptureContext, PrintCaptureJob,
    },
    style_engine::StyloStyleEnvironment,
};

use super::{PageVm, RendererCaptureScreenshotReply, RendererScreenshotPurpose};

impl PageVm {
    /// One synchronous step. Pending network/decode work is always returned to
    /// the existing Page owner, which parks this command without borrowing us.
    pub(super) fn advance_print_capture(
        &mut self,
        job: &mut PrintCaptureJob,
    ) -> Result<PrintCaptureAdvance> {
        ensure!(
            Instant::now() < job.deadline(),
            "print resource preparation timed out"
        );
        if !self.layout_configuration.policy.uses_real_layout() {
            return Ok(PrintCaptureAdvance::Complete(
                RendererCaptureScreenshotReply::LayoutDisabled,
            ));
        }

        if job.context.is_none() {
            self.flush_page_action_window(moli_action_window::ActionBarrier::Explicit)?;
            let Some(context) = self.current_print_capture_context(job)? else {
                return Ok(PrintCaptureAdvance::Complete(
                    RendererCaptureScreenshotReply::NoDocument,
                ));
            };
            // Validate clip/output options before admitting any network work.
            job.request.paint_capture_request(context.surface)?;
            job.members.push(context.identity);
            job.context = Some(context);
        }
        self.validate_print_capture_context(job)?;
        self.apply_print_resource_completions(job)?;

        loop {
            self.start_queued_print_resources(job)?;
            if job.entries.iter().any(|entry| {
                matches!(
                    entry.state,
                    CaptureResourceState::Queued | CaptureResourceState::Pending
                )
            }) {
                return Ok(PrintCaptureAdvance::Waiting);
            }

            // The barrier can dispatch page input. Revalidate afterwards,
            // before accepting any fresh manifest or final capture geometry.
            self.flush_page_action_window(moli_action_window::ActionBarrier::Explicit)?;
            self.validate_print_capture_context(job)?;
            let context = job
                .context
                .as_ref()
                .expect("initialized print context")
                .clone();
            let discovery = self.vm_mut().discover_capture_layout_resources(
                context.identity,
                context.viewport,
                context.environment_override,
                &job.overlay,
            )?;
            let added = self.admit_print_resource_discovery(job, discovery)?;
            if added {
                ensure!(
                    job.discovery_waves < MAX_CAPTURE_DISCOVERY_WAVES,
                    "print resource manifest exceeded its discovery limit"
                );
                job.discovery_waves += 1;
                continue;
            }

            ensure!(
                Instant::now() < job.deadline(),
                "print resource preparation timed out"
            );
            self.validate_print_capture_context(job)?;
            let profile_started = moli_trace::cpu_profile_enabled().then(Instant::now);
            let capture = job.request.paint_capture_request(context.surface)?;
            let Some((snapshot, actual)) = self.vm_mut().paint_capture_layout_with_resources(
                context.identity,
                context.viewport,
                LayoutFlushReason::Print,
                capture,
                context.environment_override,
                &job.overlay,
            )?
            else {
                return Ok(PrintCaptureAdvance::Complete(
                    RendererCaptureScreenshotReply::NoDocument,
                ));
            };
            let actual_viewports = actual
                .members
                .iter()
                .map(|member| (member.identity, member.viewport))
                .collect::<Vec<_>>();
            ensure!(
                actual_viewports == job.current_member_viewports,
                "print member viewports changed between discovery and final layout"
            );
            // A final pass is not allowed to encode dependencies that were
            // never prepared. No callback or await separates this check from
            // raster/encode; newly revealed inputs consume a remaining wave.
            if self.admit_print_resource_discovery(job, actual)? {
                ensure!(
                    job.discovery_waves < MAX_CAPTURE_DISCOVERY_WAVES,
                    "final print resource manifest exceeded its discovery limit"
                );
                job.discovery_waves += 1;
                drop(snapshot);
                continue;
            }
            self.validate_print_capture_context(job)?;
            let layout_us = profile_started
                .map(|started| started.elapsed().as_micros())
                .unwrap_or_default();
            let image = Self::encode_capture_snapshot(
                &snapshot,
                job.request.format,
                job.request.quality,
                job.request.optimize_for_speed,
                LayoutFlushReason::Print,
                layout_us,
                profile_started,
            )?;
            return Ok(PrintCaptureAdvance::Complete(
                RendererCaptureScreenshotReply::Captured(image),
            ));
        }
    }

    fn current_print_capture_context(
        &self,
        job: &PrintCaptureJob,
    ) -> Result<Option<PrintCaptureContext>> {
        ensure!(
            matches!(job.request.purpose, RendererScreenshotPurpose::Print { .. }),
            "print preparation requires a Print capture"
        );
        let Some(identity) = self.vm().current_capture_layout_identity() else {
            return Ok(None);
        };
        let surface = self.document_settings.viewport_surface.unwrap_or_default();
        Ok(Some(PrintCaptureContext {
            page_vm_creation_id: self.creation_id,
            document: self.document_lifecycle.identity(),
            identity,
            viewport: LayoutViewport::new(
                surface.inner_width,
                surface.inner_height,
                surface.device_pixel_ratio as f32,
            ),
            surface,
            live_environment: StyloStyleEnvironment::from_emulated_media(
                &self.document_settings.emulated_media,
            ),
            environment_override: self.capture_style_environment_override(job.request.purpose),
        }))
    }

    fn validate_print_capture_context(&self, job: &PrintCaptureJob) -> Result<()> {
        let expected = job.context.as_ref().expect("initialized print context");
        ensure!(
            self.current_print_capture_context(job)?.as_ref() == Some(expected),
            "print capture Document or environment changed during preparation"
        );
        ensure!(
            job.members
                .iter()
                .all(|identity| self.vm().validate_capture_layout_identity(*identity)),
            "print capture member Document retired during preparation"
        );
        Ok(())
    }

    fn apply_print_resource_completions(&mut self, job: &mut PrintCaptureJob) -> Result<()> {
        for (entry_id, result) in job.take_completions()? {
            let entry = job
                .entries
                .get_mut(entry_id)
                .ok_or_else(|| anyhow!("print resource completion has an unknown entry"))?;
            if !matches!(entry.state, CaptureResourceState::Pending) {
                continue;
            }
            entry.state = match result {
                CaptureResourceResult::FontBytes(bytes) => {
                    ensure!(
                        entry.spec.kind == CaptureResourceKind::Font,
                        "print resource kind mismatch"
                    );
                    let font = entry
                        .spec
                        .font
                        .as_ref()
                        .context("print font declaration is missing")?;
                    match job.overlay.register_font(
                        entry.spec.identity,
                        font.registration(bytes.to_vec()),
                        MAX_CAPTURE_FONT_DECODED_BYTES,
                        MAX_CAPTURE_TOTAL_FONT_DECODED_BYTES,
                    ) {
                        Ok(_) => CaptureResourceState::Ready,
                        Err(error @ WebFontRegistrationError::DecodedByteLimit { .. }) => {
                            return Err(error.into());
                        }
                        Err(error) => {
                            tracing::debug!(?error, url = %entry.spec.url, "print font preparation failed");
                            CaptureResourceState::Failed
                        }
                    }
                }
                CaptureResourceResult::ImageReady(image) => {
                    ensure!(
                        entry.spec.kind == CaptureResourceKind::Image,
                        "print resource kind mismatch"
                    );
                    job.overlay.insert_image(
                        entry.spec.identity,
                        entry.spec.url.to_string(),
                        image,
                    );
                    CaptureResourceState::Ready
                }
                CaptureResourceResult::Failed(error) => {
                    tracing::debug!(%error, url = %entry.spec.url, "print resource preparation failed");
                    CaptureResourceState::Failed
                }
            };
        }
        Ok(())
    }

    /// Failed and disabled entries are terminal for this job. A fresh layout
    /// may still reuse data independently made ready by the live Document.
    fn admit_print_resource_discovery(
        &mut self,
        job: &mut PrintCaptureJob,
        discovery: CaptureLayoutDiscovery,
    ) -> Result<bool> {
        let mut added = false;
        job.current_member_viewports = discovery
            .members
            .iter()
            .map(|member| (member.identity, member.viewport))
            .collect();
        for member in discovery.members {
            ensure!(
                self.vm().validate_capture_layout_identity(member.identity),
                "print member owner changed"
            );
            if !job.members.contains(&member.identity) {
                job.members.push(member.identity);
            }
            job.overlay
                .reuse_available_fonts(member.identity, member.available_fonts);
            for resource in member.fonts.web_fonts() {
                let Some(font) = resource.web_font() else {
                    continue;
                };
                let available = member
                    .available_font_slots
                    .iter()
                    .any(|slot| slot == font.slot());
                let spec = CaptureResourceSpec {
                    identity: member.identity,
                    kind: CaptureResourceKind::Font,
                    url: resource.request_url().clone(),
                    font: Some(font.clone()),
                };
                added |= admit_print_resource_entry(job, spec, available)?;
            }
        }
        for image in discovery.css_images {
            let url = url::Url::parse(&image.resolved_url)?;
            let available = image.ready.is_some();
            if let Some(ready) = image.ready {
                job.overlay
                    .insert_image(image.identity, image.resolved_url, ready);
            }
            let spec = CaptureResourceSpec {
                identity: image.identity,
                kind: CaptureResourceKind::Image,
                url,
                font: None,
            };
            added |= admit_print_resource_entry(job, spec, available)?;
        }
        Ok(added)
    }

    fn start_queued_print_resources(&mut self, job: &mut PrintCaptureJob) -> Result<()> {
        let mut pending = job
            .entries
            .iter()
            .filter(|entry| matches!(entry.state, CaptureResourceState::Pending))
            .count();
        let endpoint = job.endpoint();
        for (entry_id, entry) in job.entries.iter_mut().enumerate() {
            if pending == MAX_CAPTURE_RESOURCE_CONCURRENCY {
                break;
            }
            if !matches!(entry.state, CaptureResourceState::Queued) {
                continue;
            }
            let identity = entry.spec.identity;
            ensure!(
                self.vm().validate_capture_layout_identity(identity),
                "print request owner retired"
            );
            match self.vm_mut().start_capture_resource(
                identity.document,
                identity.target,
                entry.spec.kind,
                entry.spec.url.clone(),
                entry_id,
                endpoint.clone(),
            ) {
                Ok(Some(fetch_id)) => {
                    job.fetches.push(CaptureResourceFetch {
                        internal_id: fetch_id,
                        target: identity.target,
                        entry_id,
                        kind: entry.spec.kind,
                    });
                    entry.state = CaptureResourceState::Pending;
                    pending += 1;
                }
                Ok(None) => entry.state = CaptureResourceState::Disabled,
                Err(error) => return Err(error).context("could not admit a print resource"),
            }
        }
        Ok(())
    }

    /// Cleanup follows the original Host residence, not its current Document.
    /// document.open() transfers active loads within that same Host. A new
    /// PageVm must never receive the old job's local numeric request ids.
    pub(super) fn cancel_print_capture(&mut self, job: &mut PrintCaptureJob) {
        job.revoke();
        if job
            .context
            .as_ref()
            .is_some_and(|context| context.page_vm_creation_id == self.creation_id)
        {
            let endpoint = job.endpoint();
            self.vm_mut()
                .cancel_capture_resource_fetches(&job.fetches, &endpoint);
        }
        job.fetches.clear();
        job.overlay = Default::default();
    }
}

fn admit_print_resource_entry(
    job: &mut PrintCaptureJob,
    spec: CaptureResourceSpec,
    available: bool,
) -> Result<bool> {
    if let Some(entry) = job
        .entries
        .iter_mut()
        .find(|entry| entry.spec.has_same_key(&spec))
    {
        if available {
            entry.state = CaptureResourceState::Ready;
        }
        return Ok(false);
    }
    ensure!(
        job.entries.len() < MAX_CAPTURE_RESOURCE_ENTRIES,
        "print resource manifest entry limit exceeded"
    );
    job.entries.push(CaptureResourceEntry {
        spec,
        state: if available {
            CaptureResourceState::Ready
        } else {
            CaptureResourceState::Queued
        },
    });
    Ok(true)
}
