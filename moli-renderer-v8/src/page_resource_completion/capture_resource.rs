use crate::{
    capture_resources::{CaptureResourceEndpoint, CaptureResourceResult},
    native_bridge::WindowDocumentTaskTarget,
};

/// An owned native terminal. A producer may enqueue it but only the Page
/// resource owner may deliver it after validating both root and local owner.
#[derive(Debug)]
pub(crate) struct CaptureResourceCompletion {
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) entry_id: usize,
    pub(crate) endpoint: CaptureResourceEndpoint,
    pub(crate) result: CaptureResourceResult,
}
