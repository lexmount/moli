//! Document font requests are style/resource work. No request is discovered or
//! started by layout, a geometry getter, text shaping, or screenshot production.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{
    document_runtime::DomHandle,
    font_loading::{
        FontFaceLoad, FontFaceStatus,
        demand::{FontSourceSignature, pending_families, requested_sources},
        usage::collect_font_uses,
    },
    native_bridge::{
        JsContextHost, WindowExecutionContextOwner,
        element::{StyleObservation, style_viewport_for_document},
    },
    style_engine::{StyleViewport, StyloStyleEnvironment},
    types::SubresourceResourceType,
    util::{get_private_value, set_private_value},
};

const SOURCES_ID: &str = "__moliAutomaticFontSources";
type Store = Rc<RefCell<FontSources>>;

#[derive(Default)]
struct FontSources {
    next_id: u64,
    sets: HashMap<u64, DocumentSources>,
}

struct DocumentSources {
    _wrapper: v8::Weak<v8::Object>,
    host: usize,
    document: DomHandle,
    owner: WindowExecutionContextOwner,
    faces: Vec<FontFaceLoad>,
    manual_start: usize,
    observed: Option<Inputs>,
    #[cfg(test)]
    discoveries: u64,
}

#[derive(PartialEq)]
struct Inputs {
    dom_version: u64,
    style_generations: Vec<(DomHandle, u64, u64)>,
    visual_generation: u64,
    viewport: StyleViewport,
    environment: StyloStyleEnvironment,
    faces: Vec<FontSourceSignature>,
}

impl Inputs {
    fn new(host: &JsContextHost, document: DomHandle, faces: &[FontFaceLoad]) -> Self {
        // A ready getter can run before the resource phase has observed an
        // ancestor's CSSOM edits. Refresh embedding styles before consulting
        // the inferred viewport cache, from the outermost frame inward.
        let mut frames = Vec::new();
        let mut ancestor = document;
        while let Some(frame) = host.child_browsing_context_host_for_document_handle(ancestor) {
            if frames.contains(&frame) {
                break;
            }
            frames.push(frame);
            let Some(parent) = host.dom_host().owner_document_handle(frame) else {
                break;
            };
            ancestor = parent;
        }
        {
            let mut observation = StyleObservation::new(host);
            for frame in frames.into_iter().rev() {
                observation.read(frame);
            }
        }
        let viewport = style_viewport_for_document(host, document);
        Self {
            dom_version: host.dom_host().dom_version(),
            style_generations: host.document_style_input_generations(document),
            visual_generation: host.visual_state_generation(),
            viewport,
            environment: StyloStyleEnvironment::from_emulated_media(host.emulated_media()),
            faces: faces.iter().map(FontSourceSignature::new).collect(),
        }
    }
}

pub(super) fn update_sources<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    fonts: v8::Local<'s, v8::Object>,
    host_ptr: *mut JsContextHost,
    document: DomHandle,
    connected: &[String],
    manual: &[String],
) {
    let existing = get_private_value(scope, fonts, SOURCES_ID)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0);
    if manual.is_empty() && existing.is_none() {
        return;
    }
    let host = unsafe { &*host_ptr };
    let Some(dispatch) = host.owner_dispatch_scope_for_node(document) else {
        return;
    };
    let Some(owner) = host.current_window_execution_context_owner(dispatch) else {
        return;
    };
    let Some(faces) = super::storage::font_face_set_faces_array(scope, fonts) else {
        return;
    };
    let mut by_slot = HashMap::new();
    for index in 0..faces.length() {
        if let Some(face) = faces
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            && let Some(load) = super::state::get(scope, face)
        {
            by_slot.insert(load.borrow().slot().to_owned(), load.clone());
        }
    }
    let ordered = connected
        .iter()
        .chain(manual)
        .filter_map(|slot| by_slot.get(slot).cloned())
        .collect::<Vec<_>>();
    let manual_start = connected
        .iter()
        .filter(|slot| by_slot.contains_key(*slot))
        .count();
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = existing.unwrap_or_else(|| {
        let mut state = store.borrow_mut();
        state.next_id = state
            .next_id
            .checked_add(1)
            .expect("font source identity exhausted");
        state.next_id
    });
    if let Some(entry) = store.borrow_mut().sets.get_mut(&id) {
        if entry.manual_start != manual_start
            || entry.faces.len() != ordered.len()
            || entry
                .faces
                .iter()
                .zip(&ordered)
                .any(|(before, after)| !Rc::ptr_eq(before, after))
        {
            entry.observed = None;
        }
        entry.faces = ordered;
        entry.manual_start = manual_start;
        return;
    }
    let weak = Rc::downgrade(&store);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        fonts,
        Box::new(move |_| {
            if let Some(store) = weak.upgrade() {
                store.borrow_mut().sets.remove(&id);
            }
        }),
    );
    store.borrow_mut().sets.insert(
        id,
        DocumentSources {
            _wrapper: wrapper,
            host: host_ptr as usize,
            document,
            owner,
            faces: ordered,
            manual_start,
            observed: None,
            #[cfg(test)]
            discoveries: 0,
        },
    );
    let value = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, fonts, SOURCES_ID, value.into());
}

pub(crate) fn discover_requests(scope: &mut v8::PinScope<'_, '_>) {
    let Some(store) = scope.get_slot::<Store>().cloned() else {
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let pending = {
        let host = unsafe { &*host_ptr };
        store
            .borrow()
            .sets
            .iter()
            .filter(|(_, entry)| {
                entry.host == host_ptr as usize
                    // Implicit requests share the CSS font resource opt-in.
                    // Check before scanning faces or observing document styles.
                    && host
                        .owner_dispatch_scope_for_node(entry.document)
                        .and_then(|dispatch| {
                            host.document_resource_loader_for_dispatch_scope(dispatch)
                        })
                        .is_some_and(|loader| {
                            loader
                                .request_client()
                                .optional_resource_fetch_enabled(SubresourceResourceType::Font)
                        })
                    && entry.faces[entry.manual_start..]
                        .iter()
                        .any(|face| face.borrow().status() == FontFaceStatus::Unloaded)
            })
            .map(|(&id, entry)| {
                (
                    id,
                    entry.document,
                    entry.owner,
                    entry.faces.clone(),
                    entry.manual_start,
                )
            })
            .collect::<Vec<_>>()
    };
    for (id, document, owner, sources, manual_start) in pending {
        let requests = {
            let host = unsafe { &*host_ptr };
            let current = host
                .owner_dispatch_scope_for_node(document)
                .and_then(|dispatch| host.current_window_execution_context_owner(dispatch));
            if current != Some(owner) {
                continue;
            }
            let inputs = Inputs::new(host, document, &sources);
            if store
                .borrow()
                .sets
                .get(&id)
                .is_some_and(|entry| entry.observed.as_ref() == Some(&inputs))
            {
                continue;
            }
            let families = pending_families(&sources, manual_start);
            let uses = collect_font_uses(host, document, &families);
            let requests = requested_sources(&sources, manual_start, &uses);
            // Style observation can update its retained world. Cache the final
            // resource inputs rather than repeatedly discovering clean text.
            if let Some(entry) = store.borrow_mut().sets.get_mut(&id) {
                entry.observed = Some(Inputs::new(host, document, &sources));
                #[cfg(test)]
                {
                    entry.discoveries += 1;
                }
            }
            requests
        };
        // All native/style borrows are gone before creating a Window task.
        super::loading::queue_requested_font_loads(scope, &requests);
    }
}

#[cfg(test)]
pub(crate) fn discovery_count(scope: &mut v8::PinScope<'_, '_>) -> u64 {
    scope.get_slot::<Store>().map_or(0, |store| {
        store
            .borrow()
            .sets
            .values()
            .map(|entry| entry.discoveries)
            .sum()
    })
}
