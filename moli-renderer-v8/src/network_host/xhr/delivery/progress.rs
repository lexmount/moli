use super::super::events::{
    xhr_dispatch_progress_event_with_length_computable, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::instance_state::XHR_PROGRESS_STATE_SLOT;
use super::super::*;
use moli_webapi_declare::WebApiObject;

const XHR_PROGRESS_DATA_XHR: &str = "xhr";
const XHR_PROGRESS_DATA_INTERNAL_ID: &str = "internalId";
const XHR_PROGRESS_MINIMUM_INTERVAL_MS: u32 = 50;

const PROGRESS_TIMER: &str = "timer";
const PROGRESS_HAS_DISPATCHED: &str = "hasDispatched";
const PROGRESS_PENDING: &str = "pending";
const PROGRESS_LENGTH_COMPUTABLE: &str = "lengthComputable";
const PROGRESS_LOADED: &str = "loaded";
const PROGRESS_TOTAL: &str = "total";

/// A running throttle timer need not have a pending progress update.
#[derive(Clone, Copy, Default, WebApiObject)]
#[webapi(plain)]
struct XhrProgressState {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    // Updating an existing record must clear absent values rather than omit them.
    #[webapi(slot = PROGRESS_TIMER, value = self.timer)]
    timer: Option<u32>,
    #[webapi(slot = PROGRESS_HAS_DISPATCHED)]
    has_dispatched: bool,
    #[webapi(slot = PROGRESS_PENDING, value = self.pending)]
    pending: Option<XhrProgressSnapshot>,
}

impl XhrProgressState {
    fn read(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) -> Self {
        let Some(state) = xhr_state_value(scope, xhr, XHR_PROGRESS_STATE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            return Self::default();
        };
        let timer = get_private_value(scope, state, PROGRESS_TIMER)
            .filter(|value| !value.is_null())
            .and_then(|value| value.uint32_value(scope));
        let has_dispatched = get_private_value(scope, state, PROGRESS_HAS_DISPATCHED)
            .is_some_and(|value| value.is_true());
        let pending = get_private_value(scope, state, PROGRESS_PENDING)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .map(|snapshot| XhrProgressSnapshot {
                prototype: (),
                length_computable: get_private_value(scope, snapshot, PROGRESS_LENGTH_COMPUTABLE)
                    .is_some_and(|value| value.is_true()),
                loaded: get_private_value(scope, snapshot, PROGRESS_LOADED)
                    .and_then(|value| value.number_value(scope))
                    .expect("pending XHR progress loaded bytes"),
                total: get_private_value(scope, snapshot, PROGRESS_TOTAL)
                    .and_then(|value| value.number_value(scope))
                    .expect("pending XHR progress total bytes"),
            });
        Self {
            prototype: (),
            timer,
            has_dispatched,
            pending,
        }
    }

    fn store(&self, scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
        if let Some(state) = xhr_state_value(scope, xhr, XHR_PROGRESS_STATE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            self.initialize(scope, state)
                .expect("XHR progress state must remain complete");
        } else if self.timer.is_some() || self.has_dispatched || self.pending.is_some() {
            let state = self.bind(scope).expect("XHR progress state must bind");
            set_xhr_state_value(scope, xhr, XHR_PROGRESS_STATE_SLOT, state.into());
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct XhrProgressTimerDataDeclaration<'scope> {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    #[webapi(slot = XHR_PROGRESS_DATA_XHR)]
    xhr: v8::Local<'scope, v8::Object>,
    #[webapi(slot = XHR_PROGRESS_DATA_INTERNAL_ID)]
    internal_id: f64,
}

pub(super) fn xhr_stream_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) -> bool {
    !xhr_is_aborted(scope, xhr)
        && xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false)
        && xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT)
            .is_some_and(|active| active as u64 == internal_id)
}

pub(super) fn dispatch_or_defer_xhr_streaming_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    loaded: usize,
    total: Option<usize>,
) -> bool {
    if !xhr_stream_is_current(scope, xhr, internal_id) {
        return false;
    }

    if XhrProgressState::read(scope, xhr).timer.is_some() {
        store_deferred_xhr_progress(scope, xhr, loaded, total);
        return xhr_stream_is_current(scope, xhr, internal_id);
    }

    let snapshot = XhrProgressSnapshot::from_lengths(loaded, total);
    if !dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot) {
        return false;
    }
    schedule_xhr_progress_gate(scope, xhr, internal_id);
    xhr_stream_is_current(scope, xhr, internal_id)
}

pub(super) fn flush_xhr_streaming_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) -> bool {
    if !xhr_stream_is_current(scope, xhr, internal_id) {
        clear_xhr_progress_throttle(scope, xhr);
        return false;
    }

    cancel_xhr_progress_timer(scope, xhr);
    let deferred = take_deferred_xhr_progress(scope, xhr);
    if let Some(snapshot) = deferred
        && !dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot)
    {
        clear_xhr_progress_state(scope, xhr);
        return false;
    }
    clear_xhr_progress_state(scope, xhr);
    xhr_stream_is_current(scope, xhr, internal_id)
}

pub(in crate::network_host::xhr) fn clear_xhr_progress_throttle(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    cancel_xhr_progress_timer(scope, xhr);
    clear_xhr_progress_state(scope, xhr);
}

fn dispatch_xhr_progress_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    snapshot: XhrProgressSnapshot,
) -> bool {
    let ready_state = xhr_ready_state(scope, xhr);
    let has_dispatched = XhrProgressState::read(scope, xhr).has_dispatched;
    if ready_state != XhrReadyState::Loading || has_dispatched {
        xhr_fire_readystatechange(scope, xhr, XhrReadyState::Loading);
        if scope.is_execution_terminating() || !xhr_stream_is_current(scope, xhr, internal_id) {
            return false;
        }
    }

    XhrProgressState {
        has_dispatched: true,
        ..XhrProgressState::read(scope, xhr)
    }
    .store(scope, xhr);
    xhr_dispatch_progress_event_with_length_computable(
        scope,
        xhr,
        "progress",
        snapshot.length_computable,
        snapshot.loaded,
        snapshot.total,
    );
    !scope.is_execution_terminating() && xhr_stream_is_current(scope, xhr, internal_id)
}

fn schedule_xhr_progress_gate(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let xhr = local_object_in_scope(scope, xhr);
    let data = XhrProgressTimerDataDeclaration::new(xhr, internal_id as f64)
        .bind(scope)
        .expect("XHR progress timer data declaration should bind");
    let Some(callback) = v8::FunctionTemplate::builder(xhr_progress_timer_callback)
        .data(data.into())
        .build(scope)
        .get_function(scope)
    else {
        return;
    };
    let timer_id = unsafe { &mut *host_ptr }.queue_timeout_with_receiver(
        scope,
        callback,
        xhr,
        XHR_PROGRESS_MINIMUM_INTERVAL_MS,
        crate::host::HostTimerOwner::Window,
        Vec::new(),
    );
    XhrProgressState {
        timer: (timer_id != 0).then_some(timer_id),
        ..XhrProgressState::read(scope, xhr)
    }
    .store(scope, xhr);
}

fn xhr_progress_timer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = args.data().to_object(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(xhr) = get_private_value(scope, data, XHR_PROGRESS_DATA_XHR)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let internal_id = get_private_value(scope, data, XHR_PROGRESS_DATA_INTERNAL_ID)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u64)
        .unwrap_or(0);
    if internal_id == 0 || !xhr_stream_is_current(scope, xhr, internal_id) {
        rv.set_undefined();
        return;
    }

    let mut state = XhrProgressState::read(scope, xhr);
    state.timer = None;
    let snapshot = state.pending.take();
    state.store(scope, xhr);
    let Some(snapshot) = snapshot else {
        rv.set_undefined();
        return;
    };
    if dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot) {
        schedule_xhr_progress_gate(scope, xhr, internal_id);
    }
    rv.set_undefined();
}

fn cancel_xhr_progress_timer(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    let mut state = XhrProgressState::read(scope, xhr);
    let timer = state.timer.take();
    state.store(scope, xhr);
    if let Some(timer) = timer
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.cancel_timer(timer);
    }
}

fn store_deferred_xhr_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    loaded: usize,
    total: Option<usize>,
) {
    XhrProgressState {
        pending: Some(XhrProgressSnapshot::from_lengths(loaded, total)),
        ..XhrProgressState::read(scope, xhr)
    }
    .store(scope, xhr);
}

fn take_deferred_xhr_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<XhrProgressSnapshot> {
    let mut state = XhrProgressState::read(scope, xhr);
    let pending = state.pending.take();
    state.store(scope, xhr);
    pending
}

fn clear_xhr_progress_state(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    XhrProgressState {
        timer: XhrProgressState::read(scope, xhr).timer,
        ..XhrProgressState::default()
    }
    .store(scope, xhr);
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

#[derive(Clone, Copy, WebApiObject)]
#[webapi(plain)]
struct XhrProgressSnapshot {
    #[webapi(prototype, value = v8::null(scope))]
    prototype: (),
    #[webapi(slot = PROGRESS_LENGTH_COMPUTABLE)]
    length_computable: bool,
    #[webapi(slot = PROGRESS_LOADED)]
    loaded: f64,
    #[webapi(slot = PROGRESS_TOTAL)]
    total: f64,
}

impl XhrProgressSnapshot {
    fn from_lengths(loaded: usize, total: Option<usize>) -> Self {
        match total.filter(|total| *total > 0 && loaded <= *total) {
            Some(total) => Self {
                prototype: (),
                length_computable: true,
                loaded: loaded as f64,
                total: total as f64,
            },
            None => Self {
                prototype: (),
                length_computable: false,
                loaded: loaded as f64,
                total: 0.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::XhrProgressSnapshot;

    #[test]
    fn progress_snapshot_only_exposes_a_valid_nonzero_total() {
        let valid = XhrProgressSnapshot::from_lengths(4, Some(7));
        assert!(valid.length_computable);
        assert_eq!(valid.loaded, 4.0);
        assert_eq!(valid.total, 7.0);

        for invalid in [
            XhrProgressSnapshot::from_lengths(0, None),
            XhrProgressSnapshot::from_lengths(0, Some(0)),
            XhrProgressSnapshot::from_lengths(8, Some(7)),
        ] {
            assert!(!invalid.length_computable);
            assert_eq!(invalid.total, 0.0);
        }
    }
}
