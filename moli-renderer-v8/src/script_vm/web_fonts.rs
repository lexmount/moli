use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::{Rc, Weak},
};

use moli_layout::{DocumentLayoutServices, WebFontRegistrationError, WebFontRegistrationOutcome};

use crate::{
    css_resource_urls::{
        CompletedStylesheetWebFont, StylesheetLoadBlockingResource, StylesheetWebFont,
    },
    font_loading::{FontFaceLoad, FontFaceResource},
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
    // Observing a CSS rule is not the same as activating it for layout. Weak
    // entries let a detached rule keep its FontFace state while JS holds it,
    // without retaining every stylesheet ever seen by the document.
    loads: BTreeMap<String, Weak<RefCell<FontFaceResource>>>,
    // Removing a CSS declaration revokes registration, not promises held by JS.
    // Keep its in-flight resource until the owner-validated response settles it.
    pending: BTreeMap<u64, FontFaceLoad>,
}

#[derive(Debug)]
pub(crate) enum DocumentWebFontCompletion {
    Registered(WebFontRegistrationOutcome),
    Invalid(WebFontRegistrationError),
    NetworkFailed,
    Retry(StylesheetLoadBlockingResource),
    Stale,
}

impl DocumentWebFontState {
    pub(crate) fn observe(&mut self, resource: &StylesheetWebFont) -> FontFaceLoad {
        if let Some(load) = self.loads.get(resource.slot()).and_then(Weak::upgrade) {
            return load;
        }
        let load = FontFaceResource::new(
            resource.slot().to_owned(),
            resource.face().clone(),
            resource.sources().to_vec(),
        );
        self.loads
            .insert(resource.slot().to_owned(), Rc::downgrade(&load));
        load
    }

    pub(crate) fn retain_active_slots<'a>(
        &mut self,
        resources: impl IntoIterator<Item = &'a StylesheetLoadBlockingResource>,
        services: &mut DocumentLayoutServices,
    ) {
        let active = resources
            .into_iter()
            .filter_map(StylesheetLoadBlockingResource::web_font)
            .map(|font| font.slot().to_owned())
            .collect::<BTreeSet<_>>();
        self.slots.retain(|slot, entry| {
            if active.contains(slot) {
                return true;
            }
            entry.load.borrow().unregister(services);
            false
        });
        self.loads.retain(|_, load| load.strong_count() != 0);
    }

    fn bind_request(
        &mut self,
        resource: StylesheetLoadBlockingResource,
        load: FontFaceLoad,
    ) -> StylesheetLoadBlockingResource {
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("document web-font request identity space exhausted");
        self.pending.insert(self.next_request_id, load);
        resource.bind_web_font_request(self.next_request_id)
    }

    pub(crate) fn admit(
        &mut self,
        resource: StylesheetLoadBlockingResource,
        services: &mut DocumentLayoutServices,
    ) -> Option<StylesheetLoadBlockingResource> {
        let Some(font) = resource.web_font() else {
            return Some(resource);
        };
        if let Some(id) = font.request_id() {
            // A native source fallback already has its next exact request.
            return self
                .pending
                .get(&id)
                .is_some_and(|load| load.borrow().slot() == font.slot())
                .then_some(resource);
        }
        let load = self.observe(font);
        self.slots
            .insert(font.slot().to_owned(), WebFontSlot { load: load.clone() });
        if !load.borrow_mut().begin() {
            // Reactivating a previously loaded CSS declaration reuses its data;
            // it must restore registration without starting a second request.
            if let Err(error) = load.borrow().register(services) {
                tracing::warn!(%error, "failed to reactivate stylesheet font");
            }
            return None;
        }
        let next = load.borrow_mut().next_url(services);
        if let Some(url) = next {
            let url = url::Url::parse(&url).expect("CSS font sources retain resolved URLs");
            Some(self.bind_request(resource.with_request_url(url), load))
        } else {
            if let Err(error) = load.borrow().register(services) {
                tracing::warn!(%error, "failed to register local stylesheet font");
            }
            None
        }
    }

    pub(crate) fn complete(
        &mut self,
        terminal: CompletedStylesheetWebFont,
        services: &mut DocumentLayoutServices,
    ) -> DocumentWebFontCompletion {
        let (request, bytes) = terminal.into_parts();
        let Some(id) = request.request_id() else {
            return DocumentWebFontCompletion::Stale;
        };
        if !self
            .pending
            .get(&id)
            .is_some_and(|load| load.borrow().slot() == request.slot())
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
            let resource = StylesheetLoadBlockingResource::font(url, request);
            return DocumentWebFontCompletion::Retry(self.bind_request(resource, load));
        }
        let active = self
            .slots
            .get(request.slot())
            .is_some_and(|slot| std::rc::Rc::ptr_eq(&slot.load, &load));
        if !active {
            return DocumentWebFontCompletion::Stale;
        }
        match load.borrow().register(services) {
            Ok(Some(outcome)) => DocumentWebFontCompletion::Registered(outcome),
            Err(error) => DocumentWebFontCompletion::Invalid(error),
            Ok(None) => error
                .map(DocumentWebFontCompletion::Invalid)
                .unwrap_or(DocumentWebFontCompletion::NetworkFailed),
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
    use crate::css_resource_urls::stylesheet_load_blocking_font_resources;
    use crate::protocol_types::OptionalResourceFetchMask;
    use url::Url;

    const TEST_FONT: &[u8] = include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2");

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
        resource: StylesheetLoadBlockingResource,
        bytes: Option<Vec<u8>>,
    ) -> CompletedStylesheetWebFont {
        let (_, request) = resource.into_parts();
        let request = request.expect("font resource metadata");
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
            .unwrap();
        let second = state
            .admit(
                resource("@font-face{font-family:Second;src:url(font.woff2)}"),
                &mut services,
            )
            .unwrap();

        state.retain_active_slots([&second], &mut services);
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
        let first = state.admit(resource.clone(), &mut services).unwrap();
        assert!(state.admit(resource, &mut services).is_none());
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
        assert_eq!(
            second.request_url().as_str(),
            "https://example.test/second.woff2"
        );
        assert!(matches!(
            state.complete(terminal(duplicate, Some(TEST_FONT.to_vec())), &mut services),
            DocumentWebFontCompletion::Stale
        ));
        assert_eq!(
            load.borrow().status(),
            crate::font_loading::FontFaceStatus::Loading
        );
        assert!(matches!(
            state.complete(terminal(second, Some(TEST_FONT.to_vec())), &mut services),
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
        let request = state.admit(resource, &mut services).unwrap();
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
        let request = state.admit(resource.clone(), &mut services).unwrap();
        state.complete(terminal(request, Some(TEST_FONT.to_vec())), &mut services);
        state.retain_active_slots([], &mut services);
        assert_eq!(services.web_font_count(), 0);
        assert!(Rc::ptr_eq(
            &observer,
            &state.observe(resource.web_font().unwrap())
        ));
        assert!(
            state.admit(resource, &mut services).is_none(),
            "reactivation needs no request"
        );
        assert_eq!(state.ready_slot_count(), 1);
        assert_eq!(services.web_font_count(), 1);
    }
}
