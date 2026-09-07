//! Authoritative per-canvas native pixel surface, replacing the V8 backing
//! array.
//!
//! A Canvas with a 2D context owns one [`moli_canvas::CanvasSurface`] that
//! holds its pixels as its single writable owner (premultiplied RGBA8, with
//! straight-alpha handled at the observation/`ImageData`/publication boundary).
//! The native surface is keyed by the canvas-like JS object through a
//! weak-keyed per-context registry, so its lifetime is reclaimed with the canvas
//! (GC) and with the isolate, mirroring `state.rs`.
//!
//! The existing immediate-execution draw helpers operate on straight RGBA8, so
//! they run through [`CanvasSurface::with_straight_pixels_mut`] — a transitional
//! adapter that yields byte-identical results while the surface stays the single
//! owner. M4's ordered recorder replaces this per-call conversion with batched
//! Vello rendering.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::util::{get_private_object, get_private_value, set_private_value};
use crate::webidl;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, node_runtime_and_handle_from_object_or_detached},
};
use moli_canvas::{CanvasSurface, encode_data_url};
use moli_webapi_declare::WebApiObject;

const CANVAS_SURFACE_ID_SLOT: &str = "__moliCanvasLiveSurfaceId";
const CANVAS_OWNER_SLOT: &str = "__moliCanvasOwner";
const CANVAS_HAS_CONTEXT_SLOT: &str = "__moliCanvasHasContext";
const CANVAS_2D_CONTEXT_SLOT: &str = "__moliCanvas2DContext";

type SurfaceCell = Rc<RefCell<Option<CanvasSurface>>>;
type SurfaceStore = Rc<RefCell<SurfaceRegistry>>;

#[derive(Default)]
struct SurfaceRegistry {
    next_id: u64,
    entries: HashMap<u64, SurfaceRegistryEntry>,
}

struct SurfaceRegistryEntry {
    _context: v8::Weak<v8::Object>,
    surface: SurfaceCell,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CanvasContextOwnerDeclaration<'scope> {
    #[webapi(data_property)]
    canvas: v8::Local<'scope, v8::Object>,
}

pub(crate) fn attach_canvas_like_context_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Object>,
) {
    let _ = CanvasContextOwnerDeclaration::new(canvas).initialize(scope, context);
    set_private_value(scope, context, CANVAS_OWNER_SLOT, canvas.into());
    if get_private_value(scope, context, super::CANVAS_CONTEXT_FILL_STYLE_SLOT).is_some() {
        set_private_value(scope, canvas, CANVAS_2D_CONTEXT_SLOT, context.into());
    }
    set_private_value(
        scope,
        canvas,
        CANVAS_HAS_CONTEXT_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let _ = canvas_surface_cell(scope, canvas);
}

pub(super) fn canvas_like_has_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, canvas, CANVAS_HAS_CONTEXT_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn reset_canvas_like_backing_store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    if let Some(context) = canvas_2d_context(scope, canvas) {
        super::context2d::reset_canvas_context_state(scope, context);
    }
    let Some((width, height)) = canvas_like_dimensions(scope, canvas) else {
        remove_canvas_surface(scope, canvas);
        return;
    };
    let cell = canvas_surface_cell(scope, canvas);
    let too_large = !materialize_surface(&cell, width, height);
    if too_large {
        remove_canvas_surface(scope, canvas);
        return;
    }
    cell.borrow_mut()
        .as_mut()
        .expect("surface materialized")
        .reset();
    publish_canvas_snapshot(scope, canvas);
}

pub(crate) fn reset_html_canvas_backing_store_for_dimension_assignment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    namespace: Option<&str>,
    local_name: &str,
) {
    if namespace.is_some()
        || !unsafe { &*runtime_ptr }
            .dom_host()
            .is_html_element_named(handle, "canvas")
        || (!local_name.eq_ignore_ascii_case("width") && !local_name.eq_ignore_ascii_case("height"))
    {
        return;
    }
    let Some(canvas) = crate::util::node_wrapper_from_handle(scope, handle) else {
        let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
        return;
    };
    if !canvas_like_has_context(scope, canvas) {
        let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
        return;
    }
    reset_canvas_like_backing_store(scope, canvas);
}

pub(crate) fn canvas_like_to_data_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let (bytes, width, height) = canvas_like_pixels_copy(scope, canvas)?;
    encode_data_url(&bytes, width, height)
}

pub(super) fn canvas_owner_from_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, context, CANVAS_OWNER_SLOT)
}

pub(super) fn with_canvas_like_pixels_mut<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    mutate: F,
) -> bool
where
    F: FnOnce(&mut [u8], u32, u32),
{
    let Some((width, height)) = canvas_like_dimensions(scope, canvas) else {
        return false;
    };
    let cell = canvas_surface_cell(scope, canvas);
    if !materialize_surface(&cell, width, height) {
        return false;
    }
    {
        let mut surface = cell.borrow_mut();
        let Some(surface) = surface.as_mut() else {
            return false;
        };
        if surface.with_straight_pixels_mut(mutate).is_none() {
            return false;
        }
    }
    publish_canvas_snapshot(scope, canvas);
    true
}

pub(super) fn canvas_like_pixels_copy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(Vec<u8>, u32, u32)> {
    let (width, height) = canvas_like_dimensions(scope, canvas)?;
    let cell = canvas_surface_cell(scope, canvas);
    if !materialize_surface(&cell, width, height) {
        return None;
    }
    let mut surface = cell.borrow_mut();
    let surface = surface.as_mut()?;
    let snapshot = surface.snapshot().ok()?;
    Some((snapshot.rgba.clone(), snapshot.width, snapshot.height))
}

pub(super) fn canvas_2d_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, canvas, CANVAS_2D_CONTEXT_SLOT)
}

fn surface_store<'s>(scope: &mut v8::PinScope<'s, '_>) -> SurfaceStore {
    if let Some(store) = scope.get_slot::<SurfaceStore>() {
        return store.clone();
    }
    let store = SurfaceStore::default();
    scope.set_slot(store.clone());
    store
}

/// Returns (or creates, once per canvas lifetime) the per-canvas surface cell.
fn canvas_surface_cell<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> SurfaceCell {
    let store = surface_store(scope);
    if let Some(id) = get_private_value(scope, canvas, CANVAS_SURFACE_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
    {
        return store
            .borrow()
            .entries
            .get(&id)
            .expect("a live canvas must retain its native surface")
            .surface
            .clone();
    }
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("canvas surface identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let owner = v8::Weak::with_finalizer(
        scope,
        canvas,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    let surface: SurfaceCell = Rc::new(RefCell::new(None));
    store.borrow_mut().entries.insert(
        id,
        SurfaceRegistryEntry {
            _context: owner,
            surface: surface.clone(),
        },
    );
    let id = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, canvas, CANVAS_SURFACE_ID_SLOT, id.into());
    surface
}

/// Materializes the surface cell at `width` x `height`, resizing (and resetting
/// content) only when the dimensions change. Same-size access preserves content.
/// Returns `false` when the dimensions exceed the budget, leaving no surface.
fn materialize_surface(cell: &SurfaceCell, width: u32, height: u32) -> bool {
    let mut guard = cell.borrow_mut();
    match guard.as_mut() {
        Some(surface) => {
            if surface.width() == width && surface.height() == height {
                true
            } else {
                match surface.resize(width, height) {
                    Ok(()) => true,
                    Err(_) => {
                        *guard = None;
                        false
                    }
                }
            }
        }
        None => match CanvasSurface::new(width, height) {
            Ok(surface) => {
                *guard = Some(surface);
                true
            }
            Err(_) => false,
        },
    }
}

fn remove_canvas_surface<'s>(scope: &mut v8::PinScope<'s, '_>, canvas: v8::Local<'s, v8::Object>) {
    let cell = canvas_surface_cell(scope, canvas);
    *cell.borrow_mut() = None;
    remove_html_canvas_pixels(scope, canvas);
}

fn html_canvas_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let (runtime_ptr, handle) =
        node_runtime_and_handle_from_object_or_detached(scope, canvas).ok()?;
    unsafe { &*runtime_ptr }
        .dom_host()
        .is_html_element_named(handle, "canvas")
        .then_some((runtime_ptr, handle))
}

fn publish_canvas_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    let Some((runtime_ptr, handle)) = html_canvas_identity(scope, canvas) else {
        return;
    };
    let cell = canvas_surface_cell(scope, canvas);
    let Some(snapshot) = cell
        .borrow_mut()
        .as_mut()
        .and_then(|surface| surface.snapshot().ok())
    else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.replace_canvas_pixels(
        handle,
        snapshot.width,
        snapshot.height,
        snapshot.rgba.clone(),
    );
}

fn remove_html_canvas_pixels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) {
    let Some((runtime_ptr, handle)) = html_canvas_identity(scope, canvas) else {
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.remove_canvas_pixels(handle);
}

fn canvas_like_dimensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
) -> Option<(u32, u32)> {
    let width = canvas_like_dimension(scope, canvas, super::OFFSCREEN_CANVAS_WIDTH_SLOT, "width")?;
    let height =
        canvas_like_dimension(scope, canvas, super::OFFSCREEN_CANVAS_HEIGHT_SLOT, "height")?;
    Some((width, height))
}

fn canvas_like_dimension<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    canvas: v8::Local<'s, v8::Object>,
    slot: &str,
    public_name: &'static str,
) -> Option<u32> {
    let value = get_private_value(scope, canvas, slot)
        .and_then(|value| value.number_value(scope))
        .or_else(|| webidl::optional_number_property(scope, canvas, public_name))
        .unwrap_or(0.0);
    Some(value.max(0.0).trunc() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Native surface ownership is reclaimed with the canvas (GC) and with the
    /// isolate, matching the path-state lifecycle in `state.rs`.
    #[test]
    fn canvas_surfaces_are_reclaimed_with_canvas_gc_and_isolate_destruction() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());

        let live_surface = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            // Two canvases own two independent native surfaces in one registry,
            // with stable per-canvas identity.
            let canvas_a = v8::Object::new(scope);
            let cell_a = canvas_surface_cell(scope, canvas_a);
            let canvas_b = v8::Object::new(scope);
            let cell_b = canvas_surface_cell(scope, canvas_b);
            assert_eq!(surface_store(scope).borrow().entries.len(), 2);
            assert!(Rc::ptr_eq(&cell_a, &canvas_surface_cell(scope, canvas_a)));
            assert!(
                !Rc::ptr_eq(&cell_a, &cell_b),
                "distinct canvases own distinct surfaces"
            );
            Rc::downgrade(&cell_a)
        };

        // Both canvases went out of scope; a GC must drop their native surfaces.
        isolate.low_memory_notification();
        assert!(
            live_surface.upgrade().is_none(),
            "GC must drop the unreachable canvas native surface"
        );
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let store = scope
                .get_slot::<SurfaceStore>()
                .expect("isolate slot retains registry");
            assert_eq!(
                store.borrow().entries.len(),
                0,
                "GC releases the native surface registry entries"
            );
        }

        // Isolate teardown needs no explicit cleanup.
        let _ = isolate;
    }
}
