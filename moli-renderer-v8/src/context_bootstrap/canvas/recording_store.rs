//! Per-context ordered drawing recording.
//!
//! The isolate owns the table. Weak context handles remove entries on GC;
//! dropping the isolate also drops every remaining entry without needing GC.
//! This mirrors the pattern in `state.rs` for path state.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::util::{get_private_value, set_private_value};
use moli_canvas::DrawRecording;

const RECORDING_STATE_SLOT: &str = "__moliCanvasRecordingState";
type RecordingStore = Rc<RefCell<Recordings>>;

#[derive(Default)]
struct Recordings {
    next_id: u64,
    entries: HashMap<u64, RecordingEntry>,
}

struct RecordingEntry {
    _context: v8::Weak<v8::Object>,
    recording: Rc<RefCell<DrawRecording>>,
}

fn recording_store<'s>(scope: &mut v8::PinScope<'s, '_>) -> RecordingStore {
    if let Some(store) = scope.get_slot::<RecordingStore>() {
        store.clone()
    } else {
        let store = RecordingStore::default();
        scope.set_slot(store.clone());
        store
    }
}

pub(super) fn canvas_recording_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<DrawRecording>> {
    let store = recording_store(scope);
    if let Some(id) = get_private_value(scope, context, RECORDING_STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
    {
        return store
            .borrow()
            .entries
            .get(&id)
            .expect("a live context must retain its recording")
            .recording
            .clone();
    }
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("canvas recording identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let owner = v8::Weak::with_finalizer(
        scope,
        context,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    let recording = Rc::new(RefCell::new(DrawRecording::new()));
    store.borrow_mut().entries.insert(
        id,
        RecordingEntry {
            _context: owner,
            recording: recording.clone(),
        },
    );
    let id = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, context, RECORDING_STATE_SLOT, id.into());
    recording
}

pub(super) fn reset_canvas_recording<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    let recording = canvas_recording_state(scope, context);
    recording.borrow_mut().clear();
}

/// Flushes all live recordings against their surfaces.
///
/// For each recording entry whose context handle is still alive, the recording
/// is executed against the associated canvas surface and the snapshot is
/// published. This must be called before any page-level paint (screencast,
/// screenshot, layout) that reads published canvas pixels.
pub(crate) fn flush_all_recordings<'s>(scope: &mut v8::PinScope<'s, '_>) {
    let store = recording_store(scope);
    let ids: Vec<u64> = {
        let s = store.borrow();
        s.entries.keys().copied().collect()
    };
    for id in ids {
        let (context_obj, recording) = {
            let s = store.borrow();
            let Some(entry) = s.entries.get(&id) else {
                continue;
            };
            let Some(context_obj) = entry._context.to_local(scope) else {
                continue;
            };
            (context_obj, entry.recording.clone())
        };
        let mut rec = recording.borrow_mut();
        if rec.is_empty() {
            continue;
        }
        let Some(canvas) = super::backing_store::canvas_owner_from_context(scope, context_obj)
        else {
            rec.clear();
            continue;
        };
        let (width, height) = match super::backing_store::canvas_like_dimensions(scope, canvas) {
            Some(dims) => dims,
            None => {
                rec.clear();
                continue;
            }
        };
        let cell = super::backing_store::canvas_surface_cell(scope, canvas);
        if !super::backing_store::materialize_surface(&cell, width, height) {
            rec.clear();
            continue;
        }
        {
            let mut surface = cell.borrow_mut();
            let Some(surface) = surface.as_mut() else {
                rec.clear();
                continue;
            };
            let _ = rec.execute(surface);
        }
        rec.clear();
        drop(rec);
        super::backing_store::publish_canvas_snapshot(scope, canvas);
    }
}
