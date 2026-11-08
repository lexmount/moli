use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::{Rc, Weak},
};

use moli_layout::{
    DocumentLayoutServices, WebFontRegistrationError, WebFontRegistrationOutcome,
    WebFontRegistrationScope,
};

use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource, StylesheetSubresourceRequest,
        StylesheetWebFont,
    },
    font_loading::{FontFaceLoad, FontFaceResource, FontSourceResource},
};

#[derive(Clone, Debug)]
struct WebFontSlot {
    load: FontFaceLoad,
}

/// Document membership and exact request identities, not a second font load
/// state machine. CSS requests and JS wrappers share FontFaceResource.
#[derive(Default)]
pub(crate) struct DocumentWebFontState {
    next_request_id: u64,
    slots: BTreeMap<String, WebFontSlot>,
    resource_generation: Option<crate::style_engine::StylesheetResourceGeneration>,
    // Observing a CSS rule is not the same as activating it for layout. Weak
    // entries let a detached rule keep its FontFace state while JS holds it,
    // without retaining every stylesheet ever seen by the document.
    loads: BTreeMap<String, Weak<RefCell<FontFaceResource>>>,
    sources: BTreeMap<String, Weak<RefCell<FontSourceResource>>>,
    // Removing a CSS declaration revokes registration, not promises held by JS.
    // Keep its in-flight resource until the owner-validated response settles it.
    pending: BTreeMap<u64, FontFaceLoad>,
}

#[derive(Debug)]
pub(crate) enum DocumentWebFontCompletion {
    Registered(WebFontRegistrationOutcome),
    Invalid(WebFontRegistrationError),
    NetworkFailed,
    Retry(Box<StylesheetSubresourceRequest>),
    Stale,
}

/// Loading can finish from cached/local data without producing a request.
/// Publish registration changes independently so geometry is invalidated then.
pub(crate) struct PreparedStylesheetResourceLoad {
    pub(crate) request: Option<StylesheetSubresourceRequest>,
    pub(crate) fonts_changed: bool,
}

impl DocumentWebFontState {
    pub(crate) fn is_pristine(&self) -> bool {
        self.resource_generation.is_none() && self.slots.is_empty()
    }

    pub(crate) fn resources_are_current(
        &self,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) -> bool {
        self.resource_generation == Some(generation)
    }

    pub(crate) fn publish_resource_generation(
        &mut self,
        generation: crate::style_engine::StylesheetResourceGeneration,
    ) {
        self.resource_generation = Some(generation);
    }

    pub(crate) fn has_pending_requests(&self) -> bool {
        self.slots
            .values()
            .any(|slot| slot.load.borrow().status() == crate::font_loading::FontFaceStatus::Loading)
    }

    pub(crate) fn observe(&mut self, resource: &StylesheetWebFont) -> FontFaceLoad {
        if let Some(load) = self
            .loads
            .get(resource.registration_slot())
            .and_then(Weak::upgrade)
        {
            return load;
        }
        let source = self
            .sources
            .get(resource.resource_key())
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let source = FontSourceResource::new(
                    resource.sources().to_vec(),
                    Some(resource.resource_key().to_owned()),
                );
                self.sources
                    .insert(resource.resource_key().to_owned(), Rc::downgrade(&source));
                source
            });
        let load = FontFaceResource::with_source(
            resource.registration_slot().to_owned(),
            resource.face().clone(),
            source,
        );
        self.loads.insert(
            resource.registration_slot().to_owned(),
            Rc::downgrade(&load),
        );
        load
    }

    pub(crate) fn retain_active_slots<'a>(
        &mut self,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
        services: &mut DocumentLayoutServices,
    ) -> bool {
        let resources = resources
            .into_iter()
            .filter_map(StylesheetLoadBlockingResource::web_font)
            .collect::<Vec<_>>();
        // Establish the new declarations before retiring old ones, so their
        // shared decoded source survives an owner/projection identity change.
        for font in &resources {
            let load = self.observe(font);
            self.slots
                .entry(font.registration_slot().to_owned())
                .or_insert(WebFontSlot { load });
        }
        let order = resources
            .into_iter()
            .map(|font| font.registration_slot().to_owned())
            .collect::<Vec<_>>();
        let active = order.iter().collect::<BTreeSet<_>>();
        let removed = self
            .slots
            .keys()
            .filter(|slot| !active.contains(slot))
            .cloned()
            .collect::<Vec<_>>();
        self.slots.retain(|slot, _| active.contains(slot));
        let changed = services.remove_stylesheet_web_fonts(removed.iter().map(String::as_str));
        self.loads.retain(|_, load| load.strong_count() != 0);
        self.sources.retain(|_, source| source.strong_count() != 0);
        changed | services.set_stylesheet_web_font_order(&order)
    }

    fn bind_request(
        &mut self,
        url: url::Url,
        font: StylesheetWebFont,
        load: FontFaceLoad,
    ) -> StylesheetSubresourceRequest {
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("document web-font request identity space exhausted");
        self.pending.insert(self.next_request_id, load);
        StylesheetSubresourceRequest::font(url, font.bind_request(self.next_request_id))
    }

    /// Resolve local sources and bind a request only when a network source is needed.
    pub(crate) fn admit(
        &mut self,
        resource: StylesheetLoadBlockingResource,
        services: &mut DocumentLayoutServices,
    ) -> PreparedStylesheetResourceLoad {
        let font = match resource {
            StylesheetLoadBlockingResource::Image(url) => {
                return PreparedStylesheetResourceLoad {
                    request: Some(StylesheetSubresourceRequest::image(url, None)),
                    fonts_changed: false,
                };
            }
            StylesheetLoadBlockingResource::Font { font, .. } => font,
        };
        let load = self.observe(&font);
        self.slots.insert(
            font.registration_slot().to_owned(),
            WebFontSlot { load: load.clone() },
        );
        let next = if load.borrow_mut().begin() {
            load.borrow_mut().next_url(services)
        } else {
            None
        };
        if let Some(url) = next {
            let url = url::Url::parse(&url).expect("CSS font sources retain resolved URLs");
            return PreparedStylesheetResourceLoad {
                request: Some(self.bind_request(url, font, load)),
                fonts_changed: false,
            };
        }
        let fonts_changed = match load
            .borrow()
            .register(services, WebFontRegistrationScope::Stylesheet)
        {
            Ok(Some(WebFontRegistrationOutcome::Added | WebFontRegistrationOutcome::Replaced)) => {
                true
            }
            Ok(_) => false,
            Err(error) => {
                tracing::warn!(%error, "failed to register cached/local stylesheet font");
                false
            }
        };
        PreparedStylesheetResourceLoad {
            request: None,
            fonts_changed,
        }
    }

    pub(crate) fn complete(
        &mut self,
        terminal: CompletedStylesheetWebFont,
        services: &mut DocumentLayoutServices,
    ) -> DocumentWebFontCompletion {
        let (request, bytes) = terminal.into_parts();
        let id = request.request_id();
        if self
            .pending
            .get(&id)
            .is_none_or(|load| load.borrow().slot() != request.font().registration_slot())
        {
            return DocumentWebFontCompletion::Stale;
        }
        let load = self
            .pending
            .remove(&id)
            .expect("validated exact font request");
        let error = load.borrow_mut().accept_response(bytes.as_deref()).err();
        let next = load.borrow_mut().next_url(services);
        if let Some(url) = next {
            let url = url::Url::parse(&url).expect("CSS font fallback retains its parser URL");
            return DocumentWebFontCompletion::Retry(Box::new(self.bind_request(
                url,
                request.into_font(),
                load,
            )));
        }
        let mut active = false;
        let mut registered = None;
        let mut registration_error = None;
        for entry in self.slots.values() {
            let face = entry.load.borrow();
            if !face.shares_source(&load.borrow()) {
                continue;
            }
            active = true;
            match face.register(services, WebFontRegistrationScope::Stylesheet) {
                Ok(Some(outcome)) => {
                    registered.get_or_insert(outcome);
                }
                Err(error) => registration_error = Some(error),
                Ok(None) => {}
            }
        }
        if !active {
            return DocumentWebFontCompletion::Stale;
        }
        if let Some(outcome) = registered {
            DocumentWebFontCompletion::Registered(outcome)
        } else {
            registration_error
                .or(error)
                .map(DocumentWebFontCompletion::Invalid)
                .unwrap_or(DocumentWebFontCompletion::NetworkFailed)
        }
    }

    #[cfg(test)]
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    #[cfg(test)]
    pub(crate) fn ready_slot_count(&self) -> usize {
        self.slots
            .values()
            .filter(|slot| {
                slot.load.borrow().status() == crate::font_loading::FontFaceStatus::Loaded
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_resource_urls::{
        StylesheetSubresourceTarget, stylesheet_load_blocking_font_resources,
    };
    use crate::protocol_types::OptionalResourceFetchMask;
    use url::Url;

    const TEST_FONT: &[u8] = include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2");

    #[test]
    fn independent_declarations_share_source_loading_but_not_descriptors_or_registrations() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let first = resource("@font-face{font-family:First;src:url(font.woff2)}");
        let second = resource("@font-face{font-family:Second;src:url(font.woff2)}");
        assert_eq!(
            first.web_font().unwrap().resource_key(),
            second.web_font().unwrap().resource_key()
        );
        let first_load = state.observe(first.web_font().unwrap());
        let second_load = state.observe(second.web_font().unwrap());
        assert!(!Rc::ptr_eq(&first_load, &second_load));
        assert!(first_load.borrow().shares_source(&second_load.borrow()));
        let request = state.admit(first, &mut services).request.unwrap();
        assert_eq!(
            second_load.borrow().status(),
            crate::font_loading::FontFaceStatus::Unloaded
        );
        assert!(
            state.admit(second, &mut services).request.is_none(),
            "the second face joins the existing source operation"
        );
        assert_eq!(state.pending.len(), 1);
        assert!(matches!(
            state.complete(terminal(request, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Registered(_)
        ));
        assert_eq!(services.web_font_count(), 2);
        assert_eq!(state.ready_slot_count(), 2);
        assert!(state.pending.is_empty());
        first_load
            .borrow_mut()
            .set_descriptor(moli_layout::WebFontFace::new("Moved"));
        assert_eq!(second_load.borrow().descriptor().family_name(), "Second");
        assert_ne!(first_load.borrow().slot(), second_load.borrow().slot());
    }

    fn resource(css: &str) -> StylesheetLoadBlockingResource {
        stylesheet_load_blocking_font_resources(
            css,
            &Url::parse("https://example.test/style.css").unwrap(),
            OptionalResourceFetchMask::FONT,
        )
        .into_iter()
        .next()
        .expect("fixture should contain one downloadable font")
    }

    fn terminal(
        resource: StylesheetSubresourceRequest,
        bytes: Option<Vec<u8>>,
    ) -> CompletedStylesheetWebFont {
        let (_, request) = resource.into_parts();
        let StylesheetSubresourceTarget::Font(request) = request else {
            panic!("fixture must prepare a font request");
        };
        match bytes {
            Some(bytes) => CompletedStylesheetWebFont::response(request, bytes),
            None => CompletedStylesheetWebFont::failure(request),
        }
    }

    #[test]
    fn superseded_request_cannot_commit_into_the_new_face_slot() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let first = state
            .admit(
                resource("@font-face{font-family:First;src:url(font.woff2)}"),
                &mut services,
            )
            .request
            .unwrap();
        let second_resource = resource("@font-face{font-family:Second;src:url(replacement.woff2)}");
        let second = state
            .admit(second_resource.clone(), &mut services)
            .request
            .unwrap();

        state.retain_active_slots([&second_resource], &mut services);
        assert!(matches!(
            state.complete(terminal(first, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Stale
        ));
        assert!(matches!(
            state.complete(terminal(second, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Registered(WebFontRegistrationOutcome::Added)
        ));
        assert_eq!(services.web_font_count(), 1);
        assert_eq!(state.ready_slot_count(), 1);
    }

    #[test]
    fn removal_revokes_pending_and_ready_declarations_without_a_generation() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let admitted = state
            .admit(
                resource("@font-face{font-family:Demo;src:url(font.woff2)}"),
                &mut services,
            )
            .request
            .unwrap();
        let late = admitted.clone();
        assert!(matches!(
            state.complete(terminal(admitted, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Registered(WebFontRegistrationOutcome::Added)
        ));

        state.retain_active_slots([], &mut services);
        assert_eq!(state.slot_count(), 0);
        assert_eq!(services.web_font_count(), 0);
        assert!(matches!(
            state.complete(terminal(late, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Stale
        ));
    }

    #[test]
    fn failed_sources_advance_one_native_load_and_reject_duplicate_completions() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let resource =
            resource("@font-face{font-family:Demo;src:url(first.woff2),url(second.woff2)}");
        let load = state.observe(resource.web_font().unwrap());
        let first = state
            .admit(resource.clone(), &mut services)
            .request
            .unwrap();
        assert!(state.admit(resource, &mut services).request.is_none());
        assert_eq!(
            load.borrow().status(),
            crate::font_loading::FontFaceStatus::Loading
        );
        let duplicate = first.clone();
        let DocumentWebFontCompletion::Retry(second) =
            state.complete(terminal(first, Some(b"bad".to_vec())), &mut services)
        else {
            panic!("invalid first source should advance to its fallback");
        };
        assert_eq!(second.url().as_str(), "https://example.test/second.woff2");
        assert!(matches!(
            state.complete(terminal(duplicate, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Stale
        ));
        assert_eq!(
            load.borrow().status(),
            crate::font_loading::FontFaceStatus::Loading
        );
        assert!(matches!(
            state.complete(terminal(*second, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Registered(_)
        ));
        assert_eq!(
            load.borrow().status(),
            crate::font_loading::FontFaceStatus::Loaded
        );
        assert_eq!(services.web_font_count(), 1);
    }

    #[test]
    fn removed_pending_css_font_still_settles_native_observers_without_registering() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let resource = resource("@font-face{font-family:Demo;src:url(font.woff2)}");
        let observer = state.observe(resource.web_font().unwrap());
        let request = state.admit(resource, &mut services).request.unwrap();
        state.retain_active_slots([], &mut services);
        assert!(matches!(
            state.complete(terminal(request, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Stale
        ));
        assert_eq!(
            observer.borrow().status(),
            crate::font_loading::FontFaceStatus::Loaded
        );
        assert_eq!(services.web_font_count(), 0);
    }

    #[test]
    fn observing_is_not_admission_and_reactivation_reuses_loaded_data() {
        let mut state = DocumentWebFontState::default();
        let mut services = DocumentLayoutServices::new();
        let resource = resource("@font-face{font-family:Demo;src:url(font.woff2)}");
        let observer = state.observe(resource.web_font().unwrap());
        assert_eq!(
            state.slot_count(),
            0,
            "a CSSOM observer does not activate the rule"
        );
        let request = state
            .admit(resource.clone(), &mut services)
            .request
            .unwrap();
        state.complete(terminal(request, Some(TEST_FONT.to_vec())), &mut services);
        state.retain_active_slots([], &mut services);
        assert_eq!(services.web_font_count(), 0);
        assert!(Rc::ptr_eq(
            &observer,
            &state.observe(resource.web_font().unwrap())
        ));
        assert!(
            state.admit(resource, &mut services).request.is_none(),
            "reactivation needs no request"
        );
        assert_eq!(state.ready_slot_count(), 1);
        assert_eq!(services.web_font_count(), 1);
    }
}
