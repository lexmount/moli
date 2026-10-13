//! Resources retained only by one explicit capture job. No entry is published
//! into live font/image registries or a canonical screen layout.

use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutViewport, WebFontRegistration, WebFontRegistrationError,
    WebFontRegistrationOutcome,
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{CaptureReadyImage, WindowDocumentTaskTarget},
    style_engine::StylesheetResourceSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CaptureLayoutIdentity {
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) document: DomHandle,
    pub(crate) root: DomHandle,
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureLayoutMember {
    pub(crate) identity: CaptureLayoutIdentity,
    pub(crate) viewport: LayoutViewport,
    pub(crate) fonts: StylesheetResourceSnapshot,
    pub(crate) available_font_slots: Vec<String>,
    pub(crate) available_fonts: CaptureAvailableFonts,
}

/// Already selected CSS font registrations with shared decoded byte Arcs.
/// This owns no Parley contexts and contains no unrelated programmatic fonts.
pub(crate) struct CaptureAvailableFonts(DocumentLayoutServices);

impl CaptureAvailableFonts {
    pub(crate) fn from_selected(services: &DocumentLayoutServices) -> Self {
        Self(
            DocumentLayoutServices::with_system_font_policy(services.system_font_policy())
                .clone_for_temporary_layout(|_| true, Some(services)),
        )
    }
}

impl Clone for CaptureAvailableFonts {
    fn clone(&self) -> Self {
        Self::from_selected(&self.0)
    }
}

impl std::fmt::Debug for CaptureAvailableFonts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureAvailableFonts")
            .field("slots", &self.0.web_font_count())
            .field("system_font_policy", &self.0.system_font_policy())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureLayoutCssImage {
    pub(crate) identity: CaptureLayoutIdentity,
    pub(crate) resolved_url: String,
    /// A live image can be reused only while this strong resource owner keeps
    /// its decoded-byte permit alive. The job inserts this into its overlay.
    pub(crate) ready: Option<CaptureReadyImage>,
}

#[derive(Default, Debug)]
pub(crate) struct CaptureLayoutDiscovery {
    pub(crate) members: Vec<CaptureLayoutMember>,
    pub(crate) css_images: Vec<CaptureLayoutCssImage>,
}

#[derive(Default)]
pub(crate) struct CaptureLayoutOverlay {
    // Newly decoded fonts are charged below. Reused snapshots only clone
    // existing byte Arcs and must not be double-counted as newly decoded bytes.
    fonts: HashMap<CaptureLayoutIdentity, DocumentLayoutServices>,
    reused_fonts: HashMap<CaptureLayoutIdentity, CaptureAvailableFonts>,
    images: HashMap<(CaptureLayoutIdentity, String), CaptureReadyImage>,
}

impl CaptureLayoutOverlay {
    /// Includes actual retained decoded SFNT bytes, including a distinct copy
    /// registered for another document. Shared live font bytes are not added.
    pub(crate) fn retained_font_bytes(&self) -> usize {
        self.fonts.values().fold(0usize, |total, fonts| {
            total.saturating_add(fonts.retained_web_font_bytes())
        })
    }

    pub(crate) fn register_font(
        &mut self,
        identity: CaptureLayoutIdentity,
        registration: WebFontRegistration,
        max_decoded_font_bytes: usize,
        max_total_decoded_font_bytes: usize,
    ) -> Result<WebFontRegistrationOutcome, WebFontRegistrationError> {
        let previous = self
            .fonts
            .get(&identity)
            .and_then(|fonts| fonts.web_font_decoded_bytes(registration.slot()))
            .unwrap_or_default();
        let other_bytes = self.retained_font_bytes().saturating_sub(previous);
        let limit =
            max_decoded_font_bytes.min(max_total_decoded_font_bytes.saturating_sub(other_bytes));
        self.fonts
            .entry(identity)
            .or_default()
            .register_web_font_with_decoded_limit(registration.with_stylesheet_scope(), limit)
    }

    pub(crate) fn insert_image(
        &mut self,
        identity: CaptureLayoutIdentity,
        resolved_url: String,
        ready: CaptureReadyImage,
    ) {
        self.images.insert((identity, resolved_url), ready);
    }

    pub(crate) fn reuse_available_fonts(
        &mut self,
        identity: CaptureLayoutIdentity,
        available: CaptureAvailableFonts,
    ) {
        self.reused_fonts.insert(identity, available);
    }

    pub(crate) fn font_services_for_document(
        &self,
        identity: CaptureLayoutIdentity,
        live: &DocumentLayoutServices,
        is_active: impl Fn(&str) -> bool,
    ) -> DocumentLayoutServices {
        if let Some(reused) = self.reused_fonts.get(&identity) {
            let selected = live.clone_for_temporary_layout(&is_active, Some(&reused.0));
            selected.clone_for_temporary_layout(is_active, self.fonts.get(&identity))
        } else {
            live.clone_for_temporary_layout(is_active, self.fonts.get(&identity))
        }
    }

    pub(crate) fn image(
        &self,
        identity: CaptureLayoutIdentity,
        resolved_url: &str,
    ) -> Option<&CaptureReadyImage> {
        self.images.get(&(identity, resolved_url.to_owned()))
    }
}

/// One synchronous adapter call. The overlay remains immutable throughout the
/// pass; all discovered inputs are owned before the owner is allowed to await.
pub(crate) struct CaptureLayoutPass<'a> {
    pub(crate) overlay: &'a CaptureLayoutOverlay,
    pub(crate) discovery: &'a mut CaptureLayoutDiscovery,
}
