//! Native FontFace loading shared by CSS resources and JS wrappers.
//!
//! This layer owns source fallback, decoded bytes and registration. It does not
//! own V8 handles, promises, network tasks or document lifetimes: callers retain
//! the existing resource owner's request identity and publish changes to JS.

use std::{cell::RefCell, rc::Rc, sync::Arc};

use moli_css_parse::CssFontSource;
use moli_layout::{
    DocumentLayoutServices, FontFaceData, WebFontFace, WebFontRegistration,
    WebFontRegistrationError, WebFontRegistrationOutcome, WebFontRegistrationScope,
};

pub(crate) mod demand;
pub(crate) mod usage;

pub(crate) type FontFaceLoad = Rc<RefCell<FontFaceResource>>;
pub(crate) type FontSourceLoad = Rc<RefCell<FontSourceResource>>;

pub(crate) fn script_font_descriptor(
    family: &str,
    weight: &str,
    stretch: &str,
    style: &str,
    unicode_range: &str,
) -> WebFontFace {
    WebFontFace::new(moli_css_parse::unquote_css_string(family))
        .with_css_descriptors(weight, stretch, style)
        .with_unicode_ranges(
            crate::css_resource_urls::parse_font_unicode_ranges(unicode_range).unwrap_or_default(),
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
pub(crate) struct FontSourceResource {
    source_key: Option<String>,
    sources: std::vec::IntoIter<CssFontSource>,
    state: LoadState,
}

impl FontSourceResource {
    pub(crate) fn new(sources: Vec<CssFontSource>, source_key: Option<String>) -> FontSourceLoad {
        Rc::new(RefCell::new(Self {
            sources: sources.into_iter(),
            source_key,
            state: LoadState::Unloaded,
        }))
    }
}

/// One independent FontFace declaration. The source operation and immutable
/// font data may be shared; descriptors, activation and registration identity
/// belong to this face. Document membership is supplied at registration time.
#[derive(Debug)]
pub(crate) struct FontFaceResource {
    slot: String,
    descriptor: Arc<WebFontFace>,
    source: FontSourceLoad,
    activated: bool,
    started: bool,
}

impl FontFaceResource {
    pub(crate) fn new(
        slot: String,
        descriptor: WebFontFace,
        sources: Vec<CssFontSource>,
    ) -> FontFaceLoad {
        Self::with_source(
            slot,
            Arc::new(descriptor),
            FontSourceResource::new(sources, None),
        )
    }

    pub(crate) fn with_source(
        slot: String,
        descriptor: Arc<WebFontFace>,
        source: FontSourceLoad,
    ) -> FontFaceLoad {
        Rc::new(RefCell::new(Self {
            slot,
            descriptor,
            source,
            activated: false,
            started: false,
        }))
    }

    pub(crate) fn slot(&self) -> &str {
        &self.slot
    }

    pub(crate) fn set_descriptor(&mut self, descriptor: WebFontFace) {
        if self.descriptor().as_ref() != &descriptor {
            self.descriptor = Arc::new(descriptor);
        }
    }

    pub(crate) fn descriptor(&self) -> &Arc<WebFontFace> {
        &self.descriptor
    }

    pub(crate) fn shares_source(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.source, &other.source)
    }

    pub(crate) fn status(&self) -> FontFaceStatus {
        if !self.activated {
            return FontFaceStatus::Unloaded;
        }
        match self.source.borrow().state {
            LoadState::Unloaded => FontFaceStatus::Unloaded,
            LoadState::Loading => FontFaceStatus::Loading,
            LoadState::Loaded(_) => FontFaceStatus::Loaded,
            LoadState::Error(_) => FontFaceStatus::Error,
        }
    }

    pub(crate) fn error(&self) -> Option<FontFaceError> {
        if !self.activated {
            return None;
        }
        match self.source.borrow().state {
            LoadState::Error(error) => Some(error),
            _ => None,
        }
    }

    pub(crate) fn was_started(&self) -> bool {
        self.started
    }

    pub(crate) fn font_data(&self) -> Option<FontFaceData> {
        if !self.activated {
            return None;
        }
        match &self.source.borrow().state {
            LoadState::Loaded(data) => Some(data.clone()),
            _ => None,
        }
    }

    pub(crate) fn fail(&mut self, name: &'static str, message: &'static str) {
        self.activated = true;
        self.source.borrow_mut().state = LoadState::Error(FontFaceError { name, message });
    }

    pub(crate) fn initialize_binary(&mut self, bytes: &[u8]) {
        match FontFaceData::from_bytes(bytes) {
            Ok(data) => {
                self.activated = true;
                self.source.borrow_mut().state = LoadState::Loaded(data);
            }
            Err(_) => self.fail("SyntaxError", "Invalid font data in ArrayBuffer."),
        }
    }

    /// Only one consumer may initiate loading, even when CSS and JS both ask.
    pub(crate) fn begin(&mut self) -> bool {
        if self.status() != FontFaceStatus::Unloaded {
            return false;
        }
        self.activated = true;
        self.started = true;
        let mut source = self.source.borrow_mut();
        if matches!(source.state, LoadState::Unloaded) {
            source.state = LoadState::Loading;
            true
        } else {
            false
        }
    }

    /// Advance after starting or after a failed URL. Local lookup and source
    /// exhaustion are terminal here; only actual URL requests leave this layer.
    pub(crate) fn next_url(&mut self, services: &mut DocumentLayoutServices) -> Option<String> {
        if self.status() != FontFaceStatus::Loading {
            return None;
        }
        let mut resource = self.source.borrow_mut();
        let FontSourceResource { sources, state, .. } = &mut *resource;
        for source in sources.by_ref() {
            match source {
                CssFontSource::Local(name) => {
                    if let Some(data) = services.local_font_source(&name) {
                        *state = LoadState::Loaded(data);
                        return None;
                    }
                }
                CssFontSource::Url(url) => return Some(url),
            }
        }
        *state = LoadState::Error(FontFaceError {
            name: "NetworkError",
            message: "A network error occurred while loading the font.",
        });
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
            self.source.borrow_mut().state = LoadState::Loaded(FontFaceData::from_bytes(bytes)?);
        }
        Ok(())
    }

    pub(crate) fn register(
        &self,
        services: &mut DocumentLayoutServices,
        scope: WebFontRegistrationScope,
    ) -> Result<Option<WebFontRegistrationOutcome>, WebFontRegistrationError> {
        if !self.activated {
            return Ok(None);
        }
        let source = self.source.borrow();
        let LoadState::Loaded(data) = &source.state else {
            return Ok(None);
        };
        let registration = WebFontRegistration::from_font_data(
            self.slot.clone(),
            self.descriptor.as_ref().clone(),
            data.clone(),
        )
        .with_scope(scope)
        .with_source_key(source.source_key.clone());
        services.register_web_font(registration).map(Some)
    }
}
