//! Resources owned by one finite, asynchronous print capture.
//!
//! Producers retain a weak mailbox address. They cannot keep a cancelled
//! capture, its temporary fonts, or decoded image permits alive.

use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::{
    capture_layout_resources::{CaptureLayoutIdentity, CaptureLayoutOverlay},
    css_resource_urls::StylesheetWebFont,
    native_bridge::{CaptureReadyImage, WindowDocumentTaskTarget},
    runtime::{
        RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest,
        RendererDocumentLifecycleIdentity, RendererPageReply, RendererPrintToPdfReply,
        RendererPrintToPdfRequest, RendererScreenshotPurpose,
    },
    style_engine::StyloStyleEnvironment,
};

pub(crate) const MAX_CAPTURE_FONT_ENCODED_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_CAPTURE_IMAGE_ENCODED_BYTES: usize = 16 * 1024 * 1024;
const MAX_CAPTURE_ENCODED_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_CAPTURE_FONT_DECODED_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_CAPTURE_TOTAL_FONT_DECODED_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_CAPTURE_RESOURCE_ENTRIES: usize = 64;
pub(crate) const MAX_CAPTURE_RESOURCE_CONCURRENCY: usize = 4;
pub(crate) const MAX_CAPTURE_DISCOVERY_WAVES: usize = 2;
const PRINT_RESOURCE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureResourceKind {
    Font,
    Image,
}

#[derive(Debug)]
pub(crate) enum CaptureResourceResult {
    FontBytes(Arc<[u8]>),
    ImageReady(CaptureReadyImage),
    Failed(String),
}

#[derive(Default, Debug)]
pub(crate) struct CaptureResourceMailbox {
    closed: bool,
    encoded_entries: HashMap<usize, usize>,
    encoded_bytes: usize,
    limit_error: Option<String>,
    completions: Vec<(usize, CaptureResourceResult)>,
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureResourceEndpoint(Weak<Mutex<CaptureResourceMailbox>>);

impl CaptureResourceEndpoint {
    /// Cancellation compares the job capability even after revoke. A closed
    /// endpoint must not lose the identity needed to cancel its own transport.
    pub(crate) fn same_capture(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.0
            .upgrade()
            .is_some_and(|mailbox| !mailbox.lock().closed)
    }

    /// Account bytes before retaining a font response or admitting an image
    /// decode. Transport buffering has its own existing network limits.
    pub(crate) fn accept_encoded(
        &self,
        entry_id: usize,
        kind: CaptureResourceKind,
        len: usize,
    ) -> Result<(), String> {
        let Some(mailbox) = self.0.upgrade() else {
            return Err("print capture was cancelled".to_owned());
        };
        let mut mailbox = mailbox.lock();
        if mailbox.closed {
            return Err("print capture was cancelled".to_owned());
        }
        if let Some(previous) = mailbox.encoded_entries.get(&entry_id) {
            return if *previous == len {
                Ok(())
            } else {
                Err("print resource delivered inconsistent byte counts".to_owned())
            };
        }
        let per_resource_limit = match kind {
            CaptureResourceKind::Font => MAX_CAPTURE_FONT_ENCODED_BYTES,
            CaptureResourceKind::Image => MAX_CAPTURE_IMAGE_ENCODED_BYTES,
        };
        let total = mailbox.encoded_bytes.checked_add(len);
        if len > per_resource_limit || total.is_none_or(|total| total > MAX_CAPTURE_ENCODED_BYTES) {
            let error = "print capture resource byte limit exceeded".to_owned();
            mailbox.limit_error = Some(error.clone());
            return Err(error);
        }
        mailbox.encoded_entries.insert(entry_id, len);
        mailbox.encoded_bytes = total.expect("checked capture byte total");
        Ok(())
    }

    /// Called only after the Page resource terminal validates its exact owner.
    pub(crate) fn deliver(&self, entry_id: usize, result: CaptureResourceResult) -> bool {
        let Some(mailbox) = self.0.upgrade() else {
            return false;
        };
        let mut mailbox = mailbox.lock();
        if mailbox.closed {
            return false;
        }
        mailbox.completions.push((entry_id, result));
        true
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureResourceSpec {
    pub(crate) identity: CaptureLayoutIdentity,
    pub(crate) kind: CaptureResourceKind,
    pub(crate) url: url::Url,
    pub(crate) font: Option<StylesheetWebFont>,
}

impl CaptureResourceSpec {
    pub(crate) fn has_same_key(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.kind == other.kind
            && self.url == other.url
            && self.font.as_ref().map(StylesheetWebFont::registration_slot)
                == other
                    .font
                    .as_ref()
                    .map(StylesheetWebFont::registration_slot)
    }
}

pub(crate) enum CaptureResourceState {
    Queued,
    Pending,
    Ready,
    Failed,
    Disabled,
}

pub(crate) struct CaptureResourceEntry {
    pub(crate) spec: CaptureResourceSpec,
    pub(crate) state: CaptureResourceState,
}

/// The precise capture continuation bound to one accepted transport request.
/// Numeric fetch ids alone are local to a Host and are not cancellation authority.
pub(crate) struct CaptureResourceFetch {
    pub(crate) internal_id: u64,
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) entry_id: usize,
    pub(crate) kind: CaptureResourceKind,
}

#[derive(Clone, PartialEq)]
pub(crate) struct PrintCaptureContext {
    // Unlike Document identity, this residence survives document.open().
    pub(crate) page_vm_creation_id: u64,
    pub(crate) document: RendererDocumentLifecycleIdentity,
    pub(crate) identity: CaptureLayoutIdentity,
    pub(crate) viewport: moli_layout::LayoutViewport,
    pub(crate) surface: moli_page_types::ViewportSurface,
    pub(crate) live_environment: StyloStyleEnvironment,
    pub(crate) environment_override: Option<StyloStyleEnvironment>,
}

pub(crate) enum PrintCaptureRequest {
    Raster(RendererCaptureScreenshotRequest),
    Pdf(RendererPrintToPdfRequest),
}

pub(crate) enum PrintCaptureReply {
    Raster(RendererCaptureScreenshotReply),
    Pdf(RendererPrintToPdfReply),
}

impl From<PrintCaptureReply> for RendererPageReply {
    fn from(reply: PrintCaptureReply) -> Self {
        match reply {
            PrintCaptureReply::Raster(reply) => Self::CaptureScreenshot(reply),
            PrintCaptureReply::Pdf(reply) => Self::PrintToPdf(reply),
        }
    }
}

impl From<RendererCaptureScreenshotRequest> for PrintCaptureRequest {
    fn from(request: RendererCaptureScreenshotRequest) -> Self {
        Self::Raster(request)
    }
}

impl From<RendererPrintToPdfRequest> for PrintCaptureRequest {
    fn from(request: RendererPrintToPdfRequest) -> Self {
        Self::Pdf(request)
    }
}

impl PrintCaptureRequest {
    pub(crate) fn purpose(&self) -> RendererScreenshotPurpose {
        match self {
            Self::Raster(request) => request.purpose,
            Self::Pdf(request) => RendererScreenshotPurpose::Print {
                print_background: request.print_background,
            },
        }
    }

    pub(crate) fn paint_capture_request(
        &self,
        surface: moli_page_types::ViewportSurface,
    ) -> anyhow::Result<moli_layout::PaintCaptureRequest> {
        match self {
            Self::Raster(request) => request.paint_capture_request(surface),
            Self::Pdf(request) => Ok(moli_layout::PaintCaptureRequest {
                include_backgrounds: request.print_background,
                ..moli_layout::PaintCaptureRequest::print_document()
            }),
        }
    }

    pub(crate) fn layout_disabled_reply(&self) -> PrintCaptureReply {
        match self {
            Self::Raster(_) => {
                PrintCaptureReply::Raster(RendererCaptureScreenshotReply::LayoutDisabled)
            }
            Self::Pdf(_) => PrintCaptureReply::Pdf(RendererPrintToPdfReply::LayoutDisabled),
        }
    }

    pub(crate) fn no_document_reply(&self) -> PrintCaptureReply {
        match self {
            Self::Raster(_) => {
                PrintCaptureReply::Raster(RendererCaptureScreenshotReply::NoDocument)
            }
            Self::Pdf(_) => PrintCaptureReply::Pdf(RendererPrintToPdfReply::NoDocument),
        }
    }
}

pub(crate) struct PrintCaptureJob {
    pub(crate) request: PrintCaptureRequest,
    deadline: Instant,
    pub(crate) context: Option<PrintCaptureContext>,
    pub(crate) members: Vec<CaptureLayoutIdentity>,
    pub(crate) current_member_viewports: Vec<(CaptureLayoutIdentity, moli_layout::LayoutViewport)>,
    pub(crate) entries: Vec<CaptureResourceEntry>,
    pub(crate) fetches: Vec<CaptureResourceFetch>,
    pub(crate) overlay: CaptureLayoutOverlay,
    pub(crate) discovery_waves: usize,
    pub(crate) mailbox: Arc<Mutex<CaptureResourceMailbox>>,
}

impl PrintCaptureJob {
    pub(crate) fn new(request: impl Into<PrintCaptureRequest>) -> anyhow::Result<Self> {
        let request = request.into();
        anyhow::ensure!(
            matches!(request.purpose(), RendererScreenshotPurpose::Print { .. }),
            "print preparation requires a Print capture"
        );
        let deadline = Instant::now()
            .checked_add(PRINT_RESOURCE_TIMEOUT)
            .ok_or_else(|| anyhow::anyhow!("print capture deadline overflow"))?;
        Ok(Self {
            request,
            deadline,
            context: None,
            members: Vec::new(),
            current_member_viewports: Vec::new(),
            entries: Vec::new(),
            fetches: Vec::new(),
            overlay: CaptureLayoutOverlay::default(),
            discovery_waves: 0,
            mailbox: Arc::new(Mutex::new(CaptureResourceMailbox::default())),
        })
    }

    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn endpoint(&self) -> CaptureResourceEndpoint {
        CaptureResourceEndpoint(Arc::downgrade(&self.mailbox))
    }

    pub(crate) fn take_completions(&self) -> anyhow::Result<Vec<(usize, CaptureResourceResult)>> {
        let mut mailbox = self.mailbox.lock();
        if let Some(error) = &mailbox.limit_error {
            anyhow::bail!("{error}");
        }
        Ok(std::mem::take(&mut mailbox.completions))
    }

    pub(crate) fn revoke(&mut self) {
        let mut mailbox = self.mailbox.lock();
        mailbox.closed = true;
        mailbox.completions.clear();
    }
}

impl Drop for PrintCaptureJob {
    fn drop(&mut self) {
        self.revoke();
    }
}

pub(crate) enum PrintCaptureAdvance {
    Waiting,
    Complete(PrintCaptureReply),
}

#[cfg(test)]
mod tests;
