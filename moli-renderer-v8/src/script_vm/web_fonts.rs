use std::collections::{BTreeMap, BTreeSet};

use moli_layout::{
    DocumentLayoutServices, WebFontFace, WebFontRegistrationError, WebFontRegistrationOutcome,
};
use url::Url;

use crate::css_resource_urls::{CompletedStylesheetWebFont, StylesheetLoadBlockingResource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebFontRequestStatus {
    Pending,
    Ready,
    Failed,
}

#[derive(Clone, Debug)]
struct WebFontSlot {
    request_url: Url,
    face: WebFontFace,
    request_id: u64,
    status: WebFontRequestStatus,
}

/// Resource state for the current main Document's downloadable fonts.
///
/// A request id is an identity for one concrete fetch, not a generation or a
/// layout consistency fence. It prevents an old response from committing after
/// CSSOM replaced or removed the declaration occupying its content-addressed
/// slot. A fresh layout refresh reads only ready registrations; cached geometry
/// reads do not consult or mutate this sidecar.
#[derive(Default)]
pub(crate) struct DocumentWebFontState {
    next_request_id: u64,
    slots: BTreeMap<String, WebFontSlot>,
}

#[derive(Debug)]
pub(crate) enum DocumentWebFontCompletion {
    Registered(WebFontRegistrationOutcome),
    Invalid(WebFontRegistrationError),
    NetworkFailed,
    Stale,
}

impl DocumentWebFontState {
    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Removes registrations whose declarations are no longer in the current
    /// Stylo source set. Pending network work may still finish, but its exact
    /// request identity will no longer be accepted by [`Self::complete`].
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
        let removed = self
            .slots
            .keys()
            .filter(|slot| !active.contains(*slot))
            .cloned()
            .collect::<Vec<_>>();
        for slot in removed {
            self.slots.remove(&slot);
            services.remove_web_font(&slot);
        }
    }

    /// Binds a new request identity, or suppresses a duplicate request for a
    /// declaration that is already pending, ready, or terminally failed.
    pub(crate) fn admit(
        &mut self,
        resource: StylesheetLoadBlockingResource,
        services: &mut DocumentLayoutServices,
    ) -> Option<StylesheetLoadBlockingResource> {
        let Some(font) = resource.web_font() else {
            return Some(resource);
        };
        let slot = font.slot().to_owned();
        if self.slots.get(&slot).is_some_and(|current| {
            current.request_url == *resource.request_url() && current.face == *font.face()
        }) {
            return None;
        }

        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("document web-font request identity space exhausted");
        let request_id = self.next_request_id;
        services.remove_web_font(&slot);
        self.slots.insert(
            slot,
            WebFontSlot {
                request_url: resource.request_url().clone(),
                face: font.face().clone(),
                request_id,
                status: WebFontRequestStatus::Pending,
            },
        );
        Some(resource.bind_web_font_request(request_id))
    }

    pub(crate) fn complete(
        &mut self,
        terminal: CompletedStylesheetWebFont,
        services: &mut DocumentLayoutServices,
    ) -> DocumentWebFontCompletion {
        let (request, bytes) = terminal.into_parts();
        let Some(request_id) = request.request_id() else {
            return DocumentWebFontCompletion::Stale;
        };
        let Some(slot) = self.slots.get_mut(request.slot()) else {
            return DocumentWebFontCompletion::Stale;
        };
        if slot.request_id != request_id || slot.face != *request.face() {
            return DocumentWebFontCompletion::Stale;
        }
        let Some(bytes) = bytes else {
            slot.status = WebFontRequestStatus::Failed;
            return DocumentWebFontCompletion::NetworkFailed;
        };
        match services.register_web_font(request.registration(bytes)) {
            Ok(outcome) => {
                slot.status = WebFontRequestStatus::Ready;
                DocumentWebFontCompletion::Registered(outcome)
            }
            Err(error) => {
                slot.status = WebFontRequestStatus::Failed;
                DocumentWebFontCompletion::Invalid(error)
            }
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
            .filter(|slot| slot.status == WebFontRequestStatus::Ready)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_resource_urls::stylesheet_load_blocking_font_resources;
    use crate::protocol_types::OptionalResourceFetchMask;

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
}
