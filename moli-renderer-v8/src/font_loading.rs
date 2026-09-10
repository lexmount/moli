//! Native FontFace loading shared by CSS resources and JS wrappers.
//!
//! This layer owns source fallback, decoded bytes and registration. It does not
//! own V8 handles, promises, network tasks or document lifetimes: callers retain
//! the existing resource owner's request identity and publish changes to JS.

use std::{cell::RefCell, rc::Rc};

use moli_css_parse::CssFontSource;
use moli_layout::{
    DocumentLayoutServices, FontFaceData, WebFontFace, WebFontRegistration,
    WebFontRegistrationError, WebFontRegistrationOutcome,
};

pub(crate) type FontFaceLoad = Rc<RefCell<FontFaceResource>>;

pub(crate) fn script_font_descriptor(
    family: &str,
    weight: &str,
    stretch: &str,
    style: &str,
) -> WebFontFace {
    use crate::css_resource_urls::{
        parse_font_stretch_lower_bound, parse_font_style_lower_bound, parse_font_weight_lower_bound,
    };
    WebFontFace::new(moli_css_parse::unquote_css_string(family))
        .with_weight(parse_font_weight_lower_bound(weight).unwrap_or(400.0))
        .with_stretch(parse_font_stretch_lower_bound(stretch).unwrap_or(100.0))
        .with_style(
            parse_font_style_lower_bound(style).unwrap_or(moli_layout::WebFontStyle::Normal),
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FontFaceStatus {
    Unloaded,
    Loading,
    Loaded,
    Error,
}

impl FontFaceStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unloaded => "unloaded",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FontFaceError {
    pub(crate) name: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Debug)]
enum LoadState {
    Unloaded,
    Loading,
    Loaded(FontFaceData),
    Error(FontFaceError),
}

#[derive(Debug)]
pub(crate) struct FontFaceResource {
    slot: String,
    descriptor: WebFontFace,
    sources: std::vec::IntoIter<CssFontSource>,
    state: LoadState,
    started: bool,
}

impl FontFaceResource {
    pub(crate) fn new(
        slot: String,
        descriptor: WebFontFace,
        sources: Vec<CssFontSource>,
    ) -> FontFaceLoad {
        Rc::new(RefCell::new(Self {
            slot,
            descriptor,
            sources: sources.into_iter(),
            state: LoadState::Unloaded,
            started: false,
        }))
    }

    pub(crate) fn slot(&self) -> &str {
        &self.slot
    }

    pub(crate) fn set_descriptor(&mut self, descriptor: WebFontFace) {
        self.descriptor = descriptor.with_unicode_ranges(self.descriptor.unicode_ranges().to_vec());
    }

    pub(crate) fn status(&self) -> FontFaceStatus {
        match self.state {
            LoadState::Unloaded => FontFaceStatus::Unloaded,
            LoadState::Loading => FontFaceStatus::Loading,
            LoadState::Loaded(_) => FontFaceStatus::Loaded,
            LoadState::Error(_) => FontFaceStatus::Error,
        }
    }

    pub(crate) fn error(&self) -> Option<FontFaceError> {
        match self.state {
            LoadState::Error(error) => Some(error),
            _ => None,
        }
    }

    pub(crate) fn was_started(&self) -> bool {
        self.started
    }

    pub(crate) fn fail(&mut self, name: &'static str, message: &'static str) {
        self.state = LoadState::Error(FontFaceError { name, message });
    }

    pub(crate) fn initialize_binary(&mut self, bytes: &[u8]) {
        match FontFaceData::from_bytes(bytes) {
            Ok(data) => self.state = LoadState::Loaded(data),
            Err(_) => self.fail("SyntaxError", "Invalid font data in ArrayBuffer."),
        }
    }

    /// Only one consumer may initiate loading, even when CSS and JS both ask.
    pub(crate) fn begin(&mut self) -> bool {
        if self.status() != FontFaceStatus::Unloaded {
            return false;
        }
        self.state = LoadState::Loading;
        self.started = true;
        true
    }

    /// Advance after starting or after a failed URL. Local lookup and source
    /// exhaustion are terminal here; only actual URL requests leave this layer.
    pub(crate) fn next_url(&mut self, services: &mut DocumentLayoutServices) -> Option<String> {
        if self.status() != FontFaceStatus::Loading {
            return None;
        }
        for source in self.sources.by_ref() {
            match source {
                CssFontSource::Local(name) => {
                    if let Some(data) = services.local_font_source(&name) {
                        self.state = LoadState::Loaded(data);
                        return None;
                    }
                }
                CssFontSource::Url(url) => return Some(url),
            }
        }
        self.fail(
            "NetworkError",
            "A network error occurred while loading the font.",
        );
        None
    }

    /// A bad response leaves the resource loading so its source list can fall
    /// back. Stale/canceled document requests must be rejected by the owner
    /// before this method is called.
    pub(crate) fn accept_response(
        &mut self,
        bytes: Option<&[u8]>,
    ) -> Result<(), WebFontRegistrationError> {
        if let Some(bytes) = bytes {
            self.state = LoadState::Loaded(FontFaceData::from_bytes(bytes)?);
        }
        Ok(())
    }

    pub(crate) fn register(
        &self,
        services: &mut DocumentLayoutServices,
    ) -> Result<Option<WebFontRegistrationOutcome>, WebFontRegistrationError> {
        let LoadState::Loaded(data) = &self.state else {
            return Ok(None);
        };
        services
            .register_web_font(WebFontRegistration::new(
                self.slot.clone(),
                self.descriptor.clone(),
                data.bytes().to_vec(),
            ))
            .map(Some)
    }

    pub(crate) fn unregister(&self, services: &mut DocumentLayoutServices) {
        services.remove_web_font(&self.slot);
    }
}
